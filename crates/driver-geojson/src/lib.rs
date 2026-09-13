//! driver-geojson — `GeoJSON` ⇄ `RecordBatch`. `GeoJSON` è WGS84 per specifica
//! (`OGC:CRS84`). La geometria diventa una colonna WKB `geoarrow.wkb`.
//!
//! Lettura **streaming** (Fase 2A): l'array `features` del `FeatureCollection` è
//! scorso un feature alla volta (`geojson::FeatureReader`), senza costruire il
//! DOM `serde_json::Value` dell'intero documento. Due passate: pass 1 (`open`)
//! inferisce lo schema (unione chiavi/tipi) a RAM O(1); pass 2 (reader) è un
//! thread che produce `RecordBatch` da `batch_target` righe, consegnati via canale
//! con backpressure → memoria O(batch), non O(file). Geometrie convertite
//! direttamente a WKB, attributi in builder tipizzati (niente intermedio
//! `serde_json::Value` per colonna). La scrittura resta bufferizzante nella v1.
#![forbid(unsafe_code)]

pub(crate) mod geometry;

pub use geometry::{wkb_from_gj_value, write_geo_geojson};
// La deserializzazione limitata durante il parse (S12): usata da qui,
// non esposta. Il confine pubblico resta la lettura del dataset.
pub(crate) mod geometria_progressiva;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::builder::BinaryBuilder;
use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, RecordBatch,
    RecordBatchOptions, StringArray,
};
use arrow_schema::{Field, Schema, SchemaRef};
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{
    DeserializeSeed, Deserializer, Error as DeError, IgnoredAny, MapAccess, SeqAccess, Visitor,
};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use driver_common::{
    classify_i64, classify_u64, geometry_field, geometry_index, json_from_array, ColType,
    InferredColumnBuilder, ObservedValueClass, TypeAccumulator, OGC_CRS84,
};
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    spawn_batch_reader, BatchEmitter, FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle,
    Published, ReadOptions, Sink, Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::StagedFile;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    read_row_error, validate_write, with_write_validation, AttributeWriteSupport, CrsDerivation,
    CrsRepresentationCapabilities, CrsRepresentationState, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, SinkPathConstraint, TypeCoercionPolicy, WritePlan,
    SCALAR_TYPES, UTF8_FIELD_NAMES, WKB_XY_XYZ_GEOMETRY,
};
use plenora_io_model::contract::{
    DataContract, FieldId, GeometryColumnContract, GeometryType, LayerContract, LayerId,
};
use plenora_io_model::crs::ResolvedCrs;
#[cfg(test)]
use plenora_io_model::geometry::is_geometry_field;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::decode_wkb;
use plenora_io_model::wkb::{encode_wkb_into_bounded, WkbFlavor};
use plenora_io_model::{NumeroStrutturale, PlenoraIoError, PublicMessage, Result};

const GEOMETRY: &str = "geometry";

fn err(reason: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("geojson", reason)
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "geojson",
    Direction::Bidirectional,
    ReadMode::StreamingSequential, // array `features` scorso in streaming
    // INV-7: deserializer serde streaming direttamente nei builder.
    plenora_io_core::NativeReadMode::StreamingSequential,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Streaming), // feature-per-feature, niente buffering
    Some(plenora_io_core::DeterminismLevel::Semantic),
    false,
    false,
    ReaderConcurrency::MultipleIndependentReaders,
    plenora_io_core::ProjectionSupport::Exact,
    plenora_io_core::PredicatePruningSupport::None,
    plenora_io_core::SpatialPruningSupport::None,
    CrsHandling::FixedWgs84,
    // Finding #8 review 2026-08-15: dichiarare `Lossless` staticamente non
    // riflette il comportamento reale del driver, che non conserva `id`,
    // `bbox` ne' foreign members al re-encode (writer a riga 1088+ emette
    // solo `type`, `geometry` e `properties`). Il principio scritto in
    // `PRODUCT.md § LossReport — "un report vuoto significa 'nessuna`
    // perdita osservata', non `Lossless`" — vale anche qui: il descrittore
    // dichiara la classe potenziale, il `LossReport` dichiara le perdite
    // osservate.
    Fidelity::Conditional,
    Runtime::PureRust,
    // `hostile_input_hardened`: la geometria si deserializza direttamente nell'AST, addebitando ogni
    // posizione e ogni figlia mentre serde le consegna (S12).
    true,
    // `spec_version_supported`: il formato non si versiona in un modo che
    // il driver possa dichiarare per intero.
    None,
    Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_XY_XYZ_GEOMETRY,
        crs: CrsWriteSupport::Fixed("OGC:CRS84"),
        crs_representations: CrsRepresentationCapabilities::new(
            // Nessuna rappresentazione conservata, e l'identificatore e'
            // ricavabile lo stesso: il formato fissa il proprio CRS, quindi
            // chi rilegge il file sa qual e' senza che noi lo scriviamo.
            CrsRepresentationState::Derived(CrsDerivation::FixedByFormat),
            CrsRepresentationState::Absent,
            CrsRepresentationState::Absent,
        ),
        nullability: NullabilitySupport::Preserve,
        multi_layer: false,
        sink_path: SinkPathConstraint::Free,
    }),
    // Il driver non interpreta alcuna format_option (L0.7): l'elenco vuoto
    // e' l'affermazione che qualunque chiave e' sconosciuta, non un'omissione.
    plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
    &["geojson", "json"],
    1,
    6,
    10,
);

pub struct GeoJsonDriver;

impl FormatDriver for GeoJsonDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = plenora_io_core::preflight_source(self.descriptor(), source, &mut opts)?;
        // Pass 1: inferenza schema in streaming (RAM O(1)).
        let quote = QuoteInferenza::from_read_options(&opts);
        let (schema, cols, tipi) = infer_schema(&path, quote)?;
        let mut geometria = GeometryColumnContract::wkb_passthrough(
            FieldId(0),
            GEOMETRY,
            ResolvedCrs::wgs84(),
            true,
        );
        // I tipi visti dalla passata di inferenza, dichiarati esatti. GeoJSON
        // non ha un posto dove scriverli -- non e' un formato che dichiara uno
        // schema -- quindi l'unico modo di saperli e' guardare le feature, ed
        // e' quello che la passata fa gia'. Senza questa riga il contratto
        // arriva al writer senza tipi, e i formati che li pretendono in
        // anticipo rifiutano una conversione che il file puo' fare.
        // Senza condizione, e la condizione che c'era prima era il difetto: un
        // elenco vuoto dopo una passata arrivata a fine file **e'** una
        // conoscenza -- geometrie non ce ne sono -- e saltare la chiamata la
        // faceva sembrare ignoranza. Una FeatureCollection vuota veniva
        // rifiutata verso ogni sink che pretende i tipi in anticipo.
        geometria.set_exact_geometry_types(tipi);
        let contract = DataContract::new(schema, Some(geometria));
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(plenora_io_core::with_read_budget(
            Box::new(GeoJsonDataset {
                path,
                quote,
                cols,
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name,
                    contract,
                }],
            }),
            &opts,
            true,
        ))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(
            self.descriptor(),
            plan,
            opts.max_columns(),
            &opts.format_options,
        )?;
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraIoError::destinazione_esistente());
        }
        // Nessun controllo sul suffisso della destinazione.
        //
        // Ce n'era uno, e rifiutava una destinazione il cui nome non portasse
        // l'estensione attesa. Non era un requisito del formato: dopo il
        // controllo l'estensione non veniva usata per **niente** -- ne' per
        // derivare un nome, ne' per scegliere un comportamento -- ed era una
        // convenzione travestita da vincolo. Il costo era che il formato
        // esplicito non bastava a scegliere la destinazione: chi pubblicava su
        // un percorso di staging, o su un nome generato, veniva rifiutato per
        // il nome invece che per i dati.
        //
        // Il suffisso resta rilevante per chi **rilegge** senza dichiarare il
        // formato, e quella parte e' dichiarata e non taciuta:
        // `recognised_suffixes` del descrittore la rende, e `io.catalog` la
        // pubblica. Scrivere e riconoscere sono due cose, e ora si vedono
        // entrambe.
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("GeoJSON: un solo layer per file nella v1"),
            ));
        }
        let staging = StagedFile::new(&path, opts.durable, opts.max_output_bytes())?;
        let mut writer = BufWriter::new(staging.reopen()?);
        writer.write_all(b"{\"type\":\"FeatureCollection\",\"features\":[")?;
        with_write_validation(
            Box::new(GeoJsonWriter {
                staging,
                writer: Some(writer),
                first: true,
                wkb_limits: opts.wkb_limits(),
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

// --- lettura streaming -----------------------------------------------------

struct GeoJsonDataset {
    path: PathBuf,
    /// Le quote configurate, portate dal dataset: senza, la lettura tornerebbe
    /// al default proprio dopo che l'inferenza ha rispettato il flag.
    quote: QuoteInferenza,
    cols: Vec<(String, ColType)>,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for GeoJsonDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(
            DESCRIPTOR.id(),
            DESCRIPTOR.fidelity_class(),
        )
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let (indices, layer) = plenora_io_core::project_layer_contract(&self.layers[0], request)?;
        let include_geometry = indices.binary_search(&0).is_ok();
        let property_names = self.cols.iter().map(|(name, _)| name.clone()).collect();
        let cols = indices
            .iter()
            .filter_map(|&index| {
                index
                    .checked_sub(1)
                    .and_then(|column_index| self.cols.get(column_index))
                    .cloned()
            })
            .collect();
        let batch_sizer = plenora_io_core::AdaptiveBatchSizer::new(
            layer.contract.schema.as_ref(),
            request.batch_target,
        );
        let reader = spawn_parser(
            (self.path.clone(), self.quote),
            layer.contract.schema.clone(),
            cols,
            property_names,
            include_geometry,
            batch_sizer,
            layer,
        )?;
        Ok(plenora_io_core::with_cancellation(
            reader,
            request.cancellation.clone(),
        ))
    }
}

/// Pass 1: unione chiavi proprietà + tipo, con un deserializer serde **streaming**
/// che legge SOLO le chiavi e la classe di tipo dei valori — niente DOM, niente
/// geometria, niente valori materializzati (allocazioni ~ solo le chiavi nuove).
/// Le quote che le due passate consultano.
///
/// Le stesse in inferenza e in lettura: un tetto che vale in una passata e non
/// nell'altra sarebbe peggio che non averlo, perche' il rifiuto arriverebbe a
/// meta' lettura invece che all'apertura.
#[derive(Clone, Copy)]
struct QuoteInferenza {
    /// Tetto sui byte del testo grezzo di una geometria, applicato **prima**
    /// di deserializzarla: e' li' che l'AST verrebbe allocato.
    cella: WkbLimits,
    /// Tetto sulle feature visitate.
    ///
    /// Non e' `max_input_entries`: quella governa l'enumerazione della
    /// sorgente e il preflight l'ha gia' applicata al file. Applicarla di
    /// nuovo alle feature sarebbe la stessa quota contata due volte, e al
    /// valore predefinito rifiuterebbe un `GeoJSON` di dimensioni ordinarie.
    feature: u64,
}

impl QuoteInferenza {
    fn from_read_options(opts: &ReadOptions) -> Self {
        Self {
            cella: opts.wkb_limits(),
            feature: u64::try_from(opts.max_rows()).unwrap_or(u64::MAX),
        }
    }
}

/// Cio' che la passata di inferenza sa dire del file: lo schema, le colonne
/// con il tipo accumulato, e i tipi geometrici visti.
type SchemaInferito = (SchemaRef, Vec<(String, ColType)>, Vec<GeometryType>);

fn infer_schema(path: &Path, quote: QuoteInferenza) -> Result<SchemaInferito> {
    let file = File::open(path)?;
    let mut accs = SchemaAccumulators {
        max_feature: quote.feature,
        ..SchemaAccumulators::default()
    };
    let mut de = serde_json::Deserializer::from_reader(BufReader::new(file));
    if de.deserialize_map(TopVisitor { accs: &mut accs }).is_err() {
        // L'errore vero viene dal canale laterale, non dal testo di serde: la
        // via di serde e' quella che appiattiva codice, categoria e fase.
        let error = accs
            .errore
            .take()
            .unwrap_or_else(|| err(&PublicMessage::Curated("GeoJSON non valido")));
        return Err(if accs.in_feature {
            read_row_error(
                error,
                Some(accs.source_rows_seen),
                "geojson.invalid_feature",
                None,
            )
        } else {
            error
        });
    }
    let tipi: Vec<GeometryType> = accs.tipi_geometrici.iter().copied().collect();
    let cols = accs
        .into_columns()
        .map_err(|motivo| err(&PublicMessage::Curated(motivo)))?;
    let mut fields = vec![geometry_field(GEOMETRY, OGC_CRS84)];
    for (k, ct) in &cols {
        fields.push(Field::new(k, ct.arrow_data_type(), true));
    }
    Ok((Arc::new(Schema::new(fields)), cols, tipi))
}

// --- pass-1: visitor serde streaming (chiavi + tipo, zero valori) ----------

#[derive(Default)]
struct SchemaAccumulators {
    indices: HashMap<String, usize>,
    values: Vec<TypeAccumulator>,
    source_rows_seen: u64,
    /// Tetto sulle feature visitate dalla passata di inferenza. Zero significa
    /// "non ancora configurato", e non puo' capitare: `infer_schema` lo
    /// imposta prima di deserializzare.
    max_feature: u64,
    in_feature: bool,
    /// L'errore vero, messo da parte prima di chiedere a serde di fermarsi.
    ///
    /// `serde` sa portare solo stringhe: farci passare un `PlenoraIoError`
    /// significa perderne il codice, la categoria e la fase, e lasciare al
    /// chiamante un testo da rileggere per indovinarli.
    errore: Option<PlenoraIoError>,
    /// I tipi geometrici visti, per dichiararli esatti nel contratto.
    ///
    /// La passata di inferenza non tronca: superato `max_feature` **rifiuta**,
    /// quindi se arriva in fondo ha visto ogni feature e questo insieme e'
    /// completo. E' la condizione che rende onesto chiamarli «esatti»: un
    /// insieme raccolto da una passata troncata sarebbe un elenco parziale
    /// dichiarato come totale.
    tipi_geometrici: std::collections::BTreeSet<GeometryType>,
}

impl SchemaAccumulators {
    fn index_for(&mut self, name: &str) -> usize {
        if let Some(index) = self.indices.get(name) {
            return *index;
        }
        let index = self.values.len();
        self.indices.insert(name.to_owned(), index);
        self.values.push(TypeAccumulator::default());
        index
    }

    fn observe(
        &mut self,
        index: usize,
        value: ObservedValueClass,
    ) -> std::result::Result<(), &'static str> {
        let accumulator = self
            .values
            .get_mut(index)
            .ok_or("indice di inferenza GeoJSON incoerente")?;
        accumulator.observe(value);
        Ok(())
    }

    fn into_columns(self) -> std::result::Result<Vec<(String, ColType)>, &'static str> {
        let Self {
            indices, values, ..
        } = self;
        let mut columns = Vec::with_capacity(indices.len());
        for (name, index) in indices {
            let accumulator = values
                .get(index)
                .ok_or("indice di inferenza GeoJSON incoerente")?;
            columns.push((name, accumulator.column_type()));
        }
        columns.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        Ok(columns)
    }
}

/// Classe di tipo di un valore JSON, letta senza allocare il valore.
/// Testo minimo consegnato a `serde` per fermare la deserializzazione.
///
/// Non e' il messaggio dell'errore: quello sta nel canale laterale. Serve solo
/// perche' `DeError::custom` vuole qualcosa, ed e' scelto in modo che, se per
/// una via imprevista finisse davvero in un errore pubblico, non dica una cosa
/// falsa.
const INTERROTTO: &str = "GeoJSON non valido";

/// Mette da parte l'errore vero e chiede a serde di fermarsi.
///
/// Il primo errore vince: quelli successivi sono conseguenze
/// dell'interruzione, non cause.
///
/// Prende lo slot e non lo stato intero perche' `ValueSink` tiene in prestito
/// un builder di `RowSink` e non potrebbe prestarsi anche il resto: due campi
/// distinti si prestano insieme, la struct intera no.
fn ferma_in<E: DeError>(slot: &mut Option<PlenoraIoError>, errore: PlenoraIoError) -> E {
    if slot.is_none() {
        *slot = Some(errore);
    }
    E::custom(INTERROTTO)
}

impl SchemaAccumulators {
    fn ferma<E: DeError>(&mut self, errore: PlenoraIoError) -> E {
        ferma_in(&mut self.errore, errore)
    }
}

impl RowSink {
    fn ferma<E: DeError>(&mut self, errore: PlenoraIoError) -> E {
        ferma_in(&mut self.errore, errore)
    }
}

struct TypeTag(ObservedValueClass);

impl<'de> serde::Deserialize<'de> for TypeTag {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        d.deserialize_any(TagVisitor)
    }
}

struct TagVisitor;
impl<'de> Visitor<'de> for TagVisitor {
    type Value = TypeTag;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un valore JSON")
    }
    fn visit_i64<E>(self, value: i64) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(classify_i64(value)))
    }
    fn visit_u64<E>(self, value: u64) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(classify_u64(value)))
    }
    fn visit_i128<E>(self, value: i128) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(
            i64::try_from(value).map_or(ObservedValueClass::Text, classify_i64),
        ))
    }
    fn visit_u128<E>(self, value: u128) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(
            u64::try_from(value).map_or(ObservedValueClass::Text, classify_u64),
        ))
    }
    fn visit_f64<E>(self, _: f64) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(ObservedValueClass::Number))
    }
    fn visit_bool<E>(self, _: bool) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(ObservedValueClass::Boolean))
    }
    fn visit_str<E>(self, _: &str) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(ObservedValueClass::Text))
    }
    fn visit_none<E>(self) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(ObservedValueClass::Null))
    }
    fn visit_unit<E>(self) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(ObservedValueClass::Null))
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<TypeTag, D::Error> {
        d.deserialize_any(Self)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<TypeTag, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(TypeTag(ObservedValueClass::Text))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<TypeTag, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(TypeTag(ObservedValueClass::Text))
    }
}

/// Livello top: `FeatureCollection`; interessa solo la chiave "features".
struct TopVisitor<'a> {
    accs: &'a mut SchemaAccumulators,
}
impl<'de> Visitor<'de> for TopVisitor<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un oggetto GeoJSON")
    }
    // Finding #8 review 2026-08-15: prima del fix il top-visitor cercava
    // soltanto la chiave `features` e trattava come vuoto qualunque
    // documento, incluso `{}` o un `Feature` singolo. La specifica GeoJSON
    // (RFC 7946 §3) rende `type` obbligatorio; qui il driver e' single-layer
    // e supporta solo `FeatureCollection`. Fallire chiuso su un `type`
    // assente o inatteso e' meno pericoloso di esporre un dataset vuoto
    // silenziosamente.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        let mut type_observed: Option<String> = None;
        let mut features_observed = false;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "type" => {
                    let value: String = map.next_value()?;
                    if value != "FeatureCollection" {
                        return Err(self.accs.ferma(err(&PublicMessage::Curated(
                            "GeoJSON top-level type non supportato: atteso 'FeatureCollection'",
                        ))));
                    }
                    type_observed = Some(value);
                }
                "features" => {
                    map.next_value_seed(FeaturesSeed { accs: self.accs })?;
                    features_observed = true;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if type_observed.is_none() {
            return Err(self.accs.ferma(err(&PublicMessage::Curated(
                "GeoJSON senza campo 'type' al livello top",
            ))));
        }
        // Follow-up review 2026-08-15: la specifica GeoJSON (RFC 7946 §3.3)
        // dichiara `features` obbligatorio per un `FeatureCollection`. Un
        // documento con solo `{"type":"FeatureCollection"}` non e' vuoto
        // per definizione: e' incompleto. Fail-closed invece di trattarlo
        // come un dataset di zero righe.
        if !features_observed {
            return Err(self.accs.ferma(err(&PublicMessage::Curated(
                "FeatureCollection GeoJSON senza campo 'features'",
            ))));
        }
        Ok(())
    }
}

struct FeaturesSeed<'a> {
    accs: &'a mut SchemaAccumulators,
}
impl<'de> DeserializeSeed<'de> for FeaturesSeed<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_seq(FeaturesVisitor { accs: self.accs })
    }
}
struct FeaturesVisitor<'a> {
    accs: &'a mut SchemaAccumulators,
}
impl<'de> Visitor<'de> for FeaturesVisitor<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un array di feature")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        while seq
            .next_element_seed(FeatureSeed { accs: self.accs })?
            .is_some()
        {}
        Ok(())
    }
}

struct FeatureSeed<'a> {
    accs: &'a mut SchemaAccumulators,
}
impl<'de> DeserializeSeed<'de> for FeatureSeed<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_map(FeatureVisitor { accs: self.accs })
    }
}
struct FeatureVisitor<'a> {
    accs: &'a mut SchemaAccumulators,
}
impl<'de> Visitor<'de> for FeatureVisitor<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un Feature")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        self.accs.in_feature = true;
        // Follow-up review 2026-08-15: la specifica GeoJSON impone
        // `type=Feature` per ogni oggetto Feature (RFC 7946 §3.2). La
        // pass-1 valida il campo per rifiutare documenti con oggetti che
        // hanno tipo diverso (es. Geometry standalone in un array di
        // features) o che omettono il tipo del tutto.
        let mut type_observed: Option<String> = None;
        while let Some(key) = map.next_key_seed(FeatKeySeed)? {
            match key {
                FeatKey::Props => map.next_value_seed(PropsSeed { accs: self.accs })?,
                FeatKey::Type => {
                    let value: String = map.next_value()?;
                    if value != "Feature" {
                        return Err(self.accs.ferma(err(&PublicMessage::Curated(
                            "membro di features con type diverso da 'Feature'",
                        ))));
                    }
                    type_observed = Some(value);
                }
                FeatKey::Geom => map.next_value_seed(TipoGeometricoSeed { accs: self.accs })?,
                FeatKey::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if type_observed.is_none() {
            return Err(self.accs.ferma(err(&PublicMessage::Curated(
                "membro di features senza campo 'type'",
            ))));
        }
        self.accs.in_feature = false;
        self.accs.source_rows_seen =
            self.accs.source_rows_seen.checked_add(1).ok_or_else(|| {
                self.accs
                    .ferma(err(&PublicMessage::Curated("troppe feature GeoJSON")))
            })?;
        if self.accs.source_rows_seen > self.accs.max_feature {
            let tetto = self.accs.max_feature;
            return Err(self.accs.ferma(PlenoraIoError::limite_redatto(
                &PublicMessage::CuratedWith(
                    "l'inferenza si e' fermata al tetto di feature:",
                    NumeroStrutturale::Limite(tetto),
                ),
            )));
        }
        Ok(())
    }
}

struct PropsSeed<'a> {
    accs: &'a mut SchemaAccumulators,
}
impl<'de> DeserializeSeed<'de> for PropsSeed<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        // `deserialize_any`: le properties possono essere `null` (oltre a oggetto).
        d.deserialize_any(PropsVisitor { accs: self.accs })
    }
}
struct PropsVisitor<'a> {
    accs: &'a mut SchemaAccumulators,
}
impl<'de> Visitor<'de> for PropsVisitor<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("le properties di un Feature (oggetto o null)")
    }
    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_any(self)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        while let Some(index) = map.next_key_seed(SchemaKeySeed { accs: self.accs })? {
            let tag = map.next_value::<TypeTag>()?.0;
            if let Err(motivo) = self.accs.observe(index, tag) {
                return Err(self.accs.ferma(err(&PublicMessage::Curated(motivo))));
            }
        }
        Ok(())
    }
}

struct SchemaKeySeed<'a> {
    accs: &'a mut SchemaAccumulators,
}

impl<'de> DeserializeSeed<'de> for SchemaKeySeed<'_> {
    type Value = usize;

    fn deserialize<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        deserializer.deserialize_str(SchemaKeyVisitor { accs: self.accs })
    }
}

struct SchemaKeyVisitor<'a> {
    accs: &'a mut SchemaAccumulators,
}

impl Visitor<'_> for SchemaKeyVisitor<'_> {
    type Value = usize;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("una chiave di proprietà")
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(self.accs.index_for(value))
    }
}

/// Pass 2: thread che produce batch da `batch_size` righe via canale bounded
/// (backpressure → memoria O(batch)).
/// `sorgente` tiene insieme il percorso e le sue quote: separarli
/// consentirebbe di accoppiare il file di un dataset con il tetto di un altro,
/// ed e' anche cio' che tiene la lista dei parametri sotto il tetto di clippy.
fn spawn_parser(
    sorgente: (PathBuf, QuoteInferenza),
    schema: SchemaRef,
    cols: Vec<(String, ColType)>,
    property_names: Vec<String>,
    include_geometry: bool,
    batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    layer: LayerContract,
) -> Result<Box<dyn LayerReader>> {
    let (path, quote) = sorgente;
    spawn_batch_reader(DESCRIPTOR.id(), layer, 2, move |emitter: BatchEmitter| {
        let file = File::open(&path)?;
        let ncols = cols.len();
        let col_idx: HashMap<String, usize> = cols
            .iter()
            .enumerate()
            .map(|(i, (k, _))| (k.clone(), i))
            .collect();
        let property_idx: HashMap<String, usize> = property_names
            .into_iter()
            .enumerate()
            .map(|(index, name)| (name, index))
            .collect();
        let property_count = property_idx.len();
        let mut sink = RowSink {
            schema: schema.clone(),
            col_idx,
            property_idx,
            output: RowOutput::Worker(emitter),
            geom: include_geometry.then(BinaryBuilder::new),
            wkb_buf: Vec::new(),
            limiti_wkb: quote.cella,
            builders: cols
                .iter()
                .map(|(_, column_type)| InferredColumnBuilder::new(*column_type))
                .collect(),
            seen: vec![false; ncols],
            property_seen: vec![false; property_count],
            n: 0,
            source_rows_seen: 0,
            in_feature: false,
            batch_sizer,
            aborted: false,
            errore: None,
        };
        // Deserializer serde streaming: scrive i feature DIRETTAMENTE nei
        // builder (chiavi via key-seed = 0 alloc, valori scalari appesi
        // diretti = 0 alloc). Niente DOM Feature/JsonObject per feature.
        let mut de = serde_json::Deserializer::from_reader(BufReader::new(file));
        let result = de.deserialize_map(TopSink { sink: &mut sink });
        if sink.aborted {
            return Ok(()); // consumatore andato via: stop pulito
        }
        if result.is_err() {
            let error = sink
                .errore
                .take()
                .unwrap_or_else(|| err(&PublicMessage::Curated("GeoJSON non valido")));
            return Err(if sink.in_feature {
                read_row_error(
                    error,
                    Some(sink.source_rows_seen),
                    "geojson.invalid_feature",
                    None,
                )
            } else {
                error
            });
        }
        if sink.n > 0 {
            let batch = finish_batch(&sink.schema, &mut sink.geom, &mut sink.builders, sink.n)?;
            if !sink.output.send(batch) {
                return Ok(());
            }
        }
        Ok(())
    })
}

enum RowOutput {
    Worker(BatchEmitter),
    Discard,
}

impl RowOutput {
    fn send(&self, batch: RecordBatch) -> bool {
        match self {
            Self::Worker(emitter) => emitter.send(batch),
            Self::Discard => true,
        }
    }
}

/// Stato del pass-2: builder tipizzati + bookkeeping per feature. Possiede tutto
/// (`schema`/`tx`/`col_idx` sono clonati, cheap) così i seed serde lo passano
/// come `&mut RowSink` senza parametri di lifetime.
struct RowSink {
    schema: SchemaRef,
    col_idx: HashMap<String, usize>,
    property_idx: HashMap<String, usize>,
    output: RowOutput,
    geom: Option<BinaryBuilder>,
    wkb_buf: Vec<u8>,
    /// I tetti del bordo per una geometria.
    ///
    /// Era il solo cap in byte. Da S12 la deserializzazione applica anche
    /// componenti e profondita' **durante** il parse: il tipo che viaggia e'
    /// quindi il contratto intero, non una sua meta'.
    limiti_wkb: WkbLimits,
    builders: Vec<InferredColumnBuilder>,
    seen: Vec<bool>,
    property_seen: Vec<bool>,
    n: usize,
    source_rows_seen: u64,
    in_feature: bool,
    batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    aborted: bool,
    /// Come `SchemaAccumulators::errore`, per la passata di lettura.
    errore: Option<PlenoraIoError>,
}

// --- pass-2: catena di seed/visitor che scrivono nei builder ----------------

/// Top: `FeatureCollection`; interessa solo "features".
struct TopSink<'a> {
    sink: &'a mut RowSink,
}
impl<'de> Visitor<'de> for TopSink<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un oggetto GeoJSON")
    }
    // Finding #8: la pass-2 replica la validazione della pass-1. Un
    // documento che ha superato la pass-1 non dovrebbe fallire qui, ma il
    // controllo va replicato perche' la pass-2 non riceve garanzie dalla
    // pass-1 e potrebbe essere usata da soli in test o campagne future.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        self.sink.in_feature = true;
        let mut type_observed = false;
        let mut features_observed = false;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "type" => {
                    let value: String = map.next_value()?;
                    if value != "FeatureCollection" {
                        return Err(self.sink.ferma(err(&PublicMessage::Curated(
                            "GeoJSON top-level type non supportato: atteso 'FeatureCollection'",
                        ))));
                    }
                    type_observed = true;
                }
                "features" => {
                    map.next_value_seed(FeaturesSink { sink: self.sink })?;
                    features_observed = true;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if !type_observed {
            return Err(self.sink.ferma(err(&PublicMessage::Curated(
                "GeoJSON senza campo 'type' al livello top",
            ))));
        }
        if !features_observed {
            return Err(self.sink.ferma(err(&PublicMessage::Curated(
                "FeatureCollection GeoJSON senza campo 'features'",
            ))));
        }
        Ok(())
    }
}

struct FeaturesSink<'a> {
    sink: &'a mut RowSink,
}
impl<'de> DeserializeSeed<'de> for FeaturesSink<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for FeaturesSink<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un array di feature")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        while seq
            .next_element_seed(FeatureSink { sink: self.sink })?
            .is_some()
        {}
        Ok(())
    }
}

struct FeatureSink<'a> {
    sink: &'a mut RowSink,
}
impl<'de> DeserializeSeed<'de> for FeatureSink<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for FeatureSink<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un Feature")
    }
    // Il visitor gestisce tutte le chiavi di una Feature: type, geometry,
    // properties, dup-check, budget cap sulla geometria (finding #6), e le
    // append fisse su builder/geom. Estrarre parti in helper dedicati
    // rompe la sequenza degli stati del sink senza guadagno di leggibilita'.
    #[allow(clippy::too_many_lines)]
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        self.sink.seen.fill(false);
        self.sink.property_seen.fill(false);
        let mut geom_seen = false;
        let mut props_seen = false;
        let mut type_seen = false;
        while let Some(fk) = map.next_key_seed(FeatKeySeed)? {
            match fk {
                FeatKey::Type if type_seen => {
                    return Err(self.sink.ferma(err(&PublicMessage::Curated(
                        "chiave type duplicata nella feature GeoJSON",
                    ))));
                }
                FeatKey::Type => {
                    let value: String = map.next_value()?;
                    if value != "Feature" {
                        return Err(self.sink.ferma(err(&PublicMessage::Curated(
                            "membro di features con type diverso da 'Feature'",
                        ))));
                    }
                    type_seen = true;
                }
                FeatKey::Geom if geom_seen => {
                    return Err(self.sink.ferma(err(&PublicMessage::Curated(
                        "chiave geometry duplicata nella feature GeoJSON",
                    ))));
                }
                FeatKey::Geom => {
                    if let Some(geometry) = &mut self.sink.geom {
                        // Finding #6 review 2026-08-15, chiuso da S12.
                        //
                        // Intercettare la geometria come `RawValue` da' la sua
                        // lunghezza in byte e permette di rifiutare oltre il cap
                        // del bordo senza materializzare niente. Restava che la
                        // deserializzazione, una volta passato il cap,
                        // costruiva ricorsivamente `Vec` di coordinate, anelli
                        // e geometrie prima che un solo contatore le vedesse: un
                        // megabyte di posizioni sta sotto qualunque cap
                        // ragionevole e costa cinquantamila `Vec`.
                        //
                        // `geometria_progressiva` deserializza direttamente nel
                        // nostro AST e addebita ogni posizione e ogni figlia nel
                        // momento in cui serde gliela consegna.
                        let raw = map.next_value::<Option<Box<serde_json::value::RawValue>>>()?;
                        match raw {
                            None => geometry.append_null(),
                            Some(raw) => {
                                let raw_text = raw.get();
                                // La quota **configurata**: fino a S5 questa
                                // riga usava il default del contratto, quindi
                                // `--max-wkb-cell-bytes` non arrivava fin qui
                                // e una geometria oltre la soglia richiesta
                                // veniva deserializzata comunque.
                                let max_bytes = self.sink.limiti_wkb.max_cell_bytes;
                                if raw_text.len() > max_bytes {
                                    let letti = raw_text.len();
                                    return Err(self.sink.ferma(PlenoraIoError::limite_redatto(
                                        &PublicMessage::CuratedBetween(
                                            "geometria GeoJSON di",
                                            NumeroStrutturale::Conteggio(
                                                driver_common::saturating_u64(letti),
                                            ),
                                            "byte oltre il limite di",
                                            NumeroStrutturale::Limite(
                                                driver_common::saturating_u64(max_bytes),
                                            ),
                                        ),
                                    )));
                                }
                                // L'errore e' gia' nostro, con il suo codice:
                                // passa intero invece di essere appiattito nel
                                // testo di serde.
                                let geometria = match crate::geometria_progressiva::analizza(
                                    raw_text,
                                    &self.sink.limiti_wkb,
                                ) {
                                    Ok(geometria) => geometria,
                                    Err(errore) => return Err(self.sink.ferma(errore)),
                                };
                                self.sink.wkb_buf.clear();
                                if let Err(errore) = encode_wkb_into_bounded(
                                    &geometria,
                                    WkbFlavor::Iso,
                                    &mut self.sink.wkb_buf,
                                    max_bytes,
                                ) {
                                    return Err(self.sink.ferma(errore));
                                }
                                geometry.append_value(&self.sink.wkb_buf);
                            }
                        }
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                    geom_seen = true;
                }
                FeatKey::Props if props_seen => {
                    return Err(self.sink.ferma(err(&PublicMessage::Curated(
                        "chiave properties duplicata nella feature GeoJSON",
                    ))));
                }
                FeatKey::Props => {
                    map.next_value_seed(PropsSink { sink: self.sink })?;
                    props_seen = true;
                }
                FeatKey::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        // Follow-up review 2026-08-15: pass-2 replica il controllo di
        // pass-1 sul `type` obbligatorio di ogni Feature. Se qui manca,
        // fail-closed prima di allineare le colonne.
        if !type_seen {
            return Err(self.sink.ferma(err(&PublicMessage::Curated(
                "membro di features senza campo 'type'",
            ))));
        }
        // Allinea le colonne: una append per builder per feature.
        if !geom_seen {
            if let Some(geometry) = &mut self.sink.geom {
                geometry.append_null();
            }
        }
        for i in 0..self.sink.builders.len() {
            if !self.sink.seen[i] {
                self.sink.builders[i].append_null();
            }
        }
        self.sink.n += 1;
        if self.sink.n >= self.sink.batch_sizer.rows() {
            match finish_batch(
                &self.sink.schema,
                &mut self.sink.geom,
                &mut self.sink.builders,
                self.sink.n,
            ) {
                Ok(batch) => {
                    self.sink.batch_sizer.observe(&batch);
                    if !self.sink.output.send(batch) {
                        self.sink.aborted = true;
                        return Err(self
                            .sink
                            .ferma(err(&PublicMessage::Curated("consumatore chiuso"))));
                    }
                }
                Err(errore) => return Err(self.sink.ferma(errore)),
            }
            self.sink.n = 0;
        }
        self.sink.in_feature = false;
        self.sink.source_rows_seen =
            self.sink.source_rows_seen.checked_add(1).ok_or_else(|| {
                self.sink
                    .ferma(err(&PublicMessage::Curated("troppe feature GeoJSON")))
            })?;
        Ok(())
    }
}

/// Chiave a livello di Feature, riconosciuta senza allocare la stringa.
enum FeatKey {
    Geom,
    Props,
    /// Follow-up review 2026-08-15: prima del fix ogni Feature riconosceva
    /// solo `geometry` e `properties`, ignorando il proprio `type`. La
    /// specifica `GeoJSON` (RFC 7946 §3.2) rende `type` obbligatorio per
    /// ogni oggetto Feature. Distinguerlo qui permette al chiamante di
    /// controllarne il valore e rifiutare Feature con `type` diverso o
    /// mancante.
    Type,
    Other,
}
struct FeatKeySeed;
impl<'de> DeserializeSeed<'de> for FeatKeySeed {
    type Value = FeatKey;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<FeatKey, D::Error> {
        d.deserialize_str(self)
    }
}
impl Visitor<'_> for FeatKeySeed {
    type Value = FeatKey;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("una chiave di Feature")
    }
    fn visit_str<E>(self, s: &str) -> std::result::Result<FeatKey, E> {
        Ok(match s {
            "geometry" => FeatKey::Geom,
            "properties" => FeatKey::Props,
            "type" => FeatKey::Type,
            _ => FeatKey::Other,
        })
    }
}

/// Dal membro `geometry` legge **il solo `type`**, e ignora il resto.
///
/// Le coordinate restano `IgnoredAny`: la passata di inferenza e' O(1) in
/// memoria ed e' bene che resti tale. Il nome del tipo e' una parola sola per
/// feature, e non viene trattenuto -- entra in un insieme di al piu' sette
/// varianti.
///
/// `geometry` puo' essere `null`, ed e' un caso normale: una feature senza
/// geometria non aggiunge tipi e non e' un errore.
struct TipoGeometricoSeed<'a> {
    accs: &'a mut SchemaAccumulators,
}

impl<'de> DeserializeSeed<'de> for TipoGeometricoSeed<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for TipoGeometricoSeed<'_> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("una geometria GeoJSON, oppure null")
    }

    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_none<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_any(self)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        while let Some(chiave) = map.next_key::<String>()? {
            if chiave == "type" {
                let nome: String = map.next_value()?;
                // Un nome che non e' un tipo GeoJSON non entra: questa e' una
                // scansione descrittiva, e a rifiutarlo e' la lettura vera con
                // la propria diagnostica di riga.
                if let Some(tipo) = GeometryType::from_canonical_name(&nome.to_ascii_lowercase()) {
                    self.accs.tipi_geometrici.insert(tipo);
                }
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }
}

struct PropsSink<'a> {
    sink: &'a mut RowSink,
}
impl<'de> DeserializeSeed<'de> for PropsSink<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        // properties può essere null (→ tutte le colonne restano non-viste = null).
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for PropsSink<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("le properties di un Feature (oggetto o null)")
    }
    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_any(self)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        while let Some(hit) = map.next_key_seed(PropKeySeed {
            col_idx: &self.sink.col_idx,
            property_idx: &self.sink.property_idx,
        })? {
            if self.sink.property_seen[hit.property_idx] {
                return Err(self.sink.ferma(err(&PublicMessage::Curated(
                    "chiave duplicata nelle properties GeoJSON",
                ))));
            }
            self.sink.property_seen[hit.property_idx] = true;
            match hit.projected_idx {
                // Chiave nota e non ancora vista in questa feature: append.
                Some(idx) => {
                    map.next_value_seed(ValueSink {
                        b: &mut self.sink.builders[idx],
                        errore: &mut self.sink.errore,
                    })?;
                    self.sink.seen[idx] = true;
                }
                // Una chiave fuori projection resta intenzionalmente non letta.
                None => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

struct PropHit {
    projected_idx: Option<usize>,
    property_idx: usize,
}

/// Chiave di proprietà → indice di colonna (o None se non è nello schema).
/// La stringa non viene allocata: la lookup avviene dentro `visit_str`.
struct PropKeySeed<'a> {
    col_idx: &'a HashMap<String, usize>,
    property_idx: &'a HashMap<String, usize>,
}
impl<'de> DeserializeSeed<'de> for PropKeySeed<'_> {
    type Value = PropHit;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<PropHit, D::Error> {
        d.deserialize_str(self)
    }
}
impl Visitor<'_> for PropKeySeed<'_> {
    type Value = PropHit;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("una chiave di proprietà")
    }
    fn visit_str<E: DeError>(self, s: &str) -> std::result::Result<PropHit, E> {
        let property_idx = self
            .property_idx
            .get(s)
            .copied()
            .ok_or_else(|| E::custom("property GeoJSON assente dallo schema inferito"))?;
        Ok(PropHit {
            projected_idx: self.col_idx.get(s).copied(),
            property_idx,
        })
    }
}

/// Appende il valore di una proprietà DIRETTAMENTE nel builder tipizzato,
/// senza materializzare un `serde_json::Value` per gli scalari (il caso caldo).
struct ValueSink<'a> {
    b: &'a mut InferredColumnBuilder,
    errore: &'a mut Option<PlenoraIoError>,
}
impl<'de> DeserializeSeed<'de> for ValueSink<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for ValueSink<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un valore di proprietà")
    }
    fn visit_i64<E: DeError>(self, v: i64) -> std::result::Result<(), E> {
        self.b.append_i64(v).map_err(E::custom)
    }
    fn visit_u64<E: DeError>(self, v: u64) -> std::result::Result<(), E> {
        self.b.append_u64(v).map_err(E::custom)
    }
    fn visit_f64<E: DeError>(self, v: f64) -> std::result::Result<(), E> {
        self.b.append_f64(v).map_err(E::custom)
    }
    fn visit_bool<E: DeError>(self, v: bool) -> std::result::Result<(), E> {
        self.b.append_bool(v).map_err(E::custom)
    }
    fn visit_str<E: DeError>(self, s: &str) -> std::result::Result<(), E> {
        self.b.append_str(s).map_err(E::custom) // caso caldo: 0 alloc extra
    }
    fn visit_none<E>(self) -> std::result::Result<(), E> {
        self.b.append_null();
        Ok(())
    }
    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        self.b.append_null();
        Ok(())
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_any(self)
    }
    // Valori composti (array/oggetto) in colonna Text: rari → via DOM + stringa,
    // esattamente come il percorso `append_json` condiviso.
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> std::result::Result<(), A::Error> {
        let v = JsonValue::deserialize(SeqAccessDeserializer::new(seq))?;
        // L'errore di `append_json` e' gia' nostro, con il suo codice:
        // passa intero invece di essere appiattito nel testo di serde.
        match self.b.append_json(Some(&v)) {
            Ok(()) => Ok(()),
            Err(errore) => Err(ferma_in(self.errore, errore)),
        }
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> std::result::Result<(), A::Error> {
        let v = JsonValue::deserialize(MapAccessDeserializer::new(map))?;
        // L'errore di `append_json` e' gia' nostro, con il suo codice:
        // passa intero invece di essere appiattito nel testo di serde.
        match self.b.append_json(Some(&v)) {
            Ok(()) => Ok(()),
            Err(errore) => Err(ferma_in(self.errore, errore)),
        }
    }
}

fn finish_batch(
    schema: &SchemaRef,
    geom: &mut Option<BinaryBuilder>,
    builders: &mut [InferredColumnBuilder],
    row_count: usize,
) -> Result<RecordBatch> {
    let mut arrays: Vec<ArrayRef> =
        Vec::with_capacity(usize::from(geom.is_some()) + builders.len());
    if let Some(builder) = geom {
        arrays.push(Arc::new(builder.finish()));
    }
    for b in builders.iter_mut() {
        arrays.push(b.finish());
    }
    let options = RecordBatchOptions::new().with_row_count(Some(row_count));
    RecordBatch::try_new_with_options(schema.clone(), arrays, &options).map_err(|_| {
        err(&PublicMessage::Curated(
            "costruzione del RecordBatch fallita",
        ))
    })
}

/// Entry point per il fuzzer (NON API stabile): esegue pass-1 + pass-2 in modo
/// **sincrono** su `bytes`, con un unico batch finale. JSON invalido o colonne
/// disallineate producono `Err`, senza panic.
#[doc(hidden)]
pub fn __fuzz_read_geojson(bytes: &[u8]) -> std::result::Result<usize, String> {
    use std::io::Cursor;
    // pass-1: schema
    // Quote della campagna: tetto per cella e per feature stretti apposta,
    // cosi' un input ostile non allochi oltre il budget di libFuzzer.
    // I tre tetti sono **dichiarati**, non ereditati dal default: una campagna
    // dichiara il proprio budget, e un default qui direbbe che la campagna gira
    // con le quote della produzione mentre gira con un tetto per cella cento
    // volte piu' stretto.
    //
    // La profondita' e' 32 e non 64, e la ragione e' misurata: `serde_json` ha
    // un limite di ricorsione suo, 128 livelli JSON, e ogni livello GeoJSON ne
    // costa due -- l'oggetto e la sua lista. Oltre i sessantadue livelli e'
    // **lui** a rifiutare, quindi con il tetto di produzione il nostro non
    // morderebbe mai e la campagna non lo eserciterebbe. A 32 morde per primo,
    // ed e' cio' che un input ostile deve incontrare.
    let quote = QuoteInferenza {
        cella: WkbLimits {
            max_cell_bytes: 1_048_576,
            max_components: 100_000,
            max_depth: 32,
        },
        feature: 100_000,
    };
    let mut accs = SchemaAccumulators {
        max_feature: quote.feature,
        ..SchemaAccumulators::default()
    };
    serde_json::Deserializer::from_reader(Cursor::new(bytes))
        .deserialize_map(TopVisitor { accs: &mut accs })
        .map_err(|e| e.to_string())?;
    let cols = accs.into_columns().map_err(str::to_owned)?;
    let mut fields = vec![geometry_field(GEOMETRY, OGC_CRS84)];
    for (k, ct) in &cols {
        fields.push(Field::new(k, ct.arrow_data_type(), true));
    }
    let schema: SchemaRef = Arc::new(Schema::new(fields));
    // pass-2 sincrono: batch_size enorme → nessun flush intermedio sul canale.
    let ncols = cols.len();
    let col_idx: HashMap<String, usize> = cols
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (k.clone(), i))
        .collect();
    let mut sink = RowSink {
        schema: schema.clone(),
        property_idx: col_idx.clone(),
        col_idx,
        output: RowOutput::Discard,
        geom: Some(BinaryBuilder::new()),
        wkb_buf: Vec::new(),
        limiti_wkb: quote.cella,
        builders: cols
            .iter()
            .map(|(_, column_type)| InferredColumnBuilder::new(*column_type))
            .collect(),
        seen: vec![false; ncols],
        property_seen: vec![false; ncols],
        n: 0,
        source_rows_seen: 0,
        in_feature: false,
        batch_sizer: plenora_io_core::AdaptiveBatchSizer::new(
            schema.as_ref(),
            plenora_io_core::BatchTarget {
                target_bytes: usize::MAX,
                max_rows: usize::MAX,
            },
        ),
        aborted: false,
        errore: None,
    };
    serde_json::Deserializer::from_reader(Cursor::new(bytes))
        .deserialize_map(TopSink { sink: &mut sink })
        .map_err(|e| e.to_string())?;
    if sink.n > 0 {
        // L'entry point del fuzzer parla `String` per comodita' del
        // fuzzer, non del bordo: qui la conversione e' verso l'esterno,
        // non verso un errore pubblico.
        let batch = finish_batch(&schema, &mut sink.geom, &mut sink.builders, sink.n)
            .map_err(|errore| errore.to_string())?;
        return Ok(batch.num_rows());
    }
    Ok(0)
}

// --- scrittura (bufferizzante nella v1) -----------------------------------

struct GeoJsonWriter {
    staging: StagedFile,
    writer: Option<BufWriter<File>>,
    first: bool,
    wkb_limits: WkbLimits,
}

impl FormatWriter for GeoJsonWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let schema = batch.schema();
        let geom_idx = geometry_index(&schema).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "nessuna colonna geometria geoarrow.wkb",
            ))
        })?;
        let geom_col = batch
            .column(geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err(&PublicMessage::Curated("colonna geometria non binaria")))?;
        let limits = self.wkb_limits;
        let w = self
            .writer
            .as_mut()
            .ok_or_else(|| err(&PublicMessage::Curated("writer chiuso")))?;
        let mut first = self.first;
        for row in 0..batch.num_rows() {
            if !first {
                w.write_all(b",")?;
            }
            first = false;
            write_feature(w, &schema, geom_idx, geom_col, batch, row, &limits)?;
        }
        self.first = first;
        Ok(())
    }
    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut w = self
            .writer
            .take()
            .ok_or_else(|| err(&PublicMessage::Curated("writer già chiuso")))?;
        w.write_all(b"]}")?;
        w.flush()?;
        drop(w);
        let (bytes, outcome) = self.staging.publish()?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

/// Serializza un Feature DIRETTAMENTE nel writer, senza costruire il DOM
/// `Feature`/`JsonObject`: le chiavi/valori scalari vanno dritti nel buffer
/// (0 alloc), solo la geometria passa ancora da un `geojson::Value`.
fn write_feature<W: Write>(
    w: &mut W,
    schema: &SchemaRef,
    geom_idx: usize,
    geom_col: &BinaryArray,
    batch: &RecordBatch,
    row: usize,
    limits: &WkbLimits,
) -> Result<()> {
    w.write_all(b"{\"type\":\"Feature\",\"geometry\":")?;
    if geom_col.is_null(row) {
        w.write_all(b"null")?;
    } else {
        let geom = decode_wkb(geom_col.value(row), limits)?;
        geometry::write_wkb_geojson(w, &geom)?;
    }
    w.write_all(b",\"properties\":{")?;
    let mut first_prop = true;
    for (i, field) in schema.fields().iter().enumerate() {
        if i == geom_idx {
            continue;
        }
        if !first_prop {
            w.write_all(b",")?;
        }
        first_prop = false;
        // `to_writer` di uno `&str` scrive la chiave JSON quotata/escapata direct.
        serde_json::to_writer(&mut *w, field.name())
            .map_err(|_| err(&PublicMessage::Curated("serializzazione JSON fallita")))?;
        w.write_all(b":")?;
        write_json_value(w, batch.column(i), row)?;
    }
    w.write_all(b"}}")?;
    Ok(())
}

/// Valore di proprietà scritto DIRETTAMENTE: scalari senza `serde_json::Value`,
/// stringhe escapate via `to_writer(&str)`, tipi non comuni via fallback.
fn write_json_value<W: Write>(w: &mut W, col: &ArrayRef, row: usize) -> Result<()> {
    if col.is_null(row) {
        w.write_all(b"null")?;
        return Ok(());
    }
    if let Some(a) = col.as_any().downcast_ref::<StringArray>() {
        serde_json::to_writer(&mut *w, a.value(row))
            .map_err(|_| err(&PublicMessage::Curated("serializzazione JSON fallita")))?;
    } else if let Some(a) = col.as_any().downcast_ref::<Int64Array>() {
        write!(w, "{}", a.value(row))?;
    } else if let Some(a) = col.as_any().downcast_ref::<Float64Array>() {
        let v = a.value(row);
        if v.is_finite() {
            // serde_json (ryu): round-trippabile anche per f64 estremi.
            serde_json::to_writer(&mut *w, &v)
                .map_err(|_| err(&PublicMessage::Curated("serializzazione JSON fallita")))?;
        } else {
            return Err(err(&PublicMessage::Curated(
                "Float64 non finito non rappresentabile in GeoJSON",
            )));
        }
    } else if let Some(a) = col.as_any().downcast_ref::<BooleanArray>() {
        w.write_all(if a.value(row) { b"true" } else { b"false" })?;
    } else {
        serde_json::to_writer(&mut *w, &json_from_array(col, row)?)
            .map_err(|_| err(&PublicMessage::Curated("serializzazione JSON fallita")))?;
    }
    Ok(())
}

// Solo per i test: parse completo (documenti piccoli).
#[cfg(test)]
fn parse_features(text: &str) -> Result<Vec<geojson::Feature>> {
    let gj: geojson::GeoJson = text
        .parse()
        .map_err(|_| err(&PublicMessage::Curated("GeoJSON non valido")))?;
    match gj {
        geojson::GeoJson::FeatureCollection(c) => Ok(c.features),
        geojson::GeoJson::Feature(f) => Ok(vec![f]),
        geojson::GeoJson::Geometry(_) => Err(err(&PublicMessage::Curated(
            "atteso Feature/FeatureCollection",
        ))),
    }
}

#[cfg(test)]
mod tests;

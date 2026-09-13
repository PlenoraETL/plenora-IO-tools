//! driver-csv — CSV ⇄ `RecordBatch`. La geometria è dichiarata via
//! `format_options`: `x_column`+`y_column` (Point XY) oppure `wkt_column`
//! (WKT XY/XYZ/XYM/XYZM). CSV non porta CRS: `assume_crs` è obbligatorio
//! (`PRODUCT.md § CRS`).
//!
//! Lettura **streaming** (Fase 2A): righe scorse via `csv::StringRecord` riusato
//! (i campi sono `&str`, niente String per cella). Due passate: pass-1 (`open`)
//! inferisce i tipi colonna a RAM O(1) sondando le celle (nessuna allocazione);
//! pass-2 è un thread che produce `RecordBatch` da `batch_target` righe via canale
//! con backpressure → memoria O(batch). Geometria diretta a WKB, attributi in
//! builder tipizzati (niente intermedio `serde_json::Value`). Scrittura streaming
//! per righe (niente buffering dei batch).
#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::builder::BinaryBuilder;
use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, RecordBatch,
    RecordBatchOptions, StringArray,
};
use arrow_schema::{Field, Schema, SchemaRef};
use serde_json::Value as JsonValue;

use driver_common::wkt_lossless::{format_wkt_into, parse_wkt_bounded};
use driver_common::{
    classify_i64, geometry_field, geometry_index, json_from_array, ColType, InferredColumnBuilder,
    ObservedValueClass, TypeAccumulator,
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
    validate_write, with_write_validation, AttributeWriteSupport, CrsRepresentationCapabilities,
    CrsRepresentationState, CrsWriteSupport, FormatWriteCapabilities, NullabilitySupport,
    SinkPathConstraint, TypeCoercionPolicy, WritePlan, SCALAR_TYPES, UTF8_FIELD_NAMES,
    WKB_PASSTHROUGH_GEOMETRY,
};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::{CrsKind, ResolvedCrs};
#[cfg(test)]
use plenora_io_model::geometry::is_geometry_field;
use plenora_io_model::geometry::with_geometry_contract_metadata;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{
    decode_wkb, encode_wkb_into_bounded, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
};
use plenora_io_model::{NumeroStrutturale, PlenoraIoError, PublicMessage, Result};

const GEOMETRY: &str = "geometry";

fn err(reason: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("csv", reason)
}

use plenora_io_model::format_options::{
    FaseOpzione, OpzioneFormato, SchemaOpzioniFormato, ValoreAmmesso,
};

/// Le `format_options` interpretate dal driver CSV (L0.7, S6).
///
/// La geometria in lettura si dichiara con `wkt_column`, oppure con la coppia
/// `x_column`/`y_column`: lo schema elenca le chiavi, il vincolo che siano
/// mutuamente esclusive resta nel driver perche' e' una relazione fra chiavi,
/// non una proprieta' di un valore.
const SCHEMA_OPZIONI: SchemaOpzioniFormato = SchemaOpzioniFormato::nuovo(&[
    OpzioneFormato {
        chiave: "delimiter",
        fase: FaseOpzione::Entrambe,
        valore: ValoreAmmesso::Carattere,
        predefinito: Some(","),
        descrizione: "separatore di campo, esattamente un carattere ASCII",
    },
    OpzioneFormato {
        chiave: "geometry_encoding",
        fase: FaseOpzione::Scrittura,
        valore: ValoreAmmesso::Enumerato(&["wkt", "xy"]),
        predefinito: Some("wkt"),
        descrizione: "come scrivere la geometria: colonna WKT o colonne x/y",
    },
    OpzioneFormato {
        chiave: "wkt_column",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "colonna che contiene la geometria in WKT",
    },
    OpzioneFormato {
        chiave: "x_column",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "colonna dell'ascissa, da usare insieme a y_column",
    },
    OpzioneFormato {
        chiave: "y_column",
        fase: FaseOpzione::Lettura,
        valore: ValoreAmmesso::Testo,
        predefinito: None,
        descrizione: "colonna dell'ordinata, da usare insieme a x_column",
    },
]);

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "csv",
    Direction::Bidirectional,
    ReadMode::StreamingSequential,
    // INV-7: `read_record` riga per riga, canale a profondita' 2.
    plenora_io_core::NativeReadMode::StreamingSequential,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Streaming),
    Some(plenora_io_core::DeterminismLevel::Semantic),
    false,
    false,
    ReaderConcurrency::MultipleIndependentReaders,
    plenora_io_core::ProjectionSupport::Exact,
    plenora_io_core::PredicatePruningSupport::None,
    plenora_io_core::SpatialPruningSupport::None,
    CrsHandling::None,
    Fidelity::Conditional,
    Runtime::PureRust,
    // `hostile_input_hardened`: le celle WKT passano da `parse_wkt_bounded`, che applica byte,
    // componenti e profondita' mentre consuma il testo (S12).
    true,
    // `spec_version_supported`: il formato non si versiona in un modo che
    // il driver possa dichiarare per intero.
    None,
    Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::ExplicitText,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_PASSTHROUGH_GEOMETRY,
        crs: CrsWriteSupport::None,
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Absent,
            CrsRepresentationState::Absent,
            CrsRepresentationState::Absent,
        ),
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
        sink_path: SinkPathConstraint::Free,
    }),
    SCHEMA_OPZIONI,
    &["csv"],
    1,
    6,
    10,
);

pub struct CsvDriver;

/// Il separatore dichiarato, o la virgola se non e' dichiarato.
///
/// Prima prendeva il **primo byte** di qualunque stringa: `delimiter=";;"`
/// diventava `;` e `delimiter=""` diventava `,`, entrambi in silenzio. Lo
/// schema ammette ora esattamente un carattere ASCII, e questa funzione
/// restituisce quel carattere o niente — non un byte pescato da una stringa
/// piu' lunga. Il `None` finale e' irraggiungibile dopo la validazione, e
/// resta perche' la funzione non deve dipendere dall'averla eseguita.
fn delimiter(opts: &std::collections::BTreeMap<String, String>) -> Option<u8> {
    let Some(dichiarato) = opts.get("delimiter") else {
        return Some(b',');
    };
    let mut byte = dichiarato.bytes();
    match (byte.next(), byte.next()) {
        (Some(uno), None) if uno.is_ascii() => Some(uno),
        _ => None,
    }
}

/// Risolve le colonne geometriche dichiarate nelle opzioni contro
/// l'intestazione del file.
///
/// I nomi delle colonne **non entrano nei messaggi**. Sono valori d'opzione, e
/// l'unico testo runtime che S9 ammette e' il token bounded coniato dal
/// validatore centrale: qui non c'e', perche' `wkt_column`, `x_column` e
/// `y_column` sono dichiarate `ValoreAmmesso::Testo` — lo schema le accetta, e
/// il rifiuto nasce dopo, dal confronto con l'intestazione di **questo** file.
///
/// E' una perdita diagnostica reale, e il report di perdita la porta.
fn colonne_geometriche(
    headers: &[String],
    format_options: &std::collections::BTreeMap<String, String>,
) -> Result<(GeomSpec, HashSet<usize>)> {
    let idx = |name: &str| headers.iter().position(|h| h == name);
    if let Some(w) = format_options.get("wkt_column") {
        let wi = idx(w).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "colonna WKT assente dall'intestazione",
            ))
        })?;
        return Ok((GeomSpec::Wkt(wi), HashSet::from([wi])));
    }
    if let (Some(x), Some(y)) = (
        format_options.get("x_column"),
        format_options.get("y_column"),
    ) {
        let xi = idx(x).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "colonna X assente dall'intestazione",
            ))
        })?;
        let yi = idx(y).ok_or_else(|| {
            err(&PublicMessage::Curated(
                "colonna Y assente dall'intestazione",
            ))
        })?;
        return Ok((GeomSpec::Xy(xi, yi), HashSet::from([xi, yi])));
    }
    Err(err(&PublicMessage::Curated(
        "specificare wkt_column, oppure x_column con y_column, in format_options",
    )))
}

/// Errore per un separatore che la validazione avrebbe dovuto respingere.
fn delimiter_non_valido() -> PlenoraIoError {
    PlenoraIoError::redatto(
        plenora_io_model::IoErrorCode::Generic,
        plenora_io_model::ErrorCategory::InvalidConfiguration,
        plenora_io_model::ErrorPhase::Validate,
        plenora_io_model::RemoteEffect::None,
        plenora_io_model::RetryDisposition::Never,
        &PublicMessage::Curated("csv: delimiter deve essere esattamente un carattere ASCII"),
    )
}

fn csv_reader(path: &Path, delim: u8) -> Result<csv::Reader<File>> {
    csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(true) // salta l'intestazione automaticamente
        .flexible(false)
        .from_path(path)
        .map_err(|_| err(&PublicMessage::Curated("apertura del CSV fallita")))
}

#[derive(Clone, Copy)]
enum GeomSpec {
    Wkt(usize),
    Xy(usize, usize),
}

impl FormatDriver for CsvDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = plenora_io_core::preflight_source(self.descriptor(), source, &mut opts)?;
        let delim = delimiter(&opts.format_options).ok_or_else(delimiter_non_valido)?;
        let crs = opts.assume_crs.clone().ok_or_else(|| {
            PlenoraIoError::crs_redatto(&PublicMessage::Curated(
                "CSV con geometria richiede --assume-crs",
            ))
        })?;

        // Intestazione (nomi colonna).
        let headers: Vec<String> = {
            let mut rdr = csv::ReaderBuilder::new()
                .delimiter(delim)
                .has_headers(false)
                .flexible(true)
                .from_path(&path)
                .map_err(|_| err(&PublicMessage::Curated("apertura del CSV fallita")))?;
            let mut first = csv::StringRecord::new();
            if !rdr
                .read_record(&mut first)
                .map_err(|_| err(&PublicMessage::Curated("CSV non valido")))?
            {
                return Err(err(&PublicMessage::Curated("CSV vuoto")));
            }
            first.iter().map(str::to_owned).collect()
        };

        let (geom, geom_cols) = colonne_geometriche(&headers, &opts.format_options)?;

        // Pass 1: inferenza tipi (RAM O(ncol), nessuna String per cella).
        let quote = QuoteInferenza::from_read_options(&opts);
        let attrs = infer_types(&path, delim, &headers, &geom_cols, quote)?;

        let (dimensions, geometry_types) = match geom {
            GeomSpec::Wkt(wi) => infer_wkt_geometry(&path, delim, wi, quote)?,
            GeomSpec::Xy(_, _) => (CoordinateDimensions::Xy, vec![GeometryType::Point]),
        };
        let kind = if crs == "OGC:CRS84" || crs == "EPSG:4326" {
            CrsKind::Geographic
        } else {
            CrsKind::Unknown
        };
        let mut geometry_contract = GeometryColumnContract::wkb_xy(
            FieldId(0),
            GEOMETRY,
            ResolvedCrs::new(Some(crs.clone()), kind, None),
            true,
        );
        geometry_contract.dimensions = dimensions;
        geometry_contract.set_exact_geometry_types(geometry_types);
        let native_encoding = match geom {
            GeomSpec::Wkt(_) => "wkt",
            GeomSpec::Xy(_, _) => "xy_columns",
        };
        geometry_contract.native_metadata.insert(
            "csv.geometry_encoding".to_owned(),
            native_encoding.to_owned(),
        );
        let mut fields = vec![with_geometry_contract_metadata(
            &geometry_field(GEOMETRY, &crs),
            &geometry_contract,
        )];
        for (ci, ct) in &attrs {
            fields.push(Field::new(&headers[*ci], ct.arrow_data_type(), true));
        }
        let schema: SchemaRef = Arc::new(Schema::new(fields));
        let contract = DataContract::new(schema, Some(geometry_contract));
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(plenora_io_core::with_read_budget(
            Box::new(CsvDataset {
                path,
                delim,
                geom,
                wkb_limits: opts.wkb_limits(),
                attrs,
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
                &PublicMessage::Curated("CSV: un solo layer per file"),
            ));
        }
        // Prima: `matches!(..., Some("xy"))`, cioe' qualunque valore diverso
        // da "xy" — compreso "XY", "WKB" o un refuso — significava WKT senza
        // dirlo. Ora le due grafie ammesse sono trattate come due casi, e
        // l'assenza vale il default dichiarato nello schema.
        let xy = match opts
            .format_options
            .get("geometry_encoding")
            .map(String::as_str)
        {
            Some("xy") => true,
            None | Some("wkt") => false,
            Some(_) => {
                // Il valore non esce da qui. Lo schema dichiara
                // `geometry_encoding` come `Enumerato(&["wkt", "xy"])`, quindi
                // un valore diverso e' gia' stato respinto da `valida_opzioni`
                // con il suo token bounded: questo ramo e' difensivo, e il
                // token nasce solo nel validatore centrale.
                return Err(PlenoraIoError::redatto(
                    plenora_io_model::IoErrorCode::Generic,
                    plenora_io_model::ErrorCategory::InvalidConfiguration,
                    plenora_io_model::ErrorPhase::Validate,
                    plenora_io_model::RemoteEffect::None,
                    plenora_io_model::RetryDisposition::Never,
                    &PublicMessage::Curated("csv: geometry_encoding non riconosciuto"),
                ));
            }
        };
        let staging = StagedFile::new(&path, opts.durable, opts.max_output_bytes())?;
        let writer = csv::WriterBuilder::new()
            .delimiter(delimiter(&opts.format_options).ok_or_else(delimiter_non_valido)?)
            .from_writer(staging.reopen()?);
        with_write_validation(
            Box::new(CsvWriter {
                staging,
                writer: Some(writer),
                xy,
                header_written: false,
                wkb_limits: opts.wkb_limits(),
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

// --- lettura streaming -----------------------------------------------------

struct CsvDataset {
    path: PathBuf,
    delim: u8,
    geom: GeomSpec,
    /// Le quote WKB configurate, non i default del contratto: il dataset
    /// sopravvive alle opzioni, e senza portarsele dietro la lettura tornerebbe
    /// al default proprio dopo che l'inferenza ha rispettato il flag.
    wkb_limits: WkbLimits,
    attrs: Vec<(usize, ColType)>,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for CsvDataset {
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
        let attrs = indices
            .iter()
            .filter_map(|&index| {
                index
                    .checked_sub(1)
                    .and_then(|attr_index| self.attrs.get(attr_index))
                    .copied()
            })
            .collect();
        let batch_sizer = plenora_io_core::AdaptiveBatchSizer::new(
            layer.contract.schema.as_ref(),
            request.batch_target,
        );
        let reader = spawn_parser(
            SorgenteCsv {
                path: self.path.clone(),
                delim: self.delim,
                cella_wkt: self.wkb_limits,
            },
            include_geometry.then_some(self.geom),
            attrs,
            layer.contract.schema.clone(),
            batch_sizer,
            layer,
        )?;
        Ok(plenora_io_core::with_cancellation(
            reader,
            request.cancellation.clone(),
        ))
    }
}

/// Classe di una cella per l'inferenza, prima della promozione condivisa.
fn classify(cell: &str) -> ObservedValueClass {
    let t = cell.trim();
    if t.is_empty() {
        return ObservedValueClass::Null;
    }
    if let Ok(value) = t.parse::<i64>() {
        return classify_i64(value);
    }
    // Un intero sintattico fuori da i64 resta testo: passare prima da f64
    // ne altererebbe le cifre meno significative.
    if t.parse::<i128>().is_ok() || t.parse::<u128>().is_ok() {
        return ObservedValueClass::Text;
    }
    if t.parse::<f64>().is_ok_and(f64::is_finite) {
        return ObservedValueClass::Number;
    }
    if t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false") {
        return ObservedValueClass::Boolean;
    }
    ObservedValueClass::Text
}

/// Le quote che le due passate di inferenza consultano.
///
/// Un config privato tipizzato e non i `PipelineLimits` interi: sono le sole
/// due che servono, e passarli tutti darebbe all'inferenza accesso a quote che
/// non le competono — la memoria, per esempio, che governa il batch e non il
/// parsing di una cella.
#[derive(Clone, Copy)]
struct QuoteInferenza {
    /// I tetti del bordo per una cella WKT.
    ///
    /// Era il solo tetto in byte. Da S12 l'analisi e' progressiva e applica
    /// anche componenti e profondita' **durante** il parse: il tipo che
    /// viaggia e' quindi il contratto intero, non una sua meta'.
    cella_wkt: WkbLimits,
    /// Tetto sulle righe visitate dalle passate di inferenza.
    ///
    /// Non e' `max_input_entries`: quella quota governa l'enumerazione della
    /// **sorgente** e il preflight l'ha gia' applicata al file. Applicarla di
    /// nuovo ai record sarebbe la stessa quota contata due volte, e al valore
    /// predefinito — decine di migliaia, calibrato sui file di una directory —
    /// rifiuterebbe un CSV di dimensioni ordinarie.
    righe: usize,
}

impl QuoteInferenza {
    fn from_read_options(opts: &ReadOptions) -> Self {
        Self {
            cella_wkt: opts.wkb_limits(),
            righe: opts.max_rows(),
        }
    }
}

/// Errore di una passata di inferenza che supera il tetto di righe.
fn oltre_le_righe(righe: usize) -> PlenoraIoError {
    PlenoraIoError::limite_redatto(&PublicMessage::CuratedWith(
        "l'inferenza si e' fermata al tetto di righe:",
        NumeroStrutturale::Limite(driver_common::saturating_u64(righe)),
    ))
}

fn infer_types(
    path: &Path,
    delim: u8,
    headers: &[String],
    geom_cols: &HashSet<usize>,
    quote: QuoteInferenza,
) -> Result<Vec<(usize, ColType)>> {
    let attr_idx: Vec<usize> = (0..headers.len())
        .filter(|i| !geom_cols.contains(i))
        .collect();
    let mut accs = vec![TypeAccumulator::default(); attr_idx.len()];
    let mut rdr = csv_reader(path, delim)?;
    let mut rec = csv::StringRecord::new();
    let mut visitate = 0_usize;
    while rdr
        .read_record(&mut rec)
        .map_err(|_| err(&PublicMessage::Curated("riga CSV non valida")))?
    {
        // Prima di osservare le celle: un CSV ostile con miliardi di record
        // rende la passata di inferenza illimitata nel tempo anche se ogni
        // cella e' minuscola, e nessun contatore di riga e' ancora entrato in
        // gioco perche' il reader non ha iniziato.
        visitate = visitate.saturating_add(1);
        if visitate > quote.righe {
            return Err(oltre_le_righe(quote.righe));
        }
        for (j, &ci) in attr_idx.iter().enumerate() {
            accs[j].observe(classify(required_cell(&rec, ci)?));
        }
    }
    Ok(attr_idx
        .into_iter()
        .zip(accs)
        .map(|(ci, accumulator)| (ci, accumulator.column_type()))
        .collect())
}

fn infer_wkt_geometry(
    path: &Path,
    delim: u8,
    wkt_index: usize,
    quote: QuoteInferenza,
) -> Result<(CoordinateDimensions, Vec<GeometryType>)> {
    let mut reader = csv_reader(path, delim)?;
    let mut record = csv::StringRecord::new();
    let mut dimensions = BTreeSet::new();
    let mut geometry_types = BTreeSet::new();
    let mut visitate = 0_usize;
    while reader
        .read_record(&mut record)
        .map_err(|_| err(&PublicMessage::Curated("riga CSV non valida")))?
    {
        // Il tetto sulle righe si applica **prima** di leggere la cella:
        // fermarsi dopo averla parsata vorrebbe dire aver gia' allocato cio'
        // che il limite doveva impedire.
        visitate = visitate.saturating_add(1);
        if visitate > quote.righe {
            return Err(oltre_le_righe(quote.righe));
        }
        let text = required_cell(&record, wkt_index)?.trim();
        if text.is_empty() {
            continue;
        }
        // Il tetto e' quello **configurato** dal chiamante, non il default
        // del contratto: chi stringe `--max-wkb-cell-bytes` deve vedere il
        // rifiuto anche qui, dove l'AST wkt verrebbe allocato. Fino a S5
        // questa riga usava il default, quindi il flag non arrivava
        // all'inferenza e una cella oltre la soglia configurata veniva
        // parsata comunque.
        let geometry = parse_wkt_bounded(text, &quote.cella_wkt)?;
        dimensions.insert(geometry.dimensions);
        geometry_types.insert(geometry.geometry_type());
    }
    let dimensions = if dimensions.len() == 1 {
        dimensions
            .iter()
            .next()
            .copied()
            .unwrap_or(CoordinateDimensions::Unknown)
    } else {
        CoordinateDimensions::Unknown
    };
    Ok((dimensions, geometry_types.into_iter().collect()))
}

/// La sorgente e le quote che il parser consulta, in un solo argomento.
///
/// Raggrupparle non e' cosmetica: sono tre valori che viaggiano sempre
/// insieme e che nessun chiamante deve poter accoppiare male — un delimitatore
/// di un file con il tetto di un altro sarebbe un difetto silenzioso.
struct SorgenteCsv {
    path: PathBuf,
    delim: u8,
    cella_wkt: WkbLimits,
}

fn spawn_parser(
    sorgente: SorgenteCsv,
    geom: Option<GeomSpec>,
    attrs: Vec<(usize, ColType)>,
    schema: SchemaRef,
    mut batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    layer: LayerContract,
) -> Result<Box<dyn LayerReader>> {
    let SorgenteCsv {
        path,
        delim,
        cella_wkt,
    } = sorgente;
    spawn_batch_reader(DESCRIPTOR.id(), layer, 2, move |emitter: BatchEmitter| {
        let mut rdr = csv_reader(&path, delim)?;
        let mut rec = csv::StringRecord::new();
        let mut geom_b = geom.map(|_| BinaryBuilder::new());
        let mut wkb_buf: Vec<u8> = Vec::new(); // riusato per riga: 0 alloc WKB nel loop
        let mut builders: Vec<InferredColumnBuilder> = attrs
            .iter()
            .map(|(_, column_type)| InferredColumnBuilder::new(*column_type))
            .collect();
        let mut n = 0usize;
        loop {
            let more = rdr
                .read_record(&mut rec)
                .map_err(|_| err(&PublicMessage::Curated("riga CSV non valida")))?;
            if !more {
                break;
            }
            if let (Some(builder), Some(spec)) = (&mut geom_b, geom) {
                append_geometry(builder, spec, &rec, &mut wkb_buf, cella_wkt)?;
            }
            for (k, (ci, _)) in attrs.iter().enumerate() {
                builders[k].append_csv_cell(required_cell(&rec, *ci)?)?;
            }
            n += 1;
            if n >= batch_sizer.rows() {
                let batch = finish_batch(&schema, &mut geom_b, &mut builders, n)?;
                batch_sizer.observe(&batch);
                if !emitter.send(batch) {
                    return Ok(());
                }
                n = 0;
            }
        }
        if n > 0 {
            let batch = finish_batch(&schema, &mut geom_b, &mut builders, n)?;
            if !emitter.send(batch) {
                return Ok(());
            }
        }
        Ok(())
    })
}

fn append_geometry(
    geom_b: &mut BinaryBuilder,
    geom: GeomSpec,
    rec: &csv::StringRecord,
    buf: &mut Vec<u8>,
    cella_wkt: WkbLimits,
) -> Result<()> {
    match geom {
        GeomSpec::Wkt(wi) => {
            let cell = required_cell(rec, wi)?.trim();
            if cell.is_empty() {
                geom_b.append_null();
            } else {
                // Stessa quota dell'inferenza, non il default: un flag che
                // vale in una passata e non nell'altra sarebbe peggio che non
                // averlo, perche' il rifiuto arriverebbe a meta' lettura.
                let geometry = parse_wkt_bounded(cell, &cella_wkt)?;
                buf.clear();
                encode_wkb_into_bounded(&geometry, WkbFlavor::Iso, buf, cella_wkt.max_cell_bytes)?;
                geom_b.append_value(buf.as_slice());
            }
        }
        GeomSpec::Xy(xi, yi) => {
            let x_text = required_cell(rec, xi)?.trim();
            let y_text = required_cell(rec, yi)?.trim();
            if x_text.is_empty() && y_text.is_empty() {
                geom_b.append_null();
                return Ok(());
            }
            if x_text.is_empty() || y_text.is_empty() {
                return Err(err(&PublicMessage::Curated(
                    "coordinate CSV incomplete: X e Y devono essere entrambe presenti",
                )));
            }
            let x = x_text
                .parse::<f64>()
                .map_err(|_| err(&PublicMessage::Curated("coordinata X CSV non valida")))?;
            let y = y_text
                .parse::<f64>()
                .map_err(|_| err(&PublicMessage::Curated("coordinata Y CSV non valida")))?;
            let geometry = WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x,
                    y,
                    z: None,
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            };
            buf.clear();
            encode_wkb_into_bounded(&geometry, WkbFlavor::Iso, buf, cella_wkt.max_cell_bytes)?;
            geom_b.append_value(buf.as_slice());
        }
    }
    Ok(())
}

fn required_cell(record: &csv::StringRecord, index: usize) -> Result<&str> {
    record.get(index).ok_or_else(|| {
        err(&PublicMessage::CuratedWith(
            "riga CSV senza la colonna dichiarata nell'intestazione, indice",
            NumeroStrutturale::Indice(driver_common::saturating_u64(index)),
        ))
    })
}

fn finish_batch(
    schema: &SchemaRef,
    geom_b: &mut Option<BinaryBuilder>,
    builders: &mut [InferredColumnBuilder],
    row_count: usize,
) -> Result<RecordBatch> {
    let mut arrays: Vec<ArrayRef> =
        Vec::with_capacity(usize::from(geom_b.is_some()) + builders.len());
    if let Some(builder) = geom_b {
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

// --- scrittura streaming ---------------------------------------------------

struct CsvWriter {
    staging: StagedFile,
    writer: Option<csv::Writer<File>>,
    xy: bool,
    header_written: bool,
    wkb_limits: WkbLimits,
}

impl FormatWriter for CsvWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let schema = batch.schema();
        // La geometria e' **facoltativa**: `DataContract::geometry` lo e', e un
        // CSV di soli attributi e' un CSV. Pretenderla rendeva inconvertibile
        // una tabella -- quelle che stanno accanto alle feature class in un
        // FileGDB, per esempio -- verso il formato che di tutti e' il piu'
        // adatto a riceverla. Il descrittore lo diceva gia': `csv` dichiara
        // `CrsWriteSupport::None`, cioe' che un CRS non lo porta, e un CRS si
        // porta su una geometria.
        let geom_idx = geometry_index(&schema);
        let geom_col = match geom_idx {
            Some(indice) => Some(
                batch
                    .column(indice)
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .ok_or_else(|| err(&PublicMessage::Curated("colonna geometria non binaria")))?,
            ),
            None => None,
        };
        let limits = self.wkb_limits;
        let xy = self.xy;
        let w = self
            .writer
            .as_mut()
            .ok_or_else(|| err(&PublicMessage::Curated("writer chiuso")))?;

        if !self.header_written {
            let mut header: Vec<&str> = Vec::new();
            for (i, f) in schema.fields().iter().enumerate() {
                if Some(i) != geom_idx {
                    header.push(f.name());
                }
            }
            // Le colonne della geometria si aggiungono solo se una geometria
            // c'e': un'intestazione con `geometry` sopra una colonna sempre
            // vuota direbbe che il dato e' andato perso, invece che assente.
            if geom_idx.is_some() {
                if xy {
                    header.push("x");
                    header.push("y");
                } else {
                    header.push("geometry");
                }
            }
            w.write_record(&header).map_err(|_| {
                err(&PublicMessage::Curated(
                    "scrittura dell'intestazione CSV fallita",
                ))
            })?;
            self.header_written = true;
        }

        // Scrittura per-campo DIRETTA (niente Vec<String> né serde_json::Value
        // per cella): `fbuf` è riusato per formattare numeri/bool.
        let mut fbuf = String::new();
        for row in 0..batch.num_rows() {
            for (i, _) in schema.fields().iter().enumerate() {
                if Some(i) != geom_idx {
                    write_cell(w, batch.column(i), row, &mut fbuf)?;
                }
            }
            if let Some(geom_col) = geom_col {
                if geom_col.is_null(row) {
                    w.write_field("").map_err(|_| {
                        err(&PublicMessage::Curated("scrittura di un campo CSV fallita"))
                    })?;
                    if xy {
                        w.write_field("").map_err(|_| {
                            err(&PublicMessage::Curated("scrittura di un campo CSV fallita"))
                        })?;
                    }
                } else {
                    let geom = decode_wkb(geom_col.value(row), &limits)?;
                    if xy {
                        match geom.value {
                            WkbValue::Point(point)
                                if geom.dimensions == CoordinateDimensions::Xy =>
                            {
                                fbuf.clear();
                                let _ = write!(fbuf, "{}", point.x);
                                w.write_field(&fbuf).map_err(|_| {
                                    err(&PublicMessage::Curated(
                                        "scrittura di un campo CSV fallita",
                                    ))
                                })?;
                                fbuf.clear();
                                let _ = write!(fbuf, "{}", point.y);
                                w.write_field(&fbuf).map_err(|_| {
                                    err(&PublicMessage::Curated(
                                        "scrittura di un campo CSV fallita",
                                    ))
                                })?;
                            }
                            _ => {
                                return Err(err(&PublicMessage::Curated(
                                    "encoding xy richiede geometrie Point strettamente XY",
                                )))
                            }
                        }
                    } else {
                        fbuf.clear();
                        format_wkt_into(&geom, &mut fbuf)?;
                        w.write_field(&fbuf).map_err(|_| {
                            err(&PublicMessage::Curated("scrittura di un campo CSV fallita"))
                        })?;
                    }
                }
            }
            // Termina il record (dopo i write_field).
            w.write_record(None::<&[u8]>)
                .map_err(|_| err(&PublicMessage::Curated("scrittura di un campo CSV fallita")))?;
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut w = self
            .writer
            .take()
            .ok_or_else(|| err(&PublicMessage::Curated("writer già chiuso")))?;
        w.flush()
            .map_err(|_| err(&PublicMessage::Curated("flush del CSV fallito")))?;
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

/// Scrive una cella attributo DIRETTAMENTE nel writer CSV, senza passare per
/// `serde_json::Value`: stringhe dritte (0 alloc), numeri/bool formattati nel
/// buffer riusato `fbuf`. I tipi non comuni ricadono sul convertitore generico.
fn write_cell<W: std::io::Write>(
    w: &mut csv::Writer<W>,
    col: &ArrayRef,
    row: usize,
    fbuf: &mut String,
) -> Result<()> {
    if col.is_null(row) {
        return w
            .write_field("")
            .map_err(|_| err(&PublicMessage::Curated("scrittura di un campo CSV fallita")));
    }
    if let Some(a) = col.as_any().downcast_ref::<StringArray>() {
        w.write_field(a.value(row))
            .map_err(|_| err(&PublicMessage::Curated("scrittura di un campo CSV fallita")))
    } else if let Some(a) = col.as_any().downcast_ref::<Int64Array>() {
        fbuf.clear();
        let _ = write!(fbuf, "{}", a.value(row));
        w.write_field(&*fbuf)
            .map_err(|_| err(&PublicMessage::Curated("scrittura di un campo CSV fallita")))
    } else if let Some(a) = col.as_any().downcast_ref::<Float64Array>() {
        fbuf.clear();
        let _ = write!(fbuf, "{}", a.value(row));
        w.write_field(&*fbuf)
            .map_err(|_| err(&PublicMessage::Curated("scrittura di un campo CSV fallita")))
    } else if let Some(a) = col.as_any().downcast_ref::<BooleanArray>() {
        w.write_field(if a.value(row) { "true" } else { "false" })
            .map_err(|_| err(&PublicMessage::Curated("scrittura di un campo CSV fallita")))
    } else {
        // Tipo non comune (Date, ecc.): fallback via il convertitore generico.
        let value = json_from_array(col, row)?;
        w.write_field(cell_string(&value))
            .map_err(|_| err(&PublicMessage::Curated("scrittura di un campo CSV fallita")))
    }
}

fn cell_string(v: &JsonValue) -> String {
    match v {
        JsonValue::Null => String::new(),
        JsonValue::String(s) => s.clone(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests;

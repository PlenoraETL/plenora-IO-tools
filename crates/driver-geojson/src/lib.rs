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

mod geometry;

pub use geometry::{wkb_from_gj_value, write_geo_geojson};

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
use geojson::Geometry as GjGeometry;
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
    read_row_error, validate_write, with_write_validation, AttributeWriteSupport,
    CrsRepresentationCapabilities, CrsRepresentationState, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, TypeCoercionPolicy, WritePlan, SCALAR_TYPES,
    UTF8_FIELD_NAMES, WKB_XY_XYZ_GEOMETRY,
};
use plenora_io_model::contract::{
    DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
use plenora_io_model::crs::ResolvedCrs;
#[cfg(test)]
use plenora_io_model::geometry::is_geometry_field;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::decode_wkb;
use plenora_io_model::{PlenoraIoError, Result};

const GEOMETRY: &str = "geometry";

fn err(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("geojson", reason)
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "geojson",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential, // array `features` scorso in streaming
    read_determinism: plenora_io_core::DeterminismLevel::Semantic,
    write_mode: Some(WriteMode::Streaming), // feature-per-feature, niente buffering
    write_determinism: Some(plenora_io_core::DeterminismLevel::Semantic),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    projection_support: plenora_io_core::ProjectionSupport::Exact,
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::None,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::None,
    crs_handling: CrsHandling::FixedWgs84,
    // Finding #8 review 2026-08-15: dichiarare `Lossless` staticamente non
    // riflette il comportamento reale del driver, che non conserva `id`,
    // `bbox` ne' foreign members al re-encode (writer a riga 1088+ emette
    // solo `type`, `geometry` e `properties`). Il principio scritto in
    // `IMPLEMENTATION_STATUS.md` — "un report vuoto significa 'nessuna
    // perdita osservata', non `Lossless`" — vale anche qui: il descrittore
    // dichiara la classe potenziale, il `LossReport` dichiara le perdite
    // osservate.
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_XY_XYZ_GEOMETRY,
        crs: CrsWriteSupport::Fixed("OGC:CRS84"),
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Derived,
            CrsRepresentationState::Absent,
            CrsRepresentationState::Absent,
        ),
        nullability: NullabilitySupport::Preserve,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 6,
    descriptor_version: 7,
};

pub struct GeoJsonDriver;

impl FormatDriver for GeoJsonDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path =
            source.into_path_checked(&opts.limits, &opts.cancellation, &opts.resource_budget)?;
        // Pass 1: inferenza schema in streaming (RAM O(1)).
        let (schema, cols) = infer_schema(&path)?;
        let contract = DataContract::new(
            schema,
            Some(GeometryColumnContract::wkb_passthrough(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::wgs84(),
                true,
            )),
        );
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(plenora_io_core::with_read_budget(
            Box::new(GeoJsonDataset {
                path,
                cols,
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name,
                    contract,
                }],
            }),
            opts.resource_budget.clone(),
            true,
        ))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(self.descriptor(), plan, &opts.limits)?;
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraIoError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("geojson") || e.eq_ignore_ascii_case("json"))
        {
            return Err(PlenoraIoError::Unsupported(
                "l'output deve avere estensione .geojson o .json".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::Unsupported(
                "GeoJSON: un solo layer per file nella v1".to_owned(),
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
                wkb_limits: opts.limits.effective_wkb(),
            }),
            self.descriptor(),
            plan,
            opts.limits,
            opts.cancellation.clone(),
            opts.resource_budget.clone(),
        )
    }
}

// --- lettura streaming -----------------------------------------------------

struct GeoJsonDataset {
    path: PathBuf,
    cols: Vec<(String, ColType)>,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for GeoJsonDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(DESCRIPTOR.id, DESCRIPTOR.fidelity_class)
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
            self.path.clone(),
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
fn infer_schema(path: &Path) -> Result<(SchemaRef, Vec<(String, ColType)>)> {
    let file = File::open(path)?;
    let mut accs = SchemaAccumulators::default();
    let mut de = serde_json::Deserializer::from_reader(BufReader::new(file));
    if let Err(error) = de.deserialize_map(TopVisitor { accs: &mut accs }) {
        let error = err(format!("GeoJSON non valido: {error}"));
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
    let cols = accs.into_columns().map_err(err)?;
    let mut fields = vec![geometry_field(GEOMETRY, OGC_CRS84)];
    for (k, ct) in &cols {
        fields.push(Field::new(k, ct.arrow_data_type(), true));
    }
    Ok((Arc::new(Schema::new(fields)), cols))
}

// --- pass-1: visitor serde streaming (chiavi + tipo, zero valori) ----------

#[derive(Default)]
struct SchemaAccumulators {
    indices: HashMap<String, usize>,
    values: Vec<TypeAccumulator>,
    source_rows_seen: u64,
    in_feature: bool,
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
                        return Err(<A::Error as DeError>::custom(format!(
                            "GeoJSON top-level type '{value}' non supportato: atteso 'FeatureCollection'"
                        )));
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
            return Err(<A::Error as DeError>::custom(
                "GeoJSON senza campo 'type' al livello top",
            ));
        }
        // Follow-up review 2026-08-15: la specifica GeoJSON (RFC 7946 §3.3)
        // dichiara `features` obbligatorio per un `FeatureCollection`. Un
        // documento con solo `{"type":"FeatureCollection"}` non e' vuoto
        // per definizione: e' incompleto. Fail-closed invece di trattarlo
        // come un dataset di zero righe.
        if !features_observed {
            return Err(<A::Error as DeError>::custom(
                "FeatureCollection GeoJSON senza campo 'features'",
            ));
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
                        return Err(<A::Error as DeError>::custom(format!(
                            "membro di features con type '{value}': atteso 'Feature'"
                        )));
                    }
                    type_observed = Some(value);
                }
                FeatKey::Geom | FeatKey::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if type_observed.is_none() {
            return Err(<A::Error as DeError>::custom(
                "membro di features senza campo 'type'",
            ));
        }
        self.accs.in_feature = false;
        self.accs.source_rows_seen = self
            .accs
            .source_rows_seen
            .checked_add(1)
            .ok_or_else(|| A::Error::custom("troppe feature GeoJSON"))?;
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
            self.accs.observe(index, tag).map_err(A::Error::custom)?;
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
fn spawn_parser(
    path: PathBuf,
    schema: SchemaRef,
    cols: Vec<(String, ColType)>,
    property_names: Vec<String>,
    include_geometry: bool,
    batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    layer: LayerContract,
) -> Result<Box<dyn LayerReader>> {
    spawn_batch_reader(DESCRIPTOR.id, layer, 2, move |emitter: BatchEmitter| {
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
        };
        // Deserializer serde streaming: scrive i feature DIRETTAMENTE nei
        // builder (chiavi via key-seed = 0 alloc, valori scalari appesi
        // diretti = 0 alloc). Niente DOM Feature/JsonObject per feature.
        let mut de = serde_json::Deserializer::from_reader(BufReader::new(file));
        let result = de.deserialize_map(TopSink { sink: &mut sink });
        if sink.aborted {
            return Ok(()); // consumatore andato via: stop pulito
        }
        if let Err(error) = result {
            let error = err(format!("GeoJSON non valido: {error}"));
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
            let batch = finish_batch(&sink.schema, &mut sink.geom, &mut sink.builders, sink.n)
                .map_err(err)?;
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
    builders: Vec<InferredColumnBuilder>,
    seen: Vec<bool>,
    property_seen: Vec<bool>,
    n: usize,
    source_rows_seen: u64,
    in_feature: bool,
    batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    aborted: bool,
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
                        return Err(<A::Error as DeError>::custom(format!(
                            "GeoJSON top-level type '{value}' non supportato: atteso 'FeatureCollection'"
                        )));
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
            return Err(<A::Error as DeError>::custom(
                "GeoJSON senza campo 'type' al livello top",
            ));
        }
        if !features_observed {
            return Err(<A::Error as DeError>::custom(
                "FeatureCollection GeoJSON senza campo 'features'",
            ));
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
        for s in &mut self.sink.seen {
            *s = false;
        }
        for seen in &mut self.sink.property_seen {
            *seen = false;
        }
        let mut geom_seen = false;
        let mut props_seen = false;
        let mut type_seen = false;
        while let Some(fk) = map.next_key_seed(FeatKeySeed)? {
            match fk {
                FeatKey::Type if type_seen => {
                    return Err(<A::Error as DeError>::custom(
                        "chiave type duplicata nella feature GeoJSON",
                    ));
                }
                FeatKey::Type => {
                    let value: String = map.next_value()?;
                    if value != "Feature" {
                        return Err(<A::Error as DeError>::custom(format!(
                            "membro di features con type '{value}': atteso 'Feature'"
                        )));
                    }
                    type_seen = true;
                }
                FeatKey::Geom if geom_seen => {
                    return Err(<A::Error as DeError>::custom(
                        "chiave geometry duplicata nella feature GeoJSON",
                    ));
                }
                FeatKey::Geom => {
                    if let Some(geometry) = &mut self.sink.geom {
                        // Finding #6 review 2026-08-15: la deserializzazione
                        // di `GjGeometry` costruisce ricorsivamente `Vec` di
                        // coordinate/anelli/geometrie prima che qualunque
                        // budget veda il risultato. Intercettando prima come
                        // `RawValue` conosciamo la lunghezza in byte della
                        // geometria e possiamo rifiutare payload oltre il
                        // cap del bordo (default WKB `max_cell_bytes`, 64
                        // MiB) senza mai materializzare l'AST. Un fix
                        // completo (contatori vertici/depth applicati
                        // durante il parse) richiede un Visitor dedicato:
                        // vedi lotto L6 di ROADMAP-1.1.0.md.
                        let raw = map.next_value::<Option<Box<serde_json::value::RawValue>>>()?;
                        match raw {
                            None => geometry.append_null(),
                            Some(raw) => {
                                let raw_text = raw.get();
                                let max_bytes = WkbLimits::default().max_cell_bytes;
                                if raw_text.len() > max_bytes {
                                    return Err(<A::Error as DeError>::custom(format!(
                                        "geometria GeoJSON di {} byte oltre il limite {max_bytes}",
                                        raw_text.len()
                                    )));
                                }
                                let gj: GjGeometry = serde_json::from_str(raw_text)
                                    .map_err(<A::Error as DeError>::custom)?;
                                self.sink.wkb_buf.clear();
                                wkb_from_gj_value(&gj.value, &mut self.sink.wkb_buf)
                                    .map_err(<A::Error as DeError>::custom)?;
                                geometry.append_value(&self.sink.wkb_buf);
                            }
                        }
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                    geom_seen = true;
                }
                FeatKey::Props if props_seen => {
                    return Err(<A::Error as DeError>::custom(
                        "chiave properties duplicata nella feature GeoJSON",
                    ));
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
            return Err(<A::Error as DeError>::custom(
                "membro di features senza campo 'type'",
            ));
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
                        return Err(<A::Error as DeError>::custom("consumatore chiuso"));
                    }
                }
                Err(e) => return Err(<A::Error as DeError>::custom(e)),
            }
            self.sink.n = 0;
        }
        self.sink.in_feature = false;
        self.sink.source_rows_seen = self
            .sink
            .source_rows_seen
            .checked_add(1)
            .ok_or_else(|| A::Error::custom("troppe feature GeoJSON"))?;
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
                return Err(<A::Error as DeError>::custom(
                    "chiave duplicata nelle properties GeoJSON",
                ));
            }
            self.sink.property_seen[hit.property_idx] = true;
            match hit.projected_idx {
                // Chiave nota e non ancora vista in questa feature: append.
                Some(idx) => {
                    map.next_value_seed(ValueSink {
                        b: &mut self.sink.builders[idx],
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
        self.b.append_json(Some(&v)).map_err(A::Error::custom)
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> std::result::Result<(), A::Error> {
        let v = JsonValue::deserialize(MapAccessDeserializer::new(map))?;
        self.b.append_json(Some(&v)).map_err(A::Error::custom)
    }
}

fn finish_batch(
    schema: &SchemaRef,
    geom: &mut Option<BinaryBuilder>,
    builders: &mut [InferredColumnBuilder],
    row_count: usize,
) -> std::result::Result<RecordBatch, String> {
    let mut arrays: Vec<ArrayRef> =
        Vec::with_capacity(usize::from(geom.is_some()) + builders.len());
    if let Some(builder) = geom {
        arrays.push(Arc::new(builder.finish()));
    }
    for b in builders.iter_mut() {
        arrays.push(b.finish());
    }
    let options = RecordBatchOptions::new().with_row_count(Some(row_count));
    RecordBatch::try_new_with_options(schema.clone(), arrays, &options)
        .map_err(|e| format!("record batch: {e}"))
}

/// Entry point per il fuzzer (NON API stabile): esegue pass-1 + pass-2 in modo
/// **sincrono** su `bytes`, con un unico batch finale. JSON invalido o colonne
/// disallineate producono `Err`, senza panic.
#[doc(hidden)]
pub fn __fuzz_read_geojson(bytes: &[u8]) -> std::result::Result<usize, String> {
    use std::io::Cursor;
    // pass-1: schema
    let mut accs = SchemaAccumulators::default();
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
    };
    serde_json::Deserializer::from_reader(Cursor::new(bytes))
        .deserialize_map(TopSink { sink: &mut sink })
        .map_err(|e| e.to_string())?;
    if sink.n > 0 {
        let batch = finish_batch(&schema, &mut sink.geom, &mut sink.builders, sink.n)?;
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
        let geom_idx =
            geometry_index(&schema).ok_or_else(|| err("nessuna colonna geometria geoarrow.wkb"))?;
        let geom_col = batch
            .column(geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err("colonna geometria non binaria"))?;
        let limits = self.wkb_limits;
        let w = self.writer.as_mut().ok_or_else(|| err("writer chiuso"))?;
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
        let mut w = self.writer.take().ok_or_else(|| err("writer già chiuso"))?;
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
        serde_json::to_writer(&mut *w, field.name()).map_err(|e| err(e.to_string()))?;
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
        serde_json::to_writer(&mut *w, a.value(row)).map_err(|e| err(e.to_string()))?;
    } else if let Some(a) = col.as_any().downcast_ref::<Int64Array>() {
        write!(w, "{}", a.value(row))?;
    } else if let Some(a) = col.as_any().downcast_ref::<Float64Array>() {
        let v = a.value(row);
        if v.is_finite() {
            // serde_json (ryu): round-trippabile anche per f64 estremi.
            serde_json::to_writer(&mut *w, &v).map_err(|e| err(e.to_string()))?;
        } else {
            return Err(err("Float64 non finito non rappresentabile in GeoJSON"));
        }
    } else if let Some(a) = col.as_any().downcast_ref::<BooleanArray>() {
        w.write_all(if a.value(row) { b"true" } else { b"false" })?;
    } else {
        serde_json::to_writer(&mut *w, &json_from_array(col, row)?)
            .map_err(|e| err(e.to_string()))?;
    }
    Ok(())
}

// Solo per i test: parse completo (documenti piccoli).
#[cfg(test)]
fn parse_features(text: &str) -> Result<Vec<geojson::Feature>> {
    let gj: geojson::GeoJson = text.parse().map_err(|e| err(format!("GeoJSON: {e}")))?;
    match gj {
        geojson::GeoJson::FeatureCollection(c) => Ok(c.features),
        geojson::GeoJson::Feature(f) => Ok(vec![f]),
        geojson::GeoJson::Geometry(_) => Err(err("atteso Feature/FeatureCollection")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
    use plenora_io_core::WriteLayer;
    use plenora_io_model::contract::{CoordinateDimensions, GeometryType};
    use plenora_io_model::wkb::{from_wkb, WkbCoordinate, WkbValue};
    use plenora_io_model::CancellationToken;
    use std::fmt::Write as _;

    fn read_all(driver: &GeoJsonDriver, path: &Path) -> (RecordBatch, LayerContract) {
        let ds = driver
            .open(Source::Path(path.to_owned()), &ReadOptions::default())
            .unwrap();
        let layer = ds.layers()[0].clone();
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        (batch, layer)
    }

    #[test]
    fn round_trip_geojson_recordbatch_geojson() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.geojson");
        std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[12.5,45.9]},"properties":{"n":1,"s":"a","b":true}},
            {"type":"Feature","geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]},"properties":{"n":2,"s":"b","b":false}}
            ]}"#,
        )
        .unwrap();

        let driver = GeoJsonDriver;
        let (batch, layer) = read_all(&driver, &src);
        assert_eq!(
            layer.contract.geometry.as_ref().unwrap().crs.id(),
            Some("OGC:CRS84")
        );
        assert_eq!(
            layer
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .resolved_crs()
                .unwrap()
                .axis_order,
            plenora_io_model::crs::AxisOrder::LongitudeLatitude
        );
        assert_eq!(batch.num_rows(), 2);
        assert!(is_geometry_field(
            &batch.schema().field_with_name("geometry").unwrap().clone()
        ));

        // scrivi verso GeoJSON e rileggi
        let out = dir.path().join("out.geojson");
        let mut output_contract = layer.contract;
        output_contract
            .geometry
            .as_mut()
            .unwrap()
            .set_exact_geometry_types(vec![GeometryType::Point, GeometryType::LineString]);
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: output_contract,
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let features = parse_features(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(features.len(), 2);
        assert_eq!(
            features[0].properties.as_ref().unwrap().get("s").unwrap(),
            "a"
        );
        match &features[0].geometry.as_ref().unwrap().value {
            geojson::Value::Point(c) => assert!((c[0] - 12.5).abs() < 1e-9),
            other => panic!("atteso Point, {other:?}"),
        }
    }

    #[test]
    fn integer_outside_i64_is_preserved_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("wide-integer.geojson");
        std::fs::write(
            &source,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":null,"properties":{"identifier":18446744073709551615}}
            ]}"#,
        )
        .unwrap();

        let (batch, _) = read_all(&GeoJsonDriver, &source);
        let identifier = batch
            .column(batch.schema().index_of("identifier").unwrap())
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();

        assert_eq!(identifier.value(0), "18446744073709551615");
    }

    #[test]
    fn heterogeneous_features_align_columns() {
        // Feature con proprietà disomogenee: chiave mancante, properties null,
        // chiave sconosciuta, geometria null. Il deserializer custom deve
        // mantenere una append per builder per feature (colonne allineate).
        use arrow_array::{Int64Array, StringArray};
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("het.geojson");
        std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[1,1]},"properties":{"a":1,"b":"x"}},
            {"type":"Feature","geometry":null,"properties":{"a":2}},
            {"type":"Feature","geometry":{"type":"Point","coordinates":[3,3]},"properties":null},
            {"type":"Feature","geometry":{"type":"Point","coordinates":[4,4]},"properties":{"b":"y","c":99}}
            ]}"#,
        )
        .unwrap();

        let driver = GeoJsonDriver;
        let (batch, _layer) = read_all(&driver, &src);
        assert_eq!(batch.num_rows(), 4);
        let schema = batch.schema();

        let geom = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert!(!geom.is_null(0) && geom.is_null(1) && !geom.is_null(2) && !geom.is_null(3));

        let col = |name: &str| schema.index_of(name).unwrap();
        let a = batch
            .column(col("a"))
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(a.value(0), 1);
        assert_eq!(a.value(1), 2);
        assert!(a.is_null(2) && a.is_null(3));

        let b = batch
            .column(col("b"))
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(b.value(0), "x");
        assert!(b.is_null(1) && b.is_null(2));
        assert_eq!(b.value(3), "y");

        let c = batch
            .column(col("c"))
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert!(c.is_null(0) && c.is_null(1) && c.is_null(2));
        assert_eq!(c.value(3), 99);
    }

    #[test]
    fn source_property_order_does_not_change_inferred_schema_order() {
        let dir = tempfile::tempdir().unwrap();
        let za = dir.path().join("za.geojson");
        let az = dir.path().join("az.geojson");
        std::fs::write(
            &za,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":null,"properties":{"z":1,"a":"x"}}
            ]}"#,
        )
        .unwrap();
        std::fs::write(
            &az,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":null,"properties":{"a":"x","z":1}}
            ]}"#,
        )
        .unwrap();

        let (za_schema, za_columns) = infer_schema(&za).unwrap();
        let (az_schema, az_columns) = infer_schema(&az).unwrap();
        assert_eq!(za_schema, az_schema);
        assert_eq!(za_columns, az_columns);
        assert_eq!(
            za_schema
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>(),
            ["geometry", "a", "z"]
        );
    }

    #[test]
    fn duplicate_keys_are_rejected_instead_of_using_first_value_wins() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("dup.geojson");
        std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"geometry":{"type":"Point","coordinates":[9,9]},"properties":{"c":1,"b":"x","c":true}}
            ]}"#,
        )
        .unwrap();
        let driver = GeoJsonDriver;
        let dataset = driver
            .open(Source::Path(src), &ReadOptions::default())
            .unwrap();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let error = reader.next_batch().unwrap_err();
        assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
        assert!(error.message.contains("geometry duplicata"));
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.examples[0].source_index, 0);
        assert_eq!(diagnostics.counts["geojson.invalid_feature"], 1);
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn duplicate_property_outside_projection_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("dup-outside-projection.geojson");
        std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"properties":{"kept":1,"outside":"first","outside":"second"}}
            ]}"#,
        )
        .unwrap();
        let driver = GeoJsonDriver;
        let dataset = driver
            .open(Source::Path(src), &ReadOptions::default())
            .unwrap();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: Some(vec![FieldId(0)]),
                projection_mode: ProjectionMode::Required,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();

        let error = reader.next_batch().unwrap_err();
        assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
        assert_eq!(error.phase, plenora_io_model::ErrorPhase::Read);
        assert!(error.message.contains("chiave duplicata nelle properties"));
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.examples[0].source_index, 0);
        assert_eq!(diagnostics.counts["geojson.invalid_feature"], 1);
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn polygon_and_multipolygon_round_trip() {
        // Esercita la conversione geometria DIRETTA in entrambe le direzioni:
        // lettura geojson→WKB (wkb_from_gj_value) e scrittura WKB→JSON
        // (write_geo_geojson), su Polygon-con-buco e MultiPolygon.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("poly.geojson");
        std::fs::write(
            &src,
            r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[2,1],[2,2],[1,2],[1,1]]]},"properties":{"k":1}},
            {"type":"Feature","geometry":{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]],[[[5,5],[6,5],[6,6],[5,5]]]]},"properties":{"k":2}}
            ]}"#,
        )
        .unwrap();

        let driver = GeoJsonDriver;
        let (batch, layer) = read_all(&driver, &src);
        assert_eq!(batch.num_rows(), 2);

        // Lettura: geojson→WKB deve dare un WKB decodificabile da from_wkb.
        let geom = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let limits = WkbLimits::default();
        match from_wkb(geom.value(0), &limits).unwrap() {
            geo_types::Geometry::Polygon(pl) => {
                assert_eq!(pl.exterior().0.len(), 5);
                assert_eq!(pl.interiors().len(), 1);
                assert_eq!(pl.interiors()[0].0.len(), 5);
            }
            other => panic!("atteso Polygon, {other:?}"),
        }
        match from_wkb(geom.value(1), &limits).unwrap() {
            geo_types::Geometry::MultiPolygon(mp) => assert_eq!(mp.0.len(), 2),
            other => panic!("atteso MultiPolygon, {other:?}"),
        }

        // Scrittura: WKB→JSON diretto; rileggendo la geometria deve sopravvivere.
        let out = dir.path().join("poly-out.geojson");
        let mut output_contract = layer.contract;
        output_contract
            .geometry
            .as_mut()
            .unwrap()
            .set_exact_geometry_types(vec![GeometryType::Polygon, GeometryType::MultiPolygon]);
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: output_contract,
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let feats = parse_features(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(feats.len(), 2);
        match &feats[0].geometry.as_ref().unwrap().value {
            geojson::Value::Polygon(rings) => {
                assert_eq!(rings.len(), 2); // esterno + 1 buco
                assert_eq!(rings[0].len(), 5);
                assert_eq!(rings[1].len(), 5);
                assert!((rings[1][0][0] - 1.0).abs() < 1e-9); // il buco parte da x=1
            }
            other => panic!("atteso Polygon, {other:?}"),
        }
        match &feats[1].geometry.as_ref().unwrap().value {
            geojson::Value::MultiPolygon(polys) => assert_eq!(polys.len(), 2),
            other => panic!("atteso MultiPolygon, {other:?}"),
        }
    }

    #[test]
    fn xyz_round_trip_preserves_altitude() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("xyz.geojson");
        let output = dir.path().join("xyz-out.geojson");
        std::fs::write(
            &input,
            r#"{"type":"FeatureCollection","features":[
                {"type":"Feature","geometry":{"type":"Point","coordinates":[12.5,45.9,123.25]},"properties":{"name":"quota"}}
            ]}"#,
        )
        .unwrap();

        let driver = GeoJsonDriver;
        let (batch, layer) = read_all(&driver, &input);
        assert_eq!(
            layer.contract.geometry.as_ref().unwrap().dimensions,
            CoordinateDimensions::Unknown
        );
        let geometry = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
        assert_eq!(decoded.dimensions, CoordinateDimensions::Xyz);
        assert!(matches!(
            decoded.value,
            WkbValue::Point(WkbCoordinate {
                z: Some(123.25),
                ..
            })
        ));

        let mut output_contract = layer.contract;
        output_contract
            .geometry
            .as_mut()
            .unwrap()
            .set_exact_geometry_types(vec![GeometryType::Point]);
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "xyz".to_owned(),
                contract: output_contract,
            }],
        };
        let mut writer = driver
            .create(Sink::Path(output.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let text = std::fs::read_to_string(&output).unwrap();
        assert!(text.contains("[12.5,45.9,123.25]"));
        let (round_trip, _) = read_all(&driver, &output);
        let geometry = round_trip
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
        assert_eq!(decoded.dimensions, CoordinateDimensions::Xyz);
    }

    #[test]
    fn fourth_geojson_ordinate_is_rejected() {
        let mut output = Vec::new();
        let result = wkb_from_gj_value(
            &geojson::Value::Point(vec![1.0, 2.0, 3.0, 4.0]),
            &mut output,
        );
        assert!(result.is_err());
    }

    #[test]
    fn empty_geometry_does_not_invent_xy_dimensions() {
        let mut output = Vec::new();
        assert!(wkb_from_gj_value(&geojson::Value::LineString(vec![]), &mut output).is_err());
        assert!(
            wkb_from_gj_value(&geojson::Value::GeometryCollection(vec![]), &mut output).is_err()
        );
    }

    #[test]
    fn extreme_coordinate_survives_geojson_write() {
        // Regressione (trovato dal fuzzer): una coordinata f64 estrema deve
        // produrre JSON RI-LEGGIBILE (serde_json/ryu), non un decimale che
        // serde_json rifiuta in rilettura ("number out of range").
        let mut buf = Vec::new();
        let g = geo_types::Geometry::Point(geo_types::Point::new(f64::MAX, -1.5));
        write_geo_geojson(&mut buf, &g).unwrap();
        let parsed: geojson::Geometry = serde_json::from_slice(&buf).unwrap();
        match parsed.value {
            geojson::Value::Point(c) => {
                // Il round-trip deve restituire f64::MAX identico bit a bit:
                // il confronto esatto è il contratto della regressione.
                #[allow(clippy::float_cmp)]
                {
                    assert_eq!(c[0], f64::MAX);
                }
                assert!((c[1] + 1.5).abs() < 1e-9);
            }
            other => panic!("atteso Point, {other:?}"),
        }
    }

    #[test]
    fn streams_multiple_batches() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("many.geojson");
        let mut s = String::from("{\"type\":\"FeatureCollection\",\"features\":[");
        for i in 0..10 {
            if i > 0 {
                s.push(',');
            }
            write!(
                s,
                "{{\"type\":\"Feature\",\"geometry\":{{\"type\":\"Point\",\"coordinates\":[{i},{i}]}},\"properties\":{{\"id\":{i}}}}}"
            )
            .unwrap();
        }
        s.push_str("]}");
        std::fs::write(&src, s).unwrap();

        let driver = GeoJsonDriver;
        let ds = driver
            .open(Source::Path(src), &ReadOptions::default())
            .unwrap();
        let mut reader = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget {
                    target_bytes: 8 * 1024 * 1024,
                    max_rows: 4,
                },
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let mut total = 0;
        let mut batches = 0;
        while let Some(b) = reader.next_batch().unwrap() {
            total += b.num_rows();
            batches += 1;
        }
        assert_eq!(total, 10);
        assert!(
            batches >= 3,
            "atteso streaming multi-batch, avuti {batches}"
        );
    }
}

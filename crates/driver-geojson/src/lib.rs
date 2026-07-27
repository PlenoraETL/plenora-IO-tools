//! driver-geojson — GeoJSON ⇄ RecordBatch. GeoJSON è WGS84 per specifica
//! (`OGC:CRS84`). La geometria diventa una colonna WKB `geoarrow.wkb`.
//!
//! Lettura **streaming** (Fase 2A): l'array `features` del FeatureCollection è
//! scorso un feature alla volta (`geojson::FeatureReader`), senza costruire il
//! DOM `serde_json::Value` dell'intero documento. Due passate: pass 1 (`open`)
//! inferisce lo schema (unione chiavi/tipi) a RAM O(1); pass 2 (reader) è un
//! thread che produce RecordBatch da `batch_target` righe, consegnati via canale
//! con backpressure → memoria O(batch), non O(file). Geometrie convertite
//! direttamente a WKB, attributi in builder tipizzati (niente intermedio
//! `serde_json::Value` per colonna). La scrittura resta bufferizzante nella v1.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Float64Builder, Int64Builder, StringBuilder,
};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use geojson::Geometry as GjGeometry;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{
    DeserializeSeed, Deserializer, Error as DeError, IgnoredAny, MapAccess, SeqAccess, Visitor,
};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use driver_common::{geometry_field, json_from_array, ColType, OGC_CRS84};
use plenora_core::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
};
use plenora_core::crs::ResolvedCrs;
use plenora_core::geometry::is_geometry_field;
use plenora_core::limits::WkbLimits;
use plenora_core::wkb::{decode_wkb, WkbCoordinate, WkbGeometry, WkbValue};
use plenora_core::{PlenoraError, Result};
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, LayerReader, OpenDatasetHandle, Published, ReadOptions, Sink,
    Source, WriteOptions,
};
use plenora_io_core::loss::LossReport;
use plenora_io_core::publish::publish_file_atomic_limited;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    validate_write, with_write_validation, AttributeWriteSupport, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, TypeCoercionPolicy, WritePlan, SCALAR_TYPES,
    UTF8_FIELD_NAMES, WKB_XY_XYZ_GEOMETRY,
};

const GEOMETRY: &str = "geometry";

fn err(reason: impl Into<String>) -> PlenoraError {
    PlenoraError::Format {
        driver: "geojson",
        reason: reason.into(),
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "geojson",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential, // array `features` scorso in streaming
    write_mode: Some(WriteMode::Streaming),   // feature-per-feature, niente buffering
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    projection_support: plenora_io_core::ProjectionSupport::None,
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::None,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::None,
    crs_handling: CrsHandling::FixedWgs84,
    fidelity_class: Fidelity::Lossless,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_XY_XYZ_GEOMETRY,
        crs: CrsWriteSupport::Fixed("OGC:CRS84"),
        nullability: NullabilitySupport::Preserve,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 5,
    descriptor_version: 4,
};

pub struct GeoJsonDriver;

impl FormatDriver for GeoJsonDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = source.into_path_checked(&opts.limits)?;
        // Pass 1: inferenza schema in streaming (RAM O(1)).
        let (schema, cols) = infer_schema(&path)?;
        let contract = DataContract {
            schema: schema.clone(),
            geometry: Some(GeometryColumnContract::wkb_passthrough(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::wgs84(),
                true,
            )),
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(Box::new(GeoJsonDataset {
            path,
            schema,
            cols,
            layers: vec![LayerContract {
                id: LayerId(0),
                name,
                contract,
            }],
        }))
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
            return Err(PlenoraError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("geojson") || e.eq_ignore_ascii_case("json"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .geojson o .json".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
                "GeoJSON: un solo layer per file nella v1".to_owned(),
            ));
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let temp = tempfile::NamedTempFile::new_in(&parent)?;
        let mut writer = BufWriter::new(temp.reopen()?);
        writer.write_all(b"{\"type\":\"FeatureCollection\",\"features\":[")?;
        Ok(with_write_validation(
            Box::new(GeoJsonWriter {
                temp: Some(temp),
                writer: Some(writer),
                path,
                durable: opts.durable,
                first: true,
                wkb_limits: opts.limits.effective_wkb(),
                max_output_bytes: opts.limits.max_output_bytes,
            }),
            self.descriptor(),
            plan,
            opts.limits,
        ))
    }
}

// --- lettura streaming -----------------------------------------------------

struct GeoJsonDataset {
    path: PathBuf,
    schema: SchemaRef,
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
        let batch_size =
            plenora_io_core::effective_batch_rows(self.schema.as_ref(), request.batch_target);
        let rx = spawn_parser(
            self.path.clone(),
            self.schema.clone(),
            self.cols.clone(),
            batch_size,
        );
        Ok(Box::new(GeoJsonReader {
            rx,
            layer: self.layers[0].clone(),
        }))
    }
}

struct GeoJsonReader {
    rx: Receiver<std::result::Result<RecordBatch, String>>,
    layer: LayerContract,
}

impl LayerReader for GeoJsonReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        match self.rx.recv() {
            Ok(Ok(batch)) => Ok(Some(batch)),
            Ok(Err(e)) => Err(err(e)),
            Err(_) => Ok(None), // canale chiuso = fine stream
        }
    }
}

/// Accumulatore d'inferenza per chiave (stessa semantica di
/// `driver_common::infer_column`, ma incrementale per lo streaming).
struct KeyAcc {
    any: bool,
    all_int: bool,
    all_num: bool,
    all_bool: bool,
}

impl Default for KeyAcc {
    fn default() -> Self {
        KeyAcc {
            any: false,
            all_int: true,
            all_num: true,
            all_bool: true,
        }
    }
}

impl KeyAcc {
    /// Classe del valore: 0 null, 1 int, 2 float, 3 bool, 4 testo/altro.
    fn observe_class(&mut self, class: u8) {
        match class {
            0 => {}
            1 => {
                self.any = true;
                self.all_bool = false;
            }
            2 => {
                self.any = true;
                self.all_int = false;
                self.all_bool = false;
            }
            3 => {
                self.any = true;
                self.all_int = false;
                self.all_num = false;
            }
            _ => {
                self.any = true;
                self.all_int = false;
                self.all_num = false;
                self.all_bool = false;
            }
        }
    }
    fn coltype(&self) -> ColType {
        if !self.any {
            ColType::Text
        } else if self.all_int {
            ColType::Integer
        } else if self.all_bool {
            ColType::Boolean
        } else if self.all_num {
            ColType::Number
        } else {
            ColType::Text
        }
    }
}

fn coltype_to_dt(ct: ColType) -> DataType {
    match ct {
        ColType::Integer => DataType::Int64,
        ColType::Number => DataType::Float64,
        ColType::Boolean => DataType::Boolean,
        ColType::Text => DataType::Utf8,
    }
}

/// Pass 1: unione chiavi proprietà + tipo, con un deserializer serde **streaming**
/// che legge SOLO le chiavi e la classe di tipo dei valori — niente DOM, niente
/// geometria, niente valori materializzati (allocazioni ~ solo le chiavi nuove).
fn infer_schema(path: &Path) -> Result<(SchemaRef, Vec<(String, ColType)>)> {
    let file = File::open(path)?;
    let mut accs: BTreeMap<String, KeyAcc> = BTreeMap::new();
    let mut de = serde_json::Deserializer::from_reader(BufReader::new(file));
    de.deserialize_map(TopVisitor { accs: &mut accs })
        .map_err(|e| err(format!("GeoJSON non valido: {e}")))?;
    let cols: Vec<(String, ColType)> = accs.iter().map(|(k, a)| (k.clone(), a.coltype())).collect();
    let mut fields = vec![geometry_field(GEOMETRY, OGC_CRS84)];
    for (k, ct) in &cols {
        fields.push(Field::new(k, coltype_to_dt(*ct), true));
    }
    Ok((Arc::new(Schema::new(fields)), cols))
}

// --- pass-1: visitor serde streaming (chiavi + tipo, zero valori) ----------

/// Classe di tipo di un valore JSON, letta senza allocare il valore.
struct TypeTag(u8);

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
    fn visit_i64<E>(self, _: i64) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(1))
    }
    fn visit_u64<E>(self, _: u64) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(1))
    }
    fn visit_i128<E>(self, _: i128) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(1))
    }
    fn visit_u128<E>(self, _: u128) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(1))
    }
    fn visit_f64<E>(self, _: f64) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(2))
    }
    fn visit_bool<E>(self, _: bool) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(3))
    }
    fn visit_str<E>(self, _: &str) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(4))
    }
    fn visit_none<E>(self) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(0))
    }
    fn visit_unit<E>(self) -> std::result::Result<TypeTag, E> {
        Ok(TypeTag(0))
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<TypeTag, D::Error> {
        d.deserialize_any(TagVisitor)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<TypeTag, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(TypeTag(4))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<TypeTag, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(TypeTag(4))
    }
}

/// Livello top: FeatureCollection; interessa solo la chiave "features".
struct TopVisitor<'a> {
    accs: &'a mut BTreeMap<String, KeyAcc>,
}
impl<'a, 'de> Visitor<'de> for TopVisitor<'a> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un oggetto GeoJSON")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            if key == "features" {
                map.next_value_seed(FeaturesSeed { accs: self.accs })?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }
}

struct FeaturesSeed<'a> {
    accs: &'a mut BTreeMap<String, KeyAcc>,
}
impl<'a, 'de> DeserializeSeed<'de> for FeaturesSeed<'a> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_seq(FeaturesVisitor { accs: self.accs })
    }
}
struct FeaturesVisitor<'a> {
    accs: &'a mut BTreeMap<String, KeyAcc>,
}
impl<'a, 'de> Visitor<'de> for FeaturesVisitor<'a> {
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
    accs: &'a mut BTreeMap<String, KeyAcc>,
}
impl<'a, 'de> DeserializeSeed<'de> for FeatureSeed<'a> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_map(FeatureVisitor { accs: self.accs })
    }
}
struct FeatureVisitor<'a> {
    accs: &'a mut BTreeMap<String, KeyAcc>,
}
impl<'a, 'de> Visitor<'de> for FeatureVisitor<'a> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un Feature")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            if key == "properties" {
                map.next_value_seed(PropsSeed { accs: self.accs })?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }
}

struct PropsSeed<'a> {
    accs: &'a mut BTreeMap<String, KeyAcc>,
}
impl<'a, 'de> DeserializeSeed<'de> for PropsSeed<'a> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        // `deserialize_any`: le properties possono essere `null` (oltre a oggetto).
        d.deserialize_any(PropsVisitor { accs: self.accs })
    }
}
struct PropsVisitor<'a> {
    accs: &'a mut BTreeMap<String, KeyAcc>,
}
impl<'a, 'de> Visitor<'de> for PropsVisitor<'a> {
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
        while let Some(key) = map.next_key::<String>()? {
            let tag = map.next_value::<TypeTag>()?.0;
            // Alloca la chiave solo se nuova (dopo il 1° feature, nessuna alloc).
            if let Some(acc) = self.accs.get_mut(&key) {
                acc.observe_class(tag);
            } else {
                let mut acc = KeyAcc::default();
                acc.observe_class(tag);
                self.accs.insert(key, acc);
            }
        }
        Ok(())
    }
}

/// Pass 2: thread che produce batch da `batch_size` righe via canale bounded
/// (backpressure → memoria O(batch)).
fn spawn_parser(
    path: PathBuf,
    schema: SchemaRef,
    cols: Vec<(String, ColType)>,
    batch_size: usize,
) -> Receiver<std::result::Result<RecordBatch, String>> {
    let (tx, rx) = sync_channel::<std::result::Result<RecordBatch, String>>(2);
    std::thread::spawn(move || {
        let run = || -> std::result::Result<(), String> {
            let file = File::open(&path).map_err(|e| e.to_string())?;
            let ncols = cols.len();
            let col_idx: HashMap<String, usize> = cols
                .iter()
                .enumerate()
                .map(|(i, (k, _))| (k.clone(), i))
                .collect();
            let mut sink = RowSink {
                schema: schema.clone(),
                col_idx,
                tx: tx.clone(),
                geom: BinaryBuilder::new(),
                wkb_buf: Vec::new(),
                builders: cols.iter().map(|(_, ct)| ColBuilder::new(*ct)).collect(),
                seen: vec![false; ncols],
                n: 0,
                batch_size,
                aborted: false,
            };
            // Deserializer serde streaming: scrive i feature DIRETTAMENTE nei
            // builder (chiavi via key-seed = 0 alloc, valori scalari appesi
            // diretti = 0 alloc). Niente DOM Feature/JsonObject per feature.
            let mut de = serde_json::Deserializer::from_reader(BufReader::new(file));
            let res = de.deserialize_map(TopSink { sink: &mut sink });
            if sink.aborted {
                return Ok(()); // consumatore andato via: stop pulito
            }
            res.map_err(|e| format!("GeoJSON non valido: {e}"))?;
            if sink.n > 0 {
                let batch = finish_batch(&sink.schema, &mut sink.geom, &mut sink.builders)?;
                let _ = sink.tx.send(Ok(batch));
            }
            Ok(())
        };
        if let Err(e) = run() {
            let _ = tx.send(Err(e));
        }
    });
    rx
}

/// Stato del pass-2: builder tipizzati + bookkeeping per feature. Possiede tutto
/// (schema/tx/col_idx sono clonati, cheap) così i seed serde lo passano come
/// `&mut RowSink` senza parametri di lifetime.
struct RowSink {
    schema: SchemaRef,
    col_idx: HashMap<String, usize>,
    tx: SyncSender<std::result::Result<RecordBatch, String>>,
    geom: BinaryBuilder,
    wkb_buf: Vec<u8>,
    builders: Vec<ColBuilder>,
    seen: Vec<bool>,
    n: usize,
    batch_size: usize,
    aborted: bool,
}

// --- pass-2: catena di seed/visitor che scrivono nei builder ----------------

/// Top: FeatureCollection; interessa solo "features".
struct TopSink<'a> {
    sink: &'a mut RowSink,
}
impl<'a, 'de> Visitor<'de> for TopSink<'a> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un oggetto GeoJSON")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            if key == "features" {
                map.next_value_seed(FeaturesSink { sink: self.sink })?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }
}

struct FeaturesSink<'a> {
    sink: &'a mut RowSink,
}
impl<'a, 'de> DeserializeSeed<'de> for FeaturesSink<'a> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}
impl<'a, 'de> Visitor<'de> for FeaturesSink<'a> {
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
impl<'a, 'de> DeserializeSeed<'de> for FeatureSink<'a> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_map(self)
    }
}
impl<'a, 'de> Visitor<'de> for FeatureSink<'a> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un Feature")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
        for s in self.sink.seen.iter_mut() {
            *s = false;
        }
        let mut geom_seen = false;
        while let Some(fk) = map.next_key_seed(FeatKeySeed)? {
            match fk {
                FeatKey::Geom if geom_seen => {
                    // "geometry" duplicata nella stessa feature: consuma e ignora
                    // (tiene la 1ª, evita una seconda append → colonne disallineate).
                    map.next_value::<IgnoredAny>()?;
                }
                FeatKey::Geom => {
                    let g = map.next_value::<Option<GjGeometry>>()?;
                    match g {
                        None => self.sink.geom.append_null(),
                        Some(gj) => {
                            self.sink.wkb_buf.clear();
                            wkb_from_gj_value(&gj.value, &mut self.sink.wkb_buf)
                                .map_err(<A::Error as DeError>::custom)?;
                            self.sink.geom.append_value(&self.sink.wkb_buf);
                        }
                    }
                    geom_seen = true;
                }
                FeatKey::Props => {
                    map.next_value_seed(PropsSink { sink: self.sink })?;
                }
                FeatKey::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        // Allinea le colonne: una append per builder per feature.
        if !geom_seen {
            self.sink.geom.append_null();
        }
        for i in 0..self.sink.builders.len() {
            if !self.sink.seen[i] {
                self.sink.builders[i].append_null();
            }
        }
        self.sink.n += 1;
        if self.sink.n >= self.sink.batch_size {
            match finish_batch(
                &self.sink.schema,
                &mut self.sink.geom,
                &mut self.sink.builders,
            ) {
                Ok(batch) => {
                    if self.sink.tx.send(Ok(batch)).is_err() {
                        self.sink.aborted = true;
                        return Err(<A::Error as DeError>::custom("consumatore chiuso"));
                    }
                }
                Err(e) => return Err(<A::Error as DeError>::custom(e)),
            }
            self.sink.n = 0;
        }
        Ok(())
    }
}

/// Chiave a livello di Feature, riconosciuta senza allocare la stringa.
enum FeatKey {
    Geom,
    Props,
    Other,
}
struct FeatKeySeed;
impl<'de> DeserializeSeed<'de> for FeatKeySeed {
    type Value = FeatKey;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<FeatKey, D::Error> {
        d.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for FeatKeySeed {
    type Value = FeatKey;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("una chiave di Feature")
    }
    fn visit_str<E>(self, s: &str) -> std::result::Result<FeatKey, E> {
        Ok(match s {
            "geometry" => FeatKey::Geom,
            "properties" => FeatKey::Props,
            _ => FeatKey::Other,
        })
    }
}

struct PropsSink<'a> {
    sink: &'a mut RowSink,
}
impl<'a, 'de> DeserializeSeed<'de> for PropsSink<'a> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        // properties può essere null (→ tutte le colonne restano non-viste = null).
        d.deserialize_any(self)
    }
}
impl<'a, 'de> Visitor<'de> for PropsSink<'a> {
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
        })? {
            match hit {
                // Chiave nota e non ancora vista in questa feature: append.
                Some(idx) if !self.sink.seen[idx] => {
                    map.next_value_seed(ValueSink {
                        b: &mut self.sink.builders[idx],
                    })?;
                    self.sink.seen[idx] = true;
                }
                // Chiave duplicata (o sconosciuta): consuma e ignora il valore.
                // La 1ª occorrenza vince; niente doppia append → colonne allineate.
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

/// Chiave di proprietà → indice di colonna (o None se non è nello schema).
/// La stringa non viene allocata: la lookup avviene dentro `visit_str`.
struct PropKeySeed<'a> {
    col_idx: &'a HashMap<String, usize>,
}
impl<'a, 'de> DeserializeSeed<'de> for PropKeySeed<'a> {
    type Value = Option<usize>;
    fn deserialize<D: Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Option<usize>, D::Error> {
        d.deserialize_str(self)
    }
}
impl<'a, 'de> Visitor<'de> for PropKeySeed<'a> {
    type Value = Option<usize>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("una chiave di proprietà")
    }
    fn visit_str<E>(self, s: &str) -> std::result::Result<Option<usize>, E> {
        Ok(self.col_idx.get(s).copied())
    }
}

/// Appende il valore di una proprietà DIRETTAMENTE nel builder tipizzato,
/// senza materializzare un `serde_json::Value` per gli scalari (il caso caldo).
struct ValueSink<'a> {
    b: &'a mut ColBuilder,
}
impl<'a, 'de> DeserializeSeed<'de> for ValueSink<'a> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
        d.deserialize_any(self)
    }
}
impl<'a, 'de> Visitor<'de> for ValueSink<'a> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("un valore di proprietà")
    }
    fn visit_i64<E>(self, v: i64) -> std::result::Result<(), E> {
        match self.b {
            ColBuilder::I64(b) => b.append_value(v),
            ColBuilder::F64(b) => b.append_value(v as f64),
            ColBuilder::Bool(b) => b.append_null(),
            ColBuilder::Str(b) => b.append_value(v.to_string()),
        }
        Ok(())
    }
    fn visit_u64<E>(self, v: u64) -> std::result::Result<(), E> {
        match self.b {
            ColBuilder::I64(b) => b.append_option(i64::try_from(v).ok()),
            ColBuilder::F64(b) => b.append_value(v as f64),
            ColBuilder::Bool(b) => b.append_null(),
            ColBuilder::Str(b) => b.append_value(v.to_string()),
        }
        Ok(())
    }
    fn visit_f64<E>(self, v: f64) -> std::result::Result<(), E> {
        match self.b {
            ColBuilder::I64(b) => b.append_null(),
            ColBuilder::F64(b) => b.append_value(v),
            ColBuilder::Bool(b) => b.append_null(),
            ColBuilder::Str(b) => b.append_value(v.to_string()),
        }
        Ok(())
    }
    fn visit_bool<E>(self, v: bool) -> std::result::Result<(), E> {
        match self.b {
            ColBuilder::Bool(b) => b.append_value(v),
            ColBuilder::Str(b) => b.append_value(v.to_string()),
            ColBuilder::I64(b) => b.append_null(),
            ColBuilder::F64(b) => b.append_null(),
        }
        Ok(())
    }
    fn visit_str<E>(self, s: &str) -> std::result::Result<(), E> {
        match self.b {
            ColBuilder::Str(b) => b.append_value(s), // caso caldo: 0 alloc extra
            ColBuilder::I64(b) => b.append_null(),
            ColBuilder::F64(b) => b.append_null(),
            ColBuilder::Bool(b) => b.append_null(),
        }
        Ok(())
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
    // esattamente come faceva il percorso precedente (`ColBuilder::append`).
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> std::result::Result<(), A::Error> {
        let v = JsonValue::deserialize(SeqAccessDeserializer::new(seq))?;
        self.b.append(Some(&v));
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> std::result::Result<(), A::Error> {
        let v = JsonValue::deserialize(MapAccessDeserializer::new(map))?;
        self.b.append(Some(&v));
        Ok(())
    }
}

fn finish_batch(
    schema: &SchemaRef,
    geom: &mut BinaryBuilder,
    builders: &mut [ColBuilder],
) -> std::result::Result<RecordBatch, String> {
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(1 + builders.len());
    arrays.push(Arc::new(geom.finish()));
    for b in builders.iter_mut() {
        arrays.push(b.finish());
    }
    RecordBatch::try_new(schema.clone(), arrays).map_err(|e| format!("record batch: {e}"))
}

/// Entry point per il fuzzer (NON API stabile): esegue pass-1 + pass-2 in modo
/// **sincrono** (niente thread → un panic è catturabile dal fuzzer) su `bytes`,
/// con un unico batch finale. JSON invalido → `Err` (rifiuto pulito). Un
/// disallineamento delle colonne fa fallire `try_new` → panic via `expect`,
/// segnalato come bug del bookkeeping per-feature.
#[doc(hidden)]
pub fn __fuzz_read_geojson(bytes: &[u8]) -> std::result::Result<usize, String> {
    use std::io::Cursor;
    // pass-1: schema
    let mut accs: BTreeMap<String, KeyAcc> = BTreeMap::new();
    serde_json::Deserializer::from_reader(Cursor::new(bytes))
        .deserialize_map(TopVisitor { accs: &mut accs })
        .map_err(|e| e.to_string())?;
    let cols: Vec<(String, ColType)> = accs.iter().map(|(k, a)| (k.clone(), a.coltype())).collect();
    let mut fields = vec![geometry_field(GEOMETRY, OGC_CRS84)];
    for (k, ct) in &cols {
        fields.push(Field::new(k, coltype_to_dt(*ct), true));
    }
    let schema: SchemaRef = Arc::new(Schema::new(fields));
    // pass-2 sincrono: batch_size enorme → nessun flush intermedio sul canale.
    let ncols = cols.len();
    let col_idx: HashMap<String, usize> = cols
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (k.clone(), i))
        .collect();
    let (tx, _rx) = sync_channel::<std::result::Result<RecordBatch, String>>(1);
    let mut sink = RowSink {
        schema: schema.clone(),
        col_idx,
        tx,
        geom: BinaryBuilder::new(),
        wkb_buf: Vec::new(),
        builders: cols.iter().map(|(_, ct)| ColBuilder::new(*ct)).collect(),
        seen: vec![false; ncols],
        n: 0,
        batch_size: usize::MAX,
        aborted: false,
    };
    serde_json::Deserializer::from_reader(Cursor::new(bytes))
        .deserialize_map(TopSink { sink: &mut sink })
        .map_err(|e| e.to_string())?;
    if sink.n > 0 {
        let batch = finish_batch(&schema, &mut sink.geom, &mut sink.builders)
            .expect("colonne disallineate dopo pass-2 (bug bookkeeping)");
        return Ok(batch.num_rows());
    }
    Ok(0)
}

enum ColBuilder {
    I64(Int64Builder),
    F64(Float64Builder),
    Bool(BooleanBuilder),
    Str(StringBuilder),
}

impl ColBuilder {
    fn new(ct: ColType) -> Self {
        match ct {
            ColType::Integer => ColBuilder::I64(Int64Builder::new()),
            ColType::Number => ColBuilder::F64(Float64Builder::new()),
            ColType::Boolean => ColBuilder::Bool(BooleanBuilder::new()),
            ColType::Text => ColBuilder::Str(StringBuilder::new()),
        }
    }
    fn append(&mut self, v: Option<&JsonValue>) {
        match self {
            ColBuilder::I64(b) => b.append_option(v.and_then(JsonValue::as_i64)),
            ColBuilder::F64(b) => b.append_option(v.and_then(JsonValue::as_f64)),
            ColBuilder::Bool(b) => b.append_option(v.and_then(JsonValue::as_bool)),
            ColBuilder::Str(b) => match v {
                None | Some(JsonValue::Null) => b.append_null(),
                Some(JsonValue::String(s)) => b.append_value(s),
                Some(other) => b.append_value(other.to_string()),
            },
        }
    }
    fn append_null(&mut self) {
        match self {
            ColBuilder::I64(b) => b.append_null(),
            ColBuilder::F64(b) => b.append_null(),
            ColBuilder::Bool(b) => b.append_null(),
            ColBuilder::Str(b) => b.append_null(),
        }
    }
    fn finish(&mut self) -> ArrayRef {
        match self {
            ColBuilder::I64(b) => Arc::new(b.finish()),
            ColBuilder::F64(b) => Arc::new(b.finish()),
            ColBuilder::Bool(b) => Arc::new(b.finish()),
            ColBuilder::Str(b) => Arc::new(b.finish()),
        }
    }
}

// --- scrittura (bufferizzante nella v1) -----------------------------------

struct GeoJsonWriter {
    temp: Option<tempfile::NamedTempFile>,
    writer: Option<BufWriter<File>>,
    path: PathBuf,
    durable: bool,
    first: bool,
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
}

impl FormatWriter for GeoJsonWriter {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let schema = batch.schema();
        let geom_idx = schema
            .fields()
            .iter()
            .position(|f| is_geometry_field(f))
            .ok_or_else(|| err("nessuna colonna geometria geoarrow.wkb"))?;
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
        let temp = self.temp.take().ok_or_else(|| err("temp mancante"))?;
        let (bytes, outcome) =
            publish_file_atomic_limited(temp, &self.path, self.durable, self.max_output_bytes)?;
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
        write_wkb_geojson(w, &geom)?;
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
            w.write_all(b"null")?; // NaN/Inf non sono JSON validi
        }
    } else if let Some(a) = col.as_any().downcast_ref::<BooleanArray>() {
        w.write_all(if a.value(row) { b"true" } else { b"false" })?;
    } else {
        serde_json::to_writer(&mut *w, &json_from_array(col, row))
            .map_err(|e| err(e.to_string()))?;
    }
    Ok(())
}

// --- geometria: conversione diretta senza intermedio geo_types/Value --------

/// Emette WKB ISO XY/XYZ little-endian da una `geojson::Value` (lettura), senza
/// passare da geo_types. Le posizioni devono avere dimensionalità uniforme:
/// una quarta ordinata è rifiutata perché GeoJSON non le assegna una semantica
/// M interoperabile.
#[doc(hidden)] // esposto anche per il fuzzer (plenora-fuzz)
pub fn wkb_from_gj_value(v: &geojson::Value, out: &mut Vec<u8>) -> std::result::Result<(), String> {
    use geojson::Value::*;

    fn observe_position(
        position: &[f64],
        dimensions: &mut Option<CoordinateDimensions>,
    ) -> std::result::Result<(), String> {
        let current = match position.len() {
            2 => CoordinateDimensions::Xy,
            3 => CoordinateDimensions::Xyz,
            n => {
                return Err(format!(
                    "posizione GeoJSON con {n} ordinate: attese esattamente 2 o 3"
                ))
            }
        };
        if dimensions.is_some_and(|known| known != current) {
            return Err("dimensionalità GeoJSON non uniforme".to_owned());
        }
        *dimensions = Some(current);
        Ok(())
    }

    fn scan(
        value: &geojson::Value,
        dimensions: &mut Option<CoordinateDimensions>,
    ) -> std::result::Result<(), String> {
        match value {
            Point(position) => observe_position(position, dimensions)?,
            LineString(positions) | MultiPoint(positions) => {
                for position in positions {
                    observe_position(position, dimensions)?;
                }
            }
            Polygon(rings) | MultiLineString(rings) => {
                for ring in rings {
                    for position in ring {
                        observe_position(position, dimensions)?;
                    }
                }
            }
            MultiPolygon(polygons) => {
                for polygon in polygons {
                    for ring in polygon {
                        for position in ring {
                            observe_position(position, dimensions)?;
                        }
                    }
                }
            }
            GeometryCollection(geometries) => {
                for geometry in geometries {
                    scan(&geometry.value, dimensions)?;
                }
            }
        }
        Ok(())
    }

    fn hdr(out: &mut Vec<u8>, code: u32, dimensions: CoordinateDimensions) {
        out.push(1);
        let dimensional_code = match dimensions {
            CoordinateDimensions::Xy => code,
            CoordinateDimensions::Xyz => code + 1000,
            _ => unreachable!("la scansione GeoJSON produce solo XY/XYZ"),
        };
        out.extend_from_slice(&dimensional_code.to_le_bytes());
    }

    fn pos(
        out: &mut Vec<u8>,
        p: &[f64],
        dimensions: CoordinateDimensions,
    ) -> std::result::Result<(), String> {
        let x = *p.first().ok_or("posizione GeoJSON senza x")?;
        let y = *p.get(1).ok_or("posizione GeoJSON senza y")?;
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
        if dimensions == CoordinateDimensions::Xyz {
            out.extend_from_slice(
                &p.get(2)
                    .ok_or("posizione GeoJSON XYZ senza z")?
                    .to_le_bytes(),
            );
        }
        Ok(())
    }

    fn ring(
        out: &mut Vec<u8>,
        r: &[Vec<f64>],
        dimensions: CoordinateDimensions,
    ) -> std::result::Result<(), String> {
        out.extend_from_slice(&(r.len() as u32).to_le_bytes());
        for p in r {
            pos(out, p, dimensions)?;
        }
        Ok(())
    }

    fn write_value(
        value: &geojson::Value,
        out: &mut Vec<u8>,
        dimensions: CoordinateDimensions,
    ) -> std::result::Result<(), String> {
        match value {
            Point(p) => {
                hdr(out, 1, dimensions);
                pos(out, p, dimensions)?;
            }
            LineString(ls) => {
                hdr(out, 2, dimensions);
                ring(out, ls, dimensions)?;
            }
            Polygon(rings) => {
                hdr(out, 3, dimensions);
                out.extend_from_slice(&(rings.len() as u32).to_le_bytes());
                for r in rings {
                    ring(out, r, dimensions)?;
                }
            }
            MultiPoint(pts) => {
                hdr(out, 4, dimensions);
                out.extend_from_slice(&(pts.len() as u32).to_le_bytes());
                for p in pts {
                    hdr(out, 1, dimensions);
                    pos(out, p, dimensions)?;
                }
            }
            MultiLineString(lss) => {
                hdr(out, 5, dimensions);
                out.extend_from_slice(&(lss.len() as u32).to_le_bytes());
                for ls in lss {
                    hdr(out, 2, dimensions);
                    ring(out, ls, dimensions)?;
                }
            }
            MultiPolygon(polys) => {
                hdr(out, 6, dimensions);
                out.extend_from_slice(&(polys.len() as u32).to_le_bytes());
                for poly in polys {
                    hdr(out, 3, dimensions);
                    out.extend_from_slice(&(poly.len() as u32).to_le_bytes());
                    for r in poly {
                        ring(out, r, dimensions)?;
                    }
                }
            }
            GeometryCollection(gs) => {
                hdr(out, 7, dimensions);
                out.extend_from_slice(&(gs.len() as u32).to_le_bytes());
                for geometry in gs {
                    write_value(&geometry.value, out, dimensions)?;
                }
            }
        }
        Ok(())
    }

    let mut dimensions = None;
    scan(v, &mut dimensions)?;
    write_value(v, out, dimensions.unwrap_or(CoordinateDimensions::Xy))
}

/// Scrive direttamente il modello WKB lossless come GeoJSON, preservando Z.
fn write_wkb_geojson<W: Write>(w: &mut W, geometry: &WkbGeometry) -> Result<()> {
    fn position<W: Write>(
        w: &mut W,
        coordinate: &WkbCoordinate,
        dimensions: CoordinateDimensions,
    ) -> Result<()> {
        if !coordinate.x.is_finite()
            || !coordinate.y.is_finite()
            || coordinate.z.is_some_and(|z| !z.is_finite())
        {
            return Err(err("coordinata non finita non rappresentabile in GeoJSON"));
        }
        w.write_all(b"[")?;
        serde_json::to_writer(&mut *w, &coordinate.x).map_err(|e| err(e.to_string()))?;
        w.write_all(b",")?;
        serde_json::to_writer(&mut *w, &coordinate.y).map_err(|e| err(e.to_string()))?;
        if dimensions == CoordinateDimensions::Xyz {
            w.write_all(b",")?;
            serde_json::to_writer(
                &mut *w,
                &coordinate
                    .z
                    .ok_or_else(|| err("coordinata XYZ senza ordinata z"))?,
            )
            .map_err(|e| err(e.to_string()))?;
        }
        w.write_all(b"]")?;
        Ok(())
    }

    fn positions<W: Write>(
        w: &mut W,
        coordinates: &[WkbCoordinate],
        dimensions: CoordinateDimensions,
    ) -> Result<()> {
        w.write_all(b"[")?;
        for (index, coordinate) in coordinates.iter().enumerate() {
            if index > 0 {
                w.write_all(b",")?;
            }
            position(w, coordinate, dimensions)?;
        }
        w.write_all(b"]")?;
        Ok(())
    }

    fn polygon<W: Write>(
        w: &mut W,
        rings: &[Vec<WkbCoordinate>],
        dimensions: CoordinateDimensions,
    ) -> Result<()> {
        w.write_all(b"[")?;
        for (index, ring) in rings.iter().enumerate() {
            if index > 0 {
                w.write_all(b",")?;
            }
            positions(w, ring, dimensions)?;
        }
        w.write_all(b"]")?;
        Ok(())
    }

    if !matches!(
        geometry.dimensions,
        CoordinateDimensions::Xy | CoordinateDimensions::Xyz
    ) {
        return Err(err("GeoJSON supporta solo coordinate XY o XYZ"));
    }
    let dimensions = geometry.dimensions;
    match &geometry.value {
        WkbValue::Point(coordinate) => {
            w.write_all(b"{\"type\":\"Point\",\"coordinates\":")?;
            position(w, coordinate, dimensions)?;
            w.write_all(b"}")?;
        }
        WkbValue::LineString(coordinates) => {
            w.write_all(b"{\"type\":\"LineString\",\"coordinates\":")?;
            positions(w, coordinates, dimensions)?;
            w.write_all(b"}")?;
        }
        WkbValue::Polygon(rings) => {
            w.write_all(b"{\"type\":\"Polygon\",\"coordinates\":")?;
            polygon(w, rings, dimensions)?;
            w.write_all(b"}")?;
        }
        WkbValue::MultiPoint(points) => {
            w.write_all(b"{\"type\":\"MultiPoint\",\"coordinates\":[")?;
            for (index, point) in points.iter().enumerate() {
                if index > 0 {
                    w.write_all(b",")?;
                }
                let WkbValue::Point(coordinate) = &point.value else {
                    return Err(err("MultiPoint WKB con membro non-Point"));
                };
                position(w, coordinate, dimensions)?;
            }
            w.write_all(b"]}")?;
        }
        WkbValue::MultiLineString(lines) => {
            w.write_all(b"{\"type\":\"MultiLineString\",\"coordinates\":[")?;
            for (index, line) in lines.iter().enumerate() {
                if index > 0 {
                    w.write_all(b",")?;
                }
                let WkbValue::LineString(coordinates) = &line.value else {
                    return Err(err("MultiLineString WKB con membro non-LineString"));
                };
                positions(w, coordinates, dimensions)?;
            }
            w.write_all(b"]}")?;
        }
        WkbValue::MultiPolygon(polygons) => {
            w.write_all(b"{\"type\":\"MultiPolygon\",\"coordinates\":[")?;
            for (index, value) in polygons.iter().enumerate() {
                if index > 0 {
                    w.write_all(b",")?;
                }
                let WkbValue::Polygon(rings) = &value.value else {
                    return Err(err("MultiPolygon WKB con membro non-Polygon"));
                };
                polygon(w, rings, dimensions)?;
            }
            w.write_all(b"]}")?;
        }
        WkbValue::GeometryCollection(geometries) => {
            w.write_all(b"{\"type\":\"GeometryCollection\",\"geometries\":[")?;
            for (index, value) in geometries.iter().enumerate() {
                if index > 0 {
                    w.write_all(b",")?;
                }
                write_wkb_geojson(w, value)?;
            }
            w.write_all(b"]}")?;
        }
    }
    Ok(())
}

/// Scrive una geometria geo_types come oggetto GeoJSON DIRETTAMENTE nel writer
/// (scrittura), senza costruire un `geojson::Value` intermedio.
#[doc(hidden)] // esposto anche per il fuzzer (plenora-fuzz)
pub fn write_geo_geojson<W: Write>(w: &mut W, g: &geo_types::Geometry<f64>) -> Result<()> {
    use geo_types::Geometry as G;
    match g {
        G::Point(p) => {
            w.write_all(b"{\"type\":\"Point\",\"coordinates\":")?;
            write_pos(w, p.x(), p.y())?;
            w.write_all(b"}")?;
        }
        G::LineString(ls) => {
            w.write_all(b"{\"type\":\"LineString\",\"coordinates\":")?;
            write_line(w, ls)?;
            w.write_all(b"}")?;
        }
        G::Polygon(pl) => {
            w.write_all(b"{\"type\":\"Polygon\",\"coordinates\":")?;
            write_poly(w, pl)?;
            w.write_all(b"}")?;
        }
        G::MultiPoint(mp) => {
            w.write_all(b"{\"type\":\"MultiPoint\",\"coordinates\":[")?;
            for (i, p) in mp.0.iter().enumerate() {
                if i > 0 {
                    w.write_all(b",")?;
                }
                write_pos(w, p.x(), p.y())?;
            }
            w.write_all(b"]}")?;
        }
        G::MultiLineString(ml) => {
            w.write_all(b"{\"type\":\"MultiLineString\",\"coordinates\":[")?;
            for (i, ls) in ml.0.iter().enumerate() {
                if i > 0 {
                    w.write_all(b",")?;
                }
                write_line(w, ls)?;
            }
            w.write_all(b"]}")?;
        }
        G::MultiPolygon(mp) => {
            w.write_all(b"{\"type\":\"MultiPolygon\",\"coordinates\":[")?;
            for (i, pl) in mp.0.iter().enumerate() {
                if i > 0 {
                    w.write_all(b",")?;
                }
                write_poly(w, pl)?;
            }
            w.write_all(b"]}")?;
        }
        G::GeometryCollection(gc) => {
            w.write_all(b"{\"type\":\"GeometryCollection\",\"geometries\":[")?;
            for (i, gg) in gc.0.iter().enumerate() {
                if i > 0 {
                    w.write_all(b",")?;
                }
                write_geo_geojson(w, gg)?;
            }
            w.write_all(b"]}")?;
        }
        _ => return Err(err("geometria con Z/M non rappresentabile in GeoJSON 2D")),
    }
    Ok(())
}

fn write_pos<W: Write>(w: &mut W, x: f64, y: f64) -> Result<()> {
    if !x.is_finite() || !y.is_finite() {
        return Err(err("coordinata non finita non rappresentabile in GeoJSON"));
    }
    // Formattazione via serde_json (ryu): sempre ri-leggibile. `write!("{}", f64)`
    // per valori estremi (es. f64::MAX) produrrebbe un decimale che serde_json
    // rifiuta in rilettura ("number out of range") — trovato dal fuzzer.
    w.write_all(b"[")?;
    serde_json::to_writer(&mut *w, &x).map_err(|e| err(e.to_string()))?;
    w.write_all(b",")?;
    serde_json::to_writer(&mut *w, &y).map_err(|e| err(e.to_string()))?;
    w.write_all(b"]")?;
    Ok(())
}
fn write_line<W: Write>(w: &mut W, ls: &geo_types::LineString<f64>) -> Result<()> {
    w.write_all(b"[")?;
    for (i, c) in ls.0.iter().enumerate() {
        if i > 0 {
            w.write_all(b",")?;
        }
        write_pos(w, c.x, c.y)?;
    }
    w.write_all(b"]")?;
    Ok(())
}
fn write_poly<W: Write>(w: &mut W, pl: &geo_types::Polygon<f64>) -> Result<()> {
    w.write_all(b"[")?;
    write_line(w, pl.exterior())?;
    for r in pl.interiors() {
        w.write_all(b",")?;
        write_line(w, r)?;
    }
    w.write_all(b"]")?;
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
    use plenora_core::wkb::from_wkb;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

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
                batch_target: BatchTarget::default(),
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
            plenora_core::crs::AxisOrder::LongitudeLatitude
        );
        assert_eq!(batch.num_rows(), 2);
        assert!(is_geometry_field(
            &batch.schema().field_with_name("geometry").unwrap().clone()
        ));

        // scrivi verso GeoJSON e rileggi
        let out = dir.path().join("out.geojson");
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: layer.contract.clone(),
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
    fn duplicate_keys_read_without_error() {
        // Regressione (trovato dal fuzzer): chiavi duplicate in una feature
        // (JSON malformato ma accettato da molti parser). La 1ª occorrenza vince
        // e la lettura NON deve fallire con "colonne disallineate".
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
        let (batch, _layer) = read_all(&driver, &src);
        assert_eq!(batch.num_rows(), 1);
        // geometry duplicata → vince la 1ª: Point(1,2).
        let geom = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        match from_wkb(geom.value(0), &WkbLimits::default()).unwrap() {
            geo_types::Geometry::Point(p) => assert!((p.x() - 1.0).abs() < 1e-9),
            other => panic!("atteso Point(1,2), {other:?}"),
        }
        // 'c' compare due volte (int poi bool → colonna Text): vince "1".
        let schema = batch.schema();
        let c = batch
            .column(schema.index_of("c").unwrap())
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(c.value(0), "1");
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
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: layer.contract.clone(),
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

        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "xyz".to_owned(),
                contract: layer.contract,
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
                assert_eq!(c[0], f64::MAX);
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
            s.push_str(&format!(
                "{{\"type\":\"Feature\",\"geometry\":{{\"type\":\"Point\",\"coordinates\":[{i},{i}]}},\"properties\":{{\"id\":{i}}}}}"
            ));
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
                batch_target: BatchTarget {
                    target_bytes: 8 * 1024 * 1024,
                    max_rows: 4,
                },
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

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

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Float64Builder, Int64Builder, StringBuilder,
};
use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use geojson::{Feature, FeatureReader, Geometry as GjGeometry, JsonObject};
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value as JsonValue;

use driver_common::{geometry_field, json_from_array, ColType, OGC_CRS84};
use plenora_core::contract::{DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId};
use plenora_core::crs::ResolvedCrs;
use plenora_core::geometry::is_geometry_field;
use plenora_core::limits::WkbLimits;
use plenora_core::wkb::{from_wkb, to_wkb};
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
use plenora_io_core::publish::publish_file_atomic;
use plenora_io_core::request::ReadRequest;
use plenora_io_core::WritePlan;

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
    crs_handling: CrsHandling::FixedWgs84,
    fidelity_class: Fidelity::Lossless,
    runtime: Runtime::PureRust,
    semantic_version: 1,
    driver_version: 2,
    descriptor_version: 1,
};

pub struct GeoJsonDriver;

impl FormatDriver for GeoJsonDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, _opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let Source::Path(path) = source;
        // Pass 1: inferenza schema in streaming (RAM O(1)).
        let (schema, cols) = infer_schema(&path)?;
        let contract = DataContract {
            schema: schema.clone(),
            geometry: Some(GeometryColumnContract {
                field_id: FieldId(0),
                name: GEOMETRY.to_owned(),
                crs: ResolvedCrs::wgs84(),
                nullable: true,
            }),
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
        Ok(Box::new(GeoJsonWriter {
            temp: Some(temp),
            writer: Some(writer),
            path,
            durable: opts.durable,
            first: true,
        }))
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
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        let batch_size = request.batch_target.max_rows.max(1);
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

fn feature_reader(path: &Path) -> Result<FeatureReader<BufReader<File>>> {
    let file = File::open(path)?;
    Ok(FeatureReader::from_reader(BufReader::new(file)))
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
    let cols: Vec<(String, ColType)> =
        accs.iter().map(|(k, a)| (k.clone(), a.coltype())).collect();
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
        d.deserialize_map(PropsVisitor { accs: self.accs })
    }
}
struct PropsVisitor<'a> {
    accs: &'a mut BTreeMap<String, KeyAcc>,
}
impl<'a, 'de> Visitor<'de> for PropsVisitor<'a> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("le properties di un Feature")
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
            let reader = feature_reader(&path).map_err(|e| e.to_string())?;
            let mut geom = BinaryBuilder::new();
            let mut builders: Vec<ColBuilder> =
                cols.iter().map(|(_, ct)| ColBuilder::new(*ct)).collect();
            let mut n = 0usize;
            for feat in reader.features() {
                let f = feat.map_err(|e| format!("GeoJSON non valido: {e}"))?;
                let Feature {
                    geometry,
                    properties,
                    ..
                } = f;
                match geometry {
                    None => geom.append_null(),
                    Some(g) => {
                        let gt = geo_types::Geometry::<f64>::try_from(g.value)
                            .map_err(|e| format!("geometria non convertibile: {e}"))?;
                        geom.append_value(to_wkb(&gt).map_err(|e| e.to_string())?);
                    }
                }
                for (i, (k, _)) in cols.iter().enumerate() {
                    builders[i].append(properties.as_ref().and_then(|p| p.get(k)));
                }
                n += 1;
                if n >= batch_size {
                    let batch = finish_batch(&schema, &mut geom, &mut builders)?;
                    if tx.send(Ok(batch)).is_err() {
                        return Ok(()); // consumatore andato via
                    }
                    n = 0;
                }
            }
            if n > 0 {
                let batch = finish_batch(&schema, &mut geom, &mut builders)?;
                let _ = tx.send(Ok(batch));
            }
            Ok(())
        };
        if let Err(e) = run() {
            let _ = tx.send(Err(e));
        }
    });
    rx
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
        let limits = WkbLimits::default();
        let w = self.writer.as_mut().ok_or_else(|| err("writer chiuso"))?;
        let mut first = self.first;
        for row in 0..batch.num_rows() {
            let feature = row_to_feature(&schema, geom_idx, geom_col, batch, row, &limits)?;
            if !first {
                w.write_all(b",")?;
            }
            first = false;
            serde_json::to_writer(&mut *w, &feature)?;
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
        let (bytes, outcome) = publish_file_atomic(temp, &self.path, self.durable)?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            outcome,
        })
    }
}

/// Un feature dalla riga `row` (geometria da WKB, proprietà da colonne Arrow).
fn row_to_feature(
    schema: &SchemaRef,
    geom_idx: usize,
    geom_col: &BinaryArray,
    batch: &RecordBatch,
    row: usize,
    limits: &WkbLimits,
) -> Result<Feature> {
    let geometry = if geom_col.is_null(row) {
        None
    } else {
        let geom = from_wkb(geom_col.value(row), limits)?;
        Some(GjGeometry::new(geojson::Value::from(&geom)))
    };
    let mut props = JsonObject::new();
    for (i, field) in schema.fields().iter().enumerate() {
        if i == geom_idx {
            continue;
        }
        props.insert(field.name().clone(), json_from_array(batch.column(i), row));
    }
    Ok(Feature {
        bbox: None,
        geometry,
        id: None,
        properties: Some(props),
        foreign_members: None,
    })
}

// Solo per i test: parse completo (documenti piccoli).
#[cfg(test)]
fn parse_features(text: &str) -> Result<Vec<Feature>> {
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
            layer.contract.geometry.as_ref().unwrap().crs.id.as_deref(),
            Some("OGC:CRS84")
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
        assert!(batches >= 3, "atteso streaming multi-batch, avuti {batches}");
    }
}

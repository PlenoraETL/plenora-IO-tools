//! driver-filegdb — FileGDB → RecordBatch (Fase 1, read-only, "tier GDB"). È
//! l'unica eccezione alla policy puro-Rust: FileGDB richiede GDAL. Dietro la
//! feature `gdal-backend` legge via GDAL; senza feature è uno stub che fallisce
//! tipizzato (il binario di default resta puro-Rust). Multi-layer.
#![forbid(unsafe_code)]

use plenora_core::Result;
use plenora_io_core::descriptor::{
    CrsHandling, Direction, Fidelity, FormatDescriptor, ReadMode, ReaderConcurrency, Runtime,
    WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, OpenDatasetHandle, ReadOptions, Sink, Source, WriteOptions,
};
use plenora_io_core::{
    validate_write, AttributeWriteSupport, CrsWriteSupport, FormatWriteCapabilities,
    NullabilitySupport, TypeCoercionPolicy, WritePlan, SCALAR_TYPES, UTF8_FIELD_NAMES,
    WKB_PASSTHROUGH_GEOMETRY,
};

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "filegdb",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::Materializing,
    write_mode: Some(WriteMode::Streaming),
    multi_layer: true,
    multi_file: true, // una .gdb è una directory
    reader_concurrency: ReaderConcurrency::SingleActiveReader,
    projection_support: plenora_io_core::ProjectionSupport::None,
    crs_handling: CrsHandling::Embedded,
    fidelity_class: Fidelity::Lossless,
    runtime: Runtime::Gdal,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_PASSTHROUGH_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        nullability: NullabilitySupport::Preserve,
        multi_layer: true,
    }),
    semantic_version: 1,
    driver_version: 1,
    descriptor_version: 3,
};

pub struct FileGdbDriver;

impl FormatDriver for FileGdbDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    // I `return` cfg-gated servono per il caso feature-on (il blocco feature-off,
    // pur rimosso, segue sintatticamente); clippy non lo coglie.
    #[allow(clippy::needless_return)]
    fn open(&self, source: Source, _opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        #[cfg(feature = "gdal-backend")]
        {
            let path = source.into_path_checked(&_opts.limits)?;
            return backend::open(&path);
        }
        #[cfg(not(feature = "gdal-backend"))]
        {
            let _ = source;
            Err(plenora_core::PlenoraError::Unsupported(
                "FileGDB richiede il tier GDB: compilare con --features gdal-backend".to_owned(),
            ))
        }
    }

    #[allow(clippy::needless_return)]
    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(self.descriptor(), plan, &opts.limits)?;
        #[cfg(feature = "gdal-backend")]
        {
            let Sink::Path(path) = sink;
            return backend::create(&path, plan, opts).map(|writer| {
                plenora_io_core::with_write_validation(writer, self.descriptor(), plan, opts.limits)
            });
        }
        #[cfg(not(feature = "gdal-backend"))]
        {
            let _ = (sink, plan, opts);
            Err(plenora_core::PlenoraError::Unsupported(
                "scrittura FileGDB richiede il tier GDB: compilare con --features gdal-backend"
                    .to_owned(),
            ))
        }
    }
}

#[cfg(feature = "gdal-backend")]
mod backend {
    use std::sync::mpsc::{sync_channel, Receiver};
    use std::sync::Arc;

    use arrow_array::builder::{BinaryBuilder, Float64Builder, Int64Builder, StringBuilder};
    use arrow_array::{ArrayRef, BinaryArray, RecordBatch};
    use arrow_schema::{Field, Schema, SchemaRef};
    use gdal::vector::LayerAccess;
    use gdal::Dataset;

    use driver_common::{geometry_field, json_from_array};
    use plenora_core::contract::{
        DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId,
    };
    use plenora_core::crs::{CrsKind, ResolvedCrs};
    use plenora_core::{PlenoraError, Result};
    use plenora_io_core::driver::{LayerReader, OpenDatasetHandle};
    use plenora_io_core::request::ReadRequest;

    // --- scrittura (tier GDB via GDAL OpenFileGDB) --------------------------
    use std::path::{Path, PathBuf};

    use arrow_array::Array;
    use arrow_schema::DataType;
    use gdal::spatial_ref::SpatialRef;
    use gdal::vector::{Geometry, LayerOptions, OGRwkbGeometryType};
    use gdal::DriverManager;

    use plenora_core::geometry::is_geometry_field;
    use plenora_io_core::driver::{FormatWriter, Published, WriteOptions};
    use plenora_io_core::loss::LossReport;
    use plenora_io_core::publish::publish_dir_atomic;
    use plenora_io_core::{SingleReaderGate, WriteLayer, WritePlan};
    use serde_json::Value as JsonValueW;

    #[derive(Clone, Copy)]
    enum FieldKind {
        Int,
        Real,
        Str,
        Bool,
    }

    impl FieldKind {
        fn from(dt: &DataType) -> Self {
            match dt {
                DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64 => FieldKind::Int,
                DataType::Float16 | DataType::Float32 | DataType::Float64 => FieldKind::Real,
                DataType::Boolean => FieldKind::Bool,
                _ => FieldKind::Str,
            }
        }
        fn ogr(self) -> gdal::vector::OGRFieldType::Type {
            match self {
                FieldKind::Int => gdal::vector::OGRFieldType::OFTInteger64,
                FieldKind::Real => gdal::vector::OGRFieldType::OFTReal,
                FieldKind::Bool => gdal::vector::OGRFieldType::OFTInteger,
                FieldKind::Str => gdal::vector::OGRFieldType::OFTString,
            }
        }
    }

    struct PlanLayer {
        name: String,
        geom_idx: usize,
        fields: Vec<(String, usize, FieldKind)>,
        epsg: Option<u32>,
        gdal_idx: Option<usize>,
    }

    struct GdbWriter {
        ds: Dataset,
        staging: PathBuf,
        dest: PathBuf,
        durable: bool,
        max_output_bytes: u64,
        layers: Vec<PlanLayer>,
        next_gdal_idx: usize,
    }

    fn layer_epsg(l: &WriteLayer) -> Option<u32> {
        let id = l
            .contract
            .geometry
            .as_ref()
            .and_then(|g| g.crs.id().map(str::to_owned))?;
        if id.eq_ignore_ascii_case("OGC:CRS84") {
            return Some(4326);
        }
        let (auth, code) = id.split_once(':')?;
        if auth.eq_ignore_ascii_case("EPSG") {
            return code.parse::<u32>().ok();
        }
        None
    }

    /// Tipo geometria OGR dal codice-tipo WKB (FileGDB richiede un tipo concreto).
    fn wkb_ogr_type(bytes: &[u8]) -> OGRwkbGeometryType::Type {
        if bytes.len() < 5 {
            return OGRwkbGeometryType::wkbPoint;
        }
        let raw = if bytes[0] == 1 {
            u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]])
        } else {
            u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]])
        };
        match raw % 1000 {
            1 => OGRwkbGeometryType::wkbPoint,
            2 => OGRwkbGeometryType::wkbLineString,
            3 => OGRwkbGeometryType::wkbPolygon,
            4 => OGRwkbGeometryType::wkbMultiPoint,
            5 => OGRwkbGeometryType::wkbMultiLineString,
            6 => OGRwkbGeometryType::wkbMultiPolygon,
            7 => OGRwkbGeometryType::wkbGeometryCollection,
            _ => OGRwkbGeometryType::wkbPoint,
        }
    }

    fn staging_path(dest: &Path) -> PathBuf {
        let parent = dest.parent().filter(|p| !p.as_os_str().is_empty());
        let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
        let name = format!("{stem}.plenora-tmp.gdb");
        let p = match parent {
            Some(pp) => pp.join(name),
            None => PathBuf::from(name),
        };
        if p.exists() {
            let _ = std::fs::remove_dir_all(&p);
        }
        p
    }

    fn field_value(kind: FieldKind, v: JsonValueW) -> gdal::vector::FieldValue {
        use gdal::vector::FieldValue as F;
        match kind {
            FieldKind::Int => F::Integer64Value(v.as_i64().unwrap_or(0)),
            FieldKind::Real => F::RealValue(v.as_f64().unwrap_or(0.0)),
            FieldKind::Bool => F::IntegerValue(i32::from(v.as_bool().unwrap_or(false))),
            FieldKind::Str => F::StringValue(match v {
                JsonValueW::String(s) => s,
                other => other.to_string(),
            }),
        }
    }

    fn dir_size(p: &Path) -> u64 {
        let mut total = 0;
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                match e.metadata() {
                    Ok(m) if m.is_dir() => total += dir_size(&e.path()),
                    Ok(m) => total += m.len(),
                    _ => {}
                }
            }
        }
        total
    }

    pub fn create(
        path: &Path,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        if path.exists() {
            return Err(PlenoraError::OutputExists(path.display().to_string()));
        }
        let staging = staging_path(path);
        let driver = DriverManager::get_driver_by_name("OpenFileGDB")
            .map_err(|e| err(format!("driver OpenFileGDB non disponibile: {e}")))?;
        let ds = driver
            .create_vector_only(&staging)
            .map_err(|e| err(format!("creazione FileGDB: {e}")))?;

        // I layer GDAL sono creati PIGRAMENTE al primo write: FileGDB richiede il
        // tipo geometria concreto, che conosciamo solo vedendo i dati.
        let mut infos = Vec::new();
        for l in &plan.layers {
            let schema = &l.contract.schema;
            let geom_idx = schema
                .fields()
                .iter()
                .position(|f| is_geometry_field(f))
                .ok_or_else(|| err(format!("layer '{}' senza colonna geometria", l.name)))?;
            let fields = schema
                .fields()
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != geom_idx)
                .map(|(i, f)| (f.name().clone(), i, FieldKind::from(f.data_type())))
                .collect();
            infos.push(PlanLayer {
                name: l.name.clone(),
                geom_idx,
                fields,
                epsg: layer_epsg(l),
                gdal_idx: None,
            });
        }

        Ok(Box::new(GdbWriter {
            ds,
            staging,
            dest: path.to_owned(),
            durable: opts.durable,
            max_output_bytes: opts.limits.max_output_bytes,
            layers: infos,
            next_gdal_idx: 0,
        }))
    }

    impl FormatWriter for GdbWriter {
        fn write(&mut self, batch: &RecordBatch) -> Result<()> {
            self.write_to_layer(LayerId(0), batch)
        }

        fn write_to_layer(&mut self, layer: LayerId, batch: &RecordBatch) -> Result<()> {
            let li = layer.0 as usize;
            if li >= self.layers.len() {
                return Err(err(format!("layer {} inesistente", layer.0)));
            }
            let geom_idx = self.layers[li].geom_idx;
            let geom_col = batch
                .column(geom_idx)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .ok_or_else(|| err("colonna geometria non binaria"))?;

            // Creazione pigra del layer GDAL col tipo geometria dedotto dai dati.
            if self.layers[li].gdal_idx.is_none() {
                let ty = (0..batch.num_rows())
                    .find(|&r| !geom_col.is_null(r))
                    .map(|r| wkb_ogr_type(geom_col.value(r)))
                    .unwrap_or(OGRwkbGeometryType::wkbPoint);
                let lname = self.layers[li].name.clone();
                let epsg = self.layers[li].epsg;
                let defs_owned: Vec<(String, gdal::vector::OGRFieldType::Type)> = self.layers[li]
                    .fields
                    .iter()
                    .map(|(n, _, k)| (n.clone(), k.ogr()))
                    .collect();
                let defs: Vec<(&str, gdal::vector::OGRFieldType::Type)> =
                    defs_owned.iter().map(|(n, t)| (n.as_str(), *t)).collect();
                let srs = epsg.and_then(|c| SpatialRef::from_epsg(c).ok());
                let gl = self
                    .ds
                    .create_layer(LayerOptions {
                        name: &lname,
                        srs: srs.as_ref(),
                        ty,
                        options: None,
                    })
                    .map_err(|e| err(format!("create_layer '{lname}': {e}")))?;
                gl.create_defn_fields(&defs)
                    .map_err(|e| err(format!("definizione campi '{lname}': {e}")))?;
                self.layers[li].gdal_idx = Some(self.next_gdal_idx);
                self.next_gdal_idx += 1;
            }

            let gidx = self.layers[li].gdal_idx.unwrap();
            let fields = self.layers[li].fields.clone();
            let mut gl = self
                .ds
                .layer(gidx)
                .map_err(|e| err(format!("accesso layer {gidx}: {e}")))?;
            for row in 0..batch.num_rows() {
                if geom_col.is_null(row) {
                    continue; // FileGDB: feature senza geometria saltata (v1)
                }
                let geom = Geometry::from_wkb(geom_col.value(row))
                    .map_err(|e| err(format!("WKB->GDAL: {e}")))?;
                let mut fnames: Vec<&str> = Vec::new();
                let mut fvals: Vec<gdal::vector::FieldValue> = Vec::new();
                for (n, i, kind) in &fields {
                    let v = json_from_array(batch.column(*i), row);
                    if v.is_null() {
                        continue;
                    }
                    fnames.push(n.as_str());
                    fvals.push(field_value(*kind, v));
                }
                gl.create_feature_fields(geom, &fnames, &fvals)
                    .map_err(|e| err(format!("create_feature: {e}")))?;
            }
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<Published> {
            let GdbWriter {
                ds,
                staging,
                dest,
                durable,
                max_output_bytes,
                ..
            } = *self;
            drop(ds); // chiude e flush della .gdb
            let bytes = dir_size(&staging);
            if bytes > max_output_bytes {
                return Err(PlenoraError::LimitExceeded(format!(
                    "output FileGDB da {bytes} byte oltre il limite di {max_output_bytes}"
                )));
            }
            let outcome = publish_dir_atomic(&staging, &dest, durable)?;
            Ok(Published {
                bytes,
                loss: LossReport::default(),
                fidelity: plenora_io_core::FidelityAssessment::lossless(),
                outcome,
            })
        }
    }

    const GEOMETRY: &str = "geometry";

    fn err(reason: impl Into<String>) -> PlenoraError {
        PlenoraError::Format {
            driver: "filegdb",
            reason: reason.into(),
        }
    }

    pub fn open(path: &std::path::Path) -> Result<Box<dyn OpenDatasetHandle>> {
        // Schema dai def GDAL, SENZA leggere feature (poi il reader streamma).
        let ds = Dataset::open(path).map_err(|e| err(format!("apertura GDAL: {e}")))?;
        let mut layers = Vec::new();
        let mut metas = Vec::new();
        for (i, layer) in ds.layers().enumerate() {
            let crs = layer
                .spatial_ref()
                .and_then(|sr| match (sr.auth_name(), sr.auth_code()) {
                    (Ok(a), Ok(c)) => Some(format!("{a}:{c}")),
                    _ => None,
                });
            let fields: Vec<(String, DataType)> = layer
                .defn()
                .fields()
                .map(|f| (f.name(), ogr_to_arrow(f.field_type())))
                .collect();
            let mut arrow_fields = vec![geometry_field(
                GEOMETRY,
                crs.as_deref().unwrap_or("unknown"),
            )];
            for (n, dt) in &fields {
                arrow_fields.push(Field::new(n, dt.clone(), true));
            }
            let schema: SchemaRef = Arc::new(Schema::new(arrow_fields));
            let contract = DataContract {
                schema: schema.clone(),
                geometry: Some(GeometryColumnContract::wkb_passthrough(
                    FieldId(0),
                    GEOMETRY,
                    ResolvedCrs {
                        id: crs,
                        kind: CrsKind::Unknown,
                        definition: None,
                    },
                    true,
                )),
            };
            layers.push(LayerContract {
                id: LayerId(i as u32),
                name: layer.name(),
                contract,
            });
            metas.push(LayerMeta {
                gdal_idx: i,
                schema,
                fields,
            });
        }
        Ok(Box::new(GdbDataset {
            path: path.to_owned(),
            layers,
            metas,
            reader_gate: SingleReaderGate::new(DESCRIPTOR.id),
        }))
    }

    fn ogr_to_arrow(ft: gdal::vector::OGRFieldType::Type) -> DataType {
        use gdal::vector::OGRFieldType;
        if ft == OGRFieldType::OFTInteger || ft == OGRFieldType::OFTInteger64 {
            DataType::Int64
        } else if ft == OGRFieldType::OFTReal {
            DataType::Float64
        } else {
            DataType::Utf8
        }
    }

    struct LayerMeta {
        gdal_idx: usize,
        schema: SchemaRef,
        fields: Vec<(String, DataType)>,
    }

    struct GdbDataset {
        path: PathBuf,
        layers: Vec<LayerContract>,
        metas: Vec<LayerMeta>,
        reader_gate: SingleReaderGate,
    }

    impl OpenDatasetHandle for GdbDataset {
        fn layers(&self) -> &[LayerContract] {
            &self.layers
        }
        fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
            plenora_io_core::FidelityAssessment::for_format(
                DESCRIPTOR.id,
                DESCRIPTOR.fidelity_class,
            )
        }
        fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
            plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
            let idx = self
                .layers
                .iter()
                .position(|l| l.id.0 == request.layer.0)
                .ok_or_else(|| err(format!("layer {} inesistente", request.layer.0)))?;
            let m = &self.metas[idx];
            let batch_size =
                plenora_io_core::effective_batch_rows(m.schema.as_ref(), request.batch_target);
            self.reader_gate.open(request.layer, || {
                let rx = spawn_reader(
                    self.path.clone(),
                    m.gdal_idx,
                    m.schema.clone(),
                    m.fields.clone(),
                    batch_size,
                );
                Ok(Box::new(GdbReader {
                    rx,
                    layer: self.layers[idx].clone(),
                }))
            })
        }
    }

    struct GdbReader {
        rx: Receiver<std::result::Result<RecordBatch, String>>,
        layer: LayerContract,
    }

    impl LayerReader for GdbReader {
        fn contract(&self) -> &LayerContract {
            &self.layer
        }
        fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            match self.rx.recv() {
                Ok(Ok(b)) => Ok(Some(b)),
                Ok(Err(e)) => Err(err(e)),
                Err(_) => Ok(None),
            }
        }
    }

    /// Il thread apre il PROPRIO Dataset GDAL (non-Send, quindi mai attraversa il
    /// confine) e scorre le feature in batch, consegnati via canale.
    fn spawn_reader(
        path: PathBuf,
        gdal_idx: usize,
        schema: SchemaRef,
        fields: Vec<(String, DataType)>,
        batch_size: usize,
    ) -> Receiver<std::result::Result<RecordBatch, String>> {
        let (tx, rx) = sync_channel::<std::result::Result<RecordBatch, String>>(2);
        std::thread::spawn(move || {
            let run = || -> std::result::Result<(), String> {
                let ds = Dataset::open(&path).map_err(|e| e.to_string())?;
                let mut layer = ds.layer(gdal_idx).map_err(|e| e.to_string())?;
                let mut geom = BinaryBuilder::new();
                let mut builders: Vec<ReadCol> =
                    fields.iter().map(|(_, dt)| ReadCol::new(dt)).collect();
                let mut n = 0usize;
                for feature in layer.features() {
                    match feature.geometry().and_then(|g| g.wkb().ok()) {
                        Some(bytes) => geom.append_value(&bytes),
                        None => geom.append_null(),
                    }
                    for (k, (name, _)) in fields.iter().enumerate() {
                        builders[k].append(feature.field(name).ok().flatten());
                    }
                    n += 1;
                    if n >= batch_size {
                        let batch = finish_read_batch(&schema, &mut geom, &mut builders)?;
                        if tx.send(Ok(batch)).is_err() {
                            return Ok(());
                        }
                        n = 0;
                    }
                }
                if n > 0 {
                    let batch = finish_read_batch(&schema, &mut geom, &mut builders)?;
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

    fn finish_read_batch(
        schema: &SchemaRef,
        geom: &mut BinaryBuilder,
        builders: &mut [ReadCol],
    ) -> std::result::Result<RecordBatch, String> {
        let mut arrays: Vec<ArrayRef> = Vec::with_capacity(1 + builders.len());
        arrays.push(Arc::new(geom.finish()));
        for b in builders.iter_mut() {
            arrays.push(b.finish());
        }
        RecordBatch::try_new(schema.clone(), arrays).map_err(|e| format!("batch: {e}"))
    }

    enum ReadCol {
        I64(Int64Builder),
        F64(Float64Builder),
        Str(StringBuilder),
    }

    impl ReadCol {
        fn new(dt: &DataType) -> Self {
            match dt {
                DataType::Int64 => ReadCol::I64(Int64Builder::new()),
                DataType::Float64 => ReadCol::F64(Float64Builder::new()),
                _ => ReadCol::Str(StringBuilder::new()),
            }
        }
        fn append(&mut self, v: Option<gdal::vector::FieldValue>) {
            use gdal::vector::FieldValue as F;
            match self {
                ReadCol::I64(b) => b.append_option(match v {
                    Some(F::IntegerValue(i)) => Some(i as i64),
                    Some(F::Integer64Value(i)) => Some(i),
                    Some(F::RealValue(f)) => Some(f as i64),
                    _ => None,
                }),
                ReadCol::F64(b) => b.append_option(match v {
                    Some(F::RealValue(f)) => Some(f),
                    Some(F::Integer64Value(i)) => Some(i as f64),
                    Some(F::IntegerValue(i)) => Some(i as f64),
                    _ => None,
                }),
                ReadCol::Str(b) => match v {
                    Some(F::StringValue(s)) => b.append_value(s),
                    Some(F::IntegerValue(i)) => b.append_value(i.to_string()),
                    Some(F::Integer64Value(i)) => b.append_value(i.to_string()),
                    Some(F::RealValue(f)) => b.append_value(f.to_string()),
                    _ => b.append_null(),
                },
            }
        }
        fn finish(&mut self) -> ArrayRef {
            match self {
                ReadCol::I64(b) => Arc::new(b.finish()),
                ReadCol::F64(b) => Arc::new(b.finish()),
                ReadCol::Str(b) => Arc::new(b.finish()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_without_gdal_feature_is_typed() {
        // Nel build di default (senza feature) l'apertura fallisce tipizzata.
        #[cfg(not(feature = "gdal-backend"))]
        {
            let e = FileGdbDriver
                .open(Source::Path("x.gdb".into()), &ReadOptions::default())
                .map(|_| ())
                .unwrap_err();
            assert!(matches!(e, plenora_core::PlenoraError::Unsupported(_)));
        }
    }
}

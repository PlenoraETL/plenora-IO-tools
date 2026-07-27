//! driver-filegdb — FileGDB ⇄ RecordBatch (Fase 1, "tier GDB"). È
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
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::None,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::None,
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
    driver_version: 4,
    descriptor_version: 4,
};

pub struct FileGdbDriver;

impl FormatDriver for FileGdbDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    // I `return` cfg-gated servono per il caso feature-on (il blocco feature-off,
    // pur rimosso, segue sintatticamente); clippy non lo coglie.
    #[allow(clippy::needless_return)]
    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        #[cfg(feature = "gdal-backend")]
        {
            let path = source.into_path_checked(&opts.limits)?;
            return backend::open(&path, opts.assume_crs.as_deref());
        }
        #[cfg(not(feature = "gdal-backend"))]
        {
            let _ = (source, opts);
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
    use super::DESCRIPTOR;

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
    use plenora_core::crs::{AxisOrder, CrsKind, RawCrs, ResolvedCrs};
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
        srs: SpatialRef,
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

    fn layer_spatial_ref(layer: &WriteLayer) -> Result<SpatialRef> {
        let resolved = layer
            .contract
            .geometry
            .as_ref()
            .and_then(GeometryColumnContract::resolved_crs)
            .ok_or_else(|| {
                PlenoraError::Crs(format!(
                    "FileGDB richiede un CRS risolto per il layer '{}'",
                    layer.name
                ))
            })?;
        let definition = resolved
            .definition
            .as_deref()
            .filter(|definition| !definition.trim().is_empty())
            .or(resolved.id.as_deref())
            .ok_or_else(|| PlenoraError::CrsUnresolved {
                driver: "filegdb",
                raw: RawCrs {
                    definition: "ResolvedCrs senza identificatore o definizione".to_owned(),
                    authority_hint: None,
                },
            })?;
        let spatial_ref =
            SpatialRef::from_definition(definition).map_err(|_| PlenoraError::CrsUnresolved {
                driver: "filegdb",
                raw: RawCrs {
                    definition: definition.to_owned(),
                    authority_hint: resolved.id.clone(),
                },
            })?;
        if let (Some(expected), Some(actual)) = (resolved.id.as_deref(), authority_id(&spatial_ref))
        {
            if !expected.eq_ignore_ascii_case(&actual) {
                return Err(PlenoraError::CrsUnresolved {
                    driver: "filegdb",
                    raw: RawCrs {
                        definition: definition.to_owned(),
                        authority_hint: resolved.id.clone(),
                    },
                });
            }
        }
        Ok(spatial_ref)
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

        // Risolvi ogni CRS prima di creare lo staging: un CRS non rappresentabile
        // fallisce senza lasciare output parziali.
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
                srs: layer_spatial_ref(l)?,
                gdal_idx: None,
            });
        }

        let staging = staging_path(path);
        let driver = DriverManager::get_driver_by_name("OpenFileGDB")
            .map_err(|e| err(format!("driver OpenFileGDB non disponibile: {e}")))?;
        let ds = driver
            .create_vector_only(&staging)
            .map_err(|e| err(format!("creazione FileGDB: {e}")))?;

        // I layer GDAL sono creati PIGRAMENTE al primo write: FileGDB richiede il
        // tipo geometria concreto, che conosciamo solo vedendo i dati.
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
                let defs_owned: Vec<(String, gdal::vector::OGRFieldType::Type)> = self.layers[li]
                    .fields
                    .iter()
                    .map(|(n, _, k)| (n.clone(), k.ogr()))
                    .collect();
                let defs: Vec<(&str, gdal::vector::OGRFieldType::Type)> =
                    defs_owned.iter().map(|(n, t)| (n.as_str(), *t)).collect();
                let gl = self
                    .ds
                    .create_layer(LayerOptions {
                        name: &lname,
                        srs: Some(&self.layers[li].srs),
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

    fn authority_id(spatial_ref: &SpatialRef) -> Option<String> {
        match (spatial_ref.auth_name(), spatial_ref.auth_code()) {
            (Ok(authority), Ok(code)) => Some(format!("{}:{code}", authority.to_ascii_uppercase())),
            _ => None,
        }
    }

    fn crs_kind(spatial_ref: &SpatialRef) -> CrsKind {
        if spatial_ref.is_geographic() {
            CrsKind::Geographic
        } else if spatial_ref.is_projected() {
            CrsKind::Projected
        } else {
            CrsKind::Unknown
        }
    }

    fn has_any(value: &str, needles: &[&str]) -> bool {
        let value = value.to_ascii_lowercase();
        needles.iter().any(|needle| value.contains(needle))
    }

    fn declared_axis_order(spatial_ref: &SpatialRef, kind: CrsKind) -> AxisOrder {
        let target = match kind {
            CrsKind::Geographic => "GEOGCS",
            CrsKind::Projected => "PROJCS",
            CrsKind::Unknown => return AxisOrder::Unknown,
        };
        let Ok(first) = spatial_ref.axis_name(target, 0) else {
            return AxisOrder::Unknown;
        };
        let Ok(second) = spatial_ref.axis_name(target, 1) else {
            return AxisOrder::Unknown;
        };
        match kind {
            CrsKind::Geographic
                if has_any(&first, &["longitude", "lon"])
                    && has_any(&second, &["latitude", "lat"]) =>
            {
                AxisOrder::LongitudeLatitude
            }
            CrsKind::Geographic
                if has_any(&first, &["latitude", "lat"])
                    && has_any(&second, &["longitude", "lon"]) =>
            {
                AxisOrder::LatitudeLongitude
            }
            CrsKind::Projected
                if has_any(&first, &["easting", "east"])
                    && has_any(&second, &["northing", "north"]) =>
            {
                AxisOrder::EastingNorthing
            }
            CrsKind::Geographic | CrsKind::Projected | CrsKind::Unknown => AxisOrder::Unknown,
        }
    }

    fn resolve_layer_crs(
        embedded: Option<SpatialRef>,
        assume_crs: Option<&str>,
    ) -> Result<ResolvedCrs> {
        let spatial_ref = match embedded {
            Some(spatial_ref) => spatial_ref,
            None => {
                let definition = assume_crs.ok_or_else(|| {
                    PlenoraError::Crs(
                        "FileGDB con geometria senza CRS: fornire --assume-crs".to_owned(),
                    )
                })?;
                SpatialRef::from_definition(definition).map_err(|_| {
                    PlenoraError::CrsUnresolved {
                        driver: "filegdb",
                        raw: RawCrs {
                            definition: definition.to_owned(),
                            authority_hint: Some(definition.to_owned()),
                        },
                    }
                })?
            }
        };
        let definition = spatial_ref
            .to_wkt()
            .map_err(|_| PlenoraError::CrsUnresolved {
                driver: "filegdb",
                raw: RawCrs {
                    definition: "SpatialRef GDAL presente ma WKT non esportabile".to_owned(),
                    authority_hint: authority_id(&spatial_ref),
                },
            })?;
        let id = authority_id(&spatial_ref);
        let kind = crs_kind(&spatial_ref);
        let mut resolved = ResolvedCrs::new(id, kind, Some(definition));
        let declared_axis_order = declared_axis_order(&spatial_ref, kind);
        if declared_axis_order != AxisOrder::Unknown {
            resolved.axis_order = declared_axis_order;
        }
        Ok(resolved)
    }

    pub fn open(
        path: &std::path::Path,
        assume_crs: Option<&str>,
    ) -> Result<Box<dyn OpenDatasetHandle>> {
        // Schema dai def GDAL, SENZA leggere feature (poi il reader streamma).
        let ds = Dataset::open(path).map_err(|e| err(format!("apertura GDAL: {e}")))?;
        let mut layers = Vec::new();
        let mut metas = Vec::new();
        for (i, layer) in ds.layers().enumerate() {
            let crs = resolve_layer_crs(layer.spatial_ref(), assume_crs)?;
            let crs_label = crs
                .id
                .as_deref()
                .or(crs.definition.as_deref())
                .expect("un CRS risolto da GDAL ha sempre id o WKT");
            let fields: Vec<(String, DataType)> = layer
                .defn()
                .fields()
                .map(|f| (f.name(), ogr_to_arrow(f.field_type())))
                .collect();
            let mut arrow_fields = vec![geometry_field(GEOMETRY, crs_label)];
            for (n, dt) in &fields {
                arrow_fields.push(Field::new(n, dt.clone(), true));
            }
            let schema: SchemaRef = Arc::new(Schema::new(arrow_fields));
            let contract = DataContract {
                schema: schema.clone(),
                geometry: Some(GeometryColumnContract::wkb_passthrough(
                    FieldId(0),
                    GEOMETRY,
                    crs,
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

    #[cfg(test)]
    mod tests {
        use super::*;

        use arrow_array::RecordBatch;
        use plenora_core::contract::GeometryType;
        use plenora_core::wkb::{encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
        use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source};
        use plenora_io_core::request::{BatchTarget, ProjectionMode};

        fn read_request() -> ReadRequest {
            ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
            }
        }

        fn write_layer(crs: ResolvedCrs) -> WriteLayer {
            let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(
                GEOMETRY,
                crs.id.as_deref().unwrap_or("custom"),
            )]));
            WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(GeometryColumnContract::wkb_passthrough(
                        FieldId(0),
                        GEOMETRY,
                        crs,
                        true,
                    )),
                },
            }
        }

        #[test]
        fn gdal_reports_authority_axis_order_without_canonicalization() {
            let epsg = resolve_layer_crs(
                Some(SpatialRef::from_definition("EPSG:4326").unwrap()),
                None,
            )
            .unwrap();
            let crs84 = resolve_layer_crs(
                Some(SpatialRef::from_definition("OGC:CRS84").unwrap()),
                None,
            )
            .unwrap();
            let projected = resolve_layer_crs(
                Some(SpatialRef::from_definition("EPSG:3857").unwrap()),
                None,
            )
            .unwrap();

            assert_eq!(epsg.axis_order, AxisOrder::LatitudeLongitude);
            assert_eq!(crs84.axis_order, AxisOrder::LongitudeLatitude);
            assert_ne!(crs84.id.as_deref(), Some("EPSG:4326"));
            assert_eq!(projected.axis_order, AxisOrder::EastingNorthing);

            let write_crs84 = resolve_layer_crs(
                Some(layer_spatial_ref(&write_layer(ResolvedCrs::wgs84())).unwrap()),
                None,
            )
            .unwrap();
            assert_eq!(write_crs84.axis_order, AxisOrder::LongitudeLatitude);
            assert_ne!(write_crs84.id.as_deref(), Some("EPSG:4326"));
        }

        #[test]
        fn conflicting_id_and_wkt_fail_before_output_creation() {
            let epsg_4326_wkt = SpatialRef::from_definition("EPSG:4326")
                .unwrap()
                .to_wkt()
                .unwrap();
            let layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                Some(epsg_4326_wkt),
            ));
            assert!(matches!(
                layer_spatial_ref(&layer),
                Err(PlenoraError::CrsUnresolved { .. })
            ));
        }

        #[test]
        fn missing_crs_requires_a_valid_explicit_assumption() {
            assert!(matches!(
                resolve_layer_crs(None, None),
                Err(PlenoraError::Crs(_))
            ));
            assert!(matches!(
                resolve_layer_crs(None, Some("not-a-crs-secret")),
                Err(PlenoraError::CrsUnresolved { .. })
            ));
            let assumed = resolve_layer_crs(None, Some("EPSG:3857")).unwrap();
            assert_eq!(assumed.id.as_deref(), Some("EPSG:3857"));
            assert_eq!(assumed.axis_order, AxisOrder::EastingNorthing);
        }

        #[test]
        fn filegdb_round_trip_preserves_crs_and_enforces_single_reader() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("round-trip.gdb");
            let wkb = encode_wkb(
                &WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 1_113_194.0,
                        y: 5_621_521.0,
                        z: None,
                        m: None,
                    }),
                    dimensions: plenora_core::contract::CoordinateDimensions::Xy,
                    srid: None,
                },
                WkbFlavor::Iso,
            )
            .unwrap();
            let schema: SchemaRef =
                Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_passthrough(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.geometry_types = vec![GeometryType::Point];
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "points".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: Some(geometry),
                    },
                }],
            };

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();

            let dataset = driver
                .open(Source::Path(path), &ReadOptions::default())
                .unwrap();
            let crs = dataset.layers()[0]
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .resolved_crs()
                .unwrap();
            assert_eq!(crs.id.as_deref(), Some("EPSG:3857"));
            assert_eq!(crs.axis_order, AxisOrder::EastingNorthing);
            assert!(crs.definition.is_some());

            let first = dataset.open_layer_reader(&read_request()).unwrap();
            assert!(matches!(
                dataset.open_layer_reader(&read_request()),
                Err(PlenoraError::ReaderBusy {
                    driver: "filegdb",
                    layer: 0
                })
            ));
            drop(first);
            assert!(dataset.open_layer_reader(&read_request()).is_ok());
        }
    }
}

#[cfg(all(test, not(feature = "gdal-backend")))]
mod tests {
    use super::*;

    #[test]
    fn open_without_gdal_feature_is_typed() {
        // Nel build di default (senza feature) l'apertura fallisce tipizzata.
        let e = FileGdbDriver
            .open(Source::Path("x.gdb".into()), &ReadOptions::default())
            .map(|_| ())
            .unwrap_err();
        assert!(matches!(e, plenora_core::PlenoraError::Unsupported(_)));
    }
}

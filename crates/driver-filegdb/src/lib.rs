//! driver-filegdb — FileGDB ⇄ RecordBatch (Fase 1, "tier GDB"). È
//! l'unica eccezione alla policy puro-Rust: FileGDB richiede GDAL. Dietro la
//! feature `gdal-backend` legge via GDAL; senza feature è uno stub che fallisce
//! tipizzato (il binario di default resta puro-Rust). Multi-layer.
#![forbid(unsafe_code)]

use plenora_core::contract::{CoordinateDimensions, GeometryEncoding, SpatialSemantics};
use plenora_core::Result;
use plenora_io_core::descriptor::{
    ArrowTypeClass, CrsHandling, Direction, Fidelity, FormatDescriptor, GeometryWriteSupport,
    ReadMode, ReaderConcurrency, Runtime, WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, OpenDatasetHandle, ReadOptions, Sink, Source, WriteOptions,
};
use plenora_io_core::{
    validate_write, AttributeWriteSupport, CrsWriteSupport, FormatWriteCapabilities,
    NullabilitySupport, TypeCoercionPolicy, WritePlan, UTF8_FIELD_NAMES,
};

const FILEGDB_ATTRIBUTE_TYPES: &[ArrowTypeClass] = &[
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
];

const FILEGDB_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: &[CoordinateDimensions::Xy, CoordinateDimensions::Xyz],
    spatial_semantics: &[SpatialSemantics::Geometry],
    mixed_types: false,
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
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::Gdal,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: FILEGDB_ATTRIBUTE_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: FILEGDB_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: true,
    }),
    semantic_version: 1,
    driver_version: 5,
    descriptor_version: 5,
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

    use arrow_array::builder::{
        BinaryBuilder, Float64Builder, Int32Builder, Int64Builder, StringBuilder,
    };
    use arrow_array::{ArrayRef, BinaryArray, Float64Array, Int32Array, RecordBatch, StringArray};
    use arrow_schema::{Field, Schema, SchemaRef};
    use gdal::vector::LayerAccess;
    use gdal::Dataset;

    use driver_common::geometry_field;
    use plenora_core::contract::{
        CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
        LayerContract, LayerId,
    };
    use plenora_core::crs::{AxisOrder, CrsKind, RawCrs, ResolvedCrs};
    use plenora_core::geometry::with_geometry_contract_metadata;
    use plenora_core::{CapabilityReason, PlenoraError, Result};
    use plenora_io_core::driver::{LayerReader, OpenDatasetHandle};
    use plenora_io_core::request::ReadRequest;

    // --- scrittura (tier GDB via GDAL OpenFileGDB) --------------------------
    use std::path::{Path, PathBuf};

    use arrow_array::Array;
    use arrow_schema::DataType;
    use gdal::spatial_ref::SpatialRef;
    use gdal::vector::{Feature, Geometry, LayerOptions, OGRwkbGeometryType};
    use gdal::DriverManager;

    use plenora_core::geometry::is_geometry_field;
    use plenora_io_core::driver::{FormatWriter, Published, WriteOptions};
    use plenora_io_core::loss::LossReport;
    use plenora_io_core::publish::publish_dir_atomic;
    use plenora_io_core::{SingleReaderGate, WriteLayer, WritePlan};

    #[derive(Clone, Copy)]
    enum FieldKind {
        Int32,
        Float64,
        Utf8,
    }

    impl FieldKind {
        fn from(field: &Field) -> Result<Self> {
            let kind = match field.data_type() {
                DataType::Int32 => FieldKind::Int32,
                DataType::Float64 => FieldKind::Float64,
                DataType::Utf8 => FieldKind::Utf8,
                other => {
                    return Err(PlenoraError::Capability {
                        driver: "filegdb",
                        field: Some(field.name().clone()),
                        reason: CapabilityReason::TypeNotRepresentable,
                        detail: format!(
                            "tipo Arrow {other:?} non round-trip nativo; supportati esattamente Int32, Float64 e Utf8"
                        ),
                    });
                }
            };
            Ok(kind)
        }

        fn ogr(self) -> gdal::vector::OGRFieldType::Type {
            match self {
                FieldKind::Int32 => gdal::vector::OGRFieldType::OFTInteger,
                FieldKind::Float64 => gdal::vector::OGRFieldType::OFTReal,
                FieldKind::Utf8 => gdal::vector::OGRFieldType::OFTString,
            }
        }
    }

    struct PlanLayer {
        name: String,
        geom_idx: usize,
        fields: Vec<(String, usize, FieldKind)>,
        srs: SpatialRef,
        ogr_type: OGRwkbGeometryType::Type,
        gdal_idx: usize,
    }

    struct GdbWriter {
        ds: Dataset,
        staging: PathBuf,
        dest: PathBuf,
        durable: bool,
        max_output_bytes: u64,
        layers: Vec<PlanLayer>,
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

    fn geometry_capability(
        field: &str,
        reason: CapabilityReason,
        detail: impl Into<String>,
    ) -> PlenoraError {
        PlenoraError::Capability {
            driver: "filegdb",
            field: Some(field.to_owned()),
            reason,
            detail: detail.into(),
        }
    }

    /// FileGDB richiede tipo e dimensionalità del feature class prima dei dati:
    /// non li deduciamo dal primo record, che renderebbe vuoti/null dipendenti
    /// dall'ordine dei batch.
    fn contract_ogr_type(layer: &WriteLayer) -> Result<OGRwkbGeometryType::Type> {
        let geometry = layer.contract.geometry.as_ref().ok_or_else(|| {
            geometry_capability(
                "geometry",
                CapabilityReason::GeometryNotSupported,
                format!("layer '{}' senza contratto geometrico", layer.name),
            )
        })?;
        let geometry_type = match geometry.geometry_types.as_slice() {
            [geometry_type] => *geometry_type,
            [] => {
                return Err(geometry_capability(
                    &geometry.name,
                    CapabilityReason::GeometryNotSupported,
                    "FileGDB richiede un tipo geometrico dichiarato",
                ));
            }
            _ => {
                return Err(geometry_capability(
                    &geometry.name,
                    CapabilityReason::MixedGeometry,
                    "FileGDB richiede un solo tipo geometrico per layer",
                ));
            }
        };
        if geometry_type == GeometryType::GeometryCollection {
            return Err(geometry_capability(
                &geometry.name,
                CapabilityReason::GeometryNotSupported,
                "GeometryCollection non è un feature-class FileGDB nativo",
            ));
        }

        use CoordinateDimensions as D;
        use GeometryType as G;
        use OGRwkbGeometryType as O;
        match (geometry_type, geometry.dimensions) {
            (G::Point, D::Xy) => Ok(O::wkbPoint),
            (G::MultiPoint, D::Xy) => Ok(O::wkbMultiPoint),
            (G::MultiLineString, D::Xy) => Ok(O::wkbMultiLineString),
            (G::MultiPolygon, D::Xy) => Ok(O::wkbMultiPolygon),
            (G::Point, D::Xyz) => Ok(O::wkbPoint25D),
            (G::MultiPoint, D::Xyz) => Ok(O::wkbMultiPoint25D),
            (G::MultiLineString, D::Xyz) => Ok(O::wkbMultiLineString25D),
            (G::MultiPolygon, D::Xyz) => Ok(O::wkbMultiPolygon25D),
            (G::LineString | G::Polygon, D::Xy | D::Xyz) => Err(geometry_capability(
                &geometry.name,
                CapabilityReason::GeometryNotSupported,
                "FileGDB normalizza le famiglie lineari/poligonali native a MultiLineString/MultiPolygon; dichiarare il tipo multipart per un round-trip stabile",
            )),
            (_, D::Xym | D::Xyzm) => Err(geometry_capability(
                &geometry.name,
                CapabilityReason::CoordinateDimensions,
                "il backend GDAL 0.17 non espone una creazione FileGDB M/ZM verificata; il writer rifiuta invece di perdere M",
            )),
            (_, D::Unknown) => Err(geometry_capability(
                &geometry.name,
                CapabilityReason::CoordinateDimensions,
                "FileGDB richiede dimensionalità XY o XYZ dichiarata",
            )),
            (G::GeometryCollection, _) => unreachable!("rifiutata sopra"),
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

    fn field_value(
        kind: FieldKind,
        array: &ArrayRef,
        row: usize,
    ) -> Result<Option<gdal::vector::FieldValue>> {
        use gdal::vector::FieldValue as F;
        if array.is_null(row) {
            return Ok(None);
        }
        let value = match kind {
            FieldKind::Int32 => F::IntegerValue(
                array
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .ok_or_else(|| err("schema Int32 ma array runtime differente"))?
                    .value(row),
            ),
            FieldKind::Float64 => F::RealValue(
                array
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| err("schema Float64 ma array runtime differente"))?
                    .value(row),
            ),
            FieldKind::Utf8 => F::StringValue(
                array
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| err("schema Utf8 ma array runtime differente"))?
                    .value(row)
                    .to_owned(),
            ),
        };
        Ok(Some(value))
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
                .map(|(i, f)| FieldKind::from(f).map(|kind| (f.name().clone(), i, kind)))
                .collect::<Result<Vec<_>>>()?;
            infos.push(PlanLayer {
                name: l.name.clone(),
                geom_idx,
                fields,
                srs: layer_spatial_ref(l)?,
                ogr_type: contract_ogr_type(l)?,
                gdal_idx: infos.len(),
            });
        }

        let staging = staging_path(path);
        let driver = DriverManager::get_driver_by_name("OpenFileGDB")
            .map_err(|e| err(format!("driver OpenFileGDB non disponibile: {e}")))?;
        let mut ds = driver
            .create_vector_only(&staging)
            .map_err(|e| err(format!("creazione FileGDB: {e}")))?;

        // Il contratto basta a creare anche layer vuoti o con sole geometrie
        // nulle, senza dipendere dal primo record osservato. Verifichiamo
        // subito anche che GDAL non abbia rinominato o riclassificato campi.
        let layer_result = (|| -> Result<()> {
            for info in &infos {
                let definitions: Vec<(&str, gdal::vector::OGRFieldType::Type)> = info
                    .fields
                    .iter()
                    .map(|(name, _, kind)| (name.as_str(), kind.ogr()))
                    .collect();
                let layer = ds
                    .create_layer(LayerOptions {
                        name: &info.name,
                        srs: Some(&info.srs),
                        ty: info.ogr_type,
                        options: None,
                    })
                    .map_err(|e| err(format!("create_layer '{}': {e}", info.name)))?;
                layer
                    .create_defn_fields(&definitions)
                    .map_err(|e| err(format!("definizione campi '{}': {e}", info.name)))?;
                let actual: Vec<(String, gdal::vector::OGRFieldType::Type)> = layer
                    .defn()
                    .fields()
                    .map(|field| (field.name(), field.field_type()))
                    .collect();
                if actual.len() != definitions.len() {
                    return Err(geometry_capability(
                        &info.name,
                        CapabilityReason::TypeNotRepresentable,
                        "GDAL ha creato un numero di campi diverso dal contratto",
                    ));
                }
                for ((expected_name, _, expected_kind), (actual_name, actual_type)) in
                    info.fields.iter().zip(actual)
                {
                    if expected_name != &actual_name {
                        return Err(PlenoraError::Capability {
                            driver: "filegdb",
                            field: Some(expected_name.clone()),
                            reason: CapabilityReason::FieldNameCollision,
                            detail: format!(
                                "GDAL ha normalizzato il nome in '{actual_name}'; scrittura rifiutata"
                            ),
                        });
                    }
                    if expected_kind.ogr() != actual_type {
                        return Err(PlenoraError::Capability {
                            driver: "filegdb",
                            field: Some(expected_name.clone()),
                            reason: CapabilityReason::TypeNotRepresentable,
                            detail: format!(
                                "GDAL ha riclassificato il tipo OGR {} in {actual_type}",
                                expected_kind.ogr()
                            ),
                        });
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = layer_result {
            drop(ds);
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }

        Ok(Box::new(GdbWriter {
            ds,
            staging,
            dest: path.to_owned(),
            durable: opts.durable,
            max_output_bytes: opts.limits.max_output_bytes,
            layers: infos,
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

            let gidx = self.layers[li].gdal_idx;
            let fields = self.layers[li].fields.clone();
            let gl = self
                .ds
                .layer(gidx)
                .map_err(|e| err(format!("accesso layer {gidx}: {e}")))?;
            for row in 0..batch.num_rows() {
                let mut feature =
                    Feature::new(gl.defn()).map_err(|e| err(format!("creazione feature: {e}")))?;
                if !geom_col.is_null(row) {
                    let geometry = Geometry::from_wkb(geom_col.value(row))
                        .map_err(|e| err(format!("WKB->GDAL: {e}")))?;
                    feature
                        .set_geometry(geometry)
                        .map_err(|e| err(format!("geometria feature: {e}")))?;
                }
                for (name, index, kind) in &fields {
                    match field_value(*kind, batch.column(*index), row)? {
                        Some(value) => feature
                            .set_field(name, &value)
                            .map_err(|e| err(format!("campo '{name}': {e}")))?,
                        None => feature
                            .set_field_null(name)
                            .map_err(|e| err(format!("null campo '{name}': {e}")))?,
                    }
                }
                feature
                    .create(&gl)
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

    fn geometry_contract_from_ogr(
        ogr_type: OGRwkbGeometryType::Type,
        crs: ResolvedCrs,
    ) -> GeometryColumnContract {
        let raw = ogr_type;
        let without_25d = raw & !0x8000_0000;
        let dimension_code = without_25d / 1000;
        let dimensions = match (
            raw & 0x8000_0000 != 0 || matches!(dimension_code, 1 | 3),
            matches!(dimension_code, 2 | 3),
        ) {
            (false, false) => CoordinateDimensions::Xy,
            (true, false) => CoordinateDimensions::Xyz,
            (false, true) => CoordinateDimensions::Xym,
            (true, true) => CoordinateDimensions::Xyzm,
        };
        let geometry_types = match without_25d % 1000 {
            1 => vec![GeometryType::Point],
            2 => vec![GeometryType::LineString],
            3 => vec![GeometryType::Polygon],
            4 => vec![GeometryType::MultiPoint],
            5 => vec![GeometryType::MultiLineString],
            6 => vec![GeometryType::MultiPolygon],
            7 => vec![GeometryType::GeometryCollection],
            _ => Vec::new(),
        };
        let mut geometry = GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs, true);
        geometry.dimensions = if geometry_types.is_empty() {
            CoordinateDimensions::Unknown
        } else {
            dimensions
        };
        geometry.geometry_types = geometry_types;
        geometry
            .native_metadata
            .insert("filegdb.ogr_geometry_type".to_owned(), raw.to_string());
        geometry
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
                .expect("un CRS risolto da GDAL ha sempre id o WKT")
                .to_owned();
            let ogr_geometry_type = layer
                .defn()
                .geom_fields()
                .next()
                .map(|field| field.field_type())
                .unwrap_or(OGRwkbGeometryType::wkbUnknown);
            let geometry = geometry_contract_from_ogr(ogr_geometry_type, crs);
            let fields: Vec<(String, DataType)> = layer
                .defn()
                .fields()
                .map(|field| {
                    let name = field.name();
                    ogr_to_arrow(field.field_type(), &name).map(|data_type| (name, data_type))
                })
                .collect::<Result<Vec<_>>>()?;
            let geometry_arrow_field =
                with_geometry_contract_metadata(&geometry_field(GEOMETRY, &crs_label), &geometry);
            let mut arrow_fields = vec![geometry_arrow_field];
            for (n, dt) in &fields {
                arrow_fields.push(Field::new(n, dt.clone(), true));
            }
            let schema: SchemaRef = Arc::new(Schema::new(arrow_fields));
            let contract = DataContract {
                schema: schema.clone(),
                geometry: Some(geometry),
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

    fn ogr_to_arrow(ft: gdal::vector::OGRFieldType::Type, name: &str) -> Result<DataType> {
        use gdal::vector::OGRFieldType;
        if ft == OGRFieldType::OFTInteger {
            Ok(DataType::Int32)
        } else if ft == OGRFieldType::OFTInteger64 {
            Ok(DataType::Int64)
        } else if ft == OGRFieldType::OFTReal {
            Ok(DataType::Float64)
        } else if ft == OGRFieldType::OFTString || ft == OGRFieldType::OFTWideString {
            Ok(DataType::Utf8)
        } else {
            Err(PlenoraError::Capability {
                driver: "filegdb",
                field: Some(name.to_owned()),
                reason: CapabilityReason::TypeNotRepresentable,
                detail: format!(
                    "tipo campo OGR {ft} non ancora rappresentato senza perdita nel bordo Arrow"
                ),
            })
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
                    match feature
                        .geometry_by_index(0)
                        .ok()
                        .and_then(|geometry| geometry.wkb().ok())
                    {
                        Some(bytes) => geom.append_value(&bytes),
                        None => geom.append_null(),
                    }
                    for (k, (name, _)) in fields.iter().enumerate() {
                        let value = feature.field(name).map_err(|e| e.to_string())?;
                        builders[k].append(value)?;
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
        I32(Int32Builder),
        I64(Int64Builder),
        F64(Float64Builder),
        Str(StringBuilder),
    }

    impl ReadCol {
        fn new(dt: &DataType) -> Self {
            match dt {
                DataType::Int32 => ReadCol::I32(Int32Builder::new()),
                DataType::Int64 => ReadCol::I64(Int64Builder::new()),
                DataType::Float64 => ReadCol::F64(Float64Builder::new()),
                _ => ReadCol::Str(StringBuilder::new()),
            }
        }
        fn append(
            &mut self,
            v: Option<gdal::vector::FieldValue>,
        ) -> std::result::Result<(), String> {
            use gdal::vector::FieldValue as F;
            match self {
                ReadCol::I32(b) => match v {
                    Some(F::IntegerValue(i)) => b.append_value(i),
                    None => b.append_null(),
                    Some(other) => {
                        return Err(format!(
                            "campo OGR intero 32-bit ha restituito valore incompatibile {other:?}"
                        ));
                    }
                },
                ReadCol::I64(b) => match v {
                    Some(F::Integer64Value(i)) => b.append_value(i),
                    None => b.append_null(),
                    Some(other) => {
                        return Err(format!(
                            "campo OGR intero 64-bit ha restituito valore incompatibile {other:?}"
                        ));
                    }
                },
                ReadCol::F64(b) => match v {
                    Some(F::RealValue(f)) => b.append_value(f),
                    None => b.append_null(),
                    Some(other) => {
                        return Err(format!(
                            "campo OGR reale ha restituito valore incompatibile {other:?}"
                        ));
                    }
                },
                ReadCol::Str(b) => match v {
                    Some(F::StringValue(s)) => b.append_value(s),
                    None => b.append_null(),
                    Some(other) => {
                        return Err(format!(
                            "campo OGR stringa ha restituito valore incompatibile {other:?}"
                        ));
                    }
                },
            }
            Ok(())
        }
        fn finish(&mut self) -> ArrayRef {
            match self {
                ReadCol::I32(b) => Arc::new(b.finish()),
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
        use plenora_core::limits::WkbLimits;
        use plenora_core::wkb::{
            decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
        };
        use plenora_io_core::descriptor::Fidelity;
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
            let mut geometry = GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs, true);
            geometry.geometry_types = vec![GeometryType::Point];
            WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }
        }

        fn point_wkb(dimensions: CoordinateDimensions, z: Option<f64>) -> Vec<u8> {
            encode_wkb(
                &WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 10.5,
                        y: 20.25,
                        z,
                        m: None,
                    }),
                    dimensions,
                    srid: None,
                },
                WkbFlavor::Iso,
            )
            .unwrap()
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
            let mut geometry = GeometryColumnContract::wkb_xy(
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

        #[test]
        fn filegdb_round_trip_preserves_null_geometry_and_exact_attributes() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("null-and-attributes.gdb");
            let geometry_field = geometry_field(GEOMETRY, "EPSG:3857");
            let schema: SchemaRef = Arc::new(Schema::new(vec![
                geometry_field,
                Field::new("count", DataType::Int32, true),
                Field::new("ratio", DataType::Float64, true),
                Field::new("label", DataType::Utf8, true),
            ]));
            let wkb = point_wkb(CoordinateDimensions::Xy, None);
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(BinaryArray::from(vec![Some(wkb.as_slice()), None])),
                    Arc::new(Int32Array::from(vec![Some(i32::MAX), None])),
                    Arc::new(Float64Array::from(vec![Some(12.5), None])),
                    Arc::new(StringArray::from(vec![Some("città"), None])),
                ],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_xy(
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
            let published = writer.finish().unwrap();
            assert_eq!(published.fidelity.level, Fidelity::Conditional);
            assert!(published.loss.is_empty());

            let dataset = driver
                .open(Source::Path(path), &ReadOptions::default())
                .unwrap();
            let output_geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
            assert_eq!(output_geometry.dimensions, CoordinateDimensions::Xy);
            assert_eq!(output_geometry.geometry_types, vec![GeometryType::Point]);
            assert!(output_geometry
                .native_metadata
                .contains_key("filegdb.ogr_geometry_type"));

            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            let output = reader.next_batch().unwrap().unwrap();
            assert_eq!(output.num_rows(), 2);
            let output_geometry = output
                .column(0)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            assert!(!output_geometry.is_null(0));
            assert!(output_geometry.is_null(1));
            let counts = output
                .column(1)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap();
            assert_eq!(counts.value(0), i32::MAX);
            assert!(counts.is_null(1));
            let ratios = output
                .column(2)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap();
            assert_eq!(ratios.value(0), 12.5);
            assert!(ratios.is_null(1));
            let labels = output
                .column(3)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            assert_eq!(labels.value(0), "città");
            assert!(labels.is_null(1));
            assert!(reader.next_batch().unwrap().is_none());
        }

        #[test]
        fn filegdb_xyz_round_trip_preserves_z_and_contract_metadata() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("xyz.gdb");
            let schema: SchemaRef =
                Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
            let wkb = point_wkb(CoordinateDimensions::Xyz, Some(123.25));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())]))],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.dimensions = CoordinateDimensions::Xyz;
            geometry.geometry_types = vec![GeometryType::Point];
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "points_z".to_owned(),
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
            let output_contract = dataset.layers()[0].contract.geometry.as_ref().unwrap();
            assert_eq!(output_contract.dimensions, CoordinateDimensions::Xyz);
            assert_eq!(output_contract.geometry_types, vec![GeometryType::Point]);
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            let output = reader.next_batch().unwrap().unwrap();
            let geometry = output
                .column(0)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
            assert_eq!(decoded.dimensions, CoordinateDimensions::Xyz);
            assert_eq!(
                decoded.value,
                WkbValue::Point(WkbCoordinate {
                    x: 10.5,
                    y: 20.25,
                    z: Some(123.25),
                    m: None,
                })
            );
        }

        #[test]
        fn filegdb_empty_layer_is_created_from_the_contract() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("empty.gdb");
            let plan = WritePlan {
                layers: vec![write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ))],
            };
            let driver = super::super::FileGdbDriver;
            driver
                .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
                .unwrap()
                .finish()
                .unwrap();

            let dataset = driver
                .open(Source::Path(path), &ReadOptions::default())
                .unwrap();
            assert_eq!(dataset.layers().len(), 1);
            let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
            assert_eq!(geometry.dimensions, CoordinateDimensions::Xy);
            assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            assert!(reader.next_batch().unwrap().is_none());
        }

        #[test]
        fn filegdb_empty_layers_preserve_every_declared_geometry_family() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("geometry-families.gdb");
            let geometry_types = [
                GeometryType::Point,
                GeometryType::MultiPoint,
                GeometryType::MultiLineString,
                GeometryType::MultiPolygon,
            ];
            let layers = geometry_types
                .iter()
                .enumerate()
                .map(|(index, geometry_type)| {
                    let name = format!("family_{index}");
                    let schema: SchemaRef =
                        Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
                    let mut geometry = GeometryColumnContract::wkb_xy(
                        FieldId(0),
                        GEOMETRY,
                        ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                        true,
                    );
                    geometry.geometry_types = vec![*geometry_type];
                    WriteLayer {
                        name,
                        contract: DataContract {
                            schema,
                            geometry: Some(geometry),
                        },
                    }
                })
                .collect();
            let plan = WritePlan { layers };
            let driver = super::super::FileGdbDriver;
            driver
                .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
                .unwrap()
                .finish()
                .unwrap();

            let dataset = driver
                .open(Source::Path(path), &ReadOptions::default())
                .unwrap();
            assert_eq!(dataset.layers().len(), geometry_types.len());
            for (layer, expected) in dataset.layers().iter().zip(geometry_types) {
                let geometry = layer.contract.geometry.as_ref().unwrap();
                assert_eq!(geometry.dimensions, CoordinateDimensions::Xy);
                assert_eq!(geometry.geometry_types, vec![expected]);
            }
        }

        #[test]
        fn filegdb_rejects_geometry_families_normalized_by_the_format() {
            for geometry_type in [GeometryType::LineString, GeometryType::Polygon] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("normalized-family.gdb");
                let mut layer = write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ));
                layer.contract.geometry.as_mut().unwrap().geometry_types = vec![geometry_type];
                let plan = WritePlan {
                    layers: vec![layer],
                };

                let result = super::super::FileGdbDriver.create(
                    Sink::Path(path.clone()),
                    &plan,
                    &WriteOptions::default(),
                );
                assert!(matches!(
                    result,
                    Err(PlenoraError::Capability {
                        reason: CapabilityReason::GeometryNotSupported,
                        ..
                    })
                ));
                assert!(!path.exists());
            }
        }

        #[test]
        fn filegdb_rejects_non_round_trip_attribute_type_before_output() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("int64.gdb");
            let mut layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            layer.contract.schema = Arc::new(Schema::new(vec![
                geometry_field(GEOMETRY, "EPSG:3857"),
                Field::new("too_wide", DataType::Int64, true),
            ]));
            let plan = WritePlan {
                layers: vec![layer],
            };

            let result = super::super::FileGdbDriver.create(
                Sink::Path(path.clone()),
                &plan,
                &WriteOptions::default(),
            );
            assert!(matches!(
                result,
                Err(PlenoraError::Capability {
                    reason: CapabilityReason::TypeNotRepresentable,
                    ..
                })
            ));
            assert!(!path.exists());
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

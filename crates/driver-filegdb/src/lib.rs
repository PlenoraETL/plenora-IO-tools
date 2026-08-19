//! driver-filegdb — `FileGDB` ⇄ `RecordBatch` (Fase 1, "tier GDB"). È
//! l'unica eccezione alla policy puro-Rust: `FileGDB` richiede GDAL. Dietro la
//! feature `gdal-backend` legge via GDAL; senza feature è uno stub che fallisce
//! tipizzato (il binario di default resta puro-Rust). Multi-layer.
#![forbid(unsafe_code)]

use plenora_io_core::descriptor::{
    ArrowTypeClass, CrsHandling, Direction, Fidelity, FormatDescriptor, GeometryWriteSupport,
    ReadMode, ReaderConcurrency, Runtime, WriteMode,
};
use plenora_io_core::driver::{
    FormatDriver, FormatWriter, OpenDatasetHandle, ReadOptions, Sink, Source, WriteOptions,
};
use plenora_io_core::{
    validate_write, AttributeWriteSupport, CrsRepresentationCapabilities, CrsRepresentationState,
    CrsWriteSupport, FormatWriteCapabilities, NullabilitySupport, TypeCoercionPolicy, WritePlan,
    UTF8_FIELD_NAMES,
};
use plenora_io_model::contract::{
    CoordinateDimensions, GeometryEncoding, GeometryType, SpatialSemantics,
};
use plenora_io_model::Result;

const FILEGDB_ATTRIBUTE_TYPES: &[ArrowTypeClass] = &[
    ArrowTypeClass::SignedInteger,
    ArrowTypeClass::Floating,
    ArrowTypeClass::Utf8,
];

const FILEGDB_GEOMETRY: GeometryWriteSupport = GeometryWriteSupport {
    supported: true,
    encodings: &[GeometryEncoding::Wkb],
    dimensions: &[
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Xym,
        CoordinateDimensions::Xyzm,
    ],
    spatial_semantics: &[SpatialSemantics::Geometry],
    geometry_types: &[
        GeometryType::Point,
        GeometryType::MultiPoint,
        GeometryType::MultiLineString,
        GeometryType::MultiPolygon,
    ],
    mixed_types: false,
};

static DESCRIPTOR: FormatDescriptor = FormatDescriptor::const_new(
    "filegdb",
    Direction::Bidirectional,
    ReadMode::Materializing,
    // INV-7: `for feature in layer.features()`, iteratore GDAL in avanti.
    plenora_io_core::NativeReadMode::StreamingSequential,
    // Il drenaggio e lo spool sono dell'adapter comune, non di
    // questo driver: `BudgetedReader` li impone a tutti.
    plenora_io_core::DeliverySemantics::OperationAtomic,
    plenora_io_core::BufferingStrategy::AdaptiveMemoryThenDisk,
    plenora_io_core::DeterminismLevel::Semantic,
    Some(WriteMode::Streaming),
    Some(plenora_io_core::DeterminismLevel::Semantic),
    true,
    true, // una .gdb è una directory
    ReaderConcurrency::SingleActiveReader,
    plenora_io_core::ProjectionSupport::Exact,
    plenora_io_core::PredicatePruningSupport::None,
    plenora_io_core::SpatialPruningSupport::None,
    CrsHandling::Embedded,
    Fidelity::Conditional,
    Runtime::Gdal,
    Some(FormatWriteCapabilities {
        field_names: UTF8_FIELD_NAMES,
        allowed_types: FILEGDB_ATTRIBUTE_TYPES,
        type_coercion: TypeCoercionPolicy::Reject,
        attributes: AttributeWriteSupport::All,
        geometry: FILEGDB_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Derived,
            CrsRepresentationState::Absent,
            CrsRepresentationState::Derived,
        ),
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: true,
    }),
    // Il driver non interpreta alcuna format_option (L0.7): l'elenco vuoto
    // e' l'affermazione che qualunque chiave e' sconosciuta, non un'omissione.
    plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
    1,
    10,
    11,
);

pub struct FileGdbDriver;

/// Verifica a runtime che GDAL esponga `OpenFileGDB` come driver vettoriale
/// bidirezionale.
///
/// In assenza della feature, o se una capability non è dichiarata dal runtime
/// caricato, il risultato è deliberatamente fail-closed.
// Senza `gdal-backend` il corpo si riduce a `false` e clippy la vorrebbe
// `const fn`; con la feature attiva chiama GDAL e const non è possibile. La
// firma pubblica resta unica in entrambe le configurazioni.
#[allow(clippy::missing_const_for_fn)]
#[must_use]
pub fn runtime_available() -> bool {
    #[cfg(feature = "gdal-backend")]
    {
        backend::runtime_available()
    }
    #[cfg(not(feature = "gdal-backend"))]
    {
        false
    }
}

impl FormatDriver for FileGdbDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    // I `return` cfg-gated servono per il caso feature-on (il blocco feature-off,
    // pur rimosso, segue sintatticamente); clippy non lo coglie.
    #[allow(clippy::needless_return)]
    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        #[cfg(feature = "gdal-backend")]
        {
            let path = plenora_io_core::preflight_source(self.descriptor(), source, &mut opts)?;
            let dataset = backend::open(&path, opts.assume_crs.as_deref())?;
            return Ok(plenora_io_core::with_read_budget(dataset, &opts, false));
        }
        #[cfg(not(feature = "gdal-backend"))]
        {
            let _ = (source, opts);
            Err(plenora_io_model::PlenoraIoError::Unsupported(
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
        validate_write(
            self.descriptor(),
            plan,
            opts.max_columns(),
            &opts.format_options,
        )?;
        #[cfg(feature = "gdal-backend")]
        {
            let Sink::Path(path) = sink;
            return backend::create(&path, plan, opts).and_then(|writer| {
                plenora_io_core::with_write_validation(writer, self.descriptor(), plan, opts)
            });
        }
        #[cfg(not(feature = "gdal-backend"))]
        {
            let _ = (sink, plan, opts);
            Err(plenora_io_model::PlenoraIoError::Unsupported(
                "scrittura FileGDB richiede il tier GDB: compilare con --features gdal-backend"
                    .to_owned(),
            ))
        }
    }
}

#[cfg(feature = "gdal-backend")]
mod backend {
    use super::DESCRIPTOR;

    use std::collections::{HashMap, HashSet};
    use std::fs::{File, OpenOptions, TryLockError};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use arrow_array::builder::{
        BinaryBuilder, Float64Builder, Int32Builder, Int64Builder, StringBuilder,
    };
    use arrow_array::{
        ArrayRef, BinaryArray, Float64Array, Int32Array, RecordBatch, RecordBatchOptions,
        StringArray,
    };
    use arrow_schema::{Field, Schema, SchemaRef};
    use gdal::vector::LayerAccess;
    use gdal::{Dataset, Metadata};

    use driver_common::geometry_field;
    use plenora_io_core::driver::{
        spawn_batch_reader, BatchEmitter, LayerReader, OpenDatasetHandle,
    };
    use plenora_io_core::request::ReadRequest;
    use plenora_io_model::contract::{
        CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
        LayerContract, LayerId,
    };
    use plenora_io_model::crs::{AxisOrder, CrsKind, RawCrs, ResolvedCrs};
    use plenora_io_model::geometry::with_geometry_contract_metadata;
    use plenora_io_model::{CapabilityReason, PlenoraIoError, Result};

    pub fn runtime_available() -> bool {
        let Ok(driver) = DriverManager::get_driver_by_name("OpenFileGDB") else {
            return false;
        };
        ["DCAP_VECTOR", "DCAP_OPEN", "DCAP_CREATE"]
            .into_iter()
            .all(|capability| driver.metadata_item(capability, "") == Some("YES".to_owned()))
    }

    // --- scrittura (tier GDB via GDAL OpenFileGDB) --------------------------
    use std::path::{Path, PathBuf};

    use arrow_array::Array;
    use arrow_schema::DataType;
    use driver_common::geometry_index;
    use gdal::errors::GdalError;
    use gdal::spatial_ref::SpatialRef;
    use gdal::vector::{Feature, FieldDefn, Geometry, LayerOptions, OGRwkbGeometryType};
    use gdal::DriverManager;

    use plenora_io_core::driver::{FormatWriter, Published, WriteOptions};
    use plenora_io_core::loss::LossReport;
    use plenora_io_core::publish::publish_dir_atomic;
    use plenora_io_core::{SingleReaderGate, WriteLayer, WritePlan};

    const OGR_FIELD_TYPE_KEY: &str = "plenora.filegdb.ogr_field_type";
    const OGR_FIELD_WIDTH_KEY: &str = "plenora.filegdb.width";
    const OGR_FIELD_PRECISION_KEY: &str = "plenora.filegdb.precision";
    const STAGING_MARKER: &str = ".plenora-tmp-";
    static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone, Copy)]
    enum FieldKind {
        Int32,
        Float64,
        Utf8,
    }

    impl FieldKind {
        fn from(field: &Field) -> Result<Self> {
            let kind = match field.data_type() {
                DataType::Int32 => Self::Int32,
                DataType::Float64 => Self::Float64,
                DataType::Utf8 => Self::Utf8,
                other => {
                    return Err(PlenoraIoError::capability(
                        "filegdb",
                        Some(field.name().clone()),
                        CapabilityReason::TypeNotRepresentable,
                        format!(
                            "tipo Arrow {other:?} non round-trip nativo; supportati esattamente Int32, Float64 e Utf8"
                        ),
                    ));
                }
            };
            Ok(kind)
        }

        const fn ogr(self) -> gdal::vector::OGRFieldType::Type {
            match self {
                Self::Int32 => gdal::vector::OGRFieldType::OFTInteger,
                Self::Float64 => gdal::vector::OGRFieldType::OFTReal,
                Self::Utf8 => gdal::vector::OGRFieldType::OFTString,
            }
        }
    }

    fn native_i32(field: &Field, key: &str) -> Result<Option<i32>> {
        let Some(value) = field.metadata().get(key) else {
            return Ok(None);
        };
        let parsed = value.parse::<i32>().map_err(|_| {
            PlenoraIoError::capability(
                "filegdb",
                Some(field.name().clone()),
                CapabilityReason::TypeNotRepresentable,
                format!("metadato nativo '{key}' non è un intero valido"),
            )
        })?;
        if parsed < 0 {
            return Err(PlenoraIoError::capability(
                "filegdb",
                Some(field.name().clone()),
                CapabilityReason::TypeNotRepresentable,
                format!("metadato nativo '{key}' negativo"),
            ));
        }
        Ok(Some(parsed))
    }

    #[derive(Clone)]
    struct PlanField {
        name: String,
        index: usize,
        kind: FieldKind,
        width: Option<i32>,
        precision: Option<i32>,
    }

    struct PlanLayer {
        name: String,
        geom_idx: usize,
        fields: Vec<PlanField>,
        srs: SpatialRef,
        ogr_type: OGRwkbGeometryType::Type,
        gdal_idx: usize,
    }

    struct StagingGuard {
        path: PathBuf,
        lock_path: PathBuf,
        lock: Option<File>,
        armed: bool,
    }

    impl StagingGuard {
        fn create(dest: &Path) -> Result<Self> {
            recover_orphaned_staging(dest)?;
            let parent = dataset_parent(dest);
            let prefix = staging_prefix(dest);
            loop {
                let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let token = format!("{}-{sequence}", std::process::id());
                let base = format!("{prefix}{token}");
                let path = parent.join(format!("{base}.gdb"));
                let lock_path = parent.join(format!("{base}.lock"));
                let lock = match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&lock_path)
                {
                    Ok(lock) => lock,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error.into()),
                };
                lock.lock()?;
                return Ok(Self {
                    path,
                    lock_path,
                    lock: Some(lock),
                    armed: true,
                });
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn disarm(&mut self) {
            drop(self.lock.take());
            let _ = std::fs::remove_file(&self.lock_path);
            self.armed = false;
        }

        fn cleanup(&mut self) {
            if self.armed {
                let _ = std::fs::remove_dir_all(&self.path);
                drop(self.lock.take());
                let _ = std::fs::remove_file(&self.lock_path);
                self.armed = false;
            }
        }
    }

    impl Drop for StagingGuard {
        fn drop(&mut self) {
            self.cleanup();
        }
    }

    struct GdbWriter {
        ds: Option<Dataset>,
        staging: StagingGuard,
        dest: PathBuf,
        durable: bool,
        max_output_bytes: u64,
        layers: Vec<PlanLayer>,
    }

    impl Drop for GdbWriter {
        fn drop(&mut self) {
            drop(self.ds.take());
            self.staging.cleanup();
        }
    }

    fn layer_spatial_ref(layer: &WriteLayer) -> Result<SpatialRef> {
        let resolved = layer
            .contract
            .geometry
            .as_ref()
            .and_then(GeometryColumnContract::resolved_crs)
            .ok_or_else(|| {
                PlenoraIoError::Crs(format!(
                    "FileGDB richiede un CRS risolto per il layer '{}'",
                    layer.name
                ))
            })?;
        let definition = resolved
            .definition
            .as_deref()
            .filter(|definition| !definition.trim().is_empty())
            .or(resolved.id.as_deref())
            .ok_or_else(|| {
                PlenoraIoError::crs_unresolved(
                    "filegdb",
                    &RawCrs::new(
                        "ResolvedCrs senza identificatore o definizione".to_owned(),
                        None,
                    ),
                )
            })?;
        let spatial_ref = SpatialRef::from_definition(definition).map_err(|_| {
            PlenoraIoError::crs_unresolved(
                "filegdb",
                &RawCrs::new(definition.to_owned(), resolved.id.clone()),
            )
        })?;
        if let (Some(expected), Some(actual)) = (resolved.id.as_deref(), authority_id(&spatial_ref))
        {
            if !expected.eq_ignore_ascii_case(&actual) {
                return Err(PlenoraIoError::crs_unresolved(
                    "filegdb",
                    &RawCrs::new(definition.to_owned(), resolved.id.clone()),
                ));
            }
        }
        Ok(spatial_ref)
    }

    fn geometry_capability(
        field: &str,
        reason: CapabilityReason,
        detail: impl Into<String>,
    ) -> PlenoraIoError {
        PlenoraIoError::capability("filegdb", Some(field.to_owned()), reason, detail)
    }

    /// `FileGDB` richiede tipo e dimensionalità del feature class prima dei dati:
    /// non li deduciamo dal primo record, che renderebbe vuoti/null dipendenti
    /// dall'ordine dei batch.
    fn contract_ogr_type(layer: &WriteLayer) -> Result<OGRwkbGeometryType::Type> {
        use CoordinateDimensions as D;
        use GeometryType as G;
        use OGRwkbGeometryType as O;

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

        match (geometry_type, geometry.dimensions) {
            (G::Point, D::Xy) => Ok(O::wkbPoint),
            (G::MultiPoint, D::Xy) => Ok(O::wkbMultiPoint),
            (G::MultiLineString, D::Xy) => Ok(O::wkbMultiLineString),
            (G::MultiPolygon, D::Xy) => Ok(O::wkbMultiPolygon),
            (G::Point, D::Xyz) => Ok(O::wkbPoint25D),
            (G::MultiPoint, D::Xyz) => Ok(O::wkbMultiPoint25D),
            (G::MultiLineString, D::Xyz) => Ok(O::wkbMultiLineString25D),
            (G::MultiPolygon, D::Xyz) => Ok(O::wkbMultiPolygon25D),
            (G::Point, D::Xym) => Ok(O::wkbPointM),
            (G::MultiPoint, D::Xym) => Ok(O::wkbMultiPointM),
            (G::MultiLineString, D::Xym) => Ok(O::wkbMultiLineStringM),
            (G::MultiPolygon, D::Xym) => Ok(O::wkbMultiPolygonM),
            (G::Point, D::Xyzm) => Ok(O::wkbPointZM),
            (G::MultiPoint, D::Xyzm) => Ok(O::wkbMultiPointZM),
            (G::MultiLineString, D::Xyzm) => Ok(O::wkbMultiLineStringZM),
            (G::MultiPolygon, D::Xyzm) => Ok(O::wkbMultiPolygonZM),
            (G::LineString | G::Polygon, D::Xy | D::Xyz | D::Xym | D::Xyzm) => {
                Err(geometry_capability(
                    &geometry.name,
                    CapabilityReason::GeometryNotSupported,
                    "FileGDB normalizza le famiglie lineari/poligonali native a MultiLineString/MultiPolygon; dichiarare il tipo multipart per un round-trip stabile",
                ))
            }
            (_, D::Unknown) => Err(geometry_capability(
                &geometry.name,
                CapabilityReason::CoordinateDimensions,
                "FileGDB richiede dimensionalità XY o XYZ dichiarata",
            )),
            (G::GeometryCollection, _) => Err(geometry_capability(
                &geometry.name,
                CapabilityReason::GeometryNotSupported,
                "GeometryCollection non rappresentabile nel profilo FileGDB corrente",
            )),
            (unsupported, _) => Err(geometry_capability(
                &geometry.name,
                CapabilityReason::GeometryNotSupported,
                format!(
                    "tipo geometrico {} non rappresentabile nel profilo FileGDB corrente",
                    unsupported.canonical_name()
                ),
            )),
        }
    }

    fn dataset_parent(dest: &Path) -> PathBuf {
        dest.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_owned()
    }

    fn staging_prefix(dest: &Path) -> String {
        let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
        format!("{stem}{STAGING_MARKER}")
    }

    fn recover_orphaned_staging(dest: &Path) -> Result<usize> {
        let parent = dataset_parent(dest);
        let prefix = staging_prefix(dest);
        let mut recovered = 0;
        for entry in std::fs::read_dir(&parent)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(base) = name
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(".lock"))
            else {
                continue;
            };
            if base.is_empty() || base.contains(std::path::MAIN_SEPARATOR) {
                continue;
            }
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .open(entry.path())?;
            match lock.try_lock() {
                Ok(()) => {
                    let staging = parent.join(format!("{prefix}{base}.gdb"));
                    if staging.exists() {
                        std::fs::remove_dir_all(staging)?;
                    }
                    drop(lock);
                    std::fs::remove_file(entry.path())?;
                    recovered += 1;
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        Ok(recovered)
    }

    #[cfg(test)]
    fn staging_artifacts(dest: &Path) -> Vec<PathBuf> {
        let parent = dataset_parent(dest);
        let prefix = staging_prefix(dest);
        let mut artifacts = std::fs::read_dir(parent)
            .into_iter()
            .flatten()
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_str()?;
                // Gli artefatti di staging sono creati da questo driver con
                // estensione minuscola: il confronto case-sensitive e' voluto e
                // non va allargato a varianti maiuscole altrui.
                #[allow(clippy::case_sensitive_file_extension_comparisons)]
                let is_artifact = name.starts_with(&prefix)
                    && (name.ends_with(".gdb") || name.ends_with(".lock"));
                is_artifact.then(|| entry.path())
            })
            .collect::<Vec<_>>();
        artifacts.sort();
        artifacts
    }

    #[cfg(test)]
    fn crash_failpoint(point: &str) {
        if std::env::var("PLENORA_FILEGDB_CRASH_POINT").ok().as_deref() == Some(point) {
            std::process::abort();
        }
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

    // La sequenza staging → creazione layer → validazione CRS è un'unica
    // transazione: spezzarla in helper renderebbe meno leggibile l'ordine dei
    // fallimenti fail-closed senza cambiarne il comportamento.
    #[allow(clippy::too_many_lines)]
    pub fn create(
        path: &Path,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        if path.exists() {
            return Err(PlenoraIoError::OutputExists(path.display().to_string()));
        }

        // Risolvi ogni CRS prima di creare lo staging: un CRS non rappresentabile
        // fallisce senza lasciare output parziali.
        let mut infos = Vec::new();
        for l in &plan.layers {
            let schema = &l.contract.schema;
            let geom_idx = geometry_index(schema)
                .ok_or_else(|| err(format!("layer '{}' senza colonna geometria", l.name)))?;
            let fields = schema
                .fields()
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != geom_idx)
                .map(|(index, field)| {
                    let kind = FieldKind::from(field)?;
                    // OGRFieldType::Type è un u32 con costanti nell'intervallo
                    // 0..=13: il cast a i32 non può cambiare segno.
                    #[allow(clippy::cast_possible_wrap)]
                    let ogr_type = kind.ogr() as i32;
                    if native_i32(field, OGR_FIELD_TYPE_KEY)?
                        .is_some_and(|native_type| native_type != ogr_type)
                    {
                        return Err(PlenoraIoError::capability(
                            "filegdb",
                            Some(field.name().clone()),
                            CapabilityReason::TypeNotRepresentable,
                            "tipo Arrow e metadato OGR nativo incoerenti",
                        ));
                    }
                    Ok(PlanField {
                        name: field.name().clone(),
                        index,
                        kind,
                        width: native_i32(field, OGR_FIELD_WIDTH_KEY)?,
                        precision: native_i32(field, OGR_FIELD_PRECISION_KEY)?,
                    })
                })
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

        let staging = StagingGuard::create(path)?;
        let driver = DriverManager::get_driver_by_name("OpenFileGDB")
            .map_err(|e| err(format!("driver OpenFileGDB non disponibile: {e}")))?;
        let mut ds = driver
            .create_vector_only(staging.path())
            .map_err(|e| err(format!("creazione FileGDB: {e}")))?;

        // Il contratto basta a creare anche layer vuoti o con sole geometrie
        // nulle, senza dipendere dal primo record osservato. Verifichiamo
        // subito anche che GDAL non abbia rinominato o riclassificato campi.
        let layer_result = (|| -> Result<()> {
            for info in &infos {
                let layer = ds
                    .create_layer(LayerOptions {
                        name: &info.name,
                        srs: Some(&info.srs),
                        ty: info.ogr_type,
                        options: None,
                    })
                    .map_err(|e| err(format!("create_layer '{}': {e}", info.name)))?;
                for field in &info.fields {
                    let definition = FieldDefn::new(&field.name, field.kind.ogr())
                        .map_err(|e| err(format!("definizione campo '{}': {e}", field.name)))?;
                    if let Some(width) = field.width {
                        definition.set_width(width);
                    }
                    if let Some(precision) = field.precision {
                        definition.set_precision(precision);
                    }
                    definition
                        .add_to_layer(&layer)
                        .map_err(|e| err(format!("creazione campo '{}': {e}", field.name)))?;
                }
                let actual: Vec<(String, gdal::vector::OGRFieldType::Type, i32, i32)> = layer
                    .defn()
                    .fields()
                    .map(|field| {
                        (
                            field.name(),
                            field.field_type(),
                            field.width(),
                            field.precision(),
                        )
                    })
                    .collect();
                if actual.len() != info.fields.len() {
                    return Err(geometry_capability(
                        &info.name,
                        CapabilityReason::TypeNotRepresentable,
                        "GDAL ha creato un numero di campi diverso dal contratto",
                    ));
                }
                for (expected, (actual_name, actual_type, actual_width, actual_precision)) in
                    info.fields.iter().zip(actual)
                {
                    if expected.name != actual_name {
                        return Err(PlenoraIoError::capability(
                            "filegdb",
                            Some(expected.name.clone()),
                            CapabilityReason::FieldNameCollision,
                            format!(
                                "GDAL ha normalizzato il nome in '{actual_name}'; scrittura rifiutata"
                            ),
                        ));
                    }
                    if expected.kind.ogr() != actual_type {
                        return Err(PlenoraIoError::capability(
                            "filegdb",
                            Some(expected.name.clone()),
                            CapabilityReason::TypeNotRepresentable,
                            format!(
                                "GDAL ha riclassificato il tipo OGR {} in {actual_type}",
                                expected.kind.ogr()
                            ),
                        ));
                    }
                    if expected.width.is_some_and(|width| width != actual_width)
                        || expected
                            .precision
                            .is_some_and(|precision| precision != actual_precision)
                    {
                        return Err(PlenoraIoError::capability(
                            "filegdb",
                            Some(expected.name.clone()),
                            CapabilityReason::TypeNotRepresentable,
                            format!(
                                "GDAL ha normalizzato width/precision in {actual_width}/{actual_precision}"
                            ),
                        ));
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = layer_result {
            drop(ds);
            return Err(error);
        }

        Ok(Box::new(GdbWriter {
            ds: Some(ds),
            staging,
            dest: path.to_owned(),
            durable: opts.durable,
            max_output_bytes: opts.max_output_bytes(),
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
                .as_ref()
                .ok_or_else(|| err("dataset writer già chiuso"))?
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
                for field in &fields {
                    match field_value(field.kind, batch.column(field.index), row)? {
                        Some(value) => feature
                            .set_field(&field.name, &value)
                            .map_err(|e| err(format!("campo '{}': {e}", field.name)))?,
                        None => feature
                            .set_field_null(&field.name)
                            .map_err(|e| err(format!("null campo '{}': {e}", field.name)))?,
                    }
                }
                feature
                    .create(&gl)
                    .map_err(|e| err(format!("create_feature: {e}")))?;
            }
            #[cfg(test)]
            crash_failpoint("after_write");
            Ok(())
        }

        fn finish(mut self: Box<Self>) -> Result<Published> {
            let ds = self
                .ds
                .take()
                .ok_or_else(|| err("dataset writer già chiuso"))?;
            drop(ds); // chiude e flush della .gdb
            let bytes = dir_size(self.staging.path());
            if bytes > self.max_output_bytes {
                return Err(PlenoraIoError::LimitExceeded(format!(
                    "output FileGDB da {bytes} byte oltre il limite di {}",
                    self.max_output_bytes
                )));
            }
            #[cfg(test)]
            crash_failpoint("before_publish");
            let outcome = publish_dir_atomic(self.staging.path(), &self.dest, self.durable)?;
            #[cfg(test)]
            crash_failpoint("after_publish");
            self.staging.disarm();
            Ok(Published {
                bytes,
                loss: LossReport::default(),
                fidelity: plenora_io_core::FidelityAssessment::lossless(),
                outcome,
            })
        }
    }

    const GEOMETRY: &str = "geometry";

    fn err(reason: impl Into<String>) -> PlenoraIoError {
        PlenoraIoError::format("filegdb", reason)
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
        let spatial_ref = if let Some(spatial_ref) = embedded {
            spatial_ref
        } else {
            let definition = assume_crs.ok_or_else(|| {
                PlenoraIoError::Crs(
                    "FileGDB con geometria senza CRS: fornire --assume-crs".to_owned(),
                )
            })?;
            SpatialRef::from_definition(definition).map_err(|_| {
                PlenoraIoError::crs_unresolved(
                    "filegdb",
                    &RawCrs::new(definition.to_owned(), Some(definition.to_owned())),
                )
            })?
        };
        let definition = spatial_ref.to_wkt().map_err(|_| {
            PlenoraIoError::crs_unresolved(
                "filegdb",
                &RawCrs::new(
                    "SpatialRef GDAL presente ma WKT non esportabile".to_owned(),
                    authority_id(&spatial_ref),
                ),
            )
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
        geometry.set_exact_geometry_types(geometry_types);
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
                .ok_or_else(|| {
                    PlenoraIoError::Crs(
                        "CRS FileGDB risolto senza identificativo né definizione".to_owned(),
                    )
                })?
                .to_owned();
            let ogr_geometry_type = layer
                .defn()
                .geom_fields()
                .next()
                .map(|field| field.field_type())
                .ok_or_else(|| err(format!("layer '{}' senza campo geometrico", layer.name())))?;
            let geometry = geometry_contract_from_ogr(ogr_geometry_type, crs);
            let native_fields: Vec<(LayerFieldMeta, Field)> = layer
                .defn()
                .fields()
                .map(|field| {
                    let name = field.name();
                    let field_type = field.field_type();
                    ogr_to_arrow(field_type, &name).map(|data_type| {
                        let metadata = HashMap::from([
                            (OGR_FIELD_TYPE_KEY.to_owned(), field_type.to_string()),
                            (OGR_FIELD_WIDTH_KEY.to_owned(), field.width().to_string()),
                            (
                                OGR_FIELD_PRECISION_KEY.to_owned(),
                                field.precision().to_string(),
                            ),
                        ]);
                        let arrow_field =
                            Field::new(&name, data_type.clone(), true).with_metadata(metadata);
                        (
                            LayerFieldMeta {
                                name,
                                data_type,
                                ogr_type: field_type,
                            },
                            arrow_field,
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let (fields, attribute_arrow_fields): (Vec<_>, Vec<_>) =
                native_fields.into_iter().unzip();
            let geometry_arrow_field =
                with_geometry_contract_metadata(&geometry_field(GEOMETRY, &crs_label), &geometry);
            let mut arrow_fields = vec![geometry_arrow_field];
            arrow_fields.extend(attribute_arrow_fields);
            let schema: SchemaRef = Arc::new(Schema::new(arrow_fields));
            let contract = DataContract::new(schema.clone(), Some(geometry));
            // Indice di layer di un FileGDB: il formato ne ammette al piu'
            // qualche migliaio, il cast a u32 non puo' troncare.
            #[allow(clippy::cast_possible_truncation)]
            let layer_id = LayerId(i as u32);
            layers.push(LayerContract {
                id: layer_id,
                name: layer.name(),
                contract,
            });
            metas.push(LayerMeta {
                gdal_idx: i,
                fields,
            });
        }
        Ok(Box::new(GdbDataset {
            path: path.to_owned(),
            layers,
            metas,
            reader_gate: SingleReaderGate::new(DESCRIPTOR.id()),
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
            Err(PlenoraIoError::capability(
                "filegdb",
                Some(name.to_owned()),
                CapabilityReason::TypeNotRepresentable,
                format!(
                    "tipo campo OGR {ft} non ancora rappresentato senza perdita nel bordo Arrow"
                ),
            ))
        }
    }

    struct LayerFieldMeta {
        name: String,
        data_type: DataType,
        ogr_type: gdal::vector::OGRFieldType::Type,
    }

    struct ProjectedField {
        ogr_index: i32,
        name: String,
        data_type: DataType,
        ogr_type: gdal::vector::OGRFieldType::Type,
    }

    struct LayerMeta {
        gdal_idx: usize,
        fields: Vec<LayerFieldMeta>,
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
                DESCRIPTOR.id(),
                DESCRIPTOR.fidelity_class(),
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
            let (indices, layer_contract) =
                plenora_io_core::project_layer_contract(&self.layers[idx], request)?;
            let include_geometry = indices.binary_search(&0).is_ok();
            let mut fields = Vec::with_capacity(indices.len());
            for &index in &indices {
                let Some(field_index) = index.checked_sub(1) else {
                    continue;
                };
                let field = m.fields.get(field_index).ok_or_else(|| {
                    err(format!(
                        "indice Arrow {index} non presente nello schema FileGDB"
                    ))
                })?;
                let ogr_index = i32::try_from(field_index)
                    .map_err(|_| err(format!("indice OGR {field_index} fuori intervallo i32")))?;
                fields.push(ProjectedField {
                    ogr_index,
                    name: field.name.clone(),
                    data_type: field.data_type.clone(),
                    ogr_type: field.ogr_type,
                });
            }
            let batch_sizer = plenora_io_core::AdaptiveBatchSizer::new(
                layer_contract.contract.schema.as_ref(),
                request.batch_target,
            );
            let reader = self.reader_gate.open(request.layer, || {
                spawn_reader(
                    self.path.clone(),
                    m.gdal_idx,
                    layer_contract.contract.schema.clone(),
                    fields,
                    include_geometry,
                    batch_sizer,
                    layer_contract,
                )
            })?;
            Ok(plenora_io_core::with_cancellation(
                reader,
                request.cancellation.clone(),
            ))
        }
    }

    /// Il thread apre il PROPRIO Dataset GDAL (non-Send, quindi mai attraversa il
    /// confine) e scorre le feature in batch, consegnati via canale.
    fn spawn_reader(
        path: PathBuf,
        gdal_idx: usize,
        schema: SchemaRef,
        fields: Vec<ProjectedField>,
        include_geometry: bool,
        mut batch_sizer: plenora_io_core::AdaptiveBatchSizer,
        contract: LayerContract,
    ) -> Result<Box<dyn LayerReader>> {
        spawn_batch_reader(
            DESCRIPTOR.id(),
            contract,
            2,
            move |emitter: BatchEmitter| {
                let ds = Dataset::open(&path)
                    .map_err(|error| err(format!("apertura FileGDB: {error}")))?;
                let mut layer = ds
                    .layer(gdal_idx)
                    .map_err(|error| err(format!("apertura layer FileGDB: {error}")))?;
                // Il worker riapre il dataset: prima di usare gli indici OGR
                // pre-risolti verifica che nessun processo abbia cambiato lo
                // schema fra `open` e l'avvio della lettura. Un mismatch fallisce
                // chiuso invece di convertire silenziosamente il campo sbagliato.
                let actual_fields: Vec<_> = layer
                    .defn()
                    .fields()
                    .map(|field| (field.name(), field.field_type()))
                    .collect();
                for field in &fields {
                    let index = usize::try_from(field.ogr_index)
                        .map_err(|_| err(format!("indice OGR {} negativo", field.ogr_index)))?;
                    let actual = actual_fields.get(index);
                    if !matches!(
                        actual,
                        Some((name, ogr_type))
                            if name == &field.name && *ogr_type == field.ogr_type
                    ) {
                        return Err(err(format!(
                            "schema FileGDB cambiato fra apertura e lettura al campo '{}'",
                            field.name
                        )));
                    }
                }
                let selected_fields = fields
                    .iter()
                    .map(|field| usize::try_from(field.ogr_index))
                    .collect::<std::result::Result<HashSet<_>, _>>()
                    .map_err(|_| err("indice OGR negativo nella projection FileGDB"))?;
                let mut ignored_fields = actual_fields
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !selected_fields.contains(index))
                    .map(|(_, (name, _))| name.as_str())
                    .collect::<Vec<_>>();
                if !include_geometry {
                    ignored_fields.push("OGR_GEOMETRY");
                }
                layer.set_ignored_fields(&ignored_fields).map_err(|error| {
                    err(format!(
                        "projection fisica FileGDB non applicabile: {error}"
                    ))
                })?;
                let mut geom = include_geometry.then(BinaryBuilder::new);
                let mut builders: Vec<ReadCol> = fields
                    .iter()
                    .map(|field| ReadCol::new(&field.data_type))
                    .collect();
                let mut n = 0usize;
                for feature in layer.features() {
                    if let Some(builder) = &mut geom {
                        match feature.geometry_by_index(0) {
                            Ok(geometry) => {
                                let bytes = geometry.wkb().map_err(|error| {
                                    err(format!("conversione geometria FileGDB in WKB: {error}"))
                                })?;
                                builder.append_value(&bytes);
                            }
                            Err(GdalError::NullPointer {
                                method_name: "OGR_F_GetGeomFieldRef",
                                ..
                            }) => {
                                builder.append_null();
                            }
                            Err(error) => {
                                return Err(err(format!("lettura geometria FileGDB: {error}")))
                            }
                        }
                    }
                    for (builder, field) in builders.iter_mut().zip(&fields) {
                        builder.append_feature(&feature, field)?;
                    }
                    n += 1;
                    if n >= batch_sizer.rows() {
                        let batch = finish_read_batch(&schema, &mut geom, &mut builders, n)?;
                        batch_sizer.observe(&batch);
                        if !emitter.send(batch) {
                            return Ok(());
                        }
                        n = 0;
                    }
                }
                if n > 0 {
                    let batch = finish_read_batch(&schema, &mut geom, &mut builders, n)?;
                    if !emitter.send(batch) {
                        return Ok(());
                    }
                }
                Ok(())
            },
        )
    }

    fn finish_read_batch(
        schema: &SchemaRef,
        geom: &mut Option<BinaryBuilder>,
        builders: &mut [ReadCol],
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
        RecordBatch::try_new_with_options(schema.clone(), arrays, &options)
            .map_err(|error| err(format!("batch: {error}")))
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
                DataType::Int32 => Self::I32(Int32Builder::new()),
                DataType::Int64 => Self::I64(Int64Builder::new()),
                DataType::Float64 => Self::F64(Float64Builder::new()),
                _ => Self::Str(StringBuilder::new()),
            }
        }
        fn append_feature(&mut self, feature: &Feature<'_>, field: &ProjectedField) -> Result<()> {
            let read_error =
                |error| err(format!("lettura campo FileGDB '{}': {error}", field.name));
            match self {
                Self::I32(builder) => {
                    match feature
                        .field_as_integer(field.ogr_index)
                        .map_err(read_error)?
                    {
                        Some(value) => builder.append_value(value),
                        None => builder.append_null(),
                    }
                }
                Self::I64(builder) => {
                    match feature
                        .field_as_integer64(field.ogr_index)
                        .map_err(read_error)?
                    {
                        Some(value) => builder.append_value(value),
                        None => builder.append_null(),
                    }
                }
                Self::F64(builder) => {
                    match feature
                        .field_as_double(field.ogr_index)
                        .map_err(read_error)?
                    {
                        Some(value) => builder.append_value(value),
                        None => builder.append_null(),
                    }
                }
                Self::Str(builder) => {
                    match feature
                        .field_as_string(field.ogr_index)
                        .map_err(read_error)?
                    {
                        Some(value) => builder.append_value(value),
                        None => builder.append_null(),
                    }
                }
            }
            Ok(())
        }
        fn finish(&mut self) -> ArrayRef {
            match self {
                Self::I32(b) => Arc::new(b.finish()),
                Self::I64(b) => Arc::new(b.finish()),
                Self::F64(b) => Arc::new(b.finish()),
                Self::Str(b) => Arc::new(b.finish()),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use arrow_array::RecordBatch;
        use plenora_io_core::descriptor::Fidelity;
        use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
        use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
        use plenora_io_model::contract::{GeometryEncoding, GeometryType};
        use plenora_io_model::limits::WkbLimits;

        /// Opzioni di scrittura sul modello unificato (S4.e).
        fn opzioni_scrittura() -> WriteOptions {
            match plenora_io_model::budget::PipelineBudget::builder().build() {
                Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
                Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
            }
        }

        /// Opzioni di lettura sul modello unificato (S4.d).
        fn opzioni_lettura() -> ReadOptions {
            match plenora_io_model::budget::PipelineBudget::builder().build() {
                Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
                Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
            }
        }
        use plenora_io_model::wkb::{
            decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
        };
        use plenora_io_model::CancellationToken;
        use std::process::{Child, Command, ExitStatus};
        use std::time::{Duration, Instant};

        fn read_request() -> ReadRequest {
            ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            }
        }

        fn write_layer(crs: ResolvedCrs) -> WriteLayer {
            let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(
                GEOMETRY,
                crs.id.as_deref().unwrap_or("custom"),
            )]));
            let mut geometry = GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs, true);
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry),
                },
            }
        }

        fn point_wkb(dimensions: CoordinateDimensions, z: Option<f64>, m: Option<f64>) -> Vec<u8> {
            encode_wkb(
                &WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 10.5,
                        y: 20.25,
                        z,
                        m,
                    }),
                    dimensions,
                    srid: None,
                },
                WkbFlavor::Iso,
            )
            .unwrap()
        }

        fn point_write_fixture() -> (WritePlan, RecordBatch) {
            let layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            let geometry = point_wkb(CoordinateDimensions::Xy, None, None);
            let batch = RecordBatch::try_new(
                layer.contract.schema.clone(),
                vec![Arc::new(BinaryArray::from(vec![Some(geometry.as_slice())]))],
            )
            .unwrap();
            (
                WritePlan {
                    layers: vec![layer],
                },
                batch,
            )
        }

        fn assert_complete_point_dataset(path: PathBuf) {
            let dataset = super::super::FileGdbDriver
                .open(Source::Path(path), opzioni_lettura())
                .unwrap();
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 1);
            assert!(reader.next_batch().unwrap().is_none());
        }

        fn run_crash_subprocess(dest: &Path, point: &str) -> ExitStatus {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "backend::tests::filegdb_crash_subprocess_helper",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PLENORA_FILEGDB_CRASH_DEST", dest)
                .env("PLENORA_FILEGDB_CRASH_POINT", point)
                .env("RUST_BACKTRACE", "0")
                .status()
                .unwrap()
        }

        fn spawn_active_subprocess(dest: &Path, ready: &Path, release: &Path) -> Child {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "backend::tests::filegdb_active_subprocess_helper",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PLENORA_FILEGDB_ACTIVE_DEST", dest)
                .env("PLENORA_FILEGDB_ACTIVE_READY", ready)
                .env("PLENORA_FILEGDB_ACTIVE_RELEASE", release)
                .env("RUST_BACKTRACE", "0")
                .spawn()
                .unwrap()
        }

        fn wait_until_ready(child: &mut Child, ready: &Path) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if ready.exists() {
                    return;
                }
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("il writer attivo è terminato prematuramente: {status}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("timeout in attesa del writer attivo");
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
                Err(error) if error.code == plenora_io_model::IoErrorCode::CrsUnresolved
            ));
        }

        #[test]
        fn missing_crs_requires_a_valid_explicit_assumption() {
            assert!(matches!(
                resolve_layer_crs(None, None),
                Err(error) if error.code == plenora_io_model::IoErrorCode::Crs
            ));
            assert!(matches!(
                resolve_layer_crs(None, Some("not-a-crs-secret")),
                Err(error) if error.code == plenora_io_model::IoErrorCode::CrsUnresolved
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
                    dimensions: plenora_io_model::contract::CoordinateDimensions::Xy,
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
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
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
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
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
                Err(error)
                    if error.code == plenora_io_model::IoErrorCode::ReaderBusy
                        && error.driver.as_deref() == Some("filegdb")
            ));
            drop(first);
            assert!(dataset.open_layer_reader(&read_request()).is_ok());
        }

        #[test]
        fn repeated_filegdb_writes_are_semantically_deterministic() {
            let dir = tempfile::tempdir().unwrap();
            let paths = [
                dir.path().join("determinism-a.gdb"),
                dir.path().join("determinism-b.gdb"),
            ];
            let wkb = encode_wkb(
                &WkbGeometry {
                    value: WkbValue::Point(WkbCoordinate {
                        x: 1_113_194.0,
                        y: 5_621_521.0,
                        z: None,
                        m: None,
                    }),
                    dimensions: CoordinateDimensions::Xy,
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
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
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
            for path in &paths {
                let mut writer = driver
                    .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                    .unwrap();
                writer.write(&batch).unwrap();
                writer.finish().unwrap();
            }

            let semantic_signature = |path: &Path| {
                let dataset = driver
                    .open(Source::Path(path.to_owned()), opzioni_lettura())
                    .unwrap();
                let layer = &dataset.layers()[0];
                let geometry = layer.contract.geometry.as_ref().unwrap();
                let contract_signature = (
                    layer.name.clone(),
                    geometry.dimensions,
                    geometry.geometry_types.clone(),
                    geometry.crs.id().map(str::to_owned),
                    geometry.resolved_crs().map(|crs| crs.axis_order).unwrap(),
                );
                let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
                let batch = reader.next_batch().unwrap().unwrap();
                assert!(reader.next_batch().unwrap().is_none());
                let values = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .unwrap();
                let decoded = decode_wkb(values.value(0), &WkbLimits::default()).unwrap();
                (contract_signature, decoded)
            };

            assert_eq!(semantic_signature(&paths[0]), semantic_signature(&paths[1]));
        }

        // Il test copre in sequenza scrittura, riletture proiettate e proiezione
        // vuota sullo stesso dataset: spezzarlo perderebbe la condivisione della
        // fixture e la verifica dell'ordine delle proiezioni.
        #[allow(clippy::too_many_lines)]
        #[test]
        fn filegdb_round_trip_preserves_null_geometry_and_exact_attributes() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("null-and-attributes.gdb");
            let geometry_field = geometry_field(GEOMETRY, "EPSG:3857");
            let native_string_metadata = HashMap::from([
                (
                    OGR_FIELD_TYPE_KEY.to_owned(),
                    gdal::vector::OGRFieldType::OFTString.to_string(),
                ),
                (OGR_FIELD_WIDTH_KEY.to_owned(), "80".to_owned()),
                (OGR_FIELD_PRECISION_KEY.to_owned(), "0".to_owned()),
            ]);
            let schema: SchemaRef = Arc::new(Schema::new(vec![
                geometry_field,
                Field::new("count", DataType::Int32, true),
                Field::new("ratio", DataType::Float64, true),
                Field::new("label", DataType::Utf8, true)
                    .with_metadata(native_string_metadata.clone()),
            ]));
            let wkb = point_wkb(CoordinateDimensions::Xy, None, None);
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
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
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
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            let published = writer.finish().unwrap();
            assert_eq!(published.fidelity.level, Fidelity::Approximating);
            assert_eq!(
                published.loss.counts.get("crs_id_not_preserved_derived"),
                Some(&1)
            );

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            let output_geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
            assert_eq!(output_geometry.dimensions, CoordinateDimensions::Xy);
            assert_eq!(output_geometry.geometry_types, vec![GeometryType::Point]);
            assert!(output_geometry
                .native_metadata
                .contains_key("filegdb.ogr_geometry_type"));
            assert_eq!(
                dataset.layers()[0].contract.schema.field(3).metadata(),
                &native_string_metadata
            );

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
            // Il round-trip deve restituire il valore identico bit a bit: il
            // confronto esatto e' il contratto, non un'approssimazione.
            #[allow(clippy::float_cmp)]
            {
                assert_eq!(ratios.value(0), 12.5);
            }
            assert!(ratios.is_null(1));
            let labels = output
                .column(3)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            assert_eq!(labels.value(0), "città");
            assert!(labels.is_null(1));
            assert!(reader.next_batch().unwrap().is_none());
            drop(reader);

            let mut projected = dataset
                .open_layer_reader(&ReadRequest {
                    layer: LayerId(0),
                    projected_fields: Some(vec![FieldId(3)]),
                    projection_mode: ProjectionMode::Required,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: ReadScope::default(),
                    batch_target: BatchTarget::default(),
                    cancellation: CancellationToken::default(),
                })
                .unwrap();
            assert!(projected.contract().contract.geometry.is_none());
            let output = projected.next_batch().unwrap().unwrap();
            assert_eq!(output.num_rows(), 2);
            assert_eq!(output.num_columns(), 1);
            assert_eq!(output.schema().field(0).name(), "label");
            assert!(projected.next_batch().unwrap().is_none());
            drop(projected);

            let mut reversed = dataset
                .open_layer_reader(&ReadRequest {
                    layer: LayerId(0),
                    projected_fields: Some(vec![FieldId(3), FieldId(1)]),
                    projection_mode: ProjectionMode::Required,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: ReadScope::default(),
                    batch_target: BatchTarget::default(),
                    cancellation: CancellationToken::default(),
                })
                .unwrap();
            let output = reversed.next_batch().unwrap().unwrap();
            assert_eq!(output.num_rows(), 2);
            assert_eq!(output.num_columns(), 2);
            assert_eq!(output.schema().field(0).name(), "count");
            assert_eq!(output.schema().field(1).name(), "label");
            assert!(reversed.next_batch().unwrap().is_none());
            drop(reversed);

            let mut empty = dataset
                .open_layer_reader(&ReadRequest {
                    layer: LayerId(0),
                    projected_fields: Some(Vec::new()),
                    projection_mode: ProjectionMode::Required,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: ReadScope::default(),
                    batch_target: BatchTarget::default(),
                    cancellation: CancellationToken::default(),
                })
                .unwrap();
            assert!(empty.contract().contract.geometry.is_none());
            let output = empty.next_batch().unwrap().unwrap();
            assert_eq!(output.num_rows(), 2);
            assert_eq!(output.num_columns(), 0);
            assert!(empty.next_batch().unwrap().is_none());
        }

        #[test]
        fn filegdb_float64_edge_values_do_not_silently_change() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("float-edges.gdb");
            let schema: SchemaRef = Arc::new(Schema::new(vec![
                geometry_field(GEOMETRY, "EPSG:3857"),
                Field::new("value", DataType::Float64, false),
            ]));
            let wkb = point_wkb(CoordinateDimensions::Xy, None, None);
            let values = [
                f64::MIN,
                f64::MAX,
                -0.0,
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ];
            let geometries = BinaryArray::from(
                values
                    .iter()
                    .map(|_| Some(wkb.as_slice()))
                    .collect::<Vec<_>>(),
            );
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(geometries),
                    Arc::new(Float64Array::from(values.to_vec())),
                ],
            )
            .unwrap();
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                GEOMETRY,
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                false,
            );
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "float_edges".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: Some(geometry),
                    },
                }],
            };

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();
            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            let output = reader.next_batch().unwrap().unwrap();
            let actual = output
                .column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap();
            for (index, expected) in values.iter().enumerate() {
                assert!(!actual.is_null(index));
                assert_eq!(actual.value(index).to_bits(), expected.to_bits());
            }
        }

        #[test]
        fn filegdb_xyz_round_trip_preserves_z_and_contract_metadata() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("xyz.gdb");
            let schema: SchemaRef =
                Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
            let wkb = point_wkb(CoordinateDimensions::Xyz, Some(123.25), None);
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
            geometry.set_exact_geometry_types(vec![GeometryType::Point]);
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
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
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
        fn filegdb_measure_round_trip_preserves_xym_and_xyzm() {
            for (dimensions, z, m, suffix) in [
                (CoordinateDimensions::Xym, None, Some(7.5), "xym"),
                (CoordinateDimensions::Xyzm, Some(123.25), Some(7.5), "xyzm"),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join(format!("{suffix}.gdb"));
                let schema: SchemaRef =
                    Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
                let wkb = point_wkb(dimensions, z, m);
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
                geometry.dimensions = dimensions;
                geometry.set_exact_geometry_types(vec![GeometryType::Point]);
                let plan = WritePlan {
                    layers: vec![WriteLayer {
                        name: format!("points_{suffix}"),
                        contract: DataContract {
                            schema,
                            geometry: Some(geometry),
                        },
                    }],
                };

                let driver = super::super::FileGdbDriver;
                let mut writer = driver
                    .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                    .unwrap();
                writer.write(&batch).unwrap();
                writer.finish().unwrap();

                let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
                let output_contract = dataset.layers()[0].contract.geometry.as_ref().unwrap();
                assert_eq!(output_contract.dimensions, dimensions);
                let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
                let output = reader.next_batch().unwrap().unwrap();
                let geometry = output
                    .column(0)
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .unwrap();
                let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
                assert_eq!(decoded.dimensions, dimensions);
                assert_eq!(
                    decoded.value,
                    WkbValue::Point(WkbCoordinate {
                        x: 10.5,
                        y: 20.25,
                        z,
                        m,
                    })
                );
            }
        }

        #[test]
        fn filegdb_multipart_xyzm_round_trip_preserves_every_ordinate() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("multiline-xyzm.gdb");
            let dimensions = CoordinateDimensions::Xyzm;
            let geometry_value = WkbGeometry {
                value: WkbValue::MultiLineString(vec![WkbGeometry {
                    value: WkbValue::LineString(vec![
                        WkbCoordinate {
                            x: 1.0,
                            y: 2.0,
                            z: Some(3.0),
                            m: Some(4.0),
                        },
                        WkbCoordinate {
                            x: 5.0,
                            y: 6.0,
                            z: Some(7.0),
                            m: Some(8.0),
                        },
                    ]),
                    dimensions,
                    srid: None,
                }]),
                dimensions,
                srid: None,
            };
            let wkb = encode_wkb(&geometry_value, WkbFlavor::Iso).unwrap();
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
            geometry.dimensions = dimensions;
            geometry.set_exact_geometry_types(vec![GeometryType::MultiLineString]);
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "multiline_xyzm".to_owned(),
                    contract: DataContract {
                        schema,
                        geometry: Some(geometry),
                    },
                }],
            };

            let driver = super::super::FileGdbDriver;
            let mut writer = driver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            let output = reader.next_batch().unwrap().unwrap();
            let geometry = output
                .column(0)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            let decoded = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
            assert_eq!(decoded, geometry_value);
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
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap()
                .finish()
                .unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            assert_eq!(dataset.layers().len(), 1);
            let geometry = dataset.layers()[0].contract.geometry.as_ref().unwrap();
            assert_eq!(geometry.dimensions, CoordinateDimensions::Xy);
            assert_eq!(geometry.geometry_types, vec![GeometryType::Point]);
            let mut reader = dataset.open_layer_reader(&read_request()).unwrap();
            assert!(reader.next_batch().unwrap().is_none());
        }

        #[test]
        fn filegdb_drop_writer_aborts_and_removes_staging() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("aborted.gdb");
            let plan = WritePlan {
                layers: vec![write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ))],
            };
            let writer = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            let artifacts = staging_artifacts(&path);
            assert_eq!(artifacts.len(), 2);
            assert!(!path.exists());

            drop(writer);
            assert!(artifacts.iter().all(|artifact| !artifact.exists()));
            assert!(staging_artifacts(&path).is_empty());
            assert!(!path.exists());
        }

        #[test]
        fn filegdb_concurrent_staging_does_not_delete_active_writer() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("concurrent.gdb");
            let plan = WritePlan {
                layers: vec![write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ))],
            };
            let first = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            let first_artifacts = staging_artifacts(&path);
            assert_eq!(first_artifacts.len(), 2);

            let second = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            assert_eq!(staging_artifacts(&path).len(), 4);
            assert!(first_artifacts.iter().all(|artifact| artifact.exists()));

            drop(second);
            assert!(first_artifacts.iter().all(|artifact| artifact.exists()));
            first.finish().unwrap();
            assert!(path.exists());
            assert!(staging_artifacts(&path).is_empty());
        }

        #[test]
        #[ignore = "helper eseguito dal test di ownership cross-process"]
        fn filegdb_active_subprocess_helper() {
            let path = PathBuf::from(
                std::env::var_os("PLENORA_FILEGDB_ACTIVE_DEST")
                    .expect("PLENORA_FILEGDB_ACTIVE_DEST mancante"),
            );
            let ready = PathBuf::from(
                std::env::var_os("PLENORA_FILEGDB_ACTIVE_READY")
                    .expect("PLENORA_FILEGDB_ACTIVE_READY mancante"),
            );
            let release = PathBuf::from(
                std::env::var_os("PLENORA_FILEGDB_ACTIVE_RELEASE")
                    .expect("PLENORA_FILEGDB_ACTIVE_RELEASE mancante"),
            );
            let (plan, batch) = point_write_fixture();
            let mut writer = super::super::FileGdbDriver
                .create(Sink::Path(path), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            File::create(ready).unwrap();

            let deadline = Instant::now() + Duration::from_secs(10);
            while !release.exists() {
                assert!(
                    Instant::now() < deadline,
                    "timeout in attesa del rilascio dal processo padre"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            writer.finish().unwrap();
        }

        #[test]
        fn filegdb_recovery_preserves_active_cross_process_staging() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("cross-process.gdb");
            let ready = dir.path().join("writer.ready");
            let release = dir.path().join("writer.release");
            let mut child = spawn_active_subprocess(&path, &ready, &release);
            wait_until_ready(&mut child, &ready);

            let active_artifacts = staging_artifacts(&path);
            let (plan, _) = point_write_fixture();
            let second = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            let both_stagings_exist = staging_artifacts(&path).len() == 4;
            let active_was_preserved = active_artifacts.iter().all(|artifact| artifact.exists());
            drop(second);
            let active_survived_second_cleanup =
                active_artifacts.iter().all(|artifact| artifact.exists());

            File::create(release).unwrap();
            let status = child.wait().unwrap();
            assert!(status.success(), "writer attivo fallito: {status}");
            assert_eq!(active_artifacts.len(), 2);
            assert!(both_stagings_exist);
            assert!(active_was_preserved);
            assert!(active_survived_second_cleanup);
            assert!(staging_artifacts(&path).is_empty());
            assert_complete_point_dataset(path);
        }

        #[test]
        #[ignore = "helper eseguito dai test di fault injection in un sottoprocesso"]
        fn filegdb_crash_subprocess_helper() {
            let path = PathBuf::from(
                std::env::var_os("PLENORA_FILEGDB_CRASH_DEST")
                    .expect("PLENORA_FILEGDB_CRASH_DEST mancante"),
            );
            let point = std::env::var("PLENORA_FILEGDB_CRASH_POINT")
                .expect("PLENORA_FILEGDB_CRASH_POINT mancante");
            assert!(
                matches!(
                    point.as_str(),
                    "after_write" | "before_publish" | "after_publish"
                ),
                "failpoint sconosciuto: {point}"
            );

            let (plan, batch) = point_write_fixture();
            let mut writer = super::super::FileGdbDriver
                .create(Sink::Path(path), &plan, &opzioni_scrittura())
                .unwrap();
            writer.write(&batch).unwrap();
            writer.finish().unwrap();
            panic!("il failpoint '{point}' non ha terminato il sottoprocesso");
        }

        #[test]
        fn filegdb_process_crashes_leave_absent_or_complete_destination() {
            for point in ["after_write", "before_publish", "after_publish"] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join(format!("{point}.gdb"));
                let status = run_crash_subprocess(&path, point);
                assert!(!status.success(), "il failpoint '{point}' non è scattato");

                let orphaned = staging_artifacts(&path);
                if point == "after_publish" {
                    assert!(path.exists(), "destinazione assente dopo il rename");
                    assert_eq!(orphaned.len(), 1, "sidecar orfano atteso");
                    assert_complete_point_dataset(path.clone());

                    assert_eq!(recover_orphaned_staging(&path).unwrap(), 1);
                } else {
                    assert!(!path.exists(), "output parziale reso visibile");
                    assert_eq!(orphaned.len(), 2, "staging orfano atteso");

                    let (plan, batch) = point_write_fixture();
                    let mut writer = super::super::FileGdbDriver
                        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                        .unwrap();
                    assert!(
                        orphaned.iter().all(|artifact| !artifact.exists()),
                        "lo staging orfano non è stato recuperato"
                    );
                    writer.write(&batch).unwrap();
                    writer.finish().unwrap();
                }

                assert!(staging_artifacts(&path).is_empty());
                assert_complete_point_dataset(path);
            }
        }

        #[test]
        fn filegdb_failed_batch_poisons_writer_and_prevents_publish() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("failed-write.gdb");
            let layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            let schema = layer.contract.schema.clone();
            let hidden_z = point_wkb(CoordinateDimensions::Xyz, Some(3.0), None);
            let batch = RecordBatch::try_new(
                schema,
                vec![Arc::new(BinaryArray::from(vec![Some(hidden_z.as_slice())]))],
            )
            .unwrap();
            let plan = WritePlan {
                layers: vec![layer],
            };
            let mut writer = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap();
            writer.declare_input_total(LayerId(0), 1).unwrap();

            let write_error = writer
                .write(&batch)
                .expect_err("WKB XYZ nascosto in un contratto XY deve essere rifiutato");
            assert_eq!(
                write_error.category,
                plenora_io_model::ErrorCategory::DataMapping
            );
            assert_eq!(write_error.phase, plenora_io_model::ErrorPhase::Write);
            assert_eq!(write_error.retry, plenora_io_model::RetryDisposition::Never);
            assert_eq!(
                write_error.capability_reason,
                Some(CapabilityReason::CoordinateDimensions)
            );
            let diagnostics = write_error.row_diagnostics.as_deref().unwrap();
            assert_eq!(diagnostics.observed_total, 1);
            assert_eq!(diagnostics.input_total, Some(1));
            assert_eq!(diagnostics.examples[0].source_index, 0);
            assert_eq!(
                diagnostics.examples[0].cause,
                "contract.coordinate_dimensions"
            );
            let artifacts = staging_artifacts(&path);
            assert_eq!(artifacts.len(), 2);
            assert!(matches!(
                writer.write(&batch),
                Err(error)
                    if error.code == plenora_io_model::IoErrorCode::Format
                        && error.driver.as_deref() == Some("filegdb")
            ));
            assert!(matches!(
                writer.finish(),
                Err(error)
                    if error.code == plenora_io_model::IoErrorCode::Format
                        && error.driver.as_deref() == Some("filegdb")
            ));
            assert!(artifacts.iter().all(|artifact| !artifact.exists()));
            assert!(staging_artifacts(&path).is_empty());
            assert!(!path.exists());
        }

        #[test]
        fn filegdb_output_limit_failure_removes_staging_without_publish() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("too-large.gdb");
            let plan = WritePlan {
                layers: vec![write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ))],
            };
            // Un tetto di uscita a zero: la scrittura deve fallire prima di
            // pubblicare, e lo staging deve sparire.
            let options = match plenora_io_model::budget::PipelineBudget::builder()
                .limits(
                    plenora_io_model::budget::PipelineLimits::default().with_max_output_bytes(1),
                )
                .build()
            {
                Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
                Err(error) => unreachable!("bundle di test: {error:?}"),
            };
            let writer = super::super::FileGdbDriver
                .create(Sink::Path(path.clone()), &plan, &options)
                .unwrap();
            let artifacts = staging_artifacts(&path);
            assert_eq!(artifacts.len(), 2);

            assert!(matches!(
                writer.finish(),
                Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
            ));
            assert!(artifacts.iter().all(|artifact| !artifact.exists()));
            assert!(staging_artifacts(&path).is_empty());
            assert!(!path.exists());
        }

        #[test]
        fn filegdb_empty_layers_preserve_native_families_and_dimensions() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("geometry-families.gdb");
            let geometry_types = [
                GeometryType::Point,
                GeometryType::MultiPoint,
                GeometryType::MultiLineString,
                GeometryType::MultiPolygon,
            ];
            let dimensions = [
                CoordinateDimensions::Xy,
                CoordinateDimensions::Xyz,
                CoordinateDimensions::Xym,
                CoordinateDimensions::Xyzm,
            ];
            let expected: Vec<(GeometryType, CoordinateDimensions)> = geometry_types
                .iter()
                .flat_map(|geometry_type| {
                    dimensions
                        .iter()
                        .map(move |dimensions| (*geometry_type, *dimensions))
                })
                .collect();
            let layers = expected
                .iter()
                .enumerate()
                .map(|(index, (geometry_type, dimensions))| {
                    let name = format!("family_{index}");
                    let schema: SchemaRef =
                        Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:3857")]));
                    let mut geometry = GeometryColumnContract::wkb_xy(
                        FieldId(0),
                        GEOMETRY,
                        ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                        true,
                    );
                    geometry.dimensions = *dimensions;
                    geometry.set_exact_geometry_types(vec![*geometry_type]);
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
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .unwrap()
                .finish()
                .unwrap();

            let dataset = driver.open(Source::Path(path), opzioni_lettura()).unwrap();
            assert_eq!(dataset.layers().len(), expected.len());
            for (layer, (expected_type, expected_dimensions)) in
                dataset.layers().iter().zip(expected)
            {
                let geometry = layer.contract.geometry.as_ref().unwrap();
                assert_eq!(geometry.dimensions, expected_dimensions);
                assert_eq!(geometry.geometry_types, vec![expected_type]);
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
                layer
                    .contract
                    .geometry
                    .as_mut()
                    .unwrap()
                    .set_exact_geometry_types(vec![geometry_type]);
                let plan = WritePlan {
                    layers: vec![layer],
                };

                let result = super::super::FileGdbDriver.create(
                    Sink::Path(path.clone()),
                    &plan,
                    &opzioni_scrittura(),
                );
                assert!(matches!(
                    result,
                    Err(error)
                        if error.capability_reason
                            == Some(CapabilityReason::GeometryNotSupported)
                ));
                assert!(!path.exists());
            }
        }

        #[test]
        fn filegdb_rejects_ewkb_before_output_creation() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ewkb.gdb");
            let mut layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            layer.contract.geometry.as_mut().unwrap().encoding = GeometryEncoding::Ewkb;
            let plan = WritePlan {
                layers: vec![layer],
            };

            let result = super::super::FileGdbDriver.create(
                Sink::Path(path.clone()),
                &plan,
                &opzioni_scrittura(),
            );
            assert!(matches!(
                result,
                Err(error)
                    if error.capability_reason == Some(CapabilityReason::GeometryEncoding)
            ));
            assert!(!path.exists());
        }

        #[test]
        fn filegdb_rejects_non_round_trip_attribute_types_before_output() {
            for (data_type, suffix) in [
                (DataType::Int64, "int64"),
                (DataType::Boolean, "boolean"),
                (DataType::Date32, "date32"),
                (DataType::Binary, "binary"),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join(format!("{suffix}.gdb"));
                let mut layer = write_layer(ResolvedCrs::new(
                    Some("EPSG:3857".to_owned()),
                    CrsKind::Projected,
                    None,
                ));
                layer.contract.schema = Arc::new(Schema::new(vec![
                    geometry_field(GEOMETRY, "EPSG:3857"),
                    Field::new("unsupported", data_type, true),
                ]));
                let plan = WritePlan {
                    layers: vec![layer],
                };

                let result = super::super::FileGdbDriver.create(
                    Sink::Path(path.clone()),
                    &plan,
                    &opzioni_scrittura(),
                );
                assert!(matches!(
                    result,
                    Err(error)
                        if error.capability_reason
                            == Some(CapabilityReason::TypeNotRepresentable)
                ));
                assert!(!path.exists());
            }
        }

        #[test]
        fn filegdb_rejects_incoherent_native_field_metadata_before_output() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("bad-native-metadata.gdb");
            let mut layer = write_layer(ResolvedCrs::new(
                Some("EPSG:3857".to_owned()),
                CrsKind::Projected,
                None,
            ));
            let metadata = HashMap::from([(
                OGR_FIELD_TYPE_KEY.to_owned(),
                gdal::vector::OGRFieldType::OFTReal.to_string(),
            )]);
            layer.contract.schema = Arc::new(Schema::new(vec![
                geometry_field(GEOMETRY, "EPSG:3857"),
                Field::new("text", DataType::Utf8, true).with_metadata(metadata),
            ]));
            let plan = WritePlan {
                layers: vec![layer],
            };

            let result = super::super::FileGdbDriver.create(
                Sink::Path(path.clone()),
                &plan,
                &opzioni_scrittura(),
            );
            assert!(matches!(
                result,
                Err(error)
                    if error.capability_reason
                        == Some(CapabilityReason::TypeNotRepresentable)
            ));
            assert!(!path.exists());
        }
    }
}

#[cfg(all(test, not(feature = "gdal-backend")))]
mod tests {
    use super::*;

    /// Opzioni di lettura sul modello unificato.
    ///
    /// Da S4.d il percorso di lettura vive interamente li': la memoria dei
    /// batch e' una `InternalMemoryLease`, che esiste solo dentro un
    /// `PipelineContext`. `opzioni_lettura()` costruisce ancora il ramo
    /// legacy — sparira' in S4.e — e con quello `open` fallisce chiuso.
    /// Opzioni di scrittura sul modello unificato.
    ///
    /// `opzioni_scrittura()` non esiste piu' (S4.e): le opzioni portano un
    /// `OperationBudget`, che nasce da una costruzione che puo' fallire.
    fn opzioni_scrittura() -> WriteOptions {
        match plenora_io_model::budget::PipelineBudget::builder().build() {
            Ok(bundle) => WriteOptions::from_write_parts(bundle.into_write_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    fn opzioni_lettura() -> ReadOptions {
        match plenora_io_model::budget::PipelineBudget::builder().build() {
            Ok(bundle) => ReadOptions::from_read_parts(bundle.into_read_parts()),
            Err(error) => unreachable!("bundle di test non costruibile: {error:?}"),
        }
    }

    #[test]
    fn open_without_gdal_feature_is_typed() {
        // Nel build di default (senza feature) l'apertura fallisce tipizzata.
        let e = FileGdbDriver
            .open(Source::Path("x.gdb".into()), opzioni_lettura())
            .map(|_| ())
            .unwrap_err();
        assert!(matches!(
            e,
            error if error.code == plenora_io_model::IoErrorCode::Unsupported
        ));
    }
}

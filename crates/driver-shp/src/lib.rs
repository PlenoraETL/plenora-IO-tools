//! driver-shp — Shapefile ⇄ `RecordBatch`. Le shape XY/M/Z diventano WKB
//! `geoarrow.wkb` XY/XYM/XYZ/XYZM senza passare da `geo-types`; il dbf fornisce
//! gli attributi e il `.prj` (o `assume_crs`) il CRS.
//!
//! Scrittura (Fase 2B): capability-check fail-closed (ADR-IO 3) — nomi campo dbf
//! ≤10 char (imposto da `FieldName`), tipo geometria unico per file (imposto da
//! shapefile). Il publish **multi-file** espone entrambe le modalità di ADR-IO 2:
//! `*.shp.d` è uno `ShapefileDirectoryDataset` pubblicato con un unico rename
//! atomico; `*.shp` è un `LooseShapefileSet` compatibile, pubblicato con rename
//! ordinati e `.shp` per ultimo. `.prj` è scritto se c'è una definizione WKT o
//! per WGS84; nessuna riproiezione (ADR-IO 4).
#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::builder::BinaryBuilder;
use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, RecordBatchOptions};
#[cfg(test)]
use arrow_schema::DataType;
use arrow_schema::{Field, Schema, SchemaRef};
use serde_json::Value as JsonValue;
use shapefile::dbase::{FieldValue, Record, TableWriterBuilder};
use shapefile::{
    Multipoint, MultipointM, MultipointZ, Point, PointM, PointZ, Polygon, PolygonM, PolygonRing,
    PolygonZ, Polyline, PolylineM, PolylineZ, Shape, ShapeReader, ShapeType, Writer, NO_DATA,
};

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
use plenora_io_core::loss::{LossExample, LossReport};
use plenora_io_core::publish::{
    create_staged_dir, publish_dir_atomic, publish_files_ordered_limited,
};
use plenora_io_core::request::{ReadRequest, ReadScope};
use plenora_io_core::{
    validate_write, with_write_validation, write_row_rejection, AttributeWriteSupport,
    CrsRepresentationCapabilities, CrsRepresentationState, CrsWriteSupport,
    FormatWriteCapabilities, NullabilitySupport, TypeCoercionPolicy, WritePlan, DBF_FIELD_NAMES,
    SCALAR_TYPES, WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY,
};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::{CrsKind, RawCrs, ResolvedCrs};
use plenora_io_model::geometry::{with_geometry_contract_metadata, GEO_CRS_KEY};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{
    decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
};
use plenora_io_model::{
    PlenoraIoError, Result, RowDiagnosticExample, RowDiagnosticKey, RowDiagnosticKeyState,
    RowDiagnosticKeyValue, RowDiagnosticScope, RowDiagnostics, RowDiagnosticsCompleteness,
    ROW_DIAGNOSTICS_CONTRACT, ROW_DIAGNOSTICS_INDEX_BASIS,
};

const GEOMETRY: &str = "geometry";
const DIRECTORY_DATASET_SUFFIX: &str = ".shp.d";
const DIRECTORY_DATASET_MODE: &str = "shapefile_directory_dataset";
const LOOSE_SET_MODE: &str = "loose_shapefile_set";
const DBF_NUMERIC_INTEGER_PRECISION_UNVERIFIABLE: &str =
    "dbf_numeric_integer_precision_unverifiable";
const FIRST_F64_INTEGER_WITHOUT_UNIT_PRECISION: f64 = 9_007_199_254_740_992.0;
const DBF_HEADER_SIZE: usize = 32;
const DBF_FIELD_DESCRIPTOR_SIZE: usize = 32;
const DBF_HEADER_TERMINATOR_SIZE: usize = 1;
#[cfg(test)]
const DBF_FIELD_NAME_SIZE: usize = 11;
const DBF_VISUAL_FOXPRO_VERSION: u8 = 0x30;
const DBF_VISUAL_FOXPRO_BACKLINK_SIZE: usize = 263;
const DEFAULT_ROW_DIAGNOSTICS_EXAMPLES_LIMIT: u64 = 64;
const MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT: u64 = 64;
const INNER_RING_WITHOUT_OUTER_CAUSE: &str = "shapefile.inner_ring_without_outer";
const POLYGON_WITHOUT_OUTER_CAUSE: &str = "shapefile.polygon_without_outer";
const UNCLOSED_RING_CAUSE: &str = "shapefile.unclosed_ring";
const DEGENERATE_RING_CAUSE: &str = "shapefile.degenerate_ring";
const ATTRIBUTE_NUMERIC_INVALID_CAUSE: &str = "shapefile.attribute_numeric_invalid";

/// WKT standard per WGS84 (accettato da GDAL), usato per il `.prj` quando la
/// sorgente dà solo il codice autorità e non una definizione WKT.
const WGS84_WKT: &str = "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]";

fn err(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("shp", reason)
}

#[derive(Clone, Copy)]
enum DiagnosticKeyPolicy {
    /// Espone il valore lessicale DBF soltanto quando `key_field` e' stato
    /// configurato esplicitamente e il valore e' attestabile.
    Emit,
    /// Espone esclusivamente nome campo e stato `redacted`, mai il valore DBF.
    Redact,
}

#[derive(Clone)]
struct DiagnosticKeyConfig {
    field: String,
    policy: DiagnosticKeyPolicy,
    raw_numeric_field_index: Option<usize>,
}

#[derive(Clone)]
struct ShpRowDiagnosticsConfig {
    examples_limit: u64,
    /// `None` significa che gli esempi non contengono alcun oggetto `key`;
    /// non esiste una policy implicita.
    key: Option<DiagnosticKeyConfig>,
}

impl ShpRowDiagnosticsConfig {
    fn from_options(
        options: &BTreeMap<String, String>,
        columns: &[ShpColumn],
        dbf_layout: &DbfLayout,
    ) -> Result<Self> {
        let examples_limit = options.get("row_diagnostics.examples_limit").map_or(
            Ok(DEFAULT_ROW_DIAGNOSTICS_EXAMPLES_LIMIT),
            |value| {
                value.parse::<u64>().map_err(|_| {
                    PlenoraIoError::new(
                        plenora_io_model::ErrorCategory::InvalidConfiguration,
                        plenora_io_model::ErrorPhase::Validate,
                        plenora_io_model::RemoteEffect::None,
                        plenora_io_model::RetryDisposition::Never,
                        "row_diagnostics.examples_limit deve essere un intero",
                    )
                })
            },
        )?;
        if !(1..=MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT).contains(&examples_limit) {
            return Err(PlenoraIoError::new(
                plenora_io_model::ErrorCategory::InvalidConfiguration,
                plenora_io_model::ErrorPhase::Validate,
                plenora_io_model::RemoteEffect::None,
                plenora_io_model::RetryDisposition::Never,
                format!(
                    "row_diagnostics.examples_limit deve essere compreso tra 1 e {MAX_ROW_DIAGNOSTICS_EXAMPLES_LIMIT}"
                ),
            ));
        }

        let key = match options.get("row_diagnostics.key_field") {
            None => {
                if options.contains_key("row_diagnostics.key_policy") {
                    return Err(PlenoraIoError::new(
                        plenora_io_model::ErrorCategory::InvalidConfiguration,
                        plenora_io_model::ErrorPhase::Validate,
                        plenora_io_model::RemoteEffect::None,
                        plenora_io_model::RetryDisposition::Never,
                        "row_diagnostics.key_policy richiede row_diagnostics.key_field",
                    ));
                }
                None
            }
            Some(field) => {
                let _column = columns
                    .iter()
                    .find(|column| column.name == *field)
                    .ok_or_else(|| {
                        PlenoraIoError::new(
                            plenora_io_model::ErrorCategory::InvalidConfiguration,
                            plenora_io_model::ErrorPhase::Validate,
                            plenora_io_model::RemoteEffect::None,
                            plenora_io_model::RetryDisposition::Never,
                            "row_diagnostics.key_field non esiste nello schema DBF",
                        )
                    })?;
                let policy = match options
                    .get("row_diagnostics.key_policy")
                    .map(String::as_str)
                {
                    Some("emit") => DiagnosticKeyPolicy::Emit,
                    Some("redact") => DiagnosticKeyPolicy::Redact,
                    _ => {
                        return Err(PlenoraIoError::new(
                            plenora_io_model::ErrorCategory::InvalidConfiguration,
                            plenora_io_model::ErrorPhase::Validate,
                            plenora_io_model::RemoteEffect::None,
                            plenora_io_model::RetryDisposition::Never,
                            "row_diagnostics.key_policy deve essere 'emit' o 'redact'",
                        ))
                    }
                };
                Some(DiagnosticKeyConfig {
                    field: field.clone(),
                    policy,
                    raw_numeric_field_index: dbf_layout.fields.iter().position(|layout| {
                        layout.name == *field && matches!(layout.field_type, b'N' | b'F')
                    }),
                })
            }
        };
        Ok(Self {
            examples_limit,
            key,
        })
    }
}

struct ShpRowDiagnostics {
    config: ShpRowDiagnosticsConfig,
    counts: BTreeMap<String, u64>,
    observed_total: u64,
    examples: Vec<RowDiagnosticExample>,
}

impl ShpRowDiagnostics {
    const fn new(config: ShpRowDiagnosticsConfig) -> Self {
        Self {
            config,
            counts: BTreeMap::new(),
            observed_total: 0,
            examples: Vec::new(),
        }
    }

    const fn is_empty(&self) -> bool {
        self.observed_total == 0
    }

    fn record(
        &mut self,
        source_index: u64,
        cause: &'static str,
        record: Option<&Record>,
        raw_numeric_key: Option<&str>,
    ) {
        self.observed_total += 1;
        *self.counts.entry(cause.to_owned()).or_default() += 1;
        if self.examples.len() as u64 >= self.config.examples_limit {
            return;
        }
        let key = self.config.key.as_ref().map(|config| match config.policy {
            DiagnosticKeyPolicy::Redact => RowDiagnosticKey {
                field: config.field.clone(),
                state: RowDiagnosticKeyState::Redacted,
                value: None,
            },
            DiagnosticKeyPolicy::Emit => {
                let decoded = config
                    .raw_numeric_field_index
                    .is_none()
                    .then(|| {
                        record
                            .and_then(|row| row.get(&config.field))
                            .and_then(fv_string)
                    })
                    .flatten();
                let value = raw_numeric_key.map(str::to_owned).or(decoded);
                match value {
                    Some(value) if value.len() <= 1024 => RowDiagnosticKey {
                        field: config.field.clone(),
                        state: RowDiagnosticKeyState::Value,
                        value: Some(RowDiagnosticKeyValue::String(value)),
                    },
                    _ => RowDiagnosticKey {
                        field: config.field.clone(),
                        state: RowDiagnosticKeyState::Unavailable,
                        value: None,
                    },
                }
            }
        });
        self.examples.push(RowDiagnosticExample {
            source_index,
            cause: cause.to_owned(),
            column: None,
            key,
            write_state: None,
        });
    }

    fn into_report(self) -> RowDiagnostics {
        let total = self.observed_total;
        self.into_report_with(RowDiagnosticsCompleteness::Complete, None, Some(total))
    }

    fn into_partial_report(self, knowledge_limit: &str) -> RowDiagnostics {
        self.into_report_with(
            RowDiagnosticsCompleteness::Partial,
            Some(vec![knowledge_limit.to_owned()]),
            None,
        )
    }

    fn into_partial_error(self, error: PlenoraIoError, knowledge_limit: &str) -> PlenoraIoError {
        if self.is_empty() {
            error
        } else {
            error.with_row_diagnostics(self.into_partial_report(knowledge_limit))
        }
    }

    fn into_report_with(
        self,
        completeness: RowDiagnosticsCompleteness,
        knowledge_limits: Option<Vec<String>>,
        total: Option<u64>,
    ) -> RowDiagnostics {
        let examples_truncated = self.observed_total > self.examples.len() as u64;
        RowDiagnostics {
            contract: ROW_DIAGNOSTICS_CONTRACT.to_owned(),
            scope: RowDiagnosticScope::Read,
            index_basis: ROW_DIAGNOSTICS_INDEX_BASIS.to_owned(),
            completeness,
            knowledge_limits,
            observed_total: self.observed_total,
            total,
            input_total: None,
            counts: self.counts,
            examples_limit: self.config.examples_limit,
            examples_truncated,
            examples: self.examples,
            diagnostic_state_counts: None,
            write_outcome: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShapefilePublishMode {
    DirectoryDataset,
    LooseSet,
}

fn is_directory_dataset_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.to_ascii_lowercase()
                .ends_with(DIRECTORY_DATASET_SUFFIX)
        })
}

fn publish_mode(path: &Path, opts: &WriteOptions) -> Result<ShapefilePublishMode> {
    let inferred = if is_directory_dataset_path(path) {
        ShapefilePublishMode::DirectoryDataset
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("shp"))
    {
        ShapefilePublishMode::LooseSet
    } else {
        return Err(PlenoraIoError::Unsupported(
            "l'output Shapefile deve terminare con .shp (loose set) o .shp.d (directory dataset)"
                .to_owned(),
        ));
    };
    let Some(requested) = opts.format_options.get("publish_mode") else {
        return Ok(inferred);
    };
    let requested = match requested.as_str() {
        DIRECTORY_DATASET_MODE => ShapefilePublishMode::DirectoryDataset,
        LOOSE_SET_MODE => ShapefilePublishMode::LooseSet,
        other => {
            return Err(PlenoraIoError::Unsupported(format!(
                "publish_mode Shapefile '{other}' non valido; usare '{DIRECTORY_DATASET_MODE}' o '{LOOSE_SET_MODE}'"
            )))
        }
    };
    if requested != inferred {
        return Err(PlenoraIoError::Unsupported(format!(
            "publish_mode '{}' richiede una destinazione {}",
            requested.name(),
            requested.destination_suffix()
        )));
    }
    Ok(requested)
}

impl ShapefilePublishMode {
    const fn name(self) -> &'static str {
        match self {
            Self::DirectoryDataset => DIRECTORY_DATASET_MODE,
            Self::LooseSet => LOOSE_SET_MODE,
        }
    }

    const fn destination_suffix(self) -> &'static str {
        match self {
            Self::DirectoryDataset => "*.shp.d",
            Self::LooseSet => "*.shp",
        }
    }
}

fn shapefile_source_path(path: PathBuf) -> Result<PathBuf> {
    if !path.is_dir() {
        return Ok(path);
    }
    if !is_directory_dataset_path(&path) {
        return Err(PlenoraIoError::Unsupported(
            "directory Shapefile non riconosciuta (atteso *.shp.d)".to_owned(),
        ));
    }
    let source = path.join("data.shp");
    if !source.is_file() {
        return Err(err("directory dataset senza data.shp"));
    }
    Ok(source)
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "shp",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential,
    read_determinism: plenora_io_core::DeterminismLevel::Semantic,
    write_mode: Some(WriteMode::Streaming),
    write_determinism: Some(plenora_io_core::DeterminismLevel::Semantic),
    multi_layer: false,
    multi_file: true, // .shp/.shx/.dbf/.prj
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    projection_support: plenora_io_core::ProjectionSupport::Exact,
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::None,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::None,
    crs_handling: CrsHandling::Embedded,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
        field_names: DBF_FIELD_NAMES,
        allowed_types: SCALAR_TYPES,
        type_coercion: TypeCoercionPolicy::ExplicitText,
        attributes: AttributeWriteSupport::All,
        geometry: WKB_SINGLE_TYPE_ALL_DIMENSIONS_GEOMETRY,
        crs: CrsWriteSupport::Embedded,
        crs_representations: CrsRepresentationCapabilities::new(
            CrsRepresentationState::Derived,
            CrsRepresentationState::Absent,
            CrsRepresentationState::Preserved,
        ),
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 9,
    descriptor_version: 7,
};

pub struct ShpDriver;

impl FormatDriver for ShpDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, mut opts: ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = shapefile_source_path(plenora_io_core::preflight_source(source, &mut opts)?)?;
        let crs = resolve_crs(&path, &opts)?;
        // Pass 1: inferenza schema (nomi + tipi) dai record, a RAM O(ncol).
        let ShpInference {
            cols,
            dbf_layout,
            geometry_info,
            active_row_count,
            loss,
        } = infer_shp_schema(&path)?;
        let row_diagnostics =
            ShpRowDiagnosticsConfig::from_options(&opts.format_options, &cols, &dbf_layout)?;
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, crs.clone(), true);
        geometry_contract.dimensions = geometry_info.dimensions;
        geometry_contract.set_exact_geometry_types(geometry_info.geometry_types);
        if let Some(shape_type) = geometry_info.shape_type {
            geometry_contract
                .native_metadata
                .insert("shp.shape_type".to_owned(), shape_type.to_owned());
        }
        if matches!(
            geometry_contract.dimensions,
            CoordinateDimensions::Xym | CoordinateDimensions::Xyzm
        ) {
            geometry_contract
                .native_metadata
                .insert("shp.measure_no_data".to_owned(), NO_DATA.to_string());
        }
        let crs_id = resolved_crs_id(&crs)?;
        let geometry_field =
            with_geometry_contract_metadata(&geometry_field(GEOMETRY, crs_id), &geometry_contract);
        let mut fields = vec![geometry_field];
        for column in &cols {
            fields.push(Field::new(
                &column.name,
                column.column_type.arrow_data_type(),
                true,
            ));
        }
        let schema: SchemaRef = Arc::new(Schema::new(fields));
        let contract = DataContract::new(schema, Some(geometry_contract.clone()));
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("layer")
            .to_owned();
        Ok(plenora_io_core::with_read_budget(
            Box::new(ShpDataset {
                path,
                cols,
                dbf_layout,
                dimensions: geometry_contract.dimensions,
                shape_type: geometry_info.shape_type,
                active_row_count,
                loss,
                row_diagnostics,
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name,
                    contract,
                }],
            }),
            &opts,
            false,
        ))
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(self.descriptor(), plan, opts.max_columns())?;
        let Sink::Path(dest) = sink;
        let publish_mode = publish_mode(&dest, opts)?;
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::Unsupported(
                "Shapefile: un solo layer per file".to_owned(),
            ));
        }
        match publish_mode {
            ShapefilePublishMode::DirectoryDataset => {
                if dest.exists() {
                    return Err(PlenoraIoError::OutputExists(dest.display().to_string()));
                }
            }
            ShapefilePublishMode::LooseSet => {
                // no-clobber sull'intero set.
                for ext in ["shp", "shx", "dbf", "prj"] {
                    let sibling = dest.with_extension(ext);
                    if sibling.exists() {
                        return Err(PlenoraIoError::OutputExists(sibling.display().to_string()));
                    }
                }
            }
        }

        let layer = &plan.layers[0];
        let schema = &layer.contract.schema;
        let geom_idx = geometry_index(schema)
            .ok_or_else(|| err("il contratto non ha una colonna geometria geoarrow.wkb"))?;

        // Capability-check (ADR-IO 3): costruisce il dbf, fail-closed sui nomi.
        let mut table = TableWriterBuilder::new();
        let mut attrs: Vec<(usize, String, DbfKind)> = Vec::new();
        for (i, f) in schema.fields().iter().enumerate() {
            if i == geom_idx {
                continue;
            }
            let fname = shapefile::dbase::FieldName::try_from(f.name().as_str()).map_err(|_| {
                PlenoraIoError::Unsupported(format!(
                    "nome campo '{}' non valido per dbf (max 10 caratteri ASCII)",
                    f.name()
                ))
            })?;
            let kind = DbfKind::from(f.data_type());
            table = match kind {
                DbfKind::Char => table.add_character_field(fname, 254),
                DbfKind::Int => table.add_numeric_field(fname, 18, 0),
                DbfKind::Float => table.add_numeric_field(fname, 20, 8),
                DbfKind::Logical => table.add_logical_field(fname),
            };
            attrs.push((i, f.name().clone(), kind));
        }

        let staging = create_staged_dir(&dest)?;
        let shp_path = staging.path().join("data.shp");
        let writer = Writer::from_path(&shp_path, table)
            .map_err(|e| err(format!("creazione shapefile: {e}")))?;

        with_write_validation(
            Box::new(ShpWriter {
                staging: Some(staging),
                writer: Some(writer),
                dest,
                durable: opts.durable,
                publish_mode,
                attrs,
                geom_idx,
                prj: resolve_prj(layer, schema, geom_idx),
                shape_type: None,
                rows: 0,
                input_total: None,
                wkb_limits: opts.wkb_limits(),
                max_output_bytes: opts.max_output_bytes(),
            }),
            self.descriptor(),
            plan,
            opts,
        )
    }
}

// --- lettura streaming -----------------------------------------------------

struct ShpDataset {
    path: PathBuf,
    cols: Vec<ShpColumn>,
    dbf_layout: DbfLayout,
    dimensions: CoordinateDimensions,
    shape_type: Option<&'static str>,
    active_row_count: u64,
    loss: LossReport,
    row_diagnostics: ShpRowDiagnosticsConfig,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for ShpDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(DESCRIPTOR.id, DESCRIPTOR.fidelity_class)
            .with_loss_report(&self.loss)
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let (indices, layer) = plenora_io_core::project_layer_contract(&self.layers[0], request)?;
        let include_geometry = indices.binary_search(&0).is_ok();
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
        let reader = spawn_parser(ShpParserInput {
            path: self.path.clone(),
            schema: layer.contract.schema.clone(),
            cols,
            dbf_layout: self.dbf_layout.clone(),
            dimensions: self.dimensions,
            expected_shape_type: self.shape_type,
            expected_active_rows: self.active_row_count,
            include_geometry,
            batch_sizer,
            layer,
            loss: self.loss.clone(),
            row_diagnostics: self.row_diagnostics.clone(),
            scope: request.scope,
            cancellation: request.cancellation.clone(),
        })?;
        Ok(plenora_io_core::with_cancellation(
            reader,
            request.cancellation.clone(),
        ))
    }
}

// --- scrittura -------------------------------------------------------------

#[derive(Clone, Copy)]
enum DbfKind {
    Char,
    Int,
    Float,
    Logical,
}

impl DbfKind {
    const fn from(dt: &arrow_schema::DataType) -> Self {
        use arrow_schema::DataType as D;
        match dt {
            D::Int8
            | D::Int16
            | D::Int32
            | D::Int64
            | D::UInt8
            | D::UInt16
            | D::UInt32
            | D::UInt64 => Self::Int,
            D::Float16 | D::Float32 | D::Float64 => Self::Float,
            D::Boolean => Self::Logical,
            _ => Self::Char,
        }
    }
}

struct ShpWriter {
    staging: Option<tempfile::TempDir>,
    writer: Option<Writer<BufWriter<File>>>,
    dest: PathBuf,
    durable: bool,
    publish_mode: ShapefilePublishMode,
    attrs: Vec<(usize, String, DbfKind)>,
    geom_idx: usize,
    prj: Option<String>,
    shape_type: Option<&'static str>,
    rows: u64,
    input_total: Option<u64>,
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
}

impl FormatWriter for ShpWriter {
    fn declare_input_total(&mut self, layer: LayerId, total: u64) -> Result<()> {
        if layer.0 != 0 {
            return Err(PlenoraIoError::Unsupported(
                "Shapefile supporta un solo layer".to_owned(),
            ));
        }
        self.input_total = Some(total);
        Ok(())
    }

    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        let geom_col = batch
            .column(self.geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err("colonna geometria non binaria"))?;
        let limits = self.wkb_limits;
        let mut st = self.shape_type;
        let mut prepared = Vec::with_capacity(batch.num_rows());
        let mut rejections = Vec::new();
        for row in 0..batch.num_rows() {
            if geom_col.is_null(row) {
                rejections.push((row, "shapefile.null_geometry_unsupported", GEOMETRY));
                continue;
            }
            let Ok(geometry) = decode_wkb(geom_col.value(row), &limits) else {
                rejections.push((row, "shapefile.invalid_geometry", GEOMETRY));
                continue;
            };
            let Ok(shape) = shape_from_wkb(geometry) else {
                rejections.push((row, "shapefile.geometry_not_representable", GEOMETRY));
                continue;
            };
            // Capability-check (ADR-IO 3): un unico tipo di geometria per file.
            let tag = shape_tag(&shape);
            if tag == "unsupported" {
                rejections.push((row, "shapefile.geometry_type_unsupported", GEOMETRY));
                continue;
            }
            if !tag.is_empty() && st.is_some_and(|existing| existing != tag) {
                rejections.push((row, "shapefile.mixed_geometry_type", GEOMETRY));
                continue;
            }
            let mut rec = Record::default();
            let mut valid_record = true;
            for (col, name, kind) in &self.attrs {
                let Ok(value) = cell_to_field(batch.column(*col), row, *kind) else {
                    rejections.push((row, "shapefile.cell_not_representable", name.as_str()));
                    valid_record = false;
                    break;
                };
                rec.insert(name.clone(), value);
            }
            if valid_record {
                if !tag.is_empty() && st.is_none() {
                    st = Some(tag);
                }
                prepared.push((shape, rec));
            }
        }
        if !rejections.is_empty() {
            return Err(write_row_rejection(
                "shp",
                self.rows,
                batch.num_rows(),
                &rejections,
                self.input_total,
            ));
        }
        let w = self.writer.as_mut().ok_or_else(|| err("writer chiuso"))?;
        for (shape, rec) in prepared {
            write_shape(w, shape, &rec)?;
        }
        self.shape_type = st;
        self.rows = self
            .rows
            .checked_add(
                u64::try_from(batch.num_rows()).map_err(|_| {
                    PlenoraIoError::LimitExceeded("troppe righe Shapefile".to_owned())
                })?,
            )
            .ok_or_else(|| PlenoraIoError::LimitExceeded("troppe righe Shapefile".to_owned()))?;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        // Finalizza .shp/.shx/.dbf (header + bounding box) rilasciando il writer.
        let w = self.writer.take().ok_or_else(|| err("writer già chiuso"))?;
        drop(w);
        let staging = self.staging.take().ok_or_else(|| err("staging mancante"))?;

        if let Some(wkt) = &self.prj {
            std::fs::write(staging.path().join("data.prj"), wkt)?;
        }

        let staged_bytes = ["dbf", "shx", "prj", "shp"]
            .into_iter()
            .map(|ext| staging.path().join(format!("data.{ext}")))
            .filter(|path| path.exists())
            .try_fold(0_u64, |total, path| {
                let bytes = std::fs::metadata(path)?.len();
                total.checked_add(bytes).ok_or_else(|| {
                    PlenoraIoError::LimitExceeded(
                        "overflow nel conteggio dell'output Shapefile".to_owned(),
                    )
                })
            })?;
        if staged_bytes > self.max_output_bytes {
            return Err(PlenoraIoError::LimitExceeded(format!(
                "output Shapefile da {staged_bytes} byte oltre il limite di {}",
                self.max_output_bytes
            )));
        }

        let (bytes, outcome) = match self.publish_mode {
            ShapefilePublishMode::DirectoryDataset => {
                let outcome = publish_dir_atomic(staging.path(), &self.dest, self.durable)?;
                (staged_bytes, outcome)
            }
            ShapefilePublishMode::LooseSet => {
                // Companion prima, .shp marker per ultimo.
                let files = ["dbf", "shx", "prj", "shp"]
                    .into_iter()
                    .map(|extension| {
                        (
                            staging.path().join(format!("data.{extension}")),
                            self.dest.with_extension(extension),
                        )
                    })
                    .filter(|(source, _)| source.exists())
                    .collect::<Vec<_>>();
                publish_files_ordered_limited(&files, self.durable, self.max_output_bytes)?
            }
        };
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            fidelity: plenora_io_core::FidelityAssessment::lossless(),
            outcome,
        })
    }
}

enum ShpTopology {
    Point(WkbCoordinate),
    Multipoint(Vec<WkbCoordinate>),
    Polyline(Vec<Vec<WkbCoordinate>>),
    /// `true` marks an exterior ring, `false` an interior ring.
    Polygon(Vec<(bool, Vec<WkbCoordinate>)>),
}

fn take_child(
    child: WkbGeometry,
    parent_dimensions: CoordinateDimensions,
    expected: GeometryType,
) -> Result<WkbValue> {
    if child.srid.is_some()
        || child.dimensions != parent_dimensions
        || child.geometry_type() != expected
    {
        return Err(err("geometria WKB annidata incoerente per Shapefile"));
    }
    Ok(child.value)
}

fn polygon_rings(
    rings: Vec<Vec<WkbCoordinate>>,
    destination: &mut Vec<(bool, Vec<WkbCoordinate>)>,
) -> Result<()> {
    if rings.is_empty() {
        return Err(err("poligono vuoto non rappresentabile in Shapefile"));
    }
    for (index, ring) in rings.into_iter().enumerate() {
        if ring.len() < 4 || ring.first() != ring.last() {
            return Err(err(
                "anello WKB non chiuso o con meno di quattro coordinate",
            ));
        }
        destination.push((index == 0, ring));
    }
    Ok(())
}

fn topology_from_wkb(geometry: WkbGeometry) -> Result<ShpTopology> {
    if geometry.srid.is_some() {
        return Err(err(
            "SRID embedded non rappresentabile nel payload Shapefile; usare il CRS del layer",
        ));
    }
    let dimensions = geometry.dimensions;
    match geometry.value {
        WkbValue::Point(coordinate) => Ok(ShpTopology::Point(coordinate)),
        WkbValue::MultiPoint(children) => {
            if children.is_empty() {
                return Err(err("MultiPoint vuoto non rappresentabile in Shapefile"));
            }
            let mut coordinates = Vec::with_capacity(children.len());
            for child in children {
                match take_child(child, dimensions, GeometryType::Point)? {
                    WkbValue::Point(coordinate) => coordinates.push(coordinate),
                    _ => return Err(err("MultiPoint con membro non-Point")),
                }
            }
            Ok(ShpTopology::Multipoint(coordinates))
        }
        WkbValue::LineString(coordinates) => {
            if coordinates.len() < 2 {
                return Err(err(
                    "LineString con meno di due coordinate non rappresentabile in Shapefile",
                ));
            }
            Ok(ShpTopology::Polyline(vec![coordinates]))
        }
        WkbValue::MultiLineString(children) => {
            if children.is_empty() {
                return Err(err(
                    "MultiLineString vuoto non rappresentabile in Shapefile",
                ));
            }
            let mut parts = Vec::with_capacity(children.len());
            for child in children {
                match take_child(child, dimensions, GeometryType::LineString)? {
                    WkbValue::LineString(coordinates) if coordinates.len() >= 2 => {
                        parts.push(coordinates);
                    }
                    WkbValue::LineString(_) => {
                        return Err(err(
                            "parte LineString con meno di due coordinate in Shapefile",
                        ))
                    }
                    _ => return Err(err("MultiLineString con membro non-LineString")),
                }
            }
            Ok(ShpTopology::Polyline(parts))
        }
        WkbValue::Polygon(rings) => {
            let mut destination = Vec::with_capacity(rings.len());
            polygon_rings(rings, &mut destination)?;
            Ok(ShpTopology::Polygon(destination))
        }
        WkbValue::MultiPolygon(children) => {
            if children.is_empty() {
                return Err(err("MultiPolygon vuoto non rappresentabile in Shapefile"));
            }
            let mut destination = Vec::new();
            for child in children {
                match take_child(child, dimensions, GeometryType::Polygon)? {
                    WkbValue::Polygon(rings) => polygon_rings(rings, &mut destination)?,
                    _ => return Err(err("MultiPolygon con membro non-Polygon")),
                }
            }
            Ok(ShpTopology::Polygon(destination))
        }
        WkbValue::GeometryCollection(_) => {
            Err(err("GeometryCollection non rappresentabile in Shapefile"))
        }
        WkbValue::CircularString(_)
        | WkbValue::CompoundCurve(_)
        | WkbValue::CurvePolygon(_)
        | WkbValue::MultiCurve(_)
        | WkbValue::MultiSurface(_)
        | WkbValue::PolyhedralSurface(_)
        | WkbValue::Tin(_)
        | WkbValue::Triangle(_) => Err(err(
            "tipo WKB esteso non rappresentabile in Shapefile senza normalizzazione",
        )),
    }
}

fn point_m(coordinate: WkbCoordinate) -> Result<PointM> {
    let measure = coordinate
        .m
        .ok_or_else(|| err("coordinata XYM senza ordinata M"))?;
    Ok(PointM::new(coordinate.x, coordinate.y, measure))
}

fn point_z(coordinate: WkbCoordinate, require_measure: bool) -> Result<PointZ> {
    let z = coordinate
        .z
        .ok_or_else(|| err("coordinata XYZ senza ordinata Z"))?;
    let measure = if require_measure {
        coordinate
            .m
            .ok_or_else(|| err("coordinata XYZM senza ordinata M"))?
    } else {
        NO_DATA
    };
    Ok(PointZ::new(coordinate.x, coordinate.y, z, measure))
}

fn convert_parts<T, F>(parts: Vec<Vec<WkbCoordinate>>, convert: F) -> Result<Vec<Vec<T>>>
where
    F: Fn(WkbCoordinate) -> Result<T> + Copy,
{
    parts
        .into_iter()
        .map(|part| part.into_iter().map(convert).collect())
        .collect()
}

fn convert_rings<T, F>(
    rings: Vec<(bool, Vec<WkbCoordinate>)>,
    convert: F,
) -> Result<Vec<PolygonRing<T>>>
where
    F: Fn(WkbCoordinate) -> Result<T> + Copy,
{
    rings
        .into_iter()
        .map(|(outer, ring)| {
            let points = ring.into_iter().map(convert).collect::<Result<Vec<_>>>()?;
            Ok(if outer {
                PolygonRing::Outer(points)
            } else {
                PolygonRing::Inner(points)
            })
        })
        .collect()
}

fn shape_from_wkb(geometry: WkbGeometry) -> Result<Shape> {
    let dimensions = geometry.dimensions;
    let topology = topology_from_wkb(geometry)?;
    match (dimensions, topology) {
        (CoordinateDimensions::Xy, ShpTopology::Point(c)) => Ok(Shape::Point(Point::new(c.x, c.y))),
        (CoordinateDimensions::Xym, ShpTopology::Point(c)) => Ok(Shape::PointM(point_m(c)?)),
        (CoordinateDimensions::Xyz, ShpTopology::Point(c)) => Ok(Shape::PointZ(point_z(c, false)?)),
        (CoordinateDimensions::Xyzm, ShpTopology::Point(c)) => Ok(Shape::PointZ(point_z(c, true)?)),
        (CoordinateDimensions::Xy, ShpTopology::Multipoint(coordinates)) => {
            Ok(Shape::Multipoint(Multipoint::new(
                coordinates
                    .into_iter()
                    .map(|c| Point::new(c.x, c.y))
                    .collect(),
            )))
        }
        (CoordinateDimensions::Xym, ShpTopology::Multipoint(coordinates)) => {
            let points = coordinates
                .into_iter()
                .map(point_m)
                .collect::<Result<Vec<_>>>()?;
            Ok(Shape::MultipointM(MultipointM::new(points)))
        }
        (CoordinateDimensions::Xyz, ShpTopology::Multipoint(coordinates)) => {
            let points = coordinates
                .into_iter()
                .map(|coordinate| point_z(coordinate, false))
                .collect::<Result<Vec<_>>>()?;
            Ok(Shape::MultipointZ(MultipointZ::new(points)))
        }
        (CoordinateDimensions::Xyzm, ShpTopology::Multipoint(coordinates)) => {
            let points = coordinates
                .into_iter()
                .map(|coordinate| point_z(coordinate, true))
                .collect::<Result<Vec<_>>>()?;
            Ok(Shape::MultipointZ(MultipointZ::new(points)))
        }
        (CoordinateDimensions::Xy, ShpTopology::Polyline(parts)) => {
            Ok(Shape::Polyline(Polyline::with_parts(
                parts
                    .into_iter()
                    .map(|part| part.into_iter().map(|c| Point::new(c.x, c.y)).collect())
                    .collect(),
            )))
        }
        (CoordinateDimensions::Xym, ShpTopology::Polyline(parts)) => Ok(Shape::PolylineM(
            PolylineM::with_parts(convert_parts(parts, point_m)?),
        )),
        (CoordinateDimensions::Xyz, ShpTopology::Polyline(parts)) => Ok(Shape::PolylineZ(
            PolylineZ::with_parts(convert_parts(parts, |coordinate| {
                point_z(coordinate, false)
            })?),
        )),
        (CoordinateDimensions::Xyzm, ShpTopology::Polyline(parts)) => Ok(Shape::PolylineZ(
            PolylineZ::with_parts(convert_parts(parts, |coordinate| {
                point_z(coordinate, true)
            })?),
        )),
        (CoordinateDimensions::Xy, ShpTopology::Polygon(rings)) => {
            Ok(Shape::Polygon(Polygon::with_rings(
                rings
                    .into_iter()
                    .map(|(outer, ring)| {
                        let points = ring.into_iter().map(|c| Point::new(c.x, c.y)).collect();
                        if outer {
                            PolygonRing::Outer(points)
                        } else {
                            PolygonRing::Inner(points)
                        }
                    })
                    .collect(),
            )))
        }
        (CoordinateDimensions::Xym, ShpTopology::Polygon(rings)) => Ok(Shape::PolygonM(
            PolygonM::with_rings(convert_rings(rings, point_m)?),
        )),
        (CoordinateDimensions::Xyz, ShpTopology::Polygon(rings)) => Ok(Shape::PolygonZ(
            PolygonZ::with_rings(convert_rings(rings, |coordinate| {
                point_z(coordinate, false)
            })?),
        )),
        (CoordinateDimensions::Xyzm, ShpTopology::Polygon(rings)) => Ok(Shape::PolygonZ(
            PolygonZ::with_rings(convert_rings(rings, |coordinate| {
                point_z(coordinate, true)
            })?),
        )),
        (CoordinateDimensions::Unknown, _) => {
            Err(err("dimensionalità WKB ignota non scrivibile in Shapefile"))
        }
    }
}

const fn shape_tag(s: &Shape) -> &'static str {
    match s {
        Shape::Point(_) => "point-xy",
        Shape::PointM(_) => "point-m",
        Shape::PointZ(_) => "point-z",
        Shape::Polyline(_) => "polyline-xy",
        Shape::PolylineM(_) => "polyline-m",
        Shape::PolylineZ(_) => "polyline-z",
        Shape::Polygon(_) => "polygon-xy",
        Shape::PolygonM(_) => "polygon-m",
        Shape::PolygonZ(_) => "polygon-z",
        Shape::Multipoint(_) => "multipoint-xy",
        Shape::MultipointM(_) => "multipoint-m",
        Shape::MultipointZ(_) => "multipoint-z",
        Shape::NullShape => "",
        Shape::Multipatch(_) => "unsupported",
    }
}

/// Scrive la shape come tipo ESRI concreto (l'enum `Shape` non è `EsriShape`).
fn write_shape(w: &mut Writer<BufWriter<File>>, shape: Shape, rec: &Record) -> Result<()> {
    let me = |e| err(format!("scrittura record shapefile: {e}"));
    match shape {
        Shape::Point(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PointM(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PointZ(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::Polyline(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PolylineM(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PolylineZ(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::Polygon(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PolygonM(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::PolygonZ(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::Multipoint(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::MultipointM(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::MultipointZ(s) => w.write_shape_and_record(&s, rec).map_err(me),
        Shape::NullShape => Err(err("geometria nulla non supportata in scrittura Shapefile")),
        Shape::Multipatch(_) => Err(err("Multipatch non supportato in scrittura Shapefile")),
    }
}

fn cell_to_field(array: &ArrayRef, row: usize, kind: DbfKind) -> Result<FieldValue> {
    let v = json_from_array(array, row)?;
    Ok(match kind {
        DbfKind::Char => FieldValue::Character(match v {
            JsonValue::Null => None,
            JsonValue::String(s) => Some(s),
            other => Some(other.to_string()),
        }),
        DbfKind::Int | DbfKind::Float => FieldValue::Numeric(v.as_f64()),
        DbfKind::Logical => FieldValue::Logical(v.as_bool()),
    })
}

fn wkt_for_id(id: Option<&str>) -> Option<String> {
    match id {
        Some("EPSG:4326" | "OGC:CRS84") => Some(WGS84_WKT.to_owned()),
        _ => None,
    }
}

fn resolve_prj(
    layer: &plenora_io_core::WriteLayer,
    schema: &Schema,
    geom_idx: usize,
) -> Option<String> {
    if let Some(g) = &layer.contract.geometry {
        if let Some(def) = g.crs.definition() {
            return Some(def.to_owned());
        }
        if let Some(wkt) = wkt_for_id(g.crs.id()) {
            return Some(wkt);
        }
    }
    let id = schema
        .field(geom_idx)
        .metadata()
        .get(GEO_CRS_KEY)
        .map(String::as_str);
    wkt_for_id(id)
}

// --- lettura: helpers ------------------------------------------------------

fn resolve_crs(path: &Path, opts: &ReadOptions) -> Result<ResolvedCrs> {
    let prj = path.with_extension("prj");
    if let Ok(wkt) = std::fs::read_to_string(&prj) {
        let id = opts
            .assume_crs
            .clone()
            .or_else(|| authority_id_from_wkt(&wkt));
        let Some(id) = id else {
            let raw = RawCrs::new(wkt, None);
            return Err(PlenoraIoError::crs_unresolved("shp", &raw));
        };
        let kind = crs_kind(&id, Some(&wkt));
        return Ok(ResolvedCrs::new(Some(id), kind, Some(wkt)));
    }
    opts.assume_crs.as_ref().map_or_else(
        || {
            Err(PlenoraIoError::Crs(
                "Shapefile senza .prj: fornire --assume-crs".to_owned(),
            ))
        },
        |id| Ok(ResolvedCrs::new(Some(id.clone()), crs_kind(id, None), None)),
    )
}

fn resolved_crs_id(crs: &ResolvedCrs) -> Result<&str> {
    crs.id.as_deref().ok_or_else(|| {
        PlenoraIoError::Crs(
            "Shapefile: CRS risolto senza identificatore; vietato inventare un'etichetta Arrow"
                .to_owned(),
        )
    })
}

fn authority_id_from_wkt(wkt: &str) -> Option<String> {
    let upper = wkt.to_ascii_uppercase();
    if upper.trim() == "OGC:CRS84" {
        return Some("OGC:CRS84".to_owned());
    }
    // Il writer Shapefile emette questa forma ESRI WKT1 canonica, che non
    // contiene AUTHORITY ma identifica senza ambiguità WGS 84.
    if upper.contains("GEOGCS[\"WGS 84\"") && upper.contains("DATUM[\"WGS_1984\"") {
        return Some("EPSG:4326".to_owned());
    }
    let epsg = upper.rfind("\"EPSG\"")?;
    let tail = &upper[epsg + "\"EPSG\"".len()..];
    let start = tail.find(char::is_numeric)?;
    let code: String = tail[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (!code.is_empty()).then(|| format!("EPSG:{code}"))
}

fn crs_kind(id: &str, definition: Option<&str>) -> CrsKind {
    let definition = definition.unwrap_or_default().to_ascii_uppercase();
    if definition.contains("PROJCS[")
        || definition.contains("PROJCRS[")
        || id.eq_ignore_ascii_case("EPSG:3857")
    {
        CrsKind::Projected
    } else if id.eq_ignore_ascii_case("OGC:CRS84")
        || id.eq_ignore_ascii_case("EPSG:4326")
        || definition.contains("GEOGCS[")
        || definition.contains("GEOGCRS[")
    {
        CrsKind::Geographic
    } else {
        CrsKind::Unknown
    }
}

fn dbf_numeric_integer_precision_unverifiable(value: &FieldValue) -> bool {
    matches!(
        value,
        FieldValue::Numeric(Some(number))
            if number.is_finite()
                && number.fract() == 0.0
                && number.abs() >= FIRST_F64_INTEGER_WITHOUT_UNIT_PRECISION
    )
}

/// Classe dbf per l'inferenza (Numeric/Double/Float=numero, Integer=int).
const fn classify(v: &FieldValue) -> ObservedValueClass {
    match v {
        FieldValue::Integer(_) => ObservedValueClass::Integer,
        FieldValue::Numeric(Some(_)) | FieldValue::Double(_) | FieldValue::Float(Some(_)) => {
            ObservedValueClass::Number
        }
        FieldValue::Logical(Some(_)) => ObservedValueClass::Boolean,
        FieldValue::Character(Some(_)) | FieldValue::Date(Some(_)) => ObservedValueClass::Text,
        _ => ObservedValueClass::Null,
    }
}

#[derive(Clone, Debug)]
struct DbfFieldLayout {
    name: String,
    field_type: u8,
    offset: usize,
    width: usize,
    exact_integer_slot: Option<usize>,
}

#[derive(Clone, Debug)]
struct DbfLayout {
    header_length: usize,
    record_length: usize,
    record_count: u32,
    fields: Vec<DbfFieldLayout>,
    exact_integer_count: usize,
}

/// Legge la parte strutturale del DBF che `dbase::Record` non espone.
///
/// Due proprieta' dipendono dai descrittori originali: i nomi duplicati devono
/// essere respinti prima che `Record` li comprima in una `HashMap`, e un campo
/// Numeric largo, senza decimali, deve essere letto dal testo ASCII originale
/// anziche' dal `f64` gia' arrotondato dalla dipendenza.
fn read_dbf_layout(shp_path: &Path) -> Result<DbfLayout> {
    let path = shp_path.with_extension("dbf");
    let decoded_names = shapefile::dbase::Reader::from_path(&path)
        .map_err(|error| err(format!("apertura schema DBF: {error}")))?
        .fields()
        .iter()
        .map(|field| field.name().to_owned())
        .collect::<Vec<_>>();
    let mut reader =
        BufReader::new(File::open(&path).map_err(|error| err(format!("apertura DBF: {error}")))?);
    let mut header = [0_u8; DBF_HEADER_SIZE];
    reader
        .read_exact(&mut header)
        .map_err(|error| err(format!("header DBF incompleto: {error}")))?;
    let record_count = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let header_length = usize::from(u16::from_le_bytes([header[8], header[9]]));
    let declared_record_length = usize::from(u16::from_le_bytes([header[10], header[11]]));
    let descriptor_end = if header[0] == DBF_VISUAL_FOXPRO_VERSION {
        header_length
            .checked_sub(DBF_VISUAL_FOXPRO_BACKLINK_SIZE)
            .ok_or_else(|| err("header Visual FoxPro piu' corto del backlink"))?
    } else {
        header_length
    };
    let descriptor_bytes = descriptor_end
        .checked_sub(DBF_HEADER_SIZE + DBF_HEADER_TERMINATOR_SIZE)
        .ok_or_else(|| err("lunghezza header DBF non valida"))?;
    if descriptor_bytes % DBF_FIELD_DESCRIPTOR_SIZE != 0 {
        return Err(err("lunghezza descrittori DBF non valida"));
    }

    let field_count = descriptor_bytes / DBF_FIELD_DESCRIPTOR_SIZE;
    if decoded_names.len() != field_count {
        return Err(err(format!(
            "numero descrittori DBF incoerente: header={field_count}, decoder={}",
            decoded_names.len()
        )));
    }
    let mut fields = Vec::with_capacity(field_count);
    let mut seen = BTreeSet::new();
    let mut offset = 1_usize; // deletion flag
    let mut exact_integer_count = 0_usize;
    for (index, decoded_name) in decoded_names.into_iter().enumerate() {
        let mut descriptor = [0_u8; DBF_FIELD_DESCRIPTOR_SIZE];
        reader
            .read_exact(&mut descriptor)
            .map_err(|error| err(format!("descrittore campo DBF incompleto: {error}")))?;
        let name = decoded_name;
        if name.is_empty() {
            return Err(err(format!("nome campo DBF vuoto all'indice {index}")));
        }
        let normalized = name.to_ascii_uppercase();
        if !seen.insert(normalized) {
            return Err(err(format!(
                "nomi campo DBF duplicati: '{name}'; il file e' rifiutato per non perdere una colonna"
            )));
        }
        let width = usize::from(descriptor[16]);
        if width == 0 {
            return Err(err(format!("campo DBF '{name}' con larghezza zero")));
        }
        let exact_integer_slot = (descriptor[11] == b'N' && descriptor[17] == 0 && width >= 10)
            .then(|| {
                let slot = exact_integer_count;
                exact_integer_count += 1;
                slot
            });
        fields.push(DbfFieldLayout {
            name,
            field_type: descriptor[11],
            offset,
            width,
            exact_integer_slot,
        });
        offset = offset
            .checked_add(width)
            .ok_or_else(|| err("overflow nella lunghezza record DBF"))?;
    }
    let mut terminator = [0_u8; 1];
    reader
        .read_exact(&mut terminator)
        .map_err(|error| err(format!("terminatore header DBF mancante: {error}")))?;
    if terminator[0] != 0x0d {
        return Err(err("terminatore header DBF non valido"));
    }
    if declared_record_length != offset && declared_record_length.checked_add(1) != Some(offset) {
        return Err(err(format!(
            "record DBF dichiarato lungo {declared_record_length} byte ma i campi ne richiedono {offset}"
        )));
    }
    Ok(DbfLayout {
        header_length,
        // `dbase` adotta la lunghezza calcolata quando un produttore omette il
        // deletion flag dalla lunghezza dichiarata; il lettore raw deve restare
        // allineato allo stesso comportamento.
        record_length: offset,
        record_count,
        fields,
        exact_integer_count,
    })
}

struct DbfExactIntegerRows {
    reader: BufReader<File>,
    layout: DbfLayout,
    records_read: u32,
    buffer: Vec<u8>,
}

enum DbfPhysicalRow {
    Deleted,
    Active {
        exact_values: Vec<Option<i64>>,
        raw_numeric_key: Option<String>,
        rejection_cause: Option<&'static str>,
    },
}

impl DbfExactIntegerRows {
    fn open(shp_path: &Path, layout: &DbfLayout) -> Result<Self> {
        let mut reader = BufReader::new(
            File::open(shp_path.with_extension("dbf"))
                .map_err(|error| err(format!("apertura DBF: {error}")))?,
        );
        reader
            .seek(SeekFrom::Start(layout.header_length as u64))
            .map_err(|error| err(format!("posizionamento sui record DBF: {error}")))?;
        Ok(Self {
            reader,
            layout: layout.clone(),
            records_read: 0,
            buffer: vec![0_u8; layout.record_length],
        })
    }

    fn next_physical(
        &mut self,
        raw_numeric_field_index: Option<usize>,
    ) -> Result<Option<DbfPhysicalRow>> {
        if self.records_read >= self.layout.record_count {
            return Ok(None);
        }
        self.reader
            .read_exact(&mut self.buffer)
            .map_err(|error| err(format!("record DBF incompleto: {error}")))?;
        self.records_read += 1;
        match self.buffer[0] {
            b'*' => return Ok(Some(DbfPhysicalRow::Deleted)),
            b' ' => {}
            marker => {
                return Err(err(format!(
                    "marcatore record DBF non valido: 0x{marker:02x}"
                )))
            }
        }
        let mut values = vec![None; self.layout.exact_integer_count];
        let mut rejection_cause = None;
        for field in &self.layout.fields {
            let Some(slot) = field.exact_integer_slot else {
                continue;
            };
            let end = field
                .offset
                .checked_add(field.width)
                .ok_or_else(|| err("overflow nell'offset del campo DBF"))?;
            let raw = self
                .buffer
                .get(field.offset..end)
                .ok_or_else(|| err(format!("campo DBF '{}' fuori record", field.name)))?;
            let Ok(text) = std::str::from_utf8(raw) else {
                rejection_cause = Some(ATTRIBUTE_NUMERIC_INVALID_CAUSE);
                continue;
            };
            let text = text.trim();
            if !text.is_empty() {
                match text.parse::<i64>() {
                    Ok(value) => values[slot] = Some(value),
                    Err(_) => rejection_cause = Some(ATTRIBUTE_NUMERIC_INVALID_CAUSE),
                }
            }
        }
        let raw_numeric_key = if let Some(index) = raw_numeric_field_index {
            let field = self
                .layout
                .fields
                .get(index)
                .ok_or_else(|| err("indice campo chiave DBF fuori schema"))?;
            let end = field
                .offset
                .checked_add(field.width)
                .ok_or_else(|| err("overflow nell'offset della chiave DBF"))?;
            let raw = self
                .buffer
                .get(field.offset..end)
                .ok_or_else(|| err("chiave DBF fuori record"))?;
            std::str::from_utf8(raw).map_or_else(
                |_| {
                    rejection_cause = Some(ATTRIBUTE_NUMERIC_INVALID_CAUSE);
                    None
                },
                |text| {
                    let text = text.trim();
                    (!text.is_empty()).then(|| text.to_owned())
                },
            )
        } else {
            None
        };
        Ok(Some(DbfPhysicalRow::Active {
            exact_values: values,
            raw_numeric_key,
            rejection_cause,
        }))
    }
}

#[derive(Clone)]
struct ShpColumn {
    name: String,
    column_type: ColType,
    exact_integer_slot: Option<usize>,
}

struct ShpGeometryInfo {
    dimensions: CoordinateDimensions,
    geometry_types: Vec<GeometryType>,
    shape_type: Option<&'static str>,
}

struct ShpInference {
    cols: Vec<ShpColumn>,
    dbf_layout: DbfLayout,
    geometry_info: ShpGeometryInfo,
    active_row_count: u64,
    loss: LossReport,
}

trait NativePoint {
    fn x(&self) -> f64;
    fn y(&self) -> f64;
    fn z(&self) -> Option<f64>;
    fn m(&self) -> Option<f64>;
}

impl NativePoint for Point {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn z(&self) -> Option<f64> {
        None
    }
    fn m(&self) -> Option<f64> {
        None
    }
}

impl NativePoint for PointM {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn z(&self) -> Option<f64> {
        None
    }
    fn m(&self) -> Option<f64> {
        Some(self.m)
    }
}

impl NativePoint for PointZ {
    fn x(&self) -> f64 {
        self.x
    }
    fn y(&self) -> f64 {
        self.y
    }
    fn z(&self) -> Option<f64> {
        Some(self.z)
    }
    fn m(&self) -> Option<f64> {
        Some(self.m)
    }
}

fn native_coordinate<P: NativePoint>(
    point: &P,
    dimensions: CoordinateDimensions,
) -> Result<WkbCoordinate> {
    let (z, m) = match dimensions {
        CoordinateDimensions::Xy if point.z().is_none() && point.m().is_none() => (None, None),
        CoordinateDimensions::Xym if point.z().is_none() => (
            None,
            Some(
                point
                    .m()
                    .ok_or_else(|| err("coordinata ShapeM senza misura"))?,
            ),
        ),
        CoordinateDimensions::Xyz => {
            let z = point
                .z()
                .ok_or_else(|| err("coordinata ShapeZ senza quota"))?;
            if point.m().is_some_and(|measure| {
                !matches!(
                    measure.partial_cmp(&NO_DATA),
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                )
            }) {
                return Err(err(
                    "misura valida trovata in un dataset ShapeZ dichiarato XYZ",
                ));
            }
            (Some(z), None)
        }
        CoordinateDimensions::Xyzm => (
            Some(
                point
                    .z()
                    .ok_or_else(|| err("coordinata ShapeZ senza quota"))?,
            ),
            Some(
                point
                    .m()
                    .ok_or_else(|| err("coordinata ShapeZ senza misura nativa"))?,
            ),
        ),
        CoordinateDimensions::Unknown => {
            return Err(err("dimensionalità Shapefile non determinata"))
        }
        _ => {
            return Err(err(
                "variante Shape incoerente con la dimensionalità del layer",
            ))
        }
    };
    Ok(WkbCoordinate {
        x: point.x(),
        y: point.y(),
        z,
        m,
    })
}

fn native_coordinates<P: NativePoint>(
    points: &[P],
    dimensions: CoordinateDimensions,
) -> Result<Vec<WkbCoordinate>> {
    points
        .iter()
        .map(|point| native_coordinate(point, dimensions))
        .collect()
}

fn polyline_wkb<P: NativePoint>(
    parts: &[Vec<P>],
    dimensions: CoordinateDimensions,
) -> Result<WkbGeometry> {
    let children = parts
        .iter()
        .map(|part| {
            Ok(WkbGeometry {
                value: WkbValue::LineString(native_coordinates(part, dimensions)?),
                dimensions,
                srid: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(WkbGeometry {
        value: WkbValue::MultiLineString(children),
        dimensions,
        srid: None,
    })
}

fn polygon_wkb<P: NativePoint>(
    rings: &[PolygonRing<P>],
    dimensions: CoordinateDimensions,
) -> Result<WkbGeometry> {
    let mut polygons = Vec::<WkbGeometry>::new();
    let mut current = None::<Vec<Vec<WkbCoordinate>>>;
    for ring in rings {
        match ring {
            PolygonRing::Outer(points) => {
                if let Some(rings) = current.take() {
                    polygons.push(WkbGeometry {
                        value: WkbValue::Polygon(rings),
                        dimensions,
                        srid: None,
                    });
                }
                current = Some(vec![native_coordinates(points, dimensions)?]);
            }
            PolygonRing::Inner(points) => {
                let current = current
                    .as_mut()
                    .ok_or_else(|| err("anello interno Shapefile senza anello esterno"))?;
                current.push(native_coordinates(points, dimensions)?);
            }
        }
    }
    if let Some(rings) = current {
        polygons.push(WkbGeometry {
            value: WkbValue::Polygon(rings),
            dimensions,
            srid: None,
        });
    }
    if polygons.is_empty() {
        return Err(err("Polygon Shapefile senza anelli esterni"));
    }
    Ok(WkbGeometry {
        value: WkbValue::MultiPolygon(polygons),
        dimensions,
        srid: None,
    })
}

// Un anello Shapefile e' chiuso se e solo se primo e ultimo vertice coincidono
// bit a bit: il confronto esatto e' la definizione del formato, una tolleranza
// accetterebbe come chiusi anelli che GDAL e il corpus reale considerano aperti.
#[allow(clippy::float_cmp)]
fn polygon_rejection_cause<P: NativePoint>(rings: &[PolygonRing<P>]) -> Option<&'static str> {
    if rings.is_empty() {
        return Some(POLYGON_WITHOUT_OUTER_CAUSE);
    }
    if rings.iter().any(|ring| {
        let points = ring.points();
        !matches!(
            (points.first(), points.last()),
            (Some(first), Some(last)) if first.x() == last.x() && first.y() == last.y()
        )
    }) {
        return Some(UNCLOSED_RING_CAUSE);
    }
    if rings.iter().any(|ring| {
        let points = ring.points();
        if points.len() < 4 {
            return true;
        }
        // Niente mul_add/FMA: la fusione cambia l'arrotondamento IEEE e
        // romperebbe il determinismo bit-esatto della somma dell'area doppia.
        #[allow(clippy::suboptimal_flops)]
        let twice_area = points.windows(2).fold(0.0, |area, edge| {
            area + edge[0].x() * edge[1].y() - edge[1].x() * edge[0].y()
        });
        !twice_area.is_finite() || twice_area == 0.0
    }) {
        return Some(DEGENERATE_RING_CAUSE);
    }
    let mut has_outer = false;
    for ring in rings {
        match ring {
            PolygonRing::Outer(_) => has_outer = true,
            PolygonRing::Inner(_) if !has_outer => return Some(INNER_RING_WITHOUT_OUTER_CAUSE),
            PolygonRing::Inner(_) => {}
        }
    }
    if !has_outer {
        return Some(POLYGON_WITHOUT_OUTER_CAUSE);
    }
    None
}

fn shape_rejection_cause(shape: &Shape) -> Option<&'static str> {
    match shape {
        Shape::Polygon(polygon) => polygon_rejection_cause(polygon.rings()),
        Shape::PolygonM(polygon) => polygon_rejection_cause(polygon.rings()),
        Shape::PolygonZ(polygon) => polygon_rejection_cause(polygon.rings()),
        _ => None,
    }
}

fn multipoint_wkb<P: NativePoint>(
    points: &[P],
    dimensions: CoordinateDimensions,
) -> Result<WkbGeometry> {
    let children = points
        .iter()
        .map(|point| {
            Ok(WkbGeometry {
                value: WkbValue::Point(native_coordinate(point, dimensions)?),
                dimensions,
                srid: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(WkbGeometry {
        value: WkbValue::MultiPoint(children),
        dimensions,
        srid: None,
    })
}

fn shape_to_wkb(shape: &Shape, dimensions: CoordinateDimensions) -> Result<Option<WkbGeometry>> {
    let geometry = match shape {
        Shape::NullShape => return Ok(None),
        Shape::Point(point) => WkbGeometry {
            value: WkbValue::Point(native_coordinate(point, dimensions)?),
            dimensions,
            srid: None,
        },
        Shape::PointM(point) => WkbGeometry {
            value: WkbValue::Point(native_coordinate(point, dimensions)?),
            dimensions,
            srid: None,
        },
        Shape::PointZ(point) => WkbGeometry {
            value: WkbValue::Point(native_coordinate(point, dimensions)?),
            dimensions,
            srid: None,
        },
        Shape::Polyline(polyline) => polyline_wkb(polyline.parts(), dimensions)?,
        Shape::PolylineM(polyline) => polyline_wkb(polyline.parts(), dimensions)?,
        Shape::PolylineZ(polyline) => polyline_wkb(polyline.parts(), dimensions)?,
        Shape::Polygon(polygon) => polygon_wkb(polygon.rings(), dimensions)?,
        Shape::PolygonM(polygon) => polygon_wkb(polygon.rings(), dimensions)?,
        Shape::PolygonZ(polygon) => polygon_wkb(polygon.rings(), dimensions)?,
        Shape::Multipoint(multipoint) => multipoint_wkb(multipoint.points(), dimensions)?,
        Shape::MultipointM(multipoint) => multipoint_wkb(multipoint.points(), dimensions)?,
        Shape::MultipointZ(multipoint) => multipoint_wkb(multipoint.points(), dimensions)?,
        Shape::Multipatch(_) => {
            return Err(err(
                "Multipatch non ha una conversione WKB univoca ed è rifiutato",
            ))
        }
    };
    Ok(Some(geometry))
}

fn shape_has_valid_measure(shape: &Shape) -> bool {
    let valid = |measure: f64| {
        !matches!(
            measure.partial_cmp(&NO_DATA),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        )
    };
    match shape {
        Shape::PointZ(point) => valid(point.m),
        Shape::PolylineZ(polyline) => polyline
            .parts()
            .iter()
            .flatten()
            .any(|point| valid(point.m)),
        Shape::PolygonZ(polygon) => polygon
            .rings()
            .iter()
            .flat_map(PolygonRing::points)
            .any(|point| valid(point.m)),
        Shape::MultipointZ(multipoint) => multipoint.points().iter().any(|point| valid(point.m)),
        _ => false,
    }
}

fn dimensions_for_shape_tag(shape_type: Option<&str>, z_has_measure: bool) -> CoordinateDimensions {
    match shape_type {
        Some(tag) if tag.ends_with("-xy") => CoordinateDimensions::Xy,
        Some(tag) if tag.ends_with("-m") => CoordinateDimensions::Xym,
        Some(tag) if tag.ends_with("-z") && z_has_measure => CoordinateDimensions::Xyzm,
        Some(tag) if tag.ends_with("-z") => CoordinateDimensions::Xyz,
        _ => CoordinateDimensions::Unknown,
    }
}

fn header_geometry(shape_type: ShapeType) -> Result<(Option<&'static str>, Vec<GeometryType>)> {
    let value = match shape_type {
        ShapeType::NullShape => (None, Vec::new()),
        ShapeType::Point => (Some("point-xy"), vec![GeometryType::Point]),
        ShapeType::PointM => (Some("point-m"), vec![GeometryType::Point]),
        ShapeType::PointZ => (Some("point-z"), vec![GeometryType::Point]),
        ShapeType::Polyline => (Some("polyline-xy"), vec![GeometryType::MultiLineString]),
        ShapeType::PolylineM => (Some("polyline-m"), vec![GeometryType::MultiLineString]),
        ShapeType::PolylineZ => (Some("polyline-z"), vec![GeometryType::MultiLineString]),
        ShapeType::Polygon => (Some("polygon-xy"), vec![GeometryType::MultiPolygon]),
        ShapeType::PolygonM => (Some("polygon-m"), vec![GeometryType::MultiPolygon]),
        ShapeType::PolygonZ => (Some("polygon-z"), vec![GeometryType::MultiPolygon]),
        ShapeType::Multipoint => (Some("multipoint-xy"), vec![GeometryType::MultiPoint]),
        ShapeType::MultipointM => (Some("multipoint-m"), vec![GeometryType::MultiPoint]),
        ShapeType::MultipointZ => (Some("multipoint-z"), vec![GeometryType::MultiPoint]),
        ShapeType::Multipatch => return Err(err("Multipatch Shapefile non supportato")),
    };
    Ok(value)
}

fn shape_type_label(shape_type: Option<&str>) -> &str {
    shape_type.unwrap_or("null")
}

/// Il tipo geometrico e' una proprieta' dell'header Shapefile. Per i tipi Z
/// soltanto, M e' opzionale record per record e richiede una scansione; i
/// comuni percorsi XY/M non devono decodificare tutte le geometrie durante
/// l'apertura per poi decodificarle di nuovo durante la lettura.
fn infer_geometry_info(path: &Path, dbf_record_count: u32) -> Result<ShpGeometryInfo> {
    let mut reader = ShapeReader::from_path(path)
        .map_err(|error| err(format!("apertura geometrie Shapefile: {error}")))?;
    let native_type = reader.header().shape_type;
    if let Ok(shape_count) = reader.shape_count() {
        if shape_count != dbf_record_count as usize {
            return Err(err(format!(
                "numero di geometrie ({shape_count}) diverso dai record DBF ({dbf_record_count})"
            )));
        }
    }
    let (shape_type, geometry_types) = header_geometry(native_type)?;
    let mut z_has_measure = false;
    if native_type.has_z() {
        for shape in reader.iter_shapes() {
            let shape =
                shape.map_err(|error| err(format!("record geometrico Shapefile: {error}")))?;
            let tag = shape_tag(&shape);
            if !tag.is_empty() && Some(tag) != shape_type {
                return Err(err(format!(
                    "tipo Shape nel record '{tag}' incoerente con l'header '{}'",
                    shape_type_label(shape_type)
                )));
            }
            z_has_measure |= shape_has_valid_measure(&shape);
        }
    }
    Ok(ShpGeometryInfo {
        dimensions: dimensions_for_shape_tag(shape_type, z_has_measure),
        geometry_types,
        shape_type,
    })
}

/// Pass 1: nomi campo, tipo DBF e contratto geometrico nativo, a RAM O(ncol).
// Passata unica sul DBF: layout, accumulatori di tipo, righe cancellate e
// rischio di precisione condividono lo stesso scorrimento dei record. Spezzarla
// significherebbe rileggere il file e perdere la garanzia O(ncol).
#[allow(clippy::too_many_lines)]
fn infer_shp_schema(path: &Path) -> Result<ShpInference> {
    let dbf_layout = read_dbf_layout(path)?;
    let mut exact_rows = DbfExactIntegerRows::open(path, &dbf_layout)?;
    let geometry_info = infer_geometry_info(path, dbf_layout.record_count)?;
    let mut reader = shapefile::dbase::Reader::from_path(path.with_extension("dbf"))
        .map_err(|error| err(format!("apertura DBF: {error}")))?;
    let mut accs: HashMap<String, TypeAccumulator> = dbf_layout
        .fields
        .iter()
        .map(|field| {
            let mut accumulator = TypeAccumulator::default();
            if field.exact_integer_slot.is_some() {
                // Il tipo e' dichiarato dal descrittore N(width>=10, decimals=0),
                // anche quando tutti i valori sono nulli.
                accumulator.observe(ObservedValueClass::Integer);
            } else {
                accumulator.observe(match field.field_type {
                    b'N' | b'F' => ObservedValueClass::Number,
                    b'L' => ObservedValueClass::Boolean,
                    _ => ObservedValueClass::Text,
                });
            }
            (field.name.clone(), accumulator)
        })
        .collect();
    let mut loss = LossReport::default();
    let mut precision_risk_fields = BTreeSet::new();
    let mut active_row_count = 0_u64;
    let mut records = reader.iter_records();
    while let Some(physical_row) = exact_rows.next_physical(None)? {
        let exact_values = match physical_row {
            DbfPhysicalRow::Deleted => continue,
            DbfPhysicalRow::Active { exact_values, .. } => exact_values,
        };
        active_row_count = active_row_count
            .checked_add(1)
            .ok_or_else(|| err("numero di record DBF fuori intervallo u64"))?;
        let record = match records.next() {
            Some(Ok(record)) => record,
            Some(Err(_)) => continue,
            None => return Err(err("numero di record DBF incoerente con l'header")),
        };
        for field in &dbf_layout.fields {
            let accumulator = accs.get_mut(&field.name).ok_or_else(|| {
                err(format!(
                    "schema DBF senza accumulatore per '{}'",
                    field.name
                ))
            })?;
            if let Some(slot) = field.exact_integer_slot {
                accumulator
                    .observe(exact_values[slot].map_or(ObservedValueClass::Null, classify_i64));
                continue;
            }
            let value = record.get(&field.name);
            if value.is_some_and(dbf_numeric_integer_precision_unverifiable) {
                loss.record(DBF_NUMERIC_INTEGER_PRECISION_UNVERIFIABLE, 1);
                precision_risk_fields.insert(field.name.clone());
            }
            accumulator.observe(value.map_or(ObservedValueClass::Null, classify));
        }
    }
    if records.next().is_some() {
        return Err(err("numero di record DBF incoerente con l'header"));
    }
    let columns = dbf_layout
        .fields
        .iter()
        .map(|field| field.name.clone())
        .map(|name| {
            let column_type = accs
                .get(&name)
                .ok_or_else(|| err(format!("schema DBF senza accumulatore per '{name}'")))?
                .column_type();
            let exact_integer_slot = dbf_layout
                .fields
                .iter()
                .find(|field| field.name == name)
                .and_then(|field| field.exact_integer_slot);
            Ok(ShpColumn {
                name,
                column_type,
                exact_integer_slot,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for name in precision_risk_fields {
        loss.add_example(LossExample {
            category: DBF_NUMERIC_INTEGER_PRECISION_UNVERIFIABLE.to_owned(),
            context: format!(
                "field={name}: DBF Numeric già decodificato come f64 senza precisione intera unitaria"
            ),
        });
    }
    Ok(ShpInference {
        cols: columns,
        dbf_layout,
        geometry_info,
        active_row_count,
        loss,
    })
}

struct ShpParserInput {
    path: PathBuf,
    schema: SchemaRef,
    cols: Vec<ShpColumn>,
    dbf_layout: DbfLayout,
    dimensions: CoordinateDimensions,
    expected_shape_type: Option<&'static str>,
    expected_active_rows: u64,
    include_geometry: bool,
    batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    layer: LayerContract,
    loss: LossReport,
    row_diagnostics: ShpRowDiagnosticsConfig,
    scope: ReadScope,
    cancellation: plenora_io_model::CancellationToken,
}

/// Pass 2: thread che scorre i record e produce batch da `batch_size` righe.
// Corpo unico del thread di parsing: lo stato del ciclo (reader shp, reader
// dbf, builder, diagnostica, scope) e' condiviso da tutte le fasi e spezzarlo
// richiederebbe di esporre quello stato in strutture ausiliarie.
#[allow(clippy::too_many_lines)]
fn spawn_parser(input: ShpParserInput) -> Result<Box<dyn LayerReader>> {
    let ShpParserInput {
        path,
        schema,
        cols,
        dbf_layout,
        dimensions,
        expected_shape_type,
        expected_active_rows,
        include_geometry,
        mut batch_sizer,
        layer,
        loss,
        row_diagnostics,
        scope,
        cancellation,
    } = input;
    let reader = spawn_batch_reader(DESCRIPTOR.id, layer, 2, move |emitter: BatchEmitter| {
        if scope == ReadScope::AcceptedRows(0) {
            return Ok(());
        }
        let mut shape_reader = shapefile::ShapeReader::from_path(&path)
            .map_err(|error| err(format!("shapefile non valido: {error}")))?;
        let mut dbf_reader = shapefile::dbase::Reader::from_path(path.with_extension("dbf"))
            .map_err(|error| err(format!("apertura DBF: {error}")))?;
        let mut shapes = shape_reader.iter_shapes();
        let mut records = dbf_reader.iter_records();
        let mut exact_rows = DbfExactIntegerRows::open(&path, &dbf_layout)?;
        let mut geom = include_geometry.then(BinaryBuilder::new);
        let mut builders: Vec<InferredColumnBuilder> = cols
            .iter()
            .map(|column| InferredColumnBuilder::new(column.column_type))
            .collect();
        let mut n = 0usize;
        let mut source_rows_seen = 0_u64;
        let mut active_rows_seen = 0_u64;
        let raw_numeric_field_index = row_diagnostics
            .key
            .as_ref()
            .and_then(|key| key.raw_numeric_field_index);
        let mut diagnostics = ShpRowDiagnostics::new(row_diagnostics);
        loop {
            if !diagnostics.is_empty()
                && matches!(scope, ReadScope::AcceptedRows(limit) if active_rows_seen >= limit)
            {
                return Err(diagnostics.into_partial_error(
                    err("limite di righe richiesto raggiunto durante la diagnostica Shapefile"),
                    "read_scope_row_limit_reached",
                ));
            }
            if !diagnostics.is_empty()
                && source_rows_seen.is_multiple_of(1_024)
                && !emitter.is_receiver_alive()
            {
                return Ok(());
            }
            if let Err(error) =
                plenora_io_core::check_cancelled(&cancellation, plenora_io_model::ErrorPhase::Read)
            {
                return Err(diagnostics.into_partial_error(error, "shapefile.scan_cancelled"));
            }
            let physical_row = match exact_rows.next_physical(raw_numeric_field_index) {
                Ok(Some(row)) => row,
                Ok(None) => break,
                Err(error) => {
                    return Err(diagnostics
                        .into_partial_error(error, "shapefile.dbf_exact_scan_interrupted"));
                }
            };
            let source_index = source_rows_seen;
            source_rows_seen = source_rows_seen
                .checked_add(1)
                .ok_or_else(|| err("numero di record Shapefile fuori intervallo u64"))?;
            let shape = match shapes.next() {
                Some(Ok(shape)) => shape,
                Some(Err(error)) => {
                    return Err(diagnostics.into_partial_error(
                        err(format!("record shapefile non valido: {error}")),
                        "shapefile.scan_interrupted",
                    ));
                }
                None => {
                    return Err(diagnostics.into_partial_error(
                        err("numero di geometrie incoerente con i record DBF"),
                        "shapefile.scan_interrupted",
                    ));
                }
            };
            let (exact_values, raw_numeric_key, physical_rejection) = match physical_row {
                DbfPhysicalRow::Deleted => continue,
                DbfPhysicalRow::Active {
                    exact_values,
                    raw_numeric_key,
                    rejection_cause,
                } => (exact_values, raw_numeric_key, rejection_cause),
            };
            active_rows_seen = active_rows_seen
                .checked_add(1)
                .ok_or_else(|| err("numero di record DBF attivi fuori intervallo u64"))?;
            let record = match records.next() {
                Some(Ok(record)) => record,
                Some(Err(_)) => {
                    let cause = physical_rejection
                        .map_or("shapefile.attribute_decode_failed", |cause| cause);
                    diagnostics.record(source_index, cause, None, raw_numeric_key.as_deref());
                    continue;
                }
                None => {
                    return Err(diagnostics.into_partial_error(
                        err("numero di record DBF attivi incoerente con le geometrie"),
                        "shapefile.scan_interrupted",
                    ));
                }
            };
            if let Some(cause) = physical_rejection {
                diagnostics.record(
                    source_index,
                    cause,
                    Some(&record),
                    raw_numeric_key.as_deref(),
                );
                continue;
            }
            let tag = shape_tag(&shape);
            if !tag.is_empty() && Some(tag) != expected_shape_type {
                diagnostics.record(
                    source_index,
                    "shapefile.shape_type_mismatch",
                    Some(&record),
                    raw_numeric_key.as_deref(),
                );
                continue;
            }
            if let Some(cause) = shape_rejection_cause(&shape) {
                diagnostics.record(
                    source_index,
                    cause,
                    Some(&record),
                    raw_numeric_key.as_deref(),
                );
                continue;
            }
            let Ok(converted_geometry) = shape_to_wkb(&shape, dimensions) else {
                diagnostics.record(
                    source_index,
                    "shapefile.geometry_conversion_failed",
                    Some(&record),
                    raw_numeric_key.as_deref(),
                );
                continue;
            };
            let encoded_geometry = match converted_geometry {
                Some(geometry) if include_geometry => {
                    let Ok(bytes) = encode_wkb(&geometry, WkbFlavor::Iso) else {
                        diagnostics.record(
                            source_index,
                            "shapefile.geometry_encoding_failed",
                            Some(&record),
                            raw_numeric_key.as_deref(),
                        );
                        continue;
                    };
                    Some(bytes)
                }
                _ => None,
            };
            if !diagnostics.is_empty() {
                // Dopo il primo rifiuto la scansione continua soltanto per
                // completare conteggi/esempi; nessun altro batch viene emesso.
                continue;
            }
            if let Some(builder) = &mut geom {
                match encoded_geometry {
                    Some(bytes) => builder.append_value(bytes),
                    None => builder.append_null(),
                }
            }
            // Lookup per nome (l'ordine di iterazione del Record non è garantito).
            for (k, column) in cols.iter().enumerate() {
                if let Some(slot) = column.exact_integer_slot {
                    match exact_values[slot] {
                        Some(value) => {
                            if let Err(error) = builders[k].append_i64(value) {
                                diagnostics.record(
                                    source_index,
                                    "shapefile.attribute_conversion_failed",
                                    Some(&record),
                                    raw_numeric_key.as_deref(),
                                );
                                return Err(diagnostics.into_partial_error(
                                    error,
                                    "shapefile.attribute_scan_interrupted",
                                ));
                            }
                        }
                        None => builders[k].append_null(),
                    }
                    continue;
                }
                let value = record
                    .get(&column.name)
                    .filter(|value| classify(value) != ObservedValueClass::Null);
                if let Err(error) =
                    builders[k].append_converted(value, fv_i64, fv_f64, fv_bool, |value| {
                        fv_string(value).map(Cow::Owned)
                    })
                {
                    diagnostics.record(
                        source_index,
                        "shapefile.attribute_conversion_failed",
                        Some(&record),
                        raw_numeric_key.as_deref(),
                    );
                    return Err(diagnostics
                        .into_partial_error(error, "shapefile.attribute_scan_interrupted"));
                }
            }
            n += 1;
            if n >= batch_sizer.rows() {
                let batch = finish_batch(&schema, &mut geom, &mut builders, n)?;
                batch_sizer.observe(&batch);
                if !emitter.send_cancellable(
                    batch,
                    &cancellation,
                    plenora_io_model::ErrorPhase::Read,
                )? {
                    return Ok(());
                }
                n = 0;
            }
        }
        if source_rows_seen != u64::from(dbf_layout.record_count)
            || active_rows_seen != expected_active_rows
            || shapes.next().is_some()
            || records.next().is_some()
        {
            return Err(diagnostics.into_partial_error(
                err("cardinalita' Shapefile cambiata durante la lettura"),
                "shapefile.scan_interrupted",
            ));
        }
        if !diagnostics.is_empty() {
            let rejected = diagnostics.observed_total;
            return Err(err(format!(
                "{rejected} righe Shapefile non valide; consultare row_diagnostics"
            ))
            .with_row_diagnostics(diagnostics.into_report()));
        }
        if n > 0 {
            let batch = finish_batch(&schema, &mut geom, &mut builders, n)?;
            if !emitter.send_cancellable(
                batch,
                &cancellation,
                plenora_io_model::ErrorPhase::Read,
            )? {
                return Ok(());
            }
        }
        Ok(())
    })?;
    Ok(Box::new(ShpLossReader {
        inner: reader,
        loss,
    }))
}

struct ShpLossReader {
    inner: Box<dyn LayerReader>,
    loss: LossReport,
}

impl LayerReader for ShpLossReader {
    fn contract(&self) -> &LayerContract {
        self.inner.contract()
    }

    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        self.inner.next_batch()
    }

    fn loss_report(&self) -> LossReport {
        let mut loss = self.inner.loss_report();
        loss.merge(&self.loss);
        loss
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
    RecordBatch::try_new_with_options(schema.clone(), arrays, &options)
        .map_err(|error| err(format!("batch: {error}")))
}

fn fv_i64(v: &FieldValue) -> Option<i64> {
    match v {
        FieldValue::Integer(i) => Some(i64::from(*i)),
        // I campi DBF Numeric/Double/Float sono decodificati come virgola
        // mobile dal parser dbase: la conversione a intero tronca verso zero e
        // satura, esattamente come prima. Il caso davvero esatto (N con
        // width>=10 e decimals=0) non passa da qui ma dallo slot
        // `exact_integer_slot`, che legge i byte ASCII del record.
        #[allow(clippy::cast_possible_truncation)]
        FieldValue::Numeric(Some(n)) => Some(*n as i64),
        #[allow(clippy::cast_possible_truncation)]
        FieldValue::Double(d) => Some(*d as i64),
        #[allow(clippy::cast_possible_truncation)]
        FieldValue::Float(Some(f)) => Some(*f as i64),
        _ => None,
    }
}

fn fv_f64(v: &FieldValue) -> Option<f64> {
    match v {
        FieldValue::Numeric(Some(n)) => Some(*n),
        FieldValue::Double(d) => Some(*d),
        FieldValue::Float(Some(f)) => Some(f64::from(*f)),
        FieldValue::Integer(i) => Some(f64::from(*i)),
        _ => None,
    }
}

const fn fv_bool(v: &FieldValue) -> Option<bool> {
    match v {
        FieldValue::Logical(Some(b)) => Some(*b),
        _ => None,
    }
}

fn fv_string(v: &FieldValue) -> Option<String> {
    match v {
        FieldValue::Character(Some(s)) => Some(s.clone()),
        FieldValue::Date(Some(d)) => {
            Some(format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()))
        }
        FieldValue::Integer(i) => Some(i.to_string()),
        FieldValue::Numeric(Some(n)) => Some(n.to_string()),
        FieldValue::Double(d) => Some(d.to_string()),
        FieldValue::Float(Some(f)) => Some(f.to_string()),
        FieldValue::Logical(Some(b)) => Some(b.to_string()),
        _ => None,
    }
}

/// Entry point non stabile per libFuzzer: decodifica WKB dimensionale,
/// conversione nella shape ESRI concreta e ritorno a WKB.
#[doc(hidden)]
pub fn __fuzz_wkb_roundtrip(bytes: &[u8]) -> Result<usize> {
    let geometry = decode_wkb(bytes, &WkbLimits::default())?;
    let dimensions = geometry.dimensions;
    let shape = shape_from_wkb(geometry)?;
    let round_trip = shape_to_wkb(&shape, dimensions)?
        .ok_or_else(|| err("la conversione di una geometria ha prodotto NullShape"))?;
    Ok(encode_wkb(&round_trip, WkbFlavor::Iso)?.len())
}

#[cfg(test)]
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

    use std::io::Write as _;

    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;
    use plenora_io_model::wkb::to_wkb;
    use plenora_io_model::CancellationToken;

    const EPSG_3003_WKT: &str = include_str!("../tests/fixtures/epsg3003.prj");

    fn read_opts() -> ReadOptions {
        opzioni_lettura().with_assume_crs("EPSG:4326")
    }

    fn req() -> ReadRequest {
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

    fn make_polygon_ring_unclosed(path: &Path, target_record: usize) {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let mut record_offset = 100_u64;
        for record_index in 0..=target_record {
            file.seek(SeekFrom::Start(record_offset)).unwrap();
            let mut record_header = [0_u8; 8];
            file.read_exact(&mut record_header).unwrap();
            let content_bytes =
                u64::from(u32::from_be_bytes(record_header[4..8].try_into().unwrap())) * 2;
            let body_offset = record_offset + 8;
            if record_index == target_record {
                file.seek(SeekFrom::Start(body_offset + 36)).unwrap();
                let mut counts = [0_u8; 8];
                file.read_exact(&mut counts).unwrap();
                let part_count = u64::from(u32::from_le_bytes(counts[0..4].try_into().unwrap()));
                let point_count = u64::from(u32::from_le_bytes(counts[4..8].try_into().unwrap()));
                assert!(part_count > 0 && point_count > 1);
                let points_offset = body_offset + 44 + part_count * 4;
                let last_x_offset = points_offset + (point_count - 1) * 16;
                file.seek(SeekFrom::Start(last_x_offset)).unwrap();
                file.write_all(&1.0_f64.to_le_bytes()).unwrap();
                return;
            }
            record_offset += 8 + content_bytes;
        }
        panic!("record Shapefile {target_record} inesistente");
    }

    fn truncate_dbf_mid_record(path: &Path, complete_records: u64) {
        let dbf_path = path.with_extension("dbf");
        let header = std::fs::read(&dbf_path).unwrap();
        let header_length = u64::from(u16::from_le_bytes(header[8..10].try_into().unwrap()));
        let record_length = u64::from(u16::from_le_bytes(header[10..12].try_into().unwrap()));
        assert!(record_length > 1);
        let truncated_length = header_length + complete_records * record_length + record_length / 2;
        std::fs::OpenOptions::new()
            .write(true)
            .open(dbf_path)
            .unwrap()
            .set_len(truncated_length)
            .unwrap();
    }

    fn mark_dbf_record_deleted(path: &Path, source_index: u64) {
        let dbf_path = path.with_extension("dbf");
        let header = std::fs::read(&dbf_path).unwrap();
        let header_length = u64::from(u16::from_le_bytes(header[8..10].try_into().unwrap()));
        let record_length = u64::from(u16::from_le_bytes(header[10..12].try_into().unwrap()));
        let marker_offset = header_length + source_index * record_length;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(dbf_path)
            .unwrap();
        file.seek(SeekFrom::Start(marker_offset)).unwrap();
        file.write_all(b"*").unwrap();
    }

    fn overwrite_dbf_ascii_field(path: &Path, source_index: u64, field_name: &str, value: &str) {
        let dbf_path = path.with_extension("dbf");
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dbf_path)
            .unwrap();
        let mut header = [0_u8; 32];
        file.read_exact(&mut header).unwrap();
        let header_length = u64::from(u16::from_le_bytes(header[8..10].try_into().unwrap()));
        let record_length = u64::from(u16::from_le_bytes(header[10..12].try_into().unwrap()));
        let mut descriptor_offset = 32_u64;
        let mut field_offset = 1_u64;
        loop {
            file.seek(SeekFrom::Start(descriptor_offset)).unwrap();
            let mut descriptor = [0_u8; 32];
            file.read_exact(&mut descriptor).unwrap();
            assert_ne!(descriptor[0], 0x0d, "campo DBF non trovato");
            let name_end = descriptor[..11]
                .iter()
                .position(|byte| *byte == 0)
                .map_or(11, |position| position);
            let name = std::str::from_utf8(&descriptor[..name_end]).unwrap();
            let width = usize::from(descriptor[16]);
            if name == field_name {
                assert!(value.len() <= width);
                let mut encoded = vec![b' '; width];
                encoded[width - value.len()..].copy_from_slice(value.as_bytes());
                let record_offset = header_length + source_index * record_length + field_offset;
                file.seek(SeekFrom::Start(record_offset)).unwrap();
                file.write_all(&encoded).unwrap();
                return;
            }
            field_offset += width as u64;
            descriptor_offset += 32;
        }
    }

    fn consume_until_error(reader: &mut dyn LayerReader) -> (usize, PlenoraIoError) {
        let mut emitted_rows = 0;
        loop {
            match reader.next_batch() {
                Ok(Some(batch)) => emitted_rows += batch.num_rows(),
                Ok(None) => panic!("atteso rifiuto row-scoped"),
                Err(error) => return (emitted_rows, error),
            }
        }
    }

    #[test]
    fn degenerate_polygon_rings_have_a_stable_rejection_cause() {
        let repeated = Point::new(1.0, 1.0);
        let rings = vec![PolygonRing::Outer(vec![
            repeated, repeated, repeated, repeated,
        ])];

        assert_eq!(polygon_rejection_cause(&rings), Some(DEGENERATE_RING_CAUSE));
    }

    // Una sola fixture copre scrittura, corruzioni mirate e le varianti di
    // configurazione della diagnostica: separarle duplicherebbe la costruzione
    // dello shapefile e ne perderebbe la sequenza.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn invalid_polygon_rows_return_complete_bounded_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("invalid-polygons.shp");
        let key_name = shapefile::dbase::FieldName::try_from("ID_PART").unwrap();
        let numeric_key_name = shapefile::dbase::FieldName::try_from("NUM_KEY").unwrap();
        let integer_value_name = shapefile::dbase::FieldName::try_from("INT_VALUE").unwrap();
        let table = TableWriterBuilder::new()
            .add_character_field(key_name, 32)
            .add_numeric_field(numeric_key_name, 20, 2)
            .add_numeric_field(integer_value_name, 18, 0);
        let mut writer = Writer::from_path(&path, table).unwrap();

        let key_base = 9_007_199_254_740_992_u64;
        for source_index in 0..128 {
            let points = vec![
                Point::new(0.0, 0.0),
                Point::new(0.0, 5.0),
                Point::new(5.0, 5.0),
                Point::new(5.0, 0.0),
                Point::new(0.0, 0.0),
            ];
            let rings = if matches!(source_index, 17 | 113) {
                vec![PolygonRing::Inner(points)]
            } else {
                vec![PolygonRing::Outer(points)]
            };
            let polygon = Polygon::with_rings(rings);
            // source_index < 128: la conversione in f64 e' esatta.
            #[allow(clippy::cast_precision_loss)]
            let numeric_value = source_index as f64;
            let mut record = Record::default();
            record.insert(
                "ID_PART".to_owned(),
                FieldValue::Character(Some((key_base + source_index).to_string())),
            );
            record.insert(
                "NUM_KEY".to_owned(),
                FieldValue::Numeric(Some(numeric_value)),
            );
            record.insert(
                "INT_VALUE".to_owned(),
                FieldValue::Numeric(Some(numeric_value)),
            );
            writer.write_shape_and_record(&polygon, &record).unwrap();
        }
        drop(writer);
        make_polygon_ring_unclosed(&path, 89);
        mark_dbf_record_deleted(&path, 20);
        for source_index in [17_u64, 89, 113] {
            overwrite_dbf_ascii_field(
                &path,
                source_index,
                "NUM_KEY",
                &format!("{}.25", key_base + source_index),
            );
        }
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();
        let malformed_directory = tempfile::tempdir().unwrap();
        let malformed_path = malformed_directory.path().join("invalid-attribute.shp");
        for extension in ["shp", "shx", "dbf", "prj"] {
            std::fs::copy(
                path.with_extension(extension),
                malformed_path.with_extension(extension),
            )
            .unwrap();
        }

        let mut options = read_opts();
        options
            .format_options
            .insert("row_diagnostics.examples_limit".to_owned(), "2".to_owned());
        options
            .format_options
            .insert("row_diagnostics.key_field".to_owned(), "ID_PART".to_owned());
        options
            .format_options
            .insert("row_diagnostics.key_policy".to_owned(), "emit".to_owned());
        let dataset = ShpDriver.open(Source::Path(path.clone()), options).unwrap();
        let request = ReadRequest {
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 8,
            },
            ..req()
        };
        let mut reader = dataset.open_layer_reader(&request).unwrap();
        let (emitted_rows, error) = consume_until_error(reader.as_mut());
        assert_eq!(emitted_rows, 0);
        let diagnostics = error
            .row_diagnostics
            .expect("diagnostica row-scoped mancante");
        assert_eq!(diagnostics.observed_total, 3);
        assert_eq!(diagnostics.total, Some(3));
        assert_eq!(
            diagnostics.counts.get("shapefile.inner_ring_without_outer"),
            Some(&2)
        );
        assert_eq!(diagnostics.counts.get("shapefile.unclosed_ring"), Some(&1));
        assert_eq!(diagnostics.examples_limit, 2);
        assert!(diagnostics.examples_truncated);
        assert_eq!(diagnostics.examples.len(), 2);
        assert_eq!(diagnostics.examples[0].source_index, 17);
        assert_eq!(diagnostics.examples[1].source_index, 89);
        assert_eq!(
            diagnostics.examples[0]
                .key
                .as_ref()
                .and_then(|key| key.value.as_ref()),
            Some(&plenora_io_model::RowDiagnosticKeyValue::String(
                (key_base + 17).to_string()
            ))
        );
        assert_eq!(
            diagnostics.examples[1]
                .key
                .as_ref()
                .and_then(|key| key.value.as_ref()),
            Some(&plenora_io_model::RowDiagnosticKeyValue::String(
                (key_base + 89).to_string()
            ))
        );

        let mut attribute_only_request = req();
        attribute_only_request.projected_fields = Some(vec![FieldId(1)]);
        attribute_only_request.batch_target = BatchTarget {
            target_bytes: 8 * 1024 * 1024,
            max_rows: 8,
        };
        let attribute_only_dataset = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        let mut attribute_only_reader = attribute_only_dataset
            .open_layer_reader(&attribute_only_request)
            .unwrap();
        let (attribute_rows, attribute_error) = consume_until_error(attribute_only_reader.as_mut());
        assert_eq!(attribute_rows, 0);
        assert_eq!(attribute_error.row_diagnostics.unwrap().observed_total, 3);

        let mut numeric_key_options = read_opts();
        numeric_key_options
            .format_options
            .insert("row_diagnostics.examples_limit".to_owned(), "2".to_owned());
        numeric_key_options
            .format_options
            .insert("row_diagnostics.key_field".to_owned(), "NUM_KEY".to_owned());
        numeric_key_options
            .format_options
            .insert("row_diagnostics.key_policy".to_owned(), "emit".to_owned());
        let numeric_key_dataset = ShpDriver
            .open(Source::Path(path.clone()), numeric_key_options)
            .unwrap();
        let mut numeric_key_reader = numeric_key_dataset.open_layer_reader(&request).unwrap();
        let (_, numeric_key_error) = consume_until_error(numeric_key_reader.as_mut());
        let numeric_examples = numeric_key_error.row_diagnostics.unwrap().examples;
        for (example, source_index) in numeric_examples.iter().zip([17_u64, 89]) {
            assert_eq!(
                example.key.as_ref().and_then(|key| key.value.as_ref()),
                Some(&plenora_io_model::RowDiagnosticKeyValue::String(format!(
                    "{}.25",
                    key_base + source_index
                )))
            );
        }

        let dataset_without_key = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        let mut reader_without_key = dataset_without_key.open_layer_reader(&request).unwrap();
        let (_, error_without_key) = consume_until_error(reader_without_key.as_mut());
        assert!(error_without_key
            .row_diagnostics
            .unwrap()
            .examples
            .iter()
            .all(|example| example.key.is_none()));

        let mut redacted_options = read_opts();
        redacted_options
            .format_options
            .insert("row_diagnostics.key_field".to_owned(), "ID_PART".to_owned());
        redacted_options
            .format_options
            .insert("row_diagnostics.key_policy".to_owned(), "redact".to_owned());
        let redacted_dataset = ShpDriver
            .open(Source::Path(path.clone()), redacted_options)
            .unwrap();
        let mut redacted_reader = redacted_dataset.open_layer_reader(&request).unwrap();
        let (_, redacted_error) = consume_until_error(redacted_reader.as_mut());
        let redacted_key = redacted_error.row_diagnostics.unwrap().examples[0]
            .key
            .clone()
            .unwrap();
        assert_eq!(redacted_key.state, RowDiagnosticKeyState::Redacted);
        assert!(redacted_key.value.is_none());

        let mut missing_policy = read_opts();
        missing_policy
            .format_options
            .insert("row_diagnostics.key_field".to_owned(), "ID_PART".to_owned());
        let Err(missing_policy_error) = ShpDriver.open(Source::Path(path.clone()), missing_policy)
        else {
            panic!("key_field senza policy deve essere rifiutato")
        };
        assert_eq!(
            missing_policy_error.category,
            plenora_io_model::ErrorCategory::InvalidConfiguration
        );

        let mut zero_limit = read_opts();
        zero_limit
            .format_options
            .insert("row_diagnostics.examples_limit".to_owned(), "0".to_owned());
        let Err(zero_limit_error) = ShpDriver.open(Source::Path(path.clone()), zero_limit) else {
            panic!("examples_limit zero deve essere rifiutato")
        };
        assert_eq!(
            zero_limit_error.category,
            plenora_io_model::ErrorCategory::InvalidConfiguration
        );

        let mut cancelled_diagnostics = ShpRowDiagnostics::new(ShpRowDiagnosticsConfig {
            examples_limit: 1,
            key: None,
        });
        cancelled_diagnostics.record(
            17,
            INNER_RING_WITHOUT_OUTER_CAUSE,
            Some(&Record::default()),
            None,
        );
        let cancellation = plenora_io_model::CancellationToken::default();
        cancellation.cancel();
        let cancellation_error =
            plenora_io_core::check_cancelled(&cancellation, plenora_io_model::ErrorPhase::Read)
                .expect_err("la cancellation richiesta deve essere osservata");
        let cancelled = cancelled_diagnostics
            .into_partial_error(cancellation_error, "shapefile.scan_cancelled")
            .row_diagnostics
            .unwrap();
        assert_eq!(cancelled.completeness, RowDiagnosticsCompleteness::Partial);
        assert_eq!(
            cancelled.knowledge_limits,
            Some(vec!["shapefile.scan_cancelled".to_owned()])
        );
        assert!(cancelled.total.is_none());

        let partial_dataset = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        truncate_dbf_mid_record(&path, 50);
        let mut partial_reader = partial_dataset.open_layer_reader(&request).unwrap();
        let (_, partial_error) = consume_until_error(partial_reader.as_mut());
        let partial = partial_error.row_diagnostics.unwrap();
        assert_eq!(partial.completeness, RowDiagnosticsCompleteness::Partial);
        assert_eq!(partial.total, None);
        assert_eq!(partial.observed_total, 1);
        assert_eq!(
            partial.knowledge_limits,
            Some(vec!["shapefile.dbf_exact_scan_interrupted".to_owned()])
        );
        assert_eq!(
            partial.counts.get("shapefile.inner_ring_without_outer"),
            Some(&1)
        );
        assert_eq!(partial.examples.len(), 1);
        assert!(!partial.examples_truncated);

        overwrite_dbf_ascii_field(&malformed_path, 42, "INT_VALUE", "not-an-integer");
        let malformed_dataset = ShpDriver
            .open(Source::Path(malformed_path), read_opts())
            .unwrap();
        let mut malformed_reader = malformed_dataset.open_layer_reader(&request).unwrap();
        let (_, malformed_error) = consume_until_error(malformed_reader.as_mut());
        let malformed = malformed_error.row_diagnostics.unwrap();
        assert_eq!(malformed.completeness, RowDiagnosticsCompleteness::Complete);
        assert_eq!(malformed.observed_total, 4);
        assert_eq!(
            malformed.counts.get(ATTRIBUTE_NUMERIC_INVALID_CAUSE),
            Some(&1)
        );
        assert!(malformed.examples.iter().any(|example| {
            example.source_index == 42 && example.cause == ATTRIBUTE_NUMERIC_INVALID_CAUSE
        }));
    }

    #[test]
    fn accepted_rows_stops_invalid_shapefile_scan_at_active_row_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bounded-invalid.shp");
        let id = shapefile::dbase::FieldName::try_from("ID").unwrap();
        let table = TableWriterBuilder::new().add_numeric_field(id, 9, 0);
        let mut writer = Writer::from_path(&path, table).unwrap();
        for source_index in 0..4_096 {
            let points = vec![
                Point::new(0.0, 0.0),
                Point::new(0.0, 5.0),
                Point::new(5.0, 5.0),
                Point::new(5.0, 0.0),
                Point::new(0.0, 0.0),
            ];
            let rings = if matches!(source_index, 17 | 89 | 3_000) {
                vec![PolygonRing::Inner(points)]
            } else {
                vec![PolygonRing::Outer(points)]
            };
            let mut record = Record::default();
            record.insert(
                "ID".to_owned(),
                FieldValue::Numeric(Some(f64::from(source_index))),
            );
            writer
                .write_shape_and_record(&Polygon::with_rings(rings), &record)
                .unwrap();
        }
        drop(writer);
        mark_dbf_record_deleted(&path, 20);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        // Fault-tail deterministico: il dataset e' inferito quando integro, poi
        // la coda oltre il prefisso richiesto diventa illeggibile.
        let dataset = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        truncate_dbf_mid_record(&path, 200);
        let request = ReadRequest {
            scope: plenora_io_core::ReadScope::AcceptedRows(32),
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 8,
            },
            ..req()
        };
        let mut reader = dataset.open_layer_reader(&request).unwrap();
        let (emitted_rows, error) = consume_until_error(reader.as_mut());
        assert_eq!(emitted_rows, 0);
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Partial
        );
        assert_eq!(diagnostics.total, None);
        assert_eq!(diagnostics.observed_total, 1);
        assert_eq!(diagnostics.counts[INNER_RING_WITHOUT_OUTER_CAUSE], 1);
        assert_eq!(diagnostics.examples[0].source_index, 17);
        assert_eq!(
            diagnostics.knowledge_limits.as_deref(),
            Some(["read_scope_row_limit_reached".to_owned()].as_slice())
        );
        assert!(diagnostics.validate().is_ok());
    }

    #[test]
    fn accepted_rows_preserves_valid_shapefile_batch_overshoot_and_skips_late_invalidity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("late-invalid.shp");
        let id = shapefile::dbase::FieldName::try_from("ID").unwrap();
        let table = TableWriterBuilder::new().add_numeric_field(id, 9, 0);
        let mut writer = Writer::from_path(&path, table).unwrap();
        for source_index in 0..25 {
            let points = vec![
                Point::new(0.0, 0.0),
                Point::new(0.0, 5.0),
                Point::new(5.0, 5.0),
                Point::new(5.0, 0.0),
                Point::new(0.0, 0.0),
            ];
            let rings = if source_index == 20 {
                vec![PolygonRing::Inner(points)]
            } else {
                vec![PolygonRing::Outer(points)]
            };
            let mut record = Record::default();
            record.insert(
                "ID".to_owned(),
                FieldValue::Numeric(Some(f64::from(source_index))),
            );
            writer
                .write_shape_and_record(&Polygon::with_rings(rings), &record)
                .unwrap();
        }
        drop(writer);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        let dataset = ShpDriver
            .open(Source::Path(path.clone()), read_opts())
            .unwrap();
        let request = ReadRequest {
            scope: ReadScope::AcceptedRows(10),
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows: 8,
            },
            ..req()
        };
        let mut reader = dataset.open_layer_reader(&request).unwrap();
        let mut rows = Vec::new();
        while let Some(batch) = reader.next_batch().unwrap() {
            rows.push(batch.num_rows());
        }
        assert_eq!(rows, vec![8, 8]);

        let complete_dataset = ShpDriver.open(Source::Path(path), read_opts()).unwrap();
        let mut complete_request = request;
        complete_request.scope = ReadScope::Complete;
        let mut complete = complete_dataset
            .open_layer_reader(&complete_request)
            .unwrap();
        let (published_rows, error) = consume_until_error(complete.as_mut());
        assert_eq!(published_rows, 0);
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(
            diagnostics.completeness,
            RowDiagnosticsCompleteness::Complete
        );
        assert_eq!(diagnostics.examples[0].source_index, 20);
        assert_eq!(diagnostics.total, Some(1));
    }

    #[test]
    fn prj_authority_is_resolved_and_keeps_epsg_axis_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roads.shp");
        std::fs::write(
            path.with_extension("prj"),
            "GEOGCS[\"WGS 84\",AUTHORITY[\"EPSG\",\"4326\"]]",
        )
        .unwrap();

        let crs = resolve_crs(&path, &opzioni_lettura()).unwrap();
        assert_eq!(crs.id.as_deref(), Some("EPSG:4326"));
        assert_eq!(
            crs.axis_order,
            plenora_io_model::crs::AxisOrder::LatitudeLongitude
        );
    }

    #[test]
    fn projected_prj_with_nested_geogcs_keeps_projected_kind_and_axis_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("parcels.shp");
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        let crs = resolve_crs(&path, &opzioni_lettura()).unwrap();
        assert_eq!(crs.id.as_deref(), Some("EPSG:3003"));
        assert_eq!(crs.kind, CrsKind::Projected);
        assert_eq!(
            crs.axis_order,
            plenora_io_model::crs::AxisOrder::EastingNorthing
        );
    }

    #[test]
    fn wide_zero_decimal_dbf_numeric_is_read_exactly_as_i64() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("parcels.shp");
        let field_name = shapefile::dbase::FieldName::try_from("parcel_id").unwrap();
        let table = TableWriterBuilder::new().add_numeric_field(field_name, 18, 0);
        let mut writer = Writer::from_path(&path, table).unwrap();
        for coordinate in [0.0, 1.0] {
            let mut record = Record::default();
            record.insert("parcel_id".to_owned(), FieldValue::Numeric(Some(0.0)));
            writer
                .write_shape_and_record(&Point::new(coordinate, coordinate), &record)
                .unwrap();
        }
        drop(writer);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        // Il writer dbase accetta già f64. Si sostituiscono i byte del campo
        // con due interi ASCII distinti per riprodurre un DBF patrimoniale
        // reale prima che dbase 0.5.0 li converta nello stesso f64.
        let mut dbf = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("dbf"))
            .unwrap();
        let mut header = [0_u8; 32];
        dbf.read_exact(&mut header).unwrap();
        let header_length = u64::from(u16::from_le_bytes([header[8], header[9]]));
        let record_length = u64::from(u16::from_le_bytes([header[10], header[11]]));
        for (row, value) in ["9007199254740992", "9007199254740993"]
            .into_iter()
            .enumerate()
        {
            dbf.seek(SeekFrom::Start(
                header_length + (row as u64 * record_length) + 1,
            ))
            .unwrap();
            dbf.write_all(format!("{value:>18}").as_bytes()).unwrap();
        }
        drop(dbf);

        let dataset = ShpDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let assessment = dataset.fidelity_assessment();
        assert_eq!(assessment.level, Fidelity::Conditional);

        let mut reader = dataset.open_layer_reader(&req()).unwrap();
        let loss = reader.loss_report();
        assert!(!loss
            .counts
            .contains_key(DBF_NUMERIC_INTEGER_PRECISION_UNVERIFIABLE));
        let batch = reader.next_batch().unwrap().unwrap();
        let ids = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow_array::Int64Array>()
            .unwrap();
        assert_eq!(ids.value(0), 9_007_199_254_740_992);
        assert_eq!(ids.value(1), 9_007_199_254_740_993);
    }

    #[test]
    fn narrow_or_decimal_dbf_numeric_keeps_float_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("numeric-shapes.shp");
        let narrow = shapefile::dbase::FieldName::try_from("narrow").unwrap();
        let decimal = shapefile::dbase::FieldName::try_from("decimal").unwrap();
        let table = TableWriterBuilder::new()
            .add_numeric_field(narrow, 9, 0)
            .add_numeric_field(decimal, 18, 2);
        let mut writer = Writer::from_path(&path, table).unwrap();
        let mut record = Record::default();
        record.insert("narrow".to_owned(), FieldValue::Numeric(Some(123.0)));
        record.insert("decimal".to_owned(), FieldValue::Numeric(Some(12.5)));
        writer
            .write_shape_and_record(&Point::new(0.0, 0.0), &record)
            .unwrap();
        drop(writer);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        let dataset = ShpDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let schema = &dataset.layers()[0].contract.schema;
        assert_eq!(schema.field(1).data_type(), &DataType::Float64);
        assert_eq!(schema.field(2).data_type(), &DataType::Float64);
    }

    #[test]
    fn duplicate_dbf_field_names_are_rejected_before_record_map_collapse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("duplicates.shp");
        let first = shapefile::dbase::FieldName::try_from("first").unwrap();
        let second = shapefile::dbase::FieldName::try_from("second").unwrap();
        let table = TableWriterBuilder::new()
            .add_character_field(first, 16)
            .add_character_field(second, 16);
        let mut writer = Writer::from_path(&path, table).unwrap();
        let mut record = Record::default();
        record.insert(
            "first".to_owned(),
            FieldValue::Character(Some("a".to_owned())),
        );
        record.insert(
            "second".to_owned(),
            FieldValue::Character(Some("b".to_owned())),
        );
        writer
            .write_shape_and_record(&Point::new(0.0, 0.0), &record)
            .unwrap();
        drop(writer);
        std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();

        let mut dbf = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("dbf"))
            .unwrap();
        dbf.seek(SeekFrom::Start(
            (DBF_HEADER_SIZE + DBF_FIELD_DESCRIPTOR_SIZE) as u64,
        ))
        .unwrap();
        let mut duplicate = [0_u8; DBF_FIELD_NAME_SIZE];
        duplicate[..5].copy_from_slice(b"first");
        dbf.write_all(&duplicate).unwrap();
        drop(dbf);

        let error = ShpDriver
            .open(Source::Path(path), opzioni_lettura())
            .err()
            .expect("il DBF con nomi duplicati deve essere rifiutato");
        assert!(error.to_string().contains("nomi campo DBF duplicati"));
    }

    #[test]
    fn unresolved_prj_is_preserved_in_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.shp");
        let definition = "LOCAL_CS[\"survey-grid-secret\"]";
        std::fs::write(path.with_extension("prj"), definition).unwrap();

        let error = resolve_crs(&path, &opzioni_lettura()).unwrap_err();
        assert_eq!(error.code, plenora_io_model::IoErrorCode::CrsUnresolved);
        assert_eq!(error.driver.as_deref(), Some("shp"));
        assert!(!error.to_string().contains("survey-grid-secret"));
    }

    #[test]
    fn assumed_unknown_epsg_does_not_invent_an_axis_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-prj.shp");
        let crs = resolve_crs(&path, &opzioni_lettura().with_assume_crs("EPSG:4258")).unwrap();
        assert_eq!(crs.kind, CrsKind::Unknown);
        assert_eq!(crs.axis_order, plenora_io_model::crs::AxisOrder::Unknown);
    }

    #[test]
    fn resolved_crs_without_id_cannot_be_relabelled_as_unknown() {
        let crs = ResolvedCrs::new(
            None,
            CrsKind::Unknown,
            Some("LOCAL_CS[\"private\"]".to_owned()),
        );

        assert!(matches!(
            resolved_crs_id(&crs),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Crs
        ));
    }

    #[test]
    fn write_then_read_round_trip() {
        use arrow_array::{Int64Array, StringArray};
        use arrow_schema::DataType;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("pts.shp");

        let wkb1 = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            12.5, 45.9,
        )))
        .unwrap();
        let wkb2 = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            9.19, 45.46,
        )))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("nome", DataType::Utf8, true),
            Field::new("pop", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![
                    Some(wkb1.as_slice()),
                    Some(wkb2.as_slice()),
                ])),
                Arc::new(StringArray::from(vec!["Roma", "Milano"])),
                Arc::new(Int64Array::from(vec![2_800_000i64, 1_400_000])),
            ],
        )
        .unwrap();

        let driver = ShpDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        // il set è stato pubblicato
        assert!(out.exists());
        assert!(out.with_extension("dbf").exists());
        assert!(out.with_extension("prj").exists());

        // rilettura
        let ds = driver.open(Source::Path(out), read_opts()).unwrap();
        let mut r = ds.open_layer_reader(&req()).unwrap();
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 2);
        let nome = rb
            .column_by_name("nome")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(nome.value(0), "Roma");
    }

    #[test]
    fn writer_adapter_attributes_mixed_geometry_and_prevents_publish() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("mixed.shp");
        let point = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1.0, 2.0))).unwrap();
        let line = to_wkb(&geo_types::Geometry::LineString(
            geo_types::LineString::from(vec![(0.0, 0.0), (1.0, 1.0)]),
        ))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(BinaryArray::from(vec![
                Some(point.as_slice()),
                Some(line.as_slice()),
            ]))],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "mixed".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut writer = ShpDriver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.declare_input_total(LayerId(0), 2).unwrap();

        let error = writer.write(&batch).unwrap_err();
        let diagnostics = error.row_diagnostics.as_deref().unwrap();
        assert_eq!(diagnostics.input_total, Some(2));
        assert_eq!(diagnostics.examples[0].source_index, 1);
        assert_eq!(diagnostics.counts["shapefile.mixed_geometry_type"], 1);
        assert!(diagnostics.validate().is_ok());
        assert!(writer.finish().is_err());
        assert!(!output.exists());
        assert!(!output.with_extension("dbf").exists());
    }

    #[test]
    fn directory_dataset_round_trip_uses_atomic_directory_unit() {
        use arrow_array::Int64Array;
        use arrow_schema::DataType;

        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("points.shp.d");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
            12.5, 45.9,
        )))
        .unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(Int64Array::from(vec![1_i64])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let options = opzioni_scrittura()
            .with_durable(true)
            .with_format_option("publish_mode", DIRECTORY_DATASET_MODE);

        let driver = ShpDriver;
        let mut writer = driver
            .create(Sink::Path(output.clone()), &plan, &options)
            .unwrap();
        writer.write(&batch).unwrap();
        assert!(
            !output.exists(),
            "la directory dataset è diventata visibile prima di finish"
        );
        let published = writer.finish().unwrap();

        let expected_outcome = if cfg!(unix) {
            plenora_io_core::PublishOutcome::Published
        } else {
            plenora_io_core::PublishOutcome::PublishedButDurabilityUnconfirmed
        };
        assert_eq!(published.outcome, expected_outcome);
        assert!(output.is_dir());
        assert!(output.join("data.shp").is_file());
        assert!(output.join("data.shx").is_file());
        assert!(output.join("data.dbf").is_file());
        assert!(output.join("data.prj").is_file());

        let dataset = driver.open(Source::Path(output), read_opts()).unwrap();
        let mut reader = dataset.open_layer_reader(&req()).unwrap();
        assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 1);
    }

    #[test]
    fn directory_dataset_abort_removes_staging() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("aborted.shp.d");
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };

        let writer = ShpDriver
            .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        drop(writer);

        assert!(!output.exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn publish_mode_must_match_destination_shape() {
        let root = tempfile::tempdir().unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![geometry_field(GEOMETRY, "EPSG:4326")]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut options = opzioni_scrittura();
        options
            .format_options
            .insert("publish_mode".to_owned(), DIRECTORY_DATASET_MODE.to_owned());

        let result = ShpDriver
            .create(Sink::Path(root.path().join("points.shp")), &plan, &options)
            .map(|_| ());

        assert!(matches!(
            result,
            Err(error) if error.code == plenora_io_model::IoErrorCode::Unsupported
        ));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn rejects_long_field_name() {
        use arrow_schema::DataType;
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("nome_campo_troppo_lungo", DataType::Utf8, true),
        ]));
        let driver = ShpDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let dir = tempfile::tempdir().unwrap();
        let e = driver
            .create(
                Sink::Path(dir.path().join("x.shp")),
                &plan,
                &opzioni_scrittura(),
            )
            .map(|_| ())
            .unwrap_err();
        assert_eq!(
            e.capability_reason,
            Some(plenora_io_model::CapabilityReason::FieldNameTooLong)
        );
    }

    #[test]
    fn streams_multiple_batches() {
        use arrow_array::Int64Array;
        use arrow_schema::DataType;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("many.shp");
        let wkb: Vec<Vec<u8>> = (0..10)
            .map(|i| {
                to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(
                    f64::from(i),
                    f64::from(i),
                )))
                .unwrap()
            })
            .collect();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("id", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(
                    wkb.iter().map(|w| Some(w.as_slice())).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from((0..10i64).collect::<Vec<_>>())),
            ],
        )
        .unwrap();

        let driver = ShpDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let ds = driver.open(Source::Path(out), read_opts()).unwrap();
        let req = ReadRequest {
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
        };
        let mut r = ds.open_layer_reader(&req).unwrap();
        let (mut total, mut batches) = (0, 0);
        while let Some(b) = r.next_batch().unwrap() {
            total += b.num_rows();
            batches += 1;
        }
        assert_eq!(total, 10);
        assert!(
            batches >= 3,
            "atteso streaming multi-batch, avuti {batches}"
        );
    }

    fn dimensional_point(
        dimensions: CoordinateDimensions,
        z: Option<f64>,
        m: Option<f64>,
    ) -> WkbGeometry {
        WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 12.5,
                y: 45.9,
                z,
                m,
            }),
            dimensions,
            srid: None,
        }
    }

    fn round_trip_dimensional_point(
        dimensions: CoordinateDimensions,
        geometry: &WkbGeometry,
    ) -> WkbGeometry {
        use arrow_array::Int64Array;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join(format!("{dimensions:?}.shp"));
        let bytes = encode_wkb(geometry, WkbFlavor::Iso).unwrap();
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), false);
        geometry_contract.dimensions = dimensions;
        geometry_contract.set_exact_geometry_types(vec![GeometryType::Point]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            with_geometry_contract_metadata(
                &geometry_field(GEOMETRY, "EPSG:4326"),
                &geometry_contract,
            ),
            Field::new("id", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())])),
                Arc::new(Int64Array::from(vec![1_i64])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "points".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry_contract),
                },
            }],
        };

        let driver = ShpDriver;
        let mut writer = driver
            .create(Sink::Path(out.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let mut native = shapefile::Reader::from_path(&out).unwrap();
        let (shape, _) = native.iter_shapes_and_records().next().unwrap().unwrap();
        match dimensions {
            CoordinateDimensions::Xym => assert!(matches!(shape, Shape::PointM(_))),
            CoordinateDimensions::Xyz | CoordinateDimensions::Xyzm => {
                assert!(matches!(shape, Shape::PointZ(_)));
            }
            _ => unreachable!("test solo dimensionale"),
        }

        let dataset = driver.open(Source::Path(out), read_opts()).unwrap();
        let layer = &dataset.layers()[0];
        let output_contract = layer.contract.geometry.as_ref().unwrap();
        assert_eq!(output_contract.dimensions, dimensions);
        assert_eq!(output_contract.geometry_types, vec![GeometryType::Point]);
        assert!(output_contract
            .native_metadata
            .contains_key("shp.shape_type"));
        let mut reader = dataset.open_layer_reader(&req()).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let geometry = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap()
    }

    #[test]
    fn round_trip_preserves_xyz_xym_and_xyzm_points() {
        let cases = [
            dimensional_point(CoordinateDimensions::Xyz, Some(123.25), None),
            dimensional_point(CoordinateDimensions::Xym, None, Some(7.5)),
            dimensional_point(CoordinateDimensions::Xyzm, Some(123.25), Some(7.5)),
        ];
        for expected in cases {
            let actual = round_trip_dimensional_point(expected.dimensions, &expected);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn direct_conversion_preserves_xyzm_multiline_and_no_data_measure() {
        let dimensions = CoordinateDimensions::Xyzm;
        let line = WkbGeometry {
            value: WkbValue::MultiLineString(vec![WkbGeometry {
                value: WkbValue::LineString(vec![
                    WkbCoordinate {
                        x: 0.0,
                        y: 1.0,
                        z: Some(2.0),
                        m: Some(NO_DATA),
                    },
                    WkbCoordinate {
                        x: 3.0,
                        y: 4.0,
                        z: Some(5.0),
                        m: Some(6.0),
                    },
                ]),
                dimensions,
                srid: None,
            }]),
            dimensions,
            srid: None,
        };
        let shape = shape_from_wkb(line.clone()).unwrap();
        assert!(matches!(shape, Shape::PolylineZ(_)));
        let decoded = shape_to_wkb(&shape, dimensions).unwrap().unwrap();
        assert_eq!(decoded, line);
    }

    #[test]
    fn direct_conversion_preserves_xyzm_multipolygon_rings() {
        let dimensions = CoordinateDimensions::Xyzm;
        let coordinate = |x, y, z, m| WkbCoordinate {
            x,
            y,
            z: Some(z),
            m: Some(m),
        };
        let exterior = vec![
            coordinate(0.0, 0.0, 1.0, 10.0),
            coordinate(0.0, 5.0, 2.0, 11.0),
            coordinate(5.0, 5.0, 3.0, 12.0),
            coordinate(5.0, 0.0, 4.0, 13.0),
            coordinate(0.0, 0.0, 1.0, 10.0),
        ];
        let interior = vec![
            coordinate(1.0, 1.0, 5.0, 14.0),
            coordinate(4.0, 1.0, 6.0, 15.0),
            coordinate(4.0, 4.0, 7.0, 16.0),
            coordinate(1.0, 4.0, 8.0, 17.0),
            coordinate(1.0, 1.0, 5.0, 14.0),
        ];
        let polygon = WkbGeometry {
            value: WkbValue::MultiPolygon(vec![WkbGeometry {
                value: WkbValue::Polygon(vec![exterior, interior]),
                dimensions,
                srid: None,
            }]),
            dimensions,
            srid: None,
        };
        let shape = shape_from_wkb(polygon.clone()).unwrap();
        assert!(matches!(shape, Shape::PolygonZ(_)));
        let decoded = shape_to_wkb(&shape, dimensions).unwrap().unwrap();
        assert_eq!(decoded, polygon);
    }

    #[test]
    fn geometry_collection_is_rejected_without_xy_normalization() {
        let geometry = WkbGeometry {
            value: WkbValue::GeometryCollection(Vec::new()),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        assert!(shape_from_wkb(geometry).is_err());
    }

    #[test]
    fn declared_dimensions_without_required_ordinates_are_rejected() {
        let missing_z = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: None,
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        };
        let missing_m = WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(3.0),
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyzm,
            srid: None,
        };

        assert!(shape_from_wkb(missing_z).is_err());
        assert!(shape_from_wkb(missing_m).is_err());
    }
}

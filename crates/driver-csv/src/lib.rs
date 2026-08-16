//! driver-csv — CSV ⇄ `RecordBatch`. La geometria è dichiarata via
//! `format_options`: `x_column`+`y_column` (Point XY) oppure `wkt_column`
//! (WKT XY/XYZ/XYM/XYZM). CSV non porta CRS: `assume_crs` è obbligatorio
//! (ADR-IO 4).
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
    TypeCoercionPolicy, WritePlan, SCALAR_TYPES, UTF8_FIELD_NAMES, WKB_PASSTHROUGH_GEOMETRY,
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
    decode_wkb, encode_wkb_into, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue,
};
use plenora_io_model::{PlenoraIoError, Result};

const GEOMETRY: &str = "geometry";

fn err(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("csv", reason)
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "csv",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential,
    read_determinism: plenora_io_core::DeterminismLevel::Semantic,
    write_mode: Some(WriteMode::Streaming),
    write_determinism: Some(plenora_io_core::DeterminismLevel::Semantic),
    multi_layer: false,
    multi_file: false,
    reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
    projection_support: plenora_io_core::ProjectionSupport::Exact,
    predicate_pruning_support: plenora_io_core::PredicatePruningSupport::None,
    spatial_pruning_support: plenora_io_core::SpatialPruningSupport::None,
    crs_handling: CrsHandling::None,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    write_capabilities: Some(FormatWriteCapabilities {
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
    }),
    semantic_version: 1,
    driver_version: 6,
    descriptor_version: 7,
};

pub struct CsvDriver;

fn delimiter(opts: &std::collections::BTreeMap<String, String>) -> u8 {
    opts.get("delimiter")
        .and_then(|s| s.bytes().next())
        .unwrap_or(b',')
}

fn csv_reader(path: &Path, delim: u8) -> Result<csv::Reader<File>> {
    csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(true) // salta l'intestazione automaticamente
        .flexible(false)
        .from_path(path)
        .map_err(|e| err(format!("apertura CSV: {e}")))
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
        let path = plenora_io_core::preflight_source(source, &mut opts)?;
        let delim = delimiter(&opts.format_options);
        let crs = opts.assume_crs.clone().ok_or_else(|| {
            PlenoraIoError::Crs("CSV con geometria richiede --assume-crs".to_owned())
        })?;

        // Intestazione (nomi colonna).
        let headers: Vec<String> = {
            let mut rdr = csv::ReaderBuilder::new()
                .delimiter(delim)
                .has_headers(false)
                .flexible(true)
                .from_path(&path)
                .map_err(|e| err(format!("apertura CSV: {e}")))?;
            let mut first = csv::StringRecord::new();
            if !rdr
                .read_record(&mut first)
                .map_err(|e| err(format!("CSV non valido: {e}")))?
            {
                return Err(err("CSV vuoto"));
            }
            first.iter().map(str::to_owned).collect()
        };

        let idx = |name: &str| headers.iter().position(|h| h == name);
        let (geom, geom_cols): (GeomSpec, HashSet<usize>) =
            if let Some(w) = opts.format_options.get("wkt_column") {
                let wi = idx(w).ok_or_else(|| err(format!("colonna WKT '{w}' assente")))?;
                (GeomSpec::Wkt(wi), HashSet::from([wi]))
            } else if let (Some(x), Some(y)) = (
                opts.format_options.get("x_column"),
                opts.format_options.get("y_column"),
            ) {
                let xi = idx(x).ok_or_else(|| err(format!("colonna X '{x}' assente")))?;
                let yi = idx(y).ok_or_else(|| err(format!("colonna Y '{y}' assente")))?;
                (GeomSpec::Xy(xi, yi), HashSet::from([xi, yi]))
            } else {
                return Err(err(
                    "specificare wkt_column, oppure x_column con y_column, in format_options",
                ));
            };

        // Pass 1: inferenza tipi (RAM O(ncol), nessuna String per cella).
        let attrs = infer_types(&path, delim, &headers, &geom_cols)?;

        let (dimensions, geometry_types) = match geom {
            GeomSpec::Wkt(wi) => infer_wkt_geometry(&path, delim, wi)?,
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
        plenora_io_core::with_read_budget(
            Box::new(CsvDataset {
                path,
                delim,
                geom,
                attrs,
                layers: vec![LayerContract {
                    id: LayerId(0),
                    name,
                    contract,
                }],
            }),
            &opts,
            true,
        )
    }

    fn create(
        &self,
        sink: Sink,
        plan: &WritePlan,
        opts: &WriteOptions,
    ) -> Result<Box<dyn FormatWriter>> {
        validate_write(self.descriptor(), plan, opts.max_columns())?;
        let Sink::Path(path) = sink;
        if path.exists() {
            return Err(PlenoraIoError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        {
            return Err(PlenoraIoError::Unsupported(
                "l'output deve avere estensione .csv".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::Unsupported(
                "CSV: un solo layer per file".to_owned(),
            ));
        }
        let xy = matches!(
            opts.format_options
                .get("geometry_encoding")
                .map(String::as_str),
            Some("xy")
        );
        let staging = StagedFile::new(&path, opts.durable, opts.max_output_bytes())?;
        let writer = csv::WriterBuilder::new()
            .delimiter(delimiter(&opts.format_options))
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
    attrs: Vec<(usize, ColType)>,
    layers: Vec<LayerContract>,
}

impl OpenDatasetHandle for CsvDataset {
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
            self.path.clone(),
            self.delim,
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

fn infer_types(
    path: &Path,
    delim: u8,
    headers: &[String],
    geom_cols: &HashSet<usize>,
) -> Result<Vec<(usize, ColType)>> {
    let attr_idx: Vec<usize> = (0..headers.len())
        .filter(|i| !geom_cols.contains(i))
        .collect();
    let mut accs = vec![TypeAccumulator::default(); attr_idx.len()];
    let mut rdr = csv_reader(path, delim)?;
    let mut rec = csv::StringRecord::new();
    while rdr
        .read_record(&mut rec)
        .map_err(|e| err(format!("riga CSV non valida: {e}")))?
    {
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
) -> Result<(CoordinateDimensions, Vec<GeometryType>)> {
    let mut reader = csv_reader(path, delim)?;
    let mut record = csv::StringRecord::new();
    let mut dimensions = BTreeSet::new();
    let mut geometry_types = BTreeSet::new();
    while reader
        .read_record(&mut record)
        .map_err(|error| err(format!("riga CSV non valida: {error}")))?
    {
        let text = required_cell(&record, wkt_index)?.trim();
        if text.is_empty() {
            continue;
        }
        // Finding #6: cap a livello driver, in attesa che i `Limits` CLI
        // arrivino fino a qui (roadmap 1.1, lotto L6). Il default WKB
        // (`WkbLimits::default().max_cell_bytes`, 64 MiB) rifiuta payload
        // che superano il contratto del bordo prima di allocare l'AST wkt.
        let geometry = parse_wkt_bounded(text, WkbLimits::default().max_cell_bytes)?;
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

fn spawn_parser(
    path: PathBuf,
    delim: u8,
    geom: Option<GeomSpec>,
    attrs: Vec<(usize, ColType)>,
    schema: SchemaRef,
    mut batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    layer: LayerContract,
) -> Result<Box<dyn LayerReader>> {
    spawn_batch_reader(DESCRIPTOR.id, layer, 2, move |emitter: BatchEmitter| {
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
                .map_err(|error| err(format!("riga CSV non valida: {error}")))?;
            if !more {
                break;
            }
            if let (Some(builder), Some(spec)) = (&mut geom_b, geom) {
                append_geometry(builder, spec, &rec, &mut wkb_buf)?;
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
) -> Result<()> {
    match geom {
        GeomSpec::Wkt(wi) => {
            let cell = required_cell(rec, wi)?.trim();
            if cell.is_empty() {
                geom_b.append_null();
            } else {
                // Finding #6: vedi commento in `infer_wkt_geometry`.
                let geometry = parse_wkt_bounded(cell, WkbLimits::default().max_cell_bytes)?;
                buf.clear();
                encode_wkb_into(&geometry, WkbFlavor::Iso, buf)?;
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
                return Err(err(
                    "coordinate CSV incomplete: X e Y devono essere entrambe presenti",
                ));
            }
            let x = x_text
                .parse::<f64>()
                .map_err(|error| err(format!("coordinata X CSV non valida: {error}")))?;
            let y = y_text
                .parse::<f64>()
                .map_err(|error| err(format!("coordinata Y CSV non valida: {error}")))?;
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
            encode_wkb_into(&geometry, WkbFlavor::Iso, buf)?;
            geom_b.append_value(buf.as_slice());
        }
    }
    Ok(())
}

fn required_cell(record: &csv::StringRecord, index: usize) -> Result<&str> {
    record.get(index).ok_or_else(|| {
        err(format!(
            "riga CSV senza la colonna {index} dichiarata nell'intestazione"
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
    RecordBatch::try_new_with_options(schema.clone(), arrays, &options)
        .map_err(|error| err(format!("record batch: {error}")))
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
        let geom_idx = geometry_index(&schema).ok_or_else(|| err("nessuna colonna geometria"))?;
        let geom_col = batch
            .column(geom_idx)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| err("colonna geometria non binaria"))?;
        let limits = self.wkb_limits;
        let xy = self.xy;
        let w = self.writer.as_mut().ok_or_else(|| err("writer chiuso"))?;

        if !self.header_written {
            let mut header: Vec<&str> = Vec::new();
            for (i, f) in schema.fields().iter().enumerate() {
                if i != geom_idx {
                    header.push(f.name());
                }
            }
            if xy {
                header.push("x");
                header.push("y");
            } else {
                header.push("geometry");
            }
            w.write_record(&header).map_err(|e| err(e.to_string()))?;
            self.header_written = true;
        }

        // Scrittura per-campo DIRETTA (niente Vec<String> né serde_json::Value
        // per cella): `fbuf` è riusato per formattare numeri/bool.
        let mut fbuf = String::new();
        for row in 0..batch.num_rows() {
            for (i, _) in schema.fields().iter().enumerate() {
                if i != geom_idx {
                    write_cell(w, batch.column(i), row, &mut fbuf)?;
                }
            }
            if geom_col.is_null(row) {
                w.write_field("").map_err(|e| err(e.to_string()))?;
                if xy {
                    w.write_field("").map_err(|e| err(e.to_string()))?;
                }
            } else {
                let geom = decode_wkb(geom_col.value(row), &limits)?;
                if xy {
                    match geom.value {
                        WkbValue::Point(point) if geom.dimensions == CoordinateDimensions::Xy => {
                            fbuf.clear();
                            let _ = write!(fbuf, "{}", point.x);
                            w.write_field(&fbuf).map_err(|e| err(e.to_string()))?;
                            fbuf.clear();
                            let _ = write!(fbuf, "{}", point.y);
                            w.write_field(&fbuf).map_err(|e| err(e.to_string()))?;
                        }
                        _ => {
                            return Err(err("encoding xy richiede geometrie Point strettamente XY"))
                        }
                    }
                } else {
                    fbuf.clear();
                    format_wkt_into(&geom, &mut fbuf)?;
                    w.write_field(&fbuf).map_err(|e| err(e.to_string()))?;
                }
            }
            // Termina il record (dopo i write_field).
            w.write_record(None::<&[u8]>)
                .map_err(|e| err(e.to_string()))?;
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Published> {
        let mut w = self.writer.take().ok_or_else(|| err("writer già chiuso"))?;
        w.flush().map_err(|e| err(e.to_string()))?;
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
        return w.write_field("").map_err(|error| err(error.to_string()));
    }
    if let Some(a) = col.as_any().downcast_ref::<StringArray>() {
        w.write_field(a.value(row))
            .map_err(|error| err(error.to_string()))
    } else if let Some(a) = col.as_any().downcast_ref::<Int64Array>() {
        fbuf.clear();
        let _ = write!(fbuf, "{}", a.value(row));
        w.write_field(&*fbuf)
            .map_err(|error| err(error.to_string()))
    } else if let Some(a) = col.as_any().downcast_ref::<Float64Array>() {
        fbuf.clear();
        let _ = write!(fbuf, "{}", a.value(row));
        w.write_field(&*fbuf)
            .map_err(|error| err(error.to_string()))
    } else if let Some(a) = col.as_any().downcast_ref::<BooleanArray>() {
        w.write_field(if a.value(row) { "true" } else { "false" })
            .map_err(|error| err(error.to_string()))
    } else {
        // Tipo non comune (Date, ecc.): fallback via il convertitore generico.
        let value = json_from_array(col, row)?;
        w.write_field(cell_string(&value))
            .map_err(|error| err(error.to_string()))
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
mod tests {
    use super::*;
    use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadScope};
    use plenora_io_core::WriteLayer;
    use plenora_io_model::CancellationToken;
    use std::collections::BTreeMap;

    fn opts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn read_opts(pairs: &[(&str, &str)]) -> ReadOptions {
        ReadOptions::default()
            .with_assume_crs("EPSG:4326")
            .with_format_options(opts(pairs))
    }

    fn req(max_rows: usize) -> ReadRequest {
        ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget {
                target_bytes: 8 * 1024 * 1024,
                max_rows,
            },
            cancellation: CancellationToken::default(),
        }
    }

    #[test]
    fn round_trip_csv_xy() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.csv");
        std::fs::write(&src, "nome,lon,lat,pop\nA,12.5,45.9,100\nB,9.1,45.4,200\n").unwrap();

        let driver = CsvDriver;
        let ds = driver
            .open(
                Source::Path(src),
                read_opts(&[("x_column", "lon"), ("y_column", "lat")]),
            )
            .unwrap();
        let geom = ds.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(geom.crs.id(), Some("EPSG:4326"));
        let mut reader = ds.open_layer_reader(&req(65_536)).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert!(is_geometry_field(
            &batch.schema().field_with_name("geometry").unwrap().clone()
        ));
        let contract = ds.layers()[0].contract.clone();

        // scrivi come WKT e rileggi
        let out = dir.path().join("out.csv");
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract,
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("POINT"));
        assert!(text.contains("nome"));
    }

    #[test]
    fn integer_outside_i64_is_preserved_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("wide-integer.csv");
        std::fs::write(&source, "identifier,x,y\n18446744073709551615,12.5,45.9\n").unwrap();

        let dataset = CsvDriver
            .open(
                Source::Path(source),
                read_opts(&[("x_column", "x"), ("y_column", "y")]),
            )
            .unwrap();
        let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let identifier = batch
            .column(batch.schema().index_of("identifier").unwrap())
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();

        assert_eq!(identifier.value(0), "18446744073709551615");
    }

    #[test]
    fn target_bytes_splits_streaming_batches() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("many.csv");
        let mut s = String::from("id,geom\n");
        for i in 0..10 {
            writeln!(s, "{i},\"POINT ({i} {i})\"").unwrap();
        }
        std::fs::write(&src, s).unwrap();

        let driver = CsvDriver;
        let ds = driver
            .open(Source::Path(src), read_opts(&[("wkt_column", "geom")]))
            .unwrap();
        let mut request = req(100);
        request.batch_target.target_bytes = 1;
        let mut reader = ds.open_layer_reader(&request).unwrap();
        let (mut total, mut batches) = (0, 0);
        while let Some(b) = reader.next_batch().unwrap() {
            total += b.num_rows();
            batches += 1;
        }
        assert_eq!(total, 10);
        assert_eq!(batches, 10, "target byte non applicato: {batches} batch");
    }

    #[test]
    fn wkt_xyzm_round_trip_preserves_payload_and_contract() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("xyzm.csv");
        std::fs::write(
            &source,
            "id,geom\n1,\"MULTIPOLYGON ZM (((0 0 1 10,0 2 2 11,2 0 3 12,0 0 1 10)))\"\n",
        )
        .unwrap();

        let driver = CsvDriver;
        let dataset = driver
            .open(Source::Path(source), read_opts(&[("wkt_column", "geom")]))
            .unwrap();
        let contract = dataset.layers()[0].contract.clone();
        let geometry_contract = contract.geometry.as_ref().unwrap();
        assert_eq!(geometry_contract.dimensions, CoordinateDimensions::Xyzm);
        assert_eq!(
            geometry_contract.geometry_types,
            vec![GeometryType::MultiPolygon]
        );
        let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let input_geometry = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let expected = decode_wkb(input_geometry.value(0), &WkbLimits::default()).unwrap();

        let output = dir.path().join("xyzm-out.csv");
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "layer".to_owned(),
                contract,
            }],
        };
        let mut writer = driver
            .create(Sink::Path(output.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let reopened = driver
            .open(
                Source::Path(output),
                read_opts(&[("wkt_column", "geometry")]),
            )
            .unwrap();
        assert_eq!(
            reopened.layers()[0]
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .dimensions,
            CoordinateDimensions::Xyzm
        );
        let mut reader = reopened.open_layer_reader(&req(65_536)).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let output_geometry = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let actual = decode_wkb(output_geometry.value(0), &WkbLimits::default()).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn mixed_wkt_dimensions_are_declared_unknown_without_normalization() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("mixed.csv");
        std::fs::write(
            &source,
            "id,geom\n1,\"POINT Z (1 2 3)\"\n2,\"POINT M (4 5 6)\"\n",
        )
        .unwrap();
        let dataset = CsvDriver
            .open(Source::Path(source), read_opts(&[("wkt_column", "geom")]))
            .unwrap();
        assert_eq!(
            dataset.layers()[0]
                .contract
                .geometry
                .as_ref()
                .unwrap()
                .dimensions,
            CoordinateDimensions::Unknown
        );
        let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let geometries = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(
            decode_wkb(geometries.value(0), &WkbLimits::default())
                .unwrap()
                .dimensions,
            CoordinateDimensions::Xyz
        );
        assert_eq!(
            decode_wkb(geometries.value(1), &WkbLimits::default())
                .unwrap()
                .dimensions,
            CoordinateDimensions::Xym
        );
    }

    #[test]
    fn ragged_rows_are_rejected_instead_of_inventing_empty_cells() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("ragged.csv");
        std::fs::write(&source, "id,x,y\n1,12.5\n").unwrap();

        assert!(CsvDriver
            .open(
                Source::Path(source),
                read_opts(&[("x_column", "x"), ("y_column", "y")]),
            )
            .is_err());
    }

    #[test]
    fn malformed_xy_is_rejected_instead_of_becoming_null_geometry() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("invalid-xy.csv");
        std::fs::write(&source, "id,x,y\n1,not-a-number,45.0\n").unwrap();
        let dataset = CsvDriver
            .open(
                Source::Path(source),
                read_opts(&[("x_column", "x"), ("y_column", "y")]),
            )
            .unwrap();
        let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();

        assert!(reader.next_batch().is_err());
    }

    #[test]
    fn background_reader_preserves_wkb_error_variant() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("invalid-wkt-after-open.csv");
        std::fs::write(&source, "id,wkt\n1,POINT (12 45)\n").unwrap();
        let dataset = CsvDriver
            .open(
                Source::Path(source.clone()),
                read_opts(&[("wkt_column", "wkt")]),
            )
            .unwrap();

        std::fs::write(&source, "id,wkt\n1,NOT_A_GEOMETRY\n").unwrap();
        let mut reader = dataset.open_layer_reader(&req(65_536)).unwrap();

        assert!(matches!(
            reader.next_batch(),
            Err(error) if error.code == plenora_io_model::IoErrorCode::Wkb
        ));
    }
}

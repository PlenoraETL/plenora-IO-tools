//! driver-xls — XLSX ↔ RecordBatch. Foglio tabellare: la
//! geometria è dichiarata via `format_options` (`x_column`+`y_column` XY o
//! `wkt_column` XY/XYZ/XYM/XYZM), il CRS via `assume_crs` (ADR-IO 4). Foglio scelto con
//! `format_options["sheet"]` o il primo. Multi-foglio: incremento futuro.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, BufWriter, Read, Seek, Write as _};
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::builder::BinaryBuilder;
use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch, RecordBatchOptions};
use arrow_schema::{Field, Schema, SchemaRef};
use calamine::{open_workbook, Data, Reader, Xlsx, XlsxCellReader};
use rust_xlsxwriter::Workbook;
use serde_json::Value as JsonValue;

use driver_common::wkt_lossless::{format_wkt, parse_wkt};
use driver_common::{
    classify_i64, geometry_field, json_from_array, ColType, InferredColumnBuilder,
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
use plenora_io_core::publish::{create_staged_file, publish_file_atomic_limited};
use plenora_io_core::request::ReadRequest;
use plenora_io_core::{
    check_cancelled, check_cancelled_periodically, validate_write, with_write_validation,
    AttributeWriteSupport, CrsWriteSupport, FormatWriteCapabilities, NullabilitySupport,
    SingleReaderGate, TypeCoercionPolicy, WritePlan, SCALAR_TYPES, UTF8_FIELD_NAMES,
    WKB_PASSTHROUGH_GEOMETRY,
};
use plenora_io_model::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_io_model::crs::{CrsKind, ResolvedCrs};
use plenora_io_model::geometry::{is_geometry_field, with_geometry_contract_metadata};
use plenora_io_model::limits::Limits;
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{decode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
use plenora_io_model::{CancellationToken, ErrorPhase, PlenoraIoError, Result};

#[cfg(test)]
use plenora_io_model::wkb::encode_wkb;

const GEOMETRY: &str = "geometry";

fn err(reason: impl Into<String>) -> PlenoraIoError {
    PlenoraIoError::format("xls", reason)
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "xls",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::StreamingSequential,
    read_determinism: plenora_io_core::DeterminismLevel::Semantic,
    write_mode: Some(WriteMode::Buffered),
    write_determinism: Some(plenora_io_core::DeterminismLevel::Semantic),
    multi_layer: false, // primo foglio nella v1; multi-foglio futuro
    multi_file: false,
    reader_concurrency: ReaderConcurrency::SingleActiveReader,
    projection_support: plenora_io_core::ProjectionSupport::None,
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
        nullability: NullabilitySupport::FormatDefined,
        multi_layer: false,
    }),
    semantic_version: 1,
    driver_version: 5,
    descriptor_version: 6,
};

pub struct XlsDriver;

impl FormatDriver for XlsDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = source.into_path_checked(&opts.limits, &opts.cancellation)?;
        let mut wb: Xlsx<_> =
            open_workbook(&path).map_err(|e| err(format!("apertura XLSX: {e}")))?;
        check_cancelled(&opts.cancellation, ErrorPhase::Read)?;
        let sheet = opts
            .format_options
            .get("sheet")
            .cloned()
            .or_else(|| wb.sheet_names().first().cloned())
            .ok_or_else(|| err("nessun foglio nel workbook"))?;
        let crs = opts.assume_crs.clone().ok_or_else(|| {
            PlenoraIoError::Crs("XLSX con geometria richiede --assume-crs".to_owned())
        })?;
        let (layout, contract, spool) = infer_layout(
            &mut wb,
            &sheet,
            &opts.format_options,
            &crs,
            &opts.cancellation,
            &opts.limits,
        )?;
        Ok(Box::new(XlsDataset {
            layers: vec![LayerContract {
                id: LayerId(0),
                name: sheet.clone(),
                contract,
            }],
            layout,
            spool,
            reader_gate: SingleReaderGate::new(DESCRIPTOR.id),
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
            return Err(PlenoraIoError::OutputExists(path.display().to_string()));
        }
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"))
        {
            return Err(PlenoraIoError::Unsupported(
                "l'output deve avere estensione .xlsx".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraIoError::Unsupported(
                "XLSX: un solo foglio per file nella v1".to_owned(),
            ));
        }
        let xy = matches!(
            opts.format_options
                .get("geometry_encoding")
                .map(String::as_str),
            Some("xy")
        );
        with_write_validation(
            Box::new(XlsWriterState {
                path,
                durable: opts.durable,
                xy,
                batches: Vec::new(),
                wkb_limits: opts.limits.effective_wkb(),
                max_output_bytes: opts.limits.max_output_bytes,
            }),
            self.descriptor(),
            plan,
            opts.limits,
            opts.cancellation.clone(),
        )
    }
}

struct XlsDataset {
    layers: Vec<LayerContract>,
    layout: XlsxLayout,
    spool: Arc<tempfile::NamedTempFile>,
    reader_gate: SingleReaderGate,
}

impl OpenDatasetHandle for XlsDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(DESCRIPTOR.id, DESCRIPTOR.fidelity_class)
    }
    fn open_layer_reader(&self, request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        plenora_io_core::validate_read_projection(&DESCRIPTOR, request)?;
        let layout = self.layout.clone();
        let spool = Arc::clone(&self.spool);
        let layer = self.layers[0].clone();
        let cancellation = request.cancellation.clone();
        let batch_sizer = plenora_io_core::AdaptiveBatchSizer::new(
            layer.contract.schema.as_ref(),
            request.batch_target,
        );
        let reader = self.reader_gate.open(request.layer, || {
            spawn_xlsx_reader(spool, layout, batch_sizer, layer, cancellation.clone())
        })?;
        Ok(plenora_io_core::with_cancellation(
            reader,
            request.cancellation.clone(),
        ))
    }
}

#[derive(Clone, Copy)]
enum XlsxGeomSpec {
    Wkt(u32),
    Xy(u32, u32),
}

#[derive(Clone)]
struct XlsxLayout {
    attrs: Vec<(u32, ColType)>,
    schema: SchemaRef,
    data_rows: usize,
}

#[derive(Clone, Copy)]
struct SheetBounds {
    start: (u32, u32),
    end: (u32, u32),
}

// --- scrittura -------------------------------------------------------------

struct XlsWriterState {
    path: PathBuf,
    durable: bool,
    xy: bool,
    batches: Vec<RecordBatch>,
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
}

fn xls_err(e: rust_xlsxwriter::XlsxError) -> PlenoraIoError {
    err(format!("XLSX: {e}"))
}

fn write_cell(
    sheet: &mut rust_xlsxwriter::Worksheet,
    r: u32,
    c: u16,
    array: &ArrayRef,
    row: usize,
) -> Result<()> {
    match json_from_array(array, row)? {
        JsonValue::Null => {}
        JsonValue::Bool(b) => {
            sheet.write_boolean(r, c, b).map_err(xls_err)?;
        }
        JsonValue::Number(n) => {
            let value = n
                .as_f64()
                .ok_or_else(|| err(format!("numero XLSX non rappresentabile come f64: {n}")))?;
            sheet.write_number(r, c, value).map_err(xls_err)?;
        }
        JsonValue::String(s) => {
            sheet.write_string(r, c, &s).map_err(xls_err)?;
        }
        other => {
            sheet
                .write_string(r, c, other.to_string())
                .map_err(xls_err)?;
        }
    }
    Ok(())
}

impl FormatWriter for XlsWriterState {
    fn write(&mut self, batch: &RecordBatch) -> Result<()> {
        self.batches.push(batch.clone());
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<Published> {
        let mut wb = Workbook::new();
        let sheet = wb.add_worksheet();
        let limits = self.wkb_limits;
        let mut wrote_header = false;
        let mut r: u32 = 0;

        for batch in &self.batches {
            let schema = batch.schema();
            let geom_idx = schema
                .fields()
                .iter()
                .position(|f| is_geometry_field(f))
                .ok_or_else(|| err("nessuna colonna geometria"))?;
            let geom_col = batch
                .column(geom_idx)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .ok_or_else(|| err("colonna geometria non binaria"))?;

            if !wrote_header {
                let mut col: u16 = 0;
                for (i, f) in schema.fields().iter().enumerate() {
                    if i != geom_idx {
                        sheet.write_string(0, col, f.name()).map_err(xls_err)?;
                        col += 1;
                    }
                }
                if self.xy {
                    sheet.write_string(0, col, "x").map_err(xls_err)?;
                    sheet.write_string(0, col + 1, "y").map_err(xls_err)?;
                } else {
                    sheet.write_string(0, col, "geometry").map_err(xls_err)?;
                }
                wrote_header = true;
                r = 1;
            }

            for row in 0..batch.num_rows() {
                let mut col: u16 = 0;
                for (i, _) in schema.fields().iter().enumerate() {
                    if i != geom_idx {
                        write_cell(sheet, r, col, batch.column(i), row)?;
                        col += 1;
                    }
                }
                if !geom_col.is_null(row) {
                    let g = decode_wkb(geom_col.value(row), &limits)?;
                    if self.xy {
                        match &g.value {
                            WkbValue::Point(point) if g.dimensions == CoordinateDimensions::Xy => {
                                sheet.write_number(r, col, point.x).map_err(xls_err)?;
                                sheet.write_number(r, col + 1, point.y).map_err(xls_err)?;
                            }
                            _ => {
                                return Err(err(
                                    "encoding xy richiede geometrie Point strettamente XY",
                                ))
                            }
                        }
                    } else {
                        sheet
                            .write_string(r, col, format_wkt(&g)?)
                            .map_err(xls_err)?;
                    }
                }
                r += 1;
            }
        }

        let buf = wb.save_to_buffer().map_err(xls_err)?;
        let mut temp = create_staged_file(&self.path)?;
        temp.as_file_mut().write_all(&buf)?;
        temp.as_file_mut().flush()?;
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

fn data_to_string(d: &Data) -> String {
    match d {
        Data::String(s) => s.clone(),
        Data::Empty => String::new(),
        other => other.to_string(),
    }
}

fn classify_data(data: &Data) -> ObservedValueClass {
    match data {
        Data::Int(value) => classify_i64(*value),
        Data::Float(value) if value.is_finite() => ObservedValueClass::Number,
        Data::String(_) | Data::DateTimeIso(_) | Data::DurationIso(_) => ObservedValueClass::Text,
        Data::Bool(_) => ObservedValueClass::Boolean,
        _ => ObservedValueClass::Null,
    }
}

fn data_row_width(bounds: SheetBounds) -> Result<usize> {
    bounds
        .end
        .1
        .checked_sub(bounds.start.1)
        .and_then(|width| width.checked_add(1))
        .and_then(|width| usize::try_from(width).ok())
        .ok_or_else(|| err("dimensioni XLSX non valide"))
}

fn data_row_count(bounds: SheetBounds) -> Result<usize> {
    bounds
        .end
        .0
        .checked_sub(bounds.start.0)
        .and_then(|rows| usize::try_from(rows).ok())
        .ok_or_else(|| err("dimensioni XLSX non valide"))
}

fn for_each_dense_row<RS, F>(
    reader: &mut XlsxCellReader<'_, RS>,
    bounds: SheetBounds,
    cancellation: &CancellationToken,
    mut visit: F,
) -> Result<usize>
where
    RS: Read + Seek,
    F: FnMut(u32, &[Data]) -> Result<bool>,
{
    let width = data_row_width(bounds)?;
    let mut pending: Option<(u32, u32, Data)> = None;
    let mut observed_cells = 0usize;

    for (row_index, row) in (bounds.start.0..=bounds.end.0).enumerate() {
        check_cancelled_periodically(cancellation, ErrorPhase::Read, row_index)?;
        let mut values = vec![Data::Empty; width];
        loop {
            let next = if let Some(cell) = pending.take() {
                Some(cell)
            } else {
                let cell = reader
                    .next_cell()
                    .map_err(|error| err(format!("lettura celle XLSX: {error}")))?;
                if cell.is_some() {
                    observed_cells += 1;
                }
                cell.map(|cell| {
                    let (cell_row, cell_column) = cell.get_position();
                    let value: Data = cell.get_value().clone().into();
                    (cell_row, cell_column, value)
                })
            };
            let Some((cell_row, cell_column, value)) = next else {
                break;
            };
            if cell_row > row {
                pending = Some((cell_row, cell_column, value));
                break;
            }
            if cell_row < row {
                return Err(err("ordine delle celle XLSX non monotono"));
            }
            if cell_column < bounds.start.1 || cell_column > bounds.end.1 {
                return Err(err("cella XLSX fuori dalle dimensioni dichiarate"));
            }
            let offset = usize::try_from(cell_column - bounds.start.1)
                .map_err(|_| err("indice colonna XLSX non rappresentabile"))?;
            values[offset] = value;
        }
        if !visit(row, &values)? {
            break;
        }
    }
    Ok(observed_cells)
}

fn resolve_geometry(
    headers: &[String],
    start_column: u32,
    opts: &BTreeMap<String, String>,
) -> Result<(XlsxGeomSpec, BTreeSet<u32>)> {
    let index = |name: &str| {
        headers
            .iter()
            .position(|header| header == name)
            .and_then(|offset| u32::try_from(offset).ok())
            .and_then(|offset| start_column.checked_add(offset))
    };
    if let Some(wkt_name) = opts.get("wkt_column") {
        let column =
            index(wkt_name).ok_or_else(|| err(format!("colonna WKT '{wkt_name}' assente")))?;
        return Ok((XlsxGeomSpec::Wkt(column), BTreeSet::from([column])));
    }
    if let (Some(x_name), Some(y_name)) = (opts.get("x_column"), opts.get("y_column")) {
        let x_column = index(x_name).ok_or_else(|| err(format!("colonna X '{x_name}' assente")))?;
        let y_column = index(y_name).ok_or_else(|| err(format!("colonna Y '{y_name}' assente")))?;
        return Ok((
            XlsxGeomSpec::Xy(x_column, y_column),
            BTreeSet::from([x_column, y_column]),
        ));
    }
    Err(err(
        "specificare wkt_column, oppure x_column con y_column, in format_options",
    ))
}

fn cell_at(row: &[Data], bounds: SheetBounds, column: u32) -> Result<&Data> {
    let offset = column
        .checked_sub(bounds.start.1)
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or_else(|| err("indice colonna XLSX non valido"))?;
    row.get(offset)
        .ok_or_else(|| err("riga XLSX fuori dalle dimensioni dichiarate"))
}

fn encode_geometry_cell(
    row: &[Data],
    bounds: SheetBounds,
    geom: XlsxGeomSpec,
    detected_dimensions: &mut BTreeSet<CoordinateDimensions>,
    detected_types: &mut BTreeSet<GeometryType>,
    wkb_buffer: &mut Vec<u8>,
) -> Result<bool> {
    match geom {
        XlsxGeomSpec::Wkt(column) => {
            let text = data_to_string(cell_at(row, bounds, column)?);
            if text.trim().is_empty() {
                return Ok(false);
            }
            let geometry = parse_wkt(text.trim())?;
            detected_dimensions.insert(geometry.dimensions);
            detected_types.insert(geometry.geometry_type());
            wkb_buffer.clear();
            plenora_io_model::wkb::encode_wkb_into(&geometry, WkbFlavor::Iso, wkb_buffer)?;
        }
        XlsxGeomSpec::Xy(x_column, y_column) => {
            let x = coordinate_cell(Some(cell_at(row, bounds, x_column)?), "X")?;
            let y = coordinate_cell(Some(cell_at(row, bounds, y_column)?), "Y")?;
            match (x, y) {
                (Some(x), Some(y)) => {
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
                    wkb_buffer.clear();
                    plenora_io_model::wkb::encode_wkb_into(&geometry, WkbFlavor::Iso, wkb_buffer)?;
                }
                (None, None) => return Ok(false),
                _ => {
                    return Err(err(
                        "geometria XY incompleta: X e Y devono essere entrambi presenti",
                    ))
                }
            }
        }
    }
    Ok(true)
}

const SPOOL_NULL_GEOMETRY: u32 = u32::MAX;
const SPOOL_NULL: u8 = 0;
const SPOOL_INTEGER: u8 = 1;
const SPOOL_NUMBER: u8 = 2;
const SPOOL_BOOLEAN: u8 = 3;
const SPOOL_TEXT: u8 = 4;

struct BoundedSpoolWriter<'a> {
    writer: BufWriter<&'a std::fs::File>,
    bytes: u64,
    limit: u64,
}

impl<'a> BoundedSpoolWriter<'a> {
    fn new(file: &'a std::fs::File, limit: u64) -> Self {
        Self {
            writer: BufWriter::new(file),
            bytes: 0,
            limit,
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let length =
            u64::try_from(bytes.len()).map_err(|_| err("spool XLSX non rappresentabile"))?;
        let next = self
            .bytes
            .checked_add(length)
            .ok_or_else(|| err("dimensione spool XLSX fuori intervallo"))?;
        if next > self.limit {
            return Err(PlenoraIoError::LimitExceeded(format!(
                "spool XLSX: {next} byte eccedono il limite {}",
                self.limit
            )));
        }
        self.writer.write_all(bytes)?;
        self.bytes = next;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }

    fn geometry(&mut self, value: Option<&[u8]>) -> Result<()> {
        let length = match value {
            None => SPOOL_NULL_GEOMETRY,
            Some(bytes) => u32::try_from(bytes.len()).map_err(|_| {
                PlenoraIoError::LimitExceeded(
                    "geometria XLSX troppo grande per lo spool".to_owned(),
                )
            })?,
        };
        self.write(&length.to_le_bytes())?;
        if let Some(bytes) = value {
            self.write(bytes)?;
        }
        Ok(())
    }

    fn data(&mut self, value: &Data) -> Result<()> {
        match value {
            Data::Int(value) => {
                self.write(&[SPOOL_INTEGER])?;
                self.write(&value.to_le_bytes())
            }
            Data::Float(value) if value.is_finite() => {
                self.write(&[SPOOL_NUMBER])?;
                self.write(&value.to_le_bytes())
            }
            Data::Bool(value) => self.write(&[SPOOL_BOOLEAN, u8::from(*value)]),
            Data::String(value) | Data::DateTimeIso(value) | Data::DurationIso(value) => {
                let bytes = value.as_bytes();
                let length = u32::try_from(bytes.len()).map_err(|_| {
                    PlenoraIoError::LimitExceeded(
                        "testo XLSX troppo grande per lo spool".to_owned(),
                    )
                })?;
                self.write(&[SPOOL_TEXT])?;
                self.write(&length.to_le_bytes())?;
                self.write(bytes)
            }
            _ => self.write(&[SPOOL_NULL]),
        }
    }
}

fn infer_layout<RS>(
    workbook: &mut Xlsx<RS>,
    sheet: &str,
    opts: &BTreeMap<String, String>,
    crs: &str,
    cancellation: &CancellationToken,
    limits: &Limits,
) -> Result<(XlsxLayout, DataContract, Arc<tempfile::NamedTempFile>)>
where
    RS: Read + Seek,
{
    check_cancelled(cancellation, ErrorPhase::Read)?;
    let mut reader = workbook
        .worksheet_cells_reader(sheet)
        .map_err(|error| err(format!("foglio '{sheet}': {error}")))?;
    let dimensions = reader.dimensions();
    let bounds = SheetBounds {
        start: dimensions.start,
        end: dimensions.end,
    };
    let width = data_row_width(bounds)?;
    let row_count = data_row_count(bounds)?;
    if width > limits.max_columns {
        return Err(PlenoraIoError::LimitExceeded(format!(
            "XLSX: {width} colonne eccedono il limite {}",
            limits.max_columns
        )));
    }
    if row_count > limits.max_rows {
        return Err(PlenoraIoError::LimitExceeded(format!(
            "XLSX: {row_count} righe eccedono il limite {}",
            limits.max_rows
        )));
    }

    let mut headers: Option<Vec<String>> = None;
    let mut geom = None;
    let mut geom_columns = BTreeSet::new();
    let mut accumulators: Vec<TypeAccumulator> = Vec::new();
    let mut detected_dimensions = BTreeSet::new();
    let mut detected_types = BTreeSet::new();
    let spool = Arc::new(tempfile::NamedTempFile::new()?);
    let mut spool_writer = BoundedSpoolWriter::new(spool.as_file(), limits.max_input_bytes);
    let mut wkb_buffer = Vec::new();
    let observed_cells =
        for_each_dense_row(&mut reader, bounds, cancellation, |row_index, row| {
            if row_index == bounds.start.0 {
                let row_headers: Vec<String> = row.iter().map(data_to_string).collect();
                let (resolved_geom, resolved_columns) =
                    resolve_geometry(&row_headers, bounds.start.1, opts)?;
                accumulators = vec![TypeAccumulator::default(); width - resolved_columns.len()];
                geom = Some(resolved_geom);
                geom_columns = resolved_columns;
                headers = Some(row_headers);
                return Ok(true);
            }
            let resolved_geom = geom.ok_or_else(|| err("intestazione XLSX assente"))?;
            let has_geometry = encode_geometry_cell(
                row,
                bounds,
                resolved_geom,
                &mut detected_dimensions,
                &mut detected_types,
                &mut wkb_buffer,
            )?;
            spool_writer.geometry(has_geometry.then_some(wkb_buffer.as_slice()))?;
            let mut attribute_index = 0usize;
            for (offset, data) in row.iter().enumerate() {
                let column = bounds
                    .start
                    .1
                    .checked_add(u32::try_from(offset).map_err(|_| err("troppe colonne XLSX"))?)
                    .ok_or_else(|| err("indice colonna XLSX fuori intervallo"))?;
                if geom_columns.contains(&column) {
                    continue;
                }
                accumulators[attribute_index].observe(classify_data(data));
                spool_writer.data(data)?;
                attribute_index += 1;
            }
            Ok(true)
        })?;
    spool_writer.finish()?;
    if observed_cells == 0 {
        return Err(err("foglio vuoto"));
    }
    let headers = headers.ok_or_else(|| err("intestazione XLSX assente"))?;
    let geom = geom.ok_or_else(|| err("geometria XLSX non configurata"))?;

    if matches!(geom, XlsxGeomSpec::Xy(_, _)) {
        detected_dimensions.insert(CoordinateDimensions::Xy);
        detected_types.insert(GeometryType::Point);
    }
    let dimensions = if detected_dimensions.len() == 1 {
        detected_dimensions
            .iter()
            .next()
            .copied()
            .unwrap_or(CoordinateDimensions::Unknown)
    } else {
        CoordinateDimensions::Unknown
    };
    let kind = if crs == "OGC:CRS84" || crs == "EPSG:4326" {
        CrsKind::Geographic
    } else {
        CrsKind::Unknown
    };
    let mut geometry_contract = GeometryColumnContract::wkb_xy(
        FieldId(0),
        GEOMETRY,
        ResolvedCrs::new(Some(crs.to_owned()), kind, None),
        true,
    );
    geometry_contract.dimensions = dimensions;
    geometry_contract.set_exact_geometry_types(detected_types.into_iter().collect());
    geometry_contract.native_metadata.insert(
        "xlsx.geometry_encoding".to_owned(),
        if matches!(geom, XlsxGeomSpec::Wkt(_)) {
            "wkt"
        } else {
            "xy_columns"
        }
        .to_owned(),
    );
    let mut fields = vec![with_geometry_contract_metadata(
        &geometry_field(GEOMETRY, crs),
        &geometry_contract,
    )];
    let mut attrs = Vec::with_capacity(accumulators.len());
    let mut attribute_index = 0usize;
    for (offset, name) in headers.iter().enumerate() {
        let column = bounds
            .start
            .1
            .checked_add(u32::try_from(offset).map_err(|_| err("troppe colonne XLSX"))?)
            .ok_or_else(|| err("indice colonna XLSX fuori intervallo"))?;
        if geom_columns.contains(&column) {
            continue;
        }
        let column_type = accumulators[attribute_index].column_type();
        fields.push(Field::new(name, column_type.arrow_data_type(), true));
        attrs.push((column, column_type));
        attribute_index += 1;
    }

    let schema: SchemaRef = Arc::new(Schema::new(fields));
    let contract = DataContract::new(schema.clone(), Some(geometry_contract));
    Ok((
        XlsxLayout {
            attrs,
            schema,
            data_rows: row_count,
        },
        contract,
        spool,
    ))
}

fn finish_read_batch(
    schema: &SchemaRef,
    geometry: &mut BinaryBuilder,
    attributes: &mut [InferredColumnBuilder],
    row_count: usize,
) -> Result<RecordBatch> {
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(1 + attributes.len());
    arrays.push(Arc::new(geometry.finish()));
    for builder in attributes {
        arrays.push(builder.finish());
    }
    let options = RecordBatchOptions::new().with_row_count(Some(row_count));
    RecordBatch::try_new_with_options(schema.clone(), arrays, &options)
        .map_err(|error| err(format!("batch XLSX: {error}")))
}

fn read_spool_exact(reader: &mut impl Read, bytes: &mut [u8]) -> Result<()> {
    reader
        .read_exact(bytes)
        .map_err(|error| err(format!("spool XLSX troncato o illeggibile: {error}")))
}

fn read_spool_geometry(
    reader: &mut impl Read,
    builder: &mut BinaryBuilder,
    buffer: &mut Vec<u8>,
) -> Result<()> {
    let mut length_bytes = [0u8; 4];
    read_spool_exact(reader, &mut length_bytes)?;
    let length = u32::from_le_bytes(length_bytes);
    if length == SPOOL_NULL_GEOMETRY {
        builder.append_null();
        return Ok(());
    }
    let length =
        usize::try_from(length).map_err(|_| err("lunghezza geometria spool non valida"))?;
    buffer.resize(length, 0);
    read_spool_exact(reader, buffer)?;
    builder.append_value(buffer.as_slice());
    Ok(())
}

fn read_spool_data(
    reader: &mut impl Read,
    builder: &mut InferredColumnBuilder,
    buffer: &mut Vec<u8>,
) -> Result<()> {
    let mut tag = [0u8; 1];
    read_spool_exact(reader, &mut tag)?;
    match tag[0] {
        SPOOL_NULL => {
            builder.append_null();
            Ok(())
        }
        SPOOL_INTEGER => {
            let mut bytes = [0u8; 8];
            read_spool_exact(reader, &mut bytes)?;
            builder.append_i64(i64::from_le_bytes(bytes))
        }
        SPOOL_NUMBER => {
            let mut bytes = [0u8; 8];
            read_spool_exact(reader, &mut bytes)?;
            builder.append_f64(f64::from_le_bytes(bytes))
        }
        SPOOL_BOOLEAN => {
            let mut value = [0u8; 1];
            read_spool_exact(reader, &mut value)?;
            match value[0] {
                0 => builder.append_bool(false),
                1 => builder.append_bool(true),
                _ => Err(err("booleano spool XLSX non valido")),
            }
        }
        SPOOL_TEXT => {
            let mut length = [0u8; 4];
            read_spool_exact(reader, &mut length)?;
            let length = usize::try_from(u32::from_le_bytes(length))
                .map_err(|_| err("lunghezza testo spool non valida"))?;
            buffer.resize(length, 0);
            read_spool_exact(reader, buffer)?;
            let text =
                std::str::from_utf8(buffer).map_err(|_| err("testo spool XLSX non UTF-8"))?;
            builder.append_str(text)
        }
        _ => Err(err("tag spool XLSX non valido")),
    }
}

fn spawn_xlsx_reader(
    spool: Arc<tempfile::NamedTempFile>,
    layout: XlsxLayout,
    mut batch_sizer: plenora_io_core::AdaptiveBatchSizer,
    layer: LayerContract,
    cancellation: CancellationToken,
) -> Result<Box<dyn LayerReader>> {
    spawn_batch_reader(DESCRIPTOR.id, layer, 2, move |emitter: BatchEmitter| {
        let file = spool.reopen()?;
        let mut reader = BufReader::new(file);
        let mut geometry = BinaryBuilder::new();
        let mut attributes: Vec<InferredColumnBuilder> = layout
            .attrs
            .iter()
            .map(|(_, column_type)| InferredColumnBuilder::new(*column_type))
            .collect();
        let mut geometry_buffer = Vec::new();
        let mut text_buffer = Vec::new();
        let mut rows_in_batch = 0usize;
        for row_index in 0..layout.data_rows {
            check_cancelled_periodically(&cancellation, ErrorPhase::Read, row_index)?;
            read_spool_geometry(&mut reader, &mut geometry, &mut geometry_buffer)?;
            for builder in &mut attributes {
                read_spool_data(&mut reader, builder, &mut text_buffer)?;
            }
            rows_in_batch += 1;
            if rows_in_batch >= batch_sizer.rows() {
                let batch = finish_read_batch(
                    &layout.schema,
                    &mut geometry,
                    &mut attributes,
                    rows_in_batch,
                )?;
                batch_sizer.observe(&batch);
                rows_in_batch = 0;
                if !emitter.send(batch) {
                    return Ok(());
                }
            }
        }
        if rows_in_batch > 0 {
            let batch = finish_read_batch(
                &layout.schema,
                &mut geometry,
                &mut attributes,
                rows_in_batch,
            )?;
            if !emitter.send(batch) {
                return Ok(());
            }
        }
        Ok(())
    })
}

fn coordinate_cell(cell: Option<&Data>, axis: &'static str) -> Result<Option<f64>> {
    const MAX_EXACT_F64_INTEGER: i64 = 1_i64 << 53;

    let value = match cell {
        None | Some(Data::Empty) => return Ok(None),
        Some(Data::Float(value)) if value.is_finite() => *value,
        Some(Data::Int(value))
            if *value >= -MAX_EXACT_F64_INTEGER && *value <= MAX_EXACT_F64_INTEGER =>
        {
            *value as f64
        }
        Some(Data::String(value)) if value.trim().is_empty() => return Ok(None),
        Some(Data::String(value)) => value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| err(format!("coordinata {axis} non numerica o non finita")))?,
        Some(_) => {
            return Err(err(format!(
                "coordinata {axis} non numerica, non finita o non rappresentabile senza perdita"
            )))
        }
    };
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

    #[test]
    fn coordinate_cells_fail_closed_on_invalid_or_lossy_values() {
        assert!(coordinate_cell(Some(&Data::String("not-a-number".to_owned())), "X").is_err());
        assert!(coordinate_cell(Some(&Data::Float(f64::INFINITY)), "X").is_err());
        assert!(coordinate_cell(Some(&Data::Int((1_i64 << 53) + 1)), "X").is_err());
        assert_eq!(coordinate_cell(Some(&Data::Empty), "X").unwrap(), None);
        assert_eq!(
            coordinate_cell(Some(&Data::String(" 12.5 ".to_owned())), "X").unwrap(),
            Some(12.5)
        );
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        let wkb = encode_wkb(&parse_wkt("POINT (12.5 45.9)").unwrap(), WkbFlavor::Iso).unwrap();
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            geometry_field(GEOMETRY, "EPSG:4326"),
            Field::new("nome", arrow_schema::DataType::Utf8, true),
            Field::new("pop", arrow_schema::DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(wkb.as_slice())])),
                Arc::new(arrow_array::StringArray::from(vec!["Roma"])),
                Arc::new(arrow_array::Int64Array::from(vec![2_800_000i64])),
            ],
        )
        .unwrap();

        let driver = XlsDriver;
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "l".to_owned(),
                contract: DataContract {
                    schema: schema.clone(),
                    geometry: None,
                },
            }],
        };
        let mut w = driver
            .create(Sink::Path(out.clone()), &plan, &WriteOptions::default())
            .unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();

        let ropts = ReadOptions {
            assume_crs: Some("EPSG:4326".to_owned()),
            format_options: [("wkt_column".to_owned(), "geometry".to_owned())]
                .into_iter()
                .collect(),
            ..ReadOptions::default()
        };
        let ds = driver.open(Source::Path(out), &ropts).unwrap();
        let mut r = ds
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
                cancellation: Default::default(),
            })
            .unwrap();
        let rb = r.next_batch().unwrap().unwrap();
        assert_eq!(rb.num_rows(), 1);
        let nome = rb
            .column_by_name("nome")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        assert_eq!(nome.value(0), "Roma");
    }

    #[test]
    fn xlsx_reader_emits_bounded_batches_and_preserves_sparse_rows() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("sparse.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "name").unwrap();
        sheet.write_string(0, 1, "geometry").unwrap();
        sheet.write_string(1, 0, "first").unwrap();
        sheet.write_string(1, 1, "POINT (1 2)").unwrap();
        sheet.write_string(3, 0, "third").unwrap();
        sheet.write_string(3, 1, "POINT (3 4)").unwrap();
        workbook.save(&output).unwrap();

        let driver = XlsDriver;
        let dataset = driver
            .open(
                Source::Path(output),
                &ReadOptions {
                    assume_crs: Some("EPSG:4326".to_owned()),
                    format_options: [("wkt_column".to_owned(), "geometry".to_owned())]
                        .into_iter()
                        .collect(),
                    ..ReadOptions::default()
                },
            )
            .unwrap();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget {
                    target_bytes: 1024,
                    max_rows: 1,
                },
                cancellation: Default::default(),
            })
            .unwrap();

        let first = reader.next_batch().unwrap().unwrap();
        let empty = reader.next_batch().unwrap().unwrap();
        let third = reader.next_batch().unwrap().unwrap();
        assert_eq!(
            [first.num_rows(), empty.num_rows(), third.num_rows()],
            [1, 1, 1]
        );
        assert!(empty
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .is_null(0));
        assert!(empty
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap()
            .is_null(0));
        assert!(reader.next_batch().unwrap().is_none());
    }

    #[test]
    fn xlsx_reader_stops_after_cancellation_between_batches() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("cancel.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "geometry").unwrap();
        for row in 1..=4 {
            sheet
                .write_string(row, 0, format!("POINT ({row} {row})"))
                .unwrap();
        }
        workbook.save(&output).unwrap();

        let driver = XlsDriver;
        let dataset = driver
            .open(
                Source::Path(output),
                &ReadOptions {
                    assume_crs: Some("EPSG:4326".to_owned()),
                    format_options: [("wkt_column".to_owned(), "geometry".to_owned())]
                        .into_iter()
                        .collect(),
                    ..ReadOptions::default()
                },
            )
            .unwrap();
        let cancellation = CancellationToken::new();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget {
                    target_bytes: 1024,
                    max_rows: 1,
                },
                cancellation: cancellation.clone(),
            })
            .unwrap();
        assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 1);
        cancellation.cancel();
        assert!(reader.next_batch().is_err());
    }

    #[test]
    fn xlsx_spool_is_bounded_by_the_input_byte_limit() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("bounded.xlsx");
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "name").unwrap();
        sheet.write_string(0, 1, "geometry").unwrap();
        let repeated = "x".repeat(1_000);
        for row in 1..=100 {
            sheet.write_string(row, 0, &repeated).unwrap();
            sheet.write_string(row, 1, "POINT (1 2)").unwrap();
        }
        workbook.save(&output).unwrap();
        let input_bytes = std::fs::metadata(&output).unwrap().len();

        let result = XlsDriver.open(
            Source::Path(output),
            &ReadOptions {
                assume_crs: Some("EPSG:4326".to_owned()),
                format_options: [("wkt_column".to_owned(), "geometry".to_owned())]
                    .into_iter()
                    .collect(),
                limits: Limits {
                    max_input_bytes: input_bytes,
                    ..Limits::default()
                },
                ..ReadOptions::default()
            },
        );
        let error = result.err().expect("lo spool deve rispettare il limite");
        assert_eq!(error.code, plenora_io_model::IoErrorCode::LimitExceeded);
    }

    #[test]
    fn xlsx_wkt_xym_round_trip_preserves_payload_and_contract() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("xym.xlsx");
        let expected = parse_wkt("MULTILINESTRING M ((0 0 5,1 1 6))").unwrap();
        let bytes = encode_wkb(&expected, WkbFlavor::Iso).unwrap();
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), false);
        geometry_contract.dimensions = CoordinateDimensions::Xym;
        geometry_contract.set_exact_geometry_types(vec![GeometryType::MultiLineString]);
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            with_geometry_contract_metadata(
                &geometry_field(GEOMETRY, "EPSG:4326"),
                &geometry_contract,
            ),
            Field::new("id", arrow_schema::DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![Some(bytes.as_slice())])),
                Arc::new(arrow_array::Int64Array::from(vec![1_i64])),
            ],
        )
        .unwrap();
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "layer".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: Some(geometry_contract),
                },
            }],
        };

        let driver = XlsDriver;
        let mut writer = driver
            .create(Sink::Path(output.clone()), &plan, &WriteOptions::default())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();

        let read_options = ReadOptions {
            assume_crs: Some("EPSG:4326".to_owned()),
            format_options: [("wkt_column".to_owned(), "geometry".to_owned())]
                .into_iter()
                .collect(),
            ..ReadOptions::default()
        };
        let dataset = driver.open(Source::Path(output), &read_options).unwrap();
        let output_contract = dataset.layers()[0].contract.geometry.as_ref().unwrap();
        assert_eq!(output_contract.dimensions, CoordinateDimensions::Xym);
        assert_eq!(
            output_contract.geometry_types,
            vec![GeometryType::MultiLineString]
        );
        assert_eq!(
            output_contract
                .native_metadata
                .get("xlsx.geometry_encoding")
                .map(String::as_str),
            Some("wkt")
        );
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                batch_target: BatchTarget::default(),
                cancellation: Default::default(),
            })
            .unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let geometry = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let actual = decode_wkb(geometry.value(0), &WkbLimits::default()).unwrap();
        assert_eq!(actual, expected);
    }
}

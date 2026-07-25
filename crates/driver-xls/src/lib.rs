//! driver-xls — XLSX → RecordBatch (Fase 1, read-only). Foglio tabellare: la
//! geometria è dichiarata via `format_options` (`x_column`+`y_column` o
//! `wkt_column`), il CRS via `assume_crs` (ADR-IO 4). Foglio scelto con
//! `format_options["sheet"]` o il primo. Multi-foglio e scrittura: incrementi.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch};
use arrow_schema::{Field, Schema, SchemaRef};
use calamine::{open_workbook, Data, Reader, Xlsx};
use rust_xlsxwriter::Workbook;
use serde_json::Value as JsonValue;
use wkt::{ToWkt, TryFromWkt};

use driver_common::{build_property_array, geometry_field, infer_column, json_from_array};
use plenora_core::contract::{DataContract, FieldId, GeometryColumnContract, LayerContract, LayerId};
use plenora_core::crs::{CrsKind, ResolvedCrs};
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
        driver: "xls",
        reason: reason.into(),
    }
}

static DESCRIPTOR: FormatDescriptor = FormatDescriptor {
    id: "xls",
    direction: Direction::Bidirectional,
    read_mode: ReadMode::Materializing,
    write_mode: Some(WriteMode::Buffered),
    multi_layer: false, // primo foglio nella v1; multi-foglio futuro
    multi_file: false,
    reader_concurrency: ReaderConcurrency::SingleActiveReader,
    crs_handling: CrsHandling::None,
    fidelity_class: Fidelity::Conditional,
    runtime: Runtime::PureRust,
    semantic_version: 1,
    driver_version: 1,
    descriptor_version: 1,
};

pub struct XlsDriver;

impl FormatDriver for XlsDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let Source::Path(path) = source;
        let mut wb: Xlsx<_> =
            open_workbook(&path).map_err(|e| err(format!("apertura XLSX: {e}")))?;
        let sheet = opts
            .format_options
            .get("sheet")
            .cloned()
            .or_else(|| wb.sheet_names().first().cloned())
            .ok_or_else(|| err("nessun foglio nel workbook"))?;
        let range = wb
            .worksheet_range(&sheet)
            .map_err(|e| err(format!("foglio '{sheet}': {e}")))?;
        let crs = opts.assume_crs.clone().ok_or_else(|| {
            PlenoraError::Crs("XLSX con geometria richiede --assume-crs".to_owned())
        })?;
        let (batch, contract) = build_batch(&range, &opts.format_options, &crs)?;
        Ok(Box::new(XlsDataset {
            layers: vec![LayerContract {
                id: LayerId(0),
                name: sheet,
                contract,
            }],
            batch,
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
            .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"))
        {
            return Err(PlenoraError::Unsupported(
                "l'output deve avere estensione .xlsx".to_owned(),
            ));
        }
        if plan.layers.len() != 1 {
            return Err(PlenoraError::Unsupported(
                "XLSX: un solo foglio per file nella v1".to_owned(),
            ));
        }
        let xy = matches!(
            opts.format_options.get("geometry_encoding").map(String::as_str),
            Some("xy")
        );
        Ok(Box::new(XlsWriterState {
            path,
            durable: opts.durable,
            xy,
            batches: Vec::new(),
        }))
    }
}

struct XlsDataset {
    layers: Vec<LayerContract>,
    batch: RecordBatch,
}

impl OpenDatasetHandle for XlsDataset {
    fn layers(&self) -> &[LayerContract] {
        &self.layers
    }
    fn open_layer_reader(&self, _request: &ReadRequest) -> Result<Box<dyn LayerReader>> {
        Ok(Box::new(XlsReader {
            batch: Some(self.batch.clone()),
            layer: self.layers[0].clone(),
        }))
    }
}

struct XlsReader {
    batch: Option<RecordBatch>,
    layer: LayerContract,
}

impl LayerReader for XlsReader {
    fn contract(&self) -> &LayerContract {
        &self.layer
    }
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        Ok(self.batch.take())
    }
}

// --- scrittura -------------------------------------------------------------

struct XlsWriterState {
    path: PathBuf,
    durable: bool,
    xy: bool,
    batches: Vec<RecordBatch>,
}

fn xls_err(e: rust_xlsxwriter::XlsxError) -> PlenoraError {
    err(format!("XLSX: {e}"))
}

fn write_cell(
    sheet: &mut rust_xlsxwriter::Worksheet,
    r: u32,
    c: u16,
    array: &ArrayRef,
    row: usize,
) -> Result<()> {
    match json_from_array(array, row) {
        JsonValue::Null => {}
        JsonValue::Bool(b) => {
            sheet.write_boolean(r, c, b).map_err(xls_err)?;
        }
        JsonValue::Number(n) => {
            sheet.write_number(r, c, n.as_f64().unwrap_or(0.0)).map_err(xls_err)?;
        }
        JsonValue::String(s) => {
            sheet.write_string(r, c, &s).map_err(xls_err)?;
        }
        other => {
            sheet.write_string(r, c, other.to_string()).map_err(xls_err)?;
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
        let limits = WkbLimits::default();
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
                    let g = from_wkb(geom_col.value(row), &limits)?;
                    if self.xy {
                        match g {
                            geo_types::Geometry::Point(p) => {
                                sheet.write_number(r, col, p.x()).map_err(xls_err)?;
                                sheet.write_number(r, col + 1, p.y()).map_err(xls_err)?;
                            }
                            _ => return Err(err("encoding xy richiede geometrie Point")),
                        }
                    } else {
                        sheet.write_string(r, col, g.wkt_string()).map_err(xls_err)?;
                    }
                }
                r += 1;
            }
        }

        let buf = wb.save_to_buffer().map_err(xls_err)?;
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut temp = tempfile::NamedTempFile::new_in(&parent)?;
        temp.as_file_mut().write_all(&buf)?;
        temp.as_file_mut().flush()?;
        let (bytes, outcome) = publish_file_atomic(temp, &self.path, self.durable)?;
        Ok(Published {
            bytes,
            loss: LossReport::default(),
            outcome,
        })
    }
}

fn data_to_json(d: &Data) -> JsonValue {
    match d {
        Data::Int(i) => JsonValue::from(*i),
        Data::Float(f) => JsonValue::from(*f),
        Data::String(s) => JsonValue::String(s.clone()),
        Data::Bool(b) => JsonValue::Bool(*b),
        Data::DateTimeIso(s) | Data::DurationIso(s) => JsonValue::String(s.clone()),
        _ => JsonValue::Null,
    }
}

fn data_to_string(d: &Data) -> String {
    match d {
        Data::String(s) => s.clone(),
        Data::Empty => String::new(),
        other => other.to_string(),
    }
}

fn build_batch(
    range: &calamine::Range<Data>,
    opts: &BTreeMap<String, String>,
    crs: &str,
) -> Result<(RecordBatch, DataContract)> {
    let mut rows = range.rows();
    let header = rows.next().ok_or_else(|| err("foglio vuoto"))?;
    let headers: Vec<String> = header.iter().map(data_to_string).collect();
    let idx = |name: &str| headers.iter().position(|h| h == name);
    let data_rows: Vec<&[Data]> = rows.collect();

    let (geom_cols, wkb): (Vec<usize>, Vec<Option<Vec<u8>>>) =
        if let Some(w) = opts.get("wkt_column") {
            let wi = idx(w).ok_or_else(|| err(format!("colonna WKT '{w}' assente")))?;
            let mut out = Vec::new();
            for row in &data_rows {
                let cell = row.get(wi).map(data_to_string).unwrap_or_default();
                if cell.trim().is_empty() {
                    out.push(None);
                } else {
                    let geom = geo_types::Geometry::<f64>::try_from_wkt_str(cell.trim())
                        .map_err(|e| err(format!("WKT non valido: {e}")))?;
                    out.push(Some(to_wkb(&geom)?));
                }
            }
            (vec![wi], out)
        } else if let (Some(x), Some(y)) = (opts.get("x_column"), opts.get("y_column")) {
            let xi = idx(x).ok_or_else(|| err(format!("colonna X '{x}' assente")))?;
            let yi = idx(y).ok_or_else(|| err(format!("colonna Y '{y}' assente")))?;
            let mut out = Vec::new();
            for row in &data_rows {
                let xv = row.get(xi).and_then(cell_f64);
                let yv = row.get(yi).and_then(cell_f64);
                match (xv, yv) {
                    (Some(x), Some(y)) => out.push(Some(to_wkb(&geo_types::Geometry::Point(
                        geo_types::Point::new(x, y),
                    ))?)),
                    _ => out.push(None),
                }
            }
            (vec![xi, yi], out)
        } else {
            return Err(err(
                "specificare wkt_column, oppure x_column con y_column, in format_options",
            ));
        };

    let mut fields = vec![geometry_field(GEOMETRY, crs)];
    let mut arrays: Vec<ArrayRef> = vec![Arc::new(BinaryArray::from(
        wkb.iter().map(|o| o.as_deref()).collect::<Vec<_>>(),
    ))];
    for (ci, name) in headers.iter().enumerate() {
        if geom_cols.contains(&ci) {
            continue;
        }
        let values: Vec<Option<JsonValue>> = data_rows
            .iter()
            .map(|r| Some(r.get(ci).map(data_to_json).unwrap_or(JsonValue::Null)))
            .collect();
        let col = infer_column(values.iter().filter_map(|v| v.as_ref()));
        let (dt, arr) = build_property_array(col, &values);
        fields.push(Field::new(name, dt, true));
        arrays.push(arr);
    }

    let schema: SchemaRef = Arc::new(Schema::new(fields));
    let batch =
        RecordBatch::try_new(schema.clone(), arrays).map_err(|e| err(format!("batch: {e}")))?;
    let kind = if crs == "OGC:CRS84" || crs == "EPSG:4326" {
        CrsKind::Geographic
    } else {
        CrsKind::Unknown
    };
    let contract = DataContract {
        schema,
        geometry: Some(GeometryColumnContract {
            field_id: FieldId(0),
            name: GEOMETRY.to_owned(),
            crs: ResolvedCrs {
                id: Some(crs.to_owned()),
                kind,
                definition: None,
            },
            nullable: true,
        }),
    };
    Ok((batch, contract))
}

fn cell_f64(d: &Data) -> Option<f64> {
    match d {
        Data::Float(f) => Some(*f),
        Data::Int(i) => Some(*i as f64),
        Data::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plenora_io_core::request::{BatchTarget, ProjectionMode};
    use plenora_io_core::WriteLayer;

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.xlsx");
        let wkb = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(12.5, 45.9))).unwrap();
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
}

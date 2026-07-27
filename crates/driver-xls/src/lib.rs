//! driver-xls — XLSX → RecordBatch (Fase 1, read-only). Foglio tabellare: la
//! geometria è dichiarata via `format_options` (`x_column`+`y_column` XY o
//! `wkt_column` XY/XYZ/XYM/XYZM), il CRS via `assume_crs` (ADR-IO 4). Foglio scelto con
//! `format_options["sheet"]` o il primo. Multi-foglio e scrittura: incrementi.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, RecordBatch};
use arrow_schema::{Field, Schema, SchemaRef};
use calamine::{open_workbook, Data, Reader, Xlsx};
use rust_xlsxwriter::Workbook;
use serde_json::Value as JsonValue;

use driver_common::wkt_lossless::{format_wkt, parse_wkt};
use driver_common::{build_property_array, geometry_field, infer_column, json_from_array};
use plenora_core::contract::{
    CoordinateDimensions, DataContract, FieldId, GeometryColumnContract, GeometryType,
    LayerContract, LayerId,
};
use plenora_core::crs::{CrsKind, ResolvedCrs};
use plenora_core::geometry::{is_geometry_field, with_geometry_contract_metadata};
use plenora_core::limits::WkbLimits;
use plenora_core::wkb::{decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue};
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
    UTF8_FIELD_NAMES, WKB_PASSTHROUGH_GEOMETRY,
};

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
    driver_version: 2,
    descriptor_version: 2,
};

pub struct XlsDriver;

impl FormatDriver for XlsDriver {
    fn descriptor(&self) -> &FormatDescriptor {
        &DESCRIPTOR
    }

    fn open(&self, source: Source, opts: &ReadOptions) -> Result<Box<dyn OpenDatasetHandle>> {
        let path = source.into_path_checked(&opts.limits)?;
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
        validate_write(self.descriptor(), plan, &opts.limits)?;
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
            opts.format_options
                .get("geometry_encoding")
                .map(String::as_str),
            Some("xy")
        );
        Ok(with_write_validation(
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
        ))
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
    fn fidelity_assessment(&self) -> plenora_io_core::FidelityAssessment {
        plenora_io_core::FidelityAssessment::for_format(DESCRIPTOR.id, DESCRIPTOR.fidelity_class)
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
    wkb_limits: WkbLimits,
    max_output_bytes: u64,
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
            sheet
                .write_number(r, c, n.as_f64().unwrap_or(0.0))
                .map_err(xls_err)?;
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
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut temp = tempfile::NamedTempFile::new_in(&parent)?;
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
    let mut detected_dimensions = BTreeSet::new();
    let mut detected_types = BTreeSet::new();

    let (geom_cols, wkb): (Vec<usize>, Vec<Option<Vec<u8>>>) =
        if let Some(w) = opts.get("wkt_column") {
            let wi = idx(w).ok_or_else(|| err(format!("colonna WKT '{w}' assente")))?;
            let mut out = Vec::new();
            for row in &data_rows {
                let cell = row.get(wi).map(data_to_string).unwrap_or_default();
                if cell.trim().is_empty() {
                    out.push(None);
                } else {
                    let geometry = parse_wkt(cell.trim())?;
                    detected_dimensions.insert(geometry.dimensions);
                    detected_types.insert(geometry.geometry_type());
                    out.push(Some(encode_wkb(&geometry, WkbFlavor::Iso)?));
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
                        out.push(Some(encode_wkb(&geometry, WkbFlavor::Iso)?));
                    }
                    _ => out.push(None),
                }
            }
            detected_dimensions.insert(CoordinateDimensions::Xy);
            detected_types.insert(GeometryType::Point);
            (vec![xi, yi], out)
        } else {
            return Err(err(
                "specificare wkt_column, oppure x_column con y_column, in format_options",
            ));
        };

    let dimensions = if detected_dimensions.len() == 1 {
        *detected_dimensions.iter().next().expect("una dimensione")
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
        ResolvedCrs {
            id: Some(crs.to_owned()),
            kind,
            definition: None,
        },
        true,
    );
    geometry_contract.dimensions = dimensions;
    geometry_contract.geometry_types = detected_types.into_iter().collect();
    let native_encoding = if opts.contains_key("wkt_column") {
        "wkt"
    } else {
        "xy_columns"
    };
    geometry_contract.native_metadata.insert(
        "xlsx.geometry_encoding".to_owned(),
        native_encoding.to_owned(),
    );
    let mut fields = vec![with_geometry_contract_metadata(
        &geometry_field(GEOMETRY, crs),
        &geometry_contract,
    )];
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
    let contract = DataContract {
        schema,
        geometry: Some(geometry_contract),
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
    fn xlsx_wkt_xym_round_trip_preserves_payload_and_contract() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("xym.xlsx");
        let expected = parse_wkt("MULTILINESTRING M ((0 0 5,1 1 6))").unwrap();
        let bytes = encode_wkb(&expected, WkbFlavor::Iso).unwrap();
        let mut geometry_contract =
            GeometryColumnContract::wkb_xy(FieldId(0), GEOMETRY, ResolvedCrs::wgs84(), false);
        geometry_contract.dimensions = CoordinateDimensions::Xym;
        geometry_contract.geometry_types = vec![GeometryType::MultiLineString];
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

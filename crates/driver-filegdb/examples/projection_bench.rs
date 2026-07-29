//! Benchmark ripetibile del percorso di lettura FileGDB.
//!
//! Non fa parte della libreria: genera una fixture larga una volta e misura
//! separatamente proiezione completa e proiezione stretta.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use arrow_array::{Array, ArrayRef, BinaryArray, Int32Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use driver_common::geometry_field;
use plenora_io_core::driver::{FormatDriver, ReadOptions, Sink, Source, WriteOptions};
use plenora_io_core::request::{BatchTarget, ProjectionMode, ReadRequest};
use plenora_io_core::{WriteLayer, WritePlan};
use plenora_io_model::contract::{
    DataContract, FieldId, GeometryColumnContract, GeometryType, LayerId,
};
use plenora_io_model::crs::{CrsKind, ResolvedCrs};

const CHUNK_ROWS: usize = 4_096;

fn point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(21);
    bytes.push(1);
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes
}

fn schema(field_count: usize) -> SchemaRef {
    let mut fields = Vec::with_capacity(field_count + 1);
    fields.push(geometry_field("geometry", "EPSG:3857"));
    fields.extend(
        (0..field_count)
            .map(|index| Field::new(format!("field_{index:03}"), DataType::Int32, false)),
    );
    Arc::new(Schema::new(fields))
}

fn plan(field_count: usize) -> WritePlan {
    let schema = schema(field_count);
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
        false,
    );
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    WritePlan {
        layers: vec![WriteLayer {
            name: "wide".to_owned(),
            contract: DataContract {
                schema,
                geometry: Some(geometry),
            },
        }],
    }
}

fn batch(schema: SchemaRef, field_count: usize, first_row: usize, rows: usize) -> RecordBatch {
    let geometry = point_wkb(1_000.0, 2_000.0);
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(field_count + 1);
    columns.push(Arc::new(BinaryArray::from(
        (0..rows)
            .map(|_| Some(geometry.as_slice()))
            .collect::<Vec<_>>(),
    )));
    columns.extend((0..field_count).map(|field_index| {
        let values = (0..rows)
            .map(|row| (first_row + row + field_index) as i32)
            .collect::<Vec<_>>();
        Arc::new(Int32Array::from(values)) as ArrayRef
    }));
    RecordBatch::try_new(schema, columns).expect("fixture batch coerente")
}

fn generate(path: PathBuf, rows: usize, field_count: usize) {
    assert!(!path.exists(), "la fixture esiste già: {}", path.display());
    let driver = driver_filegdb::FileGdbDriver;
    let plan = plan(field_count);
    let mut writer = driver
        .create(Sink::Path(path.clone()), &plan, &WriteOptions::default())
        .expect("creazione FileGDB");
    let schema = plan.layers[0].contract.schema.clone();
    let mut first_row = 0;
    while first_row < rows {
        let count = CHUNK_ROWS.min(rows - first_row);
        writer
            .write(&batch(schema.clone(), field_count, first_row, count))
            .expect("scrittura batch");
        first_row += count;
    }
    writer.finish().expect("pubblicazione FileGDB");
    println!(
        "generated path={} rows={} fields={}",
        path.display(),
        rows,
        field_count
    );
}

fn read(path: PathBuf, runs: usize, mode: &str) {
    let driver = driver_filegdb::FileGdbDriver;
    for run in 0..=runs {
        let dataset = driver
            .open(Source::Path(path.clone()), &ReadOptions::default())
            .expect("apertura FileGDB");
        let source_fields = dataset.layers()[0].contract.schema.fields().len() - 1;
        let projected_fields = match mode {
            "full" => None,
            "narrow" => Some(vec![
                FieldId(1),
                FieldId((source_fields / 2 + 1) as u32),
                FieldId(source_fields as u32),
            ]),
            _ => panic!("modalità attesa: full oppure narrow"),
        };
        let request = ReadRequest {
            layer: LayerId(0),
            projected_fields,
            projection_mode: ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            batch_target: BatchTarget::default(),
            cancellation: Default::default(),
        };
        let start = Instant::now();
        let mut reader = dataset
            .open_layer_reader(&request)
            .expect("apertura reader FileGDB");
        let mut rows = 0_u64;
        let mut checksum = 0_i64;
        while let Some(batch) = reader.next_batch().expect("lettura batch") {
            rows += batch.num_rows() as u64;
            for column in batch.columns() {
                if let Some(values) = column.as_any().downcast_ref::<Int32Array>() {
                    checksum += values.iter().flatten().map(i64::from).sum::<i64>();
                } else if let Some(values) = column.as_any().downcast_ref::<BinaryArray>() {
                    checksum += values
                        .iter()
                        .flatten()
                        .map(|value| value.len() as i64)
                        .sum::<i64>();
                }
            }
        }
        let elapsed = start.elapsed();
        if run > 0 {
            println!(
                "run={} mode={} rows={} elapsed_ms={:.3} rows_per_second={:.0} checksum={}",
                run,
                mode,
                rows,
                elapsed.as_secs_f64() * 1_000.0,
                rows as f64 / elapsed.as_secs_f64(),
                checksum
            );
        }
    }
}

fn parse_usize(value: Option<String>, name: &str) -> usize {
    match value {
        Some(value) => match value.parse() {
            Ok(parsed) => parsed,
            Err(_) => panic!("{name} non numerico"),
        },
        None => panic!("argomento mancante: {name}"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().expect("comando atteso: generate oppure read");
    let path = PathBuf::from(args.next().expect("percorso fixture mancante"));
    match command.as_str() {
        "generate" => generate(
            path,
            parse_usize(args.next(), "rows"),
            parse_usize(args.next(), "fields"),
        ),
        "read" => read(
            path,
            parse_usize(args.next(), "runs"),
            &args.next().expect("modalità mancante"),
        ),
        _ => panic!("comando atteso: generate oppure read"),
    }
}

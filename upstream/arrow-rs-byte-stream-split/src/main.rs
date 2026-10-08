//! Minimal reproducer: an out-of-bounds index in the BYTE_STREAM_SPLIT decoder
//! when a data page header declares more values than the page actually holds.
//!
//! Run with `cargo run`. Expected: a clean `Err` from the reader. Actual: a
//! panic inside `parquet`.
//!
//! The file is written by `parquet` itself, so the starting point is known to
//! be valid: the program reads it back before touching anything. Then it flips
//! a single byte -- the zigzag varint of `num_values` in the data page header,
//! from 8 to 63 -- and reads again. 63 is the largest value a one-byte zigzag
//! varint can hold, so no offset in the file moves and the rest of the layout
//! stays exactly as the writer produced it.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::basic::Encoding;
use parquet::file::properties::{WriterProperties, WriterVersion};

/// Eight doubles in one column, encoded with BYTE_STREAM_SPLIT.
///
/// `WriterVersion::PARQUET_2_0` is required: v1 does not allow
/// BYTE_STREAM_SPLIT, and the writer would silently fall back to another
/// encoding -- which would leave the decoder under test unreached.
fn valid_parquet(values: &[f64]) -> Vec<u8> {
    let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Float64, false)]));
    let column: ArrayRef = Arc::new(Float64Array::from(values.to_vec()));
    let batch = RecordBatch::try_new(Arc::clone(&schema), vec![column]).expect("batch");

    let properties = WriterProperties::builder()
        .set_writer_version(WriterVersion::PARQUET_2_0)
        .set_dictionary_enabled(false)
        .set_encoding(Encoding::BYTE_STREAM_SPLIT)
        .build();

    let mut bytes = Vec::new();
    let mut writer =
        ArrowWriter::try_new(&mut bytes, schema, Some(properties)).expect("writer");
    writer.write(&batch).expect("write");
    writer.close().expect("close");
    bytes
}

/// Writes the bytes to `name` and reads every row back, returning the count.
///
/// Going through a real file keeps the dependency list to `parquet` and
/// `arrow`, and leaves both inputs on disk so they can be inspected with other
/// tools or attached to a report.
///
/// The read is wrapped in `catch_unwind` only so that this program can report
/// on every candidate offset instead of stopping at the first panic. The panic
/// itself is the finding; the wrapper does not make it benign.
fn write_and_read(name: &str, bytes: &[u8]) -> Result<usize, String> {
    std::fs::write(name, bytes).map_err(|e| e.to_string())?;
    let path = name.to_owned();
    std::panic::catch_unwind(move || {
        let file = std::fs::File::open(&path).expect("open");
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .expect("footer")
            .build()
            .expect("reader");
        let mut rows = 0;
        for batch in reader {
            rows += batch.expect("batch").num_rows();
        }
        rows
    })
    .map_err(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "panic with a non-string payload".to_owned())
    })
}

/// The zigzag varint of a small non-negative number, as thrift compact writes it.
fn zigzag_varint(value: i64) -> Vec<u8> {
    let mut n = ((value << 1) ^ (value >> 63)) as u64;
    let mut out = Vec::new();
    loop {
        let seven = (n & 0x7F) as u8;
        if n < 0x80 {
            out.push(seven);
            return out;
        }
        out.push(seven | 0x80);
        n >>= 7;
    }
}

/// Offsets where field 1 of a thrift compact struct carries `expected`.
///
/// `0x15` is "field id delta 1, type i32"; the varint follows. `num_values`
/// is field 1 of both `DataPageHeaderV2` and `ColumnMetaData`, so this finds
/// every candidate and the program tries them all instead of guessing.
fn num_values_sites(bytes: &[u8], expected: usize) -> Vec<usize> {
    let varint = zigzag_varint(expected as i64);
    let mut sites = Vec::new();
    for i in 0..bytes.len().saturating_sub(varint.len()) {
        if bytes[i] == 0x15 && bytes[i + 1..i + 1 + varint.len()] == varint[..] {
            sites.push(i);
        }
    }
    sites
}

/// Start of the footer, as the file itself declares it.
fn footer_start(bytes: &[u8]) -> usize {
    let length = u32::from_le_bytes([
        bytes[bytes.len() - 8],
        bytes[bytes.len() - 7],
        bytes[bytes.len() - 6],
        bytes[bytes.len() - 5],
    ]);
    bytes.len() - 8 - usize::try_from(length).expect("a u32 fits in usize here")
}

fn main() {
    let values: Vec<f64> = (0..8).map(|i| f64::from(i) + 0.5).collect();
    let original = valid_parquet(&values);

    // The counterfactual comes first: without it, a later failure would not
    // show that the single altered byte is what causes it.
    match write_and_read("valid.parquet", &original) {
        Ok(rows) => println!("valid file reads back: {rows} rows"),
        Err(message) => {
            println!("the starting file does not read: {message}");
            println!("nothing below would prove anything; stopping.");
            return;
        }
    }

    let footer = footer_start(&original);
    let sites = num_values_sites(&original, values.len());
    println!("num_values candidates at offsets {sites:?} (footer starts at {footer})");

    for site in sites {
        let mut altered = original.clone();
        altered[site + 1] = zigzag_varint(63)[0];
        assert_eq!(altered.len(), original.len(), "length must not change");

        let place = if site < footer {
            "data page header"
        } else {
            "column metadata (footer)"
        };
        println!();
        println!("--- offset {site} [{place}]: num_values 8 -> 63, one byte");
        let name = format!("altered-at-{site}.parquet");
        match write_and_read(&name, &altered) {
            Ok(rows) => println!("    read {rows} rows, no error  ({name})"),
            Err(message) => println!("    PANIC or Err: {message}  ({name})"),
        }
    }
}

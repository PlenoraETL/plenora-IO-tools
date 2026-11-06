//! Uno schema Arrow incorporato non trasforma le pagine Parquet in run-end.
use std::sync::Arc;

use arrow_array::{Int32Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::arrow_writer::ArrowWriterOptions;
use parquet::arrow::{encode_arrow_schema, ArrowWriter};
use parquet::file::metadata::KeyValue;

#[test]
fn uno_schema_run_end_incorporato_non_costruisce_array_run_end() {
    let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int32, false)]));
    let batch =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int32Array::from(vec![7, 9]))]).unwrap();
    let ostile = Schema::new(vec![Field::new(
        "v",
        DataType::RunEndEncoded(
            Arc::new(Field::new("run_ends", DataType::Int32, false)),
            Arc::new(Field::new("values", DataType::Int32, false)),
        ),
        false,
    )]);
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = ArrowWriter::try_new_with_options(
        file.reopen().unwrap(),
        schema,
        ArrowWriterOptions::new().with_skip_arrow_metadata(true),
    )
    .unwrap();
    writer.write(&batch).unwrap();
    // Codifica diretta: add_encoded_arrow_schema_to_metadata appiattirebbe REE.
    writer.append_key_value_metadata(KeyValue::new(
        "ARROW:schema".to_owned(),
        encode_arrow_schema(&ostile),
    ));
    writer.close().unwrap();
    // L'hint non applicabile viene ignorato: la rappresentazione fisica resta
    // Int32, senza creare figli run_ends/values o invocare la loro validazione.
    let builder = ParquetRecordBatchReaderBuilder::try_new(file.reopen().unwrap()).unwrap();
    assert_eq!(builder.schema().field(0).data_type(), &DataType::Int32);
    let mut reader = builder.build().unwrap();
    let letto = reader.next().unwrap().unwrap();
    assert_eq!(letto.column(0), batch.column(0));
    assert!(reader.next().is_none());
}

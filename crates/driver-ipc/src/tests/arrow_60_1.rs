//! Contratti osservabili dell'aggiornamento Arrow e confine RUSTSEC-2026-0442.
use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{Array, Int32Array, RecordBatch, RunArray};
use arrow_ipc::writer::FileWriter;
use arrow_schema::{DataType, Field, Schema};

#[test]
fn array_del_chiamante_non_puo_dichiarare_righe_senza_run() {
    let array = RunArray::<Int32Type>::try_new(
        &Int32Array::from(vec![2, 3]),
        &Int32Array::from(vec![7, 9]),
    )
    .unwrap();
    // Riproduttore upstream: nessun accesso fuori limite viene eseguito.
    for (offset, len) in [(0, 5), (2, 2), (3, 1)] {
        assert!(array
            .to_data()
            .into_builder()
            .offset(offset)
            .len(len)
            .build()
            .is_err());
    }
    assert!(array
        .to_data()
        .into_builder()
        .offset(1)
        .len(2)
        .build()
        .is_ok());
}

#[test]
fn ipc_senza_null_scrive_bitmap_vuota_e_rilegge_gli_stessi_valori() {
    let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int32, true)]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int32Array::from(vec![Some(7), Some(9)]))],
    )
    .unwrap();
    let mut bytes = Vec::new();
    let mut writer = FileWriter::try_new(&mut bytes, &schema).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);
    let n = bytes.len();
    let footer_len =
        usize::try_from(i32::from_le_bytes(bytes[n - 10..n - 6].try_into().unwrap())).unwrap();
    let footer = arrow_ipc::root_as_footer(&bytes[n - 10 - footer_len..n - 10]).unwrap();
    let offset = usize::try_from(footer.recordBatches().unwrap().get(0).offset()).unwrap();
    let length = usize::try_from(i32::from_le_bytes(
        bytes[offset + 4..offset + 8].try_into().unwrap(),
    ))
    .unwrap();
    let message = arrow_ipc::root_as_message(&bytes[offset + 8..offset + 8 + length]).unwrap();
    assert_eq!(
        message
            .header_as_record_batch()
            .unwrap()
            .buffers()
            .unwrap()
            .get(0)
            .length(),
        0
    );
    let mut reader =
        arrow_ipc::reader::FileReader::try_new(std::io::Cursor::new(bytes), None).unwrap();
    assert_eq!(reader.next().unwrap().unwrap(), batch);
}

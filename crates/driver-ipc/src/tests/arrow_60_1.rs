//! Contratti osservabili dell'aggiornamento Arrow e confine RUSTSEC-2026-0442.
use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{Array, Int32Array, RecordBatch, RunArray};
use arrow_ipc::writer::FileWriter;
use arrow_schema::{DataType, Field, Schema};

#[test]
fn liste_affettate_di_union_conservano_i_valori_attraverso_io() {
    use super::*;
    use arrow_array::{ArrayRef, ListArray, UnionArray};
    use arrow_schema::{UnionFields, UnionMode};

    for mode in [UnionMode::Dense, UnionMode::Sparse] {
        let fields = UnionFields::try_new(
            [3, 7],
            [
                Field::new("a", DataType::Int32, true),
                Field::new("b", DataType::Int32, true),
            ],
        )
        .unwrap();
        let union: ArrayRef = Arc::new(
            UnionArray::try_new(
                fields,
                vec![3_i8, 7, 3].into(),
                (mode == UnionMode::Dense).then(|| vec![2, 0, 1].into()),
                vec![
                    Arc::new(Int32Array::from(vec![Some(11), None, Some(33)])),
                    Arc::new(Int32Array::from(vec![Some(101), Some(202), None])),
                ],
            )
            .unwrap(),
        );
        let list = ListArray::new(
            Arc::new(Field::new("item", union.data_type().clone(), true)),
            ListArray::from_iter_primitive::<Int32Type, _, _>([
                Some(vec![Some(0)]),
                Some(vec![Some(0), Some(0)]),
            ])
            .offsets()
            .clone(),
            union,
            None,
        )
        .slice(1, 1);
        let schema = Arc::new(Schema::new(vec![Field::new(
            "v",
            list.data_type().clone(),
            true,
        )]));
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(list)]).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("union.arrow");
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "union".to_owned(),
                contract: DataContract {
                    schema,
                    geometry: None,
                },
            }],
        };
        let mut writer = IpcDriver
            .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        let dataset = IpcDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let mut reader = dataset
            .open_layer_reader(&ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: ReadScope::default(),
                batch_target: BatchTarget::default(),
                cancellation: CancellationToken::default(),
            })
            .unwrap();
        let output = reader.next_batch().unwrap().unwrap();
        assert_eq!(output.num_rows(), 1);
        let values = output
            .column(0)
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap()
            .value(0);
        let union = values.as_any().downcast_ref::<UnionArray>().unwrap();
        assert_eq!(union.len(), 2);
        // Attesi dalla costruzione, senza usare uguaglianza ArrayData o il
        // decoder upstream come oracolo (anch'essi corretti in questa release).
        for (index, atteso) in [
            Some(if mode == UnionMode::Dense { 101 } else { 202 }),
            if mode == UnionMode::Dense {
                None
            } else {
                Some(33)
            },
        ]
        .into_iter()
        .enumerate()
        {
            let value = union.value(index);
            let value = value.as_any().downcast_ref::<Int32Array>().unwrap();
            assert_eq!(value.iter().next().unwrap(), atteso);
        }
        assert!(reader.next_batch().unwrap().is_none());
    }
}

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

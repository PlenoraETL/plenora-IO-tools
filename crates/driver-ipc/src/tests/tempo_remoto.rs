use super::*;
#[test]
fn tempo_remoto_ipc_extreme_ticks_preserved() {
    use arrow_array::{
        TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
        TimestampSecondArray,
    };
    let dir = tempfile::tempdir().unwrap();
    let values = vec![i64::MIN, -1, 0, i64::MAX];
    let arrays: Vec<Arc<dyn arrow_array::Array>> = vec![
        Arc::new(TimestampSecondArray::from(values.clone())),
        Arc::new(TimestampMillisecondArray::from(values.clone())),
        Arc::new(TimestampMicrosecondArray::from(values.clone())),
        Arc::new(TimestampNanosecondArray::from(values)),
    ];
    let fields: Vec<_> = arrays
        .iter()
        .enumerate()
        .map(|(i, a)| Field::new(format!("t{i}"), a.data_type().clone(), false))
        .collect();
    let schema = with_contract_version(Arc::new(Schema::new(fields)));
    let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
    let path = dir.path().join("ticks.arrow");
    let mut writer = FileWriter::try_new(File::create(&path).unwrap(), &schema).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);
    let ds = IpcDriver
        .open(Source::Path(path.clone()), opzioni_lettura())
        .unwrap();
    let mut reader = ds
        .open_layer_reader(&ReadRequest {
            cancellation: Default::default(),
            layer: ds.layers()[0].id.clone(),
            projected_fields: None,
            projection_mode: ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: ReadScope::default(),
            batch_target: BatchTarget::default(),
        })
        .unwrap();
    let result = reader.next_batch().unwrap().unwrap();
    for (a, b) in result.columns().iter().zip(batch.columns()) {
        assert_eq!(a.to_data(), b.to_data());
    }
    assert!(reader.next_batch().unwrap().is_none());
    if std::env::var_os("TEMPO_GENERATE_SEEDS").is_some() {
        std::fs::copy(
            path,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fuzz/seeds/ipc_reader/tempo-extreme-ticks.arrow"),
        )
        .unwrap();
    }
}

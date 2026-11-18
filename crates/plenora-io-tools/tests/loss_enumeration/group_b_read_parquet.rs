//! `GeoParquet` 1.1 documents with native bounding boxes, covering and feature IDs.
use super::*;

fn write(path: &Path, element: E, bounds: [f64; 4]) -> RecordBatch {
    let mut column = json!({"encoding":"WKB","geometry_types":["Point"]});
    let mut fields = vec![Field::new("geometry", DataType::Binary, true)];
    let mut arrays = vec![base().column(0).clone()];
    if element == E::Bbox {
        column["bbox"] = json!(bounds);
    } else if element == E::Covering {
        let mut paths = serde_json::Map::new();
        let mut children = Vec::new();
        for (name, value) in ["xmin", "ymin", "xmax", "ymax"].into_iter().zip(bounds) {
            paths.insert(name.to_owned(), json!(["bbox", name]));
            children.push((
                Arc::new(Field::new(name, DataType::Float64, true)),
                Arc::new(Float64Array::from(vec![value])) as ArrayRef,
            ));
        }
        column["covering"] = json!({"bbox":paths});
        let array = arrow_array::StructArray::from(children);
        fields.push(Field::new("bbox", array.data_type().clone(), true));
        arrays.push(Arc::new(array));
    } else {
        fields.push(Field::new("fid", DataType::Int64, false));
        arrays.push(Arc::new(Int64Array::from(vec![987_654_321])));
    }
    let geo = json!({"version":"1.1.0","primary_column":"geometry","columns":{"geometry":column}})
        .to_string();
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
    let props = parquet::file::properties::WriterProperties::builder()
        .set_key_value_metadata(Some(vec![parquet::file::metadata::KeyValue::new(
            "geo".to_owned(),
            geo,
        )]))
        .build();
    let mut writer = parquet::arrow::ArrowWriter::try_new(
        std::fs::File::create(path).unwrap(),
        schema,
        Some(props),
    )
    .unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    let mut expected = base();
    // The native file declares CRS84 implicitly, as required by GeoParquet.
    // Only WKB values and native columns are the semantic source here.
    let geometry = Field::new("geometry", DataType::Binary, true)
        .with_metadata([("ARROW:extension:name".to_owned(), "geoarrow.wkb".to_owned())]);
    let mut fields = vec![geometry];
    let mut arrays = vec![expected.column(0).clone()];
    if element == E::FeatureId {
        fields.push(batch.schema().field(1).clone());
        arrays.push(batch.column(1).clone());
    }
    expected = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).unwrap();
    expected
}

pub(super) fn exercise(native: NativeDriver, element: E, root: &Path, row: &mut Value) {
    let mut failures = Vec::new();
    for (variant, bounds) in [[1.0, 2.0, 1.0, 2.0], [-7.0, 2.0, 1.0, 2.0]]
        .into_iter()
        .enumerate()
    {
        let dir = tempfile::tempdir_in(root).unwrap();
        let input = dir.path().join("native.parquet");
        let output = dir.path().join("result.arrow");
        let expected = write(&input, element, bounds);
        let (ok, report) = run(&input, &output, "geoparquet", "ipc");
        if !ok {
            failures
                .push(json!({"variant":variant,"native_read_failed":report,"fixture":dir.keep()}));
            continue;
        }
        let after = native_batch(&input, &output, "geoparquet");
        let differences = compare(&expected, &after);
        let mut observed = BTreeMap::<String, u64>::new();
        for difference in &differences {
            *observed
                .entry(category(native, D::Read, difference))
                .or_default() += 1;
        }
        // Reconstruct the actual bbox independently from delivered geometry.
        let values = after
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        let geometry = decode_wkb(values.value(0), &WkbLimits::default()).unwrap();
        let WkbValue::Point(c) = geometry.value else {
            panic!("native Point changed type")
        };
        let received = if element == E::Bbox {
            after
                .schema()
                .field(0)
                .metadata()
                .get("plenora.geometry.native.geoparquet.bbox")
                .map(|value| {
                    value
                        .split(',')
                        .map(|v| v.parse::<f64>().unwrap().to_bits())
                        .collect::<Vec<_>>()
                })
        } else {
            Some([c.x, c.y, c.x, c.y].map(f64::to_bits).to_vec())
        };
        if element != E::FeatureId && received != Some(bounds.map(f64::to_bits).to_vec()) {
            let NativeRule::Transformed(change) = native_rule(native, D::Read, element) else {
                panic!("bbox table rule")
            };
            *observed.entry(change.category().to_owned()).or_default() += 1;
        }
        let declared = report_categories(&report, "read_loss");
        if observed != declared {
            failures.push(json!({"variant":variant,"observed":observed,"declared":declared,"differences":format!("{differences:?}"),"fixture":dir.keep()}));
        }
    }
    row["status"] = json!(if failures.is_empty() {
        "passed"
    } else {
        "failed"
    });
    row["variants_exercised"] = json!(2);
    row["witness"] = json!("native GeoParquet 1.1 footer and columns via Apache Parquet; independent geometry/bbox/ID comparison");
    row["failures"] = json!(failures);
}

use super::*;

#[test]
fn native_field_order_reads_both_definition_vocabularies() {
    for tag in ["Field", "GPFieldInfoEx"] {
        let xml = format!("<Definition><Name>layer</Name><Fields><{tag}><Name>geometry</Name></{tag}><{tag}><Name>a&amp;b</Name></{tag}></Fields></Definition>");
        assert_eq!(definition_field_order(&xml).unwrap(), ["geometry", "a&b"]);
    }
}

fn assert_native_alias(reader: &dyn LayerReader, index: usize) {
    let loss = reader.loss_report();
    assert!(loss.counts["native_layer_metadata_not_preserved"] > 0);
    assert!(loss
        .esempi_canonici()
        .all(|example| example.posizione.layer_index == Some(index as u64)));
}

#[test]
fn native_domains_aliases_and_subtypes_are_declared() {
    let mut report = LossReport::default();
    observe_definition_xml(&mut report, "<DEFeatureClassInfo><AliasName>opaque_alias</AliasName><Fields><Field><DomainName>opaque_domain</DomainName></Field></Fields><Subtypes><Subtype><SubtypeName>opaque_subtype</SubtypeName></Subtype></Subtypes></DEFeatureClassInfo>", 3).unwrap();
    assert_eq!(report.counts["native_layer_metadata_not_preserved"], 3);
    for example in report.esempi_canonici() {
        assert_eq!(example.posizione.layer_index, Some(3));
        assert!(!example.context.contains("opaque"));
    }
    let mut empty = LossReport::default();
    observe_definition_xml(
        &mut empty,
        "<Definition><DomainName/><AliasName></AliasName><Subtypes/></Definition>",
        0,
    )
    .unwrap();
    assert!(empty.is_empty());
    let mut escaped = LossReport::default();
    observe_definition_xml(&mut escaped, "<Definition><AliasName>A&amp;B<![CDATA[C]]></AliasName><DomainName><![CDATA[D]]></DomainName></Definition>", 0).unwrap();
    assert_eq!(escaped.counts["native_layer_metadata_not_preserved"], 2);
}

#[test]
fn native_hierarchy_and_extension_payloads_are_counted_without_names() {
    let mut report = LossReport::default();
    observe_definition_xml(&mut report, r"<Definition><CatalogPath>\private_group\features</CatalogPath><ExtensionProperties><PropertySetProperty><Key>private_key</Key><Value>private_payload</Value></PropertySetProperty></ExtensionProperties></Definition>", 4).unwrap();
    assert_eq!(report.counts["native_hierarchy_flattened"], 1);
    assert_eq!(report.counts["native_layer_metadata_not_preserved"], 1);
    for example in report.esempi_canonici() {
        assert_eq!(example.posizione.layer_index, Some(4));
        assert!(!example.context.contains("private"));
    }
    let mut empty = LossReport::default();
    observe_definition_xml(&mut empty, r"<Definition><CatalogPath>\features</CatalogPath><ExtensionProperties>  </ExtensionProperties></Definition>", 0).unwrap();
    assert!(empty.is_empty());
}

#[test]
fn catalogo_con_oggetti_non_leggibili_non_scala_gli_indici() {
    use gdal::vector::{FieldDefn, LayerOptions, OGRFieldType};
    use plenora_io_model::CapabilityReason as R;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("objects.gdb");
    let driver = gdal::DriverManager::get_driver_by_name("OpenFileGDB").unwrap();
    let mut dataset = driver.create_vector_only(&path).unwrap();
    let cases = [
        ("Healthy", OGRFieldType::OFTString, None),
        ("NativeDate", OGRFieldType::OFTDate, None),
        ("DateTime", OGRFieldType::OFTDateTime, None),
        ("Binary", OGRFieldType::OFTBinary, None),
        ("Missing", OGRFieldType::OFTString, Some(R::ObjectMissing)),
        ("Empty", OGRFieldType::OFTString, Some(R::ObjectEmpty)),
    ];
    for (name, typ, _) in cases {
        let layer = dataset
            .create_layer(LayerOptions {
                name,
                ty: gdal::vector::OGRwkbGeometryType::wkbNone,
                options: Some(&["LAYER_ALIAS=opaque_native_alias"]),
                ..Default::default()
            })
            .unwrap();
        FieldDefn::new("value", typ)
            .unwrap()
            .add_to_layer(&layer)
            .unwrap();
    }
    drop(dataset);
    let objects = catalog_objects(&path).unwrap();
    for (id, name) in objects {
        if name == "Missing" {
            std::fs::remove_file(path.join(format!("a{id:08x}.gdbtable"))).unwrap();
        }
        if name == "Empty" {
            std::fs::write(path.join(format!("a{id:08x}.gdbtablx")), []).unwrap();
        }
    }
    for _ in 0..2 {
        let dataset = open(&path, None).unwrap();
        assert_eq!(dataset.layers().len(), cases.len());
        for (index, (name, _, reason)) in cases.iter().enumerate() {
            let layer = &dataset.layers()[index];
            assert_eq!(&layer.name, name);
            assert_eq!(layer.id.0 as usize, index);
            let request = ReadRequest {
                layer: layer.id,
                projected_fields: None,
                projection_mode: plenora_io_core::ProjectionMode::BestEffort,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: plenora_io_core::ReadScope::Complete,
                batch_target: plenora_io_core::BatchTarget::default(),
                cancellation: plenora_io_model::CancellationToken::default(),
            };
            if let Some(reason) = reason {
                assert_eq!(
                    dataset
                        .rifiuto_del_layer(layer.id)
                        .unwrap()
                        .capability_reason,
                    Some(*reason)
                );
                assert_eq!(
                    dataset
                        .open_layer_reader(&request)
                        .err()
                        .unwrap()
                        .capability_reason,
                    Some(*reason)
                );
            } else {
                assert!(dataset.rifiuto_del_layer(layer.id).is_none());
                let reader = dataset.open_layer_reader(&request).unwrap();
                assert_native_alias(reader.as_ref(), index);
                drop(reader);
                assert!(dataset
                    .open_layer_reader(&request)
                    .unwrap()
                    .next_batch()
                    .unwrap()
                    .is_none());
            }
        }
    }
}

#[test]
// Keep native fixture construction and both reader projections in one regression.
#[allow(clippy::too_many_lines)]
fn native_binary_dates_and_fractional_datetime_are_readable() {
    use arrow_array::{Array, BinaryArray, TimestampMicrosecondArray};
    use gdal::vector::{FieldDefn, LayerOptions, OGRFieldType};
    for (label, options, expected_timezone) in [
        ("utc", vec!["TIME_IN_UTC=YES"], Some("UTC")),
        ("local", vec!["TIME_IN_UTC=NO"], None),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{label}.gdb"));
        let driver = gdal::DriverManager::get_driver_by_name("OpenFileGDB").unwrap();
        let mut ds = driver.create_vector_only(&path).unwrap();
        let layer = ds
            .create_layer(LayerOptions {
                name: "native_values",
                ty: gdal::vector::OGRwkbGeometryType::wkbNone,
                options: Some(&options),
                ..Default::default()
            })
            .unwrap();
        for (name, typ) in [
            ("bytes", OGRFieldType::OFTBinary),
            ("date", OGRFieldType::OFTDate),
            ("time", OGRFieldType::OFTDateTime),
        ] {
            FieldDefn::new(name, typ)
                .unwrap()
                .add_to_layer(&layer)
                .unwrap();
        }
        let mut feature = Feature::new(layer.defn()).unwrap();
        assert_eq!(feature.field_as_binary(0).unwrap(), None);
        assert_eq!(feature.field_as_datetime_components(2).unwrap(), None);
        assert!(feature.field_as_binary(usize::MAX).is_err());
        assert!(feature.field_as_datetime_components(usize::MAX).is_err());

        feature.set_field_string(1, "2024/02/29").unwrap();
        feature
            .set_field_string(2, "2024/02/29 12:34:56.125")
            .unwrap();

        assert_eq!(
            feature
                .field_as_datetime_components(2)
                .unwrap()
                .unwrap()
                .5
                .to_bits(),
            56.125_f32.to_bits()
        );
        feature.create(&layer).unwrap();
        let null = Feature::new(layer.defn()).unwrap();
        null.create(&layer).unwrap();
        drop(null);
        drop(feature);
        ds.execute_sql(
            "UPDATE \"native_values\" SET bytes=X'00017FFF' WHERE OBJECTID=1",
            None,
            gdal::vector::sql::Dialect::SQLITE,
        )
        .unwrap();
        let layer = ds.layer(0).unwrap();
        let mut feature = layer.feature(1).unwrap();
        feature
            .set_field_string(2, "2024/02/29 12:34:56.125+00")
            .unwrap();
        layer.set_feature(feature).unwrap();
        drop(ds);
        // The runtime may expose whole seconds even if the input had fractions.
        // Compare against OGR, the authority at this boundary, without inventing digits.
        let raw = apri_solo_filegdb(&path).unwrap();
        let raw_layer = raw.layer(0).unwrap();
        let raw_feature = raw_layer.feature(1).unwrap();
        let components = raw_feature
            .field_as_datetime_components(2)
            .unwrap()
            .unwrap();
        let expected_time = temporal_value(components, expected_timezone.is_some()).unwrap();
        drop(raw_feature);
        drop(raw);
        let dataset = open(&path, None).unwrap();
        assert!(
            dataset.rifiuto_del_layer(LayerId(0)).is_none(),
            "{:?}",
            dataset.rifiuto_del_layer(LayerId(0))
        );
        let contract = &dataset.layers()[0];
        assert_eq!(
            contract.contract.schema.field(0).data_type(),
            &DataType::Binary
        );
        // OpenFileGDB's default target exposes Date as DateTime, not Date.
        assert_eq!(
            contract.contract.schema.field(2).data_type(),
            &DataType::Timestamp(
                arrow_schema::TimeUnit::Microsecond,
                expected_timezone.map(Into::into)
            )
        );
        let request = ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: plenora_io_core::ProjectionMode::BestEffort,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: plenora_io_core::ReadScope::Complete,
            batch_target: plenora_io_core::BatchTarget::default(),
            cancellation: plenora_io_model::CancellationToken::default(),
        };
        let mut reader = dataset.open_layer_reader(&request).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let bytes = batch
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(bytes.value(0), &[0, 1, 127, 255]);
        assert!(bytes.is_null(1));
        let time = batch
            .column(2)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap();
        assert_eq!(time.value(0), expected_time);
        assert!(time.is_null(1));
        assert!(!reader
            .loss_report()
            .counts
            .contains_key("native_number_precision"));
        assert!(reader.next_batch().unwrap().is_none());
        drop(reader);
        let projected = ReadRequest {
            projected_fields: Some(vec![FieldId(2)]),
            ..request
        };
        let mut reader = dataset.open_layer_reader(&projected).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        assert_eq!(batch.num_columns(), 1);
        assert_eq!(
            batch
                .column(0)
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap()
                .value(0),
            expected_time
        );
    }
}

#[test]
fn temporal_components_preserve_offsets_and_never_invent_utc() {
    let utc = temporal_value((2024, 2, 29, 12, 0, 0.125, 100), true).unwrap();
    assert_eq!(
        temporal_value((2024, 2, 29, 13, 0, 0.125, 104), true).unwrap(),
        utc
    );
    assert_eq!(
        temporal_value((2024, 2, 29, 7, 0, 0.125, 80), true).unwrap(),
        utc
    );
    assert!(temporal_value((2024, 2, 29, 12, 0, 0.0, 0), true).is_err());
    assert!(temporal_value((2024, 2, 29, 12, 0, 0.0, 100), false).is_err());
    assert!(temporal_value((2023, 2, 29, 12, 0, 0.0, 0), false).is_err());
    assert_eq!(ogr_timezone(0).unwrap(), None);
    assert_eq!(ogr_timezone(1).unwrap(), None);
    assert_eq!(ogr_timezone(104).unwrap().as_deref(), Some("+01:00"));
    assert_eq!(
        ogr_to_arrow(gdal::vector::OGRFieldType::OFTDate).unwrap(),
        DataType::Date32
    );
}

#[test]
fn fractional_seconds_accessor_and_arrow_rounding_are_explicit() {
    use gdal::vector::{FieldDefn, LayerOptions, OGRFieldType};
    let driver = gdal::DriverManager::get_driver_by_name("Memory").unwrap();
    let mut dataset = driver.create_vector_only("").unwrap();
    let layer = dataset
        .create_layer(LayerOptions {
            ty: gdal::vector::OGRwkbGeometryType::wkbNone,
            ..Default::default()
        })
        .unwrap();
    FieldDefn::new("instant", OGRFieldType::OFTDateTime)
        .unwrap()
        .add_to_layer(&layer)
        .unwrap();
    let mut feature = Feature::new(layer.defn()).unwrap();
    let field = ProjectedField {
        ogr_index: 0,
        name: "instant".into(),
        data_type: DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into())),
        ogr_type: OGRFieldType::OFTDateTime,
    };
    let mut builder = ReadCol::new(&field.data_type);
    feature
        .set_field_string(0, "2024/02/29 12:34:56.125+00")
        .unwrap();
    let components = feature.field_as_datetime_components(0).unwrap().unwrap();
    assert_eq!(components.5.to_bits(), 56.125_f32.to_bits());
    assert_eq!(components.6, 100);
    assert!(!builder.append_feature(&feature, &field).unwrap());
    feature
        .set_field_string(0, "2024/02/29 12:34:56.123+00")
        .unwrap();
    assert!(builder.append_feature(&feature, &field).unwrap());
    let values = builder.finish();
    let values = values
        .as_any()
        .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
        .unwrap();
    assert_eq!(values.value(0), 1_709_210_096_125_000);
    assert_eq!(values.value(1), 1_709_210_096_123_001);
    assert_eq!(
        definition_time_in_utc("<Definition><IsTimeInUTC>true</IsTimeInUTC></Definition>").unwrap(),
        Some(true)
    );
    assert!(
        definition_time_in_utc("<Definition><IsTimeInUTC>bogus</IsTimeInUTC></Definition>")
            .is_err()
    );
}

#[test]
fn native_date32_keeps_calendar_day_and_null() {
    use gdal::vector::{FieldDefn, LayerOptions, OGRFieldType};
    let mut dataset = gdal::DriverManager::get_driver_by_name("Memory")
        .unwrap()
        .create_vector_only("")
        .unwrap();
    let layer = dataset
        .create_layer(LayerOptions {
            ty: gdal::vector::OGRwkbGeometryType::wkbNone,
            ..Default::default()
        })
        .unwrap();
    FieldDefn::new("calendar", OGRFieldType::OFTDate)
        .unwrap()
        .add_to_layer(&layer)
        .unwrap();
    let mut feature = Feature::new(layer.defn()).unwrap();
    let field = ProjectedField {
        ogr_index: 0,
        name: "calendar".into(),
        data_type: DataType::Date32,
        ogr_type: OGRFieldType::OFTDate,
    };
    let mut builder = ReadCol::new(&field.data_type);
    assert!(!builder.append_feature(&feature, &field).unwrap());
    feature.set_field_string(0, "2024/02/29").unwrap();
    assert!(!builder.append_feature(&feature, &field).unwrap());
    let array = builder.finish();
    assert!(array.is_null(0));
    assert_eq!(
        array
            .as_any()
            .downcast_ref::<arrow_array::Date32Array>()
            .unwrap()
            .value(1),
        19_782
    );
}

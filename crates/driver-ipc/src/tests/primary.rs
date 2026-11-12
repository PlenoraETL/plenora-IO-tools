use super::*;
use plenora_io_model::primary::{PrimaryState, KEY};

fn schema(key: Option<&str>) -> SchemaRef {
    let fields = [17, 91]
        .into_iter()
        .enumerate()
        .map(|(i, id)| {
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(u32::try_from(i).unwrap()),
                format!("g{i}"),
                ResolvedCrs::wgs84(),
                true,
            );
            geometry.identita_dichiarata = Some(id);
            geometry
                .set_exact_geometry_types(vec![plenora_io_model::contract::GeometryType::Point]);
            plenora_io_model::geometry::with_geometry_contract_metadata(
                &Field::new(format!("g{i}"), DataType::Binary, true).with_metadata(
                    std::iter::once(("ARROW:extension:name".to_owned(), "geoarrow.wkb".to_owned()))
                        .collect::<arrow_schema::Metadata>(),
                ),
                &geometry,
            )
        })
        .collect::<Vec<_>>();
    let mut metadata = arrow_schema::Metadata::new();
    metadata.insert("plenora.contract.version", "1");
    if let Some(key) = key {
        metadata.insert(KEY, key);
    }
    Arc::new(Schema::new_with_metadata(fields, metadata))
}

#[test]
fn primary_file_stream_projection_and_empty_batches() {
    for stream in [false, true] {
        for key in [None, Some("none"), Some("17"), Some("91")] {
            for empty in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("source.arrow");
                let schema = schema(key);
                let wkb =
                    to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1., 2.))).unwrap();
                let batch = RecordBatch::try_new(
                    schema.clone(),
                    vec![
                        Arc::new(BinaryArray::from(vec![Some(wkb.as_slice()), None])),
                        Arc::new(BinaryArray::from(vec![None, Some(wkb.as_slice())])),
                    ],
                )
                .unwrap();
                if stream {
                    let mut writer =
                        StreamWriter::try_new(File::create(&path).unwrap(), &schema).unwrap();
                    if !empty {
                        writer.write(&batch).unwrap();
                        writer.write(&batch).unwrap();
                    }
                    writer.finish().unwrap();
                } else {
                    let mut writer =
                        FileWriter::try_new(File::create(&path).unwrap(), &schema).unwrap();
                    if !empty {
                        writer.write(&batch).unwrap();
                        writer.write(&batch).unwrap();
                    }
                    writer.finish().unwrap();
                }
                let dataset = IpcDriver
                    .open(Source::Path(path), opzioni_lettura())
                    .unwrap();
                // Oracolo indipendente: due geometrie, quindi assenza/none
                // non designano una primaria; gli ID non sono posizioni.
                let original = match key {
                    Some("17") => Some(0),
                    Some("91") => Some(1),
                    _ => None,
                };
                for selection in [
                    vec![],
                    vec![FieldId(0)],
                    vec![FieldId(1)],
                    vec![FieldId(0), FieldId(1)],
                ] {
                    let request = ReadRequest {
                        layer: LayerId(0),
                        projected_fields: Some(selection.clone()),
                        projection_mode: plenora_io_core::ProjectionMode::Required,
                        pruning_predicate: None,
                        spatial_pruning_hint: None,
                        scope: plenora_io_core::ReadScope::default(),
                        batch_target: plenora_io_core::BatchTarget::default(),
                        cancellation: plenora_io_model::CancellationToken::default(),
                    };
                    let mut reader = dataset.open_layer_reader(&request).unwrap();
                    let contract = reader.contract().clone();
                    let expected_position = original
                        .and_then(|old| selection.iter().position(|id| id.0 as usize == old));
                    let mut md = schema.metadata().clone();
                    md.remove(KEY);
                    if selection.len() == 1 && expected_position.is_none() {
                        md.insert(KEY, "none");
                    } else if selection.len() == 2 && expected_position.is_some() {
                        md.insert(KEY, if original == Some(0) { "17" } else { "91" });
                    }
                    let expected = Arc::new(Schema::new_with_metadata(
                        selection
                            .iter()
                            .map(|id| schema.field(id.0 as usize).as_ref().clone())
                            .collect::<Vec<_>>(),
                        md,
                    ));
                    assert_eq!(contract.contract.schema, expected);
                    assert_eq!(
                        PrimaryState::read(&expected).unwrap().position(&expected),
                        expected_position
                    );
                    let mut rows = 0;
                    while let Some(observed) = reader.next_batch().unwrap() {
                        assert_eq!(observed.schema(), expected);
                        for (i, id) in selection.iter().enumerate() {
                            assert_eq!(observed.column(i), batch.column(id.0 as usize));
                        }
                        rows += observed.num_rows();
                    }
                    assert_eq!(rows, if empty { 0 } else { 4 });
                }
            }
        }
    }
}

#[test]
fn dangling_reference_is_not_completed_and_fields_precede_primary() {
    let mut metadata = schema(Some("2")).metadata().clone();
    let input = Schema::new_with_metadata(schema(None).fields().clone(), metadata.clone());
    assert_eq!(
        plenora_io_model::vocabolario::verifica_schema(&input)
            .unwrap_err()
            .category,
        plenora_io_model::ErrorCategory::Schema
    );
    metadata.insert(KEY, "00091");
    assert!(
        PrimaryState::read(&Schema::new_with_metadata(input.fields().clone(), metadata)).is_err()
    );
}

#[test]
fn direct_plan_does_not_repair_dangling_or_conflicting_primary() {
    for key in ["999", "91"] {
        let input = schema(Some(key));
        let mut geometry =
            GeometryColumnContract::wkb_xy(FieldId(0), "g0", ResolvedCrs::wgs84(), true);
        geometry.identita_dichiarata = Some(999);
        let contract = DataContract::new(input.clone(), Some(geometry));
        assert_eq!(contract.schema, input, "il costruttore conserva il difetto");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.arrow");
        let plan = WritePlan {
            layers: vec![plenora_io_core::WriteLayer {
                name: "l".into(),
                contract,
            }],
        };
        let error = IpcDriver
            .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
            .err()
            .unwrap();
        assert_eq!(error.category, plenora_io_model::ErrorCategory::Schema);
        assert_eq!(error.phase, plenora_io_model::ErrorPhase::Validate);
        assert!(!path.exists());
    }
}

#[test]
fn invalid_secondary_later_batch_never_publishes() {
    for key in [None, Some("91")] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("late.arrow");
        let schema = schema(key);
        let primary = plenora_io_model::primary::PrimaryState::read(&schema)
            .unwrap()
            .position(&schema);
        let geometry = primary.map(|i| {
            let mut g = GeometryColumnContract::wkb_xy(
                FieldId(u32::try_from(i).unwrap()),
                format!("g{i}"),
                ResolvedCrs::wgs84(),
                true,
            );
            read_geometry_contract_metadata(schema.field(i), &mut g).unwrap();
            g
        });
        let plan = WritePlan {
            layers: vec![plenora_io_core::WriteLayer {
                name: "l".into(),
                contract: DataContract::new(schema.clone(), geometry),
            }],
        };
        let mut writer = IpcDriver
            .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
            .unwrap();
        let valid = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(BinaryArray::from(vec![None::<&[u8]>])),
                Arc::new(BinaryArray::from(vec![None::<&[u8]>])),
            ],
        )
        .unwrap();
        writer.write(&valid).unwrap();
        let invalid = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(BinaryArray::from(vec![None::<&[u8]>])),
                Arc::new(BinaryArray::from(vec![Some(&[1_u8, 1, 0][..])])),
            ],
        )
        .unwrap();
        assert!(writer.write(&invalid).is_err());
        assert!(!path.exists());
        drop(writer);
        assert!(!path.exists());
    }
}

#[test]
fn vocabulary_precedence_is_fields_then_primary_then_crs() {
    let source = schema(Some("bad"));
    let mut fields: Vec<_> = source.fields().iter().map(|f| f.as_ref().clone()).collect();
    let mut md = fields[1].metadata().clone();
    md.insert("plenora.field_id", "17");
    fields[1] = fields[1].clone().with_metadata(md);
    let duplicate = Schema::new_with_metadata(fields, source.metadata().clone());
    let error = plenora_io_model::vocabolario::verifica_schema(&duplicate).unwrap_err();
    assert!(error.message.contains("ripetuto"));
    let mut fields: Vec<_> = source.fields().iter().map(|f| f.as_ref().clone()).collect();
    let mut md = fields[1].metadata().clone();
    md.remove("plenora.geometry.axis_order");
    fields[1] = fields[1].clone().with_metadata(md);
    let no_axis = Schema::new_with_metadata(fields, source.metadata().clone());
    let error = plenora_io_model::vocabolario::verifica_schema(&no_axis).unwrap_err();
    assert!(error.message.contains("primary_geometry"));
}

#[test]
fn primary_decimal_beyond_u32_is_support_not_grammar() {
    let base = schema(None);
    let mut fields: Vec<_> = base.fields().iter().map(|f| f.as_ref().clone()).collect();
    let mut md = fields[1].metadata().clone();
    md.insert("plenora.field_id", "4294967296");
    fields[1] = fields[1].clone().with_metadata(md);
    let mut md = base.metadata().clone();
    md.insert(KEY, "4294967296");
    let schema = Schema::new_with_metadata(fields, md);
    plenora_io_model::vocabolario::verifica_schema(&schema).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large-id.arrow");
    FileWriter::try_new(File::create(&path).unwrap(), &schema)
        .unwrap()
        .finish()
        .unwrap();
    let error = IpcDriver
        .open(Source::Path(path), opzioni_lettura())
        .err()
        .unwrap();
    assert_eq!(error.category, plenora_io_model::ErrorCategory::Unsupported);
}

#[test]
#[allow(clippy::too_many_lines)] // Matrice di propriet? per due forme di schema e due costruttori.
fn raw_and_constructed_plans_reject_every_schema_contract_disagreement() {
    use plenora_io_model::contract::{CoordinateDimensions, CoordinatePrecision, GeometryEncoding};
    let input = schema(Some("91"));
    let mut primary = GeometryColumnContract::wkb_xy(FieldId(1), "g1", ResolvedCrs::wgs84(), true);
    plenora_io_model::geometry::read_geometry_contract_metadata(input.field(1), &mut primary)
        .unwrap();
    let mut cases = Vec::new();
    let external = Arc::new(Schema::new(vec![arrow_schema::Field::new(
        "g",
        arrow_schema::DataType::Binary,
        true,
    )
    .with_metadata(
        std::iter::once(("ARROW:extension:name", "geoarrow.wkb"))
            .collect::<arrow_schema::Metadata>(),
    )]));
    for (input, primary) in [
        (input.clone(), primary.clone()),
        (
            external.clone(),
            DataContract::from_schema(external)
                .unwrap()
                .geometry
                .unwrap(),
        ),
    ] {
        for mutation in 0..12 {
            let mut g = primary.clone();
            match mutation {
                0 => g.identita_dichiarata = Some(999),
                1 => g.encoding = GeometryEncoding::Ewkb,
                2 => g.dimensions = CoordinateDimensions::Xyz,
                3 => g.precision = CoordinatePrecision::Float32,
                4 => g.geometry_types = vec![GeometryType::Polygon],
                5 => {
                    g.crs = if g.crs == CrsResolution::Missing {
                        ResolvedCrs::wgs84().into()
                    } else {
                        CrsResolution::Missing
                    }
                }
                6 => g.nullable = false,
                7 => g.name = "other".into(),
                8 => g.srid = Some(3003),
                9 => g.spatial_semantics = plenora_io_model::contract::SpatialSemantics::Geography,
                10 => {
                    g.types_declaration = if g.types_declaration
                        == plenora_io_model::contract::TypesDeclaration::Exact
                    {
                        plenora_io_model::contract::TypesDeclaration::Mixed
                    } else {
                        plenora_io_model::contract::TypesDeclaration::Exact
                    }
                }
                _ => {
                    g.native_metadata
                        .insert("source.test".into(), "different".into());
                }
            }
            cases.push((input.clone(), Some(g)));
        }
    }
    cases.push((schema(None), Some(primary.clone())));
    let single = Arc::new(Schema::new_with_metadata(
        vec![input.field(1).as_ref().clone()],
        schema(None).metadata().clone(),
    ));
    cases.push((single, None));
    cases.push((
        Arc::new(Schema::new(vec![arrow_schema::Field::new(
            "g1",
            arrow_schema::DataType::Binary,
            true,
        )])),
        Some(primary.clone()),
    ));
    for field_index in [0, 1] {
        let mut fields = input
            .fields()
            .iter()
            .map(|f| f.as_ref().clone())
            .collect::<Vec<_>>();
        let mut md = fields[field_index].metadata().clone();
        md.remove("plenora.field_id");
        fields[field_index] = fields[field_index].clone().with_metadata(md);
        cases.push((
            Arc::new(Schema::new_with_metadata(
                fields,
                schema(None).metadata().clone(),
            )),
            Some(primary.clone()),
        ));
    }
    for (input, geometry) in cases {
        for constructed in [false, true] {
            let contract = if constructed {
                DataContract::new(input.clone(), geometry.clone())
            } else {
                DataContract {
                    schema: input.clone(),
                    geometry: geometry.clone(),
                }
            };
            assert_eq!(contract.schema, input);
            assert!(
                contract.geometries().is_err(),
                "unexpected valid contract: {contract:?}"
            );
            assert_eq!(
                contract.geometries().unwrap_err().category,
                plenora_io_model::ErrorCategory::Schema
            );
            let dir = tempfile::tempdir().unwrap();
            let output = dir.path().join("rejected.arrow");
            let plan = WritePlan {
                layers: vec![WriteLayer {
                    name: "l".into(),
                    contract: contract.clone(),
                }],
            };
            let error = IpcDriver
                .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
                .err()
                .unwrap();
            assert_eq!(error.category, plenora_io_model::ErrorCategory::Schema);
            assert!(!output.exists());
            let layer = LayerContract {
                id: LayerId(0),
                name: "l".into(),
                contract,
            };
            let request = ReadRequest {
                layer: LayerId(0),
                projected_fields: None,
                projection_mode: plenora_io_core::ProjectionMode::Required,
                pruning_predicate: None,
                spatial_pruning_hint: None,
                scope: plenora_io_core::ReadScope::default(),
                batch_target: plenora_io_core::BatchTarget::default(),
                cancellation: plenora_io_model::CancellationToken::default(),
            };
            assert_eq!(
                plenora_io_core::project_layer_contract(&layer, &request)
                    .unwrap_err()
                    .category,
                plenora_io_model::ErrorCategory::Schema
            );
        }
    }
}

#[test]
fn cumulative_component_budget_without_primary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("budget.arrow");
    let schema = schema(None);
    let point = to_wkb(&geo_types::Geometry::Point(geo_types::Point::new(1., 2.))).unwrap();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from(vec![
                Some(point.as_slice()),
                None,
                Some(point.as_slice()),
            ])),
            Arc::new(BinaryArray::from(vec![
                None,
                Some(point.as_slice()),
                Some(point.as_slice()),
            ])),
        ],
    )
    .unwrap();
    let mut writer = FileWriter::try_new(File::create(&path).unwrap(), &schema).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);
    for limit in [3, 4] {
        let budget = plenora_io_model::budget::PipelineBudget::builder()
            .limits(
                plenora_io_model::budget::PipelineLimits::default()
                    .with_max_geometry_components(limit),
            )
            .build()
            .unwrap();
        let dataset = IpcDriver
            .open(
                Source::Path(path.clone()),
                ReadOptions::from_read_parts(budget.into_read_parts()),
            )
            .unwrap();
        let request = ReadRequest {
            layer: LayerId(0),
            projected_fields: None,
            projection_mode: plenora_io_core::ProjectionMode::Required,
            pruning_predicate: None,
            spatial_pruning_hint: None,
            scope: plenora_io_core::ReadScope::default(),
            batch_target: plenora_io_core::BatchTarget::default(),
            cancellation: plenora_io_model::CancellationToken::default(),
        };
        let mut reader = dataset.open_layer_reader(&request).unwrap();
        if limit == 3 {
            assert_eq!(
                reader.next_batch().unwrap_err().category,
                plenora_io_model::ErrorCategory::ResourceLimit
            );
        } else {
            let observed = reader.next_batch().unwrap().unwrap();
            assert_eq!(observed.column(0), batch.column(0));
            assert!(reader.next_batch().unwrap().is_none());
        }
    }
}

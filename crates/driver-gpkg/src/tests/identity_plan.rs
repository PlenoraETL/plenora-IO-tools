use super::*;

#[test]
fn writer_reopened_reader_preserves_noncontiguous_wire_ids() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.gpkg");
    let schema = Arc::new(Schema::new(vec![
        Field::new("FID", DataType::Int64, false)
            .with_metadata([("plenora.field_id".to_owned(), "0".to_owned())]),
        Field::new("geom", DataType::Binary, true),
    ]));
    let mut geometry = GeometryColumnContract::wkb_passthrough(
        FieldId(1),
        "geom",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None)
            .stored_east_first(),
        true,
    );
    geometry.identita_dichiarata = Some(41);
    geometry.set_exact_geometry_types(vec![plenora_io_model::contract::GeometryType::Point]);
    geometry.dimensions = CoordinateDimensions::Xy;
    let schema = plenora_io_model::geometry::schema_from_native_fields(&schema, &geometry);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "features".to_owned(),
            contract: DataContract::new(schema, Some(geometry)),
        }],
    };
    GpkgDriver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap()
        .finish()
        .unwrap();
    let dataset = GpkgDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    let contract = &dataset.layers()[0].contract;
    assert_eq!(
        contract.geometry.as_ref().unwrap().identita_dichiarata,
        Some(41)
    );
    for (name, id) in [("FID", "0"), ("geom", "41"), ("fid_", "1")] {
        let field = contract.schema.field_with_name(name).unwrap();
        assert_eq!(field.metadata()["plenora.field_id"], id);
    }
}

#[test]
fn invalid_metadata_reference_and_registration_are_rejected() {
    for mutation in [
        "UPDATE gpkg_metadata_reference SET reference_scope='row'",
        "UPDATE gpkg_metadata_reference SET column_name='geom'",
        "UPDATE gpkg_metadata_reference SET row_id_value=1",
        "UPDATE gpkg_metadata_reference SET md_parent_id=1",
        "DELETE FROM gpkg_extensions",
    ] {
        let conn = Connection::open_in_memory().unwrap();
        let schema = input();
        contract_plan::ContractPlan::from_schema(&schema)
            .unwrap()
            .persist(&conn, "features")
            .unwrap();
        assert!(contract_plan::restore(&conn, "features", output_fields()).is_ok());
        conn.execute_batch(mutation).unwrap();
        assert!(contract_plan::restore(&conn, "features", output_fields()).is_err());
    }
}

fn input() -> Schema {
    Schema::new(vec![
        Field::new("FID", DataType::Int64, false)
            .with_metadata([("plenora.field_id".to_owned(), "0".to_owned())]),
        geometry_field("geom", "EPSG:4326")
            .with_metadata([("plenora.field_id".to_owned(), "41".to_owned())]),
    ])
}

fn output_fields() -> Vec<Field> {
    let schema = input();
    vec![
        schema.field(1).clone(),
        schema.field(0).clone(),
        Field::new("fid_", DataType::Int64, false),
    ]
}

fn document(conn: &Connection) -> String {
    conn.query_row("SELECT metadata FROM gpkg_metadata", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn identity_map_roundtrip_reserves_all_ids_and_generated_primary() {
    let conn = Connection::open_in_memory().unwrap();
    let schema = input();
    let plan = contract_plan::ContractPlan::from_schema(&schema).unwrap();
    plan.persist(&conn, "features").unwrap();
    let reordered = vec![
        schema.field(1).clone(),
        schema.field(0).clone(),
        Field::new("fid_", DataType::Int64, false),
    ];
    let restored = contract_plan::restore(&conn, "features", reordered).unwrap();
    let ids: Vec<_> = restored
        .iter()
        .map(|f| f.metadata()["plenora.field_id"].as_str())
        .collect();
    assert_eq!(ids, ["41", "0", "1"]);
}

#[test]
fn identity_map_corruption_never_falls_back_to_positions() {
    for mutation in 0..5 {
        let conn = Connection::open_in_memory().unwrap();
        contract_plan::ContractPlan::from_schema(&input())
            .unwrap()
            .persist(&conn, "features")
            .unwrap();
        assert!(contract_plan::restore(&conn, "features", output_fields()).is_ok());
        let original = document(&conn);
        let changed = match mutation {
            0 => original.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
            1 => original.replacen("\"version\":1", "\"version\":2", 1),
            2 => original.replacen("\"field_id\":41", "\"field_id\":0", 1),
            3 => original.replacen("\"column\":\"geom\"", "\"column\":\"missing\"", 1),
            _ => "SECRET_PAYLOAD".to_owned(),
        };
        assert_ne!(changed, original);
        conn.execute("UPDATE gpkg_metadata SET metadata=?1", [changed])
            .unwrap();
        let error = contract_plan::restore(&conn, "features", output_fields()).unwrap_err();
        assert!(!error.to_string().contains("SECRET_PAYLOAD"));
    }
}

#[test]
fn identity_map_rejects_duplicate_and_noncanonical_ids_before_write() {
    for id in ["0", "01", "-1", "4294967296", "SECRET_PAYLOAD"] {
        let schema = input();
        let fields = vec![
            schema.field(0).clone(),
            schema
                .field(1)
                .clone()
                .with_metadata([("plenora.field_id".to_owned(), id.to_owned())]),
        ];
        assert!(contract_plan::ContractPlan::from_schema(&Schema::new(fields)).is_err());
    }
}

#[test]
fn identity_map_absent_is_deterministic_and_oversize_is_rejected() {
    let conn = Connection::open_in_memory().unwrap();
    let fields = vec![
        Field::new("a", DataType::Int64, false),
        Field::new("b", DataType::Int64, false),
    ];
    let restored = contract_plan::restore(&conn, "external", fields).unwrap();
    assert_eq!(restored[0].metadata()["plenora.field_id"], "0");
    assert_eq!(restored[1].metadata()["plenora.field_id"], "1");
    let schema = input();
    contract_plan::ContractPlan::from_schema(&schema)
        .unwrap()
        .persist(&conn, "features")
        .unwrap();
    assert!(contract_plan::restore(&conn, "features", output_fields()).is_ok());
    let large = document(&conn) + &" ".repeat(1_048_577);
    assert!(plenora_io_model::json::from_str::<serde_json::Value>(&large).is_ok());
    conn.execute("UPDATE gpkg_metadata SET metadata=?1", [large])
        .unwrap();
    assert!(contract_plan::restore(&conn, "features", output_fields()).is_err());
}

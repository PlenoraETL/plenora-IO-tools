//! Matrice storage/tipi e regressioni di fedelta' del `GeoPackage`.
use super::*;
use crate::schema::sqlite_declared_to_arrow;

const DECLARATIONS: &[(&str, DataType)] = &[
    ("BOOLEAN", DataType::Int64),
    ("TINYINT", DataType::Int64),
    ("SMALLINT", DataType::Int64),
    ("MEDIUMINT", DataType::Int64),
    ("INT", DataType::Int64),
    ("INTEGER", DataType::Int64),
    ("FLOAT", DataType::Float64),
    ("DOUBLE", DataType::Float64),
    ("REAL", DataType::Float64),
    ("TEXT", DataType::Utf8),
    ("BLOB", DataType::Binary),
    ("DATE", DataType::Utf8),
    ("DATETIME", DataType::Utf8),
    // Classi di normalizzazione delle dichiarazioni esterne non canoniche.
    ("BIGINT", DataType::Int64),
    ("DOUBLE PRECISION", DataType::Float64),
    ("VARCHAR(20)", DataType::Utf8),
    ("BLOB(20)", DataType::Binary),
    ("NUMERIC", DataType::Utf8),
    ("", DataType::Utf8),
];

#[test]
fn ogni_storage_contro_ogni_tipo_dichiarato() {
    let storage = [
        ValueRef::Null,
        ValueRef::Integer(7),
        ValueRef::Real(1.5),
        ValueRef::Text(b"abc"),
        ValueRef::Blob(b"AB"),
    ];
    let mut cases = 0;
    for (declared, expected_type) in DECLARATIONS {
        assert_eq!(sqlite_declared_to_arrow(declared), *expected_type);
        for (index, value) in storage.iter().enumerate() {
            let expected_loss = match (expected_type, index) {
                (DataType::Int64, 2) => Some(Coercion::RealTruncated),
                (DataType::Int64 | DataType::Float64, 3 | 4)
                | (DataType::Utf8, 4)
                | (DataType::Binary, 1..=3) => Some(Coercion::IncompatibleStorage),
                _ => None,
            };
            let mut builder = ColBuilder::new(expected_type);
            assert_eq!(builder.append(*value), expected_loss, "{declared}: {index}");
            let array = builder.finish();
            assert_eq!(
                array.is_null(0),
                index == 0 || expected_loss == Some(Coercion::IncompatibleStorage),
                "{declared}: {index}"
            );
            if !array.is_null(0) {
                match expected_type {
                    DataType::Int64 => assert_eq!(
                        array
                            .as_any()
                            .downcast_ref::<Int64Array>()
                            .unwrap()
                            .value(0),
                        if index == 1 { 7 } else { 1 }
                    ),
                    DataType::Float64 => assert_eq!(
                        array
                            .as_any()
                            .downcast_ref::<Float64Array>()
                            .unwrap()
                            .value(0)
                            .to_bits(),
                        if index == 1 {
                            7.0_f64.to_bits()
                        } else {
                            1.5_f64.to_bits()
                        }
                    ),
                    DataType::Utf8 => assert_eq!(
                        array
                            .as_any()
                            .downcast_ref::<StringArray>()
                            .unwrap()
                            .value(0),
                        ["", "7", "1.5", "abc"][index]
                    ),
                    DataType::Binary => assert_eq!(
                        array
                            .as_any()
                            .downcast_ref::<BinaryArray>()
                            .unwrap()
                            .value(0),
                        b"AB"
                    ),
                    _ => unreachable!(),
                }
            }
            cases += 1;
        }
    }
    assert_eq!(cases, 95);
}

fn external(path: &std::path::Path, columns: &str) -> Connection {
    let conn = Connection::open(path).unwrap();
    init_gpkg(&conn).unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE features (fid INTEGER PRIMARY KEY, geom BLOB, {columns});
        INSERT INTO gpkg_contents (table_name, data_type, identifier, last_change, srs_id)
        VALUES ('features','features','features','2026-01-01T00:00:00.000Z',4326);
        INSERT INTO gpkg_geometry_columns VALUES ('features','geom','POINT',4326,0,0);"
    ))
    .unwrap();
    conn
}

#[test]
fn storage_esterno_e_null_hanno_esiti_distinti() {
    // SQLite applica l'affinita' prima del reader: la matrice precedente
    // include anche coppie che l'affinita' rende irraggiungibili su disco.
    for (declared, _) in DECLARATIONS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("external.gpkg");
        let conn = external(&path, &format!("value {declared}"));
        conn.execute_batch(
            "INSERT INTO features(value) VALUES (NULL),(7),(1.5),('abc'),(X'4142');",
        )
        .unwrap();
        let observed: Vec<(String, bool)> = conn
            .prepare("SELECT typeof(value), value IS NULL FROM features ORDER BY fid")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert_eq!(observed.len(), 5);
        assert!(observed[0].1);
        assert!(observed[1..].iter().all(|(_, null)| !null));
        let expected: u64 = observed
            .iter()
            .filter(|(storage, _)| match sqlite_declared_to_arrow(declared) {
                DataType::Int64 | DataType::Float64 => storage == "text" || storage == "blob",
                DataType::Utf8 => storage == "blob",
                DataType::Binary => storage != "null" && storage != "blob",
                _ => unreachable!(),
            })
            .count()
            .try_into()
            .unwrap();
        drop(conn);
        let ds = GpkgDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let loss = read_all_and_loss(ds.as_ref());
        assert_eq!(
            loss.counts.get(INCOMPATIBLE_STORAGE).copied().unwrap_or(0),
            expected,
            "{declared}"
        );
    }
}

#[test]
fn utf8_invalido_e_carattere_sostitutivo_reale_sono_distinti_nel_report() {
    for (sql, count) in [
        ("CAST(X'80' AS TEXT)", 1),
        ("CAST(X'EFBFBD' AS TEXT)", 0),
        ("NULL", 0),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("utf8.gpkg");
        let conn = external(&path, "value TEXT");
        conn.execute_batch(&format!("INSERT INTO features(value) VALUES ({sql});"))
            .unwrap();
        drop(conn);
        let ds = GpkgDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let loss = read_all_and_loss(ds.as_ref());
        assert_eq!(loss.counts.get(INVALID_UTF8).copied().unwrap_or(0), count);
        assert!(loss
            .esempi_canonici()
            .all(|e| !e.context.contains('\u{fffd}')));
    }
}

#[test]
fn riscrittura_conserva_pk_tipi_e_not_null() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.gpkg");
    let target = dir.path().join("target.gpkg");
    let conn = external(
        &source,
        "flag BOOLEAN NOT NULL, day DATE NOT NULL, label TEXT NOT NULL",
    );
    conn.execute_batch("INSERT INTO features(fid, flag, day, label) VALUES (10,1,'2026-11-02','a'),(100,0,'2026-11-03','b');").unwrap();
    drop(conn);
    let ds = GpkgDriver
        .open(Source::Path(source), opzioni_lettura())
        .unwrap();
    let contract = ds.layers()[0].contract.clone();
    assert!(!contract
        .schema
        .field_with_name("flag")
        .unwrap()
        .is_nullable());
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "features".to_owned(),
            contract,
        }],
    };
    let mut writer = GpkgDriver
        .create(Sink::Path(target.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    let mut reader = ds.open_layer_reader(&richiesta(LayerId(0))).unwrap();
    while let Some(batch) = reader.next_batch().unwrap() {
        writer.write(&batch).unwrap();
    }
    writer.finish().unwrap();
    let conn = Connection::open(&target).unwrap();
    let ids: Vec<i64> = conn
        .prepare("SELECT fid FROM features ORDER BY fid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();
    assert_eq!(ids, [10, 100]);
    let schema: Vec<(String, String, i64, i64)> = conn
        .prepare("PRAGMA table_info(features)")
        .unwrap()
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();
    for (name, declared) in [("flag", "BOOLEAN"), ("day", "DATE"), ("label", "TEXT")] {
        assert!(schema.contains(&(name.to_owned(), declared.to_owned(), 1, 0)));
    }
    assert!(schema.contains(&("fid".to_owned(), "INTEGER".to_owned(), 1, 1)));
    assert!(conn
        .execute("UPDATE features SET label = NULL", [])
        .is_err());
    drop(conn);
    let reread = GpkgDriver
        .open(Source::Path(target), opzioni_lettura())
        .unwrap();
    assert_eq!(
        read_all_and_loss(reread.as_ref())
            .counts
            .get(SCHEMA_NORMALIZED),
        None
    );
}

#[test]
fn schema_non_ricostruibile_e_dichiarato() {
    for decl in [
        "value NUMERIC",
        "value TEXT DEFAULT 'secret'",
        "value INTEGER CHECK(value > 0)",
        "value TEXT UNIQUE",
        "value TEXT COLLATE NOCASE",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schema.gpkg");
        drop(external(&path, decl));
        let ds = GpkgDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let loss = read_all_and_loss(ds.as_ref());
        assert_eq!(loss.counts.get(SCHEMA_NORMALIZED), Some(&1));
        assert!(loss
            .esempi_canonici()
            .all(|e| !e.context.contains("secret")));
    }
}

#[test]
fn layer_misto_dichiara_geometry_e_rilegge_entrambi_i_tipi() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.gpkg");
    let schema = Arc::new(Schema::new(vec![geometry_field("geom", "EPSG:4326")]));
    let mut geometry = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geom".to_owned(),
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None)
            .stored_east_first(),
        true,
    );
    geometry.set_exact_geometry_types(vec![GeometryType::Point, GeometryType::LineString]);
    let contract = DataContract::new(
        plenora_io_model::geometry::schema_from_native_fields(&schema, &geometry),
        Some(geometry),
    );
    let geometries = [
        geo_types::Geometry::Point(geo_types::Point::new(1., 2.)),
        geo_types::Geometry::LineString(geo_types::LineString::from(vec![(1., 2.), (3., 4.)])),
    ];
    let values: Vec<Vec<u8>> = geometries.iter().map(|g| to_wkb(g).unwrap()).collect();
    let batch = RecordBatch::try_new(
        contract.schema.clone(),
        vec![Arc::new(BinaryArray::from_iter_values(
            values.iter().map(Vec::as_slice),
        ))],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "features".to_owned(),
            contract,
        }],
    };
    let mut writer = GpkgDriver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    let conn = Connection::open(&path).unwrap();
    let declared: String = conn
        .query_row(
            "SELECT geometry_type_name FROM gpkg_geometry_columns",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(declared, "GEOMETRY");
    drop(conn);
    let ds = GpkgDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    assert_eq!(
        ds.layers()[0]
            .contract
            .geometry
            .as_ref()
            .unwrap()
            .geometry_types,
        [GeometryType::Point, GeometryType::LineString]
    );
    let mut reader = ds.open_layer_reader(&richiesta(LayerId(0))).unwrap();
    assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 2);
}

#[test]
fn metadati_sql_non_validi_sono_rifiutati_prima_di_creare_il_file() {
    for (declared, primary, nullable) in [
        ("INTEGER); DROP TABLE features; --", None, false),
        ("TEXT", None, false),
        ("INTEGER", Some("false"), false),
        ("INTEGER", Some("true"), true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.gpkg");
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(DECLARED_TYPE.to_owned(), declared.to_owned());
        if let Some(primary) = primary {
            metadata.insert(PRIMARY_KEY.to_owned(), primary.to_owned());
        }
        let schema = Arc::new(Schema::new(vec![
            geometry_field("geom", "EPSG:4326"),
            Field::new("id", DataType::Int64, nullable).with_metadata(metadata),
        ]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "features".to_owned(),
                contract: DataContract::from_schema(schema).unwrap(),
            }],
        };
        assert!(GpkgDriver
            .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
            .is_err());
        assert!(!path.exists());
    }
}

#[test]
fn colonne_generate_conservano_i_valori_e_dichiarano_la_regola_persa() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("generated.gpkg");
    let conn = external(&path, "value INTEGER, doubled INTEGER AS (value * 2)");
    conn.execute_batch("INSERT INTO features(value) VALUES (21);")
        .unwrap();
    drop(conn);
    let ds = GpkgDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    let mut reader = ds.open_layer_reader(&richiesta(LayerId(0))).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    assert_eq!(
        batch
            .column_by_name("doubled")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0),
        42
    );
    assert_eq!(reader.loss_report().counts.get(SCHEMA_NORMALIZED), Some(&1));
    assert!(reader
        .loss_report()
        .esempi_canonici()
        .any(|e| e.context.contains("vincoli SQL")));
}

#[test]
fn attributo_fid_non_marcato_non_collide_con_la_chiave_generata() {
    let conn = Connection::open_in_memory().unwrap();
    init_gpkg(&conn).unwrap();
    let schema = Schema::new(vec![
        geometry_field("geom", "EPSG:4326"),
        Field::new("FID", DataType::Int64, false),
    ]);
    create_feature_table(&conn, "features", &schema, 4326, None, None).unwrap();
    let key: String = conn
        .query_row(
            "SELECT name FROM pragma_table_info('features') WHERE pk = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(key, "fid_");
}

#[test]
fn geometrie_multiple_restano_rifiutate_anche_con_primaria_mista() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("multiple.gpkg");
    let schema = Arc::new(Schema::new(vec![
        geometry_field("geom", "EPSG:4326"),
        geometry_field("second", "EPSG:4326"),
    ]));
    let mut geometry = GeometryColumnContract::wkb_passthrough(
        FieldId(0),
        "geom".to_owned(),
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None)
            .stored_east_first(),
        true,
    );
    geometry.set_exact_geometry_types(vec![GeometryType::Point, GeometryType::LineString]);
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "features".to_owned(),
            contract: DataContract::new(
                plenora_io_model::geometry::schema_from_native_fields(&schema, &geometry),
                Some(geometry),
            ),
        }],
    };
    assert!(GpkgDriver
        .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
        .is_err());
    assert!(!path.exists());
}

#[test]
fn schema_sql_usa_il_confronto_degli_identificatori_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("case.gpkg");
    let conn = external(&path, "value INTEGER NOT NULL");
    conn.execute_batch("INSERT INTO features(value) VALUES (42); UPDATE gpkg_contents SET table_name = 'FEATURES';").unwrap();
    drop(conn);
    let ds = GpkgDriver
        .open(Source::Path(path), opzioni_lettura())
        .unwrap();
    let mut reader = ds.open_layer_reader(&richiesta(LayerId(0))).unwrap();
    assert_eq!(reader.next_batch().unwrap().unwrap().num_rows(), 1);
}

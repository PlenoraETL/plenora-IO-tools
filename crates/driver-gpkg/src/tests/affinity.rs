//! Matrice dichiarazioni/Arrow/affinita', confrontata con SQLite direttamente.
use super::*;

// Tutto il vocabolario ammesso, con le cinque regole SQLite datatype3 §3.1.
const DECLARED: &[(&str, &str, DataType)] = &[
    ("BOOLEAN", "NUMERIC", DataType::Int64),
    ("TINYINT", "INTEGER", DataType::Int64),
    ("SMALLINT", "INTEGER", DataType::Int64),
    ("MEDIUMINT", "INTEGER", DataType::Int64),
    ("INT", "INTEGER", DataType::Int64),
    ("INTEGER", "INTEGER", DataType::Int64),
    ("FLOAT", "REAL", DataType::Float64),
    ("DOUBLE", "REAL", DataType::Float64),
    ("REAL", "REAL", DataType::Float64),
    ("TEXT", "TEXT", DataType::Utf8),
    ("BLOB", "BLOB", DataType::Binary),
    ("DATE", "NUMERIC", DataType::Utf8),
    ("DATETIME", "NUMERIC", DataType::Utf8),
];

fn field(dt: DataType, declared: &str) -> Field {
    Field::new("value", dt, true).with_metadata([(DECLARED_TYPE.to_owned(), declared.to_owned())])
}

#[test]
fn matrice_ogni_declared_type_ogni_arrow_e_affinita() {
    let types = [
        DataType::Boolean,
        DataType::Int8,
        DataType::Int16,
        DataType::Int32,
        DataType::Int64,
        DataType::UInt8,
        DataType::UInt16,
        DataType::UInt32,
        DataType::UInt64,
        DataType::Float16,
        DataType::Float32,
        DataType::Float64,
        DataType::Utf8,
        DataType::LargeUtf8,
        DataType::Utf8View,
        DataType::Binary,
        DataType::LargeBinary,
        DataType::BinaryView,
        DataType::FixedSizeBinary(2),
        DataType::Null,
        DataType::Date32,
        DataType::Date64,
        DataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, None),
        DataType::Decimal128(10, 2),
        DataType::List(Arc::new(Field::new("item", DataType::Int64, true))),
    ];
    let conn = Connection::open_in_memory().unwrap();
    for (declared, affinity, accepted) in DECLARED {
        for spelling in [(*declared).to_owned(), declared.to_ascii_lowercase()] {
            for dt in &types {
                let schema = Schema::new(vec![field(dt.clone(), &spelling)]);
                assert_eq!(
                    schema::validate_sqlite_metadata(&schema).is_ok(),
                    dt == accepted,
                    "{spelling}/{affinity}/{dt}"
                );
            }
            conn.execute_batch(&format!("CREATE TABLE probe(v {spelling});"))
                .unwrap();
            conn.execute_batch("INSERT INTO probe VALUES ('007'),(1.5),(X'303037'),(NULL);")
                .unwrap();
            let storage: Vec<String> = conn
                .prepare("SELECT typeof(v) FROM probe")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(std::result::Result::unwrap)
                .collect();
            let expected = match *affinity {
                "INTEGER" | "NUMERIC" => ["integer", "real", "blob", "null"],
                "REAL" => ["real", "real", "blob", "null"],
                "TEXT" => ["text", "text", "blob", "null"],
                "BLOB" => ["text", "real", "blob", "null"],
                _ => unreachable!(),
            };
            assert_eq!(storage, expected, "{spelling}");
            conn.execute_batch("DROP TABLE probe").unwrap();
        }
    }
}

fn write_values(path: &std::path::Path, field: Field, values: ArrayRef) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![
        geometry_field("geom", "EPSG:4326"),
        field,
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(BinaryArray::from_iter(std::iter::repeat_n(
                None::<&[u8]>,
                values.len(),
            ))),
            values,
        ],
    )
    .unwrap();
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "features".to_owned(),
            contract: DataContract::from_schema(schema).unwrap(),
        }],
    };
    let mut writer = GpkgDriver.create(Sink::Path(path.to_owned()), &plan, &opzioni_scrittura())?;
    writer.write(&batch)?;
    writer.finish()?;
    Ok(())
}

#[test]
fn date_datetime_confrontano_ogni_testo_con_sqlite() {
    let numeric = [
        "007",
        "1.0",
        "1e3",
        " 12 ",
        "+5",
        "-0",
        "1.50",
        "12345678901234567890",
        "1e999",
        "1e-999",
        ".5",
        "1.",
        "-.5E+2",
        "\t\n\r\u{b}\u{c}42 ",
        "1\0",
        "1\0x",
        "1.5\0x",
    ];
    let text = [
        "",
        " ",
        "2026-01-01",
        "2026-01-01T12:00:00Z",
        "abc",
        "NaN",
        "Inf",
        "0x12",
        "1_000",
        "1e",
        "1e+",
        ".",
        "+",
        "1 2",
        "12x",
        "\u{a0}12",
        "１２",
        "\0",
        "x\x001",
    ];
    for declared in ["DATE", "DATETIME", "date", "datetime"] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!("CREATE TABLE probe(v {declared})"))
            .unwrap();
        for (values, should_convert) in [(numeric.as_slice(), true), (text.as_slice(), false)] {
            for value in values {
                conn.execute("INSERT INTO probe VALUES (?1)", [value])
                    .unwrap();
                let storage: String = conn
                    .query_row(
                        "SELECT typeof(v) FROM probe ORDER BY rowid DESC LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(storage != "text", should_convert, "SQLite {value:?}");
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("out.gpkg");
                let result = write_values(
                    &path,
                    field(DataType::Utf8, declared),
                    Arc::new(StringArray::from(vec![None, Some(*value)])),
                );
                assert_eq!(
                    result.is_err(),
                    should_convert,
                    "{declared}/{value:?}: {result:?}"
                );
                if !should_convert {
                    let output = Connection::open(&path).unwrap();
                    let actual: (String, String) = output
                        .query_row(
                            "SELECT typeof(value), value FROM features WHERE value IS NOT NULL",
                            [],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .unwrap();
                    assert_eq!(actual, ("text".to_owned(), (*value).to_owned()));
                }
            }
        }
    }
}

#[test]
fn zero_negativo_rifiutato_per_tutti_i_float_e_dichiarazioni_real() {
    let zero = Float16Array::new_null(1);
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Float16Array::from_iter_values([-zero.value(0)])),
        Arc::new(Float32Array::from(vec![-0.0])),
        Arc::new(Float64Array::from(vec![-0.0])),
    ];
    for array in arrays {
        assert!(arrow_cell_to_sql_ref(&array, 0).is_err());
        let dir = tempfile::tempdir().unwrap();
        assert!(write_values(
            &dir.path().join("zero.gpkg"),
            Field::new("value", array.data_type().clone(), true),
            array
        )
        .is_err());
    }
    for declared in ["REAL", "FLOAT", "DOUBLE", "real", "float", "double"] {
        for value in [
            -0.0_f64,
            0.0,
            -1.5,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            f64::MAX,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("zero.gpkg");
            let result = write_values(
                &path,
                field(DataType::Float64, declared),
                Arc::new(Float64Array::from(vec![value])),
            );
            assert_eq!(
                result.is_err(),
                value.to_bits() == (-0.0_f64).to_bits(),
                "{result:?}"
            );
            if result.is_ok() {
                let output = Connection::open(path).unwrap();
                let actual: f64 = output
                    .query_row("SELECT value FROM features", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(actual.to_bits(), value.to_bits());
            }
        }
    }
}

#[test]
fn zero_negativo_storage_reale_in_colonne_intere_dichiara_la_perdita() {
    for (declared, _, dt) in DECLARED.iter().filter(|(_, _, dt)| *dt == DataType::Int64) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("external.gpkg");
        let conn = Connection::open(&path).unwrap();
        init_gpkg(&conn).unwrap();
        // BLOB conserva REAL -0.0; writable_schema cambia solo il tipo.
        conn.execute_batch(
            "CREATE TABLE features(fid INTEGER PRIMARY KEY, geom BLOB, value BLOB);
            INSERT INTO gpkg_contents(table_name,data_type,identifier,last_change,srs_id)
            VALUES('features','features','features','2026-01-01T00:00:00.000Z',4326);
            INSERT INTO gpkg_geometry_columns VALUES('features','geom','POINT',4326,0,0);",
        )
        .unwrap();
        conn.execute("INSERT INTO features(value) VALUES (?1)", [-0.0_f64])
            .unwrap();
        conn.execute("INSERT INTO features(value) VALUES (?1)", [0.0_f64])
            .unwrap();
        conn.execute_batch(&format!("PRAGMA writable_schema=ON;
            UPDATE sqlite_master SET sql=replace(sql,'value BLOB','value {declared}') WHERE name='features';
            PRAGMA writable_schema=OFF;")).unwrap();
        drop(conn);
        let conn = Connection::open(&path).unwrap();
        let raw: (String, f64) = conn
            .query_row(
                "SELECT typeof(value),value FROM features WHERE fid=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(raw.0, "real");
        assert_eq!(raw.1.to_bits(), (-0.0_f64).to_bits());
        let mut builder = ColBuilder::new(dt);
        assert_eq!(
            builder.append(ValueRef::Real(raw.1)),
            Some(Coercion::RealTruncated)
        );
        let ds = GpkgDriver
            .open(Source::Path(path), opzioni_lettura())
            .unwrap();
        let loss = read_all_and_loss(ds.as_ref());
        assert_eq!(
            loss.counts.get(INTEGER_COLUMN_REAL_TRUNCATED),
            Some(&1),
            "{declared}"
        );
    }
}

#[test]
fn valori_di_ogni_tipo_arrow_restano_esatti_per_ogni_ddl_ammesso() {
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(BooleanArray::from(vec![Some(false), Some(true), None])),
        Arc::new(Int8Array::from(vec![Some(i8::MIN), Some(i8::MAX), None])),
        Arc::new(Int16Array::from(vec![Some(i16::MIN), Some(i16::MAX), None])),
        Arc::new(Int32Array::from(vec![Some(i32::MIN), Some(i32::MAX), None])),
        Arc::new(Int64Array::from(vec![Some(i64::MIN), Some(i64::MAX), None])),
        Arc::new(UInt8Array::from(vec![Some(0), Some(u8::MAX), None])),
        Arc::new(UInt16Array::from(vec![Some(0), Some(u16::MAX), None])),
        Arc::new(UInt32Array::from(vec![Some(0), Some(u32::MAX), None])),
        Arc::new(UInt64Array::from(vec![
            Some(0),
            Some(i64::MAX.unsigned_abs()),
            None,
        ])),
        Arc::new(Float16Array::from(vec![
            Some(Float16Array::new_null(1).value(0)),
            None,
        ])),
        Arc::new(Float32Array::from(vec![
            Some(f32::MIN),
            Some(f32::MAX),
            None,
        ])),
        Arc::new(Float64Array::from(vec![
            Some(f64::MIN),
            Some(f64::MAX),
            None,
        ])),
        Arc::new(StringArray::from(vec![
            Some("2026-01-01"),
            Some("x\0y"),
            None,
        ])),
        Arc::new(LargeStringArray::from(vec![
            Some("007"),
            Some("x\0y"),
            None,
        ])),
        Arc::new(StringViewArray::from(vec![Some("007"), Some("x\0y"), None])),
        Arc::new(BinaryArray::from(vec![Some(b"007".as_slice()), None])),
        Arc::new(LargeBinaryArray::from(vec![Some(b"007".as_slice()), None])),
        Arc::new(BinaryViewArray::from(vec![Some(b"007".as_slice()), None])),
        Arc::new(FixedSizeBinaryArray::try_from_iter([b"00", b"07"].into_iter()).unwrap()),
    ];
    for array in arrays {
        let declarations = std::iter::once(None).chain(
            DECLARED
                .iter()
                .filter(|(_, _, dt)| dt == array.data_type())
                .map(|(name, _, _)| Some(*name)),
        );
        for declared in declarations {
            let field = declared.map_or_else(
                || Field::new("value", array.data_type().clone(), true),
                |name| field(array.data_type().clone(), name),
            );
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("values.gpkg");
            write_values(&path, field, array.clone()).unwrap();
            let conn = Connection::open(path).unwrap();
            let mut stmt = conn
                .prepare("SELECT value FROM features ORDER BY fid")
                .unwrap();
            let mut rows = stmt.query([]).unwrap();
            for i in 0..array.len() {
                let row = rows.next().unwrap().unwrap();
                let raw = row.get_ref(0).unwrap();
                match (arrow_cell_to_sql_ref(&array, i).unwrap(), raw) {
                    (BorrowedSqlValue::Null, ValueRef::Null) => (),
                    (BorrowedSqlValue::Integer(a), ValueRef::Integer(b)) => assert_eq!(a, b),
                    (BorrowedSqlValue::Real(a), ValueRef::Real(b)) => {
                        assert_eq!(a.to_bits(), b.to_bits());
                    }
                    (BorrowedSqlValue::Text(a), ValueRef::Text(b)) => assert_eq!(a.as_bytes(), b),
                    (BorrowedSqlValue::Blob(a), ValueRef::Blob(b)) => assert_eq!(a, b),
                    pair => panic!("{declared:?}/{:?}: {pair:?}", array.data_type()),
                }
            }
            assert!(rows.next().unwrap().is_none());
        }
    }
}

#[test]
fn grammatica_numerica_differenziale_sqlite() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE probe(v NUMERIC)").unwrap();
    let field = field(DataType::Utf8, "DATE");
    let alphabet = b"01.eE+- x\t\n\r\x0b\x0c\0";
    let mut state = 17_u64;
    for i in 0..10000 {
        let mut value = String::new();
        for _ in 0..(i % 12) {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let index =
                usize::try_from((state >> 32) % u64::try_from(alphabet.len()).unwrap()).unwrap();
            value.push(char::from(alphabet[index]));
        }
        conn.execute("DELETE FROM probe", []).unwrap();
        conn.execute("INSERT INTO probe VALUES (?1)", [&value])
            .unwrap();
        let remains_text: bool = conn
            .query_row("SELECT typeof(v) = 'text' FROM probe", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            crate::affinity::validate_value(&field, &BorrowedSqlValue::Text(&value)).is_ok(),
            remains_text,
            "{value:?}"
        );
    }
}

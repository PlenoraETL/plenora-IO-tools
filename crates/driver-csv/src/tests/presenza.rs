//! Oracoli lessicali indipendenti e dominio dei tipi del writer CSV.
use super::*;
use arrow_schema::{DataType, IntervalUnit, TimeUnit, UnionFields, UnionMode};

fn write_attribute(path: &Path, column: ArrayRef) -> Result<Published> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        column.data_type().clone(),
        true,
    )]));
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "layer".to_owned(),
            contract: DataContract::new(schema.clone(), None),
        }],
    };
    let mut writer = CsvDriver.create(Sink::Path(path.to_owned()), &plan, &opzioni_scrittura())?;
    writer.write(&RecordBatch::try_new(schema, vec![column]).unwrap())?;
    writer.finish()
}

#[test]
fn testo_presenza_e_quoting_round_trip() {
    let values = vec![
        None,
        Some(""),
        Some(" "),
        Some("x"),
        Some("001"),
        Some("1e3"),
        Some(" 12 "),
        Some("true"),
        Some("NaN"),
        Some("a,\"b\r\nc"),
        Some("é"),
    ];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text.csv");
    write_attribute(&path, Arc::new(StringArray::from(values.clone()))).unwrap();
    let bytes = std::fs::read_to_string(&path).unwrap();
    assert_eq!(bytes, "value\r\n\r\n\"\"\r\n\" \"\r\n\"x\"\r\n\"001\"\r\n\"1e3\"\r\n\" 12 \"\r\n\"true\"\r\n\"NaN\"\r\n\"a,\"\"b\r\nc\"\r\n\"é\"\r\n");
    // Il reader pubblico richiede geometria: aggiungerla senza trasformare le celle.
    let mut wire = wire::Reader::new(bytes.as_bytes(), b',');
    let mut record = csv::StringRecord::new();
    wire.read_record(&mut record).unwrap();
    let mut builder = InferredColumnBuilder::new(ColType::Text);
    for expected in &values {
        assert!(wire.read_record(&mut record).unwrap());
        assert_eq!(wire.quoted[0], expected.is_some());
        if let Some(value) = expected {
            assert_eq!(&record[0], *value);
            builder.append_csv_cell(&record[0]).unwrap();
        } else {
            builder.append_null();
        }
    }
    assert!(!wire.read_record(&mut record).unwrap());
    assert_eq!(builder.finish().as_ref(), &StringArray::from(values));
}

#[test]
fn inferenza_e_reader_pubblico_conservano_il_testo() {
    for value in [
        "001",
        "1e3",
        " 12 ",
        "NaN",
        "-0.0",
        "1.0",
        "+1",
        "TRUE",
        " ",
        "9007199254740993.0",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.csv");
        std::fs::write(&path, format!("value,x,y\n{value},1,2\n")).unwrap();
        let ds = CsvDriver
            .open(
                Source::Path(path),
                read_opts(&[("x_column", "x"), ("y_column", "y")]),
            )
            .unwrap();
        let mut reader = ds.open_layer_reader(&req(1)).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let actual = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert!(actual.is_valid(0));
        assert_eq!(actual.value(0), value);
    }
    for value in ["true", "12", "0.5", ""] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.csv");
        std::fs::write(&path, format!("value,x,y\n\"{value}\",1,2\n,3,4\n")).unwrap();
        let ds = CsvDriver
            .open(
                Source::Path(path),
                read_opts(&[("x_column", "x"), ("y_column", "y")]),
            )
            .unwrap();
        let mut reader = ds.open_layer_reader(&req(10)).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        let actual = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(actual.value(0), value);
        assert!(actual.is_valid(0));
        assert!(actual.is_null(1));
    }
}

#[test]
fn tutti_i_tipi_scrivibili_hanno_un_oracolo_e_una_perdita_di_schema() {
    let cases: Vec<(ArrayRef, &str)> = vec![
        (
            Arc::new(BooleanArray::from(vec![true, false])),
            "true\r\nfalse\r\n",
        ),
        (
            Arc::new(arrow_array::Int8Array::from(vec![i8::MIN, i8::MAX])),
            "-128\r\n127\r\n",
        ),
        (
            Arc::new(arrow_array::Int16Array::from(vec![i16::MIN, i16::MAX])),
            "-32768\r\n32767\r\n",
        ),
        (
            Arc::new(arrow_array::Int32Array::from(vec![i32::MIN, i32::MAX])),
            "-2147483648\r\n2147483647\r\n",
        ),
        (
            Arc::new(Int64Array::from(vec![i64::MIN, i64::MAX])),
            "-9223372036854775808\r\n9223372036854775807\r\n",
        ),
        (
            Arc::new(arrow_array::UInt8Array::from(vec![0, u8::MAX])),
            "0\r\n255\r\n",
        ),
        (
            Arc::new(arrow_array::UInt16Array::from(vec![0, u16::MAX])),
            "0\r\n65535\r\n",
        ),
        (
            Arc::new(arrow_array::UInt32Array::from(vec![0, u32::MAX])),
            "0\r\n4294967295\r\n",
        ),
        (
            Arc::new(arrow_array::UInt64Array::from(vec![0, u64::MAX])),
            "0\r\n18446744073709551615\r\n",
        ),
        (
            Arc::new(arrow_array::Float32Array::from(vec![-0.0, 0.5])),
            "-0.0\r\n0.5\r\n",
        ),
        (
            Arc::new(Float64Array::from(vec![-0.0, 0.5])),
            "-0\r\n0.5\r\n",
        ),
        (
            Arc::new(arrow_array::LargeStringArray::from(vec!["", "001"])),
            "\"\"\r\n\"001\"\r\n",
        ),
    ];
    for (array, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.csv");
        let published = write_attribute(&path, array).unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            format!("value\r\n{expected}")
        );
        assert_eq!(
            published.loss.counts.get("coercion tipo attributo"),
            Some(&1)
        );
    }
}

#[test]
fn non_finiti_rifiutati_senza_pubblicazione() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(0x7ff8_0000_0000_0001),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.csv");
        assert!(write_attribute(&path, Arc::new(Float64Array::from(vec![value]))).is_err());
        assert!(!path.exists());
    }
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.csv");
        assert!(write_attribute(
            &path,
            Arc::new(arrow_array::Float32Array::from(vec![value]))
        )
        .is_err());
        assert!(!path.exists());
    }
}

#[test]
fn dominio_arrow_rifiutato_anche_senza_righe() {
    let item = Arc::new(Field::new("item", DataType::Int32, true));
    let mut types = vec![
        DataType::Null,
        DataType::Float16,
        DataType::Utf8View,
        DataType::Binary,
        DataType::LargeBinary,
        DataType::FixedSizeBinary(4),
        DataType::BinaryView,
        DataType::Decimal32(9, 2),
        DataType::Decimal64(18, 2),
        DataType::Decimal128(38, 2),
        DataType::Decimal256(76, 2),
        DataType::List(item.clone()),
        DataType::LargeList(item.clone()),
        DataType::FixedSizeList(item.clone(), 2),
        DataType::ListView(item.clone()),
        DataType::LargeListView(item.clone()),
        DataType::Struct(vec![item.clone()].into()),
        DataType::Map(item.clone(), false),
        DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
        DataType::RunEndEncoded(item.clone(), item),
        DataType::Union(UnionFields::empty(), UnionMode::Dense),
        DataType::Union(UnionFields::empty(), UnionMode::Sparse),
    ];
    types.extend([
        DataType::Time32(TimeUnit::Microsecond),
        DataType::Time32(TimeUnit::Nanosecond),
        DataType::Time64(TimeUnit::Second),
        DataType::Time64(TimeUnit::Millisecond),
    ]);
    for unit in [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ] {
        types.push(DataType::Duration(unit));
    }
    for unit in [
        IntervalUnit::YearMonth,
        IntervalUnit::DayTime,
        IntervalUnit::MonthDayNano,
    ] {
        types.push(DataType::Interval(unit));
    }
    for data_type in types {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.csv");
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            data_type.clone(),
            true,
        )]));
        let plan = WritePlan {
            layers: vec![WriteLayer {
                name: "layer".to_owned(),
                contract: DataContract::new(schema, None),
            }],
        };
        assert!(
            CsvDriver
                .create(Sink::Path(path.clone()), &plan, &opzioni_scrittura())
                .is_err(),
            "{data_type:?}"
        );
        assert!(!path.exists());
    }
}

#[test]
fn parser_rifiuta_quoting_ambiguo_e_utf8_invalido() {
    for bytes in [
        b"a\n\"unterminated".as_slice(),
        b"a\n\"x\"tail\n",
        b"a\nx\"y\n",
        b"a\n\xff\n",
    ] {
        let mut reader = wire::Reader::new(bytes, b',');
        let mut record = csv::StringRecord::new();
        assert!(reader.read_record(&mut record).unwrap());
        assert!(reader.read_record(&mut record).is_err());
    }
}

#[test]
fn temporali_testo_esatto_e_perdita_dichiarata() {
    use arrow_array::*;
    let mut cases: Vec<(ArrayRef, String)> = vec![
        (Arc::new(Date32Array::from(vec![-1])), "1969-12-31".into()),
        (
            Arc::new(Date64Array::from(vec![-86_400_000])),
            "1969-12-31".into(),
        ),
        (
            Arc::new(Time32SecondArray::from(vec![86_399])),
            "23:59:59".into(),
        ),
        (
            Arc::new(Time32MillisecondArray::from(vec![86_399_999])),
            "23:59:59.999".into(),
        ),
        (
            Arc::new(Time64MicrosecondArray::from(vec![86_399_999_999])),
            "23:59:59.999999".into(),
        ),
        (
            Arc::new(Time64NanosecondArray::from(vec![86_399_999_999_999])),
            "23:59:59.999999999".into(),
        ),
    ];
    for zone in [None, Some("UTC"), Some("Europe/Rome"), Some("+05:30")] {
        let suffix = if zone.is_some() { "Z" } else { "" };
        cases.extend([
            (
                Arc::new(TimestampSecondArray::from(vec![-1]).with_timezone_opt(zone)) as ArrayRef,
                format!("1969-12-31T23:59:59{suffix}"),
            ),
            (
                Arc::new(TimestampMillisecondArray::from(vec![-1]).with_timezone_opt(zone))
                    as ArrayRef,
                format!("1969-12-31T23:59:59.999{suffix}"),
            ),
            (
                Arc::new(TimestampMicrosecondArray::from(vec![-1]).with_timezone_opt(zone))
                    as ArrayRef,
                format!("1969-12-31T23:59:59.999999{suffix}"),
            ),
            (
                Arc::new(TimestampNanosecondArray::from(vec![-1]).with_timezone_opt(zone))
                    as ArrayRef,
                format!("1969-12-31T23:59:59.999999999{suffix}"),
            ),
        ]);
    }
    for (array, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("time.csv");
        let published = write_attribute(&path, array).unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            format!("value\r\n\"{expected}\"\r\n")
        );
        assert_eq!(
            published.loss.counts.get("coercion tipo attributo"),
            Some(&1)
        );
    }
    let invalid: Vec<ArrayRef> = vec![
        Arc::new(Date64Array::from(vec![1])),
        Arc::new(Date64Array::from(vec![-1])),
        Arc::new(Time32SecondArray::from(vec![-1])),
        Arc::new(Time32SecondArray::from(vec![86_400])),
        Arc::new(Time32MillisecondArray::from(vec![86_400_000])),
        Arc::new(Time64MicrosecondArray::from(vec![86_400_000_000])),
        Arc::new(Time64NanosecondArray::from(vec![86_400_000_000_000])),
    ];
    for array in invalid {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.csv");
        assert!(write_attribute(&path, array).is_err());
        assert!(!path.exists());
    }
}

#[test]
fn round_trip_pubblico_presenza_e_testo() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.csv");
    std::fs::write(&path, "value,x,y\n,1,2\n\"\",1,2\n\" \",1,2\nx,1,2\n001,1,2\n1e3,1,2\n 12 ,1,2\ntrue,1,2\nNaN,1,2\n").unwrap();
    let ds = CsvDriver
        .open(
            Source::Path(path),
            read_opts(&[("x_column", "x"), ("y_column", "y")]),
        )
        .unwrap();
    let mut reader = ds.open_layer_reader(&req(100)).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    let expected = StringArray::from(vec![
        None,
        Some(""),
        Some(" "),
        Some("x"),
        Some("001"),
        Some("1e3"),
        Some(" 12 "),
        Some("true"),
        Some("NaN"),
    ]);
    assert_eq!(batch.column(1).as_ref(), &expected);
    let output = dir.path().join("output.csv");
    let plan = WritePlan {
        layers: vec![WriteLayer {
            name: "layer".into(),
            contract: ds.layers()[0].contract.clone(),
        }],
    };
    let mut writer = CsvDriver
        .create(Sink::Path(output.clone()), &plan, &opzioni_scrittura())
        .unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    let ds = CsvDriver
        .open(
            Source::Path(output),
            read_opts(&[("wkt_column", "geometry")]),
        )
        .unwrap();
    let mut reader = ds.open_layer_reader(&req(100)).unwrap();
    assert_eq!(
        reader.next_batch().unwrap().unwrap().column(1).as_ref(),
        &expected
    );
}

#[test]
fn seconda_passata_non_accetta_una_grafia_diversa() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.csv");
    std::fs::write(&path, "value,x,y\n1,1,2\n").unwrap();
    let ds = CsvDriver
        .open(
            Source::Path(path.clone()),
            read_opts(&[("x_column", "x"), ("y_column", "y")]),
        )
        .unwrap();
    std::fs::write(&path, "value,x,y\n001,1,2\n").unwrap();
    let mut reader = ds.open_layer_reader(&req(100)).unwrap();
    let error = reader.next_batch().unwrap_err();
    assert!(!error.message.contains("001"));
}

#[test]
fn tutta_la_colonna_deve_essere_lessicalmente_esatta() {
    for (rows, expected) in [
        ("1\n0.5\n", ColType::Number),
        ("9007199254740993\n0.5\n", ColType::Text),
        ("1\n001\n", ColType::Text),
        ("1\n1e3\n", ColType::Text),
        ("true\nfalse\n", ColType::Boolean),
        ("true\nTRUE\n", ColType::Text),
        ("1\n\n2\n", ColType::Integer),
        ("1\n\"\"\n", ColType::Text),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("column.csv");
        std::fs::write(&path, format!("value\n{rows}")).unwrap();
        let inferred =
            infer_types(&path, b',', &["value".into()], &HashSet::new(), quote(100)).unwrap();
        assert_eq!(inferred, vec![(0, expected)], "{rows}");
    }
}

#[test]
fn separatori_bom_multilinea_e_vuoti_finali() {
    for delimiter in *b",;\t|" {
        let text = format!("a{}\"b\r\nc", char::from(delimiter));
        let mut bytes = Vec::new();
        {
            let mut writer = wire::Writer::new(&mut bytes, delimiter);
            writer
                .write_record(["\u{feff}name", "null", "empty"])
                .unwrap();
            writer.write_text(&text).unwrap();
            writer.write_field("").unwrap();
            writer.write_text("").unwrap();
            writer.write_record(None::<&[u8]>).unwrap();
            writer.flush().unwrap();
        }
        // Il parser indipendente conferma escaping e valori; la presenza ha
        // un oracolo separato perché csv::Reader scarta il quoting.
        let mut independent = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .from_reader(bytes.as_slice());
        let record = independent.records().next().unwrap().unwrap();
        assert_eq!(&record[0], text);
        assert_eq!(&record[1], "");
        assert_eq!(&record[2], "");
        let mut reader = wire::Reader::new(bytes.as_slice(), delimiter);
        let mut record = csv::StringRecord::new();
        assert!(reader.read_record(&mut record).unwrap());
        assert_eq!(&record[0], "\u{feff}name");
        assert!(reader.read_record(&mut record).unwrap());
        assert_eq!(reader.quoted, vec![true, false, true]);
        assert_eq!(&record[0], text);
    }
    let mut reader = wire::Reader::new(b"\xef\xbb\xbf\"value\"\r\n\"\"".as_slice(), b',');
    let mut record = csv::StringRecord::new();
    assert!(reader.read_record(&mut record).unwrap());
    assert_eq!(&record[0], "value");
    assert!(reader.read_record(&mut record).unwrap());
    assert_eq!(reader.quoted, vec![true]);
    assert!(!reader.read_record(&mut record).unwrap());
}

#[test]
fn coordinate_non_finite_non_diventano_punti_vuoti() {
    for (x, y) in [
        ("NaN", "NaN"),
        ("NaN", "2"),
        ("1", "NaN"),
        ("inf", "2"),
        ("1", "-inf"),
        ("1e999", "2"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.csv");
        std::fs::write(&path, format!("x,y\n{x},{y}\n")).unwrap();
        let result = CsvDriver.open(
            Source::Path(path),
            read_opts(&[("x_column", "x"), ("y_column", "y")]),
        );
        match result {
            Err(error) => assert!(error.message.contains("coordinate CSV non finite")),
            Ok(ds) => {
                let mut reader = ds.open_layer_reader(&req(1)).unwrap();
                let error = reader.next_batch().unwrap_err();
                assert!(error.message.contains("coordinate CSV non finite"));
            }
        }
    }
}

#[test]
fn quoting_rigoroso_anche_a_cavallo_dei_buffer() {
    // Sposta ogni carattere speciale prima, sopra e dopo il confine di 8 KiB
    // del BufReader, includendo UTF-8 spezzato e celle oltre due buffer.
    for length in (8188..8196).chain(16380..16388) {
        let text = "x".repeat(length);
        for (input, expected, quoted) in [
            (format!("{text}è,z\r\n"), format!("{text}è"), false),
            (
                format!("\"{text}\"\"è\r\n,z\",z\r\n"),
                format!("{text}\"è\r\n,z"),
                true,
            ),
        ] {
            let mut reader = wire::Reader::new(input.as_bytes(), b',');
            let mut record = csv::StringRecord::new();
            assert!(reader.read_record(&mut record).unwrap());
            assert_eq!(&record[0], expected);
            assert_eq!(&record[1], "z");
            assert_eq!(reader.quoted, vec![quoted, false]);
            assert!(!reader.read_record(&mut record).unwrap());
        }
        for input in [
            format!("{text}\"oops,z\n").into_bytes(),
            format!("\"{text}\"oops,z\n").into_bytes(),
            format!("\"{text}").into_bytes(),
            [text.as_bytes(), b"\xff,z\n"].concat(),
            format!("a,b\n{text},z,extra\n").into_bytes(),
        ] {
            let mut reader = wire::Reader::new(input.as_slice(), b',');
            let mut record = csv::StringRecord::new();
            loop {
                match reader.read_record(&mut record) {
                    Ok(true) => (),
                    Ok(false) => panic!("CSV malformato accettato"),
                    Err(error) => {
                        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                        break;
                    }
                }
            }
        }
    }
}

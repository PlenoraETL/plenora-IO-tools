//! Oracolo fisico: i byte del record, non soltanto il numero riletto.
use super::*;
use arrow_array::*;

fn batch(array: ArrayRef) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        geometry_field(GEOMETRY, "EPSG:4326"),
        Field::new("valore", array.data_type().clone(), true),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(BinaryArray::from(vec![None::<&[u8]>; array.len()])),
            array,
        ],
    )
    .unwrap()
}

fn writer(path: &Path, batch: &RecordBatch) -> Box<dyn FormatWriter> {
    ShpDriver
        .create(
            Sink::Path(path.to_owned()),
            &WritePlan {
                layers: vec![WriteLayer {
                    name: "l".to_owned(),
                    contract: DataContract::from_schema(batch.schema()).unwrap(),
                }],
            },
            &if path.extension().is_some_and(|ext| ext == "d") {
                opzioni_scrittura()
            } else {
                opzioni_scrittura_loose()
            },
        )
        .unwrap()
}

fn rifiutato(array: ArrayRef) {
    let invalid = batch(array);
    for name in ["out.shp", "out.shp.d"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        let mut w = writer(&path, &invalid);
        w.declare_input_total(LayerId(0), 2).unwrap();
        // Anche un batch precedente valido deve restare privato.
        let valid = batch(new_null_array(invalid.column(1).data_type(), 1));
        w.write(&valid).unwrap();
        let error = w.write(&invalid).unwrap_err();
        assert!(!error.to_string().contains("valore"));
        assert_eq!(
            error
                .row_diagnostics
                .as_ref()
                .expect("il rifiuto porta la diagnostica strutturale")
                .counts
                .get("shapefile.cell_not_representable"),
            Some(&1)
        );
        assert!(w.finish().is_err());
        assert!(!path.exists());
        for ext in ["shp", "shx", "dbf", "prj", "cpg"] {
            assert!(!path.with_extension(ext).exists());
        }
    }
}

fn scritto(array: ArrayRef, expected: &[u8], width: usize, scale: u8) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.shp");
    let batch = batch(array);
    let mut w = writer(&path, &batch);
    w.write(&batch).unwrap();
    w.finish().unwrap();
    let dbf = std::fs::read(path.with_extension("dbf")).unwrap();
    let start = usize::from(u16::from_le_bytes([dbf[8], dbf[9]]));
    assert_eq!(usize::from(dbf[48]), width);
    assert_eq!(dbf[49], scale);
    assert_eq!(std::fs::read(path.with_extension("cpg")).unwrap(), b"UTF-8");
    assert_eq!(dbf[29], 0xf0, "codepage UTF-8 dichiarata");
    assert_eq!(dbf[start], b' ');
    assert_eq!(&dbf[start + 1..start + 1 + width], expected);
}

#[test]
fn testo_limiti_utf8_e_non_bmp() {
    for text in ["a".repeat(255), format!("{}😀", "a".repeat(253))] {
        rifiutato(Arc::new(StringArray::from(vec![text])));
    }
    for text in [
        "a".repeat(253),
        "a".repeat(254),
        format!("{}😀", "a".repeat(250)),
    ] {
        let mut bytes = text.as_bytes().to_vec();
        bytes.resize(254, b' ');
        scritto(Arc::new(StringArray::from(vec![text])), &bytes, 254, 0);
    }
}

#[test]
fn numeri_non_rappresentabili_non_pubblicano() {
    for n in [
        (1_i64 << 53) + 1,
        i64::MAX,
        i64::MIN,
        1_000_000_000_000_000_000,
    ] {
        rifiutato(Arc::new(Int64Array::from(vec![n])));
    }
    rifiutato(Arc::new(UInt64Array::from(vec![u64::MAX])));
    for n in [
        0.123_456_789,
        1e11,
        -1e10,
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        rifiutato(Arc::new(Float64Array::from(vec![n])));
    }
    rifiutato(Arc::new(Float32Array::from(vec![0.1])));
}

#[test]
fn numeri_esatti_byte_del_dbf() {
    for (n, expected) in [
        ((1_i64 << 53) - 1, "  9007199254740991"),
        (1_i64 << 53, "  9007199254740992"),
        (1_i64 << 53 | 2, "  9007199254740994"),
        (100_000_000_000_000_000, "100000000000000000"),
        (-10_000_000_000_000_000, "-10000000000000000"),
    ] {
        // Il padding atteso e' indipendente dal formatter del driver.
        scritto(
            Arc::new(Int64Array::from(vec![n])),
            expected.as_bytes(),
            18,
            0,
        );
    }
    for (n, expected) in [
        (0.125, "          0.12500000"),
        (-0.0, "         -0.00000000"),
        (0.123_456_78, "          0.12345678"),
        (99_999_999_999.0, "99999999999.00000000"),
        (-9_999_999_999.0, "-9999999999.00000000"),
    ] {
        scritto(
            Arc::new(Float64Array::from(vec![n])),
            expected.as_bytes(),
            20,
            8,
        );
    }
}

#[test]
fn tipi_arrow_nativi_e_temporali() {
    let integers: Vec<ArrayRef> = vec![
        Arc::new(Int8Array::from(vec![i8::MIN])),
        Arc::new(Int16Array::from(vec![i16::MIN])),
        Arc::new(Int32Array::from(vec![i32::MIN])),
        Arc::new(UInt8Array::from(vec![u8::MAX])),
        Arc::new(UInt16Array::from(vec![u16::MAX])),
        Arc::new(UInt32Array::from(vec![u32::MAX])),
        Arc::new(UInt64Array::from(vec![1_u64 << 53])),
    ];
    for a in integers {
        assert!(cell_to_field(&ValidatedArray::new(&a).unwrap(), 0, DbfKind::Int).is_ok());
    }
    scritto(Arc::new(BooleanArray::from(vec![true])), b"t", 1, 0);
    scritto(Arc::new(BooleanArray::from(vec![false])), b"f", 1, 0);
    for a in [
        Arc::new(Date32Array::from(vec![0])) as ArrayRef,
        Arc::new(Date64Array::from(vec![0])),
    ] {
        let mut expected = b"1970-01-01".to_vec();
        expected.resize(254, b' ');
        scritto(a, &expected, 254, 0);
    }
    for n in [-1, 1, 86_400_001] {
        rifiutato(Arc::new(Date64Array::from(vec![n])));
    }
}

#[test]
fn tipi_non_convertibili_rifiutati_senza_null_implicito() {
    // Anche i tipi che la capability puo' rifiutare prima di create non
    // devono avere un fallback locale che trasformi una cella in null.
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Decimal32Array::from(vec![123_i32])),
        Arc::new(Decimal64Array::from(vec![123_i64])),
        Arc::new(
            Decimal128Array::from(vec![123_456_789_i128])
                .with_precision_and_scale(20, 9)
                .unwrap(),
        ),
        Arc::new(LargeStringArray::from(vec!["test"])),
        Arc::new(StringViewArray::from(vec!["test"])),
        Arc::new(BinaryArray::from(vec![b"test".as_slice()])),
        Arc::new(DurationSecondArray::from(vec![1])),
        Arc::new(Decimal256Array::from(vec!["123"
            .parse::<<types::Decimal256Type as types::ArrowPrimitiveType>::Native>()
            .unwrap()])),
        Arc::new(Float16Array::from(vec!["0.5"
            .parse::<<types::Float16Type as types::ArrowPrimitiveType>::Native>()
            .unwrap()])),
        Arc::new(LargeBinaryArray::from(vec![b"test".as_slice()])),
        Arc::new(BinaryViewArray::from(vec![b"test".as_slice()])),
        Arc::new(FixedSizeBinaryArray::try_from_iter([b"ab".as_slice()].into_iter()).unwrap()),
        Arc::new(IntervalYearMonthArray::from(vec![1])),
    ];
    for a in arrays {
        assert!(cell_to_field(
            &ValidatedArray::new(&a).unwrap(),
            0,
            DbfKind::from(a.data_type())
        )
        .is_err());
    }
}

#[test]
fn temporali_conservano_tutte_le_cifre() {
    let cases: Vec<(ArrayRef, &str)> = vec![
        (Arc::new(Date32Array::from(vec![-1])), "1969-12-31"),
        (Arc::new(Date64Array::from(vec![-86_400_000])), "1969-12-31"),
        (Arc::new(Date32Array::from(vec![11_016])), "2000-02-29"),
        (Arc::new(Time32SecondArray::from(vec![1])), "00:00:01"),
        (
            Arc::new(Time32MillisecondArray::from(vec![1])),
            "00:00:00.001",
        ),
        (
            Arc::new(Time64MicrosecondArray::from(vec![1])),
            "00:00:00.000001",
        ),
        (
            Arc::new(Time64NanosecondArray::from(vec![1])),
            "00:00:00.000000001",
        ),
        (
            Arc::new(TimestampSecondArray::from(vec![1])),
            "1970-01-01T00:00:01",
        ),
        (
            Arc::new(TimestampMillisecondArray::from(vec![1])),
            "1970-01-01T00:00:00.001",
        ),
        (
            Arc::new(TimestampMicrosecondArray::from(vec![1])),
            "1970-01-01T00:00:00.000001",
        ),
        (
            Arc::new(TimestampNanosecondArray::from(vec![1])),
            "1970-01-01T00:00:00.000000001",
        ),
        (
            Arc::new(TimestampNanosecondArray::from(vec![1]).with_timezone("Europe/Berlin")),
            "1970-01-01T00:00:00.000000001Z",
        ),
    ];
    for (array, text) in cases {
        let mut expected = text.as_bytes().to_vec();
        expected.resize(254, b' ');
        scritto(array, &expected, 254, 0);
    }
    scritto(
        Arc::new(Float32Array::from(vec![0.125])),
        b"          0.12500000",
        20,
        8,
    );
}

#[test]
fn character_non_invertibili_sono_rifiutati() {
    for text in ["", "   ", "  a", "a  ", "  a  ", "p\0q"] {
        rifiutato(Arc::new(StringArray::from(vec![text])));
    }
}

#[test]
fn cpg_pubblicato_e_contato_in_entrambe_le_modalita() {
    for name in ["out.shp", "out.shp.d"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        let batch = batch(Arc::new(StringArray::from(vec!["Citt?"])));
        let mut w = writer(&path, &batch);
        w.write(&batch).unwrap();
        let published = w.finish().unwrap();
        let base = if path.is_dir() {
            path.join("data.shp")
        } else {
            path
        };
        assert_eq!(std::fs::read(base.with_extension("cpg")).unwrap(), b"UTF-8");
        let total: u64 = ["shp", "shx", "dbf", "prj", "cpg"]
            .iter()
            .map(|ext| std::fs::metadata(base.with_extension(ext)).unwrap().len())
            .sum();
        assert_eq!(published.bytes, total);
    }
}

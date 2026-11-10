//! Regressioni di fedelta': XML indipendente dal reader IO e fixture esterne.
use super::*;
use arrow_array::*;

// Le fixture costruiscono e cedono array temporanei di tipi differenti.
#[allow(clippy::needless_pass_by_value)]
fn scritto(array: ArrayRef) -> Result<Vec<u8>> {
    let mut wb = Workbook::new();
    write_cell(wb.add_worksheet(), 0, 0, &ValidatedArray::new(&array)?, 0)?;
    wb.save_to_buffer().map_err(xls_err)
}

fn xml(bytes: &[u8], parte: &str) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut testo = String::new();
    zip.by_name(parte)
        .unwrap()
        .read_to_string(&mut testo)
        .unwrap();
    testo
}

fn rileggi(bytes: Vec<u8>) -> Data {
    let mut wb: Xlsx<_> = Xlsx::new(std::io::Cursor::new(bytes)).unwrap();
    wb.worksheet_range("Sheet1")
        .unwrap()
        .get_value((0, 0))
        .unwrap()
        .clone()
}

#[test]
fn interi_esatti_xml_e_rilettura_e_rifiuto_senza_pubblicazione() {
    for value in [i64::MIN, -(1_i64 << 53) - 1, i64::MAX, (1 << 53) + 1] {
        let array: ArrayRef = Arc::new(Int64Array::from(vec![value]));
        assert!(scritto(array.clone()).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rifiutato.xlsx");
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("v", arrow_schema::DataType::Int64, false),
                geometry_field("geometry", "EPSG:4326"),
            ])),
            vec![array, Arc::new(BinaryArray::from(vec![None::<&[u8]>]))],
        )
        .unwrap();
        let error = scrittore(&path, false, vec![batch]).finish().err().unwrap();
        assert_eq!(error.phase, ErrorPhase::Finalize);
        assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
        assert_eq!(error.remote_effect, plenora_io_model::RemoteEffect::None);
        assert!(!error.message.contains(&value.to_string()));
        assert!(!path.exists());
    }
    for value in [
        u64::MAX,
        (1_u64 << 53) + 1,
        1 << 63,
        (1 << 63) + 1,
        u64::MAX - 2047,
    ] {
        assert!(scritto(Arc::new(UInt64Array::from(vec![value]))).is_err());
    }
    for value in [-(1_i64 << 53), -1, 0, (1 << 53) - 1, 1 << 53] {
        let bytes = scritto(Arc::new(Int64Array::from(vec![value]))).unwrap();
        assert!(xml(&bytes, "xl/worksheets/sheet1.xml").contains(&format!("<v>{value}.0</v>")));
        assert_eq!(rileggi(bytes).to_string(), value.to_string());
    }
}

#[test]
fn tutti_i_tipi_scritti_e_rifiuti_numerici() {
    macro_rules! prova {
        ($tipo:ty, $value:expr, $text:expr) => {{
            let bytes = scritto(Arc::new(<$tipo>::from(vec![$value]))).unwrap();
            assert_eq!(rileggi(bytes).to_string(), $text);
        }};
    }
    prova!(Int8Array, i8::MIN, "-128");
    prova!(Int16Array, i16::MAX, "32767");
    prova!(Int32Array, i32::MIN, "-2147483648");
    prova!(Int64Array, 42, "42");
    prova!(UInt8Array, u8::MAX, "255");
    prova!(UInt16Array, u16::MAX, "65535");
    prova!(UInt32Array, u32::MAX, "4294967295");
    prova!(UInt64Array, 42, "42");
    prova!(BooleanArray, true, "true");
    prova!(StringArray, "ciao", "ciao");
    prova!(Date32Array, 0, "1970-01-01");
    prova!(Date64Array, -86_400_000, "1969-12-31");
    prova!(Time32SecondArray, 1, "00:00:01");
    prova!(Time32MillisecondArray, 1, "00:00:00.001");
    prova!(Time64MicrosecondArray, 1, "00:00:00.000001");
    prova!(Time64NanosecondArray, 1, "00:00:00.000000001");
    prova!(TimestampSecondArray, 0, "1970-01-01T00:00:00");
    prova!(TimestampMillisecondArray, 1, "1970-01-01T00:00:00.001");
    prova!(TimestampMicrosecondArray, 1, "1970-01-01T00:00:00.000001");
    prova!(TimestampNanosecondArray, 1, "1970-01-01T00:00:00.000000001");
    for value in [
        0.1_f32,
        f32::MIN_POSITIVE,
        f32::from_bits(1),
        f32::MAX,
        -0.0,
    ] {
        let bytes = scritto(Arc::new(Float32Array::from(vec![value]))).unwrap();
        let Data::Float(actual) = rileggi(bytes) else {
            panic!("numero atteso")
        };
        assert_eq!(actual.to_bits(), f64::from(value).to_bits());
    }
    for value in [0.1, f64::MIN_POSITIVE, f64::from_bits(1), f64::MAX, -0.0] {
        let bytes = scritto(Arc::new(Float64Array::from(vec![value]))).unwrap();
        let Data::Float(actual) = rileggi(bytes) else {
            panic!("numero atteso")
        };
        assert_eq!(actual.to_bits(), value.to_bits());
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(scritto(Arc::new(Float64Array::from(vec![value]))).is_err());
    }
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(scritto(Arc::new(Float32Array::from(vec![value]))).is_err());
    }
    for array in [
        Arc::new(
            Decimal128Array::from(vec![123])
                .with_precision_and_scale(38, 2)
                .unwrap(),
        ) as ArrayRef,
        Arc::new(
            Decimal128Array::from(vec![9_007_199_254_740_993])
                .with_precision_and_scale(38, 0)
                .unwrap(),
        ) as ArrayRef,
        Arc::new(
            Decimal128Array::from(vec![1])
                .with_precision_and_scale(38, 38)
                .unwrap(),
        ) as ArrayRef,
        Arc::new(
            Decimal256Array::from_iter_values(["9007199254740993".parse().unwrap()])
                .with_precision_and_scale(76, 0)
                .unwrap(),
        ) as ArrayRef,
        Arc::new(DurationSecondArray::from(vec![1])) as ArrayRef,
        Arc::new(LargeStringArray::from(vec!["x"])) as ArrayRef,
        Arc::new(BinaryArray::from(vec![b"x".as_slice()])) as ArrayRef,
    ] {
        assert!(scritto(array).is_err());
    }
    let bytes = scritto(Arc::new(Int64Array::from(vec![None::<i64>]))).unwrap();
    assert!(!xml(&bytes, "xl/worksheets/sheet1.xml").contains("<c "));
}

#[test]
fn varianti_calamine_non_rappresentabili_non_diventano_null() {
    for data in [
        Data::Float(f64::NAN),
        Data::Float(f64::INFINITY),
        Data::Float(f64::NEG_INFINITY),
        Data::Error(calamine::CellErrorType::Div0),
        Data::Error(calamine::CellErrorType::NA),
        Data::DateTime(calamine::ExcelDateTime::new(
            60.0,
            calamine::ExcelDateTimeType::DateTime,
            false,
        )),
        Data::DateTime(calamine::ExcelDateTime::new(
            1.5,
            calamine::ExcelDateTimeType::TimeDelta,
            true,
        )),
    ] {
        let error = classify_data(&data).unwrap_err();
        assert_eq!(error.phase, ErrorPhase::Read);
        assert_eq!(error.category, plenora_io_model::ErrorCategory::DataMapping);
        assert_eq!(error.remote_effect, plenora_io_model::RemoteEffect::None);
        assert!(byte_spool(|spool| spool.data(&data)).is_err());
    }
    assert!(matches!(
        classify_data(&Data::Empty),
        Ok(ObservedValueClass::Null)
    ));
}

// Provenienza: contenitore rust_xlsxwriter fissato dal Cargo.lock, parti OOXML
// modificate qui secondo ECMA-376. Nessun writer IO produce queste fixture.
fn calendario(bytes: Vec<u8>, is_1904: bool) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().unwrap().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if name == "xl/workbook.xml" && is_1904 {
            let text = String::from_utf8(data).unwrap();
            assert!(text.contains("<workbookPr"));
            data = text
                .replace("<workbookPr", "<workbookPr date1904=\"1\"")
                .into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(&data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn workbook_esterni_date_durate_errori_e_vuoti() {
    for is_1904 in [false, true] {
        for (format, serial) in [
            ("yyyy-mm-dd", 0.0_f64),
            ("yyyy-mm-dd", 59.0),
            ("yyyy-mm-dd", 60.0),
            ("yyyy-mm-dd", 61.0),
            ("yyyy-mm-dd hh:mm:ss", 45292.5),
            ("[h]:mm:ss", 1.5),
        ] {
            let mut wb = Workbook::new();
            let sheet = wb.add_worksheet();
            sheet.write_string(0, 0, "geometry").unwrap();
            sheet.write_string(0, 1, "valore").unwrap();
            sheet.write_string(1, 0, "POINT (1 2)").unwrap();
            sheet
                .write_number_with_format(
                    1,
                    1,
                    serial,
                    &rust_xlsxwriter::Format::new().set_num_format(format),
                )
                .unwrap();
            let bytes = calendario(wb.save_to_buffer().unwrap(), is_1904);

            let mut parsed: Xlsx<_> = Xlsx::new(std::io::Cursor::new(&bytes)).unwrap();
            let range = parsed.worksheet_range("Sheet1").unwrap();
            let Data::DateTime(date) = range.get_value((1, 1)).unwrap() else {
                panic!("data nativa attesa")
            };
            assert_eq!(date.as_f64().to_bits(), serial.to_bits());
            assert_eq!(date.is_duration(), format == "[h]:mm:ss");
            assert_eq!(
                *date,
                calamine::ExcelDateTime::new(
                    serial,
                    if format == "[h]:mm:ss" {
                        calamine::ExcelDateTimeType::TimeDelta
                    } else {
                        calamine::ExcelDateTimeType::DateTime
                    },
                    is_1904
                )
            );
            if !is_1904 && (serial == 0.0 || serial.to_bits() == 60.0_f64.to_bits()) {
                assert!(apri_e_conta(&bytes, "esterno.xlsx").is_err());
            } else {
                assert_eq!(apri_e_conta(&bytes, "esterno.xlsx").unwrap().0, 1);
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    for (value, expected) in [
        ("#DIV/0!", calamine::CellErrorType::Div0),
        ("#N/A", calamine::CellErrorType::NA),
    ] {
        let rows = format!(
            "<row r=\"1\">{}{}</row><row r=\"2\">{}<c r=\"B2\" t=\"e\"><v>{value}</v></c></row>",
            cella("A1", "geometry"),
            cella("B1", "valore"),
            cella("A2", "POINT (1 2)")
        );
        let path = xlsx_con_foglio(dir.path(), "errore.xlsx", &foglio_xml("A1:B2", &rows));
        let mut parsed: Xlsx<_> = open_workbook(&path).unwrap();
        assert_eq!(
            parsed.worksheet_range("Sheet1").unwrap().get_value((1, 1)),
            Some(&Data::Error(expected))
        );
        assert!(XlsDriver
            .open(Source::Path(path), opzioni_lettura_wkt())
            .is_err());
    }
    let rows = format!(
        "<row r=\"1\">{}{}</row><row r=\"2\">{}</row>",
        cella("A1", "geometry"),
        cella("B1", "valore"),
        cella("A2", "POINT (1 2)")
    );
    let path = xlsx_con_foglio(dir.path(), "vuoto.xlsx", &foglio_xml("A1:B2", &rows));
    let mut parsed: Xlsx<_> = open_workbook(&path).unwrap();
    assert_eq!(
        parsed.worksheet_range("Sheet1").unwrap().get_value((1, 1)),
        Some(&Data::Empty)
    );
    assert_eq!(
        apri_e_conta(&std::fs::read(path).unwrap(), "vuoto.xlsx")
            .unwrap()
            .0,
        1
    );
}

fn leggi_valori(bytes: &[u8]) -> Result<RecordBatch> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("date.xlsx");
    std::fs::write(&path, bytes).unwrap();
    let dataset = XlsDriver.open(Source::Path(path), opzioni_lettura_wkt())?;
    let mut reader = dataset.open_layer_reader(&ReadRequest {
        layer: LayerId(0),
        projected_fields: None,
        projection_mode: ProjectionMode::BestEffort,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::default(),
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    })?;
    Ok(reader.next_batch()?.unwrap())
}

fn fixture_date(epoch: bool, format: &str, serials: &[Option<f64>], mixed: bool) -> Vec<u8> {
    let mut wb = Workbook::new();
    let sheet = wb.add_worksheet();
    sheet.write_string(0, 0, "geometry").unwrap();
    sheet.write_string(0, 1, "valore").unwrap();
    for (index, serial) in serials.iter().enumerate() {
        let row = u32::try_from(index + 1).unwrap();
        sheet.write_string(row, 0, "POINT (1 2)").unwrap();
        if let Some(serial) = serial {
            sheet
                .write_number_with_format(
                    row,
                    1,
                    *serial,
                    &rust_xlsxwriter::Format::new().set_num_format(format),
                )
                .unwrap();
        }
    }
    if mixed {
        sheet.write_string(2, 1, "testo").unwrap();
    }
    calendario(wb.save_to_buffer().unwrap(), epoch)
}

#[test]
fn date_native_rilettura_arrow_esatta_in_entrambi_i_calendari() {
    for epoch in [false, true] {
        let serials = if epoch {
            vec![
                Some(0.0),
                Some(1.0),
                Some(59.0),
                Some(60.0),
                Some(61.0),
                None,
            ]
        } else {
            vec![Some(1.0), Some(59.0), Some(61.0), None]
        };
        let expected = if epoch {
            vec![
                Some(-24107),
                Some(-24106),
                Some(-24048),
                Some(-24047),
                Some(-24046),
                None,
            ]
        } else {
            vec![Some(-25567), Some(-25509), Some(-25508), None]
        };
        let bytes = fixture_date(epoch, "yyyy-mm-dd", &serials, false);
        for _ in 0..2 {
            let batch = leggi_valori(&bytes).unwrap();
            let dates = batch
                .column(1)
                .as_any()
                .downcast_ref::<Date32Array>()
                .unwrap();
            assert_eq!(dates.iter().collect::<Vec<_>>(), expected);
        }
        for (format, serial) in [
            ("yyyy-mm-dd hh:mm:ss", 45292.5),
            ("yyyy-mm-dd hh:mm:ss", 45292.0),
            ("hh:mm:ss.000", 61.0 + 1.0 / 1024.0),
        ] {
            let bytes = fixture_date(epoch, format, &[Some(serial), None], false);
            let offset = if epoch { 24107_i64 } else { 25569 };
            let expected = if serial.to_bits() == 45292.5_f64.to_bits() {
                (45292 - offset) * 86_400_000_000 + 43_200_000_000
            } else if serial.to_bits() == 45292.0_f64.to_bits() {
                (45292 - offset) * 86_400_000_000
            } else {
                (61 - offset) * 86_400_000_000 + 84_375_000
            };
            for _ in 0..2 {
                let batch = leggi_valori(&bytes).unwrap();
                let dates = batch
                    .column(1)
                    .as_any()
                    .downcast_ref::<TimestampMicrosecondArray>()
                    .unwrap();
                assert_eq!(dates.iter().collect::<Vec<_>>(), vec![Some(expected), None]);
            }
        }
        let bytes = fixture_date(epoch, "[h]:mm:ss", &[Some(1.5), Some(0.0), None], false);
        let batch = leggi_valori(&bytes).unwrap();
        let durations = batch
            .column(1)
            .as_any()
            .downcast_ref::<DurationMicrosecondArray>()
            .unwrap();
        assert_eq!(
            durations.iter().collect::<Vec<_>>(),
            vec![Some(129_600_000_000), Some(0), None]
        );
    }
}

#[test]
fn date_native_rifiuti_espliciti_senza_null() {
    for epoch in [false, true] {
        for serial in [-1.0, 2_958_466.0, f64::from_bits(61.1_f64.to_bits() + 1)] {
            let bytes = fixture_date(epoch, "yyyy-mm-dd hh:mm:ss.000", &[Some(serial)], false);
            let error = leggi_valori(&bytes).unwrap_err();
            assert!(error.message.contains("seriale"));
            assert!(!error.message.contains(&serial.to_string()));
        }
        for serial in [0.0, 60.0, 60.5] {
            let bytes = fixture_date(epoch, "yyyy-mm-dd", &[Some(serial)], false);
            assert_eq!(leggi_valori(&bytes).is_err(), !epoch);
        }
        let bytes = fixture_date(epoch, "yyyy-mm-dd", &[Some(61.0), None], true);
        assert!(leggi_valori(&bytes).unwrap_err().message.contains("misti"));
        // Anche nell'ordine inverso: il testo non deve mascherare la data.
        let bytes = fixture_date(epoch, "yyyy-mm-dd", &[None, None, Some(61.0)], true);
        assert!(leggi_valori(&bytes).unwrap_err().message.contains("misti"));
    }
}

#[test]
fn orari_excel_al_millisecondo_e_durate_attraverso_ooxml() {
    for epoch in [false, true] {
        for (serial, millis) in [
            (45000.0 + 47_862.0 / 86400.0, 47_862_000),
            (45000.0 + 86399.999 / 86400.0, 86_399_999),
            (45000.0 + 0.001 / 86400.0, 1),
            (45000.5, 43_200_000),
            (45000.0, 0),
            (45000.1, 8_640_000),
        ] {
            // Un formato solo data non puo' cancellare l'ora memorizzata.
            for format in ["yyyy-mm-dd hh:mm:ss.000", "yyyy-mm-dd"] {
                let bytes = fixture_date(epoch, format, &[Some(serial), None], false);
                let batch = leggi_valori(&bytes).unwrap();
                let offset = if epoch { 24107 } else { 25569 };
                if format == "yyyy-mm-dd" && millis == 0 {
                    let values = batch
                        .column(1)
                        .as_any()
                        .downcast_ref::<Date32Array>()
                        .unwrap();
                    assert_eq!(
                        values.iter().collect::<Vec<_>>(),
                        vec![Some(45000 - offset), None]
                    );
                } else {
                    let values = batch
                        .column(1)
                        .as_any()
                        .downcast_ref::<TimestampMicrosecondArray>()
                        .unwrap();
                    assert_eq!(
                        values.iter().collect::<Vec<_>>(),
                        vec![
                            Some(((45000 - i64::from(offset)) * 86_400_000 + millis) * 1000),
                            None
                        ]
                    );
                }
            }
        }
        let serial = 134_130.5 / 86400.0;
        let bytes = fixture_date(epoch, "[h]:mm:ss.000", &[Some(serial), None], false);
        let batch = leggi_valori(&bytes).unwrap();
        let values = batch
            .column(1)
            .as_any()
            .downcast_ref::<DurationMicrosecondArray>()
            .unwrap();
        assert_eq!(
            values.iter().collect::<Vec<_>>(),
            vec![Some(134_130_500_000), None]
        );
        for format in ["yyyy-mm-dd hh:mm:ss.000", "[h]:mm:ss.000"] {
            for serial in [
                f64::from_bits(45000.5_f64.to_bits() - 1),
                f64::from_bits(45000.5_f64.to_bits() + 1),
            ] {
                let bytes = fixture_date(epoch, format, &[Some(serial)], false);
                assert!(leggi_valori(&bytes)
                    .unwrap_err()
                    .message
                    .contains("round-trip"));
            }
        }
    }
}

#[test]
#[allow(clippy::cast_precision_loss)] // ms < 2^48: conversione intera esatta, divisione IEEE come oracolo indipendente.
fn millisecondi_excel_round_trip_deterministico_nel_dominio() {
    use arrow_schema::{DataType, TimeUnit};
    use calamine::{ExcelDateTime, ExcelDateTimeType};
    let day = 86_400_000_u64;
    let mut seed = 0x48ac_37de_902b_16f5_u64;
    for epoch in [false, true] {
        for duration in [false, true] {
            let limit = if epoch && !duration {
                2_957_004
            } else {
                2_958_466
            } * day;
            let boundaries = [
                0,
                1,
                day - 1,
                day,
                day + 1,
                59 * day,
                60 * day - 1,
                60 * day,
                61 * day - 1,
                61 * day,
                limit - 1,
            ];
            for index in 0..20_000 {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let millis = boundaries.get(index).copied().unwrap_or(seed % limit);
                let serial = millis as f64 / day as f64;
                let kind = if duration {
                    ExcelDateTimeType::TimeDelta
                } else {
                    ExcelDateTimeType::DateTime
                };
                let result = super::super::date_native::convert(
                    ExcelDateTime::new(serial, kind, epoch),
                    epoch,
                    false,
                    false,
                );
                if !duration && !epoch && (millis < day || (60 * day..61 * day).contains(&millis)) {
                    assert!(result.is_err());
                    continue;
                }
                let offset = if duration {
                    0
                } else if epoch {
                    24107
                } else if millis < 60 * day {
                    25568
                } else {
                    25569
                };
                let (native_kind, ticks) = result.unwrap();
                assert_eq!(
                    ticks,
                    (i64::try_from(millis).unwrap() - offset * 86_400_000) * 1000
                );
                assert_eq!(
                    native_kind.data_type(),
                    if duration {
                        DataType::Duration(TimeUnit::Microsecond)
                    } else {
                        DataType::Timestamp(TimeUnit::Microsecond, None)
                    }
                );
            }
        }
    }
}

fn modifica_parte(bytes: &[u8], part: &str, edit: impl Fn(&str) -> String) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().unwrap().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if name == part {
            data = edit(std::str::from_utf8(&data).unwrap()).into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(&data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn epoca_excel_extlst_booleani_e_namespace() {
    let source = fixture_date(true, "yyyy-mm-dd", &[Some(0.0), Some(43830.0)], false);
    // Struttura copiata dal workbook Excel reale di calamine #706: provenienza in fixtures/README.md.
    let workbook = include_str!("../../tests/fixtures/excel-workbook-1904.xml");
    for boolean in ["1", "true", "0", "false"] {
        let bytes = modifica_parte(&source, "xl/workbook.xml", |_| {
            workbook.replace("date1904=\"1\"", &format!("date1904=\"{boolean}\""))
        });
        if matches!(boolean, "0" | "false") {
            assert!(leggi_valori(&bytes).is_err());
            continue;
        }
        let batch = leggi_valori(&bytes).unwrap();
        let values = batch
            .column(1)
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        assert_eq!(values.values().as_ref(), &[-24107, 19723]);
    }
    // Namespace equivalenti con prefissi diversi; attributi qualificati estranei ignorati.
    let bytes = modifica_parte(&source, "xl/workbook.xml", |xml| {
        xml.replace("<workbookPr date1904=\"1\"", "<s:workbookPr xmlns:s=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:f=\"urn:foreign\" f:date1904=\"false\" date1904=\"true\"")
            .replace("xmlns:r=", "xmlns:rel=").replace("r:id=", "rel:id=")
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefissi.xlsx");
    std::fs::write(&path, &bytes).unwrap();
    assert!(
        NativeStyles::read(&path, "Sheet1", opzioni_lettura_wkt().budget())
            .unwrap()
            .epoch
    );
}

#[test]
fn ore_pure_cjk_e_formule_con_cache_al_millisecondo() {
    for epoch in [false, true] {
        for format in [
            "hh:mm",
            "hh:mm:ss.000",
            "h:mm AM/PM",
            "[Magenta]hh:mm",
            "[$-en-US]hh:mm",
        ] {
            let bytes = fixture_date(
                epoch,
                format,
                &[Some(0.0), Some(47_862.123 / 86400.0), None],
                false,
            );
            let bytes = modifica_parte(&bytes, "xl/worksheets/sheet1.xml", |xml| {
                xml.replace("<v>0.0</v>", "<f>TIME(0,0,0)</f><v>0.0</v>")
                    .replace("<v>0.553", "<f>TIME(13,17,42)+123/86400000</f><v>0.553")
            });
            let batch = leggi_valori(&bytes).unwrap();
            let values = batch
                .column(1)
                .as_any()
                .downcast_ref::<Time64MicrosecondArray>()
                .unwrap();
            assert_eq!(
                values.iter().collect::<Vec<_>>(),
                vec![Some(0), Some(47_862_123_000), None]
            );
        }
    }
    for id in (27..=36).chain(50..=58) {
        let time = matches!(id, 32 | 33 | 55 | 56);
        let bytes = fixture_date(
            false,
            "yyyy-mm-dd",
            &[Some(if time { 0.5 } else { 45292.0 })],
            false,
        );
        let bytes = modifica_parte(&bytes, "xl/styles.xml", |xml| {
            xml.replace("numFmtId=\"164\"", &format!("numFmtId=\"{id}\""))
                .replace(
                    "<numFmts count=\"1\"><numFmt",
                    "<numFmts count=\"1\"><foreign:numFmt xmlns:foreign=\"urn:foreign\"",
                )
                .replace(
                    "<cellXfs",
                    "<xf xmlns=\"urn:foreign\" numFmtId=\"0\"/><cellXfs",
                )
        });
        let batch = leggi_valori(&bytes).unwrap();
        assert_eq!(
            batch.column(1).data_type(),
            &if time {
                arrow_schema::DataType::Time64(arrow_schema::TimeUnit::Microsecond)
            } else {
                arrow_schema::DataType::Date32
            }
        );
    }
}

#[test]
#[allow(clippy::cast_precision_loss)] // ms < 2^48, conversione esatta.
fn centomila_seriali_decimali_degli_scrittori_reali() {
    use calamine::{ExcelDateTime, ExcelDateTimeType};
    let mut seed = 0x193a_746b_857c_098d_u64;
    for index in 0..100_000 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        // Date al secondo in tutto il dominio; anche campioni con millisecondi.
        let ms = (61 * 86400 + seed % (2_956_000 * 86400)) * 1000
            + if index % 2 == 0 { 0 } else { seed % 1000 };
        for (millis, kind, time) in [
            (ms, ExcelDateTimeType::DateTime, false),
            (ms, ExcelDateTimeType::TimeDelta, false),
            (ms % 86_400_000, ExcelDateTimeType::DateTime, true),
        ] {
            let canonical = millis as f64 / 86_400_000.0;
            for text in [
                format!("{canonical:.14e}"),
                format!("{canonical:.15e}"),
                format!("{canonical:.16e}"),
                canonical.to_string(),
            ] {
                let serial = text.parse().unwrap();
                let (_, ticks) = super::super::date_native::convert(
                    ExcelDateTime::new(serial, kind, false),
                    false,
                    false,
                    time,
                )
                .unwrap();
                let offset = if kind == ExcelDateTimeType::DateTime && !time {
                    25569 * 86_400_000
                } else {
                    0
                };
                assert_eq!(
                    ticks,
                    (i64::try_from(millis).unwrap() - offset) * 1000,
                    "{text}"
                );
            }
        }
    }
}

#[test]
fn caratteri_non_conservabili_e_float_esponenziali() {
    for value in ["a\u{fffe}b", "a\u{ffff}b"] {
        assert!(scritto(Arc::new(StringArray::from(vec![value]))).is_err());
    }
    for value in [1e300, 5e-324, f64::MAX, -1e300] {
        let bytes = scritto(Arc::new(Float64Array::from(vec![value]))).unwrap();
        let sheet = xml(&bytes, "xl/worksheets/sheet1.xml");
        let text = sheet
            .split("<v>")
            .nth(1)
            .unwrap()
            .split("</v>")
            .next()
            .unwrap();
        assert!(text.len() < 25 && text.contains('e'), "{text}");
        assert_eq!(text.parse::<f64>().unwrap().to_bits(), value.to_bits());
        let Data::Float(actual) = rileggi(bytes) else {
            panic!("float atteso")
        };
        assert_eq!(actual.to_bits(), value.to_bits());
    }
}

#[test]
fn metadati_xml_ignorano_omonimi_estranei_e_discendenti() {
    let source = fixture_date(true, "yyyy-mm-dd", &[Some(1.0)], false);
    let source = modifica_parte(&source, "xl/workbook.xml", |xml| {
        xml.replace(
            "</sheets>",
            "<f:sheet xmlns:f=\"urn:foreign\" name=\"Sheet1\" r:id=\"bad\"/></sheets>",
        )
        .replace(
            "</workbook>",
            "<extLst><ext><workbookPr date1904=\"false\"/></ext></extLst></workbook>",
        )
    });
    let source = modifica_parte(&source, "xl/_rels/workbook.xml.rels", |xml| {
        xml.replace(
            "</Relationships>",
            "<Relationship xmlns=\"urn:foreign\" Id=\"rId1\" Target=\"bad\"/></Relationships>",
        )
    });
    let source = modifica_parte(&source, "xl/styles.xml", |xml| {
        xml.replace(
            "</numFmts>",
            "<numFmt xmlns=\"urn:foreign\" numFmtId=\"164\" formatCode=\"0.00\"/></numFmts>",
        )
        .replace(
            "<cellXfs count=\"2\">",
            "<cellXfs count=\"2\"><xf xmlns=\"urn:foreign\" numFmtId=\"0\"/>",
        )
    });
    let source = modifica_parte(&source, "xl/worksheets/sheet1.xml", |xml| {
        xml.replace(
            "</row>",
            "<c xmlns=\"urn:foreign\" r=\"Z99\" s=\"1\"><v>1</v></c></row>",
        )
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("namespace.xlsx");
    std::fs::write(&path, &source).unwrap();
    let styles = NativeStyles::read(&path, "Sheet1", opzioni_lettura_wkt().budget()).unwrap();
    assert!(styles.epoch);
    assert!(styles.date_only(1, 1));
    assert!(!styles.date_only(98, 25));
}

#[test]
#[allow(clippy::cast_precision_loss)] // ms nel dominio intero esatto di f64.
fn centomila_date_decimali_attraversano_il_parser_ooxml() {
    use std::fmt::Write as _;
    let mut seed = 0x973a_5bf6_171c_098d_u64;
    let mut rows = String::new();
    let mut expected = Vec::new();
    for row in 1..=100_000 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let ms = (61 * 86400 + seed % (2_956_000 * 86400)) * 1000;
        let canonical = ms as f64 / 86_400_000.0;
        expected.push((i64::try_from(ms).unwrap() - 25569 * 86_400_000) * 1000);
        write!(rows, "<row r=\"{row}\">").unwrap();
        for (col, text) in ['A', 'B', 'C', 'D'].into_iter().zip([
            format!("{canonical:.14e}"),
            format!("{canonical:.15e}"),
            format!("{canonical:.16e}"),
            canonical.to_string(),
        ]) {
            write!(rows, "<c r=\"{col}{row}\" s=\"1\"><v>{text}</v></c>").unwrap();
        }
        rows.push_str("</row>");
    }
    let source = fixture_date(false, "yyyy-mm-dd hh:mm:ss.000", &[Some(61.0)], false);
    let source = modifica_parte(&source, "xl/worksheets/sheet1.xml", |_| {
        foglio_xml("A1:D100000", &rows)
    });
    let mut parsed: Xlsx<_> = Xlsx::new(std::io::Cursor::new(source)).unwrap();
    let range = parsed.worksheet_range("Sheet1").unwrap();
    assert_eq!(range.height(), expected.len());
    for (row, expected) in range.rows().zip(expected) {
        for data in row {
            let Data::DateTime(value) = data else {
                panic!("data attesa")
            };
            let (_, ticks) =
                super::super::date_native::convert(*value, false, false, false).unwrap();
            assert_eq!(ticks, expected);
        }
    }
}

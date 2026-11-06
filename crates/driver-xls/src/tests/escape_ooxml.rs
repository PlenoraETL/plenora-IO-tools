//! `ST_Xstring`: attesi indipendenti dal decoder e passaggio completo nello spool IO.
use super::*;
use std::fmt::Write as _;

// ECMA-376, parte 1, 22.9.2.19 (ST_Xstring): quattro cifre esadecimali
// rappresentano un'unita' UTF-16; D83D DE00 rappresenta U+1F600.
// L'underscore protetto da 005F non innesca una seconda decodifica.
const CONFORMI: &[(&str, &str)] = &[
    ("_x03A9_", "\u{03a9}"),
    ("_xD83D__xDE00_", "\u{1f600}"),
    ("_x_", "_x_"),
    ("_x005F_x03A9_", "_x03A9_"),
];

// Politica osservata, NON prescrizione OOXML per UTF-16 malformato:
// sequenze lessicalmente incomplete/non esadecimali conservate; surrogate
// isolate sostituite con U+FFFD. Un solo esito per caso, anche alle riletture.
const MALFORMATI: &[(&str, &str)] = &[
    ("_xZZZZ_", "_xZZZZ_"),
    ("_x03A9", "_x03A9"),
    ("_x03", "_x03"),
    ("_xD83D_", "\u{fffd}"),
    ("_xDE00_", "\u{fffd}"),
    ("_xD83D__x03A9_", "\u{fffd}\u{03a9}"),
    ("_xDE00__xD83D_", "\u{fffd}\u{fffd}"),
    ("_x0\u{00bd}_", "_x0\u{00bd}_"),
];

/// ZIP con ordine, timestamp e contenuti fissi; nessun writer riscrive gli escape.
/// La stessa stringa e' intestazione e valore, su due righe e batch distinti.
fn fixture(casi: &[(&str, &str)], tipo: &str, rinominato: bool) -> Vec<u8> {
    let mut righe = String::new();
    let mut condivise = String::new();
    for (raw, _) in casi {
        write!(condivise, "<si><t>{raw}</t></si>").unwrap();
    }
    for riga in 1..=3 {
        write!(righe, "<row r=\"{riga}\">").unwrap();
        righe.push_str(&cella(
            &format!("A{riga}"),
            if riga == 1 { "geometry" } else { "POINT (1 2)" },
        ));
        for (i, (raw, _)) in casi.iter().enumerate() {
            let col = char::from(b'B' + u8::try_from(i).unwrap());
            let riferimento = format!("{col}{riga}");
            match tipo {
                "s" => write!(righe, "<c r=\"{riferimento}\" t=\"s\"><v>{i}</v></c>").unwrap(),
                "str" => {
                    write!(
                        righe,
                        "<c r=\"{riferimento}\" t=\"str\"><f>\"cached\"</f><v>{raw}</v></c>"
                    )
                    .unwrap();
                }
                "inlineStr" => righe.push_str(&cella(&riferimento, raw)),
                _ => panic!("tipo della fixture sconosciuto"),
            }
        }
        righe.push_str("</row>");
    }
    let ultima = char::from(b'A' + u8::try_from(casi.len()).unwrap());
    let workbook = if rinominato {
        "renamed.xml"
    } else {
        "workbook.xml"
    };
    let parti = [
        ("[Content_Types].xml".to_owned(), format!(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/{workbook}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"#)),
        ("_rels/.rels".to_owned(), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/{workbook}"/></Relationships>"#)),
        (format!("xl/{workbook}"), r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_owned()),
        (format!("xl/_rels/{workbook}.rels"), r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#.to_owned()),
        ("xl/worksheets/sheet1.xml".to_owned(), foglio_xml(&format!("A1:{ultima}3"), &righe)),
        ("xl/sharedStrings.xml".to_owned(), format!(r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{condivise}</sst>"#)),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opzioni = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    for (nome, xml) in parti {
        zip.start_file(nome, opzioni).unwrap();
        std::io::Write::write_all(&mut zip, xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn verifica(casi: &[(&str, &str)]) {
    let dir = tempfile::tempdir().unwrap();
    for tipo in ["inlineStr", "s", "str"] {
        let bytes = fixture(casi, tipo, false);
        assert_eq!(bytes, fixture(casi, tipo, false), "fixture deterministica");
        let path = dir.path().join(format!("{tipo}.xlsx"));
        std::fs::write(&path, bytes).unwrap();
        for _ in 0..3 {
            let dataset = XlsDriver
                .open(Source::Path(path.clone()), opzioni_lettura_wkt())
                .unwrap();
            let mut reader = dataset
                .open_layer_reader(&ReadRequest {
                    layer: LayerId(0),
                    projected_fields: None,
                    projection_mode: ProjectionMode::BestEffort,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: ReadScope::default(),
                    batch_target: BatchTarget {
                        max_rows: 1,
                        ..BatchTarget::default()
                    },
                    cancellation: CancellationToken::default(),
                })
                .unwrap();
            for _ in 0..2 {
                let batch = reader.next_batch().unwrap().unwrap();
                assert_eq!(batch.num_rows(), 1);
                for (_, atteso) in casi {
                    let valori = batch
                        .column_by_name(atteso)
                        .expect("intestazione decodificata")
                        .as_any()
                        .downcast_ref::<arrow_array::StringArray>()
                        .unwrap();
                    assert_eq!(valori.value(0), *atteso, "{tipo}: valore dopo lo spool");
                }
            }
            assert!(reader.next_batch().unwrap().is_none());
        }
    }
}

#[test]
fn st_xstring_celle_e_intestazioni_attraversano_lo_spool() {
    verifica(CONFORMI);
}

#[test]
fn escape_malformati_hanno_un_solo_esito_senza_panico() {
    // Le due surrogate isolate hanno lo stesso risultato: fixture separate
    // evitano la deduplicazione dei nomi di colonna, estranea a questa prova.
    for caso in MALFORMATI {
        verifica(std::slice::from_ref(caso));
    }
}

#[test]
fn workbook_rinominato_accettato_da_calamine_ma_rifiutato_da_io() {
    let bytes = fixture(CONFORMI, "inlineStr", true);
    let mut workbook = Xlsx::new(std::io::Cursor::new(&bytes)).unwrap();
    assert_eq!(
        workbook
            .worksheet_range("Sheet1")
            .unwrap()
            .get_value((1, 1)),
        Some(&Data::String("\u{03a9}".to_owned()))
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rinominato.xlsx");
    std::fs::write(&path, bytes).unwrap();
    for _ in 0..3 {
        let Err(errore) = XlsDriver.open(Source::Path(path.clone()), opzioni_lettura_wkt()) else {
            panic!("i percorsi fissi di NativeStyles devono rifiutare la parte mancante");
        };
        assert_eq!(errore.message, "metadati temporali XML XLSX non validi");
    }
}

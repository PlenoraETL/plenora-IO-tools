//! Fixture sintetiche originali, costruite qui offline: header dBASE III di
//! 32 byte, un descrittore di 32, terminatore 0x0d, flag attivo e payload
//! esatto fornito dal test. Nessun writer numerico genera i byte sotto prova.
use super::*;

fn fixture(
    kind: u8,
    scale: u8,
    payload: &[u8],
    ldid: u8,
    cpg: Option<&[u8]>,
) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("raw.shp");
    let mut writer = shapefile::ShapeWriter::from_path(&path).unwrap();
    writer.write_shape(&Point::new(0.0, 0.0)).unwrap();
    drop(writer);
    let mut dbf = vec![0_u8; 65];
    dbf[0] = 3;
    dbf[1..4].copy_from_slice(&[126, 11, 2]);
    dbf[4..8].copy_from_slice(&1_u32.to_le_bytes());
    dbf[8..10].copy_from_slice(&65_u16.to_le_bytes());
    dbf[10..12].copy_from_slice(&u16::try_from(payload.len() + 1).unwrap().to_le_bytes());
    dbf[29] = ldid;
    dbf[32..38].copy_from_slice(b"valore");
    dbf[43] = kind;
    dbf[48] = u8::try_from(payload.len()).unwrap();
    dbf[49] = scale;
    dbf[64] = 0x0d;
    dbf.push(b' ');
    dbf.extend_from_slice(payload);
    dbf.push(0x1a);
    std::fs::write(path.with_extension("dbf"), dbf).unwrap();
    std::fs::write(path.with_extension("prj"), EPSG_3003_WKT).unwrap();
    if let Some(cpg) = cpg {
        std::fs::write(path.with_extension("cpg"), cpg).unwrap();
    }
    (dir, path)
}

fn read(path: &Path) -> Result<RecordBatch> {
    let dataset = ShpDriver.open(Source::Path(path.to_owned()), opzioni_lettura())?;
    let mut reader = dataset.open_layer_reader(&req())?;
    let batch = reader.next_batch()?.unwrap();
    assert!(reader.loss_report().counts.is_empty());
    assert!(reader.next_batch()?.is_none());
    Ok(batch)
}

#[test]
fn float_e_numeric_conservano_il_decimale_senza_passare_da_f32() {
    for kind in *b"FN" {
        for (text, expected) in [
            ("16777217.00000000", 16_777_217.0_f64),
            ("1.23456789", 1.234_567_89),
            ("0.10000000", 0.1),
            ("9007199254740992.00", 9_007_199_254_740_992.0),
            ("-0.00000000", -0.0),
        ] {
            let (_dir, path) = fixture(kind, 8, format!("{text:>20}").as_bytes(), 0, None);
            let batch = read(&path).unwrap();
            let values = batch
                .column(1)
                .as_any()
                .downcast_ref::<arrow_array::Float64Array>()
                .unwrap();
            assert_eq!(values.value(0).to_bits(), expected.to_bits());
        }
        for text in [
            "-9999999999.99999999",
            "9007199254740993.00",
            "0.123456789012345678",
            "1e-999",
        ] {
            let (_dir, path) = fixture(kind, 8, format!("{text:>20}").as_bytes(), 0, None);
            let error = read(&path).unwrap_err();
            assert!(!error.to_string().contains(text));
        }
    }
}

#[test]
fn character_preserva_spazi_iniziali_e_rifiuta_nul() {
    let (_dir, path) = fixture(b'C', 0, b"  a  ", 0, None);
    let batch = read(&path).unwrap();
    let values = batch
        .column(1)
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(values.value(0), "  a");
    let (_dir, path) = fixture(b'C', 0, b"p\0q", 0, None);
    assert!(read(&path).is_err());
}

#[test]
fn codepage_esplicita_precedenza_cpg_e_nessuna_sostituzione() {
    for (ldid, cpg, payload, expected) in [
        (0x57, None, &b"Citt\xe0"[..], "Città"),
        (0, Some(&b"1252"[..]), &b"Citt\xe0"[..], "Città"),
        (0x57, Some(&b"UTF-8"[..]), "Città".as_bytes(), "Città"),
        (0x57, None, &b"\x80"[..], "\u{80}"),
        (0, Some(&b"CP1252"[..]), &b"\x80"[..], "€"),
        (0, Some(&b"ISO-8859-1"[..]), &b"\x80"[..], "\u{80}"),
    ] {
        let (_dir, path) = fixture(b'C', 0, payload, ldid, cpg);
        let batch = read(&path).unwrap();
        let values = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        assert_eq!(values.value(0), expected);
    }
    for (ldid, cpg, payload) in [
        (0, None, &b"Citt\xe0"[..]),
        (0xf0, None, &b"Citt\xe0"[..]),
        (0x57, Some(&b"UTF-8"[..]), &b"Citt\xe0"[..]),
        (0xfe, None, &b"ascii"[..]),
        (0xf0, Some(&b"unknown"[..]), &b"ascii"[..]),
    ] {
        let (_dir, path) = fixture(b'C', 0, payload, ldid, cpg);
        assert!(read(&path).is_err());
    }
}

#[test]
fn codepage_valida_anche_i_nomi_e_il_sidecar() {
    let (_dir, path) = fixture(b'C', 0, b"ascii", 0x57, None);
    let mut dbf = std::fs::read(path.with_extension("dbf")).unwrap();
    dbf[32..43].fill(0);
    dbf[32..37].copy_from_slice(b"citt\xe0");
    std::fs::write(path.with_extension("dbf"), &dbf).unwrap();
    assert_eq!(read(&path).unwrap().schema().field(1).name(), "città");
    dbf[29] = 0xf0;
    std::fs::write(path.with_extension("dbf"), dbf).unwrap();
    assert!(read(&path).is_err());

    let (_dir, path) = fixture(b'C', 0, b"ascii", 0xf0, Some(&[b'A'; 129]));
    assert!(read(&path).is_err());
    std::fs::write(path.with_extension("cpg"), [0xff]).unwrap();
    assert!(read(&path).is_err());
}

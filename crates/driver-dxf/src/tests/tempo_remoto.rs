use dxf::{Drawing, DrawingEntityReader, DxfError};
fn rejected(bytes: &[u8]) {
    let full = Drawing::load(&mut std::io::Cursor::new(bytes));
    assert!(
        matches!(
            full,
            Err(DxfError::InvalidTemporalValue | DxfError::ParseError(_))
        ),
        "full: {full:?}; input: {bytes:?}"
    );
    let result =
        DrawingEntityReader::load(std::io::Cursor::new(bytes.to_vec())).and_then(|r| r.finish());
    assert!(matches!(
        result,
        Err(DxfError::InvalidTemporalValue | DxfError::ParseError(_))
    ));
}
#[test]
fn tempo_remoto_header_ascii_binary() {
    for field in [
        "TDCREATE",
        "TDUCREATE",
        "TDUPDATE",
        "TDUUPDATE",
        "TDINDWG",
        "TDUSRTIMER",
    ] {
        for value in [
            -1e300,
            1e300,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
        ] {
            let text =
                format!("0\nSECTION\n2\nHEADER\n9\n${field}\n40\n{value}\n0\nENDSEC\n0\nEOF\n");
            rejected(text.as_bytes());
            let mut binary = b"AutoCAD Binary DXF\r\n\x1a\0".to_vec();
            for (code, text) in [(0u8, "SECTION"), (2, "HEADER")] {
                binary.push(code);
                binary.extend(text.as_bytes());
                binary.push(0);
            }
            binary.push(9);
            binary.extend(format!("${field}\0").as_bytes());
            binary.push(40);
            binary.extend(value.to_le_bytes());
            for text in ["ENDSEC", "EOF"] {
                binary.push(0);
                binary.extend(text.as_bytes());
                binary.push(0);
            }
            rejected(&binary);
        }
    }
}
#[test]
fn tempo_remoto_sunstudy() {
    for (day, seconds) in [
        (i32::MIN, 0),
        (i32::MAX, 0),
        (0, 0),
        (2451545, -1),
        (2451545, 86400),
        (5373484, i32::MAX),
    ] {
        rejected(format!("0\nSECTION\n2\nOBJECTS\n0\nSUNSTUDY\n90\n1\n90\n{day}\n90\n{seconds}\n0\nENDSEC\n0\nEOF\n").as_bytes());
    }
}
#[test]
fn tempo_remoto_regression_seeds() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/dxf_reader");
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("tempo-")
        {
            rejected(&std::fs::read(path).unwrap());
        }
    }
}

#[test]
fn tempo_remoto_object_timestamps() {
    for kind in ["LAYER_INDEX", "SPATIAL_INDEX"] {
        for value in ["0", "-1", "1e300", "-1e300", "NaN", "inf", "5373485"] {
            rejected(
                format!("0\nSECTION\n2\nOBJECTS\n0\n{kind}\n40\n{value}\n0\nENDSEC\n0\nEOF\n")
                    .as_bytes(),
            );
        }
    }
}

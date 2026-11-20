//! E-02/E-03/E-04: contenitori corrotti prima di CRS e mapping.
use plenora_io_model::{error_table::Situation, CancellationToken};
use std::path::{Path, PathBuf};

const DRIVERS: &[(&str, &str, &str)] = &[
    ("csv", "csv", "CSV"),
    ("shp", "shp", "Shapefile"),
    ("gpkg", "gpkg", "GeoPackage"),
    ("filegdb", "gdb", "FileGDB"),
    ("geojson", "geojson", "GeoJSON"),
    ("kml", "kml", "KML"),
    ("dxf", "dxf", "DXF"),
    ("geoparquet", "parquet", "Parquet"),
    ("ipc", "arrow", "Arrow"),
    ("xls", "xlsx", "XLSX"),
];
fn fixture(ext: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/canoniche")
        .join(if ext == "shp" {
            "canonico_punti.shp".to_owned()
        } else {
            format!("canonico.{ext}")
        })
}
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}
// La situazione strutturale vale per un driver disponibile; nello stub
// FileGDB l'oracolo e' sempre il tier mancante, anche con struttura sana.
fn assert_error(path: &Path, id: &str, label: &str, available_situation: Situation) {
    for assume in [false, true] {
        for options in [false, true] {
            for command in ["read", "inspect", "layers", "convert"] {
                let mut args = vec![command.to_owned(), path.to_str().unwrap().to_owned()];
                if command == "convert" {
                    args.extend([
                        path.with_extension("out.arrow")
                            .to_str()
                            .unwrap()
                            .to_owned(),
                        "--from".to_owned(),
                        id.to_owned(),
                        "--to".to_owned(),
                        "ipc".to_owned(),
                    ]);
                }
                if assume {
                    args.extend(["--assume-crs".to_owned(), "EPSG:4326".to_owned()]);
                }
                if options {
                    let option = match id {
                        "csv" => "wkt_column=SECRET",
                        "xls" => "sheet=SECRET",
                        "shp" => "row_diagnostics.key_field=SECRET",
                        _ => "SECRET=SECRET",
                    };
                    args.extend(["--opt".to_owned(), option.to_owned()]);
                }
                let (_, result) = plenora_io_tools::esegui(&args, &CancellationToken::new());
                let (_, doc) = result.expect_err(&format!("{id}: {args:?}"));
                if id == "filegdb" && !cfg!(feature = "gdal-backend") {
                    assert_eq!(doc["error"]["category"], "unsupported", "{args:?}: {doc}");
                    assert_eq!(doc["error"]["code"], "UNSUPPORTED", "{args:?}: {doc}");
                    assert!(
                        doc["error"]["message"]
                            .as_str()
                            .unwrap()
                            .contains("gdal-backend"),
                        "{args:?}: {doc}"
                    );
                    continue;
                }
                let rule = available_situation.rule();
                for (key, expected) in [
                    ("category", serde_json::to_value(rule.category).unwrap()),
                    ("phase", serde_json::to_value(rule.phase).unwrap()),
                    ("code", serde_json::json!(rule.wire_code)),
                ] {
                    assert_eq!(doc["error"][key], expected, "{args:?}: {doc}");
                }
                let message = doc["error"]["message"].as_str().unwrap();
                assert!(message.contains(label), "{args:?}: {doc}");
                assert!(
                    !message.contains("CRS") && !message.contains("SECRET"),
                    "{doc}"
                );
            }
        }
    }
}
#[test]
fn every_driver_rejects_corrupt_containers_before_content_options_and_crs() {
    let catalog = plenora_io_tools::cmd_catalog();
    let mut ids: Vec<_> = catalog["drivers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
    let mut expected: Vec<_> = DRIVERS.iter().map(|d| d.0).collect();
    ids.sort_unstable();
    expected.sort_unstable();
    assert_eq!(ids, expected);
    let root = tempfile::tempdir().unwrap();
    for &(id, ext, label) in DRIVERS {
        for defect in ["empty", "garbage", "half", "wrong", "directory"] {
            let path = root.path().join(format!("SECRET-{id}-{defect}.{ext}"));
            if defect == "directory" {
                std::fs::create_dir(&path).unwrap();
            } else if id == "filegdb" {
                copy_tree(&fixture(ext), &path);
                let catalog = path.join("a00000001.gdbtable");
                let mut bytes = std::fs::read(&catalog).unwrap();
                match defect {
                    "empty" => bytes.clear(),
                    "half" => bytes.truncate(bytes.len() / 2),
                    "wrong" => bytes = b"a,b\n1,2\n".to_vec(),
                    _ => bytes = b"SECRET garbage".to_vec(),
                }
                std::fs::write(catalog, bytes).unwrap();
            } else {
                let bytes = match defect {
                    "empty" => Vec::new(),
                    "garbage" => b"\xff\0SECRET garbage".to_vec(),
                    "wrong" => {
                        if id == "csv" {
                            std::fs::read(fixture("arrow")).unwrap()
                        } else {
                            b"a,b\n1,2\n".to_vec()
                        }
                    }
                    "half" => {
                        // Il CSV non ha un terminatore obbligatorio: il taglio deve interrompere un campo quoted.
                        let mut bytes = if id == "csv" {
                            b"a\n\"abcdefghijklmnopqrstuv\"\n".to_vec()
                        } else {
                            std::fs::read(fixture(ext)).unwrap()
                        };
                        bytes.truncate(bytes.len() / 2);
                        bytes
                    }
                    _ => unreachable!(),
                };
                std::fs::write(&path, bytes).unwrap();
            }
            let situation = if defect == "directory" && id != "filegdb" {
                Situation::DirectoryInsteadOfFile
            } else {
                Situation::InvalidContent
            };
            assert_error(&path, id, label, situation);
        }
    }
}
#[test]
fn shapefile_companions_and_directory_container_are_structural() {
    let root = tempfile::tempdir().unwrap();
    for container in [false, true] {
        for ext in ["shp", "shx", "dbf"] {
            for defect in ["empty", "garbage", "half", "wrong", "directory"] {
                let dir = root
                    .path()
                    .join(format!("{container}-{ext}-{defect}.shp.d"));
                std::fs::create_dir(&dir).unwrap();
                let main = dir.join("data.shp");
                for member in ["shp", "shx", "dbf", "prj", "cpg"] {
                    std::fs::copy(
                        fixture("shp").with_extension(member),
                        main.with_extension(member),
                    )
                    .unwrap();
                }
                let member = main.with_extension(ext);
                let mut bytes = std::fs::read(&member).unwrap();
                match defect {
                    "empty" => bytes.clear(),
                    "half" => bytes.truncate(bytes.len() / 2),
                    "wrong" => bytes = b"a,b\n1,2\n".to_vec(),
                    _ => bytes = b"SECRET garbage".to_vec(),
                }
                std::fs::remove_file(&member).unwrap();
                if defect == "directory" {
                    std::fs::create_dir(&member).unwrap();
                } else {
                    std::fs::write(&member, bytes).unwrap();
                }
                assert_error(
                    if container { &dir } else { &main },
                    "shp",
                    "Shapefile",
                    if !container && ext == "shp" && defect == "directory" {
                        Situation::DirectoryInsteadOfFile
                    } else {
                        Situation::InvalidContent
                    },
                );
            }
        }
    }
}

#[test]
fn filegdb_members_and_geopackage_companions_precede_options() {
    let root = tempfile::tempdir().unwrap();
    for name in [
        "a00000001.gdbtable",
        "a00000009.gdbtable",
        "a00000009.gdbtablx",
        "a00000009.gdbindexes",
        "a0000000b.spx",
        "gdb",
    ] {
        for defect in ["empty", "garbage", "half", "wrong", "directory"] {
            let path = root.path().join(format!("{name}-{defect}.gdb"));
            copy_tree(&fixture("gdb"), &path);
            let member = path.join(name);
            let mut bytes = std::fs::read(&member).unwrap();
            match defect {
                "empty" => bytes.clear(),
                "half" => bytes.truncate(bytes.len() / 2),
                "wrong" => bytes = b"a,b\n1,2\n".to_vec(),
                _ => bytes = b"SECRET garbage".to_vec(),
            }
            std::fs::remove_file(&member).unwrap();
            if defect == "directory" {
                std::fs::create_dir(&member).unwrap();
            } else {
                std::fs::write(&member, bytes).unwrap();
            }
            if !cfg!(feature = "gdal-backend") || name == "a00000001.gdbtable" {
                assert_error(&path, "filegdb", "FileGDB", Situation::InvalidContent);
            } else {
                let (_, result) = plenora_io_tools::esegui(
                    &["layers".to_owned(), path.to_str().unwrap().to_owned()],
                    &CancellationToken::new(),
                );
                let doc = result.unwrap();
                let layers = doc["layers"].as_array().unwrap();
                assert_eq!(layers.len(), 5, "{doc}");
                assert_eq!(
                    layers
                        .iter()
                        .filter(|l| l["status"]["readable"] == false)
                        .count(),
                    usize::from(name != "gdb"),
                    "{doc}"
                );
                // Global non-owned members are diagnostic, never prerequisites.
                if name == "gdb" {
                    assert!(doc["fidelity"]["reasons"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|reason| reason["detail"].as_str().is_some_and(
                            |text| text.starts_with("membri FileGDB non attribuiti esclusi")
                        )));
                }
                for layer in layers.iter().filter(|l| l["status"]["readable"] == true) {
                    let (_, result) = plenora_io_tools::esegui(
                        &[
                            "read".to_owned(),
                            path.to_str().unwrap().to_owned(),
                            "--layer".to_owned(),
                            layer["id"].to_string(),
                        ],
                        &CancellationToken::new(),
                    );
                    assert!(result.is_ok(), "{name}/{defect}: {result:?}");
                }
            }
        }
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        for directory in [false, true] {
            let path = root.path().join(format!("{suffix}-{directory}.gpkg"));
            std::fs::copy(fixture("gpkg"), &path).unwrap();
            let member = PathBuf::from(format!("{}{suffix}", path.display()));
            if directory {
                std::fs::create_dir(&member).unwrap();
            } else {
                std::fs::write(&member, b"SECRET garbage").unwrap();
            }
            assert_error(&path, "gpkg", "GeoPackage", Situation::InvalidContent);
        }
    }
}

/// La capability precede la struttura in ogni ingresso CLI del profilo base.
#[cfg(not(feature = "gdal-backend"))]
#[test]
fn base_filegdb_tier_precedes_all_five_container_cases() {
    let root = tempfile::tempdir().unwrap();
    for case in [
        "healthy",
        "corrupt_catalog",
        "defective_member",
        "empty",
        "qualsiasi",
    ] {
        let path = root.path().join(format!("{case}.gdb"));
        if matches!(case, "empty" | "qualsiasi") {
            // `qualsiasi.gdb`: fixture esatta di scripts/smoke-profilo.py.
            std::fs::create_dir(&path).unwrap();
        } else {
            copy_tree(&fixture("gdb"), &path);
            let damaged = match case {
                "corrupt_catalog" => Some("a00000001.gdbtable"),
                "defective_member" => Some("a00000009.gdbtable"),
                _ => None,
            };
            if let Some(member) = damaged {
                std::fs::write(path.join(member), b"SECRET garbage").unwrap();
            }
        }
        assert_error(&path, "filegdb", "FileGDB", Situation::InvalidContent);
    }
}

#[test]
fn a_generic_container_is_not_the_expected_format() {
    let root = tempfile::tempdir().unwrap();
    for (ext, bytes, id, label) in [
        (
            "geojson",
            b"{\"type\":\"not-geojson\"}".as_slice(),
            "geojson",
            "GeoJSON",
        ),
        (
            "kml",
            b"<?xml version=\"1.0\"?><other/>".as_slice(),
            "kml",
            "KML",
        ),
    ] {
        let path = root.path().join(format!("other.{ext}"));
        std::fs::write(&path, bytes).unwrap();
        assert_error(&path, id, label, Situation::InvalidContent);
    }
    let sqlite = root.path().join("other.gpkg");
    let connection = rusqlite::Connection::open(&sqlite).unwrap();
    connection
        .execute_batch("CREATE TABLE ordinary (value TEXT);")
        .unwrap();
    drop(connection);
    assert_error(&sqlite, "gpkg", "GeoPackage", Situation::InvalidContent);
    let archive = root.path().join("other.xlsx");
    zip::ZipWriter::new(std::fs::File::create(&archive).unwrap())
        .finish()
        .unwrap();
    assert_error(&archive, "xls", "XLSX", Situation::InvalidContent);
}

#[test]
fn csv_has_no_magic_or_required_end_marker() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("prefix.csv");
    // Un'intestazione singola e un taglio su un confine di record restano CSV validi.
    for bytes in [b"one_header".as_slice(), b"a,b\n".as_slice()] {
        std::fs::write(&path, bytes).unwrap();
        let (_, result) = plenora_io_tools::esegui(
            &["read".to_owned(), path.to_str().unwrap().to_owned()],
            &CancellationToken::new(),
        );
        assert!(result.is_ok(), "{result:?}");
    }
}

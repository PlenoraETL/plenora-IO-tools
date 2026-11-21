//! Sorgenti da byte e da archivio sulla superficie CLI (L-IO1).
//!
//! Le stesse regole del core, viste dal processo: `--source-stdin` per i byte,
//! `--archive-entry` per le voci, `archive-list` per l'elenco, e una sola
//! regola d'errore con codici stabili per voce assente, nome non portabile,
//! archivio annidato e limite.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/canoniche")
        .join(nome)
}

fn esegui(argomenti: &[&str], stdin: Option<&[u8]>) -> Value {
    let mut comando = Command::new(BINARIO);
    comando
        .args(argomenti)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    comando.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut figlio = comando.spawn().expect("il binario parte");
    if let Some(dati) = stdin {
        let mut ingresso = figlio.stdin.take().unwrap();
        // Il figlio puo' rifiutare prima di leggere tutto: un tubo chiuso non e' un guasto.
        let _ = ingresso.write_all(dati);
    }
    let uscita = figlio.wait_with_output().unwrap();
    assert!(uscita.stderr.is_empty(), "stderr non vuoto");
    serde_json::from_slice(&uscita.stdout).expect("la busta e' JSON")
}

fn zip(voci: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (nome, dati) in voci {
        zip.start_file(
            *nome,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(dati).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn codice(busta: &Value) -> &str {
    busta["error"]["code"].as_str().unwrap_or("")
}

fn righe(busta: &Value) -> u64 {
    busta["result"]["rows_read"]
        .as_u64()
        .or_else(|| busta["result"]["rows"].as_u64())
        .unwrap_or_else(|| panic!("conteggio delle righe: {busta}"))
}

#[test]
fn stdin_bytes_read_like_the_file_and_never_open_the_name() {
    let geojson = std::fs::read(fixture("canonico.geojson")).unwrap();
    let dal_file = esegui(
        &["read", fixture("canonico.geojson").to_str().unwrap()],
        None,
    );
    // Il nome logico non esiste sul disco: si leggono i byte, non il nome.
    let da_stdin = esegui(
        &["read", "inesistente.geojson", "--source-stdin"],
        Some(&geojson),
    );
    assert_eq!(da_stdin["status"], "ok", "{da_stdin}");
    assert_eq!(righe(&dal_file), righe(&da_stdin));
    // Oltre --max-input-bytes: limite, con il nome della quota.
    let troppi = esegui(
        &[
            "read",
            "x.geojson",
            "--source-stdin",
            "--max-input-bytes",
            "8",
        ],
        Some(&geojson),
    );
    assert_eq!(codice(&troppi), "LIMIT_EXCEEDED", "{troppi}");
    // Un flag senza ingresso utile e un nome non portabile: errori d'uso.
    let nome = esegui(&["read", "../x.geojson", "--source-stdin"], Some(&geojson));
    assert_eq!(codice(&nome), "CLI_USAGE", "{nome}");
    let doppio = esegui(
        &["read", "x.geojson", "--source-stdin", "--source-stdin"],
        Some(&geojson),
    );
    assert_eq!(codice(&doppio), "CLI_USAGE", "{doppio}");
    let base64 = esegui(&["read", "x.geojson", "--source-base64", "e30="], None);
    assert_eq!(
        codice(&base64),
        "CLI_USAGE",
        "--source-base64 non esiste piu': {base64}"
    );
}

#[test]
fn archive_entry_errors_follow_one_rule() {
    let directory = tempfile::tempdir().unwrap();
    let geojson = std::fs::read(fixture("canonico.geojson")).unwrap();
    let kmz = zip(&[("doc.kml", &std::fs::read(fixture("canonico.kml")).unwrap())]);
    let archivio = directory.path().join("dati.zip");
    std::fs::write(
        &archivio,
        zip(&[("Dir/A.geojson", &geojson), ("dentro.kmz", &kmz)]),
    )
    .unwrap();
    let archivio = archivio.to_str().unwrap();
    let letto = esegui(
        &["read", archivio, "--archive-entry", "dir/a.GEOJSON"],
        None,
    );
    assert_eq!(letto["status"], "ok", "{letto}");
    for (voce, atteso) in [
        ("dir/b.geojson", "SOURCE_NOT_FOUND"),
        ("../a.geojson", "CLI_USAGE"),
        ("CON.geojson", "CLI_USAGE"),
        ("dentro.kmz", "UNSUPPORTED"),
    ] {
        for comando in ["read", "inspect", "layers"] {
            let busta = esegui(&[comando, archivio, "--archive-entry", voce], None);
            assert_eq!(codice(&busta), atteso, "{comando} {voce}: {busta}");
        }
    }
    // Lo stesso archivio da stdin: stessa voce, stesso esito.
    let byte = std::fs::read(archivio).unwrap();
    let da_stdin = esegui(
        &[
            "read",
            "dati.zip",
            "--source-stdin",
            "--archive-entry",
            "dir/a.geojson",
        ],
        Some(&byte),
    );
    assert_eq!(righe(&da_stdin), righe(&letto));
    // KMZ come file: il driver lo riconosce dal suffisso.
    let kmz_path = directory.path().join("mappa.KMZ");
    std::fs::write(&kmz_path, &kmz).unwrap();
    let busta = esegui(&["layers", kmz_path.to_str().unwrap()], None);
    assert_eq!(busta["status"], "ok", "{busta}");
}

#[test]
fn archive_list_declares_sizes_and_effective_limits() {
    let directory = tempfile::tempdir().unwrap();
    let archivio = directory.path().join("dati.zip");
    std::fs::write(&archivio, zip(&[("a.csv", b"x\n1\n"), ("b/c.csv", b"y\n")])).unwrap();
    let archivio = archivio.to_str().unwrap();
    let busta = esegui(
        &["archive-list", archivio, "--max-input-entries", "5"],
        None,
    );
    assert_eq!(busta["contract"], "plenora-io-archive-list-v1", "{busta}");
    let voci = busta["result"]["entries"].as_array().unwrap();
    assert_eq!(voci[0]["name"], "a.csv");
    assert_eq!(voci[0]["declared_uncompressed_bytes"], 4);
    let limiti = busta["result"]["limits"].as_array().unwrap();
    assert_eq!(limiti[0]["quota"], "archive_max_entries");
    assert_eq!(limiti[0]["limit"], 5);
    assert_eq!(limiti[0]["bound_by"], "max_input_entries");
    assert_eq!(limiti[1]["bound_by"], "max_input_bytes");
    for (argomenti, atteso) in [
        (vec!["--archive-max-entries", "0"], "CLI_USAGE"),
        (
            vec!["--archive-max-bytes", "9", "--archive-max-bytes", "9"],
            "CLI_USAGE",
        ),
        (vec!["--archive-entry", "a.csv"], "CLI_USAGE"),
        (vec!["--assume-crs", "EPSG:4326"], "CLI_USAGE"),
        (vec!["--archive-max-entries", "1"], "LIMIT_EXCEEDED"),
    ] {
        let mut tutti = vec!["archive-list", archivio];
        tutti.extend(argomenti.iter().copied());
        let busta = esegui(&tutti, None);
        assert_eq!(codice(&busta), atteso, "{argomenti:?}: {busta}");
    }
    let superato = esegui(
        &["archive-list", archivio, "--archive-max-entries", "1"],
        None,
    );
    assert_eq!(
        superato["error"]["details"]["quota"], "archive_max_entries",
        "{superato}"
    );
}

#[test]
fn write_measures_content_type_on_the_entry_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let flusso = directory.path().join("a.arrows");
    let consegna = esegui(
        &[
            "read",
            fixture("canonico.geojson").to_str().unwrap(),
            "--output",
            flusso.to_str().unwrap(),
            "--out-opt",
            "serialization=stream",
        ],
        None,
    );
    assert_eq!(consegna["status"], "ok", "{consegna}");
    let archivio = directory.path().join("p.zip");
    std::fs::write(
        &archivio,
        zip(&[("dentro.arrow", &std::fs::read(&flusso).unwrap())]),
    )
    .unwrap();
    // Un file omonimo nella cartella non deve essere letto al posto della voce.
    std::fs::copy(
        fixture("canonico.arrow"),
        directory.path().join("dentro.arrow"),
    )
    .unwrap();
    let busta = esegui(
        &[
            "write",
            archivio.to_str().unwrap(),
            directory.path().join("o.csv").to_str().unwrap(),
            "--to",
            "csv",
            "--archive-entry",
            "dentro.arrow",
        ],
        None,
    );
    assert_eq!(busta["status"], "ok", "{busta}");
    assert_eq!(
        busta["result"]["input"]["content_type"],
        "application/vnd.apache.arrow.stream"
    );
}

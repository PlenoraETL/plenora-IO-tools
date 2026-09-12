//! Le due superfici rendono la stessa cosa, e qui si misura.
//!
//! # Che cosa SURF-017 e CLI-2.0 §10 pretendono
//!
//! Che il nome di un comando sia una grafia dell'operazione, e che «operation
//! version, input/output contract, errors and side effects MUST remain
//! equivalent to the other advertised surfaces». Finché la superficie Rust non
//! esisteva, l'equivalenza non era verificabile: mancava un termine di
//! confronto.
//!
//! # Perché è una conseguenza e non una promessa
//!
//! Le due superfici chiamano la **stessa funzione**: `main.rs` è il binding di
//! processo — flussi, codice d'uscita, radici dell'artefatto — e
//! `operazioni::*` è la porta in-process. Non c'è una seconda
//! implementazione da tenere allineata, quindi non c'è la classe di difetti
//! che l'allineamento produce.
//!
//! Restano due cose che la costruzione **non** garantisce, e sono quelle che
//! queste prove misurano: che il binding non aggiunga o tolga campi mentre
//! costruisce la busta, e che la traduzione fra `Richiesta` e argomenti non
//! cambi il significato di un ingresso. Un binding che leggesse la
//! destinazione dal posizionale sbagliato passerebbe ogni prova interna e
//! renderebbe due risultati diversi dalle due porte.

use std::path::{Path, PathBuf};
use std::process::Command;

use plenora_io_cli::operazioni::{self, Richiesta};
use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("canoniche")
        .join(nome)
}

/// Il corpo che la CLI mette in `result`, o la busta d'errore.
fn dalla_cli(argomenti: &[String]) -> Result<Value, Value> {
    let uscita = Command::new(BINARIO)
        .args(argomenti)
        .arg("--format")
        .arg("json")
        .output()
        .expect("il binario parte");
    let stdout = String::from_utf8(uscita.stdout).expect("stdout e' UTF-8");
    let busta: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("{argomenti:?}: stdout e' JSON: {e} -- {stdout}"));
    if busta["status"] == "ok" {
        Ok(busta["result"].clone())
    } else {
        Err(busta)
    }
}

/// I campi d'identità che il binding aggiunge: sono suoi, non dell'operazione.
fn senza_identita(busta: &Value) -> Value {
    let mut copia = busta.clone();
    if let Some(oggetto) = copia.as_object_mut() {
        for campo in [
            "status",
            "protocol_version",
            "component",
            "component_version",
            "contract",
            "command",
        ] {
            oggetto.remove(campo);
        }
    }
    copia
}

#[test]
fn io_catalog_rende_lo_stesso_documento_dalle_due_porte() {
    let dalla_libreria = operazioni::catalog();
    let dal_processo = dalla_cli(&["catalog".to_owned()]).expect("il catalogo riesce");
    assert_eq!(
        dal_processo, dalla_libreria,
        "il binding non deve aggiungere né togliere nulla al documento dell'operazione"
    );
}

#[test]
fn capabilities_rende_lo_stesso_documento_dalle_due_porte() {
    let dalla_libreria = operazioni::capabilities();
    let dal_processo = dalla_cli(&["capabilities".to_owned()]).expect("capabilities riesce");
    assert_eq!(dal_processo, dalla_libreria);
}

#[test]
fn io_inspect_e_io_layers_rendono_lo_stesso_documento() {
    for (nome, sorgente) in [
        ("inspect", "canonico.geojson"),
        ("inspect", "canonico.gpkg"),
        ("layers", "canonico.gpkg"),
        ("layers", "canonico.geojson"),
    ] {
        let percorso = fixture(sorgente);
        let richiesta = Richiesta::sulla_sorgente(&percorso);
        let dalla_libreria = match nome {
            "inspect" => operazioni::inspect(richiesta),
            _ => operazioni::layers(richiesta),
        }
        .expect("l'operazione riesce sulla fixture");

        let dal_processo = dalla_cli(&[nome.to_owned(), percorso.display().to_string()])
            .expect("il comando riesce");

        assert_eq!(
            dal_processo, dalla_libreria,
            "{nome} su {sorgente}: le due porte rendono documenti diversi"
        );
    }
}

/// `io.read` nelle **due** forme: la destinazione è il campo che le distingue,
/// e la traduzione fra `Richiesta` e argomenti è dove potrebbe perdersi.
#[test]
fn io_read_rende_lo_stesso_documento_nelle_due_forme() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let sorgente = fixture("canonico.geojson");

    // Senza consegna.
    let dalla_libreria =
        operazioni::read(Richiesta::sulla_sorgente(&sorgente)).expect("la lettura riesce");
    let dal_processo =
        dalla_cli(&["read".to_owned(), sorgente.display().to_string()]).expect("riesce");
    assert_eq!(
        dal_processo, dalla_libreria,
        "la forma che conta deve essere identica dalle due porte"
    );

    // Con consegna: due destinazioni distinte, perché una destinazione
    // occupata è un conflitto e falserebbe il confronto.
    let dalla_libreria = operazioni::read(
        Richiesta::sulla_sorgente(&sorgente)
            .con_destinazione(temporanea.path().join("dalla-libreria.arrow")),
    )
    .expect("la consegna riesce");
    let dal_processo = dalla_cli(&[
        "read".to_owned(),
        sorgente.display().to_string(),
        "--output".to_owned(),
        temporanea
            .path()
            .join("dal-processo.arrow")
            .display()
            .to_string(),
    ])
    .expect("riesce");

    // `bytes_written` è lo stesso perché i byte sono gli stessi; il percorso
    // non compare nel documento, ed è la ragione per cui il confronto è
    // possibile senza normalizzare niente.
    assert_eq!(
        dal_processo, dalla_libreria,
        "la forma che consegna deve essere identica dalle due porte"
    );
}

#[test]
fn io_write_e_io_convert_rendono_lo_stesso_documento() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let sorgente = fixture("canonico.geojson");
    let arrow = temporanea.path().join("ponte.arrow");
    operazioni::read(Richiesta::sulla_sorgente(&sorgente).con_destinazione(&arrow))
        .expect("la consegna che alimenta la prova");

    let dalla_libreria = operazioni::write(
        Richiesta::sulla_sorgente(&arrow)
            .con_destinazione(temporanea.path().join("libreria.csv"))
            .con_formato_destinazione("csv"),
    )
    .expect("la pubblicazione riesce");
    let dal_processo = dalla_cli(&[
        "write".to_owned(),
        arrow.display().to_string(),
        temporanea.path().join("processo.csv").display().to_string(),
        "--to".to_owned(),
        "csv".to_owned(),
    ])
    .expect("riesce");
    assert_eq!(dal_processo, dalla_libreria, "io.write dalle due porte");

    let dalla_libreria = operazioni::convert(
        Richiesta::sulla_sorgente(&sorgente)
            .con_destinazione(temporanea.path().join("libreria-conv.csv"))
            .con_formato_sorgente("geojson")
            .con_formato_destinazione("csv"),
    )
    .expect("la conversione riesce");
    let dal_processo = dalla_cli(&[
        "convert".to_owned(),
        sorgente.display().to_string(),
        temporanea
            .path()
            .join("processo-conv.csv")
            .display()
            .to_string(),
        "--from".to_owned(),
        "geojson".to_owned(),
        "--to".to_owned(),
        "csv".to_owned(),
    ])
    .expect("riesce");
    assert_eq!(dal_processo, dalla_libreria, "io.convert dalle due porte");
}

/// Gli stessi ingressi sono rifiutati, con gli stessi assi.
///
/// # Perché il codice d'uscita non entra nel confronto
///
/// Perché non è dell'operazione: è la proiezione che il binding di processo
/// applica alla categoria. La libreria non lo rende, e un'API che lo rendesse
/// costringerebbe chi la usa a ragionare su un numero che riguarda un processo
/// che non ha avviato. Ciò che deve coincidere sono i **quattro assi**, il
/// codice e la fase — cioè tutto ciò su cui una macchina decide.
#[test]
fn gli_stessi_ingressi_sono_rifiutati_con_gli_stessi_assi() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let sorgente = fixture("canonico.geojson");

    let casi: Vec<(&str, Richiesta, Vec<String>)> = vec![
        (
            "sorgente inesistente",
            Richiesta::sulla_sorgente("/nessun-file-per-l-equivalenza.geojson"),
            vec![
                "read".to_owned(),
                "/nessun-file-per-l-equivalenza.geojson".to_owned(),
            ],
        ),
        (
            "limite con consegna",
            Richiesta::sulla_sorgente(&sorgente)
                .con_destinazione(temporanea.path().join("mai-libreria.arrow"))
                .con_limite(2),
            vec![
                "read".to_owned(),
                sorgente.display().to_string(),
                "--output".to_owned(),
                temporanea
                    .path()
                    .join("mai-processo.arrow")
                    .display()
                    .to_string(),
                "--limit".to_owned(),
                "2".to_owned(),
            ],
        ),
        (
            "formato del sink fuori dal catalogo",
            Richiesta::sulla_sorgente(&sorgente)
                .con_destinazione(temporanea.path().join("mai-libreria.dat"))
                .con_formato_destinazione("shapefile"),
            vec![
                "write".to_owned(),
                sorgente.display().to_string(),
                temporanea
                    .path()
                    .join("mai-processo.dat")
                    .display()
                    .to_string(),
                "--to".to_owned(),
                "shapefile".to_owned(),
            ],
        ),
    ];

    for (caso, richiesta, argomenti) in casi {
        let dalla_libreria = match argomenti[0].as_str() {
            "read" => operazioni::read(richiesta),
            _ => operazioni::write(richiesta),
        };
        let dal_processo = dalla_cli(&argomenti);

        let Err(errore_libreria) = dalla_libreria else {
            panic!("{caso}: la libreria non ha rifiutato");
        };
        let Err(busta_processo) = dal_processo else {
            panic!("{caso}: il processo non ha rifiutato");
        };

        assert_eq!(
            senza_identita(&busta_processo),
            senza_identita(&errore_libreria),
            "{caso}: le due porte rifiutano con assi diversi"
        );
    }
}

//! Dove va l'avviso del protocollo legacy, e dove non va.
//!
//! # Il contratto, in tre righe
//!
//! * in caso di **errore**, `stderr` contiene sempre e soltanto la busta JSON;
//! * in caso di **successo legacy**, `stdout` conserva il v1 byte per byte e
//!   `stderr` porta l'avviso;
//! * con il protocollo predefinito e successo, `stderr` resta vuoto.
//!
//! # Perche' e' cosi' e non altrimenti
//!
//! L'avviso stava all'inizio di `run`, prima di qualunque esito: bastava che il
//! flag comparisse fra gli argomenti. Su un comando che poi falliva finiva
//! **davanti alla busta**, e chi legge `stderr` con un parser vi trovava due
//! documenti dove il contratto ne promette uno.
//!
//! Sul percorso d'errore non c'e' nemmeno niente da avvertire: una busta v1 non
//! e' stata consegnata, quindi la diagnostica illimitata di cui l'avviso parla
//! non esiste. L'avviso accompagna una **consegna riuscita**, ed e' li' che
//! serve.
//!
//! Queste tre sonde girano sul binario vero, non su una funzione: e' l'unico
//! posto dove `stdout` e `stderr` sono due flussi separati davvero.

use std::path::{Path, PathBuf};
use std::process::Command;

const FLAG: &str = "--legacy-protocol-v1-unsafe";

const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

/// Una sorgente CSV minima e valida.
fn sorgente(dir: &Path) -> PathBuf {
    let percorso = dir.join("ingresso.csv");
    std::fs::write(&percorso, "id,geom\n1,POINT(1 2)\n").expect("sorgente scritta");
    percorso
}

fn inspect(percorso: &Path, legacy: bool) -> std::process::Output {
    let mut comando = Command::new(binario());
    // CRS e colonna WKT non sono dettagli della fixture: senza, il CSV viene
    // rifiutato in validazione o in lettura, e le due sonde del **successo**
    // proverebbero un errore. `--opt` e non `--in-opt`: `inspect` legge una
    // sorgente sola, e le opzioni d'ingresso e d'uscita le distingue `convert`.
    comando
        .arg("inspect")
        .arg(percorso)
        .arg("--assume-crs")
        .arg("EPSG:4326")
        .arg("--opt")
        .arg("wkt_column=geom");
    if legacy {
        comando.arg(FLAG);
    }
    comando.output().expect("il binario si esegue")
}

fn testo(byte: &[u8]) -> String {
    String::from_utf8(byte.to_vec()).expect("l'uscita e' UTF-8")
}

/// Successo legacy: il v1 su `stdout`, un avviso solo su `stderr`.
#[test]
fn un_successo_legacy_porta_l_avviso_su_stderr_e_il_v1_su_stdout() {
    let dir = tempfile::tempdir().expect("directory temporanea");
    let percorso = sorgente(dir.path());

    let esito = inspect(&percorso, true);
    assert!(
        esito.status.success(),
        "l'ispezione di una sorgente valida riesce.\nstderr: {}",
        testo(&esito.stderr)
    );

    let stderr = testo(&esito.stderr);
    assert!(
        stderr.contains("protocollo v1 legacy"),
        "l'avviso accompagna la consegna legacy: «{stderr}»"
    );
    assert_eq!(
        stderr.matches("protocollo v1 legacy").count(),
        1,
        "una volta sola, non una per comando interno: «{stderr}»"
    );

    // Su `stdout` il documento resta quello congelato: l'avviso non ci entra.
    let documento: serde_json::Value =
        serde_json::from_str(testo(&esito.stdout).trim()).expect("stdout e' il documento JSON");
    assert_eq!(
        documento["protocol_version"], 1,
        "il flag ha davvero selezionato il v1"
    );
    assert!(
        !testo(&esito.stdout).contains("protocollo v1 legacy"),
        "l'avviso non va su stdout: il v1 e' congelato byte per byte"
    );
}

/// Errore con il flag legacy: `stderr` e' una busta e basta.
///
/// E' la regressione vera. Con l'avviso all'inizio di `run`, questo `stderr`
/// portava una riga di testo e poi la busta, e il parser di un consumatore si
/// fermava sulla prima.
#[test]
fn un_errore_con_il_flag_legacy_lascia_su_stderr_una_sola_busta() {
    let dir = tempfile::tempdir().expect("directory temporanea");
    let assente = dir.path().join("questa-non-esiste.csv");

    let esito = inspect(&assente, true);
    assert!(
        !esito.status.success(),
        "una sorgente inesistente non si ispeziona"
    );

    let stderr = testo(&esito.stderr);
    assert!(
        !stderr.contains("protocollo v1 legacy"),
        "sul percorso d'errore non c'e' una busta v1 da avvertire: «{stderr}»"
    );
    // L'**intero** flusso, non l'ultima riga: analizzare l'ultima renderebbe
    // questa sonda compatibile con una riga di troppo invece di rifiutarla.
    // Un `match` e non un `unwrap_or_else`: non c'e' un valore di ripiego, e il
    // censimento dei fallback conta la forma sintattica.
    let documento: serde_json::Value = match serde_json::from_str(stderr.trim()) {
        Ok(documento) => documento,
        Err(errore) => panic!("stderr non e' una sola busta JSON ({errore}): «{stderr}»"),
    };
    assert_eq!(documento["status"], "error");
}

/// Successo con il protocollo predefinito: `stderr` resta vuoto.
///
/// Senza questa riga, un avviso emesso sempre passerebbe le altre due: la
/// prima lo troverebbe, la seconda no perche' li' non c'e' successo.
#[test]
fn un_successo_predefinito_non_scrive_niente_su_stderr() {
    let dir = tempfile::tempdir().expect("directory temporanea");
    let percorso = sorgente(dir.path());

    let esito = inspect(&percorso, false);
    assert!(
        esito.status.success(),
        "l'ispezione di una sorgente valida riesce.\nstderr: {}",
        testo(&esito.stderr)
    );
    assert_eq!(
        testo(&esito.stderr),
        "",
        "con il protocollo predefinito non c'e' niente da avvertire"
    );
}

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

// --- `catalog` e `--version`: i due punti d'ingresso senza sorgente ----------
//
// # La regressione che queste sonde chiudono
//
// Il dispatch teneva `catalog` e `--version` fuori dal parsing degli
// argomenti: `args.first()` decideva il comando e tutto il resto veniva
// scartato senza guardarlo. Ne seguivano due difetti di segno opposto.
//
// Il primo: `catalog --legacy-protocol-v1-unsafe` consegnava una busta **v2**
// senza avviso, a chi aveva chiesto il v1 a voce alta. Il v1 di `catalog` non
// era un'ipotesi: `release/cli-protocol-v1.json` lo dichiara fra le sei buste,
// `check_release_contract.py` lo pretende, `cmd_catalog` sa gia' produrlo. Era
// il dispatch a non chiamarlo piu'.
//
// Il secondo: `catalog --questa-non-esiste` e `catalog /etc/passwd` uscivano
// con zero e una busta buona. Gli altri quattro comandi rifiutano un'opzione
// sconosciuta con `CLI_USAGE`; questi due accettavano qualunque cosa e non ne
// facevano niente -- che e' il modo in cui un errore di battitura in uno script
// resta invisibile finche' qualcuno non si chiede perche' quell'opzione non
// abbia mai avuto effetto.

/// Esegue il binario con argomenti nudi e restituisce l'esito.
fn nudo(argomenti: &[&str]) -> std::process::Output {
    let mut comando = Command::new(binario());
    comando.args(argomenti);
    comando.output().expect("il binario si esegue")
}

/// La busta su `stdout`, quando il comando riesce.
fn documento_di(esito: &std::process::Output) -> serde_json::Value {
    match serde_json::from_str(testo(&esito.stdout).trim()) {
        Ok(documento) => documento,
        Err(errore) => panic!(
            "stdout non e' una busta JSON ({errore}): «{}»",
            testo(&esito.stdout)
        ),
    }
}

/// Un rifiuto d'uso: exit diverso da zero, e su `stderr` la busta col codice.
fn rifiuto_d_uso(esito: &std::process::Output, quando: &str) {
    assert!(
        !esito.status.success(),
        "{quando} deve fallire.\nstdout: {}",
        testo(&esito.stdout)
    );
    let stderr = testo(&esito.stderr);
    let documento: serde_json::Value = match serde_json::from_str(stderr.trim()) {
        Ok(documento) => documento,
        Err(errore) => panic!("stderr non e' una sola busta ({errore}): «{stderr}»"),
    };
    assert_eq!(documento["status"], "error", "{quando}");
    assert_eq!(
        documento["error"]["code"], "CLI_USAGE",
        "{quando}: il codice dice che l'uso e' sbagliato"
    );
    assert_eq!(
        testo(&esito.stdout),
        "",
        "{quando}: su un rifiuto stdout resta vuoto"
    );
}

/// Il predefinito non cambia: senza argomenti, `catalog` parla v2.
#[test]
fn catalog_senza_argomenti_consegna_il_v2() {
    let esito = nudo(&["catalog"]);
    assert!(esito.status.success(), "il catalogo si legge sempre");
    let documento = documento_di(&esito);
    assert_eq!(documento["protocol_version"], 2);
    assert_eq!(documento["contract"], "plenora-io-catalog-v2");
    assert_eq!(
        testo(&esito.stderr),
        "",
        "sul predefinito non c'e' niente da avvertire"
    );
}

/// La regressione vera: il v1 di `catalog` esiste, ed e' raggiungibile.
#[test]
fn catalog_col_flag_legacy_consegna_il_v1_con_l_avviso() {
    let esito = nudo(&["catalog", FLAG]);
    assert!(
        esito.status.success(),
        "il catalogo legacy si consegna.\nstderr: {}",
        testo(&esito.stderr)
    );

    let documento = documento_di(&esito);
    assert_eq!(
        documento["protocol_version"], 1,
        "chi chiede il v1 riceve il v1"
    );
    assert_eq!(
        documento["contract"], "plenora-io-catalog-v1",
        "ed e' il contratto che `release/cli-protocol-v1.json` dichiara"
    );

    let stderr = testo(&esito.stderr);
    assert!(
        stderr.contains("protocollo v1 legacy"),
        "l'avviso accompagna la consegna legacy anche qui: «{stderr}»"
    );
    assert!(
        !testo(&esito.stdout).contains("protocollo v1 legacy"),
        "e non entra nel documento congelato"
    );
}

/// Il catalogo dei due protocolli differisce **solo** dove deve.
///
/// Senza questa riga, un `cmd_catalog` che ignorasse il protocollo nei driver
/// e lo onorasse nell'intestazione passerebbe la sonda precedente.
#[test]
fn i_due_cataloghi_differiscono_solo_nell_intestazione() {
    let v2 = documento_di(&nudo(&["catalog"]));
    let v1 = documento_di(&nudo(&["catalog", FLAG]));
    assert_eq!(
        v1["drivers"], v2["drivers"],
        "i driver sono gli stessi: a cambiare e' la busta, non il prodotto"
    );
    assert_eq!(v1["determinism"], v2["determinism"]);
    assert_eq!(v1["status"], v2["status"]);
}

/// Un'opzione che non esiste non si ignora.
#[test]
fn catalog_rifiuta_un_opzione_sconosciuta() {
    rifiuto_d_uso(
        &nudo(&["catalog", "--questa-opzione-non-esiste"]),
        "un'opzione sconosciuta",
    );
}

/// Un percorso passato a `catalog` non ha significato: `catalog` non legge file.
#[test]
fn catalog_rifiuta_un_percorso() {
    rifiuto_d_uso(
        &nudo(&["catalog", "/etc/passwd"]),
        "un percorso posizionale",
    );
}

/// Un'opzione buona **per un altro comando** resta sbagliata qui.
#[test]
fn catalog_rifiuta_un_opzione_di_un_altro_comando() {
    rifiuto_d_uso(&nudo(&["catalog", "--limit", "5"]), "`--limit`");
    rifiuto_d_uso(
        &nudo(&["catalog", "--assume-crs", "EPSG:4326"]),
        "`--assume-crs`",
    );
}

/// Il flag ripetuto e' un uso sbagliato, non un v1 due volte.
///
/// Accettarlo sarebbe innocuo oggi -- il protocollo e' lo stesso -- e questo e'
/// esattamente il motivo per cui va rifiutato adesso: e' il caso in cui una
/// tolleranza non costa niente, e la si concede senza accorgersene.
#[test]
fn catalog_rifiuta_il_flag_duplicato() {
    rifiuto_d_uso(&nudo(&["catalog", FLAG, FLAG]), "il flag ripetuto");
}

/// `--version` risponde con due campi, e nient'altro.
#[test]
fn version_senza_coda_ha_esattamente_due_campi() {
    for forma in ["--version", "-V"] {
        let esito = nudo(&[forma]);
        assert!(esito.status.success(), "`{forma}` risponde");
        let documento = documento_di(&esito);
        let campi = documento
            .as_object()
            .expect("la busta di bootstrap e' un oggetto");
        assert_eq!(
            campi.len(),
            2,
            "`{forma}`: `status` e `version`, ne' uno di piu': {campi:?}"
        );
        assert_eq!(documento["status"], "ok");
        assert!(documento["version"].is_string());
    }
}

/// `--version` con una coda e' un uso sbagliato.
///
/// Il flag legacy in coda e' il caso che inganna di piu': la busta di bootstrap
/// **non e'** una busta v2, non ha un v1, e accettare il flag li' farebbe
/// credere il contrario a chi lo scrive.
#[test]
fn version_rifiuta_qualunque_coda() {
    rifiuto_d_uso(&nudo(&["--version", FLAG]), "`--version` col flag legacy");
    rifiuto_d_uso(&nudo(&["-V", "catalog"]), "`-V` seguito da un comando");
    rifiuto_d_uso(
        &nudo(&["--version", "/etc/passwd"]),
        "`--version` con un percorso",
    );
}

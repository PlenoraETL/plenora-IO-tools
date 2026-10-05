//! Un DXF su cui il lettore non avanza esce con la busta, non con un abort.
//!
//! Il soak di `dxf_reader` del 2026-10-05 ha trovato 235 byte su cui la 4.0.0
//! e la 4.1.0 allocavano sei gigabyte e abortivano con exit 134: nessuna busta
//! su stdout, e un codice d'uscita che il contratto non conosce. La causa e'
//! nel fork di `dxf` -- un lettore che restituiva un valore senza consumare
//! input, dentro un ciclo che lo richiamava -- e la correzione sta li', con le
//! sue prove in `driver-dxf`.
//!
//! Qui si prova cio' che vede chi usa il **binario**: busta d'errore su stdout,
//! stderr vuoto, il codice d'uscita della categoria, e un ritorno entro un tempo
//! massimo. Il tempo e' la parte che una prova in-process non puo' dare: se il
//! difetto tornasse, la chiamata non ritornerebbe, e un test che la aspetta
//! resterebbe appeso invece di diventare rosso. Il processo figlio invece si
//! puo' uccidere, e la prova fallisce dicendo perche'.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Ampio per una build di debug su un runner lento: il binario corretto
/// risponde in decine di millisecondi, quello difettoso non risponde mai.
const TEMPO_MASSIMO: Duration = Duration::from_secs(60);

fn caso() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/seeds/dxf_reader/sectionsettings-senza-progresso.dxf")
}

struct Esito {
    codice: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Esegue il binario e lo uccide se non ritorna entro `TEMPO_MASSIMO`.
///
/// stdout e stderr si leggono a processo concluso: la busta e' una riga, e un
/// figlio che riempisse una pipe senza mai finire viene comunque ucciso dalla
/// scadenza, che e' l'unica cosa che questa prova deve garantire.
fn esegui(argomenti: &[&str]) -> Esito {
    let mut figlio = Command::new(env!("CARGO_BIN_EXE_plenora-io"))
        .args(argomenti)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("il binario parte");
    let inizio = Instant::now();
    let stato = loop {
        if let Some(stato) = figlio.try_wait().expect("lo stato del figlio si legge") {
            break stato;
        }
        if inizio.elapsed() > TEMPO_MASSIMO {
            let _ = figlio.kill();
            let _ = figlio.wait();
            panic!(
                "{argomenti:?}: nessun ritorno entro {TEMPO_MASSIMO:?} -- il lettore DXF non avanza"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    figlio
        .stdout
        .take()
        .expect("stdout catturato")
        .read_to_end(&mut stdout)
        .expect("stdout si legge");
    figlio
        .stderr
        .take()
        .expect("stderr catturato")
        .read_to_end(&mut stderr)
        .expect("stderr si legge");
    Esito {
        codice: stato.code(),
        stdout,
        stderr,
    }
}

/// La busta d'errore del contratto: `FORMAT_ERROR`, categoria `data_mapping`,
/// exit `3`. Un abort darebbe stdout vuoto e un codice che non e' fra quelli
/// della proiezione -- 134 su Linux, nessun codice se ucciso da un segnale.
fn verifica_busta(comando: &str, esito: &Esito) {
    assert!(
        esito.stderr.is_empty(),
        "{comando}: stderr deve restare vuoto, trovati {} byte",
        esito.stderr.len()
    );
    let busta: Value = serde_json::from_slice(&esito.stdout).unwrap_or_else(|_| {
        panic!(
            "{comando}: stdout non e' una busta JSON ({} byte, exit {:?})",
            esito.stdout.len(),
            esito.codice
        )
    });
    assert_eq!(busta["status"], "error", "{comando}: {busta}");
    assert_eq!(busta["contract"], "plenora-error-v1", "{comando}: {busta}");
    assert_eq!(busta["command"], comando, "{comando}: {busta}");
    assert_eq!(busta["error"]["code"], "FORMAT_ERROR", "{comando}: {busta}");
    assert_eq!(
        busta["error"]["category"], "data_mapping",
        "{comando}: {busta}"
    );
    assert_eq!(
        busta["error"]["retry"]["kind"], "never",
        "{comando}: {busta}"
    );
    assert_eq!(esito.codice, Some(3), "{comando}: {busta}");
}

#[test]
fn il_caso_del_soak_esce_con_la_busta_in_read_inspect_e_layers() {
    let caso = caso();
    let percorso = caso.to_str().expect("percorso UTF-8");
    for comando in ["read", "inspect", "layers"] {
        let inizio = Instant::now();
        let esito = esegui(&[comando, percorso]);
        verifica_busta(comando, &esito);
        assert!(
            inizio.elapsed() < TEMPO_MASSIMO,
            "{comando}: la risposta deve arrivare in tempo breve"
        );
    }
}

/// `convert` apre la stessa sorgente, e in piu' non deve lasciare la
/// destinazione: il rifiuto arriva prima che lo staging pubblichi qualcosa.
#[test]
fn il_caso_del_soak_esce_con_la_busta_in_convert_senza_destinazione() {
    let caso = caso();
    let radice = tempfile::tempdir().expect("directory temporanea");
    let uscita = radice.path().join("uscita.geojson");
    let esito = esegui(&[
        "convert",
        caso.to_str().expect("percorso UTF-8"),
        uscita.to_str().expect("percorso UTF-8"),
        "--from",
        "dxf",
        "--to",
        "geojson",
        "--assume-crs",
        "EPSG:4326",
    ]);
    verifica_busta("convert", &esito);
    assert!(
        !uscita.exists(),
        "convert fallito non lascia la destinazione"
    );
}

/// La stessa famiglia per un'altra risorsa: gruppi `102/{` annidati duecentomila
/// volte esaurivano lo stack del thread principale, e il processo moriva senza
/// busta. Il fork ora ha un tetto di profondita', e il binario risponde.
#[test]
fn i_gruppi_annidati_escono_con_la_busta_invece_di_esaurire_lo_stack() {
    let radice = tempfile::tempdir().expect("directory temporanea");
    let percorso = radice.path().join("annidato.dxf");
    let livelli = 200_000;
    let mut testo = String::from("0\nSECTION\n2\nENTITIES\n0\nLINE\n");
    testo.push_str(&"102\n{a\n".repeat(livelli));
    testo.push_str(&"102\n}\n".repeat(livelli));
    testo.push_str("10\n0\n20\n0\n11\n1\n21\n1\n0\nENDSEC\n0\nEOF\n");
    std::fs::write(&percorso, testo).expect("la sorgente si scrive");
    let esito = esegui(&["read", percorso.to_str().expect("percorso UTF-8")]);
    verifica_busta("read", &esito);
}

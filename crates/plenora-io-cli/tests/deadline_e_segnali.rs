//! La deadline configurabile e la cancellazione cooperativa, dal binario.
//!
//! # Due proprieta' vicine e distinte
//!
//! La deadline e il segnale finiscono nello stesso posto — un'operazione che si
//! ferma prima di pubblicare — ma arrivano da due parti diverse e vanno provate
//! separatamente: la prima e' una quota che il chiamante dichiara, la seconda un
//! evento che arriva mentre la quota e' ancora buona.
//!
//! # Che cosa la prova del segnale puo' dire
//!
//! Che il processo esce con `130`, che la busta d'errore dichiara `CANCELLED` e
//! che **la directory di destinazione torna vuota**: nessuno staging, nessuna
//! destinazione pubblicata. Non dice **quando**: la cancellazione e'
//! cooperativa, quindi fra il segnale e il ritorno passa il tempo che passa fino
//! al prossimo punto di verifica. Misurare quel tempo dentro un test lo
//! renderebbe una prova sulla macchina invece che sul codice.
//!
//! # Perche' il segnale si prova solo su Unix
//!
//! Mandare un `SIGINT` a un processo figlio e' una riga di shell su Unix. Su
//! Windows l'equivalente — `GenerateConsoleCtrlEvent` — si applica a un gruppo
//! di console, non a un PID, e da un test che lancia il figlio senza console
//! propria non c'e' un modo che non sia fragile. La gestione **c'e'** su
//! entrambe le piattaforme, perche' `ctrlc` copre entrambe; la sonda no, e
//! dichiararlo e' meglio che una sonda che passa senza provare niente.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Righe della sorgente delle prove sulla deadline.
///
/// Abbastanza da rendere la conversione lunga rispetto a un millisecondo --
/// misurata, sono circa 270 ms per 13 MiB in build di debug -- e abbastanza
/// corta da non pesare sulla suite.
const RIGHE: usize = 40_000;

/// Righe della sorgente della prova sul segnale.
///
/// Qui la grandezza non e' un dettaglio di comodo: fra il momento in cui lo
/// staging compare e quello in cui la conversione finisce c'e' una finestra, e
/// il segnale deve cadere **dentro** quella finestra. Con 40 000 righe la
/// finestra e' di qualche centinaio di millisecondi, e una macchina carica
/// potrebbe far finire il figlio prima che il padre lo segnali: la sonda
/// diventerebbe rossa per un motivo che non c'entra con cio' che prova. Dieci
/// volte piu' righe portano la finestra a qualche secondo.
const RIGHE_SEGNALE: usize = 400_000;

const RIEMPIMENTO: usize = 320;

const CRS: &str = "EPSG:4326";
const OPZIONE_WKT: &str = "wkt_column=geom";

/// Exit code storico di `LIMIT_EXCEEDED` nella CLI.
const EXIT_LIMITE: i32 = 7;

/// `128 + SIGINT`, l'exit code della categoria `Cancelled`.
const EXIT_ANNULLATO: i32 = 130;

const fn binario() -> &'static str {
    env!("CARGO_BIN_EXE_plenora-io")
}

fn scrivi_sorgente(percorso: &Path, righe: usize) {
    let riempimento = "x".repeat(RIEMPIMENTO);
    let file = std::fs::File::create(percorso).expect("sorgente creata");
    let mut uscita = BufWriter::new(file);
    writeln!(uscita, "id,etichetta,geom").expect("intestazione scritta");
    for indice in 0..righe {
        let x = indice % 360;
        let y = indice % 180;
        writeln!(uscita, "{indice},{riempimento},POINT({x}.5 {y}.5)").expect("riga scritta");
    }
    uscita.flush().expect("sorgente completata");
}

/// Sorgente e destinazione in **due** directory.
///
/// Serve alla sonda del segnale: la prova che lo staging e' stato ripulito e'
/// che la directory di destinazione torni vuota, e una sorgente dentro la stessa
/// directory renderebbe quella verifica impossibile da scrivere.
fn ambiente(righe: usize) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let radice = tempfile::tempdir().expect("directory temporanea");
    let dir_sorgenti = radice.path().join("sorgenti");
    let dir_destinazioni = radice.path().join("destinazioni");
    std::fs::create_dir(&dir_sorgenti).expect("directory delle sorgenti");
    std::fs::create_dir(&dir_destinazioni).expect("directory delle destinazioni");
    let ingresso = dir_sorgenti.join("ingresso.csv");
    scrivi_sorgente(&ingresso, righe);
    (radice, ingresso, dir_destinazioni.join("uscita.arrow"))
}

fn comando_convert(ingresso: &Path, uscita: &Path, deadline_ms: Option<&str>) -> Command {
    let mut comando = Command::new(binario());
    comando
        .arg("convert")
        .arg(ingresso)
        .arg(uscita)
        .arg("--assume-crs")
        .arg(CRS)
        .arg("--in-opt")
        .arg(OPZIONE_WKT);
    if let Some(millisecondi) = deadline_ms {
        comando.arg("--deadline-ms").arg(millisecondi);
    }
    comando
}

/// `stderr` e' la busta d'errore: **un solo documento JSON**, e nient'altro.
///
/// Non e' una comodita' della sonda, e' il contratto. Un consumatore che legge
/// `stderr` con un parser JSON lo fa sull'intero flusso -- lo fa anche
/// `tests/ostili.rs` -- e qualunque riga di testo prima della busta lo rompe.
/// Analizzare soltanto l'ultima riga renderebbe questa sonda **compatibile**
/// con una riga di troppo invece di rifiutarla, che e' il modo in cui un
/// contratto si perde senza che nessun gate lo dica.
fn busta(stderr: &str) -> serde_json::Value {
    // Un `match` e non un `unwrap_or_else`: non c'e' un valore di ripiego, e il
    // censimento dei fallback conta la forma sintattica.
    match serde_json::from_str(stderr.trim()) {
        Ok(busta) => busta,
        Err(errore) => panic!("stderr non e' un solo documento JSON ({errore}): «{stderr}»"),
    }
}

/// Un millisecondo non basta a leggere tredici megabyte, su nessuna macchina.
///
/// L'affermazione non e' una scommessa sul tempo: e' un ordine di grandezza. La
/// sorgente supera i dodici MiB e la conversione la attraversa per intero in
/// build di debug; se un giorno esistesse una macchina che lo fa in meno di un
/// millisecondo, il test diventerebbe rosso e direbbe una cosa vera — che la
/// deadline non e' piu' stata superata — invece di mentire.
#[test]
fn una_deadline_di_un_millisecondo_ferma_la_conversione_prima_del_publish() {
    let (_radice, ingresso, uscita) = ambiente(RIGHE);

    let output = comando_convert(&ingresso, &uscita, Some("1"))
        .output()
        .expect("il binario si esegue");

    assert_eq!(
        output.status.code(),
        Some(EXIT_LIMITE),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let documento = busta(&String::from_utf8_lossy(&output.stderr));
    assert_eq!(documento["error"]["code"], "LIMIT_EXCEEDED");
    assert!(
        !uscita.exists(),
        "la destinazione non deve essere pubblicata"
    );
    assert!(
        std::fs::read_dir(uscita.parent().expect("uscita ha un padre"))
            .expect("directory leggibile")
            .next()
            .is_none(),
        "la directory di destinazione deve essere vuota: lo staging non e' stato ripulito"
    );
}

/// Il controllo: con una deadline generosa la stessa conversione arriva in fondo.
///
/// Senza questa sonda, un rosso della precedente non distinguerebbe «la deadline
/// ha funzionato» da «la conversione era rotta comunque».
#[test]
fn una_deadline_generosa_lascia_finire_la_stessa_conversione() {
    let (_radice, ingresso, uscita) = ambiente(RIGHE);

    let output = comando_convert(&ingresso, &uscita, Some("600000"))
        .output()
        .expect("il binario si esegue");

    assert!(
        output.status.success(),
        "convert fallito con deadline generosa.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let documento: serde_json::Value = serde_json::from_slice(&output.stdout).expect("busta JSON");
    assert_eq!(documento["total_rows"], RIGHE);
    assert!(uscita.is_file());
}

/// Zero non e' una deadline: e' il rifiuto della quota, non un default.
#[test]
fn una_deadline_a_zero_e_rifiutata() {
    let (_radice, ingresso, uscita) = ambiente(RIGHE);

    let output = comando_convert(&ingresso, &uscita, Some("0"))
        .output()
        .expect("il binario si esegue");

    assert_eq!(output.status.code(), Some(EXIT_LIMITE));
    assert_eq!(
        busta(&String::from_utf8_lossy(&output.stderr))["error"]["code"],
        "LIMIT_EXCEEDED"
    );
    assert!(!uscita.exists());
}

/// Un valore non intero e' un errore d'uso, non una deadline dedotta.
#[test]
fn una_deadline_non_intera_e_un_errore_d_uso() {
    let (_radice, ingresso, uscita) = ambiente(RIGHE);

    let output = comando_convert(&ingresso, &uscita, Some("presto"))
        .output()
        .expect("il binario si esegue");

    assert!(!output.status.success());
    let documento = busta(&String::from_utf8_lossy(&output.stderr));
    assert_eq!(documento["error"]["code"], "CLI_USAGE");
    assert!(!uscita.exists());
}

#[cfg(unix)]
mod segnale {
    use super::{ambiente, busta, comando_convert, EXIT_ANNULLATO, RIGHE_SEGNALE};
    use std::path::Path;
    use std::process::{Child, Command};

    /// Quanto si aspetta che lo staging compaia.
    ///
    /// Sessanta secondi, e non sei come nella prima stesura. Il numero non
    /// descrive quanto ci mette il figlio a creare lo staging -- sono
    /// millisecondi su una macchina ferma -- ma quanto puo' metterci su una
    /// **carica**: dentro lo stesso binario di test girano in parallelo altre
    /// quattro prove che convertono, e la corsa di livello 2 aggiunge la suite
    /// intera. La prima stesura e' passata in isolamento ed e' diventata rossa
    /// li', il che e' esattamente il difetto che un'attesa breve produce: una
    /// sonda che riporta il carico della macchina invece della proprieta' che
    /// dice di provare.
    ///
    /// Un'attesa lunga non rallenta nulla quando le cose funzionano: si esce al
    /// primo riscontro.
    const ATTESA_MASSIMA: std::time::Duration = std::time::Duration::from_secs(60);
    const INTERVALLO: std::time::Duration = std::time::Duration::from_millis(10);

    /// Che cosa e' successo mentre si aspettava.
    ///
    /// Tre esiti distinti, perche' due di loro sono difetti **diversi** e la
    /// prima stesura li confondeva in un `bool`: «lo staging non e' comparso»
    /// diceva la stessa cosa per un figlio lento e per un figlio morto, e nel
    /// secondo caso nascondeva l'unica informazione utile, cioe' l'errore con
    /// cui e' morto.
    enum Attesa {
        Comparso,
        FiglioUscito(std::process::Output),
        Scaduta,
    }

    /// Attende che il figlio abbia davvero cominciato a scrivere.
    ///
    /// Il segnale mandato prima che lo staging esista proverebbe un'altra cosa —
    /// che un processo appena nato muore — e la pulizia dello staging non
    /// sarebbe osservabile perche' non ci sarebbe niente da pulire. La
    /// condizione d'attesa e' percio' l'unica che rende la sonda quella che
    /// dice di essere: la directory di destinazione ha almeno una voce.
    fn attendi_lo_staging(directory: &Path, figlio: &mut Child) -> Attesa {
        let scadenza = std::time::Instant::now() + ATTESA_MASSIMA;
        while std::time::Instant::now() < scadenza {
            if std::fs::read_dir(directory)
                .expect("directory leggibile")
                .next()
                .is_some()
            {
                return Attesa::Comparso;
            }
            // Dopo la directory, non prima: un figlio che ha gia' creato lo
            // staging **e** e' gia' uscito va trattato come comparso, perche' la
            // corsa e' persa comunque e l'esito piu' informativo e' il primo.
            if figlio.try_wait().expect("stato del figlio").is_some() {
                let uscita = figlio.stdout.take();
                let errore = figlio.stderr.take();
                let mut output = std::process::Output {
                    status: figlio.wait().expect("il figlio termina"),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                };
                if let Some(mut flusso) = uscita {
                    use std::io::Read as _;
                    let _ = flusso.read_to_end(&mut output.stdout);
                }
                if let Some(mut flusso) = errore {
                    use std::io::Read as _;
                    let _ = flusso.read_to_end(&mut output.stderr);
                }
                return Attesa::FiglioUscito(output);
            }
            std::thread::sleep(INTERVALLO);
        }
        Attesa::Scaduta
    }

    #[test]
    fn sigint_annulla_la_conversione_e_non_lascia_staging() {
        let (_radice, ingresso, uscita) = ambiente(RIGHE_SEGNALE);
        let directory = uscita.parent().expect("uscita ha un padre").to_path_buf();

        // Le due pipe non sono un dettaglio: senza, `wait_with_output` torna
        // con `stderr` vuoto perche' il figlio ha ereditato quello del test, e
        // la busta d'errore -- che e' meta' di cio' che la sonda verifica --
        // non arriverebbe mai qui.
        let mut figlio = comando_convert(&ingresso, &uscita, Some("600000"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("il binario parte");

        match attendi_lo_staging(&directory, &mut figlio) {
            Attesa::Comparso => {}
            Attesa::FiglioUscito(output) => panic!(
                "il figlio e' uscito con {:?} prima di creare lo staging.\nstdout: {}\nstderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Attesa::Scaduta => panic!(
                "lo staging non e' comparso entro {ATTESA_MASSIMA:?} e il figlio e' ancora vivo: \
                 la conversione non ha raggiunto la creazione del writer"
            ),
        }

        // Il `kill` della shell invece di una chiamata alla libc: il workspace
        // vieta `unsafe`, e il builtin fa esattamente cio' che serve senza
        // pretendere che l'immagine abbia `/bin/kill` -- che l'immagine di
        // sviluppo, minimale, non ha.
        let esito = Command::new("sh")
            .arg("-c")
            .arg(format!("kill -INT {}", figlio.id()))
            .status()
            .expect("la shell si esegue");
        assert!(esito.success(), "kill -INT non riuscito");

        let output = figlio.wait_with_output().expect("il figlio termina");

        assert_eq!(
            output.status.code(),
            Some(EXIT_ANNULLATO),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(busta(&stderr)["error"]["code"], "CANCELLED");
        // La regressione. `busta` gia' pretende che l'intero flusso sia un
        // documento solo, ma il messaggio che darebbe direbbe «JSON non
        // valido», e chi legge cercherebbe un difetto nella serializzazione.
        // Questa riga dice invece che cosa e' successo davvero: qualcuno ha
        // scritto su `stderr` prima della busta. E' capitato, e la sonda che
        // avrebbe dovuto accorgersene era stata resa compatibile con la riga
        // di troppo invece di rifiutarla.
        assert!(
            stderr.trim_start().starts_with('{'),
            "prima della busta non ci va nient'altro, nemmeno una diagnostica: «{stderr}»"
        );
        assert!(!uscita.exists(), "la destinazione non deve esistere");
        assert!(
            std::fs::read_dir(&directory)
                .expect("directory leggibile")
                .next()
                .is_none(),
            "lo staging e' rimasto sul disco dopo la cancellazione"
        );
    }
}

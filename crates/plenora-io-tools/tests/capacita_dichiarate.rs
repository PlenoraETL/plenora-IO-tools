//! Che cosa il documento capability promette, e se il prodotto lo fa.
//!
//! # Perche' un file suo
//!
//! Le altre prove partono da un'invocazione e guardano il risultato. Queste
//! partono dal **descrittore** e cercano il comportamento: e' il verso opposto,
//! ed e' quello che conta per chi orchestra, perche' un orchestratore sceglie
//! leggendo il documento e non provando.
//!
//! # ARROW-011, e perche' ora si applica
//!
//! «An operation advertised with Arrow **stream** output MUST allow the
//! consumer to process batches without first materializing the complete
//! result, unless the operation descriptor explicitly declares bounded
//! materialization». Questa superficie annuncia lo stream, quindi il requisito
//! si applica -- e la seconda meta' della frase e' come lo soddisfa: il
//! descrittore dichiara `materialization: bounded`, che e' l'uscita prevista
//! per iscritto.
//!
//! Due cose restano distinte, e per un periodo le ho confuse.
//!
//! `application/vnd.apache.arrow.stream` e' una **serializzazione**: il
//! formato IPC a flusso si produce per intero e poi si consegna, come si fa
//! con un file. L'atomicita' dell'operazione -- che tutti e dieci i driver
//! hanno -- riguarda **quando** il primo batch diventa visibile, non quale
//! serializzazione lo trasporta. Produrre un flusso non ha quindi richiesto di
//! rinunciare all'atomicita', e non ci ha rinunciato: i byte si scrivono per
//! intero nello staging e la pubblicazione resta l'ultima operazione.
//!
//! Quello che segue dall'atomicita' e' un'altra cosa: la **consegna
//! incrementale** non c'e', e non e' promessa da nessuna parte.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("canoniche")
        .join(nome)
}

fn documento(argomenti: &[&str]) -> Value {
    let uscita = Command::new(BINARIO)
        .args(argomenti)
        .output()
        .expect("il binario si esegue");
    serde_json::from_slice(&uscita.stdout).expect("la busta e' JSON")
}

fn capacita() -> Value {
    documento(&["capabilities"])
}

fn operazione(documento: &Value, id: &str) -> Value {
    documento["result"]["operations"]
        .as_array()
        .expect("le operazioni sono un elenco")
        .iter()
        .find(|o| o["id"] == id)
        .unwrap_or_else(|| panic!("`{id}` non e' fra le operazioni dichiarate"))
        .clone()
}

/// Le due serializzazioni che il catalogo comune ammette sono annunciate.
///
/// Erano una, e il restringimento era registrato come deviazione. Questa sonda
/// diceva allora che il giorno in cui la superficie avesse annunciato anche lo
/// stream sarebbe stato il posto in cui accorgersene -- ed e' successo. Ora
/// dice il verso opposto, e serve alla stessa cosa: annunciare un content type
/// che non si produce e' peggio che non annunciarlo, perche' chi sceglie il
/// lettore su quel campo apre con lo strumento sbagliato.
///
/// Che i due siano **prodotti** davvero lo prova
/// `tests/serializzazione_arrow.rs`, sui byte consegnati.
#[test]
fn io_read_annuncia_le_due_serializzazioni() {
    let read = operazione(&capacita(), "io.read");
    let tipi: std::collections::BTreeSet<&str> = read["output"]["content_types"]
        .as_array()
        .expect("i content type sono un elenco")
        .iter()
        .map(|t| t.as_str().expect("ogni content type e' una stringa"))
        .collect();

    assert_eq!(
        tipi,
        [
            "application/vnd.apache.arrow.file",
            "application/vnd.apache.arrow.stream",
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>(),
        "i content type annunciati non sono i due del catalogo comune"
    );

    // `materialization: bounded` non e' piu' una nota sul perche' il requisito
    // non si applichi: e' la forma in cui ARROW-011 e' soddisfatto, ora che
    // l'antecedente e' vero. Resta diagnostica opaca per CAP-013 -- la
    // selezione si fa sui content type -- ma toglierla renderebbe il requisito
    // applicabile e non soddisfatto.
    assert_eq!(read["attributes"]["materialization"], "bounded");
    assert_eq!(read["attributes"]["delivery"], "operation_atomic");
}

/// E `io.write` accetta il payload in entrambe.
///
/// Un'operazione che producesse due serializzazioni e ne accettasse una sola
/// renderebbe non componibile il proprio stesso risultato.
#[test]
fn io_write_accetta_le_due_serializzazioni() {
    let write = operazione(&capacita(), "io.write");
    let tipi: std::collections::BTreeSet<&str> = write["input"]["content_types"]
        .as_array()
        .expect("i content type sono un elenco")
        .iter()
        .map(|t| t.as_str().expect("ogni content type e' una stringa"))
        .collect();

    for atteso in [
        "application/json",
        "application/vnd.apache.arrow.file",
        "application/vnd.apache.arrow.stream",
    ] {
        assert!(tipi.contains(atteso), "manca «{atteso}» fra {tipi:?}");
    }
}

/// E il file consegnato e' davvero un file IPC, non uno stream in un file.
///
/// Le due serializzazioni Arrow sono diverse: il file porta il magic `ARROW1`
/// in testa **e in coda**, lo stream no. Un consumatore che aprisse col lettore
/// sbagliato fallirebbe, e il content type annunciato e' cio' su cui sceglie il
/// lettore.
#[test]
fn il_consegnato_e_un_file_ipc_e_non_uno_stream() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let destinazione = temporanea.path().join("consegnato.arrow");
    let busta = documento(&[
        "read",
        fixture("canonico.geojson").to_str().expect("percorso"),
        "--output",
        destinazione.to_str().expect("percorso"),
    ]);
    assert_eq!(busta["status"], "ok", "la consegna riesce: {busta}");

    let byte = std::fs::read(&destinazione).expect("il consegnato si legge");
    assert!(
        byte.starts_with(b"ARROW1") && byte.ends_with(b"ARROW1"),
        "il magic del formato file manca a un capo: senza opzioni la consegna \
         e' il contenitore, che e' il default dichiarato dal catalogo"
    );
}

/// `delivery: operation_atomic` e' vero, e lo e' per tutti i driver.
///
/// Non e' la ragione per cui lo stream non e' annunciato -- quella e' che una
/// seconda serializzazione non esiste -- ma e' un'affermazione che il
/// descrittore fa, e un descrittore che affermasse il falso sarebbe peggio di
/// uno che tace. Se un driver diventasse a consegna incrementale, la
/// dichiarazione diverrebbe falsa in silenzio: qui e' legata al fatto.
#[test]
fn ogni_driver_raggiungibile_consegna_l_operazione_come_blocco() {
    let catalogo = documento(&["catalog"]);
    let driver = catalogo["result"]["drivers"]
        .as_array()
        .expect("i driver sono un elenco");

    let mut disponibili = 0_usize;
    for uno in driver {
        if uno["available"] != Value::Bool(true) {
            continue;
        }
        disponibili += 1;
        assert_eq!(
            uno["effective_delivery"], "operation_atomic",
            "«{}» consegna in un altro modo: l'argomento con cui `io.read` non \
             annuncia lo stream vale finche' vale per tutti",
            uno["id"]
        );
    }
    assert!(
        disponibili >= 8,
        "il catalogo deve dichiarare i driver disponibili, e ne ha {disponibili}"
    );
}

/// Le sei operazioni si servono da due superfici, e il documento lo dice.
///
/// Fino alla libreria pubblica `["cli"]` era vero. Ora `operazioni::*` rende le
/// stesse sei per nome: un documento che ne dichiarasse una sola direbbe meno
/// del vero proprio a chi decide come invocarci.
#[test]
fn ogni_operazione_dichiara_le_due_superfici() {
    let documento = capacita();
    for id in [
        "io.catalog",
        "io.inspect",
        "io.layers",
        "io.read",
        "io.write",
        "io.convert",
    ] {
        let superfici = operazione(&documento, id)["surfaces"].clone();
        assert_eq!(
            superfici,
            serde_json::json!(["cli", "rust"]),
            "`{id}`: le superfici dichiarate non sono quelle che esistono"
        );
    }
}

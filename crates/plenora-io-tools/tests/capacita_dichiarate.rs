//! Che cosa il documento capability promette, e se il prodotto lo fa.
//!
//! # Perche' un file suo
//!
//! Le altre prove partono da un'invocazione e guardano il risultato. Queste
//! partono dal **descrittore** e cercano il comportamento: e' il verso opposto,
//! ed e' quello che conta per chi orchestra, perche' un orchestratore sceglie
//! leggendo il documento e non provando.
//!
//! # ARROW-011, e perche' non e' chiuso da un'assenza
//!
//! «An operation advertised with Arrow **stream** output MUST allow the
//! consumer to process batches without first materializing the complete
//! result, unless the operation descriptor explicitly declares bounded
//! materialization». Il catalogo comune ammette per `io.read` sia
//! `arrow.stream` sia `arrow.file`; questa superficie annuncia il solo file.
//!
//! Finche' la ragione restava «non c'e' uno stream», era un'assenza, e
//! un'assenza non si prova guardandosi dentro. La ragione vera e' una scelta
//! misurabile: ogni driver raggiungibile da `io.read` e'
//! `DeliverySemantics::OperationAtomic`, cioe' rifiuta l'operazione come blocco
//! unico se una violazione emerge in un punto qualsiasi della sorgente.
//! Consegna incrementale e atomicita' dell'operazione si escludono, e
//! annunciare uno stream prometterebbe la prima mentre il prodotto sceglie la
//! seconda.

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

/// ARROW-011: la capacita' annunciata e' il file, e non lo stream.
#[test]
fn io_read_annuncia_il_file_e_non_lo_stream() {
    let read = operazione(&capacita(), "io.read");
    let tipi: Vec<&str> = read["output"]["content_types"]
        .as_array()
        .expect("i content type sono un elenco")
        .iter()
        .map(|t| t.as_str().expect("ogni content type e' una stringa"))
        .collect();

    assert_eq!(
        tipi,
        ["application/vnd.apache.arrow.file"],
        "ARROW-011 si applica a chi annuncia lo stream. Aggiungerlo qui senza \
         consegna incrementale renderebbe il requisito applicabile e non \
         soddisfatto"
    );

    // La dichiarazione esiste ed e' esplicita, cosi' che la scelta si legga nel
    // documento invece di doversi dedurre da un'assenza. Resta diagnostica
    // opaca per CAP-013: la selezione si fa sui content type.
    assert_eq!(read["attributes"]["materialization"], "bounded");
    assert_eq!(read["attributes"]["delivery"], "operation_atomic");
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
        "il magic del formato file manca a un capo: la consegna non e' il \
         `application/vnd.apache.arrow.file` che il descrittore annuncia"
    );
}

/// La scelta che rende falso l'antecedente di ARROW-011, misurata sui driver.
///
/// Se un driver diventasse a consegna incrementale, l'argomento con cui `io.read`
/// non annuncia lo stream cadrebbe -- e cadrebbe in silenzio, perche' il
/// descrittore non cambierebbe da solo. Qui la ragione e' legata al fatto.
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

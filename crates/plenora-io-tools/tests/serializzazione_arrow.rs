//! Le due serializzazioni di Arrow IPC, provate dal confine pubblico.
//!
//! # Che cosa c'era prima, e perche' non bastava
//!
//! Il catalogo comune ammette per `io.read` due content type d'uscita,
//! `application/vnd.apache.arrow.stream` e `application/vnd.apache.arrow.file`,
//! e questa superficie produceva il solo file. Era registrato come deviazione,
//! con una conseguenza pesante: i quattro archi `direct` della matrice di
//! composizione nominano il flusso, e un arco `direct` esige che sorgente e
//! destinazione condividano contratto d'interscambio **e** content type.
//!
//! # Che cosa queste sonde pretendono
//!
//! Che il percorso sia intero. Produrre un flusso non serve a niente se chi lo
//! riceve non puo' consumarlo: qui si prova la produzione, il riconoscimento e
//! la lettura, e l'ultima con la libreria che userebbe un consumatore
//! qualunque -- `arrow_ipc::reader::StreamReader` -- non con la nostra.
//!
//! # Che cosa **non** cambia, ed e' la parte che va difesa
//!
//! La consegna resta atomica sull'operazione. Il flusso descrive come i byte
//! sono disposti, non quando diventano visibili: si scrivono per intero nello
//! staging e la pubblicazione e' l'ultima operazione. Nessuna di queste sonde
//! deve poter osservare un batch prima che l'operazione sia finita, e una di
//! esse lo verifica sul caso in cui l'operazione **fallisce**.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;

use arrow_array::Array;
use arrow_ipc::reader::{FileReader, StreamReader};
use serde_json::Value;

const BINARIO: &str = env!("CARGO_BIN_EXE_plenora-io");

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("canoniche")
        .join(nome)
}

fn esegui(argomenti: &[&str]) -> Value {
    let uscita = Command::new(BINARIO)
        .args(argomenti)
        .output()
        .expect("il binario si esegue");
    serde_json::from_slice(&uscita.stdout).expect("la busta e' JSON")
}

/// Legge il file con la libreria Arrow, scegliendo il lettore **dai byte**.
///
/// E' quello che farebbe un consumatore qualunque: non ha la nostra
/// prevalidazione, non ha i nostri driver, ha `arrow-ipc` e il file.
fn batch_di_un_consumatore(
    percorso: &Path,
) -> (Vec<arrow_array::RecordBatch>, arrow_schema::SchemaRef) {
    let byte = std::fs::read(percorso).expect("il consegnato si legge");
    if byte.starts_with(b"ARROW1") {
        let lettore = FileReader::try_new(File::open(percorso).expect("si apre"), None)
            .expect("e' un file Arrow IPC valido");
        let schema = lettore.schema();
        (
            lettore.map(|b| b.expect("batch leggibile")).collect(),
            schema,
        )
    } else {
        let lettore =
            StreamReader::try_new(BufReader::new(File::open(percorso).expect("si apre")), None)
                .expect("e' un flusso Arrow IPC valido");
        let schema = lettore.schema();
        (
            lettore.map(|b| b.expect("batch leggibile")).collect(),
            schema,
        )
    }
}

/// Consegna la fixture nelle due serializzazioni, e rende i due percorsi.
fn le_due_consegne(dove: &Path, sorgente: &Path) -> (PathBuf, PathBuf) {
    let contenitore = dove.join("contenitore.arrow");
    let flusso = dove.join("flusso.arrows");
    for (destinazione, opzioni) in [
        (&contenitore, vec![]),
        (&flusso, vec!["--out-opt", "serialization=stream"]),
    ] {
        let mut argomenti = vec![
            "read",
            sorgente.to_str().expect("percorso"),
            "--output",
            destinazione.to_str().expect("percorso"),
        ];
        argomenti.extend(opzioni);
        let busta = esegui(&argomenti);
        assert_eq!(busta["status"], "ok", "consegna fallita: {busta}");
    }
    (contenitore, flusso)
}

/// I dati sono gli stessi: stesse righe, stessi valori, stesso ordine.
#[test]
fn il_flusso_porta_gli_stessi_dati_del_contenitore() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let (contenitore, flusso) = le_due_consegne(temporanea.path(), &fixture("canonico.geojson"));

    let (dal_file, _) = batch_di_un_consumatore(&contenitore);
    let (dal_flusso, _) = batch_di_un_consumatore(&flusso);

    let righe_file: usize = dal_file
        .iter()
        .map(arrow_array::RecordBatch::num_rows)
        .sum();
    let righe_flusso: usize = dal_flusso
        .iter()
        .map(arrow_array::RecordBatch::num_rows)
        .sum();
    assert!(righe_file > 0, "la fixture deve avere righe");
    assert_eq!(righe_file, righe_flusso, "il numero di righe differisce");

    // Valore per valore, non solo il conteggio: due file con lo stesso numero
    // di righe e contenuto diverso passerebbero un confronto sui conteggi.
    for (indice, (a, b)) in dal_file.iter().zip(dal_flusso.iter()).enumerate() {
        assert_eq!(
            a.num_columns(),
            b.num_columns(),
            "batch {indice}: colonne diverse"
        );
        for colonna in 0..a.num_columns() {
            assert_eq!(
                a.column(colonna).to_data(),
                b.column(colonna).to_data(),
                "batch {indice}, colonna {colonna}: i dati differiscono"
            );
        }
    }
}

/// Lo schema e i metadati del contratto sopravvivono al flusso.
///
/// `plenora.contract.version` sullo schema, l'estensione `GeoArrow` e lo stato
/// del CRS sulla colonna geometrica: sono cio' che rende un payload
/// interpretabile da un altro componente, e una serializzazione che li perdesse
/// produrrebbe byte leggibili e significato assente.
#[test]
fn lo_schema_e_i_metadati_sopravvivono_al_flusso() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let (contenitore, flusso) = le_due_consegne(temporanea.path(), &fixture("canonico.geojson"));

    let (_, schema_file) = batch_di_un_consumatore(&contenitore);
    let (_, schema_flusso) = batch_di_un_consumatore(&flusso);

    assert_eq!(
        schema_file.metadata(),
        schema_flusso.metadata(),
        "i metadati di schema differiscono fra le due serializzazioni"
    );
    assert!(
        schema_file
            .metadata()
            .contains_key("plenora.contract.version"),
        "ARROW-001: la versione del contratto deve esserci, altrimenti il \
         confronto sopra sarebbe fra due assenze"
    );

    assert_eq!(
        schema_file.fields().len(),
        schema_flusso.fields().len(),
        "il numero di campi differisce"
    );
    for (uno, altro) in schema_file.fields().iter().zip(schema_flusso.fields()) {
        assert_eq!(uno.name(), altro.name(), "i nomi dei campi differiscono");
        assert_eq!(
            uno.data_type(),
            altro.data_type(),
            "«{}»: il tipo differisce",
            uno.name()
        );
        assert_eq!(
            uno.is_nullable(),
            altro.is_nullable(),
            "«{}»: la nullabilita' differisce",
            uno.name()
        );
        assert_eq!(
            uno.metadata(),
            altro.metadata(),
            "«{}»: i metadati del campo differiscono",
            uno.name()
        );
    }

    // La colonna geometrica porta l'estensione e il CRS: senza questa riga il
    // confronto sopra passerebbe anche se entrambe le serializzazioni li
    // perdessero insieme.
    let geometrica = schema_file
        .fields()
        .iter()
        .find(|campo| campo.metadata().contains_key("ARROW:extension:name"))
        .expect("una colonna geometrica con l'estensione dichiarata");
    assert_eq!(
        geometrica
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("geoarrow.wkb")
    );
    assert!(
        geometrica
            .metadata()
            .contains_key("plenora.geometry.crs_resolution"),
        "lo stato del CRS viaggia con la colonna"
    );
}

/// ARROW-003: ogni campo porta la propria identita', e le due serializzazioni
/// portano la stessa.
#[test]
fn l_identita_dei_campi_sopravvive_al_flusso() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let (contenitore, flusso) = le_due_consegne(temporanea.path(), &fixture("canonico.geojson"));

    let identita = |percorso: &Path| -> Vec<String> {
        let (_, schema) = batch_di_un_consumatore(percorso);
        schema
            .fields()
            .iter()
            .map(|campo| {
                campo
                    .metadata()
                    .get("plenora.field_id")
                    .cloned()
                    .unwrap_or_else(|| panic!("«{}» non porta `plenora.field_id`", campo.name()))
            })
            .collect()
    };

    let dal_file = identita(&contenitore);
    let dal_flusso = identita(&flusso);
    assert_eq!(dal_file, dal_flusso, "le identita' differiscono");

    let distinti: std::collections::BTreeSet<&String> = dal_flusso.iter().collect();
    assert_eq!(
        distinti.len(),
        dal_flusso.len(),
        "ARROW-VOCABULARY §2: due campi portano la stessa identita' -- {dal_flusso:?}"
    );
}

/// Un dataset vuoto si serializza come flusso, e si rilegge come tale.
///
/// E' il caso in cui un formato a flusso puo' degenerare: zero batch, e il file
/// e' il solo messaggio di schema piu' il marcatore di fine. Un lettore che
/// pretendesse almeno un batch lo rifiuterebbe, e la sorgente vuota e' proprio
/// quella su cui B12c ha gia' insegnato che le assenze vanno distinte.
#[test]
fn un_dataset_vuoto_si_serializza_come_flusso() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let vuota = temporanea.path().join("vuota.geojson");
    std::fs::write(&vuota, br#"{"type":"FeatureCollection","features":[]}"#)
        .expect("la sorgente si scrive");
    let flusso = temporanea.path().join("vuoto.arrows");

    let busta = esegui(&[
        "read",
        vuota.to_str().expect("percorso"),
        "--output",
        flusso.to_str().expect("percorso"),
        "--out-opt",
        "serialization=stream",
    ]);
    assert_eq!(busta["status"], "ok", "{busta}");

    let (batch, schema) = batch_di_un_consumatore(&flusso);
    let righe: usize = batch.iter().map(arrow_array::RecordBatch::num_rows).sum();
    assert_eq!(righe, 0, "non sono comparse righe dal nulla");
    assert!(
        !schema.fields().is_empty(),
        "lo schema resta intero anche senza righe: e' la distinzione fra zero \
         righe e schema ignoto"
    );
}

/// Il riconoscimento guarda i byte, non il nome.
///
/// Un flusso chiamato `.arrow` e un contenitore chiamato `.arrows` si leggono
/// entrambi. E' la stessa regola di D10 -- il nome del file e' del chiamante --
/// e qui ha un costo concreto: senza, un file rinominato diventerebbe
/// illeggibile per una ragione che non ha niente a che fare con i suoi dati.
#[test]
fn il_riconoscimento_guarda_i_byte_non_il_nome() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let (contenitore, flusso) = le_due_consegne(temporanea.path(), &fixture("canonico.geojson"));

    // Scambiati: il contenitore prende il nome del flusso e viceversa.
    let contenitore_travestito = temporanea.path().join("travestito.arrows");
    let flusso_travestito = temporanea.path().join("travestito.arrow");
    std::fs::copy(&contenitore, &contenitore_travestito).expect("copia");
    std::fs::copy(&flusso, &flusso_travestito).expect("copia");

    for percorso in [&contenitore_travestito, &flusso_travestito] {
        let busta = esegui(&["inspect", percorso.to_str().expect("percorso")]);
        assert_eq!(
            busta["status"],
            "ok",
            "«{}» non si legge: il nome ha deciso al posto dei byte -- {busta}",
            percorso.display()
        );
    }
}

/// Il content type dichiarato e' quello dei byte consegnati.
///
/// Due letture della stessa opzione -- una nel driver che sceglie il writer,
/// una nella CLI che riferisce -- possono divergere, e divergerebbero in
/// silenzio: chi sceglie il lettore sul content type aprirebbe con lo
/// strumento sbagliato, che fra le due serializzazioni e' un errore di
/// apertura.
#[test]
fn il_content_type_segue_la_serializzazione() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let sorgente = fixture("canonico.geojson");

    for (nome, opzioni, atteso, magic) in [
        (
            "con-footer.arrow",
            vec![],
            "application/vnd.apache.arrow.file",
            true,
        ),
        (
            "a-flusso.arrows",
            vec!["--out-opt", "serialization=stream"],
            "application/vnd.apache.arrow.stream",
            false,
        ),
    ] {
        let destinazione = temporanea.path().join(nome);
        let mut argomenti = vec![
            "read",
            sorgente.to_str().expect("percorso"),
            "--output",
            destinazione.to_str().expect("percorso"),
        ];
        argomenti.extend(opzioni);
        let busta = esegui(&argomenti);

        assert_eq!(
            busta["result"]["delivered"]["content_type"], atteso,
            "{nome}: il content type dichiarato non e' quello atteso"
        );
        let byte = std::fs::read(&destinazione).expect("il consegnato si legge");
        assert_eq!(
            byte.starts_with(b"ARROW1"),
            magic,
            "{nome}: i byte non corrispondono al content type dichiarato"
        );
    }
}

/// `io.write` legge il payload nelle due serializzazioni, e dice quale ha letto.
#[test]
fn la_scrittura_accetta_entrambe_le_serializzazioni() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let (contenitore, flusso) = le_due_consegne(temporanea.path(), &fixture("canonico.geojson"));

    for (sorgente, atteso) in [
        (&contenitore, "application/vnd.apache.arrow.file"),
        (&flusso, "application/vnd.apache.arrow.stream"),
    ] {
        let destinazione = temporanea.path().join(format!(
            "{}.csv",
            sorgente.file_stem().expect("stem").to_string_lossy()
        ));
        let busta = esegui(&[
            "write",
            sorgente.to_str().expect("percorso"),
            destinazione.to_str().expect("percorso"),
            "--to",
            "csv",
        ]);
        assert_eq!(busta["status"], "ok", "{busta}");
        assert_eq!(
            busta["result"]["input"]["content_type"], atteso,
            "il payload letto non e' quello dichiarato"
        );
        assert!(
            busta["result"]["rows_written"]
                .as_u64()
                .is_some_and(|n| n > 0),
            "nessuna riga scritta: la lettura del payload non ha prodotto dati"
        );
    }
}

/// Un flusso troncato e' rifiutato, e il rifiuto e' tipizzato.
///
/// La prevalidazione del contenitore esisteva e quella del flusso e' nuova: una
/// porta meno sorvegliata e' il modo normale in cui una difesa si aggira. Qui
/// si taglia un flusso valido a meta' di un messaggio e si pretende un errore
/// della categoria giusta, non un panico ne' un successo parziale.
#[test]
fn un_flusso_troncato_e_rifiutato() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let (_, flusso) = le_due_consegne(temporanea.path(), &fixture("canonico.geojson"));

    let byte = std::fs::read(&flusso).expect("il flusso si legge");
    assert!(
        byte.len() > 64,
        "serve un flusso abbastanza lungo da tagliare"
    );
    let tagliato = temporanea.path().join("tagliato.arrows");
    std::fs::write(&tagliato, &byte[..byte.len() - 16]).expect("il troncato si scrive");

    let busta = esegui(&["inspect", tagliato.to_str().expect("percorso")]);
    assert_eq!(
        busta["status"], "error",
        "un flusso troncato non e' leggibile"
    );
    assert!(
        busta["error"]["category"].is_string(),
        "il rifiuto porta i quattro assi: {busta}"
    );
}

/// La consegna resta atomica: un'operazione fallita non lascia una destinazione.
///
/// E' la proprieta' che produrre un flusso **non** deve intaccare. Un writer a
/// flusso che scrivesse direttamente sulla destinazione la lascerebbe a meta'
/// quando l'operazione fallisce a lettura iniziata, ed e' esattamente cio' che
/// lo staging impedisce: i byte vanno in un file temporaneo e la pubblicazione
/// e' l'ultima operazione.
#[test]
fn la_consegna_a_flusso_resta_atomica() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let destinazione = temporanea.path().join("mai-pubblicato.arrows");

    let busta = esegui(&[
        "read",
        "/nessuna-sorgente-per-questa-sonda.geojson",
        "--output",
        destinazione.to_str().expect("percorso"),
        "--out-opt",
        "serialization=stream",
    ]);
    assert_eq!(busta["status"], "error", "{busta}");
    assert!(
        !destinazione.exists(),
        "un'operazione fallita non lascia una destinazione: la consegna a \
         flusso non e' meno atomica di quella a file"
    );
}

/// Un valore non ammesso per l'opzione e' respinto, e non vale il default.
#[test]
fn una_serializzazione_sconosciuta_e_respinta() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let destinazione = temporanea.path().join("mai.arrow");

    let busta = esegui(&[
        "read",
        fixture("canonico.geojson").to_str().expect("percorso"),
        "--output",
        destinazione.to_str().expect("percorso"),
        "--out-opt",
        "serialization=flusso-inventato",
    ]);
    assert_eq!(
        busta["status"], "error",
        "un valore fuori dall'enumerazione non deve valere il default: \
         scrivere un contenitore a chi ha chiesto altro e' peggio di un \
         rifiuto -- {busta}"
    );
    assert!(!destinazione.exists(), "e non lascia una destinazione");
}

// ---------------------------------------------------------------------------
// Le protezioni di `valida_flusso_ipc`, una prova negativa ciascuna.
//
// # Perche' esistono
//
// La prevalidazione filtra l'input **prima** che arrivi alla libreria Arrow, e
// la qualifica della 4.0.0 ha misurato che i suoi rami di rifiuto erano gli
// unici del file mai eseguiti dai test: dodici righe cambiate e scoperte, tutte
// dentro il validatore di flusso aggiunto in quel ciclo.
//
// Cio' che mancava non erano le protezioni -- esistono e rifiutano -- ma
// qualcosa che le tenesse ferme: una modifica futura poteva toglierne una senza
// che un test diventisse rosso. La copertura misurata non le vedeva perche'
// `cargo llvm-cov` gira sui test e non sui fuzzer; il fuzz ne esercita una sola
// per caso, i trentasei input del corpus di `ipc_reader` sotto gli otto byte.
//
// # Perche' si confronta il messaggio
//
// Perche' «rifiutato» non basta: un input malformato puo' essere rifiutato dal
// ramo sbagliato e la prova resterebbe verde mentre la protezione che pretende
// di pinnare e' sparita. Il messaggio dice **quale** guardia ha parlato.

/// Il prefisso di continuazione dell'incapsulamento corrente.
const CONTINUAZIONE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

/// Un'intestazione di messaggio: prefisso di continuazione e lunghezza dei metadati.
fn intestazione(lunghezza_metadati: u32) -> Vec<u8> {
    let mut byte = CONTINUAZIONE.to_vec();
    byte.extend_from_slice(&lunghezza_metadati.to_le_bytes());
    byte
}

/// Scrive i byte come flusso e restituisce la busta di `inspect`.
fn rifiuto_del_flusso(byte: &[u8]) -> (Value, tempfile::TempDir) {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let percorso = temporanea.path().join("ostile.arrows");
    std::fs::write(&percorso, byte).expect("l'input si scrive");
    let busta = esegui(&["inspect", percorso.to_str().expect("percorso")]);
    (busta, temporanea)
}

/// Il rifiuto e' tipizzato e viene dalla guardia attesa.
fn pretendi_rifiuto(byte: &[u8], frammento: &str) {
    let (busta, _temporanea) = rifiuto_del_flusso(byte);
    assert_eq!(
        busta["status"], "error",
        "l'input ostile e' accettato: {busta}"
    );
    // Niente `unwrap_or_default()` qui: un messaggio assente diventerebbe la
    // stringa vuota, e l'asserzione successiva fallirebbe lamentando il
    // frammento mancante invece del messaggio mancante -- due guasti diversi
    // con la stessa diagnosi. Meglio dire quale dei due e'.
    let Some(messaggio) = busta["error"]["message"].as_str() else {
        panic!("il rifiuto non porta un messaggio: {busta}");
    };
    assert!(
        messaggio.contains(frammento),
        "rifiutato dalla guardia sbagliata: atteso «{frammento}», ottenuto «{messaggio}»"
    );
    assert!(
        busta["error"]["category"].is_string(),
        "il rifiuto porta i quattro assi: {busta}"
    );
}

/// Sotto gli otto byte non c'e' nemmeno un'intestazione da leggere.
#[test]
fn un_flusso_sotto_gli_otto_byte_e_rifiutato() {
    pretendi_rifiuto(&[0x01, 0x02, 0x03], "troppo corto");
}

/// La lunghezza dichiarata dei metadati ha un tetto, e oltre quello si rifiuta.
///
/// Il tetto serve prima che la lunghezza diventi un'allocazione: senza, un
/// campo di quattro byte deciderebbe quanta memoria chiedere.
#[test]
fn i_metadati_oltre_il_tetto_sono_rifiutati() {
    const OLTRE_IL_TETTO: u32 = 16 * 1024 * 1024 + 1;
    pretendi_rifiuto(&intestazione(OLTRE_IL_TETTO), "fuori dai limiti");
}

/// Un messaggio che dichiara piu' byte di quanti il flusso ne contenga.
///
/// E' il caso in cui la lunghezza e' plausibile ma il flusso finisce prima: si
/// rifiuta sul confronto con la dimensione, non leggendo oltre la fine.
#[test]
fn un_messaggio_oltre_la_fine_del_flusso_e_rifiutato() {
    let mut byte = intestazione(4096);
    byte.extend_from_slice(&[0_u8; 16]);
    pretendi_rifiuto(&byte, "oltre la fine del flusso");
}

/// Un flusso che finisce senza aver mai portato uno schema.
///
/// La lunghezza zero chiude il flusso: e' terminato correttamente, e non ha
/// detto che cosa contenesse. Accettarlo significherebbe consegnare un dataset
/// senza schema a chi ne ha chiesto uno.
#[test]
fn un_flusso_senza_schema_e_rifiutato() {
    pretendi_rifiuto(&intestazione(0), "senza messaggio di schema");
}

/// Una coda piu' corta di un'intestazione non si legge come messaggio.
///
/// Il ramo e' un'**uscita** dal ciclo, non un rifiuto: i byte spaiati in fondo
/// si ignorano invece di leggerli come lunghezza. Il rifiuto arriva dopo, dallo
/// schema mai visto, ed e' quello che questa sonda pretende -- se comparisse un
/// messaggio diverso vorrebbe dire che la coda e' stata interpretata.
#[test]
fn una_coda_piu_corta_di_un_intestazione_non_si_interpreta() {
    let mut byte = intestazione(0);
    byte.extend_from_slice(&[0x01, 0x02, 0x03]);
    pretendi_rifiuto(&byte, "senza messaggio di schema");
}

// Il tetto sui messaggi -- `MAX_BLOCCHI`, 1048576 -- **non** ha una prova qui,
// ed e' una scelta dichiarata invece che una dimenticanza: raggiungerlo chiede
// piu' di un milione di messaggi decodificabili, cioe' un flusso costruito
// apposta di oltre otto megabyte, che questa suite dovrebbe generare a ogni
// corsa per esercitare un fondo di sicurezza. Che sia cosi' difficile da
// raggiungere e' anche cio' che lo rende un fondo: su quasi ogni input
// malformato scatta prima una delle guardie qui sopra.

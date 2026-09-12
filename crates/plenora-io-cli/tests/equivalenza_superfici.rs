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
//! # Che cosa la costruzione non garantisce
//!
//! Condividere la funzione centrale riduce le divergenze; non le esclude.
//! Restano gli **adattamenti** che ciascun binding introduce, e sono
//! precisamente ciò che queste prove misurano:
//!
//! * il binding di processo aggiunge i campi d'identità, sceglie il flusso su
//!   cui scrivere e proietta la categoria in un codice d'uscita — tre cose che
//!   la funzione condivisa non fa e su cui può sbagliare da sola;
//! * il binding in-process traduce una `Richiesta` a campi nominati negli
//!   argomenti che la funzione attende. Un binding che leggesse la destinazione
//!   dal posizionale sbagliato passerebbe ogni prova interna e renderebbe due
//!   risultati diversi dalle due porte.
//!
//! Per questo le sonde non si fermano a confrontare i risultati: esercitano
//! **ogni campo** della `Richiesta` contro il flag corrispondente, e verificano
//! gli adattamenti del processo uno per uno.

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

/// Un campo della richiesta e il flag che gli corrisponde, con l'esito atteso.
struct Caso {
    nome: &'static str,
    richiesta: Richiesta,
    argomenti: Vec<String>,
}

/// Ogni campo della `Richiesta` significa quello che significa il suo flag.
///
/// # Perché campo per campo e non un caso solo
///
/// La traduzione fra `Richiesta` e argomenti è scritta a mano: ogni campo ha
/// una riga, e una riga sbagliata sposta un valore su un altro flag senza che
/// niente lo dica. Un caso solo -- «la sorgente e la destinazione» -- ne
/// coprirebbe due e lascerebbe scoperti gli altri dieci.
///
/// Ciascun caso qui sotto esercita **un** campo e lo confronta col flag che gli
/// corrisponde. Il risultato dev'essere identico: se un campo finisse altrove,
/// o si perdesse, i due documenti divergerebbero.
#[test]
fn ogni_campo_della_richiesta_corrisponde_al_suo_flag() {
    let geojson = fixture("canonico.geojson");
    let gpkg = fixture("canonico.gpkg");
    let csv = fixture("canonico.csv");

    let casi = vec![
        Caso {
            nome: "layer",
            // Layer 0 su una sorgente multi-layer: senza il campo la lettura
            // sarebbe ambigua e verrebbe rifiutata, quindi il caso esercita il
            // campo anche se il valore e' zero.
            richiesta: Richiesta::sulla_sorgente(&gpkg).con_layer(0),
            argomenti: vec![
                "read".to_owned(),
                gpkg.display().to_string(),
                "--layer".to_owned(),
                "0".to_owned(),
            ],
        },
        Caso {
            nome: "limit",
            richiesta: Richiesta::sulla_sorgente(&geojson).con_limite(2),
            argomenti: vec![
                "read".to_owned(),
                geojson.display().to_string(),
                "--limit".to_owned(),
                "2".to_owned(),
            ],
        },
        Caso {
            nome: "assume_crs con opzione di lettura",
            richiesta: Richiesta::sulla_sorgente(&csv)
                .con_crs_assunto("EPSG:4326")
                .con_opzione_di_lettura("wkt_column", "geometry"),
            argomenti: vec![
                "read".to_owned(),
                csv.display().to_string(),
                "--assume-crs".to_owned(),
                "EPSG:4326".to_owned(),
                "--in-opt".to_owned(),
                "wkt_column=geometry".to_owned(),
            ],
        },
        Caso {
            nome: "opzione comune",
            richiesta: Richiesta::sulla_sorgente(&csv)
                .con_crs_assunto("EPSG:4326")
                .con_opzione("wkt_column", "geometry"),
            argomenti: vec![
                "read".to_owned(),
                csv.display().to_string(),
                "--assume-crs".to_owned(),
                "EPSG:4326".to_owned(),
                "--opt".to_owned(),
                "wkt_column=geometry".to_owned(),
            ],
        },
    ];

    for caso in casi {
        let dalla_libreria = operazioni::read(caso.richiesta).expect(caso.nome);
        let dal_processo = dalla_cli(&caso.argomenti).expect(caso.nome);
        assert_eq!(
            dal_processo, dalla_libreria,
            "«{}»: il campo non arriva dove arriva il flag",
            caso.nome
        );
    }
}

/// Gli stessi campi, per le operazioni che chiedono una destinazione.
///
/// Stanno in una prova a parte perché ognuna deve scrivere in un posto suo: una
/// destinazione riusata sarebbe un conflitto di scrittura, non un confronto.
#[test]
fn ogni_campo_con_destinazione_corrisponde_al_suo_flag() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let geojson = fixture("canonico.geojson");

    // I campi che richiedono una destinazione, con destinazioni distinte: una
    // destinazione occupata sarebbe un conflitto, non un confronto.
    //
    // `durable` da solo sul sink Arrow, che non accetta opzioni di scrittura --
    // e la prova me l'ha ricordato: «ipc: opzione sconosciuta in 'compression'».
    let dalla_libreria = operazioni::read(
        Richiesta::sulla_sorgente(&geojson)
            .con_destinazione(temporanea.path().join("durevole-lib.arrow"))
            .durevole(),
    )
    .expect("durable");
    let dal_processo = dalla_cli(&[
        "read".to_owned(),
        geojson.display().to_string(),
        "--output".to_owned(),
        temporanea
            .path()
            .join("durevole-cli.arrow")
            .display()
            .to_string(),
        "--durable".to_owned(),
    ])
    .expect("durable");
    assert_eq!(
        dal_processo, dalla_libreria,
        "«durable» non arriva dove arriva il flag"
    );

    // `output_options` dove le opzioni di scrittura esistono: il CSV accetta
    // `delimiter`, e il sink Arrow non accetta niente.
    let arrow = temporanea.path().join("ponte-campi.arrow");
    operazioni::read(Richiesta::sulla_sorgente(&geojson).con_destinazione(&arrow))
        .expect("la consegna che alimenta il caso");

    let dalla_libreria = operazioni::write(
        Richiesta::sulla_sorgente(&arrow)
            .con_destinazione(temporanea.path().join("opzioni-lib.csv"))
            .con_formato_destinazione("csv")
            .con_opzione_di_scrittura("delimiter", ";"),
    )
    .expect("output_options");
    let dal_processo = dalla_cli(&[
        "write".to_owned(),
        arrow.display().to_string(),
        temporanea
            .path()
            .join("opzioni-cli.csv")
            .display()
            .to_string(),
        "--to".to_owned(),
        "csv".to_owned(),
        "--out-opt".to_owned(),
        "delimiter=;".to_owned(),
    ])
    .expect("output_options");
    assert_eq!(
        dal_processo, dalla_libreria,
        "«output_options» non arriva dove arriva il flag"
    );

    // E l'opzione ha davvero avuto effetto: senza questa riga il caso sopra
    // passerebbe anche se entrambe le porte la ignorassero allo stesso modo.
    let scritto = std::fs::read_to_string(temporanea.path().join("opzioni-lib.csv"))
        .expect("il CSV si legge");
    assert!(
        scritto
            .lines()
            .next()
            .is_some_and(|riga| riga.contains(';')),
        "il delimitatore chiesto è quello scritto: {}",
        &scritto[..scritto.len().min(80)]
    );

    // `source_format` e `target_format` di `convert`, che sono gli unici campi
    // senza un equivalente in `read`.
    let dalla_libreria = operazioni::convert(
        Richiesta::sulla_sorgente(&geojson)
            .con_destinazione(temporanea.path().join("formati-lib.csv"))
            .con_formato_sorgente("geojson")
            .con_formato_destinazione("csv"),
    )
    .expect("i due formati");
    let dal_processo = dalla_cli(&[
        "convert".to_owned(),
        geojson.display().to_string(),
        temporanea
            .path()
            .join("formati-cli.csv")
            .display()
            .to_string(),
        "--from".to_owned(),
        "geojson".to_owned(),
        "--to".to_owned(),
        "csv".to_owned(),
    ])
    .expect("i due formati");
    assert_eq!(
        dal_processo, dalla_libreria,
        "«source_format» e «target_format» non arrivano dove arrivano i flag"
    );
}

/// Gli adattamenti che il binding di processo introduce, uno per uno.
///
/// # Che cosa aggiunge, e perché va verificato separatamente
///
/// La funzione condivisa rende un documento. Il processo ci mette intorno tre
/// cose che la funzione non fa: i campi d'**identità**, la scelta del **flusso**
/// e la **proiezione** della categoria in un codice d'uscita. Nessuna delle tre
/// è coperta dal confronto fra i due documenti, perché il confronto le toglie
/// per poterlo fare.
#[test]
fn gli_adattamenti_del_binding_di_processo_sono_quelli_dichiarati() {
    let sorgente = fixture("canonico.geojson");

    // 1. L'identità, su un successo.
    let uscita = Command::new(BINARIO)
        .args(["read", sorgente.to_str().unwrap(), "--format", "json"])
        .output()
        .expect("il binario parte");
    let stdout = String::from_utf8(uscita.stdout).expect("stdout e' UTF-8");
    let busta: Value = serde_json::from_str(&stdout).expect("stdout e' JSON");

    assert_eq!(busta["component"], "plenora-io-tools");
    assert_eq!(busta["command"], "read");
    assert_eq!(busta["contract"], "plenora-io-read-result-v1");
    assert_eq!(busta["protocol_version"], 2);
    assert!(
        busta["component_version"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "la versione dell'artefatto è dichiarata"
    );
    assert!(
        uscita.stderr.is_empty(),
        "CLI-2.0 §4: in modo macchina stderr resta vuoto"
    );
    assert_eq!(uscita.status.code(), Some(0), "un successo esce 0");

    // 2. Il flusso e la proiezione, su un errore. La busta d'errore esce su
    //    **stdout**: un consumatore che legge stdout -- cioè quello che il
    //    contratto descrive -- deve vedere il fallimento.
    let uscita = Command::new(BINARIO)
        .args([
            "read",
            "/nessun-file-per-gli-adattamenti.geojson",
            "--format",
            "json",
        ])
        .output()
        .expect("il binario parte");
    let stdout = String::from_utf8(uscita.stdout).expect("stdout e' UTF-8");
    let busta: Value = serde_json::from_str(&stdout).expect("l'errore esce su stdout ed è JSON");
    assert_eq!(busta["status"], "error");
    assert!(
        uscita.stderr.is_empty(),
        "anche sull'errore stderr resta vuoto"
    );
    assert_eq!(
        busta["contract"], "plenora-error-v1",
        "il contratto d'errore è quello comune"
    );

    // La proiezione: il codice d'uscita è quello della **categoria**, ed è la
    // sola cosa che la libreria non rende -- perché riguarda un processo che
    // chi la usa in-process non ha avviato.
    let categoria = busta["error"]["category"].as_str().expect("categoria");
    let atteso = match categoria {
        "invalid_configuration" => 2,
        "schema" | "data_mapping" | "crs" | "unsupported" => 3,
        "resource_limit" => 4,
        "io" | "conflict" | "not_found" | "permission" => 5,
        "execution" | "invalid_plan" => 6,
        "cancelled" => 130,
        _ => 70,
    };
    assert_eq!(
        uscita.status.code(),
        Some(atteso),
        "categoria «{categoria}»: la proiezione del codice d'uscita è del binding"
    );
}

/// La tabella dei casi, uno riuscito e uno fallito per ciascuna delle cinque
/// operazioni con ingresso.
///
/// Sta fuori dalla prova perché è un dato, non una verifica: tenerla dentro
/// rendeva illeggibile il confronto, che è la parte che conta.
fn casi_nei_due_esiti(
    dove: &Path,
    geojson: &Path,
    arrow: &Path,
    inesistente: &'static str,
) -> Vec<(&'static str, bool, Richiesta, Vec<String>)> {
    vec![
        (
            "io.inspect",
            true,
            Richiesta::sulla_sorgente(geojson),
            vec!["inspect".to_owned(), geojson.display().to_string()],
        ),
        (
            "io.inspect",
            false,
            Richiesta::sulla_sorgente(inesistente),
            vec!["inspect".to_owned(), inesistente.to_owned()],
        ),
        (
            "io.layers",
            true,
            Richiesta::sulla_sorgente(geojson),
            vec!["layers".to_owned(), geojson.display().to_string()],
        ),
        (
            "io.layers",
            false,
            Richiesta::sulla_sorgente(inesistente),
            vec!["layers".to_owned(), inesistente.to_owned()],
        ),
        (
            "io.read",
            true,
            Richiesta::sulla_sorgente(geojson),
            vec!["read".to_owned(), geojson.display().to_string()],
        ),
        (
            "io.read",
            false,
            Richiesta::sulla_sorgente(inesistente),
            vec!["read".to_owned(), inesistente.to_owned()],
        ),
        (
            "io.write",
            true,
            Richiesta::sulla_sorgente(arrow)
                .con_destinazione(dove.join("w-lib.csv"))
                .con_formato_destinazione("csv"),
            vec![
                "write".to_owned(),
                arrow.display().to_string(),
                dove.join("w-cli.csv").display().to_string(),
                "--to".to_owned(),
                "csv".to_owned(),
            ],
        ),
        (
            "io.write",
            false,
            Richiesta::sulla_sorgente(arrow)
                .con_destinazione(dove.join("mai-lib.dat"))
                .con_formato_destinazione("formato-che-non-esiste"),
            vec![
                "write".to_owned(),
                arrow.display().to_string(),
                dove.join("mai-cli.dat").display().to_string(),
                "--to".to_owned(),
                "formato-che-non-esiste".to_owned(),
            ],
        ),
        (
            "io.convert",
            true,
            Richiesta::sulla_sorgente(geojson)
                .con_destinazione(dove.join("c-lib.csv"))
                .con_formato_sorgente("geojson")
                .con_formato_destinazione("csv"),
            vec![
                "convert".to_owned(),
                geojson.display().to_string(),
                dove.join("c-cli.csv").display().to_string(),
                "--from".to_owned(),
                "geojson".to_owned(),
                "--to".to_owned(),
                "csv".to_owned(),
            ],
        ),
        (
            "io.convert",
            false,
            Richiesta::sulla_sorgente(inesistente)
                .con_destinazione(dove.join("mai-conv-lib.csv"))
                .con_formato_sorgente("geojson")
                .con_formato_destinazione("csv"),
            vec![
                "convert".to_owned(),
                inesistente.to_owned(),
                dove.join("mai-conv-cli.csv").display().to_string(),
                "--from".to_owned(),
                "geojson".to_owned(),
                "--to".to_owned(),
                "csv".to_owned(),
            ],
        ),
    ]
}

/// Successo ed errore per **ognuna** delle sei operazioni, dalle due porte.
///
/// Le sonde precedenti coprivano le operazioni una per volta e con un caso
/// ciascuna. Qui la tabella è unica e ogni operazione compare due volte -- un
/// caso che riesce e uno che fallisce -- perché un'equivalenza provata sui soli
/// successi lascerebbe scoperta la metà in cui i due binding fanno di più:
/// l'errore passa da `local_err_doc`, dalla proiezione e dal flusso.
#[test]
fn successo_ed_errore_di_ogni_operazione_coincidono() {
    let temporanea = tempfile::tempdir().expect("directory temporanea");
    let dove = temporanea.path();
    let geojson = fixture("canonico.geojson");
    let arrow = dove.join("ponte.arrow");
    operazioni::read(Richiesta::sulla_sorgente(&geojson).con_destinazione(&arrow))
        .expect("la consegna che alimenta la prova");

    let inesistente = "/nessun-file-per-le-sei-operazioni.geojson";
    let mut coperte = std::collections::BTreeSet::new();

    // Le cinque operazioni con ingresso, nei due esiti.
    let casi = casi_nei_due_esiti(dove, &geojson, &arrow, inesistente);

    for (operazione, deve_riuscire, richiesta, argomenti) in casi {
        let dalla_libreria = match argomenti[0].as_str() {
            "inspect" => operazioni::inspect(richiesta),
            "layers" => operazioni::layers(richiesta),
            "read" => operazioni::read(richiesta),
            "write" => operazioni::write(richiesta),
            _ => operazioni::convert(richiesta),
        };
        let dal_processo = dalla_cli(&argomenti);

        assert_eq!(
            dalla_libreria.is_ok(),
            deve_riuscire,
            "{operazione}: la libreria non ha l'esito atteso"
        );
        assert_eq!(
            dal_processo.is_ok(),
            deve_riuscire,
            "{operazione}: il processo non ha l'esito atteso"
        );

        match (dalla_libreria, dal_processo) {
            (Ok(libreria), Ok(processo)) => assert_eq!(
                processo, libreria,
                "{operazione}: il successo differisce fra le due porte"
            ),
            (Err(libreria), Err(processo)) => {
                assert_eq!(
                    senza_identita(&processo),
                    senza_identita(&libreria),
                    "{operazione}: l'errore differisce fra le due porte"
                );
                // La semantica dell'errore, asse per asse: è ciò su cui una
                // macchina decide, e confrontare i documenti interi la
                // proverebbe solo per coincidenza.
                for asse in ["category", "phase", "remote_effect", "retry", "code"] {
                    assert_eq!(
                        processo["error"][asse], libreria["error"][asse],
                        "{operazione}: l'asse `{asse}` differisce"
                    );
                }
            }
            _ => unreachable!("gli esiti sono già stati confrontati"),
        }
        coperte.insert(operazione);
    }

    // `io.catalog` non ha ingresso e non può fallire: si confronta da sola.
    assert_eq!(
        dalla_cli(&["catalog".to_owned()]).expect("il catalogo riesce"),
        operazioni::catalog()
    );
    coperte.insert("io.catalog");

    assert_eq!(
        coperte.len(),
        6,
        "le sei operazioni del catalogo sono coperte tutte: {coperte:?}"
    );
}

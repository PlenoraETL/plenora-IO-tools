//! Le sonde delle parti private della superficie runtime: la composizione
//! dei tetti, la scadenza, il nome dell'artefatto risolto, gli spazi ECMA.

use std::path::PathBuf;
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde_json::json;

use super::{
    istante_della_scadenza, millisecondi_rimasti, nome_coerente, spazio_ecma, tetti, Campi,
};
use plenora_io_model::budget::PipelineLimits;

fn campi(valore: serde_json::Value) -> Campi {
    serde_json::from_value(valore).expect("i campi si leggono")
}

/// Una scadenza sola governa la durata: l'assoluta (con il margine che la
/// lascia al token), oppure `deadline_ms`, oppure il default. Le due insieme
/// non arrivano qui: le rifiuta l'ammissione.
#[test]
fn la_durata_segue_una_scadenza_sola() {
    let predefinita = PipelineLimits::default().duration_ms();
    assert_eq!(tetti(&campi(json!({})), None).duration_ms(), predefinita);
    assert_eq!(
        tetti(&campi(json!({})), Some(1_000)).duration_ms(),
        1_000 + super::MARGINE_DELLA_DURATA_MS
    );
    assert_eq!(
        tetti(&campi(json!({})), Some(predefinita + 5_000)).duration_ms(),
        predefinita + 5_000 + super::MARGINE_DELLA_DURATA_MS,
        "la scadenza assoluta non e' tagliata dal default"
    );
    assert_eq!(
        tetti(&campi(json!({"deadline_ms": 90_000})), None).duration_ms(),
        90_000
    );
    assert_eq!(
        tetti(&campi(json!({})), Some(u64::MAX)).duration_ms(),
        u64::MAX,
        "nessun trabocco"
    );
}

/// Ogni quota del payload finisce nel campo omonimo, e in nessun altro.
#[test]
fn ogni_budget_va_nel_suo_campo() {
    let limiti = tetti(
        &campi(json!({"budgets": {
            "memory_bytes": 101,
            "max_rows": 102,
            "max_columns": 103,
            "max_input_bytes": 104,
            "max_input_entries": 105,
            "max_output_bytes": 106,
            "max_vertices": 107,
            "max_wkb_cell_bytes": 108,
            "max_wkb_components": 109,
            "max_wkb_depth": 110,
        }})),
        None,
    );
    assert_eq!(limiti.memory_bytes(), 101);
    assert_eq!(limiti.max_rows(), 102);
    assert_eq!(limiti.max_columns(), 103);
    assert_eq!(limiti.max_input_bytes(), 104);
    assert_eq!(limiti.max_input_entries(), 105);
    assert_eq!(limiti.max_output_bytes(), 106);
    assert_eq!(limiti.max_vertices(), 107);
    assert_eq!(limiti.max_wkb_cell_bytes(), 108);
    assert_eq!(limiti.max_wkb_components(), 109);
    assert_eq!(limiti.max_wkb_depth(), 110);
    // Una quota assente resta quella del modello.
    let soli = tetti(&campi(json!({"budgets": {"max_rows": 7}})), None);
    let predefiniti = PipelineLimits::default();
    assert_eq!(soli.max_rows(), 7);
    assert_eq!(soli.memory_bytes(), predefiniti.memory_bytes());
    assert_eq!(soli.max_columns(), predefiniti.max_columns());
}

#[test]
fn i_millisecondi_rimasti_arrotondano_per_difetto() {
    assert!(millisecondi_rimasti(Instant::now()).is_err());
    let lontano = Instant::now() + Duration::from_secs(60);
    let rimasti = millisecondi_rimasti(lontano).expect("ne restano");
    assert!(rimasti <= 60_000 && rimasti > 59_000, "{rimasti}");
}

#[test]
fn una_durata_non_rappresentabile_non_diventa_il_massimo() {
    use super::millisecondi_della_durata;
    let massimo = Duration::from_millis(u64::MAX);
    assert_eq!(
        millisecondi_della_durata(massimo).expect("limite"),
        u64::MAX
    );
    let errore = millisecondi_della_durata(massimo + Duration::from_millis(1))
        .expect_err("nessuna saturazione");
    assert_eq!(errore["code"], "RUNTIME_DEADLINE_INVALID");
    assert_eq!(errore["category"], "invalid_configuration");
    assert_eq!(errore["phase"], "validate");
    assert_eq!(errore["remote_effect"], "none");
    assert_eq!(errore["retry"]["kind"], "never");
    assert!(!errore.to_string().contains(&u64::MAX.to_string()));
    for durata in [Duration::ZERO, Duration::from_nanos(999_999)] {
        assert_eq!(
            millisecondi_della_durata(durata).expect_err("scaduta")["category"],
            "timeout"
        );
    }
}

#[test]
fn l_orologio_prima_dell_epoca_non_allenta_la_scadenza() {
    let prima = UNIX_EPOCH - Duration::from_secs(1);
    let errore = istante_della_scadenza("2030-01-01T00:00:00Z", prima)
        .expect_err("un orologio prima dell'epoca non si legge");
    assert_eq!(errore["code"], "RUNTIME_CLOCK_INVALID");
}

#[test]
fn una_scadenza_passata_e_quella_della_pipeline() {
    let adesso = UNIX_EPOCH + Duration::from_hours(497_544);
    for passata in [
        "2026-10-04T23:59:59Z",
        "2026-10-05T00:00:00Z",
        "1969-12-31T23:59:59Z",
    ] {
        let errore = istante_della_scadenza(passata, adesso).expect_err(passata);
        assert_eq!(errore["code"], "DEADLINE_EXCEEDED", "{passata}");
        assert_eq!(errore["category"], "timeout", "{passata}");
        assert_eq!(errore["phase"], "validate", "{passata}");
        assert_eq!(errore["retry"]["kind"], "never", "{passata}");
    }
    assert!(istante_della_scadenza("2026-10-05T00:00:01Z", adesso).is_ok());
    let invalida = istante_della_scadenza("1969-02-30T00:00:00Z", adesso).expect_err("data");
    assert_eq!(invalida["code"], "RUNTIME_DEADLINE_INVALID");
}

#[test]
fn il_nome_risolto_e_quello_del_riferimento() {
    let base = PathBuf::from("deposito");
    let casi = [
        ("artifact://input/strade", "strade.geojson", true),
        ("artifact://input/strade.geojson", "strade.geojson", true),
        ("s3://bucket/dati/strade.gpkg", "strade.gpkg", true),
        ("urn:plenora:strade", "strade.csv", true),
        ("artifact://input/strade.shp.d", "strade.shp.d", true),
        ("artifact://input/strade", "tmp-81f3a2.geojson", false),
        ("artifact://input/strade", "Strade.geojson", false),
        ("artifact://input/", "strade.geojson", false),
    ];
    for (riferimento, file, atteso) in casi {
        let esito = nome_coerente(riferimento, base.join(file));
        assert_eq!(esito.is_ok(), atteso, "{riferimento} -> {file}");
        if let Err(errore) = esito {
            assert_eq!(errore["code"], "RUNTIME_ARTIFACT_NAME_MISMATCH");
            assert!(
                !errore.to_string().contains(file),
                "il nome non entra nel messaggio"
            );
        }
    }
}

#[test]
fn gli_spazi_sono_quelli_di_ecma() {
    for spazio in [
        '\u{0020}', '\u{0009}', '\u{00A0}', '\u{2007}', '\u{FEFF}', '\u{3000}',
    ] {
        assert!(spazio_ecma(spazio), "{spazio:?}");
    }
    // U+0085 e' `White_Space` per Unicode ma non `\s` per ECMA-262.
    for non_spazio in ['\u{0085}', 'a', '/', '\u{200B}'] {
        assert!(!spazio_ecma(non_spazio), "{non_spazio:?}");
    }
}

#[test]
fn interruzione_dopo_la_produzione_del_file_rimuove_la_directory() {
    for scadenza in [false, true] {
        let privata = tempfile::tempdir().expect("directory privata");
        let percorso = privata.path().to_path_buf();
        let sorgente = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/canoniche/canonico.geojson");
        let rapporto = crate::operazioni::read(
            crate::operazioni::Richiesta::sulla_sorgente(sorgente)
                .con_destinazione(percorso.join("consegna.arrows"))
                .con_opzione_di_scrittura("serialization", "stream"),
        )
        .expect("file Arrow prodotto prima dell'interruzione");
        assert!(percorso.join("consegna.arrows").is_file());
        let token = super::CancellationToken::new();
        if scadenza {
            token.cancel_due_to_deadline();
        } else {
            token.cancel();
        }
        let errore = super::completa_flusso(privata, Ok(rapporto), &token)
            .expect_err("nessun successo dopo l'interruzione");
        assert_eq!(
            errore["category"],
            if scadenza { "timeout" } else { "cancelled" }
        );
        assert_eq!(
            errore["code"],
            if scadenza {
                "DEADLINE_EXCEEDED"
            } else {
                "CANCELLED"
            }
        );
        assert_eq!(errore["phase"], "read");
        assert_eq!(errore["remote_effect"], "none");
        assert_eq!(errore["retry"]["kind"], "never");
        assert!(!percorso.exists(), "cleanup anche con token interrotto");
    }
}

#[test]
fn interruzione_durante_un_blocco_o_eof_non_restituisce_byte() {
    struct Lettore {
        token: super::CancellationToken,
        scadenza: bool,
        letti: usize,
    }
    impl std::io::Read for Lettore {
        fn read(&mut self, byte: &mut [u8]) -> std::io::Result<usize> {
            if self.scadenza {
                self.token.cancel_due_to_deadline();
            } else {
                self.token.cancel();
            }
            byte[..self.letti].fill(42);
            Ok(self.letti)
        }
    }
    for scadenza in [false, true] {
        for letti in [0, 64 * 1024] {
            let token = super::CancellationToken::new();
            let errore = super::leggi_flusso(
                Lettore {
                    token: token.clone(),
                    scadenza,
                    letti,
                },
                &token,
            )
            .expect_err("interruzione osservata anche dopo EOF");
            assert_eq!(
                errore["category"],
                if scadenza { "timeout" } else { "cancelled" }
            );
        }
    }
}

#[test]
fn interruzione_alla_restituzione_non_rende_un_carico_gia_completo() {
    for scadenza in [false, true] {
        let token = super::CancellationToken::new();
        let byte = super::leggi_flusso(std::io::Cursor::new([42; 8]), &token)
            .expect("lettura terminata prima dell'interruzione");
        if scadenza {
            token.cancel_due_to_deadline();
        } else {
            token.cancel();
        }
        for carico in [
            super::Carico::Json(json!({})),
            super::Carico::FlussoArrow {
                byte,
                rapporto: json!({}),
            },
        ] {
            let errore = super::finalizza_carico(carico, &token).expect_err("controllo finale");
            assert_eq!(
                errore["category"],
                if scadenza { "timeout" } else { "cancelled" }
            );
            assert_eq!(errore["phase"], "finalize");
            assert_eq!(errore["remote_effect"], "none");
            assert_eq!(errore["retry"]["kind"], "never");
        }
    }
}

// Gancio locale al thread, consumato una sola volta e rimosso anche su panic.
// Non modifica token o richieste del binding: muove soltanto l'orologio
// condiviso col token pubblico, oppure cancella il token del chiamante.
type Gancio = (&'static str, Box<dyn FnOnce()>);
thread_local! {
    static GANCIO: std::cell::RefCell<Option<Gancio>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn al_passaggio(passo: &str) {
    let azione = GANCIO.with(|g| {
        let mut g = g.borrow_mut();
        if g.as_ref().is_some_and(|(atteso, _)| *atteso == passo) {
            g.take()
        } else {
            None
        }
    });
    if let Some((_, azione)) = azione {
        azione();
    }
}

struct RipristinaGancio;
impl Drop for RipristinaGancio {
    fn drop(&mut self) {
        GANCIO.with(|g| g.borrow_mut().take());
    }
}

struct ArtefattiDiProva {
    sorgente: PathBuf,
    destinazione: PathBuf,
}
impl super::RisolutoreArtefatti for ArtefattiDiProva {
    fn sorgente(&self, _: &str) -> Result<PathBuf, super::RifiutoArtefatto> {
        Ok(self.sorgente.clone())
    }
    fn destinazione(&self, _: &str) -> Result<PathBuf, super::RifiutoArtefatto> {
        Ok(self.destinazione.clone())
    }
}

fn invocazione(operazione: &str) -> super::Invocazione {
    let descrittore = super::OPERAZIONI
        .iter()
        .find(|d| d.operazione == operazione)
        .expect("operazione");
    let mut payload = json!({"source": "artifact://input/canonico", "deadline_ms": 60_000});
    if operazione == "io.write" || operazione == "io.convert" {
        payload["destination"] = json!("artifact://output/uscita.geojson");
        payload["format"] = json!("geojson");
    }
    if operazione == "io.convert" {
        payload.as_object_mut().expect("oggetto").remove("format");
        payload["source_format"] = json!("geojson");
        payload["target_format"] = json!("geojson");
    }
    serde_json::from_value(json!({
        "content_type": "application/json",
        "metadata": {
            "plenora.message.id": "018f3d84-7b2c-7f00-8000-0000000000a1",
            "plenora.capability.name": super::NOME_CAPACITA,
            "plenora.capability.version": "1",
            "plenora.capability.operation": operazione,
            "plenora.operation.version": descrittore.versione.to_string(),
            "plenora.input.contract": descrittore.contratto_ingresso,
            "plenora.trace.correlation_id": "018f3d84-7b2c-7f00-8000-0000000000a2"
        },
        "payload": payload
    }))
    .expect("invocazione")
}

fn prova_interruzione_binding(operazione: &str, passo: &'static str, scadenza: bool) {
    let deposito = tempfile::tempdir().expect("deposito");
    let consegne = tempfile::tempdir().expect("consegne private");
    let sorgente =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/canoniche/canonico.geojson");
    let sorgente = if operazione == "io.write" {
        let arrow = deposito.path().join("canonico.arrow");
        crate::operazioni::read(
            crate::operazioni::Richiesta::sulla_sorgente(sorgente).con_destinazione(&arrow),
        )
        .expect("dataset per write");
        arrow
    } else {
        sorgente
    };
    let artefatti = ArtefattiDiProva {
        sorgente,
        destinazione: deposito.path().join("uscita.geojson"),
    };
    let binding = super::BindingRuntime::new(&artefatti).con_directory_temporanea(consegne.path());
    let richiesta = invocazione(operazione);
    // Controllo positivo attraverso lo stesso entry point, senza interruzioni.
    assert!(
        !binding
            .invoca(&richiesta, super::CancellationToken::new())
            .e_un_errore(),
        "{operazione}"
    );
    if artefatti.destinazione.exists() {
        std::fs::remove_file(&artefatti.destinazione).expect("nuova destinazione");
    }
    let orologio = plenora_io_model::PipelineClock::manual(Instant::now());
    let token = super::CancellationToken::with_clock(orologio.clone());
    let chiamante = token.clone();
    let directory = consegne.path().to_path_buf();
    GANCIO.with(|g| {
        *g.borrow_mut() = Some((
            passo,
            Box::new(move || {
                assert!(
                    !chiamante.is_cancelled(),
                    "interruzione solo dopo la pipeline"
                );
                let private: Vec<_> = std::fs::read_dir(directory).expect("directory").collect();
                if passo == "finalizzazione" {
                    assert!(private.is_empty(), "cleanup gia' completato");
                } else {
                    assert_eq!(private.len(), 1);
                    assert!(private[0]
                        .as_ref()
                        .expect("privata")
                        .path()
                        .join("consegna.arrows")
                        .is_file());
                }
                if scadenza {
                    orologio.advance(Duration::from_secs(60)).expect("scade");
                } else {
                    chiamante.cancel();
                }
            }),
        ));
    });
    let _ripristina = RipristinaGancio;
    let risultato = binding.invoca(&richiesta, token);
    GANCIO.with(|g| assert!(g.borrow().is_none(), "gancio raggiunto"));
    verifica_interruzione(risultato, operazione, passo, scadenza);
    assert_eq!(
        std::fs::read_dir(consegne.path()).expect("cleanup").count(),
        0
    );
}

fn verifica_interruzione(
    risultato: super::Risultato,
    operazione: &str,
    passo: &str,
    scadenza: bool,
) {
    assert!(risultato.e_un_errore(), "{operazione}: {passo}");
    assert_eq!(risultato.metadata.contratto_uscita, super::CONTRATTO_ERRORE);
    assert_eq!(
        risultato.metadata.id_causa.as_deref(),
        Some("018f3d84-7b2c-7f00-8000-0000000000a1")
    );
    assert_eq!(
        risultato.metadata.id_correlazione.as_deref(),
        Some("018f3d84-7b2c-7f00-8000-0000000000a2")
    );
    let super::Carico::Json(errore) = risultato.payload else {
        panic!("nessun byte Arrow")
    };
    assert_eq!(
        errore["code"],
        if scadenza {
            "DEADLINE_EXCEEDED"
        } else {
            "CANCELLED"
        }
    );
    assert_eq!(
        errore["category"],
        if scadenza { "timeout" } else { "cancelled" }
    );
    assert_eq!(
        errore["phase"],
        if passo == "finalizzazione" {
            "finalize"
        } else {
            "read"
        }
    );
    assert_eq!(errore["remote_effect"], "none");
    assert_eq!(errore["retry"]["kind"], "never");
}

#[test]
fn binding_interruzione_dopo_pipeline_e_durante_consegna() {
    for passo in ["dopo_pipeline", "durante_lettura"] {
        for scadenza in [false, true] {
            prova_interruzione_binding("io.read", passo, scadenza);
        }
    }
}

#[test]
fn binding_interruzione_finale_delle_cinque_operazioni() {
    for operazione in [
        "io.inspect",
        "io.layers",
        "io.read",
        "io.write",
        "io.convert",
    ] {
        for scadenza in [false, true] {
            prova_interruzione_binding(operazione, "finalizzazione", scadenza);
        }
    }
}

#[test]
fn metadati_ignoti_non_nascondono_duplicati() {
    let originale = invocazione("io.catalog");
    let testo = serde_json::to_string(&originale).unwrap();
    let errore_atteso = super::Invocazione::da_json(b"{").unwrap_err();
    for membri in [
        r#""segreto":1,"segreto":2,"#,
        r#""segreto":{"privato":1,"privato":2},"#,
        r#""segreto":[{"privato":1,"privato":2}],"#,
    ] {
        let alterato = testo.replace("\"metadata\":{", &format!("\"metadata\":{{{membri}"));
        assert_ne!(alterato, testo);
        assert_eq!(
            super::Invocazione::da_json(alterato.as_bytes()).unwrap_err(),
            errore_atteso
        );
    }
    let valido = testo.replace(
        "\"metadata\":{",
        r#""metadata":{"segreto":[{"privato":1}],"#,
    );
    assert_eq!(
        super::Invocazione::da_json(valido.as_bytes()).unwrap(),
        originale
    );
}

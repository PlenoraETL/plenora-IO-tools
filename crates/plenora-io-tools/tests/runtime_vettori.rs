//! La superficie runtime contro i vettori `io-read-*` del pin e contro i
//! propri contratti.
//!
//! # Da dove vengono i documenti
//!
//! Vettori, schemi, registro dei binding e catalogo sono copie byte per byte
//! del checkout fissato, in `contracts/copie-dal-pin/`: il job
//! `profilo-pubblico` le confronta con l'originale (`check_copie_dal_pin.py`).
//!
//! # Che cosa provano i vettori, e che cosa no
//!
//! RUNTIME-VECTORS-1.0 §4 assegna al componente di dominio la prova di
//! selettore, invocazione e risultato/errore attraverso il proprio adattatore
//! runtime, e §5 dice che i payload delle fixture sono illustrativi e che il
//! componente li valida con il **proprio** schema. Il payload di
//! `io-read-request.json` non e' un `plenora-io-read-input-v1` valido
//! (`artifact_source` e `format` non esistono nello schema, `layer` e' un
//! intero): la prova lo esegue cosi' com'e' e pretende il rifiuto tipizzato,
//! poi esegue gli stessi metadati con un payload del nostro schema che usa lo
//! stesso riferimento. Le differenze fra le fixture e il prodotto sono
//! dichiarate nel manifesto di adozione, e le prove le fissano: se una delle
//! due parti cambia, la prova diventa rossa e la deviazione va riscritta.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use plenora_io_model::CancellationToken;
use plenora_io_tools::operazioni::{self, Richiesta};
use plenora_io_tools::runtime::{
    campi_ammessi, capacita, istante_rfc3339_utc, riferimento_opaco, verifica_instradamento,
    BindingRuntime, Carico, Instradamento, Invocazione, IstanteRfc3339, RifiutoArtefatto,
    RisolutoreArtefatti, Risultato, CONTENT_TYPE_ERRORE, CONTENT_TYPE_FLUSSO_ARROW, OPERAZIONI,
};
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn radice() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("la radice del workspace sta due livelli sopra il crate")
        .to_path_buf()
}

fn documento(percorso: &Path) -> Value {
    let testo = std::fs::read_to_string(percorso)
        .unwrap_or_else(|e| panic!("{} si legge: {e}", percorso.display()));
    serde_json::from_str(&testo).unwrap_or_else(|e| panic!("{} e' JSON: {e}", percorso.display()))
}

fn copia(nome: &str) -> Value {
    documento(&radice().join("contracts").join("copie-dal-pin").join(nome))
}

fn schema_nostro(nome: &str) -> jsonschema::Validator {
    let schema = documento(
        &radice()
            .join("contracts")
            .join("schemas")
            .join(format!("{nome}.schema.json")),
    );
    jsonschema::validator_for(&schema).unwrap_or_else(|e| panic!("{nome}: {e}"))
}

fn schema_comune(nome: &str) -> jsonschema::Validator {
    jsonschema::validator_for(&copia(nome)).unwrap_or_else(|e| panic!("{nome}: {e}"))
}

fn fixture(nome: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("canoniche")
        .join(nome)
}

/// Il 2026-10-05T00:00:00Z: l'orologio delle prove, perche' l'esito di una
/// scadenza assoluta -- quella del vettore e' il 2030-01-01 -- non dipenda dal
/// giorno in cui la suite gira.
fn orologio_fisso() -> SystemTime {
    UNIX_EPOCH + Duration::from_hours(497_544)
}

/// Un orologio oltre la scadenza del vettore.
fn orologio_del_2031() -> SystemTime {
    UNIX_EPOCH + Duration::from_hours(534_720)
}

fn binding(deposito: &Deposito) -> BindingRuntime<'_> {
    BindingRuntime::new(deposito).con_orologio(orologio_fisso)
}

/// Il risolutore di prova: `artifact://input/<nome>` e `artifact://output/<nome>`.
///
/// Conta le chiamate: un'invocazione che deve fallire prima di toccare gli
/// artefatti lo dimostra lasciando i contatori a zero.
struct Deposito {
    ingressi: Vec<(&'static str, PathBuf)>,
    uscite: PathBuf,
    chiamate: Cell<usize>,
    /// Quanto il risolutore impiega a materializzare una sorgente.
    attesa: Duration,
}

impl Deposito {
    fn nuovo(uscite: &Path) -> Self {
        let materializza = |riferimento: &'static str, fixture_nome: &str, nome: &str| {
            let destinazione = uscite.join(nome);
            std::fs::copy(fixture(fixture_nome), &destinazione).expect("la fixture si copia");
            (riferimento, destinazione)
        };
        let ingressi = vec![
            // Il riferimento del vettore, materializzato col suffisso del suo
            // formato: `io.read` riconosce il formato, come sulla CLI.
            materializza(
                "artifact://input/io-read-vector",
                "canonico.gpkg",
                "io-read-vector.gpkg",
            ),
            materializza(
                "artifact://input/canonico-geojson",
                "canonico.geojson",
                "canonico-geojson.geojson",
            ),
            materializza(
                "artifact://input/canonico-arrow",
                "canonico.arrow",
                "canonico-arrow.arrow",
            ),
        ];
        Self {
            ingressi,
            uscite: uscite.to_path_buf(),
            chiamate: Cell::new(0),
            attesa: Duration::ZERO,
        }
    }

    fn percorso(&self, riferimento: &str) -> PathBuf {
        self.ingressi
            .iter()
            .find(|(r, _)| *r == riferimento)
            .map(|(_, p)| p.clone())
            .expect("riferimento noto")
    }
}

impl RisolutoreArtefatti for Deposito {
    fn sorgente(&self, riferimento: &str) -> Result<PathBuf, RifiutoArtefatto> {
        self.chiamate.set(self.chiamate.get() + 1);
        std::thread::sleep(self.attesa);
        self.ingressi
            .iter()
            .find(|(r, _)| *r == riferimento)
            .map(|(_, p)| p.clone())
            .ok_or(RifiutoArtefatto::NonTrovato)
    }

    fn destinazione(&self, riferimento: &str) -> Result<PathBuf, RifiutoArtefatto> {
        self.chiamate.set(self.chiamate.get() + 1);
        let nome = riferimento
            .strip_prefix("artifact://output/")
            .ok_or(RifiutoArtefatto::NonAutorizzato)?;
        Ok(self.uscite.join(nome))
    }
}

/// L'invocazione con i metadati di un vettore; `Value::Null` tiene il suo payload.
fn invocazione(richiesta: &Value, payload: Value) -> Invocazione {
    let payload = if payload.is_null() {
        richiesta["payload"].clone()
    } else {
        payload
    };
    let mut documento = serde_json::Map::new();
    documento.insert("content_type".to_owned(), richiesta["content_type"].clone());
    documento.insert("metadata".to_owned(), richiesta["metadata"].clone());
    documento.insert("payload".to_owned(), payload);
    serde_json::from_value(Value::Object(documento)).expect("l'invocazione si legge")
}

fn per_operazione(operazione: &str, payload: Value) -> Invocazione {
    let descrittore = OPERAZIONI
        .iter()
        .find(|d| d.operazione == operazione)
        .expect("operazione nota");
    serde_json::from_value(json!({
        "content_type": "application/json",
        "metadata": {
            "plenora.message.id": "018f3d84-7b2c-7f00-8000-0000000000a1",
            "plenora.capability.name": "plenora.io-tools",
            "plenora.capability.version": "1",
            "plenora.capability.operation": operazione,
            "plenora.operation.version": "1",
            "plenora.input.contract": descrittore.contratto_ingresso,
            "plenora.trace.correlation_id": "018f3d84-7b2c-7f00-8000-0000000000a2",
        },
        "payload": {},
    }))
    .map(|mut invocazione: Invocazione| {
        invocazione.payload = payload;
        invocazione
    })
    .expect("l'invocazione si legge")
}

fn errore(risultato: &Risultato) -> &Value {
    assert_eq!(risultato.content_type, CONTENT_TYPE_ERRORE, "{risultato:?}");
    assert_eq!(risultato.metadata.contratto_uscita, "plenora-error-v1");
    match &risultato.payload {
        Carico::Json(documento) => {
            let errori: Vec<String> = schema_comune("error-v1.schema.json")
                .iter_errors(documento)
                .map(|e| e.to_string())
                .collect();
            assert!(
                errori.is_empty(),
                "l'errore valida contro error-v1: {errori:?}"
            );
            documento
        }
        Carico::FlussoArrow { .. } => panic!("un errore e' JSON"),
    }
}

fn successo_json(risultato: &Risultato) -> &Value {
    assert_eq!(risultato.content_type, "application/json", "{risultato:?}");
    match &risultato.payload {
        Carico::Json(documento) => documento,
        Carico::FlussoArrow { .. } => panic!("atteso JSON"),
    }
}

fn vettore(nome: &str) -> Value {
    let documento = copia(nome);
    let errori: Vec<String> = schema_comune("runtime-vector-v1.schema.json")
        .iter_errors(&documento)
        .map(|e| e.to_string())
        .collect();
    assert!(
        errori.is_empty(),
        "{nome} valida contro runtime-vector-v1: {errori:?}"
    );
    documento
}

#[test]
fn il_selettore_del_vettore_di_richiesta_e_ammesso() {
    let richiesta = vettore("io-read-request.json");
    let metadati = &richiesta["metadata"];
    let campo = |chiave: &str| metadati[chiave].as_str().expect("stringa");
    let descrittore = verifica_instradamento(&Instradamento {
        nome_capacita: campo("plenora.capability.name"),
        versione_capacita: campo("plenora.capability.version"),
        operazione: campo("plenora.capability.operation"),
        versione_operazione: campo("plenora.operation.version"),
        contratto_ingresso: campo("plenora.input.contract"),
        content_type: richiesta["content_type"].as_str().expect("stringa"),
    })
    .expect("il selettore del vettore corrisponde al descrittore runtime");
    assert_eq!(descrittore.operazione, "io.read");
}

/// RUNTIME-VECTORS-1.0 §5: ogni mutazione dell'instradamento fallisce chiusa,
/// prima dell'invocazione e con `remote_effect: none`.
///
/// Le categorie seguono la regola R1 della matrice comune del binding
/// runtime (proposta P): ben formato ma non annunciato -> `unsupported`,
/// malformato o non canonico -> `protocol`. I metadati del risultato seguono
/// R2: operazione e versione si riflettono byte per byte se canoniche, e si
/// omettono altrimenti -- mai normalizzate, mai sostituite.
/// Una mutazione: chiave, valore, categoria attesa, operazione e versione
/// riflesse nel risultato.
type Mutazione = (
    &'static str,
    Value,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
);

#[test]
// La tabella delle mutazioni e' lunga per costruzione: una riga per caso
// della matrice, e spezzarla separerebbe i casi dal confronto che li prova.
#[allow(clippy::too_many_lines)]
fn le_mutazioni_dell_instradamento_falliscono_prima_dell_invocazione() {
    let richiesta = vettore("io-read-request.json");
    // (chiave, valore, categoria, operazione riflessa, versione riflessa)
    let mutazioni: [Mutazione; 13] = [
        (
            "plenora.capability.name",
            json!("plenora.data-tools"),
            "unsupported",
            Some("io.read"),
            Some("1"),
        ),
        (
            "plenora.capability.name",
            json!("Plenora.IO-tools"),
            "protocol",
            Some("io.read"),
            Some("1"),
        ),
        (
            "plenora.capability.version",
            json!("2"),
            "unsupported",
            Some("io.read"),
            Some("1"),
        ),
        (
            "plenora.capability.version",
            json!("01"),
            "protocol",
            Some("io.read"),
            Some("1"),
        ),
        (
            "plenora.capability.operation",
            json!("io.scan"),
            "unsupported",
            Some("io.scan"),
            Some("1"),
        ),
        (
            "plenora.capability.operation",
            json!("IO.READ"),
            "protocol",
            None,
            Some("1"),
        ),
        (
            "plenora.operation.version",
            json!("2"),
            "unsupported",
            Some("io.read"),
            Some("2"),
        ),
        (
            "plenora.operation.version",
            json!("+1"),
            "protocol",
            Some("io.read"),
            None,
        ),
        (
            "plenora.operation.version",
            json!("01"),
            "protocol",
            Some("io.read"),
            None,
        ),
        (
            "plenora.operation.version",
            json!(" 1"),
            "protocol",
            Some("io.read"),
            None,
        ),
        (
            "plenora.input.contract",
            json!("plenora-io-read-input-v2"),
            "unsupported",
            Some("io.read"),
            Some("1"),
        ),
        (
            "plenora.input.contract",
            json!("plenora-io-inspect-input-v1"),
            "unsupported",
            Some("io.read"),
            Some("1"),
        ),
        (
            "plenora.input.contract",
            json!("io-read-input"),
            "protocol",
            Some("io.read"),
            Some("1"),
        ),
    ];
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let binding = binding(&deposito);
    for (chiave, valore, categoria, operazione, versione) in mutazioni {
        let mut mutata = richiesta.clone();
        mutata["metadata"][chiave] = valore.clone();
        let risultato = binding.invoca(
            &invocazione(
                &mutata,
                json!({"source": "artifact://input/io-read-vector"}),
            ),
            CancellationToken::new(),
        );
        let documento = errore(&risultato);
        assert_eq!(documento["category"], categoria, "{chiave}={valore}");
        assert_eq!(documento["remote_effect"], "none", "{chiave}={valore}");
        assert_eq!(documento["phase"], "validate", "{chiave}={valore}");
        assert_eq!(documento["retry"]["kind"], "never", "{chiave}={valore}");
        assert_eq!(
            risultato.metadata.operazione.as_deref(),
            operazione,
            "{chiave}={valore}"
        );
        assert_eq!(
            risultato.metadata.versione_operazione.as_deref(),
            versione,
            "{chiave}={valore}"
        );
    }
    // Il content type e' instradamento anch'esso (RT-005): ben formato e non
    // annunciato, poi malformato.
    for (content_type, categoria) in [
        ("application/vnd.apache.arrow.stream", "unsupported"),
        ("json", "protocol"),
    ] {
        let mut mutata = richiesta.clone();
        mutata["content_type"] = json!(content_type);
        let risultato = binding.invoca(
            &invocazione(
                &mutata,
                json!({"source": "artifact://input/io-read-vector"}),
            ),
            CancellationToken::new(),
        );
        assert_eq!(errore(&risultato)["category"], categoria, "{content_type}");
    }
    assert_eq!(
        deposito.chiamate.get(),
        0,
        "nessuna mutazione raggiunge il risolutore"
    );
}

/// RT-012 e la matrice comune, caso 6: il risultato ha un `message.id`
/// **nuovo**, la causazione e' il `message.id` della richiesta, la
/// correlazione e' quella della richiesta. Vale per il successo e per
/// l'errore.
#[test]
fn il_risultato_ha_un_identita_nuova_causata_dalla_richiesta() {
    let richiesta = vettore("io-read-request.json");
    let id_richiesta = richiesta["metadata"]["plenora.message.id"].clone();
    let correlazione = richiesta["metadata"]["plenora.trace.correlation_id"].clone();
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let binding = binding(&deposito);
    let mut con_causa = richiesta.clone();
    con_causa["metadata"]["plenora.message.causation_id"] =
        json!("018f3d84-7b2c-7f00-8000-0000000000ff");
    let mut visti = BTreeSet::new();
    for (richiesta, payload) in [
        (
            &richiesta,
            json!({"source": "artifact://input/io-read-vector"}),
        ),
        (
            &con_causa,
            json!({"source": "artifact://input/io-read-vector"}),
        ),
        (&richiesta, json!({"source": "artifact://input/assente"})),
    ] {
        let risultato = binding.invoca(&invocazione(richiesta, payload), CancellationToken::new());
        let metadati = &risultato.metadata;
        assert_ne!(json!(metadati.id_messaggio), id_richiesta, "id nuovo");
        assert!(
            uuid_canonico(&metadati.id_messaggio),
            "{}",
            metadati.id_messaggio
        );
        assert!(
            visti.insert(metadati.id_messaggio.clone()),
            "un id per risultato"
        );
        assert_eq!(json!(metadati.id_causa), id_richiesta, "causa = richiesta");
        assert_eq!(json!(metadati.id_correlazione), correlazione);
    }
}

/// Un selettore mancante non e' un'invocazione: non si legge nemmeno.
#[test]
fn un_selettore_mancante_o_null_non_si_legge() {
    let richiesta = vettore("io-read-request.json");
    for chiave in [
        "plenora.capability.name",
        "plenora.capability.version",
        "plenora.capability.operation",
        "plenora.operation.version",
        "plenora.input.contract",
        "plenora.message.id",
        "plenora.trace.correlation_id",
    ] {
        let mut mutata = richiesta.clone();
        mutata["metadata"]
            .as_object_mut()
            .expect("oggetto")
            .remove(chiave);
        assert!(
            serde_json::from_value::<Invocazione>(mutata).is_err(),
            "{chiave} mancante"
        );
    }
    for chiave in [
        "plenora.execution.deadline",
        "plenora.message.causation_id",
        "plenora.execution.idempotency_key",
    ] {
        let mut mutata = richiesta.clone();
        mutata["metadata"][chiave] = Value::Null;
        assert!(
            serde_json::from_value::<Invocazione>(mutata).is_err(),
            "{chiave} null e' rifiutato, non letto come assente"
        );
    }
    let mut ignota = richiesta;
    ignota["metadata"]["plenora.trace.correlationId"] = json!("x");
    assert!(serde_json::from_value::<Invocazione>(ignota).is_err());
}

/// Il payload illustrativo del vettore, cosi' com'e', e' rifiutato tipizzato.
#[test]
fn il_payload_illustrativo_del_vettore_e_rifiutato_dal_nostro_schema() {
    let richiesta = vettore("io-read-request.json");
    assert!(
        !schema_nostro("plenora-io-read-input-v1").is_valid(&richiesta["payload"]),
        "il payload del vettore non e' un plenora-io-read-input-v1: se lo diventa, \
         la deviazione dichiarata nel manifesto va tolta"
    );
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let risultato = binding(&deposito).invoca(
        &invocazione(&richiesta, Value::Null),
        CancellationToken::new(),
    );
    let documento = errore(&risultato);
    assert_eq!(documento["code"], "RUNTIME_PAYLOAD_INVALID");
    assert_eq!(documento["category"], "invalid_configuration");
    assert_eq!(documento["remote_effect"], "none");
    assert_eq!(deposito.chiamate.get(), 0);
    // La correlazione resta quella dell'invocazione anche nell'errore.
    assert_eq!(
        json!(risultato.metadata.id_correlazione),
        richiesta["metadata"]["plenora.trace.correlation_id"]
    );
}

/// Il vettore di successo: stesso content type, contratto, operazione e
/// versione, la correlazione della richiesta, e un risultato vero.
#[test]
fn la_lettura_rende_il_flusso_arrow_del_vettore_di_successo() {
    let richiesta = vettore("io-read-request.json");
    let atteso = vettore("io-read-success.json");
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let risultato = binding(&deposito).invoca(
        &invocazione(
            &richiesta,
            json!({"source": "artifact://input/io-read-vector"}),
        ),
        CancellationToken::new(),
    );
    assert_eq!(
        risultato.content_type, atteso["content_type"],
        "{risultato:?}"
    );
    assert_eq!(risultato.content_type, CONTENT_TYPE_FLUSSO_ARROW);
    let metadati = &atteso["metadata"];
    assert_eq!(
        json!(risultato.metadata.operazione),
        metadati["plenora.capability.operation"]
    );
    assert_eq!(
        json!(risultato.metadata.versione_operazione),
        metadati["plenora.operation.version"]
    );
    assert_eq!(
        risultato.metadata.contratto_uscita,
        metadati["plenora.output.contract"]
    );
    // RT-012: il risultato conserva la correlazione **della richiesta**; quella
    // della fixture di successo e' un'altra, ed e' illustrativa.
    assert_eq!(
        json!(risultato.metadata.id_correlazione),
        richiesta["metadata"]["plenora.trace.correlation_id"]
    );
    let Carico::FlussoArrow { byte, rapporto } = &risultato.payload else {
        panic!("io.read rende il flusso");
    };
    // I byte sono un flusso Arrow IPC che la libreria di un consumatore legge.
    let lettore = arrow_ipc::reader::StreamReader::try_new(Cursor::new(byte.as_slice()), None)
        .expect("e' un flusso Arrow IPC");
    let righe: usize = lettore
        .map(|batch| batch.expect("ogni batch si decodifica").num_rows())
        .sum();
    assert!(righe > 0, "il flusso porta righe");
    // Il rapporto e' il documento del contratto d'uscita, e dice gli stessi byte.
    let errori: Vec<String> = schema_nostro("plenora-io-read-result-v1")
        .iter_errors(rapporto)
        .map(|e| e.to_string())
        .collect();
    assert!(errori.is_empty(), "{errori:?}");
    assert_eq!(rapporto["rows_read"], righe);
    assert_eq!(
        rapporto["delivered"]["content_type"],
        CONTENT_TYPE_FLUSSO_ARROW
    );
    assert_eq!(rapporto["delivered"]["bytes_written"], byte.len());
}

/// Il vettore d'errore: cancellazione prima che esca un risultato.
///
/// Content type, contratto, categoria ed effetto coincidono con la fixture.
/// Fase, politica di ritentare e codice no, e la differenza e' **dichiarata**
/// nel manifesto di adozione: questa prova la fissa, perche' una deviazione
/// che cambia senza che il manifesto cambi e' una deviazione non dichiarata.
#[test]
fn la_cancellazione_rende_l_errore_del_vettore() {
    let richiesta = vettore("io-read-request.json");
    let atteso = vettore("io-read-cancelled-error.json");
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let cancellazione = CancellationToken::new();
    cancellazione.cancel();
    let risultato = binding(&deposito).invoca(
        &invocazione(
            &richiesta,
            json!({"source": "artifact://input/io-read-vector"}),
        ),
        cancellazione,
    );
    assert_eq!(risultato.content_type, atteso["content_type"]);
    assert_eq!(
        risultato.metadata.contratto_uscita,
        atteso["metadata"]["plenora.output.contract"]
    );
    assert_eq!(risultato.metadata.operazione.as_deref(), Some("io.read"));
    let documento = errore(&risultato);
    let fixture = &atteso["payload"];
    assert_eq!(documento["category"], fixture["category"]);
    assert_eq!(documento["remote_effect"], fixture["remote_effect"]);
    // La deviazione dichiarata, valore per valore.
    assert_eq!(fixture["retry"]["kind"], "safe");
    assert_eq!(documento["retry"]["kind"], "never");
    assert_eq!(fixture["code"], "READ_CANCELLED");
    assert_eq!(documento["code"], "CANCELLED");
    assert_eq!(fixture["phase"], "read");
    assert_eq!(documento["phase"], "probe");
}

/// La correlazione e le identita' non canoniche (RT-012).
#[test]
fn le_identita_non_canoniche_sono_rifiutate_e_non_rimandate() {
    let richiesta = vettore("io-read-request.json");
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let binding = binding(&deposito);
    for (chiave, valore) in [
        ("plenora.message.id", "018F3D84-7B2C-7F00-8000-000000000105"),
        (
            "plenora.trace.correlation_id",
            "{018f3d84-7b2c-7f00-8000-000000000005}",
        ),
        (
            "plenora.message.causation_id",
            "018f3d847b2c7f008000000000000005",
        ),
    ] {
        let mut mutata = richiesta.clone();
        mutata["metadata"][chiave] = json!(valore);
        let risultato = binding.invoca(
            &invocazione(
                &mutata,
                json!({"source": "artifact://input/io-read-vector"}),
            ),
            CancellationToken::new(),
        );
        assert_eq!(errore(&risultato)["code"], "RUNTIME_IDENTITY_INVALID");
        assert_eq!(errore(&risultato)["category"], "protocol");
        // R2: un'identita' non canonica non si riflette, non si normalizza e
        // non si sostituisce: la chiave si omette.
        let metadati = &risultato.metadata;
        assert_ne!(metadati.id_messaggio, valore, "{chiave}");
        if chiave == "plenora.trace.correlation_id" {
            assert_eq!(metadati.id_correlazione, None, "{chiave}");
        }
        if chiave == "plenora.message.id" {
            assert_eq!(metadati.id_causa, None, "{chiave}");
        }
        assert_ne!(metadati.id_causa.as_deref(), Some(valore), "{chiave}");
    }
    assert_eq!(deposito.chiamate.get(), 0);
}

/// I controlli: idempotenza mai, scadenza dove il descrittore la dichiara.
#[test]
fn i_controlli_non_dichiarati_non_sono_accettati_in_silenzio() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let binding = binding(&deposito);

    let mut chiave = per_operazione(
        "io.read",
        json!({"source": "artifact://input/io-read-vector"}),
    );
    chiave.metadata.chiave_idempotenza = Some("k-1".to_owned());
    let risultato = binding.invoca(&chiave, CancellationToken::new());
    assert_eq!(errore(&risultato)["code"], "RUNTIME_CONTROL_UNSUPPORTED");

    let mut catalogo = per_operazione("io.catalog", json!({}));
    catalogo.metadata.scadenza = Some("2030-01-01T00:00:00Z".to_owned());
    let risultato = binding.invoca(&catalogo, CancellationToken::new());
    assert_eq!(errore(&risultato)["code"], "RUNTIME_CONTROL_UNSUPPORTED");
    assert_eq!(errore(&risultato)["category"], "unsupported");

    let mut passata = per_operazione(
        "io.read",
        json!({"source": "artifact://input/io-read-vector"}),
    );
    passata.metadata.scadenza = Some("2020-01-01T00:00:00Z".to_owned());
    let risultato = binding.invoca(&passata, CancellationToken::new());
    // SURF-010: una scadenza osservata e' un `timeout`; prima
    // dell'invocazione `validate`, `none`, `never` (matrice, caso 7b).
    let documento = errore(&risultato);
    assert_eq!(documento["code"], "DEADLINE_EXCEEDED");
    assert_eq!(documento["category"], "timeout");
    assert_eq!(documento["phase"], "validate");
    assert_eq!(documento["remote_effect"], "none");
    assert_eq!(documento["retry"]["kind"], "never");

    // Una grafia sola (matrice, caso 7a): ogni altra e' `protocol`.
    for storta in [
        "2030-01-01T00:00:00+01:00",
        "2030-01-01T00:00:00+00:00",
        "2030-01-01T00:00:00-00:00",
        "2030-01-01t00:00:00Z",
        "2030-01-01T00:00:00z",
        "2030-01-01 00:00:00Z",
        "2030-01-01T00:00:60Z",
        "domani",
    ] {
        let mut invocazione = per_operazione(
            "io.read",
            json!({"source": "artifact://input/io-read-vector"}),
        );
        invocazione.metadata.scadenza = Some(storta.to_owned());
        let risultato = binding.invoca(&invocazione, CancellationToken::new());
        assert_eq!(
            errore(&risultato)["code"],
            "RUNTIME_DEADLINE_INVALID",
            "{storta}"
        );
        assert_eq!(errore(&risultato)["category"], "protocol", "{storta}");
    }

    // La scadenza da due canali: rifiutata anche a valori uguali (caso 7c).
    let mut doppia = per_operazione(
        "io.read",
        json!({"source": "artifact://input/io-read-vector", "deadline_ms": 30000}),
    );
    doppia.metadata.scadenza = Some("2030-01-01T00:00:00Z".to_owned());
    let risultato = binding.invoca(&doppia, CancellationToken::new());
    assert_eq!(errore(&risultato)["code"], "RUNTIME_DEADLINE_TWICE");
    assert_eq!(errore(&risultato)["category"], "invalid_configuration");
    assert_eq!(deposito.chiamate.get(), 0);

    // Una scadenza futura ammessa: la lettura si compie.
    let mut futura = per_operazione(
        "io.read",
        json!({"source": "artifact://input/io-read-vector"}),
    );
    futura.metadata.scadenza = Some("2999-12-31T23:59:59.5Z".to_owned());
    let risultato = binding.invoca(&futura, CancellationToken::new());
    assert_eq!(
        risultato.content_type, CONTENT_TYPE_FLUSSO_ARROW,
        "{risultato:?}"
    );
}

/// RT-013: un percorso locale non attraversa il confine, nemmeno fino al risolutore.
#[test]
fn i_percorsi_locali_non_raggiungono_il_risolutore() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let binding = binding(&deposito);
    for storto in [
        "C:\\dati\\canonico.gpkg",
        "c:/dati/canonico.gpkg",
        "/etc/passwd",
        "../canonico.gpkg",
        "file:///tmp/canonico.gpkg",
        "FILE:canonico.gpkg",
        "artifact://input/../segreto",
        "artifact://input/%2e%2e/segreto",
        "artifact://input/con spazio",
        "artifact:",
        "x:canonico",
    ] {
        assert!(!riferimento_opaco(storto), "{storto}");
        let risultato = binding.invoca(
            &per_operazione("io.inspect", json!({"source": storto})),
            CancellationToken::new(),
        );
        let documento = errore(&risultato);
        assert_eq!(
            documento["code"], "RUNTIME_ARTIFACT_REFERENCE_INVALID",
            "{storto}"
        );
        let testo = documento.to_string();
        assert!(
            !testo.contains(storto),
            "il riferimento non entra nel messaggio"
        );
    }
    assert_eq!(deposito.chiamate.get(), 0);
    for buono in [
        "artifact://input/io-read-vector",
        "s3://bucket/chiave.gpkg",
        "urn:plenora:x",
    ] {
        assert!(riferimento_opaco(buono), "{buono}");
    }
}

#[test]
fn un_riferimento_che_l_applicazione_non_risolve_e_not_found() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let risultato = binding(&deposito).invoca(
        &per_operazione("io.inspect", json!({"source": "artifact://input/assente"})),
        CancellationToken::new(),
    );
    let documento = errore(&risultato);
    assert_eq!(documento["code"], "RUNTIME_ARTIFACT_NOT_FOUND");
    assert_eq!(documento["category"], "not_found");
    assert!(!documento.to_string().contains("assente"));
}

/// Il payload a campi chiusi: ignoti, mancanti, null, tipi e vincoli.
#[test]
fn il_payload_si_legge_contro_lo_schema_dell_operazione() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let binding = binding(&deposito);
    let fonte = "artifact://input/io-read-vector";
    let casi = [
        (
            "io.inspect",
            json!({"source": fonte, "layer": 0}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
        ("io.inspect", json!({}), "RUNTIME_PAYLOAD_INVALID"),
        (
            "io.inspect",
            json!({"source": fonte, "assume_crs": null}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
        (
            "io.inspect",
            json!({"source": fonte, "deadline_ms": 1.5}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
        (
            "io.inspect",
            json!({"source": fonte, "deadline_ms": 0}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
        (
            "io.inspect",
            json!({"source": ""}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
        ("io.inspect", json!([fonte]), "RUNTIME_PAYLOAD_INVALID"),
        (
            "io.read",
            json!({"source": fonte, "layer": -1}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
        (
            "io.read",
            json!({"source": fonte, "budgets": {"max_rows": null}}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
        (
            "io.read",
            json!({"source": fonte, "budgets": {"righe": 3}}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
        (
            "io.read",
            json!({"source": fonte, "destination": "artifact://output/x.arrow"}),
            "RUNTIME_READ_DESTINATION",
        ),
        (
            "io.read",
            json!({"source": fonte, "durable": false}),
            "RUNTIME_READ_DESTINATION",
        ),
        (
            "io.read",
            json!({"source": fonte, "limit": 3}),
            "LIMIT_WITH_DELIVERY",
        ),
        (
            "io.catalog",
            json!({"source": fonte}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
        (
            "io.write",
            json!({"source": fonte, "destination": "artifact://output/x.geojson"}),
            "RUNTIME_PAYLOAD_INVALID",
        ),
    ];
    for (operazione, payload, codice) in casi {
        let risultato = binding.invoca(
            &per_operazione(operazione, payload.clone()),
            CancellationToken::new(),
        );
        assert_eq!(errore(&risultato)["code"], codice, "{operazione} {payload}");
        assert_eq!(errore(&risultato)["remote_effect"], "none");
    }
    assert_eq!(deposito.chiamate.get(), 0);
}

/// I campi ammessi dal binding sono quelli degli schemi pubblicati.
#[test]
fn i_campi_ammessi_sono_quelli_degli_schemi() {
    for descrittore in OPERAZIONI {
        let schema = documento(
            &radice()
                .join("contracts")
                .join("schemas")
                .join(format!("{}.schema.json", descrittore.contratto_ingresso)),
        );
        let proprieta: BTreeSet<String> = schema["properties"]
            .as_object()
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default();
        let richiesti: BTreeSet<String> = schema["required"]
            .as_array()
            .map(|r| {
                r.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let (ammessi, obbligatori) = campi_ammessi(descrittore.operazione);
        let ammessi: BTreeSet<String> = ammessi.iter().map(|s| (*s).to_owned()).collect();
        let obbligatori: BTreeSet<String> = obbligatori.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(ammessi, proprieta, "{}", descrittore.operazione);
        assert_eq!(obbligatori, richiesti, "{}", descrittore.operazione);
    }
}

/// Il documento capability runtime: forma, registro dei binding, catalogo.
#[test]
fn il_documento_capability_runtime_dice_cio_che_il_registro_e_il_catalogo_chiedono() {
    let documento = capacita();
    let errori: Vec<String> = schema_comune("capabilities-v2.schema.json")
        .iter_errors(&documento)
        .map(|e| e.to_string())
        .collect();
    assert!(errori.is_empty(), "{errori:?}");

    let registro = copia("runtime-v1.json");
    let nostra = registro["components"]
        .as_array()
        .expect("componenti")
        .iter()
        .find(|c| c["component"] == "plenora-io-tools")
        .expect("io-tools nel registro");
    assert_eq!(documento["interfaces"][0]["artifact"], nostra["artifact"]);
    assert_eq!(
        nostra["discovery"],
        json!(["plenora.io-tools#capabilities@1"])
    );
    let catalogo = copia("io-tools-v1.json");
    for binding in nostra["bindings"].as_array().expect("binding") {
        let id = binding["operation"].as_str().expect("id");
        let operazione = documento["operations"]
            .as_array()
            .expect("operazioni")
            .iter()
            .find(|o| o["id"] == id)
            .unwrap_or_else(|| panic!("{id}: richiesta dal registro e assente"));
        assert_eq!(operazione["version"], binding["version"]);
        assert_eq!(operazione["status"], "available");
        assert_eq!(
            binding["entrypoints"],
            json!([format!("plenora.io-tools#{id}@{}", binding["version"])])
        );
        let voce = catalogo["operations"]
            .as_array()
            .expect("catalogo")
            .iter()
            .find(|o| o["id"] == id)
            .expect("nel catalogo");
        assert_eq!(
            operazione["input"]["contract"], voce["input"]["contract"],
            "{id}"
        );
        assert_eq!(
            operazione["output"]["contract"], voce["output"]["contract"],
            "{id}"
        );
        // Su questa superficie anche `io.read` ha l'effetto del catalogo.
        assert_eq!(operazione["side_effect"], voce["side_effect"], "{id}");
        assert_eq!(operazione["controls"], voce["controls"], "{id}");
        for lato in ["input", "output"] {
            let ammessi: BTreeSet<&str> = voce[lato]["content_types"]
                .as_array()
                .expect("content types")
                .iter()
                .filter_map(Value::as_str)
                .collect();
            for dichiarato in operazione[lato]["content_types"].as_array().expect("ct") {
                assert!(
                    ammessi.contains(dichiarato.as_str().expect("stringa")),
                    "{id} {lato}: {dichiarato} non e' fra quelli del catalogo"
                );
            }
        }
    }
}

/// Il runtime e la superficie Rust rendono lo stesso documento.
#[test]
fn inspect_e_layers_coincidono_con_la_superficie_rust() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let binding = binding(&deposito);
    let fonte = "artifact://input/io-read-vector";
    for operazione in ["io.inspect", "io.layers"] {
        let risultato = binding.invoca(
            &per_operazione(operazione, json!({"source": fonte})),
            CancellationToken::new(),
        );
        let runtime = successo_json(&risultato).clone();
        let richiesta = Richiesta::sulla_sorgente(deposito.percorso(fonte));
        let rust = if operazione == "io.inspect" {
            operazioni::inspect(richiesta)
        } else {
            operazioni::layers(richiesta)
        }
        .expect("la superficie Rust riesce");
        assert_eq!(runtime, rust, "{operazione}");
    }
    let catalogo = binding.invoca(
        &per_operazione("io.catalog", json!({})),
        CancellationToken::new(),
    );
    assert_eq!(successo_json(&catalogo), &operazioni::catalog());
}

/// `io.convert` e `io.write` pubblicano dove l'applicazione risolve la destinazione.
#[test]
fn convert_e_write_pubblicano_nella_destinazione_risolta() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let binding = binding(&deposito);
    let convertito = binding.invoca(
        &per_operazione(
            "io.convert",
            json!({
                "source": "artifact://input/canonico-geojson",
                "destination": "artifact://output/convertito.csv",
                "source_format": "geojson",
                "target_format": "csv",
            }),
        ),
        CancellationToken::new(),
    );
    let documento = successo_json(&convertito);
    assert!(
        schema_nostro("plenora-io-convert-v1").is_valid(documento),
        "{documento}"
    );
    assert!(temporanea.path().join("convertito.csv").is_file());

    let scritto = binding.invoca(
        &per_operazione(
            "io.write",
            json!({
                "source": "artifact://input/canonico-arrow",
                "destination": "artifact://output/scritto.csv",
                "format": "csv",
            }),
        ),
        CancellationToken::new(),
    );
    let documento = successo_json(&scritto);
    assert!(
        schema_nostro("plenora-io-write-result-v1").is_valid(documento),
        "{documento}"
    );
    assert!(temporanea.path().join("scritto.csv").is_file());

    // La destinazione gia' presente e' rifiutata come sulla CLI.
    let ancora = binding.invoca(
        &per_operazione(
            "io.write",
            json!({
                "source": "artifact://input/canonico-arrow",
                "destination": "artifact://output/scritto.csv",
                "format": "csv",
            }),
        ),
        CancellationToken::new(),
    );
    assert_eq!(errore(&ancora)["code"], "OUTPUT_EXISTS");
}

/// La consegna di `io.read` non lascia nulla su disco fuori dalla directory privata.
#[test]
fn la_lettura_runtime_non_scrive_nel_deposito() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let prima: BTreeSet<PathBuf> = std::fs::read_dir(temporanea.path())
        .expect("dir")
        .map(|e| e.expect("voce").path())
        .collect();
    let risultato = binding(&deposito).invoca(
        &per_operazione(
            "io.read",
            json!({"source": "artifact://input/io-read-vector"}),
        ),
        CancellationToken::new(),
    );
    assert_eq!(risultato.content_type, CONTENT_TYPE_FLUSSO_ARROW);
    let dopo: BTreeSet<PathBuf> = std::fs::read_dir(temporanea.path())
        .expect("dir")
        .map(|e| e.expect("voce").path())
        .collect();
    assert_eq!(prima, dopo);
}

/// L'oracolo del calcolo delle date: un conteggio giorno per giorno.
///
/// L'algoritmo di `istante_rfc3339_utc` e' aritmetica chiusa; qui lo si
/// confronta, giorno per giorno dal 1970 al 2200, con la somma ingenua dei
/// giorni di ciascun mese, che non condivide nulla con lui.
#[test]
fn l_istante_rfc3339_coincide_con_il_conteggio_ingenuo() {
    let mut giorni_totali: u64 = 0;
    for anno in 1970u64..2200 {
        let bisestile = (anno % 4 == 0 && anno % 100 != 0) || anno % 400 == 0;
        for mese in 1u64..=12 {
            let lunghezza = match mese {
                2 if bisestile => 29,
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            for giorno in 1..=lunghezza {
                let testo = format!("{anno:04}-{mese:02}-{giorno:02}T01:02:03Z");
                let atteso = giorni_totali * 86_400 + 3_723;
                assert_eq!(secondi(&testo), Some(atteso), "{testo}");
                giorni_totali += 1;
            }
            let oltre = format!("{anno:04}-{mese:02}-{:02}T00:00:00Z", lunghezza + 1);
            assert_eq!(istante_rfc3339_utc(&oltre), None, "{oltre}");
        }
    }
    // Le forme ammesse e quelle rifiutate.
    assert_eq!(secondi("2030-01-01T00:00:00Z"), Some(1_893_456_000));
    assert_eq!(
        istante_rfc3339_utc("2030-01-01T00:00:00.25Z"),
        Some(IstanteRfc3339::DallEpoca(Duration::from_millis(
            1_893_456_000_250
        )))
    );
    // Una grafia sola: niente minuscole, niente `+00:00`.
    assert_eq!(istante_rfc3339_utc("2030-01-01t00:00:00.25Z"), None);
    assert_eq!(istante_rfc3339_utc("2030-01-01T00:00:00+00:00"), None);
    // Prima del 1970: un istante valido e passato, non un testo invalido;
    // una data impossibile resta invalida anche li'.
    assert_eq!(
        istante_rfc3339_utc("1969-12-31T23:59:59Z"),
        Some(IstanteRfc3339::PrimaDellEpoca)
    );
    assert_eq!(istante_rfc3339_utc("1900-02-29T00:00:00Z"), None);
    assert!(istante_rfc3339_utc("2000-02-29T00:00:00Z").is_some());
    for storta in [
        "2030-01-01T00:00:60Z",
        "2030-13-01T00:00:00Z",
        "2030-01-01T24:00:00Z",
        "2030-01-01T00:00:00.Z",
        "2030-01-01T00:00:00.1234567891Z",
        "2030-01-01T00:00:00",
        "+2030-01-01T00:00:00Z",
        "2030-1-01T00:00:00Z",
    ] {
        assert_eq!(istante_rfc3339_utc(storta), None, "{storta}");
    }
}

fn secondi(testo: &str) -> Option<u64> {
    match istante_rfc3339_utc(testo)? {
        IstanteRfc3339::DallEpoca(durata) => Some(durata.as_secs()),
        IstanteRfc3339::PrimaDellEpoca => None,
    }
}

/// La scadenza del vettore e' il 2030-01-01: con un orologio del 2031 la
/// stessa invocazione e' rifiutata prima del risolutore, con l'errore della
/// durata esaurita.
#[test]
fn la_scadenza_del_vettore_vale_davvero() {
    let richiesta = vettore("io-read-request.json");
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let risultato = BindingRuntime::new(&deposito)
        .con_orologio(orologio_del_2031)
        .invoca(
            &invocazione(
                &richiesta,
                json!({"source": "artifact://input/io-read-vector"}),
            ),
            CancellationToken::new(),
        );
    let documento = errore(&risultato);
    assert_eq!(documento["code"], "DEADLINE_EXCEEDED");
    assert_eq!(documento["category"], "timeout");
    assert_eq!(documento["remote_effect"], "none");
    assert_eq!(deposito.chiamate.get(), 0);
}

/// Il tempo del risolutore consuma la scadenza, invece di aggiungersi a lei.
#[test]
fn il_risolutore_lento_consuma_la_scadenza() {
    fn quasi_adesso() -> SystemTime {
        UNIX_EPOCH + Duration::from_hours(497_544)
    }
    let temporanea = tempfile::tempdir().expect("tempdir");
    let mut deposito = Deposito::nuovo(temporanea.path());
    deposito.attesa = Duration::from_millis(600);
    let mut invocazione = per_operazione(
        "io.read",
        json!({"source": "artifact://input/io-read-vector"}),
    );
    // Duecento millisecondi dopo l'orologio del binding.
    invocazione.metadata.scadenza = Some("2026-10-05T00:00:00.2Z".to_owned());
    let risultato = BindingRuntime::new(&deposito)
        .con_orologio(quasi_adesso)
        .invoca(&invocazione, CancellationToken::new());
    let documento = errore(&risultato);
    assert_eq!(documento["code"], "DEADLINE_EXCEEDED", "{documento}");
    assert_eq!(documento["category"], "timeout", "{documento}");
    assert_eq!(
        deposito.chiamate.get(),
        1,
        "la scadenza scade durante il risolutore"
    );
}

/// RT-013: il nome che l'applicazione da' al file non esce nei risultati.
#[test]
fn il_nome_del_file_materializzato_e_quello_del_riferimento() {
    struct Segreto(PathBuf);
    impl RisolutoreArtefatti for Segreto {
        fn sorgente(&self, _riferimento: &str) -> Result<PathBuf, RifiutoArtefatto> {
            Ok(self.0.clone())
        }
        fn destinazione(&self, _riferimento: &str) -> Result<PathBuf, RifiutoArtefatto> {
            Err(RifiutoArtefatto::NonAutorizzato)
        }
    }
    let temporanea = tempfile::tempdir().expect("tempdir");
    let segreto = temporanea.path().join("segreto-cliente-42.geojson");
    std::fs::copy(fixture("canonico.geojson"), &segreto).expect("copia");
    let risolutore = Segreto(segreto);
    let risultato = BindingRuntime::new(&risolutore).invoca(
        &per_operazione(
            "io.layers",
            json!({"source": "artifact://input/canonico-geojson"}),
        ),
        CancellationToken::new(),
    );
    let documento = errore(&risultato);
    assert_eq!(documento["code"], "RUNTIME_ARTIFACT_NAME_MISMATCH");
    assert!(!documento.to_string().contains("segreto"));

    // Con il nome del riferimento, il layer porta quel nome e nessun altro.
    let deposito = Deposito::nuovo(temporanea.path());
    let risultato = binding(&deposito).invoca(
        &per_operazione(
            "io.layers",
            json!({"source": "artifact://input/canonico-geojson"}),
        ),
        CancellationToken::new(),
    );
    let testo = successo_json(&risultato).to_string();
    assert!(testo.contains("\"canonico-geojson\""), "{testo}");
}

/// Un formato fuori dall'enum si rifiuta prima di chiedere gli artefatti.
#[test]
fn un_formato_ignoto_non_raggiunge_il_risolutore() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    for payload in [
        json!({"source": "artifact://input/canonico-arrow", "destination": "artifact://output/x.csv", "format": "GeoJSON"}),
        json!({"source": "artifact://input/canonico-geojson", "destination": "artifact://output/x.csv", "source_format": "json", "target_format": "csv"}),
    ] {
        let operazione = if payload.get("format").is_some() {
            "io.write"
        } else {
            "io.convert"
        };
        let risultato = binding(&deposito).invoca(
            &per_operazione(operazione, payload),
            CancellationToken::new(),
        );
        assert_eq!(errore(&risultato)["code"], "UNKNOWN_FORMAT");
        assert_eq!(errore(&risultato)["category"], "unsupported");
    }
    assert_eq!(deposito.chiamate.get(), 0);
}

/// La directory privata della consegna sparisce, anche quando la lettura fallisce.
#[test]
fn la_directory_privata_sparisce_sempre() {
    let temporanea = tempfile::tempdir().expect("tempdir");
    let deposito = Deposito::nuovo(temporanea.path());
    let genitore = tempfile::tempdir().expect("tempdir");
    let binding = binding(&deposito).con_directory_temporanea(genitore.path());
    let vuota = || {
        std::fs::read_dir(genitore.path())
            .expect("dir")
            .next()
            .is_none()
    };

    let riuscita = binding.invoca(
        &per_operazione(
            "io.read",
            json!({"source": "artifact://input/io-read-vector"}),
        ),
        CancellationToken::new(),
    );
    assert_eq!(riuscita.content_type, CONTENT_TYPE_FLUSSO_ARROW);
    assert!(vuota(), "dopo una lettura riuscita");

    let annullata = CancellationToken::new();
    annullata.cancel();
    let fallita = binding.invoca(
        &per_operazione(
            "io.read",
            json!({"source": "artifact://input/io-read-vector"}),
        ),
        annullata,
    );
    assert_eq!(errore(&fallita)["category"], "cancelled");
    assert!(vuota(), "dopo una lettura fallita");
}

/// `Invocazione::da_json`: un errore curato, senza i dati dell'ingresso.
#[test]
fn un_invocazione_malformata_e_un_errore_curato() {
    let richiesta = vettore("io-read-request.json");
    let testo = serde_json::to_vec(&json!({
        "content_type": richiesta["content_type"],
        "metadata": richiesta["metadata"],
        "payload": {"source": "artifact://input/io-read-vector"},
    }))
    .expect("json");
    assert!(Invocazione::da_json(&testo).is_ok());

    for storto in [
        b"{".to_vec(),
        b"{\"content_type\": 5}".to_vec(),
        br#"{"content_type":"application/json","metadata":{},"payload":{}}"#.to_vec(),
    ] {
        let errore = Invocazione::da_json(&storto).expect_err("malformata");
        assert_eq!(errore["code"], "RUNTIME_INVOCATION_INVALID");
        assert_eq!(errore["category"], "protocol");
        assert_eq!(errore["remote_effect"], "none");
        assert!(
            !errore.to_string().contains('5'),
            "nessun valore dell'ingresso"
        );
    }

    // Una chiave ripetuta nel payload non si legge come l'ultima delle due.
    let ripetuta = String::from_utf8(testo).expect("utf-8").replace(
        "{\"source\":\"artifact://input/io-read-vector\"}",
        "{\"source\":\"artifact://input/a\",\"source\":\"artifact://input/io-read-vector\"}",
    );
    assert!(ripetuta.contains("input/a"), "la sostituzione e' avvenuta");
    let errore = Invocazione::da_json(ripetuta.as_bytes()).expect_err("chiave ripetuta");
    assert_eq!(errore["code"], "RUNTIME_INVOCATION_INVALID");
}

fn uuid_canonico(valore: &str) -> bool {
    valore.len() == 36
        && valore.bytes().enumerate().all(|(indice, byte)| {
            if matches!(indice, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

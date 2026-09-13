//! Gli schemi ufficiali di `GeoParquet`, collegati al runtime.
//!
//! # Perche' esiste
//!
//! Il modulo `metadati` scrive a mano le regole che la specifica gia' scrive:
//! quali campi sono obbligatori, quali valori sono ammessi, che forma ha il
//! `covering`. Regole scritte due volte divergono, e la prima stesura di questo
//! lotto lo ha dimostrato tre volte -- ammetteva `" M"` e `" ZM"` che lo schema
//! non ha, deduplicava dove lo schema dice `uniqueItems`, e chiamava
//! «utilizzabile» la forma di `covering` che lo schema rifiuta.
//!
//! Qui la specifica entra nel runtime come **documento**, non come parafrasi:
//! il metadato `geo` viene validato contro lo schema ufficiale della versione
//! che dichiara, e il suo `crs` contro il PROJJSON che quello schema
//! referenzia.
//!
//! # I quattro schemi, e da dove vengono
//!
//! Sono fissati in `assurance/schemi/`, con impronta dei byte e canonica in
//! `assurance/registries/geoparquet-schemi-lock.json`, e sono **incorporati nel
//! binario**: `include_str!` li mette dentro l'eseguibile, quindi non c'e' un
//! file da leggere a runtime ne' un percorso che possa mancare.
//!
//! # Nessuna rete, nessun filesystem
//!
//! La dipendenza e' compilata con `default-features = false`, che toglie i
//! resolver HTTP e filesystem. I `$ref` verso PROJJSON si risolvono
//! **soltanto** dal registro in memoria costruito qui sotto: uno schema che si
//! scarica quando serve non e' fissato, e' sperato.
//!
//! # Fallire chiusi, senza panico e senza dire troppo
//!
//! Gli schemi sono costanti, quindi un errore di caricamento sarebbe un difetto
//! nostro e non un dato ostile -- ma un difetto nostro non e' una buona ragione
//! per far cadere il processo di chi ci usa. La compilazione avviene una volta
//! sola, in un `OnceLock`, e il suo esito e' un `Result` conservato: se fallisce
//! ogni validazione fallisce chiusa.
//!
//! I messaggi pubblici non portano **niente** del documento validato: il
//! percorso dell'errore, il valore che non andava bene, il nome della colonna
//! resterebbero appiccicati a un messaggio che finisce nei log di qualcun
//! altro. Si dice quale schema ha rifiutato, e basta.

use std::sync::OnceLock;

use jsonschema::{Draft, Registry, Resource, Validator};
use plenora_io_model::{PlenoraIoError, PublicMessage};
use serde_json::Value;

/// Lo schema `GeoParquet` 1.0.0, dai byte fissati.
const GEOPARQUET_1_0_0: &str =
    include_str!("../../../assurance/schemi/geoparquet-1.0.0.schema.json");
/// Lo schema `GeoParquet` 1.1.0, dai byte fissati.
const GEOPARQUET_1_1_0: &str =
    include_str!("../../../assurance/schemi/geoparquet-1.1.0.schema.json");
/// Il PROJJSON che lo schema 1.0.0 referenzia.
const PROJJSON_0_5: &str = include_str!("../../../assurance/schemi/projjson-0.5.schema.json");
/// Il PROJJSON che lo schema 1.1.0 referenzia.
const PROJJSON_0_7: &str = include_str!("../../../assurance/schemi/projjson-0.7.schema.json");

/// Gli identificatori con cui gli schemi si nominano fra loro.
const ID_PROJJSON_0_5: &str = "https://proj.org/schemas/v0.5/projjson.schema.json";
const ID_PROJJSON_0_7: &str = "https://proj.org/schemas/v0.7/projjson.schema.json";

/// Le versioni di `GeoParquet` che questa libreria legge, per intero.
///
/// Non `1.0.x` e `1.1.x`: `"version"` e' un `const` negli schemi ufficiali, e
/// vale esattamente questi due valori.
pub const VERSIONI_SUPPORTATE: [&str; 2] = ["1.0.0", "1.1.0"];

/// I validator compilati, uno per versione, costruiti una volta sola.
struct Validatori {
    v1_0_0: Validator,
    v1_1_0: Validator,
    /// Il PROJJSON che la 1.1.0 referenzia, interrogabile **da solo**.
    ///
    /// Serve in scrittura: un `crs` che non e' PROJJSON va fermato prima di
    /// creare qualsiasi file, e va fermato dicendo che il difetto sta nel CRS.
    /// Interrogare il solo PROJJSON e' cio' che permette di dirlo.
    projjson: Validator,
}

static VALIDATORI: OnceLock<Result<Validatori, &'static str>> = OnceLock::new();

/// Un documento che non rispetta lo schema ufficiale.
fn non_conforme(messaggio: &PublicMessage) -> PlenoraIoError {
    PlenoraIoError::formato_redatto("geoparquet", messaggio)
}

/// Costruisce i due validator, con il registro dei PROJJSON in memoria.
fn compila() -> Result<Validatori, &'static str> {
    let projjson_0_5: Value =
        serde_json::from_str(PROJJSON_0_5).map_err(|_| "PROJJSON 0.5 incorporato non e' JSON")?;
    let projjson_0_7: Value =
        serde_json::from_str(PROJJSON_0_7).map_err(|_| "PROJJSON 0.7 incorporato non e' JSON")?;

    // Il registro e' **l'unica** via con cui un `$ref` puo' essere risolto: la
    // crate e' compilata senza i resolver HTTP e filesystem, quindi un `$ref`
    // che non sia qui dentro fa fallire la compilazione dello schema invece di
    // andarselo a cercare.
    let registro: Registry = Registry::new()
        .draft(Draft::Draft7)
        .extend([
            (ID_PROJJSON_0_5, Resource::from_contents(projjson_0_5)),
            (ID_PROJJSON_0_7, Resource::from_contents(projjson_0_7)),
        ])
        .map_err(|_| "i PROJJSON incorporati non entrano nel registro")?
        .prepare()
        .map_err(|_| "il registro degli schemi non si prepara")?;

    let compila_uno = |testo: &str, nome: &'static str| -> Result<Validator, &'static str> {
        let documento: Value = serde_json::from_str(testo).map_err(|_| nome)?;
        jsonschema::options()
            .with_draft(Draft::Draft7)
            .with_registry(&registro)
            .build(&documento)
            .map_err(|_| nome)
    };

    Ok(Validatori {
        v1_0_0: compila_uno(
            GEOPARQUET_1_0_0,
            "lo schema GeoParquet 1.0.0 non si compila",
        )?,
        v1_1_0: compila_uno(
            GEOPARQUET_1_1_0,
            "lo schema GeoParquet 1.1.0 non si compila",
        )?,
        projjson: compila_uno(PROJJSON_0_7, "lo schema PROJJSON 0.7 non si compila")?,
    })
}

fn validatori() -> Result<&'static Validatori, PlenoraIoError> {
    // `map_err` e non un ripiego: il testo dell'errore interno **non** entra nel
    // messaggio pubblico -- e' nostro, non di chi legge il file, e non lo
    // aiuterebbe -- ma l'esito resta un errore, non un valore di comodo.
    VALIDATORI.get_or_init(compila).as_ref().map_err(|_| {
        non_conforme(&PublicMessage::Curated(
            "gli schemi `GeoParquet` incorporati non sono utilizzabili",
        ))
    })
}

/// Il documento e' un PROJJSON 0.7 valido?
///
/// E' la stessa autorita' che la 1.1.0 raggiunge per `$ref`, interrogata
/// direttamente. `Ok(false)` vuol dire «non e' PROJJSON»; l'errore e' riservato
/// al caso in cui gli schemi incorporati non siano utilizzabili, e allora si
/// fallisce chiusi come altrove.
///
/// # Errors
///
/// `Format` se gli schemi incorporati non sono utilizzabili.
pub fn e_projjson(documento: &Value) -> Result<bool, PlenoraIoError> {
    Ok(validatori()?.projjson.is_valid(documento))
}

/// Il documento rispetta lo schema ufficiale della versione che dichiara?
///
/// # Errors
///
/// `Format` se il documento non rispetta lo schema, o se gli schemi
/// incorporati non sono utilizzabili -- nel secondo caso si fallisce chiusi:
/// senza autorita' non si valida, e senza validare non si accetta.
pub fn valida(documento: &Value, versione: &str) -> Result<(), PlenoraIoError> {
    let pronti = validatori()?;
    let validatore = match versione {
        "1.0.0" => &pronti.v1_0_0,
        "1.1.0" => &pronti.v1_1_0,
        // Non accade: `metadati` restringe la versione prima di arrivare qui.
        // Se accadesse, non validare sarebbe peggio che rifiutare.
        _ => {
            return Err(non_conforme(&PublicMessage::CuratedPair(
                "nessuno schema GeoParquet incorporato per quella versione: sono incorporate",
                "1.0.0 e 1.1.0",
            )))
        }
    };
    if validatore.is_valid(documento) {
        return Ok(());
    }
    // Si dice **quale** schema ha rifiutato, e nient'altro: il percorso
    // dell'errore e il valore che non andava bene verrebbero dal file, e un
    // messaggio pubblico non porta cio' che ha letto.
    Err(non_conforme(&PublicMessage::CuratedPair(
        "metadato `geo` che non rispetta lo schema ufficiale GeoParquet",
        match versione {
            "1.0.0" => "1.0.0",
            _ => "1.1.0",
        },
    )))
}

#[cfg(test)]
mod sonde;

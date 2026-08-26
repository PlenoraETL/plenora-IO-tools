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
    })
}

fn validatori() -> Result<&'static Validatori, PlenoraIoError> {
    match VALIDATORI.get_or_init(compila) {
        Ok(pronti) => Ok(pronti),
        // Il testo dell'errore interno **non** entra nel messaggio pubblico: e'
        // nostro, non di chi legge il file, e non lo aiuterebbe.
        Err(_) => Err(non_conforme(&PublicMessage::Curated(
            "gli schemi GeoParquet incorporati non sono utilizzabili",
        ))),
    }
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
mod sonde {
    use super::*;
    use serde_json::json;

    fn documento_valido() -> Value {
        json!({
            "version": "1.1.0",
            "primary_column": "geometry",
            "columns": {"geometry": {"encoding": "WKB", "geometry_types": ["Point"]}},
        })
    }

    #[test]
    fn gli_schemi_incorporati_si_compilano() {
        // La controprova che regge tutto il resto: se non si compilassero, ogni
        // validazione fallirebbe chiusa e nessuna sonda distinguerebbe «rifiuta
        // il documento» da «non ha lo schema».
        assert!(validatori().is_ok());
    }

    #[test]
    fn un_documento_conforme_e_accettato() {
        assert!(valida(&documento_valido(), "1.1.0").is_ok());
    }

    #[test]
    fn lo_schema_rifiuta_cio_che_la_specifica_rifiuta() {
        // Ognuno di questi e' un caso che la prima stesura del modulo
        // `metadati` **accettava**, e che lo schema ufficiale non ammette. E'
        // la ragione per cui l'autorita' sta qui e non nella nostra prosa.
        let casi = [
            // `" M"` non esiste nel pattern.
            json!(["Point M"]),
            json!(["Point ZM"]),
        ];
        for tipi in casi {
            let mut documento = documento_valido();
            documento["columns"]["geometry"]["geometry_types"] = tipi;
            assert!(valida(&documento, "1.1.0").is_err(), "{documento}");
        }

        // `uniqueItems: true`.
        let mut ripetuto = documento_valido();
        ripetuto["columns"]["geometry"]["geometry_types"] = json!(["Point", "Point"]);
        assert!(valida(&ripetuto, "1.1.0").is_err());

        // `covering` con percorsi di un segmento solo.
        let mut piatto = documento_valido();
        piatto["columns"]["geometry"]["covering"] = json!({"bbox": {
            "xmin": ["_bbox_minx"],
            "ymin": ["_bbox_miny"],
            "xmax": ["_bbox_maxx"],
            "ymax": ["_bbox_maxy"],
        }});
        assert!(valida(&piatto, "1.1.0").is_err());

        // `covering` senza `bbox`.
        let mut senza = documento_valido();
        senza["columns"]["geometry"]["covering"] = json!({"altro": {}});
        assert!(valida(&senza, "1.1.0").is_err());
    }

    #[test]
    fn il_covering_conforme_e_accettato() {
        let mut documento = documento_valido();
        documento["columns"]["geometry"]["covering"] = json!({"bbox": {
            "xmin": ["bbox", "xmin"],
            "ymin": ["bbox", "ymin"],
            "xmax": ["bbox", "xmax"],
            "ymax": ["bbox", "ymax"],
        }});
        assert!(valida(&documento, "1.1.0").is_ok());
    }

    #[test]
    fn le_codifiche_native_sono_valide_in_1_1_e_non_in_1_0() {
        // E' lo schema a dirlo, non noi: in 1.0.0 `encoding` e' `const: "WKB"`,
        // in 1.1.0 e' un pattern che ammette anche le native.
        let mut nativa = documento_valido();
        nativa["columns"]["geometry"]["encoding"] = json!("point");
        assert!(valida(&nativa, "1.1.0").is_ok());

        nativa["version"] = json!("1.0.0");
        assert!(valida(&nativa, "1.0.0").is_err());
    }

    #[test]
    fn il_crs_e_validato_contro_il_projjson_referenziato() {
        // E' la prova che il `$ref` si risolve davvero dal registro in memoria:
        // senza registro la compilazione dello schema fallirebbe, e senza
        // risoluzione un `crs` qualunque passerebbe.
        let mut valido = documento_valido();
        valido["columns"]["geometry"]["crs"] = json!({
            "$schema": ID_PROJJSON_0_7,
            "type": "GeographicCRS",
            "name": "WGS 84 (CRS84)",
            "datum": {
                "type": "GeodeticReferenceFrame",
                "name": "World Geodetic System 1984",
                "ellipsoid": {
                    "name": "WGS 84",
                    "semi_major_axis": 6_378_137,
                    "inverse_flattening": 298.257_223_563
                }
            },
            "coordinate_system": {
                "subtype": "ellipsoidal",
                "axis": [
                    {"name": "Geodetic longitude", "abbreviation": "Lon", "direction": "east", "unit": "degree"},
                    {"name": "Geodetic latitude", "abbreviation": "Lat", "direction": "north", "unit": "degree"}
                ]
            }
        });
        assert!(valida(&valido, "1.1.0").is_ok(), "un PROJJSON valido passa");

        // `null` e' l'altra meta' dell'`oneOf`.
        let mut nullo = documento_valido();
        nullo["columns"]["geometry"]["crs"] = json!(null);
        assert!(valida(&nullo, "1.1.0").is_ok());

        // Un oggetto qualunque **non** e' PROJJSON: e' il caso che il modulo
        // `metadati` accettava chiamandolo «oggetto PROJJSON», e che solo lo
        // schema referenziato sa rifiutare.
        let mut finto = documento_valido();
        finto["columns"]["geometry"]["crs"] = json!({"id": {"authority": "EPSG", "code": 4326}});
        assert!(
            valida(&finto, "1.1.0").is_err(),
            "un oggetto con il solo `id` non e' un documento PROJJSON"
        );
    }

    #[test]
    fn il_rifiuto_non_dice_niente_del_documento() {
        // Il percorso dell'errore e il valore che non andava bene verrebbero
        // dal file: un messaggio pubblico non porta cio' che ha letto.
        let mut segreto = documento_valido();
        segreto["primary_column"] = json!("");
        segreto["columns"] = json!({"colonna-riservata-del-cliente": {}});
        let errore = valida(&segreto, "1.1.0").expect_err("non conforme");
        assert!(!errore.message.contains("colonna-riservata-del-cliente"));
        assert!(errore.message.contains("schema ufficiale"));
    }
}

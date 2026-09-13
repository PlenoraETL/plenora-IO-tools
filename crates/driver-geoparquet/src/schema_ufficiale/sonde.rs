//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;
use serde_json::json;

fn documento_valido() -> Value {
    json!({
        "version": "1.1.0",
        "primary_column": "geometry",
        "columns": {"geometry": {"encoding": "WKB", "geometry_types": ["Point"]}},
    })
}

/// Il documento di riferimento, piu' un campo della colonna geometrica.
fn con(campo: &str, valore: Value) -> Value {
    let mut d = documento_valido();
    d["columns"]["geometry"][campo] = valore;
    d
}

/// Lo stesso, in una versione dichiarata.
fn con_in(versione: &str, campo: &str, valore: Value) -> Value {
    let mut d = con(campo, valore);
    d["version"] = json!(versione);
    d
}

fn senza(campo: &str) -> Value {
    let mut d = documento_valido();
    d.as_object_mut().expect("oggetto").remove(campo);
    d
}

/// Un PROJJSON 0.7 che lo schema referenziato accetta.
fn projjson_valido() -> Value {
    json!({
        "$schema": "https://proj.org/schemas/v0.7/projjson.schema.json",
        "type": "GeographicCRS",
        "name": "WGS 84 (CRS84)",
        "datum_ensemble": {
            "name": "World Geodetic System 1984 ensemble",
            "members": [{"name": "World Geodetic System 1984 (G2296)"}],
            "accuracy": "2.0"
        },
        "coordinate_system": {
            "subtype": "ellipsoidal",
            "axis": [
                {"name": "Geodetic longitude", "abbreviation": "Lon",
                 "direction": "east", "unit": "degree"},
                {"name": "Geodetic latitude", "abbreviation": "Lat",
                 "direction": "north", "unit": "degree"}
            ]
        },
        "id": {"authority": "OGC", "code": "CRS84"}
    })
}

/// Il verdetto atteso, caso per caso.
///
/// # Perche' i verdetti stanno in tabella
///
/// L'autorita' che li decide non e' nostra: sono gli schemi ufficiali,
/// interpretati da `jsonschema`. Aggiornare quella libreria puo' cambiare
/// un verdetto senza toccare una riga di codice nostro e senza far fallire
/// niente -- un documento in piu' accettato non rompe nessun test che non
/// lo nomini.
///
/// Queste tabelle sono state usate come banco differenziale
/// nell'aggiornamento da `jsonschema` 0.51.0 a 0.55.1, il 2026-09-09:
/// gli stessi verdetti prima e dopo, caso per caso. Restano qui perche' il
/// prossimo aggiornamento trovi la domanda gia' posta.
fn verdetti(casi: &[(&str, Value, &str, bool)]) {
    for (nome, documento, versione, atteso) in casi {
        assert_eq!(
            valida(documento, versione).is_ok(),
            *atteso,
            "{nome}: lo schema ufficiale ha cambiato verdetto"
        );
    }
}

#[test]
fn verdetti_sulle_codifiche() {
    // Le codifiche native esistono dalla 1.1.0 in poi; `WKB` da sempre.
    verdetti(&[
        ("conforme 1.1.0", documento_valido(), "1.1.0", true),
        (
            "WKB in 1.0.0",
            con_in("1.0.0", "encoding", json!("WKB")),
            "1.0.0",
            true,
        ),
        (
            "point in 1.0.0",
            con_in("1.0.0", "encoding", json!("point")),
            "1.0.0",
            false,
        ),
        (
            "point in 1.1.0",
            con_in("1.1.0", "encoding", json!("point")),
            "1.1.0",
            true,
        ),
        (
            "multipolygon in 1.0.0",
            con_in("1.0.0", "encoding", json!("multipolygon")),
            "1.0.0",
            false,
        ),
        (
            "multipolygon in 1.1.0",
            con_in("1.1.0", "encoding", json!("multipolygon")),
            "1.1.0",
            true,
        ),
        // `WKT` non e' una codifica GeoParquet, e `wkb` minuscolo non e' `WKB`.
        (
            "WKT in 1.1.0",
            con_in("1.1.0", "encoding", json!("WKT")),
            "1.1.0",
            false,
        ),
        (
            "wkb minuscolo",
            con_in("1.1.0", "encoding", json!("wkb")),
            "1.1.0",
            false,
        ),
        ("encoding vuota", con("encoding", json!("")), "1.1.0", false),
    ]);
}

#[test]
fn verdetti_sui_tipi_geometrici() {
    // Il pattern della specifica: lo spazio e' singolo, e `M` da solo non esiste.
    verdetti(&[
        (
            "Point",
            con("geometry_types", json!(["Point"])),
            "1.1.0",
            true,
        ),
        (
            "Point Z",
            con("geometry_types", json!(["Point Z"])),
            "1.1.0",
            true,
        ),
        (
            "Point M",
            con("geometry_types", json!(["Point M"])),
            "1.1.0",
            false,
        ),
        (
            "Point ZM",
            con("geometry_types", json!(["Point ZM"])),
            "1.1.0",
            false,
        ),
        (
            "point minuscolo",
            con("geometry_types", json!(["point"])),
            "1.1.0",
            false,
        ),
        (
            "due spazi",
            con("geometry_types", json!(["Point  Z"])),
            "1.1.0",
            false,
        ),
        (
            "Curve",
            con("geometry_types", json!(["Curve"])),
            "1.1.0",
            false,
        ),
        (
            "GeometryCollection",
            con("geometry_types", json!(["GeometryCollection"])),
            "1.1.0",
            true,
        ),
        (
            "tipo vuoto",
            con("geometry_types", json!([""])),
            "1.1.0",
            false,
        ),
    ]);
}

#[test]
fn verdetti_sulla_forma_del_documento() {
    let mut numerico = documento_valido();
    numerico["primary_column"] = json!(7);
    let mut lista = documento_valido();
    lista["columns"] = json!([]);
    let mut dieci = documento_valido();
    dieci["version"] = json!("1.0.0");

    verdetti(&[
        ("conforme 1.0.0", dieci, "1.0.0", true),
        (
            "senza primary_column",
            senza("primary_column"),
            "1.1.0",
            false,
        ),
        ("senza columns", senza("columns"), "1.1.0", false),
        ("senza version", senza("version"), "1.1.0", false),
        ("primary_column numerico", numerico, "1.1.0", false),
        ("columns come lista", lista, "1.1.0", false),
        // I facoltativi con una forma dichiarata.
        (
            "bbox a quattro",
            con("bbox", json!([0.0, 0.0, 1.0, 1.0])),
            "1.1.0",
            true,
        ),
        (
            "bbox a tre",
            con("bbox", json!([0.0, 0.0, 1.0])),
            "1.1.0",
            false,
        ),
        (
            "orientation ammessa",
            con("orientation", json!("counterclockwise")),
            "1.1.0",
            true,
        ),
        (
            "orientation inventata",
            con("orientation", json!("clockwise")),
            "1.1.0",
            false,
        ),
        (
            "edges spherical",
            con("edges", json!("spherical")),
            "1.1.0",
            true,
        ),
        (
            "edges geodesic",
            con("edges", json!("geodesic")),
            "1.1.0",
            false,
        ),
        // Le versioni fuori dalle due incorporate.
        ("versione 0.4.0", documento_valido(), "0.4.0", false),
        ("versione 1.2.0", documento_valido(), "1.2.0", false),
        ("versione vuota", documento_valido(), "", false),
    ]);
}

/// I verdetti che passano per il `$ref` allo schema PROJJSON incorporato.
///
/// E' la via che un aggiornamento di `referencing` puo' rompere in
/// silenzio, e la prova pretende che accetti **e** che rifiuti: se il
/// `$ref` smettesse di risolversi, un `crs` qualunque diventerebbe valido
/// e solo la riga «PROJJSON invalido» se ne accorgerebbe.
#[test]
fn verdetti_sul_crs_referenziato() {
    verdetti(&[
        ("crs null", con("crs", json!(null)), "1.1.0", true),
        (
            "crs PROJJSON valido",
            con("crs", projjson_valido()),
            "1.1.0",
            true,
        ),
        (
            "crs PROJJSON invalido",
            con("crs", json!({"type": "NonEsiste", "name": 3})),
            "1.1.0",
            false,
        ),
        (
            "crs come stringa",
            con("crs", json!("EPSG:4326")),
            "1.1.0",
            false,
        ),
    ]);
    // La stessa autorita', interrogata direttamente.
    assert!(e_projjson(&projjson_valido()).expect("gli schemi si compilano"));
    assert!(!e_projjson(&json!(7)).expect("gli schemi si compilano"));
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

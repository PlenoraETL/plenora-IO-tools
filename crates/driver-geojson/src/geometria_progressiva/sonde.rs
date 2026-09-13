//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::{analizza, componenti_usati};
use plenora_io_model::limits::WkbLimits;
use plenora_io_model::wkb::{encode_wkb, inspect_wkb, WkbFlavor};
use plenora_io_model::IoErrorCode;

fn stretti(componenti: usize, profondita: usize) -> WkbLimits {
    WkbLimits {
        max_components: componenti,
        max_depth: profondita,
        ..WkbLimits::default()
    }
}

/// Una `LineString` con il numero di posizioni indicato.
fn linea(posizioni: usize) -> String {
    let mut testo = String::from(r#"{"type":"LineString","coordinates":["#);
    for indice in 0..posizioni {
        if indice > 0 {
            testo.push(',');
        }
        testo.push_str("[1,2]");
    }
    testo.push_str("]}");
    testo
}

/// **Il fatto che il lotto S12 esiste per stabilire, per il `GeoJSON`.**
///
/// Il cap in byte diceva quanto puo' essere lungo l'input;
/// `serde_json::from_str::<geojson::Geometry>` costruiva l'albero intero
/// prima che un contatore lo vedesse. Qui il rifiuto arriva alla posizione
/// che supera il tetto, e la coda del testo non viene nemmeno letta.
#[test]
fn il_rifiuto_arriva_prima_della_fine_del_testo() {
    let testo = linea(5_000);
    let limiti = stretti(10, 64);
    let errore = analizza(&testo, &limiti).expect_err("il tetto deve fermare l'analisi");
    assert_eq!(errore.code, IoErrorCode::LimitExceeded);

    // La prova che non e' arrivata in fondo: la stessa linea con una coda
    // che non e' JSON. Se l'analisi leggesse tutto prima di misurare, il
    // rifiuto sarebbe di sintassi.
    let mut con_coda = linea(5_000);
    con_coda.push_str(" questa coda non e' JSON");
    let errore = analizza(&con_coda, &limiti).expect_err("il tetto deve fermare l'analisi");
    assert_eq!(
        errore.code,
        IoErrorCode::LimitExceeded,
        "atteso il rifiuto del tetto, non quello della coda"
    );
}

/// I tre rifiuti sono tre, e si distinguono.
///
/// Dire «limite superato» a chi ha scritto `"type": "Punto"` lo manderebbe
/// ad allargare una quota che non c'entra.
#[test]
fn i_rifiuti_portano_il_codice_della_loro_causa() {
    let limiti = WkbLimits::default();
    for (testo, atteso, perche) in [
        (
            r#"{"type":"Point","coordinates":[1,2]"#,
            IoErrorCode::Format,
            "JSON troncato",
        ),
        (
            r#"{"coordinates":[1,2]}"#,
            IoErrorCode::Format,
            "type assente",
        ),
        (
            r#"{"type":"Punto","coordinates":[1,2]}"#,
            IoErrorCode::Format,
            "type sconosciuto",
        ),
        (
            r#"{"type":"LineString","coordinates":[[1,2],[1,2,3]]}"#,
            IoErrorCode::Format,
            "dimensionalita' non uniforme",
        ),
        (
            r#"{"type":"Point","coordinates":[1,2,3,4]}"#,
            IoErrorCode::Format,
            "posizione con quattro ordinate",
        ),
        (
            r#"{"type":"Point","coordinates":[1,2]} e poi altro"#,
            IoErrorCode::Format,
            "testo residuo dopo la geometria",
        ),
    ] {
        let errore = analizza(testo, &limiti).expect_err(perche);
        assert_eq!(errore.code, atteso, "{perche}: {testo}");
    }

    // Il tetto, che e' un'altra cosa.
    let errore = analizza(&linea(50), &stretti(10, 64)).expect_err("tetto sui componenti");
    assert_eq!(errore.code, IoErrorCode::LimitExceeded);

    let annidato = r#"{"type":"GeometryCollection","geometries":[{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]}]}]}"#;
    let errore = analizza(annidato, &stretti(1_000, 1)).expect_err("tetto sull'annidamento");
    assert_eq!(errore.code, IoErrorCode::LimitExceeded);
    assert!(analizza(annidato, &stretti(1_000, 4)).is_ok());
}

/// Il tetto sui componenti, provato **esattamente al confine**.
///
/// Il target di fuzzing non arriva a centomila posizioni -- non stanno nel
/// cap del harness -- quindi sotto fuzzing quel ramo non e' esercitato, e
/// il registro della misura lo dice. Qui si prova al confine: `n` passa,
/// `n+1` no.
#[test]
fn il_tetto_sui_componenti_e_esatto() {
    let casi: [(&str, usize); 5] = [
        (r#"{"type":"Point","coordinates":[1,2]}"#, 1),
        (
            r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,2]]}"#,
            3,
        ),
        (
            r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}"#,
            4,
        ),
        (r#"{"type":"MultiPoint","coordinates":[[0,0],[1,1]]}"#, 4),
        (
            r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]},{"type":"LineString","coordinates":[[0,0],[1,1]]}]}"#,
            5,
        ),
    ];
    for (testo, costo) in casi {
        let esatto = WkbLimits {
            max_components: costo,
            ..WkbLimits::default()
        };
        let stretto = WkbLimits {
            max_components: costo - 1,
            ..WkbLimits::default()
        };
        assert!(
            analizza(testo, &esatto).is_ok(),
            "{testo} costa {costo} componenti e con {costo} deve passare"
        );
        let errore = analizza(testo, &stretto)
            .expect_err(&format!("{testo} con {} deve fallire", costo - 1));
        assert_eq!(
            errore.code,
            IoErrorCode::LimitExceeded,
            "{testo}: al confine il rifiuto e' del tetto"
        );
        assert_eq!(componenti_usati(testo, &esatto), costo, "{testo}");
    }
}

/// La soglia oltre la quale rifiuta **serde**, misurata e non dedotta.
///
/// Il limite effettivo sull'annidamento e' il minimo fra il nostro
/// `max_depth` e il tetto di ricorsione di `serde_json`. Sono due rifiuti
/// diversi e portano due codici diversi: attribuire al nostro tetto un
/// rifiuto prodotto da serde sarebbe falso, e questa sonda e' il posto in
/// cui la differenza resta scritta.
#[test]
fn oltre_una_certa_profondita_rifiuta_serde_e_non_noi() {
    let annidata = |livelli: usize| {
        let mut geometria = r#"{"type":"Point","coordinates":[1,2]}"#.to_owned();
        for _ in 0..livelli {
            geometria = format!(r#"{{"type":"GeometryCollection","geometries":[{geometria}]}}"#);
        }
        geometria
    };
    // Con un tetto largo il nostro non morde: a rifiutare e' serde, e
    // l'errore e' di formato.
    let largo = WkbLimits {
        max_depth: 1_000,
        ..WkbLimits::default()
    };
    assert!(
        analizza(&annidata(40), &largo).is_ok(),
        "quaranta livelli passano"
    );
    let errore = analizza(&annidata(200), &largo).expect_err("serde rifiuta");
    assert_eq!(
        errore.code,
        IoErrorCode::Format,
        "il rifiuto di serde e' un errore di formato, non un nostro tetto"
    );

    // Con un tetto stretto mordiamo noi, e il codice lo dice.
    let stretto = WkbLimits {
        max_depth: 8,
        ..WkbLimits::default()
    };
    let errore = analizza(&annidata(20), &stretto).expect_err("il nostro tetto");
    assert_eq!(errore.code, IoErrorCode::LimitExceeded);
}

/// Il messaggio curato del rifiuto della lista vuota, e quello generico
/// della sintassi. Stanno qui, nominati, perche' la sonda della lista vuota
/// li **confronta**: portano lo stesso `IoErrorCode`, e cio' che li separa
/// e' soltanto il testo.
const MESSAGGIO_LISTA_VUOTA: &str = "coordinates GeoJSON con una lista vuota";
const MESSAGGIO_GENERICO: &str = "geometria GeoJSON non valida";

/// L'albero delle coordinate non accumula prima di addebitare.
///
/// Era il buco: una lista di soli numeri costava un componente qualunque
/// fosse la sua lunghezza, e a fermarla restava il solo cap in byte. Ora
/// una lista e' o una posizione -- al piu' quattro ordinate -- o una lista
/// di liste, dove ogni figlia si e' gia' addebitata.
#[test]
fn una_lista_di_numeri_non_cresce_oltre_una_posizione() {
    let limiti = WkbLimits::default();

    // Mille ordinate in una posizione: rifiutate dopo averne lette cinque.
    let mut ordinate = String::from(r#"{"type":"Point","coordinates":["#);
    for indice in 0..1_000 {
        if indice > 0 {
            ordinate.push(',');
        }
        ordinate.push('1');
    }
    ordinate.push_str("]}");
    let errore = analizza(&ordinate, &limiti).expect_err("non e' una posizione");
    assert_eq!(errore.code, IoErrorCode::Format);

    // Numeri e liste nella stessa lista non sono una forma valida in
    // nessuno dei due confini, e riconoscerlo subito costa un confronto.
    let misto = r#"{"type":"LineString","coordinates":[1,2,[3,4]]}"#;
    assert_eq!(
        analizza(misto, &limiti).expect_err("mista").code,
        IoErrorCode::Format
    );
    assert!(come_prima(misto).is_err(), "e il confine precedente pure");
}

/// Nemmeno una lista **vuota** entra nell'albero senza pagare.
///
/// Era la terza via dell'amplificatore, nella variante «nodi strutturali
/// senza coordinate»: `[[],[],[],...]` costava zero componenti e cresceva
/// finche' il cap in byte non lo fermava -- cioe' dopo aver costruito tutto
/// l'albero.
///
/// La coda non e' JSON, ed e' li' la prova: se l'analisi arrivasse in fondo
/// prima di rifiutare, il rifiuto sarebbe di sintassi. Siccome si ferma
/// alla **prima** lista vuota, la coda non viene nemmeno letta.
#[test]
fn una_lista_vuota_ferma_l_analisi_alla_prima() {
    let limiti = WkbLimits::default();
    let mut testo = String::from(r#"{"type":"MultiPolygon","coordinates":["#);
    for indice in 0..20_000 {
        if indice > 0 {
            testo.push(',');
        }
        testo.push_str("[]");
    }
    testo.push_str("], questa coda non e' JSON e non deve essere letta");

    let errore = analizza(&testo, &limiti).expect_err("la lista vuota ferma l'analisi");
    assert_eq!(errore.code, IoErrorCode::Format);
    // Il **codice** non basta a distinguere i due rifiuti: la coda
    // malformata porta anche lei `Format`, quindi una sonda che guardasse
    // solo quello resterebbe verde togliendo il rifiuto anticipato -- cioe'
    // proprio nel caso che deve cogliere. A distinguerli e' il messaggio
    // curato, ed e' quello che si pretende.
    assert!(
        errore.message.contains(MESSAGGIO_LISTA_VUOTA),
        "atteso il rifiuto della lista vuota, non quello della coda: {}",
        errore.message
    );

    // La controprova che rende la riga sopra una discriminazione e non una
    // formula: **la stessa coda**, dietro liste che vuote non sono. Qui
    // l'analisi arriva in fondo, e il rifiuto e' quello generico di sintassi
    // -- messaggio diverso, stesso codice.
    let mut valido = String::from(r#"{"type":"MultiPolygon","coordinates":["#);
    for indice in 0..20_000 {
        if indice > 0 {
            valido.push(',');
        }
        valido.push_str("[[[0,0],[1,0],[1,1],[0,0]]]");
    }
    valido.push_str("], questa coda non e' JSON e non deve essere letta");
    let di_sintassi = analizza(&valido, &limiti).expect_err("la coda non e' JSON");
    assert_eq!(di_sintassi.code, IoErrorCode::Format);
    assert!(
        di_sintassi.message.contains(MESSAGGIO_GENERICO),
        "la coda da' il rifiuto generico: {}",
        di_sintassi.message
    );
    assert!(
        !di_sintassi.message.contains(MESSAGGIO_LISTA_VUOTA),
        "i due messaggi devono essere distinguibili"
    );

    // E il confine precedente la rifiutava gia', quindi l'insieme accettato
    // non cambia: cambia **quando** ci si ferma.
    for testo in [
        r#"{"type":"MultiPolygon","coordinates":[[]]}"#,
        r#"{"type":"Polygon","coordinates":[[]]}"#,
        r#"{"type":"LineString","coordinates":[[]]}"#,
    ] {
        assert!(analizza(testo, &limiti).is_err(), "{testo}");
        assert!(come_prima(testo).is_err(), "{testo}: e prima pure");
    }
}

/// L'unita' di conteggio e' quella del bordo, e non «una simile».
///
/// Stessa sonda del WKT progressivo, stesso confronto: cio' che l'analisi
/// addebita leggendo il testo deve essere cio' che `inspect_wkb` conta
/// sulla stessa geometria in WKB.
#[test]
fn i_componenti_coincidono_con_quelli_del_parser_condiviso() {
    let limiti = WkbLimits::default();
    for testo in [
        r#"{"type":"Point","coordinates":[1,2]}"#,
        r#"{"type":"Point","coordinates":[1,2,3]}"#,
        r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,2]]}"#,
        r#"{"type":"MultiPoint","coordinates":[[0,0],[1,1]]}"#,
        r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}"#,
        r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]],[[0,0],[1,0],[1,1],[0,0]]]}"#,
        r#"{"type":"MultiLineString","coordinates":[[[0,0],[1,1]],[[2,2],[3,3]]]}"#,
        r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]}"#,
        r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]},{"type":"LineString","coordinates":[[0,0],[1,1]]}]}"#,
    ] {
        let nostri = componenti_usati(testo, &limiti);
        let geometria = analizza(testo, &limiti).expect("campione valido");
        let byte = encode_wkb(&geometria, WkbFlavor::Iso).expect("codificabile");
        let ispezione = inspect_wkb(&byte, &limiti).expect("ispezionabile");
        assert_eq!(
            nostri, ispezione.components,
            "{testo}: l'analisi addebita {nostri}, il parser condiviso conta {}",
            ispezione.components
        );
    }
}

/// **La prova che la grammatica non e' cambiata.**
///
/// Come per il WKT: il confronto e' con il confine **precedente** -- la
/// deserializzazione in `geojson::Geometry` piu' `convert` -- su un corpus
/// generato. La regola e' la stessa, e la riga che conta e' l'ultima:
/// accettare piu' di prima e' la regressione che nessun test esistente
/// vedrebbe.
///
/// # Qui non c'e' eccezione, e non e' un caso
///
/// Il WKT ne ha una -- il testo dopo la geometria, che la crate `wkt`
/// ignorava -- e il `GeoJSON` no: `serde_json::from_str` pretende gia' che
/// l'input sia **un** valore e nient'altro, quindi la stessa scelta era
/// gia' quella del confine precedente. L'analisi progressiva la conserva
/// chiamando `end()` sul deserializzatore, e i due insiemi coincidono
/// esattamente: nessuna divergenza, in nessuna delle due direzioni.
#[test]
fn accetta_esattamente_cio_che_accettava_il_confine_precedente() {
    let limiti = WkbLimits::default();
    let corpus = corpus_di_confronto();
    assert!(corpus.len() > 80, "corpus troppo piccolo: {}", corpus.len());

    for testo in &corpus {
        let prima = come_prima(testo);
        let dopo = analizza(testo, &limiti);
        match (prima, dopo) {
            (Ok(prima), Ok(dopo)) => {
                assert_eq!(prima, dopo, "geometrie diverse per «{testo}»");
            }
            (Err(_), Err(_)) => {}
            (Ok(_), Err(errore)) => {
                panic!("«{testo}» era accettato e ora e' rifiutato: {errore}");
            }
            (Err(errore), Ok(_)) => {
                panic!("«{testo}» era rifiutato ({errore}) e ora e' accettato");
            }
        }
    }
}

/// Il confine come era prima del lotto S12.
fn come_prima(testo: &str) -> plenora_io_model::Result<plenora_io_model::wkb::WkbGeometry> {
    let gj: geojson::Geometry = serde_json::from_str(testo).map_err(|_| {
        crate::geometry::format_error(&plenora_io_model::PublicMessage::Curated(
            "geometria GeoJSON non valida",
        ))
    })?;
    crate::geometry::converti_per_confronto(&gj.value)
}

/// Il corpus, generato per combinazione.
fn corpus_di_confronto() -> Vec<String> {
    let mut corpus = Vec::new();
    // Le negative ci sono per una ragione precisa: `serde_json` consegna
    // un intero non negativo come `u64` e uno negativo come `i64`, quindi
    // senza di loro meta' del visitor non veniva mai eseguita -- e le
    // longitudini negative sono la meta' del mondo. L'ha trovato la
    // copertura delle righe cambiate, non questa sonda.
    let posizioni = ["[1,2]", "[1,2,3]", "[-1,-2]", "[-1.5,2.5,-3]"];
    for posizione in posizioni {
        let due = format!("{posizione},{posizione}");
        let quattro = format!("{posizione},{posizione},{posizione},{posizione}");
        for forma in [
            format!(r#"{{"type":"Point","coordinates":{posizione}}}"#),
            format!(r#"{{"type":"MultiPoint","coordinates":[{due}]}}"#),
            r#"{"type":"MultiPoint","coordinates":[]}"#.to_owned(),
            format!(r#"{{"type":"LineString","coordinates":[{due}]}}"#),
            r#"{"type":"LineString","coordinates":[]}"#.to_owned(),
            format!(r#"{{"type":"Polygon","coordinates":[[{quattro}]]}}"#),
            format!(r#"{{"type":"Polygon","coordinates":[[{quattro}],[{quattro}]]}}"#),
            r#"{"type":"Polygon","coordinates":[]}"#.to_owned(),
            format!(r#"{{"type":"MultiLineString","coordinates":[[{due}],[{due}]]}}"#),
            r#"{"type":"MultiLineString","coordinates":[]}"#.to_owned(),
            format!(r#"{{"type":"MultiPolygon","coordinates":[[[{quattro}]]]}}"#),
            r#"{"type":"MultiPolygon","coordinates":[]}"#.to_owned(),
            format!(
                r#"{{"type":"GeometryCollection","geometries":[{{"type":"Point","coordinates":{posizione}}}]}}"#
            ),
            r#"{"type":"GeometryCollection","geometries":[]}"#.to_owned(),
        ] {
            corpus.push(forma.clone());
            // Le stesse forme con le chiavi in ordine inverso: in JSON non
            // hanno ordine, e il confine nuovo deve reggerlo.
            corpus.push(
                forma
                    .replace(r#"{"type":"#, r#"{"XXtype":"#)
                    .replace(r#","coordinates":"#, r#","type":"#)
                    .replace(r#"{"XXtype":"#, r#"{"coordinates":"#),
            );
            // E con un membro estraneo, che entrambi ignorano.
            corpus.push(forma.replacen('{', r#"{"bbox":[0,0,1,1],"#, 1));
        }
    }

    for storpiatura in [
        "",
        "null",
        "[]",
        "{}",
        r#"{"type":"Point"}"#,
        r#"{"coordinates":[1,2]}"#,
        r#"{"type":"Punto","coordinates":[1,2]}"#,
        r#"{"type":"Point","coordinates":[]}"#,
        r#"{"type":"Point","coordinates":[1]}"#,
        r#"{"type":"Point","coordinates":[1,2,3,4]}"#,
        r#"{"type":"Point","coordinates":[1,"due"]}"#,
        r#"{"type":"Point","coordinates":[[1,2]]}"#,
        r#"{"type":"LineString","coordinates":[[1,2],[1,2,3]]}"#,
        r#"{"type":"LineString","coordinates":[1,2]}"#,
        r#"{"type":"Polygon","coordinates":[[1,2]]}"#,
        r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2]},{"type":"Point","coordinates":[1,2,3]}]}"#,
        r#"{"type":"GeometryCollection","coordinates":[1,2]}"#,
        r#"{"type":"Point","coordinates":[1,2]} e poi altro"#,
        r#"{"type":"Point","coordinates":[1,2]}{"type":"Point","coordinates":[3,4]}"#,
        r#"{"type":"Point","coordinates":[1,2],"type":"LineString"}"#,
    ] {
        corpus.push(storpiatura.to_owned());
    }
    corpus
}

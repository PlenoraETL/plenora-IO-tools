//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::*;
use plenora_io_model::IoErrorCode;
use serde_json::json;

/// La colonna minima che la specifica ammette: i due campi obbligatori.
///
/// `geometry_types` vuoto e' legittimo e vuol dire «non vincolato»: e' il
/// caso che un writer usa quando non ha ispezionato i dati.
fn colonna_minima() -> serde_json::Value {
    json!({"encoding": "WKB", "geometry_types": []})
}

/// Un documento 1.1.0 con una sola colonna, `geometry`.
fn documento(colonna: &serde_json::Value) -> String {
    con_versione("1.1.0", colonna)
}

fn con_versione(versione: &str, colonna: &serde_json::Value) -> String {
    json!({
        "version": versione,
        "primary_column": "geometry",
        "columns": {"geometry": colonna},
    })
    .to_string()
}

/// Una colonna minima con un campo in piu'.
fn con_campo(campo: &str, valore: serde_json::Value) -> String {
    let mut colonna = colonna_minima();
    colonna[campo] = valore;
    documento(&colonna)
}

#[track_caller]
fn accettato(testo: &str) -> MetadatiGeo {
    analizza(testo, false).expect("il documento e' conforme e supportato")
}

#[track_caller]
fn non_conforme_con(testo: &str) -> PlenoraIoError {
    let errore = analizza(testo, false).expect_err("il documento non e' conforme");
    assert_eq!(
        errore.code,
        IoErrorCode::Format,
        "un documento non conforme e' un errore di formato: {}",
        errore.message
    );
    assert_eq!(errore.driver.as_deref(), Some("geoparquet"));
    errore
}

#[track_caller]
fn non_supportato_con(testo: &str) -> PlenoraIoError {
    let errore = analizza(testo, false).expect_err("la funzionalita' non e' supportata");
    assert_eq!(
        errore.code,
        IoErrorCode::Unsupported,
        "una funzionalita' valida e non implementata non e' un errore di formato: {}",
        errore.message
    );
    errore
}

// --- il documento --------------------------------------------------

#[test]
fn documento_minimo_e_accettato() {
    let letti = accettato(&documento(&colonna_minima()));
    assert_eq!(letti.versione, "1.1.0");
    assert_eq!(letti.nome_primaria, "geometry");
    assert!(letti.primaria.tipi.is_empty());
    assert!(letti.secondarie.is_empty());
    assert_eq!(letti.primaria.bordi, Bordi::Planari);
    assert_eq!(letti.primaria.crs, Crs::Assente);
    assert!(letti.primaria.covering.is_none());
}

#[test]
fn documento_che_non_e_json_o_non_e_un_oggetto_e_non_conforme() {
    for testo in ["", "{", "non json", "[]", "\"stringa\"", "7", "null"] {
        let errore = non_conforme_con(testo);
        assert!(errore.message.contains("geo"), "{}", errore.message);
    }
}

// --- version -------------------------------------------------------

#[test]
fn version_dei_due_schemi_ufficiali_e_accettata() {
    for versione in VERSIONI_SUPPORTATE {
        let letti = accettato(&con_versione(versione, &colonna_minima()));
        assert_eq!(letti.versione, versione);
    }
}

#[test]
fn version_di_un_altro_schema_e_valida_e_non_supportata() {
    // La distinzione e' il punto: questi documenti sono corretti, e
    // dichiarano una versione che non leggiamo. Mandare chi legge a
    // correggere un file che non ha niente che non va sarebbe il danno.
    for versione in [
        "0.4.0",
        "1.0.1",
        "1.1.1",
        "1.1",
        "1.2.0",
        "2.0.0",
        "1.0.0-rc1",
    ] {
        let errore = non_supportato_con(&con_versione(versione, &colonna_minima()));
        assert!(
            errore.message.contains("1.0.0 e 1.1.0"),
            "il rifiuto dice fin dove arriviamo: {}",
            errore.message
        );
    }
}

#[test]
fn version_assente_vuota_o_non_stringa_e_non_conforme() {
    for valore in [json!(null), json!(1.1), json!(""), json!(["1.1.0"])] {
        let testo = json!({
            "version": valore,
            "primary_column": "geometry",
            "columns": {"geometry": colonna_minima()},
        })
        .to_string();
        non_conforme_con(&testo);
    }
    let senza = json!({
        "primary_column": "geometry",
        "columns": {"geometry": colonna_minima()},
    })
    .to_string();
    assert!(non_conforme_con(&senza).message.contains("version"));
}

// --- primary_column e columns --------------------------------------

#[test]
fn primary_column_assente_vuota_o_non_stringa_e_non_conforme() {
    for valore in [json!(null), json!(""), json!(7), json!(["geometry"])] {
        let testo = json!({
            "version": "1.1.0",
            "primary_column": valore,
            "columns": {"geometry": colonna_minima()},
        })
        .to_string();
        non_conforme_con(&testo);
    }
    let senza = json!({
        "version": "1.1.0",
        "columns": {"geometry": colonna_minima()},
    })
    .to_string();
    assert!(non_conforme_con(&senza).message.contains("primary_column"));
}

#[test]
fn columns_assente_o_non_oggetto_e_non_conforme() {
    for valore in [json!(null), json!([]), json!("geometry")] {
        let testo = json!({
            "version": "1.1.0",
            "primary_column": "geometry",
            "columns": valore,
        })
        .to_string();
        non_conforme_con(&testo);
    }
    let senza = json!({"version": "1.1.0", "primary_column": "geometry"}).to_string();
    assert!(non_conforme_con(&senza).message.contains("columns"));
}

#[test]
fn primary_column_che_nomina_una_colonna_presente_e_accettata() {
    // Il verso positivo del campo: la colonna nominata c'e', e il nome che
    // esce e' quello che il documento dichiarava -- non uno indovinato.
    let testo = json!({
        "version": "1.1.0",
        "primary_column": "la_mia_geometria",
        "columns": {
            "la_mia_geometria": colonna_minima(),
            "geometry": colonna_minima(),
        },
    })
    .to_string();
    let letti = accettato(&testo);
    assert_eq!(letti.nome_primaria, "la_mia_geometria");
    assert!(letti.secondarie.contains_key("geometry"));
}

#[test]
fn primary_column_assente_da_columns_e_non_conforme() {
    let testo = json!({
        "version": "1.1.0",
        "primary_column": "geometry",
        "columns": {"altra": colonna_minima()},
    })
    .to_string();
    let errore = non_conforme_con(&testo);
    assert!(
        errore.message.contains("primary_column"),
        "{}",
        errore.message
    );
}

#[test]
fn columns_con_una_colonna_che_non_e_un_oggetto_e_non_conforme() {
    // La forma della colonna e' controllata, e nessuna sonda la esercitava:
    // il gate pretende i due versi **per campo**, e questa e' la forma del
    // contenitore, non di un campo. Una lacuna che l'elenco dei campi non
    // poteva vedere.
    for valore in [json!(7), json!("WKB"), json!([]), json!(null)] {
        let testo = json!({
            "version": "1.1.0",
            "primary_column": "geometry",
            "columns": {"geometry": valore},
        })
        .to_string();
        assert!(non_conforme_con(&testo)
            .message
            .contains("non e' un oggetto"));
    }
}

#[test]
fn columns_con_una_secondaria_malformata_e_non_conforme() {
    // Una colonna che non e' la primaria puo' essere letta da un
    // consumatore diverso: lasciarla passare malformata vorrebbe dire
    // validare solo cio' che usiamo noi.
    let testo = json!({
        "version": "1.1.0",
        "primary_column": "geometry",
        "columns": {
            "geometry": colonna_minima(),
            "altra": {"encoding": "WKB"},
        },
    })
    .to_string();
    assert!(non_conforme_con(&testo).message.contains("geometry_types"));
}

#[test]
fn columns_con_una_secondaria_valida_e_accettato() {
    let buono = json!({
        "version": "1.1.0",
        "primary_column": "geometry",
        "columns": {
            "geometry": colonna_minima(),
            "altra": colonna_minima(),
        },
    })
    .to_string();
    assert_eq!(accettato(&buono).secondarie.len(), 1);
}

// --- encoding ------------------------------------------------------

#[test]
fn encoding_wkb_e_accettato() {
    accettato(&con_campo("encoding", json!("WKB")));
}

#[test]
fn encoding_nativo_e_valido_e_non_supportato() {
    // Le codifiche native sono valide **da 1.1**: in un documento 1.1.0
    // sono una funzionalita' che non implementiamo...
    for nativa in CODIFICHE_NATIVE {
        let errore = non_supportato_con(&con_versione(
            "1.1.0",
            &json!({"encoding": nativa, "geometry_types": []}),
        ));
        assert!(errore.message.contains("WKB"), "{}", errore.message);
    }
    // ...e in un documento 1.0.0 non sono nemmeno valide, perche' quella
    // versione ammette solo WKB. E' la versione dichiarata a dire quale
    // delle due cose sono, ed e' il servizio che quel campo rende.
    for nativa in CODIFICHE_NATIVE {
        non_conforme_con(&con_versione(
            "1.0.0",
            &json!({"encoding": nativa, "geometry_types": []}),
        ));
    }
}

#[test]
fn encoding_assente_vuoto_o_sconosciuto_e_non_conforme() {
    for valore in [json!(null), json!(""), json!("wkb"), json!("WKT"), json!(7)] {
        non_conforme_con(&con_campo("encoding", valore));
    }
    let senza = documento(&json!({"geometry_types": []}));
    assert!(non_conforme_con(&senza).message.contains("encoding"));
}

// --- geometry_types ------------------------------------------------

#[test]
fn geometry_types_dall_insieme_chiuso_e_accettato() {
    let letti = accettato(&con_campo(
        "geometry_types",
        json!(["Point", "Point Z", "GeometryCollection", "MultiPolygon Z"]),
    ));
    assert_eq!(letti.primaria.tipi.len(), 4);
    assert_eq!(
        letti.primaria.tipi[0],
        (GeometryType::Point, CoordinateDimensions::Xy)
    );
    assert_eq!(
        letti.primaria.tipi[1],
        (GeometryType::Point, CoordinateDimensions::Xyz)
    );
}

#[test]
fn geometry_types_con_la_misura_m_e_non_conforme() {
    // Il pattern dello schema, in **entrambe** le versioni, e'
    // `^(GeometryCollection|(Multi)?(Point|LineString|Polygon))( Z)?$`:
    // `" M"` e `" ZM"` non esistono in GeoParquet.
    //
    // La prima stesura li ammetteva, e la ragione che ci aveva scritto
    // accanto -- «il nostro writer li emette» -- era il ragionamento
    // sbagliato: il writer emetteva metadati non conformi, e la
    // conclusione giusta era correggere il writer.
    for etichetta in ["Point M", "Point ZM", "LineString M", "MultiPolygon ZM"] {
        non_conforme_con(&con_campo("geometry_types", json!([etichetta])));
    }
}

#[test]
fn geometry_types_fuori_dalla_specifica_e_non_conforme() {
    // Era il difetto: `filter_map` scartava l'etichetta, il contratto della
    // colonna usciva piu' povero, e nulla lo diceva.
    for etichetta in [
        "Punto", "point", "Point X", "POINT", "Point  Z", "Curve", "",
    ] {
        let errore = non_conforme_con(&con_campo("geometry_types", json!([etichetta])));
        assert!(errore.message.contains("tipo geometrico"), "{etichetta}");
    }
}

#[test]
fn geometry_types_assente_o_non_elenco_di_stringhe_e_non_conforme() {
    for valore in [
        json!(null),
        json!("Point"),
        json!({}),
        json!([7]),
        json!([null]),
    ] {
        non_conforme_con(&con_campo("geometry_types", valore));
    }
    let senza = documento(&json!({"encoding": "WKB"}));
    assert!(non_conforme_con(&senza).message.contains("geometry_types"));
}

#[test]
fn geometry_types_ripetuto_e_non_conforme() {
    // `uniqueItems: true`. La prima stesura deduplicava in silenzio: cioe'
    // accettava un documento che lo schema rifiuta, e ne nascondeva la
    // ragione -- chi lo aveva scritto continuava a scriverlo.
    let errore = non_conforme_con(&con_campo("geometry_types", json!(["Point", "Point"])));
    assert!(errore.message.contains("ripetuto"), "{}", errore.message);
}

// --- crs -----------------------------------------------------------

#[test]
fn crs_assente_nullo_o_oggetto_e_accettato() {
    // Tre stati distinti, e la distinzione e' il rilievo: assente vuol dire
    // CRS84, `null` vuol dire che il CRS non c'e'. La prima stesura li
    // riduceva tutt'e due a «niente», e il driver li trasformava tutt'e due
    // in CRS84 -- mettendo in bocca a chi ha scritto il file
    // un'affermazione che non aveva fatto.
    assert_eq!(
        accettato(&documento(&colonna_minima())).primaria.crs,
        Crs::Assente
    );
    assert_eq!(
        accettato(&con_campo("crs", json!(null))).primaria.crs,
        Crs::Nullo
    );
    // Un PROJJSON **vero**: lo schema referenziato lo pretende completo, e
    // `{"type": ..., "name": ...}` da solo non lo e'. E' la differenza che
    // solo l'autorita' sa fare, e che la nostra prosa non sapeva.
    let letti = accettato(&con_campo(
        "crs",
        json!({
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
        }),
    ));
    assert!(matches!(letti.primaria.crs, Crs::Documento(_)));
}

#[test]
fn crs_che_non_e_un_oggetto_e_non_conforme() {
    // Una stringa li' vorrebbe dire che qualcuno ha scritto un WKT dove va
    // un documento PROJJSON.
    for valore in [json!("EPSG:4326"), json!(4326), json!([]), json!(true)] {
        non_conforme_con(&con_campo("crs", valore));
    }
}

// --- edges ---------------------------------------------------------

#[test]
fn edges_planari_o_assenti_sono_accettati() {
    assert_eq!(
        accettato(&documento(&colonna_minima())).primaria.bordi,
        Bordi::Planari
    );
    assert_eq!(
        accettato(&con_campo("edges", json!("planar")))
            .primaria
            .bordi,
        Bordi::Planari
    );
}

#[test]
fn edges_sferici_sono_validi_e_non_supportati() {
    // Validi, e non implementati: i bordi sferici cambiano il significato
    // geometrico dei dati, non la loro presentazione.
    let errore = non_supportato_con(&con_campo("edges", json!("spherical")));
    assert!(errore.message.contains("planar"), "{}", errore.message);
}

#[test]
fn edges_fuori_dalla_specifica_e_non_conforme() {
    for valore in [json!("toroidal"), json!("Planar"), json!(7), json!(null)] {
        non_conforme_con(&con_campo("edges", valore));
    }
}

// --- orientation ---------------------------------------------------

#[test]
fn orientation_assente_o_antiorario_e_accettato() {
    assert!(accettato(&documento(&colonna_minima()))
        .primaria
        .orientamento
        .is_none());
    assert_eq!(
        accettato(&con_campo("orientation", json!("counterclockwise")))
            .primaria
            .orientamento,
        Some(Orientamento::Antiorario)
    );
}

#[test]
fn orientation_fuori_dalla_specifica_e_non_conforme() {
    for valore in [json!("clockwise"), json!("ccw"), json!(7), json!(null)] {
        non_conforme_con(&con_campo("orientation", valore));
    }
}

// --- bbox ----------------------------------------------------------

#[test]
fn bbox_di_quattro_o_sei_numeri_e_accettato() {
    assert!(accettato(&documento(&colonna_minima()))
        .primaria
        .bbox
        .is_none());
    assert_eq!(
        accettato(&con_campo("bbox", json!([0.0, 0.0, 1.0, 1.0])))
            .primaria
            .bbox,
        Some(vec![0.0, 0.0, 1.0, 1.0])
    );
    assert_eq!(
        accettato(&con_campo("bbox", json!([0, 0, 0, 1, 1, 1])))
            .primaria
            .bbox
            .map(|b| b.len()),
        Some(6)
    );
}

#[test]
fn bbox_di_lunghezza_sbagliata_o_non_numerico_e_non_conforme() {
    for valore in [
        json!([]),
        json!([0, 0, 1]),
        json!([0, 0, 1, 1, 1]),
        json!([0, 0, 0, 1, 1, 1, 1]),
        json!("0,0,1,1"),
        json!([0, 0, 1, "1"]),
        json!([0, 0, 1, null]),
    ] {
        non_conforme_con(&con_campo("bbox", valore));
    }
}

#[test]
fn bbox_con_un_minimo_oltre_il_proprio_massimo_e_accettato() {
    // Lo schema non lo vieta, e un riquadro che attraversa l'antimeridiano
    // si scrive proprio cosi'. La prima stesura lo rifiutava: dichiarava
    // non conforme un documento che la specifica accetta, cioe' divergeva
    // dall'autorita' nel verso del rifiuto -- che e' comunque divergere.
    for valore in [
        json!([1.0, 0.0, 0.0, 1.0]),
        json!([0.0, 1.0, 1.0, 0.0]),
        json!([0, 0, 5, 1, 1, 1]),
    ] {
        accettato(&con_campo("bbox", valore));
    }
    accettato(&con_campo("bbox", json!([1.0, 1.0, 1.0, 1.0])));
}

#[test]
fn bbox_invertito_non_interpretabile_per_il_pruning_e_accettato() {
    // Cio' che quel riquadro non e' e' **usabile** con la semplice
    // intersezione di rettangoli: chi lo usasse leggerebbe meno del dovuto.
    // Il file resta valido e il pruning si spegne, che e' il verso in cui
    // questo driver sbaglia per contratto.
    assert!(interpretabile_per_il_pruning(&[0.0, 0.0, 1.0, 1.0]));
    assert!(interpretabile_per_il_pruning(&[1.0, 1.0, 1.0, 1.0]));
    assert!(!interpretabile_per_il_pruning(&[1.0, 0.0, 0.0, 1.0]));
    assert!(!interpretabile_per_il_pruning(&[
        0.0, 0.0, 5.0, 1.0, 1.0, 1.0
    ]));
}

#[test]
fn bbox_non_numerico_e_non_conforme_e_non_finito_non_e_esprimibile() {
    // Il `bbox` e l'`epoch` non filtrano `is_finite`, e la ragione e'
    // questa: JSON non ha `NaN` ne' infinito, e `serde_json` rifiuta da se'
    // i letterali che traboccherebbero in `f64`. Un filtro li' sarebbe una
    // guardia che non puo' scattare, e una guardia che non puo' scattare
    // non e' una difesa: e' una riga che nessuno potra' mai provare.
    //
    // La sonda fissa il fatto invece di fidarsene. Se un giorno la
    // dipendenza accettasse `1e400`, il documento arriverebbe alla
    // validazione con un infinito dentro e questa sonda sarebbe rossa --
    // che e' il momento giusto per rimettere il filtro.
    for letterale in ["1e400", "-1e400", "1e-400"] {
        let testo = format!(
            r#"{{"version":"1.1.0","primary_column":"geometry","columns":{{"geometry":{{"encoding":"WKB","geometry_types":[],"bbox":[0,0,{letterale},1]}}}}}}"#
        );
        let esito: std::result::Result<serde_json::Value, _> = serde_json::from_str(&testo);
        if let Ok(documento) = esito {
            // `1e-400` puo' arrotondare a zero invece di traboccare: e'
            // finito, e va bene. Cio' che non deve mai accadere e' un
            // valore non finito che arriva alla validazione.
            let letto = documento["columns"]["geometry"]["bbox"][2].as_f64();
            assert!(
                letto.is_some_and(f64::is_finite),
                "{letterale} e' arrivato non finito: il filtro va rimesso"
            );
        }
    }
    // E un `bbox` con un valore che numero non e' resta rifiutato.
    assert!(non_conforme_con(&con_campo("bbox", json!([0, 0, "1", 1])))
        .message
        .contains("bbox"));
}

// --- epoch ---------------------------------------------------------

#[test]
fn epoch_assente_o_numerico_e_accettato() {
    assert!(accettato(&documento(&colonna_minima()))
        .primaria
        .epoch
        .is_none());
    assert_eq!(
        accettato(&con_campo("epoch", json!(2021.5))).primaria.epoch,
        Some(2021.5)
    );
}

#[test]
fn epoch_che_non_e_un_numero_e_non_conforme() {
    for valore in [json!("2021.5"), json!(null), json!([2021]), json!({})] {
        non_conforme_con(&con_campo("epoch", valore));
    }
}

// --- covering ------------------------------------------------------

/// Il covering nella forma che lo schema 1.1.0 designa: due segmenti per
/// spigolo, il secondo uguale al nome dello spigolo.
fn covering_conforme() -> serde_json::Value {
    json!({"bbox": {
        "xmin": ["bbox", "xmin"],
        "ymin": ["bbox", "ymin"],
        "xmax": ["bbox", "xmax"],
        "ymax": ["bbox", "ymax"],
    }})
}

/// La forma che questo repository emetteva prima di S10: un segmento solo.
fn covering_piatto() -> serde_json::Value {
    json!({"bbox": {
        "xmin": ["_bbox_minx"],
        "ymin": ["_bbox_miny"],
        "xmax": ["_bbox_maxx"],
        "ymax": ["_bbox_maxy"],
    }})
}

#[test]
fn covering_di_due_segmenti_e_accettato() {
    let letti = accettato(&con_campo("covering", covering_conforme()));
    assert_eq!(
        letti.primaria.covering,
        Some([
            vec!["bbox".to_owned(), "xmin".to_owned()],
            vec!["bbox".to_owned(), "ymin".to_owned()],
            vec!["bbox".to_owned(), "xmax".to_owned()],
            vec!["bbox".to_owned(), "ymax".to_owned()],
        ])
    );
}

#[test]
fn covering_di_un_solo_segmento_e_non_conforme() {
    // E' la forma che questo writer emetteva, dichiarando 1.1.0: quei file
    // **non erano** GeoParquet 1.1 validi. Lo schema vuole
    // `minItems: 2, maxItems: 2`, e la prima stesura di questo modulo
    // chiamava «utilizzabile» proprio la forma sbagliata e «valida e
    // inutilizzabile» quella giusta -- esattamente al contrario.
    let errore = non_conforme_con(&con_campo("covering", covering_piatto()));
    assert!(errore.message.contains("segmenti"), "{}", errore.message);
}

#[test]
fn covering_col_secondo_segmento_sbagliato_e_non_conforme() {
    // Il secondo segmento e' un `const`: deve nominare **quello** spigolo.
    // Uno scambio qui darebbe al pruning le colonne incrociate.
    let scambiato = json!({"bbox": {
        "xmin": ["bbox", "ymin"],
        "ymin": ["bbox", "xmin"],
        "xmax": ["bbox", "xmax"],
        "ymax": ["bbox", "ymax"],
    }});
    let errore = non_conforme_con(&con_campo("covering", scambiato));
    assert!(errore.message.contains("spigolo"), "{}", errore.message);
}

#[test]
fn covering_senza_bbox_e_non_conforme() {
    // `required: ["bbox"]`. La prima stesura lo accettava dicendo che «la
    // specifica non chiude l'insieme delle chiavi di covering»: vero per le
    // chiavi in piu', falso per quella che manca.
    let errore = non_conforme_con(&con_campo("covering", json!({"altro": {}})));
    assert!(errore.message.contains("bbox"), "{}", errore.message);
}

#[test]
fn covering_malformato_e_non_conforme() {
    let mancante = json!({"bbox": {
        "xmin": ["bbox", "xmin"],
        "ymin": ["bbox", "ymin"],
        "xmax": ["bbox", "xmax"],
    }});
    assert!(non_conforme_con(&con_campo("covering", mancante))
        .message
        .contains("spigolo"));

    for valore in [
        json!("bbox"),
        json!([]),
        json!({"bbox": "niente"}),
        json!({"bbox": {"xmin": "a", "ymin": ["b", "ymin"], "xmax": ["c", "xmax"], "ymax": ["d", "ymax"]}}),
        json!({"bbox": {"xmin": [], "ymin": ["b", "ymin"], "xmax": ["c", "xmax"], "ymax": ["d", "ymax"]}}),
        json!({"bbox": {"xmin": [7, "xmin"], "ymin": ["b", "ymin"], "xmax": ["c", "xmax"], "ymax": ["d", "ymax"]}}),
        json!({"bbox": {"xmin": ["", "xmin"], "ymin": ["b", "ymin"], "xmax": ["c", "xmax"], "ymax": ["d", "ymax"]}}),
    ] {
        non_conforme_con(&con_campo("covering", valore));
    }
}

#[test]
fn covering_in_un_documento_1_0_0_ignorato_e_accettato() {
    // `covering` non esiste nello schema 1.0.0, e un documento 1.0.0 che lo
    // porta resta **valido**: l'oggetto-colonna non ha
    // `additionalProperties: false` in nessuna delle due versioni, quindi le
    // chiavi in piu' sono ammesse.
    //
    // Non ha pero' significato, e attribuirglielo lo attribuiremmo noi:
    // viene ignorato, e nemmeno validato nella forma -- non c'e' una forma
    // che quella versione gli imponga. Chi ha file scritti con le quattro
    // colonne piatte usa `bbox_legacy_by_name`, che e' esplicito.
    for forma in [covering_conforme(), covering_piatto(), json!({"bbox": {}})] {
        let mut colonna = colonna_minima();
        colonna["covering"] = forma;
        let letti = accettato(&con_versione("1.0.0", &colonna));
        assert_eq!(letti.versione, "1.0.0");
        assert!(letti.primaria.covering.is_none());
    }
}

// --- la via storica, stretta e spenta per default ---------------------

/// Il `crs` che questo repository scriveva fino a S10: non e' PROJJSON.
fn crs_storico() -> serde_json::Value {
    json!({"id": {"authority": "EPSG", "code": 4326}})
}

fn documento_storico() -> String {
    let mut colonna = colonna_minima();
    colonna["crs"] = crs_storico();
    con_versione("1.0.0", &colonna)
}

#[test]
fn crs_storico_senza_opt_in_e_non_conforme() {
    // Senza opzione resta `Format`, ed e' il default: la via di
    // compatibilita' che si accende da sola non e' una via, e' il
    // comportamento normale.
    let errore = analizza(&documento_storico(), false).expect_err("non conforme");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
}

#[test]
fn crs_storico_con_opt_in_conserva_l_identificatore_ed_e_accettato() {
    let letti = analizza(&documento_storico(), true).expect("accettato per compatibilita'");
    assert_eq!(letti.conformita, Conformita::CrsStoricoSoloIdentificatore);
    assert_eq!(
        letti.primaria.crs,
        Crs::StoricoSoloIdentificatore("EPSG:4326".to_owned())
    );

    // Il `code` puo' essere una stringa: e' una delle due forme che la
    // versione storica emetteva, a seconda di come l'identificatore era
    // stato scritto.
    let mut colonna = colonna_minima();
    colonna["crs"] = json!({"id": {"authority": "EPSG", "code": "4326"}});
    let con_stringa = analizza(&con_versione("1.0.0", &colonna), true).expect("accettato");
    assert_eq!(
        con_stringa.primaria.crs,
        Crs::StoricoSoloIdentificatore("EPSG:4326".to_owned())
    );
}

#[test]
fn crs_storico_in_un_documento_1_1_0_e_non_conforme() {
    // Vale solo per 1.0.0, che e' la versione che questo repository
    // scriveva: un 1.1.0 con quel `crs` non e' un nostro file storico, e
    // non c'e' ragione di tollerarlo.
    let mut colonna = colonna_minima();
    colonna["crs"] = crs_storico();
    let errore = analizza(&con_versione("1.1.0", &colonna), true).expect_err("non conforme");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
}

#[test]
fn crs_storico_di_forma_diversa_e_non_conforme() {
    // Non e' un permesso di accettare PROJJSON invalidi: e' il permesso di
    // accettare **quella** forma. Se fosse largo, sarebbe un buco travestito
    // da cortesia.
    for finto in [
        json!({"id": {"authority": "EPSG", "code": 4326}, "type": "GeographicCRS"}),
        json!({"id": {"authority": "", "code": 4326}}),
        json!({"id": {"authority": "EPSG"}}),
        json!({"id": {"authority": "EPSG", "code": 4326, "extra": 1}}),
        json!({"id": {"authority": "EPSG", "code": [4326]}}),
        json!({"identifier": {"authority": "EPSG", "code": 4326}}),
        json!({"type": "GeographicCRS", "name": "incompleto"}),
    ] {
        let mut colonna = colonna_minima();
        colonna["crs"] = finto.clone();
        let esito = analizza(&con_versione("1.0.0", &colonna), true);
        let errore = esito.expect_err("una forma diversa da quella storica non passa");
        assert_eq!(
            errore.code,
            plenora_io_model::IoErrorCode::Format,
            "{finto}"
        );
    }
}

#[test]
fn crs_storico_con_altri_difetti_e_non_conforme() {
    // Tolti i `crs` storici il documento deve essere conforme: l'opzione
    // tollera esattamente cio' che dichiara di tollerare.
    let mut colonna = colonna_minima();
    colonna["crs"] = crs_storico();
    colonna["geometry_types"] = json!(["Point M"]);
    let errore = analizza(&con_versione("1.0.0", &colonna), true).expect_err("non conforme");
    assert_eq!(errore.code, plenora_io_model::IoErrorCode::Format);
}

#[test]
fn documento_conforme_con_opt_in_acceso_e_accettato() {
    // L'opzione non cambia il giudizio su cio' che e' gia' conforme.
    let letti = analizza(&documento(&colonna_minima()), true).expect("conforme");
    assert_eq!(letti.conformita, Conformita::Conforme);
}

// --- l'insieme chiuso, letto da fuori --------------------------------

#[test]
fn geometry_types_i_quattordici_nomi_sono_accettati() {
    let mut quante = 0;
    for (nome, tipo) in NOMI_DI_TIPO {
        for (suffisso, dimensioni) in SUFFISSI {
            let etichetta = format!("{nome}{suffisso}");
            assert_eq!(
                etichetta_di_tipo(&etichetta),
                Some((tipo, dimensioni)),
                "{etichetta}"
            );
            quante += 1;
        }
    }
    assert_eq!(quante, 14, "sette nomi per due dimensionalita': XY e Z");
    for storta in ["", " Z", "Point Q", "pointz", "Point-Z"] {
        assert!(etichetta_di_tipo(storta).is_none(), "{storta}");
    }
}

//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

/// I tetti predefiniti, in un posto solo.
///
/// Le sonde di questo modulo provano la conversione fra WKT e AST, non le
/// quote: ripetere `WkbLimits::default()` su venti righe direbbe che le
/// stanno provando. A provarle sono le sonde di `wkt_progressivo`.
fn analizza_con_i_predefiniti(testo: &str) -> Result<WkbGeometry> {
    super::parse_wkt_bounded(testo, &WkbLimits::default())
}
use super::*;

#[test]
fn round_trips_all_dimensions_and_nested_geometry() {
    for text in [
        "POINT(1 2)",
        "LINESTRING Z(0 1 2,3 4 5)",
        "MULTIPOINT M((1 2 3),(4 5 6))",
        "MULTIPOLYGON ZM(((0 0 1 10,0 2 2 11,2 0 3 12,0 0 1 10)))",
        "GEOMETRYCOLLECTION Z(POINT Z(1 2 3),LINESTRING Z(0 0 0,1 1 1))",
    ] {
        let geometry = analizza_con_i_predefiniti(text).unwrap();
        let encoded = format_wkt(&geometry).unwrap();
        assert_eq!(
            analizza_con_i_predefiniti(&encoded).unwrap(),
            geometry,
            "{encoded}"
        );
    }
}

/// Un `MULTIPOLYGON` con un membro senza anelli deve serializzarsi, non
/// abbattere il processo.
///
/// Il percorso di partenza e' il WKB, non il testo: e' quello che i writer
/// CSV e XLSX ricevono dai reader, e la geometria e' rappresentabile in
/// diciotto byte. Prima della correzione la crate `wkt` faceva `.unwrap()`
/// sull'anello esterno del primo poligono e il processo moriva.
#[test]
fn un_multipoligono_con_membro_vuoto_si_serializza_invece_di_panicare() {
    let vuoto = |dimensioni| WkbGeometry {
        value: WkbValue::Polygon(vec![]),
        dimensions: dimensioni,
        srid: None,
    };
    let quadrato = WkbGeometry {
        value: WkbValue::Polygon(vec![vec![
            WkbCoordinate {
                x: 0.0,
                y: 0.0,
                z: None,
                m: None,
            },
            WkbCoordinate {
                x: 1.0,
                y: 0.0,
                z: None,
                m: None,
            },
            WkbCoordinate {
                x: 1.0,
                y: 1.0,
                z: None,
                m: None,
            },
            WkbCoordinate {
                x: 0.0,
                y: 0.0,
                z: None,
                m: None,
            },
        ]]),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };

    for (membri, atteso) in [
        (vec![vuoto(CoordinateDimensions::Xy)], "MULTIPOLYGON(EMPTY)"),
        (
            vec![quadrato.clone(), vuoto(CoordinateDimensions::Xy)],
            "MULTIPOLYGON(((0 0,1 0,1 1,0 0)),EMPTY)",
        ),
        (
            vec![vuoto(CoordinateDimensions::Xy), quadrato],
            "MULTIPOLYGON(EMPTY,((0 0,1 0,1 1,0 0)))",
        ),
    ] {
        let geometria = WkbGeometry {
            value: WkbValue::MultiPolygon(membri),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        };
        let testo = format_wkt(&geometria).expect("serializzazione");
        assert_eq!(testo, atteso);
        assert_eq!(
            analizza_con_i_predefiniti(&testo).expect("rilettura"),
            geometria,
            "round-trip di {testo}"
        );
    }
}

/// Lo stesso caso annidato in una `GEOMETRYCOLLECTION`: la deviazione deve
/// propagarsi ai figli, altrimenti il panico resta raggiungibile passando
/// da un livello in piu'.
#[test]
fn il_membro_vuoto_e_gestito_anche_dentro_una_geometrycollection() {
    let vuoto = WkbGeometry {
        value: WkbValue::Polygon(vec![]),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    let geometria = WkbGeometry {
        value: WkbValue::GeometryCollection(vec![
            WkbGeometry {
                value: WkbValue::Point(WkbCoordinate {
                    x: 1.0,
                    y: 2.0,
                    z: None,
                    m: None,
                }),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
            WkbGeometry {
                value: WkbValue::MultiPolygon(vec![vuoto]),
                dimensions: CoordinateDimensions::Xy,
                srid: None,
            },
        ]),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    let testo = format_wkt(&geometria).expect("serializzazione");
    assert_eq!(testo, "GEOMETRYCOLLECTION(POINT(1 2),MULTIPOLYGON(EMPTY))");
    assert_eq!(
        analizza_con_i_predefiniti(&testo).expect("rilettura"),
        geometria
    );
}

/// Le quattro forme che il WKT non sa esprimere fedelmente devono
/// fallire in serializzazione, e tutto il resto deve continuare a passare.
///
/// La tabella e' la mappa misurata delle perdite: senza il controllo, le
/// prime tre righe producevano testo che il nostro stesso parser rifiuta,
/// e la quarta rileggeva una geometria diversa da quella scritta.
#[test]
fn le_forme_non_esprimibili_in_wkt_falliscono_invece_di_uscire_sbagliate() {
    let c = |x: f64, y: f64| WkbCoordinate {
        x,
        y,
        z: None,
        m: None,
    };
    let quadrato = vec![c(0.0, 0.0), c(1.0, 0.0), c(1.0, 1.0), c(0.0, 0.0)];
    let xy = |value| WkbGeometry {
        value,
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };

    for (nome, value) in [
        ("anello vuoto", WkbValue::Polygon(vec![vec![]])),
        (
            "interno vuoto",
            WkbValue::Polygon(vec![quadrato.clone(), vec![]]),
        ),
        (
            "membro con anello vuoto",
            WkbValue::MultiPolygon(vec![xy(WkbValue::Polygon(vec![vec![]]))]),
        ),
        (
            "membro LineString vuoto",
            WkbValue::MultiLineString(vec![xy(WkbValue::LineString(vec![]))]),
        ),
    ] {
        let errore = format_wkt(&xy(value)).expect_err(nome);
        assert!(
            errore.to_string().contains("senza coordinate"),
            "{nome}: messaggio inatteso {errore}"
        );
    }

    // Il controllo non deve allargarsi: queste passano e fanno round-trip.
    for (nome, value) in [
        ("poligono senza anelli", WkbValue::Polygon(vec![])),
        ("linestring vuota", WkbValue::LineString(vec![])),
        (
            "anello di una sola coordinata",
            WkbValue::Polygon(vec![vec![c(0.0, 0.0)]]),
        ),
        (
            "anello non chiuso",
            WkbValue::Polygon(vec![vec![c(0.0, 0.0), c(1.0, 0.0), c(1.0, 1.0)]]),
        ),
        (
            "multipoligono con membro senza anelli",
            WkbValue::MultiPolygon(vec![xy(WkbValue::Polygon(vec![]))]),
        ),
        ("poligono valido", WkbValue::Polygon(vec![quadrato])),
    ] {
        let geometria = xy(value);
        let testo = format_wkt(&geometria).expect(nome);
        assert_eq!(
            analizza_con_i_predefiniti(&testo).expect(nome),
            geometria,
            "{nome}: round-trip di {testo}"
        );
    }
}

/// Lettura e scrittura devono avere lo stesso perimetro: se non sappiamo
/// riscrivere una geometria, non dobbiamo accettarla nemmeno da testo.
///
/// Non toglie nulla che funzionasse: `POLYGON(EMPTY)` non ha mai fatto
/// round-trip, veniva riletto come poligono senza anelli. Il fuzz target
/// `wkt_parse` asserisce esattamente questa simmetria — «WKT accettato deve
/// essere serializzabile» — ed e' cosi' che l'asimmetria e' venuta fuori.
#[test]
fn cio_che_accettiamo_da_testo_lo_sappiamo_riscrivere() {
    for testo in [
        "POLYGON(EMPTY)",
        "MULTIPOLYGON((EMPTY))",
        "MULTILINESTRING(EMPTY)",
    ] {
        let esito = analizza_con_i_predefiniti(testo);
        if let Ok(geometria) = &esito {
            format_wkt(geometria).unwrap_or_else(|errore| {
                panic!("{testo}: accettato in lettura ma non riscrivibile: {errore}")
            });
        }
    }

    // Le forme vuote di primo livello restano accettate e riscrivibili.
    for testo in ["POLYGON EMPTY", "LINESTRING EMPTY", "MULTIPOLYGON(EMPTY)"] {
        let geometria =
            analizza_con_i_predefiniti(testo).unwrap_or_else(|errore| panic!("{testo}: {errore}"));
        let riscritto = format_wkt(&geometria).unwrap_or_else(|errore| panic!("{testo}: {errore}"));
        assert_eq!(
            analizza_con_i_predefiniti(&riscritto)
                .unwrap_or_else(|errore| panic!("{testo}: {errore}")),
            geometria,
            "{testo}: round-trip via {riscritto}"
        );
    }
}

#[test]
fn rejects_empty_point_and_mixed_collection_dimensions() {
    assert!(analizza_con_i_predefiniti("POINT EMPTY").is_err());
    assert!(analizza_con_i_predefiniti("GEOMETRYCOLLECTION(POINT(1 2),POINT Z(1 2 3))").is_err());
}

#[test]
fn rejects_non_finite_coordinates_from_text_and_wkb() {
    assert!(analizza_con_i_predefiniti("POINT (2e308 -1e-308)").is_err());
    assert!(analizza_con_i_predefiniti("POINT ZM (1 2 NaN 4)").is_err());

    let geometry = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: f64::INFINITY,
            y: 2.0,
            z: None,
            m: None,
        }),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    assert!(format_wkt(&geometry).is_err());
}

#[test]
fn reusable_formatter_appends_and_preserves_buffer_on_error() {
    let geometry = analizza_con_i_predefiniti("LINESTRING Z(0 1 2,3 4 5)").unwrap();
    let mut output = "prefix:".to_owned();
    format_wkt_into(&geometry, &mut output).unwrap();
    assert_eq!(
        analizza_con_i_predefiniti(output.strip_prefix("prefix:").unwrap()).unwrap(),
        geometry
    );

    let invalid = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: f64::INFINITY,
            y: 2.0,
            z: None,
            m: None,
        }),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    let before = output.clone();
    assert!(format_wkt_into(&invalid, &mut output).is_err());
    assert_eq!(output, before);
}

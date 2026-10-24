//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

/// Su errore il buffer resta vuoto, e al tetto esatto la codifica passa.
///
/// Che il buffer non **cresca** oltre il tetto e' verificato dove si puo'
/// osservare, cioe' sul sink: qui lo svuotamento su `Err` renderebbe
/// l'assert indistinguibile fra un encoder bounded e uno che cresce e poi
/// ripulisce. Vedi `il_sink_non_lascia_mai_crescere_il_buffer_oltre_il_tetto`
/// in `wkb_lossless`.
///
/// Cio' che resta osservabile da qui e' il contratto pubblico: nessun
/// prefisso WKB parziale sopravvive all'errore. Un prefisso e' una
/// sequenza di byte ben formata fino a dove arriva, quindi riutilizzabile
/// per sbaglio senza che nulla protesti.
#[test]
fn l_encoder_bounded_lascia_il_buffer_vuoto_su_errore() {
    let punti: Vec<WkbCoordinate> = (0..64)
        .map(|indice| WkbCoordinate {
            x: f64::from(indice),
            y: f64::from(indice),
            z: None,
            m: None,
        })
        .collect();
    let geometria = WkbGeometry {
        value: WkbValue::LineString(punti),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };

    let intera = encode_wkb(&geometria, WkbFlavor::Iso).expect("codifica");
    assert!(intera.len() > 64, "la geometria deve essere non banale");

    for tetto in [1_usize, 9, 21, intera.len() - 1] {
        let mut buffer = Vec::new();
        let esito = encode_wkb_into_bounded(&geometria, WkbFlavor::Iso, &mut buffer, tetto);
        assert!(esito.is_err(), "tetto {tetto}: la codifica deve fallire");
        assert!(
            buffer.is_empty(),
            "tetto {tetto}: il buffer conserva {} byte di prefisso",
            buffer.len()
        );
    }

    // Anche un errore che non e' il tetto lascia il buffer vuoto: qui una
    // coordinata Z su una geometria dichiarata XY.
    let incoerente = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: Some(3.0),
            m: None,
        }),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    let mut buffer = Vec::new();
    assert!(
        encode_wkb_into_bounded(&incoerente, WkbFlavor::Iso, &mut buffer, usize::MAX).is_err(),
        "una geometria incoerente deve essere rifiutata"
    );
    assert!(
        buffer.is_empty(),
        "anche un errore non di tetto deve lasciare il buffer vuoto"
    );

    // Al tetto esatto passa: il confronto e' `>`, non `>=`.
    let mut buffer = Vec::new();
    assert!(encode_wkb_into_bounded(&geometria, WkbFlavor::Iso, &mut buffer, intera.len()).is_ok());
    assert_eq!(buffer, intera);
}

use geo_types::{
    Coord, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point,
    Polygon,
};

use crate::contract::{CoordinateDimensions, GeometryType};

use super::*;

fn sample_geometries() -> Vec<Geometry<f64>> {
    let polygon = Polygon::new(
        LineString(vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 4.0, y: 0.0 },
            Coord { x: 4.0, y: 4.0 },
            Coord { x: 0.0, y: 0.0 },
        ]),
        vec![],
    );
    vec![
        Geometry::Point(Point::new(1.0, 2.0)),
        Geometry::LineString(LineString(vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 1.0, y: 1.0 },
        ])),
        Geometry::Polygon(polygon.clone()),
        Geometry::MultiPoint(MultiPoint(vec![Point::new(0.0, 0.0)])),
        Geometry::MultiLineString(MultiLineString(vec![LineString(vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 1.0, y: 1.0 },
        ])])),
        Geometry::MultiPolygon(MultiPolygon(vec![polygon])),
        Geometry::GeometryCollection(GeometryCollection(vec![Geometry::Point(Point::new(
            7.0, 8.0,
        ))])),
    ]
}

fn xy(x: f64, y: f64) -> WkbCoordinate {
    WkbCoordinate {
        x,
        y,
        z: None,
        m: None,
    }
}

fn extended_geometries() -> Vec<WkbGeometry> {
    let child = |value| WkbGeometry {
        value,
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    let line = || child(WkbValue::LineString(vec![xy(0.0, 0.0), xy(1.0, 1.0)]));
    let circular = || {
        child(WkbValue::CircularString(vec![
            xy(0.0, 0.0),
            xy(1.0, 1.0),
            xy(2.0, 0.0),
        ]))
    };
    // Anelli di quattro punti chiusi: tre punti non fanno un anello lineare
    // (GEO-011), e fino alla 6.0 queste prove ne usavano tre.
    let polygon = || {
        child(WkbValue::Polygon(vec![vec![
            xy(0.0, 0.0),
            xy(2.0, 0.0),
            xy(2.0, 2.0),
            xy(0.0, 0.0),
        ]]))
    };
    let triangle = || {
        child(WkbValue::Triangle(vec![vec![
            xy(0.0, 0.0),
            xy(2.0, 0.0),
            xy(2.0, 2.0),
            xy(0.0, 0.0),
        ]]))
    };
    let compound = || child(WkbValue::CompoundCurve(vec![line(), circular()]));
    let curve_polygon = || child(WkbValue::CurvePolygon(vec![circular()]));

    vec![
        circular(),
        compound(),
        curve_polygon(),
        child(WkbValue::MultiCurve(vec![line(), compound()])),
        child(WkbValue::MultiSurface(vec![polygon(), curve_polygon()])),
        child(WkbValue::PolyhedralSurface(vec![polygon()])),
        child(WkbValue::Tin(vec![triangle()])),
        triangle(),
    ]
}

#[test]
fn geo_adapter_roundtrips_every_standard_type() {
    for geometry in sample_geometries() {
        let bytes = to_wkb(&geometry).unwrap();
        assert_eq!(from_wkb(&bytes, &WkbLimits::default()).unwrap(), geometry);
    }
}

#[test]
fn geo_adapter_is_byte_identical_to_authoritative_encoder() {
    for geometry in sample_geometries() {
        let ast = WkbGeometry::from_geo_xy(&geometry).unwrap();
        assert_eq!(
            to_wkb(&geometry).unwrap(),
            encode_wkb(&ast, WkbFlavor::Iso).unwrap()
        );
    }
}

#[test]
fn allocation_free_inspection_matches_the_authoritative_decoder() {
    for geometry in sample_geometries() {
        let bytes = to_wkb(&geometry).unwrap();
        let decoded = decode_wkb(&bytes, &WkbLimits::default()).unwrap();
        let inspected = inspect_wkb(&bytes, &WkbLimits::default()).unwrap();
        assert_eq!(inspected.geometry_type, decoded.geometry_type());
        assert_eq!(inspected.dimensions, decoded.dimensions);
        assert_eq!(inspected.srid, decoded.srid);
        assert!(inspected.nested_dimensions_coherent);
        assert!(!inspected.contains_srid);
    }

    let ewkb = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: Some(3.0),
            m: None,
        }),
        dimensions: CoordinateDimensions::Xyz,
        srid: Some(4326),
    };
    let bytes = encode_wkb(&ewkb, WkbFlavor::Ewkb).unwrap();
    let inspected = inspect_wkb(&bytes, &WkbLimits::default()).unwrap();
    assert_eq!(inspected.geometry_type, GeometryType::Point);
    assert_eq!(inspected.srid, Some(4326));
    assert!(inspected.contains_srid);
}

#[test]
fn lossless_codec_roundtrips_every_concrete_canonical_type() {
    let mut observed = sample_geometries()
        .iter()
        .map(WkbGeometry::from_geo_xy)
        .collect::<Result<Vec<_>>>()
        .unwrap();
    observed.extend(extended_geometries());

    let expected = [
        GeometryType::Point,
        GeometryType::LineString,
        GeometryType::Polygon,
        GeometryType::MultiPoint,
        GeometryType::MultiLineString,
        GeometryType::MultiPolygon,
        GeometryType::GeometryCollection,
        GeometryType::CircularString,
        GeometryType::CompoundCurve,
        GeometryType::CurvePolygon,
        GeometryType::MultiCurve,
        GeometryType::MultiSurface,
        GeometryType::PolyhedralSurface,
        GeometryType::Tin,
        GeometryType::Triangle,
    ];
    assert_eq!(observed.len(), expected.len());
    for (geometry, expected_type) in observed.iter().zip(expected) {
        assert_eq!(geometry.geometry_type(), expected_type);
        let bytes = encode_wkb(geometry, WkbFlavor::Iso).unwrap();
        assert_eq!(
            decode_wkb(&bytes, &WkbLimits::default()).unwrap(),
            *geometry
        );
        let inspection = inspect_wkb(&bytes, &WkbLimits::default()).unwrap();
        assert_eq!(inspection.geometry_type, expected_type);
        assert!(inspection.nested_dimensions_coherent);
    }
}

#[test]
fn extended_types_preserve_xyzm_and_srid_in_ewkb() {
    let coordinate = |x, y, z, m| WkbCoordinate {
        x,
        y,
        z: Some(z),
        m: Some(m),
    };
    let geometry = WkbGeometry {
        value: WkbValue::CircularString(vec![
            coordinate(0.0, 0.0, 10.0, 1.0),
            coordinate(1.0, 1.0, 11.0, 2.0),
            coordinate(2.0, 0.0, 12.0, 3.0),
        ]),
        dimensions: CoordinateDimensions::Xyzm,
        srid: Some(4979),
    };
    let bytes = encode_wkb(&geometry, WkbFlavor::Ewkb).unwrap();
    assert_eq!(decode_wkb(&bytes, &WkbLimits::default()).unwrap(), geometry);
    let inspection = inspect_wkb(&bytes, &WkbLimits::default()).unwrap();
    assert_eq!(inspection.geometry_type, GeometryType::CircularString);
    assert_eq!(inspection.dimensions, CoordinateDimensions::Xyzm);
    assert_eq!(inspection.srid, Some(4979));
    assert!(inspection.contains_srid);
}

#[test]
fn extended_aggregates_reject_noncanonical_children() {
    let invalid = WkbGeometry {
        value: WkbValue::Tin(vec![WkbGeometry {
            value: WkbValue::Polygon(vec![]),
            dimensions: CoordinateDimensions::Xy,
            srid: None,
        }]),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    assert!(encode_wkb(&invalid, WkbFlavor::Iso).is_err());
}

#[test]
fn geo_adapter_rejects_extended_types_without_linearizing_them() {
    for geometry in extended_geometries() {
        let bytes = encode_wkb(&geometry, WkbFlavor::Iso).unwrap();
        assert!(from_wkb(&bytes, &WkbLimits::default()).is_err());
    }
}

#[test]
fn hostile_input_and_limits_fail_closed() {
    assert!(from_wkb(&[], &WkbLimits::default()).is_err());
    let mut evil = vec![1_u8, 2, 0, 0, 0];
    evil.extend_from_slice(&1_000_000_000_u32.to_le_bytes());
    assert!(from_wkb(&evil, &WkbLimits::default()).is_err());

    let tight = WkbLimits {
        max_components: 1,
        ..WkbLimits::default()
    };
    let line = to_wkb(&Geometry::LineString(LineString(vec![
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 1.0 },
        Coord { x: 2.0, y: 2.0 },
    ])))
    .unwrap();
    assert!(from_wkb(&line, &tight).is_err());
}

#[test]
fn xy_adapter_uses_lossless_ewkb_parser() {
    let mut ewkb = vec![1];
    ewkb.extend_from_slice(&0xA000_0001_u32.to_le_bytes());
    ewkb.extend_from_slice(&4326_u32.to_le_bytes());
    ewkb.extend_from_slice(&1.0_f64.to_le_bytes());
    ewkb.extend_from_slice(&2.0_f64.to_le_bytes());
    ewkb.extend_from_slice(&3.0_f64.to_le_bytes());

    let decoded = decode_wkb(&ewkb, &WkbLimits::default()).unwrap();
    assert_eq!(decoded.dimensions, CoordinateDimensions::Xyz);
    assert_eq!(decoded.srid, Some(4326));
    assert_eq!(
        decoded.value,
        WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: Some(3.0),
            m: None,
        })
    );
    assert_eq!(encode_wkb(&decoded, WkbFlavor::Ewkb).unwrap(), ewkb);
    assert_eq!(
        from_wkb(&ewkb, &WkbLimits::default()).unwrap(),
        Geometry::Point(Point::new(1.0, 2.0))
    );
}

#[test]
fn iso_xym_and_xyzm_roundtrip_without_loss() {
    for (dimensions, coordinate) in [
        (
            CoordinateDimensions::Xym,
            WkbCoordinate {
                x: 10.0,
                y: 20.0,
                z: None,
                m: Some(7.0),
            },
        ),
        (
            CoordinateDimensions::Xyzm,
            WkbCoordinate {
                x: 10.0,
                y: 20.0,
                z: Some(30.0),
                m: Some(7.0),
            },
        ),
    ] {
        let geometry = WkbGeometry {
            value: WkbValue::Point(coordinate),
            dimensions,
            srid: None,
        };
        let encoded = encode_wkb(&geometry, WkbFlavor::Iso).unwrap();
        assert_eq!(
            decode_wkb(&encoded, &WkbLimits::default()).unwrap(),
            geometry
        );
    }
}

#[test]
fn all_decoders_reject_trailing_bytes() {
    let mut bytes = to_wkb(&Geometry::Point(Point::new(1.0, 2.0))).unwrap();
    bytes.push(0);
    assert!(decode_wkb(&bytes, &WkbLimits::default()).is_err());
    assert!(inspect_wkb(&bytes, &WkbLimits::default()).is_err());
    assert!(from_wkb(&bytes, &WkbLimits::default()).is_err());
}

#[test]
fn encoder_rejects_incoherent_ordinates() {
    let invalid = WkbGeometry {
        value: WkbValue::Point(WkbCoordinate {
            x: 1.0,
            y: 2.0,
            z: None,
            m: None,
        }),
        dimensions: CoordinateDimensions::Xyz,
        srid: None,
    };
    assert!(encode_wkb(&invalid, WkbFlavor::Iso).is_err());
}

// --- GEO-011: forma dei conteggi, chiusura degli anelli, annidamento ---------
//
// I byte sono scritti a mano, come li scrive `genera.py` del revisore e come li
// decide l'oracolo `arrow_data.py` dei contratti (`_geometry`): un encoder che
// sbagliasse nei due versi renderebbe la prova verde per costruzione.

/// Un'intestazione WKB ISO, little o big endian.
fn intestazione(little: bool, codice: u32) -> Vec<u8> {
    let mut byte = vec![u8::from(little)];
    byte.extend_from_slice(&u32_wkb(little, codice));
    byte
}

const fn u32_wkb(little: bool, valore: u32) -> [u8; 4] {
    if little {
        valore.to_le_bytes()
    } else {
        valore.to_be_bytes()
    }
}

fn ordinate(little: bool, valori: &[f64]) -> Vec<u8> {
    valori
        .iter()
        .flat_map(|v| {
            if little {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            }
        })
        .collect()
}

/// Una sequenza di `n` punti XY distinti, l'ultimo uguale al primo se
/// `chiusa`.
fn punti(little: bool, n: u32, chiusa: bool) -> Vec<u8> {
    let mut byte = u32_wkb(little, n).to_vec();
    for i in 0..n {
        let punto = if chiusa && i + 1 == n {
            [0.0, 0.0]
        } else {
            [f64::from(i), f64::from(i * i)]
        };
        byte.extend(ordinate(little, &punto));
    }
    byte
}

fn sequenza(little: bool, base: u32, n: u32, chiusa: bool) -> Vec<u8> {
    let mut byte = intestazione(little, base);
    byte.extend(punti(little, n, chiusa));
    byte
}

/// Un poligono (3) o triangolo (17) con gli anelli dati, gia' codificati.
fn con_anelli(little: bool, base: u32, anelli: &[Vec<u8>]) -> Vec<u8> {
    let mut byte = intestazione(little, base);
    byte.extend_from_slice(&u32_wkb(little, u32::try_from(anelli.len()).unwrap()));
    for anello in anelli {
        byte.extend_from_slice(anello);
    }
    byte
}

/// Un anello XY con i punti dati, codificati tali e quali.
fn anello(little: bool, coordinate: &[[f64; 2]]) -> Vec<u8> {
    let mut byte = u32_wkb(little, u32::try_from(coordinate.len()).unwrap()).to_vec();
    for [x, y] in coordinate {
        byte.extend(ordinate(little, &[*x, *y]));
    }
    byte
}

const QUADRATO: [[f64; 2]; 5] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
const TRIANGOLO: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]];

/// `profondita` collezioni attorno a un punto: il punto sta al livello
/// `profondita`.
fn collezioni_annidate(profondita: usize) -> Vec<u8> {
    let mut byte = intestazione(true, 1);
    byte.extend(ordinate(true, &[1.0, 2.0]));
    for _ in 0..profondita {
        let mut esterna = intestazione(true, 7);
        esterna.extend_from_slice(&1_u32.to_le_bytes());
        esterna.extend(byte);
        byte = esterna;
    }
    byte
}

fn rifiutato_ovunque(caso: &str, byte: &[u8], limiti: &WkbLimits) {
    assert!(
        inspect_wkb(byte, limiti).is_err(),
        "{caso}: l'ispezione deve rifiutarlo"
    );
    assert!(
        decode_wkb(byte, limiti).is_err(),
        "{caso}: il decoder deve rifiutarlo"
    );
}

fn accettato_ovunque(caso: &str, byte: &[u8], limiti: &WkbLimits) {
    assert!(
        inspect_wkb(byte, limiti).is_ok(),
        "{caso}: l'ispezione deve accettarlo"
    );
    assert!(
        decode_wkb(byte, limiti).is_ok(),
        "{caso}: il decoder deve accettarlo"
    );
}

/// I valori che GEO-011 rifiuta e l'oracolo dichiara `data_mapping`: una
/// linestring di un punto, una circularstring pari o di meno di tre punti, un
/// anello di meno di quattro punti o non chiuso, un triangolo di due anelli.
/// In entrambi gli ordini dei byte, e per entrambi i parser.
#[test]
fn geo_011_i_conteggi_e_gli_anelli_malformati_si_rifiutano() {
    let limiti = WkbLimits::default();
    for little in [true, false] {
        let casi: Vec<(&str, Vec<u8>)> = vec![
            ("linestring di un punto", sequenza(little, 2, 1, false)),
            ("circularstring di un punto", sequenza(little, 8, 1, false)),
            ("circularstring di due punti", sequenza(little, 8, 2, false)),
            (
                "circularstring di quattro punti",
                sequenza(little, 8, 4, false),
            ),
            ("circularstring di sei punti", sequenza(little, 8, 6, false)),
            (
                "anello non chiuso",
                con_anelli(little, 3, &[punti(little, 4, false)]),
            ),
            (
                "anello di tre punti chiuso",
                con_anelli(little, 3, &[punti(little, 3, true)]),
            ),
            (
                "anello vuoto",
                con_anelli(little, 3, &[punti(little, 0, false)]),
            ),
            (
                "secondo anello non chiuso",
                con_anelli(
                    little,
                    3,
                    &[anello(little, &QUADRATO), punti(little, 5, false)],
                ),
            ),
            (
                "triangolo di due anelli",
                con_anelli(
                    little,
                    17,
                    &[anello(little, &TRIANGOLO), anello(little, &TRIANGOLO)],
                ),
            ),
            (
                "triangolo con anello non chiuso",
                con_anelli(little, 17, &[punti(little, 4, false)]),
            ),
        ];
        for (caso, byte) in casi {
            rifiutato_ovunque(&format!("{caso}, little={little}"), &byte, &limiti);
        }
    }
}

/// I conteggi che GEO-011 ammette: zero punti dove il vuoto e' ammesso, tre o
/// cinque per una circularstring, un poligono e un triangolo senza anelli.
#[test]
fn geo_011_i_conteggi_ammessi_passano() {
    let limiti = WkbLimits::default();
    for little in [true, false] {
        let casi: Vec<(&str, Vec<u8>)> = vec![
            ("linestring vuota", sequenza(little, 2, 0, false)),
            ("linestring di due punti", sequenza(little, 2, 2, false)),
            ("circularstring vuota", sequenza(little, 8, 0, false)),
            ("circularstring di tre punti", sequenza(little, 8, 3, false)),
            (
                "circularstring di cinque punti",
                sequenza(little, 8, 5, false),
            ),
            ("poligono vuoto", con_anelli(little, 3, &[])),
            ("triangolo vuoto", con_anelli(little, 17, &[])),
            (
                "poligono con buco",
                con_anelli(
                    little,
                    3,
                    &[anello(little, &QUADRATO), anello(little, &QUADRATO)],
                ),
            ),
            (
                "triangolo di un anello",
                con_anelli(little, 17, &[anello(little, &TRIANGOLO)]),
            ),
        ];
        for (caso, byte) in casi {
            accettato_ovunque(&format!("{caso}, little={little}"), &byte, &limiti);
        }
    }
}

/// La chiusura e' l'uguaglianza dei **byte** del primo e dell'ultimo punto,
/// come in `arrow_data.py` (`data[:width] != data[-width:]`): un NaN con gli
/// stessi bit chiude, `-0.0` non chiude `0.0`, e la Z o la M diverse aprono un
/// anello chiuso in XY.
#[test]
fn geo_011_la_chiusura_e_byte_per_byte_anche_per_z_m_e_nan() {
    let limiti = WkbLimits::default();
    let nan = f64::NAN;
    let chiuso_nan = con_anelli(
        true,
        3,
        &[anello(
            true,
            &[[nan, 0.0], [1.0, 0.0], [1.0, 1.0], [nan, 0.0]],
        )],
    );
    accettato_ovunque("NaN con gli stessi bit", &chiuso_nan, &limiti);

    let zero_negativo = con_anelli(
        true,
        3,
        &[anello(
            true,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [-0.0, 0.0]],
        )],
    );
    rifiutato_ovunque("-0.0 contro 0.0", &zero_negativo, &limiti);

    // XYZ ISO (1003) e XYM ISO (2003): la terza ordinata del primo e
    // dell'ultimo punto differisce.
    for codice in [1003_u32, 2003] {
        let mut byte = intestazione(true, codice);
        byte.extend_from_slice(&1_u32.to_le_bytes());
        byte.extend_from_slice(&4_u32.to_le_bytes());
        for [x, y, terza] in [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 2.0],
        ] {
            byte.extend(ordinate(true, &[x, y, terza]));
        }
        rifiutato_ovunque(
            &format!("terza ordinata diversa, codice {codice}"),
            &byte,
            &limiti,
        );
    }
}

/// GEO-011: al piu' 32 livelli di annidamento oltre l'esterno, qualunque sia il
/// tetto configurato. Con 64, il default storico, 40 collezioni passavano.
#[test]
fn geo_011_l_annidamento_si_ferma_a_32_qualunque_sia_il_tetto() {
    for tetto in [32_usize, 64, 1_000, usize::MAX] {
        let limiti = WkbLimits {
            max_depth: tetto,
            ..WkbLimits::default()
        };
        accettato_ovunque(
            &format!("32 livelli, tetto {tetto}"),
            &collezioni_annidate(32),
            &limiti,
        );
        for profondita in [33_usize, 40] {
            rifiutato_ovunque(
                &format!("{profondita} livelli, tetto {tetto}"),
                &collezioni_annidate(profondita),
                &limiti,
            );
        }
    }
    // Un tetto piu' stretto resta piu' stretto.
    let stretto = WkbLimits {
        max_depth: 3,
        ..WkbLimits::default()
    };
    accettato_ovunque("3 livelli, tetto 3", &collezioni_annidate(3), &stretto);
    rifiutato_ovunque("4 livelli, tetto 3", &collezioni_annidate(4), &stretto);
}

/// L'encoder non emette cio' che il decoder rifiuterebbe: le stesse forme di
/// GEO-011 dall'AST, anche dentro un membro.
#[test]
fn geo_011_l_encoder_non_emette_valori_malformati() {
    let geometria = |value| WkbGeometry {
        value,
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    let aperto = vec![xy(0.0, 0.0), xy(1.0, 0.0), xy(1.0, 1.0), xy(0.0, 1.0)];
    let chiuso = vec![xy(0.0, 0.0), xy(1.0, 0.0), xy(1.0, 1.0), xy(0.0, 0.0)];
    for (caso, valore) in [
        (
            "linestring di un punto",
            WkbValue::LineString(vec![xy(0.0, 0.0)]),
        ),
        (
            "circularstring di due punti",
            WkbValue::CircularString(vec![xy(0.0, 0.0), xy(1.0, 1.0)]),
        ),
        ("anello non chiuso", WkbValue::Polygon(vec![aperto.clone()])),
        (
            "anello di tre punti",
            WkbValue::Polygon(vec![vec![xy(0.0, 0.0), xy(1.0, 0.0), xy(0.0, 0.0)]]),
        ),
        (
            "triangolo di due anelli",
            WkbValue::Triangle(vec![chiuso.clone(), chiuso.clone()]),
        ),
        (
            "anello non chiuso in un membro",
            WkbValue::MultiPolygon(vec![geometria(WkbValue::Polygon(vec![aperto]))]),
        ),
    ] {
        for flavor in [WkbFlavor::Iso, WkbFlavor::Ewkb] {
            assert!(
                encode_wkb(&geometria(valore.clone()), flavor).is_err(),
                "{caso}: l'encoder deve rifiutarlo"
            );
        }
    }
    assert!(encode_wkb(&geometria(WkbValue::Polygon(vec![chiuso])), WkbFlavor::Iso).is_ok());
}

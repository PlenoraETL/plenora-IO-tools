//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use super::{
    write_geometry, BoundedSink, CoordinateDimensions, WkbCoordinate, WkbFlavor, WkbGeometry,
    WkbValue,
};

fn linea(punti: usize) -> WkbGeometry {
    WkbGeometry {
        value: WkbValue::LineString(
            (0..punti)
                .map(|indice| WkbCoordinate {
                    #[allow(clippy::cast_precision_loss)]
                    x: indice as f64,
                    y: 2.0,
                    z: None,
                    m: None,
                })
                .collect(),
        ),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    }
}

/// Il buffer non supera il tetto **in nessun istante**, non solo alla fine.
///
/// E' il test che deve stare a questo livello. `encode_wkb_into_bounded`
/// svuota il buffer quando fallisce, quindi osservarne la lunghezza dopo
/// l'errore non distingue piu' un sink bounded da uno che cresce e poi
/// ripulisce: entrambi lascerebbero zero. Qui il sink e' costruito a mano
/// e nessuno ripulisce, quindi la lunghezza residua e' davvero il massimo
/// raggiunto.
#[test]
fn il_sink_non_lascia_mai_crescere_il_buffer_oltre_il_tetto() {
    let geometria = linea(10);
    for tetto in [0_usize, 1, 8, 9, 24, 168] {
        let mut buffer = Vec::new();
        let mut sink = BoundedSink {
            output: &mut buffer,
            max_bytes: tetto,
        };
        let esito = write_geometry(&mut sink, &geometria, WkbFlavor::Iso);
        assert!(esito.is_err(), "tetto {tetto}: la codifica deve fallire");
        assert!(
            buffer.len() <= tetto,
            "tetto {tetto}: il buffer ha raggiunto {} byte",
            buffer.len()
        );
    }
}

/// Al tetto esatto la codifica passa: il confronto e' `>`, non `>=`.
#[test]
fn il_tetto_esatto_e_ammesso() {
    let geometria = linea(10);
    // Nove byte di intestazione piu' sedici per punto.
    let esatta = 9 + 10 * 16;

    let mut buffer = Vec::new();
    let mut sink = BoundedSink {
        output: &mut buffer,
        max_bytes: esatta,
    };
    write_geometry(&mut sink, &geometria, WkbFlavor::Iso)
        .expect("una codifica lunga esattamente il tetto e' ammessa");
    assert_eq!(buffer.len(), esatta);
}

/// Un `MultiPoint` XY con un membro `Point Z`, e il suo speculare. Il WKB e'
/// scritto byte per byte: e' il valore che un file puo' portare.
fn aggregato_con_membro(contenitore: u32, membro: u32, coordinate: usize) -> Vec<u8> {
    let mut byte = vec![1];
    byte.extend_from_slice(&contenitore.to_le_bytes());
    byte.extend_from_slice(&1_u32.to_le_bytes());
    byte.push(1);
    byte.extend_from_slice(&membro.to_le_bytes());
    for _ in 0..coordinate {
        byte.extend_from_slice(&1.0_f64.to_le_bytes());
    }
    byte
}

/// GEO-011: un membro con dimensioni diverse da quelle del contenitore e'
/// malformato. Decodifica, ispezione e codifica lo accettavano, e il rifiuto
/// arrivava solo nei percorsi a valle che consultavano
/// `nested_dimensions_coherent`. Ora e' del codec, per ogni percorso.
#[test]
fn un_membro_di_altre_dimensioni_e_malformato_nel_codec() {
    let limiti = crate::limits::WkbLimits::default();
    for (contenitore, membro, coordinate) in [
        (4_u32, 1001_u32, 3_usize), // MultiPoint XY, Point Z
        (1004, 1, 2),               // MultiPoint Z, Point XY
        (7, 2001, 3),               // GeometryCollection XY, Point M
    ] {
        let byte = aggregato_con_membro(contenitore, membro, coordinate);
        let decodifica = super::decode_wkb(&byte, &limiti);
        let errore = decodifica.expect_err("decode_wkb");
        assert_eq!(errore.category, crate::ErrorCategory::DataMapping);
        let errore = super::inspect_wkb(&byte, &limiti).expect_err("inspect_wkb");
        assert_eq!(errore.category, crate::ErrorCategory::DataMapping);
    }
    // Lo stesso aggregato coerente passa.
    let coerente = aggregato_con_membro(4, 1, 2);
    assert!(super::decode_wkb(&coerente, &limiti).is_ok());
    assert!(super::inspect_wkb(&coerente, &limiti).is_ok());

    // L'encoder non emette cio' che il decoder rifiuta, in entrambi i flavor.
    let incoerente = WkbGeometry {
        value: WkbValue::MultiPoint(vec![WkbGeometry {
            value: WkbValue::Point(WkbCoordinate {
                x: 1.0,
                y: 2.0,
                z: Some(3.0),
                m: None,
            }),
            dimensions: CoordinateDimensions::Xyz,
            srid: None,
        }]),
        dimensions: CoordinateDimensions::Xy,
        srid: None,
    };
    for flavor in [WkbFlavor::Iso, WkbFlavor::Ewkb] {
        let errore = super::encode_wkb(&incoerente, flavor).expect_err("encode_wkb");
        assert_eq!(errore.category, crate::ErrorCategory::DataMapping);
    }
}

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

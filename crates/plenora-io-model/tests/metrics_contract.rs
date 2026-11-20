//! Metrics count codec operations and reset starts a fresh observation window.
#![cfg(feature = "metrics")]

use plenora_io_model::{
    contract::CoordinateDimensions,
    limits::WkbLimits,
    metrics,
    wkb::{decode_wkb, encode_wkb, WkbCoordinate, WkbFlavor, WkbGeometry, WkbValue},
};

#[test]
fn codec_counters_and_reset_describe_the_observed_round_trip() {
    // A separate integration-test process owns these global counters.
    metrics::reset();
    assert_eq!(metrics::snapshot(), (0, 0));
    let geometry = WkbGeometry {
        dimensions: CoordinateDimensions::Xy,
        srid: None,
        value: WkbValue::Point(WkbCoordinate {
            x: 9.0,
            y: 45.0,
            z: None,
            m: None,
        }),
    };
    let bytes = encode_wkb(&geometry, WkbFlavor::Iso).unwrap();
    assert_eq!(metrics::snapshot(), (0, 1));
    assert_eq!(decode_wkb(&bytes, &WkbLimits::default()).unwrap(), geometry);
    assert_eq!(metrics::snapshot(), (1, 1));
    metrics::reset();
    assert_eq!(metrics::snapshot(), (0, 0));
}

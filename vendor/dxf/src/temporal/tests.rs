use super::*;
#[test]
fn temporal_boundaries() {
    for v in [
        0.0,
        -1.0,
        1e300,
        -1e300,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e9,
        5373485.0,
    ] {
        assert!(as_datetime_utc(v).is_err());
        assert!(as_datetime_local(v).is_err());
    }
    assert_eq!(as_datetime_utc(1721426.0).unwrap().year(), 1);
    assert_eq!(as_datetime_utc(5373484.0).unwrap().year(), 9999);
    assert_eq!(
        as_datetime_utc(2451544.91568287).unwrap().to_rfc3339(),
        "1999-12-31T21:58:35+00:00"
    );
    assert_eq!(as_datetime_utc(2415020.0).unwrap().year(), 1899);
    for v in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
        assert!(as_duration(v).is_err());
    }
    for v in [0.0, -0.0, 0.5, 1.0] {
        assert_eq!(duration_as_double(as_duration(v).unwrap()), v);
    }
    for day in [i32::MIN, i32::MAX, 0, -1] {
        assert!(sunstudy(day, 0).is_err());
    }
    for sec in [i32::MIN, -1, 86400, i32::MAX] {
        assert!(sunstudy(2451545, sec).is_err());
    }
    assert!(sunstudy(2451545, 86399).is_ok());
}

#[test]
fn temporal_ticks_and_duration_limit() {
    assert!(as_datetime_utc(1721426.0 - 1.0 / 86400.0).is_err());
    assert!(as_datetime_utc(1721426.0 + 1.0 / 86400.0).is_ok());
    assert!(as_datetime_utc(5373485.0 - 1.0 / 86400.0).is_ok());
    assert!(as_datetime_utc(5373485.0).is_err());
    let limit = (u64::MAX as f64) / 86400.0;
    assert!(as_duration(limit).is_err());
    assert!(as_duration(f64::from_bits(limit.to_bits() - 1)).is_ok());
    assert_eq!(as_duration(f64::from_bits(1)).unwrap(), Duration::ZERO);
}

#[test]
fn temporal_writer_never_projects_chrono_extremes_infallibly() {
    for utc in [DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC] {
        for seconds in [-3600, 3600] {
            let value = utc.with_timezone(&chrono::FixedOffset::east_opt(seconds).unwrap());
            assert!(as_double(value).is_finite());
        }
    }
}

use super::*;
use chrono::{TimeZone, Utc};
#[test]
fn temporal_ogr_boundaries() {
    let zero = FixedOffset::east_opt(0).unwrap();
    for year in [1, 9999] {
        assert!(datetime(zero.with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap()).is_ok());
    }
    assert!(datetime(zero.with_ymd_and_hms(10000, 1, 1, 0, 0, 0).unwrap()).is_err());
    for seconds in [-900, 900] {
        let offset = FixedOffset::east_opt(seconds).unwrap();
        for utc in [DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC] {
            assert!(datetime(utc.with_timezone(&offset)).is_err());
        }
    }
    assert!(datetime(
        FixedOffset::east_opt(1)
            .unwrap()
            .with_ymd_and_hms(2020, 1, 1, 0, 0, 0)
            .unwrap()
    )
    .is_err());
    assert!(datetime(
        zero.with_ymd_and_hms(2020, 1, 1, 0, 0, 0)
            .unwrap()
            .with_nanosecond(1)
            .unwrap()
    )
    .is_err());
    assert_eq!(
        datetime(zero.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap()).unwrap()[6],
        100
    );
}

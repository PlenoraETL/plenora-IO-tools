use super::*;
use chrono::Timelike;
#[test]
fn temporal_native_roundtrip_and_rejection_is_atomic() {
    let driver = crate::DriverManager::get_driver_by_name("MEM")
        .or_else(|_| crate::DriverManager::get_driver_by_name("Memory"))
        .unwrap();
    let mut dataset = driver.create_vector_only("").unwrap();
    let layer = dataset.create_layer(Default::default()).unwrap();
    layer
        .create_defn_fields(&[("time", OGRFieldType::OFTDateTime)])
        .unwrap();
    let mut feature = Feature::new(layer.defn()).unwrap();
    for year in [1, 2026, 9999] {
        for offset in [-3600, 0, 3600] {
            let value = FixedOffset::east_opt(offset)
                .unwrap()
                .with_ymd_and_hms(year, 1, 1, 12, 0, 0)
                .unwrap();
            feature.set_field_datetime(0, value).unwrap();
            assert_eq!(feature.field_as_datetime(0).unwrap(), Some(value));
            assert!(matches!(
                feature.set_field_datetime(0, value.with_nanosecond(1).unwrap()),
                Err(GdalError::DateError(_))
            ));
            assert_eq!(feature.field_as_datetime(0).unwrap(), Some(value));
        }
    }
}

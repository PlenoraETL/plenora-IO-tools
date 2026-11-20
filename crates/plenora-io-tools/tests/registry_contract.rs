//! The registry dispatches by exact identifier, independently of insertion order.
use plenora_io_core::registry::DriverRegistry;

#[test]
fn lookup_and_catalog_share_the_registered_drivers() {
    let mut registry = DriverRegistry::new();
    assert!(registry.by_id("geojson").is_none());
    registry.register(Box::new(driver_geojson::GeoJsonDriver));
    registry.register(Box::new(driver_csv::CsvDriver));
    let identifiers: Vec<_> = registry.descriptors().iter().map(|d| d.id()).collect();
    assert_eq!(identifiers, ["csv", "geojson"]);
    for identifier in identifiers {
        assert_eq!(
            registry.by_id(identifier).unwrap().descriptor().id(),
            identifier
        );
    }
    assert!(registry.by_id("GeoJSON").is_none());
    assert!(registry.by_id("missing").is_none());
}

//! Inventario completo delle chiavi, su ogni driver ricavato dal catalogo reale.
use arrow_schema::{DataType, Field, Schema};
use plenora_io_core::property_plan::{field_in_wkb, plan_metadata, Rule, PROPERTIES};

#[test]
fn every_property_and_opaque_key_has_a_rule_for_each_driver() {
    let catalog = plenora_io_tools::cmd_catalog();
    let drivers = catalog["drivers"].as_array().unwrap();
    assert!(!drivers.is_empty());
    let mut metadata: std::collections::HashMap<String, String> = PROPERTIES
        .iter()
        .map(|(_, key)| {
            let key = if key.ends_with('.') {
                format!("{key}custom")
            } else {
                (*key).to_owned()
            };
            (key, "sentinel_value".to_owned())
        })
        .collect();
    metadata.insert("plenora.field_id".to_owned(), "41".to_owned());
    metadata.insert("plenora.geometry.encoding".to_owned(), "ewkb".to_owned());
    for key in [
        "opaque",
        "plenora.postgres.custom",
        "geo",
        "ARROW:extension:metadata",
        "plenora.geometry.native.custom",
    ] {
        metadata.insert(key.to_owned(), "sentinel_value".to_owned());
    }
    let fields = vec![
        Field::new("a", DataType::Binary, true).with_metadata(metadata.clone()),
        Field::new("b", DataType::Binary, true).with_metadata(metadata.clone()),
    ];
    let schema = Schema::new_with_metadata(fields, metadata.clone());
    for driver in drivers {
        let id = driver["id"].as_str().unwrap();
        for convert in [None, Some(0), Some(1)] {
            let effects = plan_metadata(&schema, id, convert);
            assert_eq!(effects.len(), 3 * metadata.len(), "{id}");
            for position in [None, Some(0), Some(1)] {
                let localized: Vec<_> = effects
                    .iter()
                    .filter(|e| e.field_index == position)
                    .collect();
                assert_eq!(localized.len(), metadata.len());
                assert!(localized
                    .iter()
                    .any(|e| e.property.as_deref() == Some("native_metadata")));
                assert_eq!(
                    localized.iter().map(|e| e.key_index).collect::<Vec<_>>(),
                    (0..metadata.len()).collect::<Vec<_>>()
                );
                if convert.is_some() && convert == position {
                    assert!(localized.iter().any(|e| e.rule == Rule::EwkbToWkb));
                    assert!(localized.iter().any(|e| e.rule == Rule::SridRemoved));
                } else {
                    assert!(localized
                        .iter()
                        .all(|e| !matches!(e.rule, Rule::EwkbToWkb | Rule::SridRemoved)));
                }
            }
            assert!(!serde_json::to_string(&effects)
                .unwrap()
                .contains("sentinel_value"));
        }
    }
}

#[test]
fn conversion_mutates_only_encoding_and_srid() {
    let metadata = [
        ("plenora.geometry.encoding".to_owned(), "ewkb".to_owned()),
        ("plenora.geometry.srid".to_owned(), "4326".to_owned()),
        ("opaque".to_owned(), "unchanged".to_owned()),
    ];
    let input = Field::new("geometry", DataType::Binary, true).with_metadata(metadata);
    let output = field_in_wkb(&input);
    assert_eq!(output.metadata()["plenora.geometry.encoding"], "wkb");
    assert!(!output.metadata().contains_key("plenora.geometry.srid"));
    assert_eq!(output.metadata()["opaque"], input.metadata()["opaque"]);
    assert_eq!(output.name(), input.name());
    assert_eq!(output.is_nullable(), input.is_nullable());
}

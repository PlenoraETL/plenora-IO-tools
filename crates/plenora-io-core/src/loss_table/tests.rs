use super::*;
use std::collections::HashMap;

const EXPECTED_NATIVE_ELEMENTS: &[&str] = &[
    "NullGeometry",
    "EmptyPoint",
    "SignedZero",
    "Subtypes",
    "Aliases",
    "Domains",
    "TextStyle",
    "PaperSpace",
    "Xdata",
    "Width",
    "Thickness",
    "Lineweight",
    "Linetype",
    "Color",
    "Handle",
    "DbfDate",
    "Utf8",
    "Null",
    "EmptyString",
    "SignedInteger",
    "UnsignedInteger",
    "Floating",
    "Boolean",
    "Binary",
    "Decimal",
    "Temporal",
    "NestedDictionary",
    "FieldNames",
    "NotNull",
    "FieldOrder",
    "SchemaMetadata",
    "FieldMetadata",
    "NativeMetadata",
    "Z",
    "M",
    "Dimensions",
    "GeometryType",
    "RingOrientation",
    "EmptyGeometry",
    "CoordinatePrecision",
    "CrsId",
    "CrsDefinition",
    "FeatureId",
    "Bbox",
    "ForeignMembers",
    "LayerMetadata",
    "Styles",
    "Hierarchy",
    "Extensions",
    "ExternalReference",
    "ExtendedData",
    "FeatureMetadata",
    "AltitudeMode",
    "Inference",
    "NumberPrecision",
    "StructuredValue",
    "MissingValue",
    "CoordinateColumns",
    "NumericFormat",
    "Formula",
    "FormulaCache",
    "Identifier",
    "Description",
    "DataColumns",
    "SqlConstraints",
    "ContractVersion",
    "FieldIdentity",
    "TypesDeclaration",
    "GeometryEncoding",
    "SpatialSemantics",
    "Precision",
    "Srid",
    "CrsResolution",
    "DefinitionFormat",
    "AxisOrder",
    "PrimaryGeometry",
    "ExtensionName",
    "ExtensionMetadata",
    "UnknownDimensions",
    "Covering",
];

#[test]
#[allow(clippy::too_many_lines)] // The independent inventory and the exhaustive matrix are checked together.
fn native_matrix_enumerates_every_element_and_direction() {
    let qualified: Vec<_> = DRIVERS
        .iter()
        .filter_map(|row| row.qualified_native.map(|native| (row.driver, native)))
        .collect();
    assert_eq!(qualified.len(), DRIVERS.len());
    assert_eq!(
        qualified,
        [
            ("csv", NativeDriver::Csv),
            ("shp", NativeDriver::Shp),
            ("gpkg", NativeDriver::Gpkg),
            ("filegdb", NativeDriver::FileGdb),
            ("geojson", NativeDriver::GeoJson),
            ("kml", NativeDriver::Kml),
            ("dxf", NativeDriver::Dxf),
            ("xls", NativeDriver::Xls),
            ("geoparquet", NativeDriver::GeoParquet),
            ("ipc", NativeDriver::Ipc)
        ]
    );
    for (id, driver) in &qualified {
        let catalog = rules(id).unwrap();
        for (class, element) in [
            (ArrowTypeClass::Boolean, NativeElement::Boolean),
            (ArrowTypeClass::SignedInteger, NativeElement::SignedInteger),
            (
                ArrowTypeClass::UnsignedInteger,
                NativeElement::UnsignedInteger,
            ),
            (ArrowTypeClass::Floating, NativeElement::Floating),
            (ArrowTypeClass::Utf8, NativeElement::Utf8),
            (ArrowTypeClass::Temporal, NativeElement::Temporal),
            (ArrowTypeClass::Binary, NativeElement::Binary),
            (ArrowTypeClass::Decimal, NativeElement::Decimal),
        ] {
            assert_eq!(
                catalog.allowed_types.contains(&class),
                native_rule(*driver, TransportDirection::Write, element) != NativeRule::Rejected
            );
        }
        assert!(catalog.dimensions.contains(&CoordinateDimensions::Xyz));
        assert_eq!(
            native_rule(*driver, TransportDirection::Write, NativeElement::Z),
            NativeRule::Preserved
        );
        assert_eq!(
            catalog.dimensions.contains(&CoordinateDimensions::Xym),
            matches!(
                driver,
                NativeDriver::Shp
                    | NativeDriver::FileGdb
                    | NativeDriver::Csv
                    | NativeDriver::Xls
                    | NativeDriver::Gpkg
                    | NativeDriver::Ipc
            )
        );
        assert_eq!(
            catalog.dimensions.contains(&CoordinateDimensions::Xyzm),
            matches!(
                driver,
                NativeDriver::Shp
                    | NativeDriver::FileGdb
                    | NativeDriver::Csv
                    | NativeDriver::Xls
                    | NativeDriver::Gpkg
                    | NativeDriver::Ipc
            )
        );
        assert_eq!(
            native_rule(*driver, TransportDirection::Write, NativeElement::M),
            if matches!(
                driver,
                NativeDriver::Shp
                    | NativeDriver::FileGdb
                    | NativeDriver::Csv
                    | NativeDriver::Xls
                    | NativeDriver::Gpkg
                    | NativeDriver::Ipc
            ) {
                P
            } else {
                R
            }
        );
    }
    let actual: Vec<_> = NATIVE_ELEMENTS
        .iter()
        .map(|element| format!("{element:?}"))
        .collect();
    assert_eq!(actual, EXPECTED_NATIVE_ELEMENTS);
    for driver in [
        NativeDriver::GeoJson,
        NativeDriver::Kml,
        NativeDriver::Shp,
        NativeDriver::Dxf,
        NativeDriver::FileGdb,
        NativeDriver::Csv,
        NativeDriver::Xls,
        NativeDriver::Gpkg,
        NativeDriver::GeoParquet,
        NativeDriver::Ipc,
    ] {
        for direction in [TransportDirection::Read, TransportDirection::Write] {
            for &element in NATIVE_ELEMENTS {
                let mut report = crate::LossReport::default();
                observe_native(&mut report, (driver, direction), element, 3, 7, Some(11));
                match native_rule(driver, direction, element) {
                    NativeRule::Transformed(change) => {
                        assert_eq!(report.counts[change.category()], 3);
                        let example = report.esempi_canonici().next().unwrap();
                        assert_eq!(example.posizione.layer_index, Some(7));
                        assert_eq!(example.posizione.field_index, Some(11));
                    }
                    NativeRule::Preserved | NativeRule::Rejected => assert!(report.is_empty()),
                }
            }
        }
    }
}

#[test]
fn missing_measure_cannot_escape_without_its_native_meaning() {
    let mut geometry = plenora_io_model::contract::GeometryColumnContract::wkb_xy(
        plenora_io_model::contract::FieldId(0),
        "geometry",
        plenora_io_model::crs::ResolvedCrs::wgs84(),
        true,
    );
    geometry.dimensions = CoordinateDimensions::Xym;
    geometry
        .native_metadata
        .insert("shp.measure_no_data".to_owned(), "-1e39".to_owned());
    for driver in ["csv", "xls", "geojson", "kml", "dxf", "gpkg", "filegdb"] {
        let error = validate_native_measures(driver, std::slice::from_ref(&geometry)).unwrap_err();
        assert_eq!(error.category, plenora_io_model::ErrorCategory::Unsupported);
        assert!(!error.message.contains("-1e39"));
    }
    for driver in ["ipc", "shp", "geoparquet"] {
        validate_native_measures(driver, std::slice::from_ref(&geometry)).unwrap();
    }
    geometry.native_metadata.clear();
    validate_native_measures("csv", &[geometry]).unwrap();
}

#[test]
fn opaque_schema_and_field_metadata_have_counts_without_payload() {
    let schema = arrow_schema::Schema::new_with_metadata(
        vec![
            arrow_schema::Field::new("secret_field", DataType::Utf8, true).with_metadata(
                HashMap::from([("secret_key".into(), "secret_value".into())]),
            ),
        ],
        HashMap::from([("secret_schema_key".into(), "secret_schema_value".into())]),
    );
    for driver in ["csv", "xls", "geojson", "kml", "dxf", "shp", "filegdb"] {
        let plan = crate::property_plan::PropertyPlan::new(&schema, driver, None);
        plan.validate_schema(driver, &schema).unwrap();
        let report = plan.report(3, crate::property_plan::Phase::Write).unwrap();
        assert!(report.counts.is_empty());
        let native = DRIVERS
            .iter()
            .find(|row| row.driver == driver)
            .unwrap()
            .qualified_native
            .unwrap();
        let report = metadata_changes(
            native,
            TransportDirection::Write,
            &schema,
            &arrow_schema::Schema::empty(),
        );
        assert_eq!(report.counts.get("arrow_metadata_not_preserved"), Some(&2));
        let json = serde_json::to_string(&report.esempi_canonici().collect::<Vec<_>>()).unwrap();
        assert!(!json.contains("secret"));
    }
    let plan = crate::property_plan::PropertyPlan::new(&schema, "ipc", None);
    assert!(plan
        .report(3, crate::property_plan::Phase::Write)
        .unwrap()
        .counts
        .is_empty());
}
#[test]
fn arrow_contractual_metadata_changes_are_counted_without_payload() {
    for driver in [NativeDriver::Ipc, NativeDriver::GeoParquet] {
        for direction in [TransportDirection::Read, TransportDirection::Write] {
            for (_, key, _, _) in PROPERTIES {
                if key.is_empty() {
                    continue;
                }
                let before = arrow_schema::Schema::new_with_metadata(
                    vec![
                        arrow_schema::Field::new("private-field", DataType::Utf8, true)
                            .with_metadata(HashMap::from([(
                                key.to_string(),
                                "private-value".into(),
                            )])),
                    ],
                    HashMap::from([(key.to_string(), "private-value".into())]),
                );
                assert!(metadata_changes(driver, direction, &before, &before).is_empty());
                let after = arrow_schema::Schema::new(vec![arrow_schema::Field::new(
                    "private-field",
                    DataType::Utf8,
                    true,
                )]);
                let report = metadata_changes(driver, direction, &before, &after);
                assert_eq!(report.counts.get("arrow_metadata_not_preserved"), Some(&2));
                let examples =
                    serde_json::to_string(&report.esempi_canonici().collect::<Vec<_>>()).unwrap();
                assert!(!examples.contains("private"));
            }
        }
    }
}

#[test]
fn native_grammar_applicability_is_separate_from_arrow_transport() {
    use ReadApplicability::{Expressible, NotApplicable};
    for driver in DRIVERS {
        let native = driver.qualified_native.unwrap();
        for &element in NATIVE_ELEMENTS {
            if let NotApplicable { reason, reference } = read_applicability(native, element) {
                assert!(!reason.is_empty() && !reference.is_empty());
            }
        }
    }
    assert_eq!(
        read_applicability(NativeDriver::Xls, NativeElement::Formula),
        Expressible
    );
    assert_eq!(
        read_applicability(NativeDriver::Ipc, NativeElement::Formula),
        Expressible
    );
    assert!(matches!(
        read_applicability(NativeDriver::Csv, NativeElement::Formula),
        NotApplicable { .. }
    ));
    assert!(matches!(
        read_applicability(NativeDriver::Kml, NativeElement::PaperSpace),
        NotApplicable { .. }
    ));
    assert_ne!(
        native_write_transport_rule(NativeDriver::Csv, NativeElement::Formula),
        NativeRule::Rejected
    );
    assert_eq!(
        native_rule(
            NativeDriver::Kml,
            TransportDirection::Write,
            NativeElement::ExtendedData
        ),
        NativeRule::Transformed(NativeChange::ExtendedData)
    );
}

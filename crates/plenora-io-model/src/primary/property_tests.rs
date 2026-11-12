#[test]
fn every_serialized_geometry_property_has_a_comparison() {
    use crate::{contract::*, crs::CrsResolution, geometry::*};
    let mut g =
        GeometryColumnContract::wkb_passthrough(FieldId(0), "g", CrsResolution::Missing, true);
    g.srid = Some(4326);
    g.set_exact_geometry_types(vec![GeometryType::Point]);
    let field = with_geometry_contract_metadata(
        &arrow_schema::Field::new("g", arrow_schema::DataType::Binary, true),
        &g,
    );
    for key in field
        .metadata()
        .keys()
        .filter(|k| k.starts_with("plenora.geometry."))
    {
        assert!(
            super::GEOMETRY_PROPERTIES
                .iter()
                .any(|(name, _)| *name == key),
            "{key}"
        );
    }
    // Independent inventory of every wire property defined by the geometry vocabulary.
    let actual: std::collections::BTreeSet<_> = include_str!("../geometry.rs")
        .lines()
        .filter(|line| line.starts_with("pub const "))
        .filter_map(|line| line.split_once("= \"").map(|(_, rest)| rest))
        .filter_map(|rest| rest.split_once('"').map(|(key, _)| key))
        .filter(|key| key.starts_with("plenora.geometry."))
        .collect();
    assert_eq!(
        actual,
        super::GEOMETRY_PROPERTIES
            .iter()
            .map(|(key, _)| *key)
            .collect()
    );
    // Exhaustive destructuring: a new internal property requires an explicit decision here.
    let GeometryColumnContract {
        field_id: _,
        identita_dichiarata: _,
        name: _,
        crs: _,
        nullable: _,
        encoding: _,
        dimensions: _,
        spatial_semantics: _,
        srid: _,
        precision: _,
        geometry_types: _,
        types_declaration: _,
        scansione_completa: _,
        native_metadata: _,
    } = g;
}

#[test]
fn unresolved_wire_values_are_enumerated_and_symmetric() {
    use crate::{contract::*, crs::*, geometry::*};
    let expected = [
        (PLENORA_DIMENSIONS_KEY, vec!["unknown"]),
        (PLENORA_GEOMETRY_TYPES_KEY, vec!["unknown"]),
        (PLENORA_TYPES_DECLARATION_KEY, vec!["unresolved"]),
        (
            PLENORA_CRS_RESOLUTION_KEY,
            vec!["missing", "declared_unresolved"],
        ),
        (PLENORA_AXIS_ORDER_KEY, vec!["unknown"]),
    ];
    for (key, tokens) in super::GEOMETRY_PROPERTIES {
        assert_eq!(
            *tokens,
            expected
                .iter()
                .find(|(k, _)| k == key)
                .map_or(&[][..], |(_, v)| v.as_slice()),
            "{key}"
        );
    }
    let mut resolved =
        GeometryColumnContract::wkb_passthrough(FieldId(0), "g", ResolvedCrs::wgs84(), true);
    resolved.dimensions = CoordinateDimensions::Xy;
    resolved.set_exact_geometry_types(vec![GeometryType::Point]);
    resolved.srid = Some(1234);
    let clean = arrow_schema::Field::new("g", arrow_schema::DataType::Binary, true);
    for key in [
        PLENORA_DIMENSIONS_KEY,
        PLENORA_GEOMETRY_TYPES_KEY,
        PLENORA_TYPES_DECLARATION_KEY,
    ] {
        let mut unknown = resolved.clone();
        match key {
            PLENORA_DIMENSIONS_KEY => unknown.dimensions = CoordinateDimensions::Unknown,
            PLENORA_GEOMETRY_TYPES_KEY => {
                unknown.set_exact_geometry_types(vec![GeometryType::Unknown]);
            }
            _ => {
                unknown.types_declaration = TypesDeclaration::Unresolved;
                unknown.geometry_types.clear();
            }
        }
        for (a, b) in [(&resolved, &unknown), (&unknown, &resolved)] {
            super::reconcile_geometry_properties(&with_geometry_contract_metadata(&clean, a), b)
                .unwrap();
        }
    }
    for (key, _) in super::GEOMETRY_PROPERTIES {
        let mut md = with_geometry_contract_metadata(&clean, &resolved)
            .metadata()
            .clone();
        // Missing optional properties carry no competing constraint. CRS fields
        // must be removed together to remain a valid declaration.
        if key.contains("crs_") || key.ends_with("axis_order") {
            continue;
        }
        md.remove(key);
        if *key == PLENORA_GEOMETRY_TYPES_KEY {
            md.remove(PLENORA_TYPES_DECLARATION_KEY);
        }
        super::reconcile_geometry_properties(&clean.clone().with_metadata(md), &resolved).unwrap();
    }
    let mut conflicting = resolved.clone();
    conflicting.srid = Some(4326);
    assert!(super::reconcile_geometry_properties(
        &with_geometry_contract_metadata(&clean, &resolved),
        &conflicting
    )
    .is_err());
    let mut unknown_axes = ResolvedCrs::wgs84();
    unknown_axes.axis_order = AxisOrder::Unknown;
    let unknown_axes = CrsResolution::Resolved(unknown_axes);
    for (a, b) in [
        (&resolved.crs, &unknown_axes),
        (&unknown_axes, &resolved.crs),
    ] {
        assert_eq!(super::reconcile_geometry_crs(a, b).unwrap(), resolved.crs);
    }
    for unknown in [
        CrsResolution::Missing,
        CrsResolution::DeclaredButUnresolved(RawCrs::from_authority_hint("LOCAL:x".into())),
    ] {
        for (a, b) in [(&resolved.crs, &unknown), (&unknown, &resolved.crs)] {
            assert_eq!(super::reconcile_geometry_crs(a, b).unwrap(), resolved.crs);
        }
    }
}

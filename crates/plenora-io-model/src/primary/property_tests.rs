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
        assert!(super::GEOMETRY_PROPERTIES.contains(&key.as_str()), "{key}");
    }
    // Independent inventory of every wire property defined by the geometry vocabulary.
    let actual: std::collections::BTreeSet<_> = include_str!("../geometry.rs")
        .lines()
        .filter(|line| line.starts_with("pub const "))
        .filter_map(|line| line.split_once("= \"").map(|(_, rest)| rest))
        .filter_map(|rest| rest.split_once('"').map(|(key, _)| key))
        .filter(|key| key.starts_with("plenora.geometry."))
        .collect();
    assert_eq!(actual, super::GEOMETRY_PROPERTIES.iter().copied().collect());
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

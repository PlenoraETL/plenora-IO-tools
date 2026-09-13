//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use std::collections::HashMap;

use arrow_schema::{DataType, Field};

use super::*;
use crate::contract::FieldId;
use crate::crs::CrsResolution;

fn contract() -> GeometryColumnContract {
    GeometryColumnContract::wkb_xy(FieldId(0), "geometry", CrsResolution::Missing, true)
}

fn field_with(key: &str, value: &str) -> Field {
    Field::new("geometry", DataType::Binary, true)
        .with_metadata(HashMap::from([(key.to_owned(), value.to_owned())]))
}

#[test]
fn missing_dimensions_preserve_base_and_report_absence() {
    let field = Field::new("geometry", DataType::Binary, true);
    let mut geometry = contract();

    let presence = read_geometry_contract_metadata(&field, &mut geometry).unwrap();

    assert!(!presence.has_dimensions());
    assert_eq!(geometry.dimensions, CoordinateDimensions::Xy);
}

#[test]
fn explicit_unknown_dimensions_are_preserved() {
    let field = field_with(PLENORA_DIMENSIONS_KEY, "unknown");
    let mut geometry = contract();

    let presence = read_geometry_contract_metadata(&field, &mut geometry).unwrap();

    assert!(presence.has_dimensions());
    assert_eq!(geometry.dimensions, CoordinateDimensions::Unknown);
}

#[test]
fn invalid_metadata_is_rejected_without_partial_mutation() {
    let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
        (PLENORA_ENCODING_KEY.to_owned(), "ewkb".to_owned()),
        (PLENORA_PRECISION_KEY.to_owned(), "binary128".to_owned()),
    ]));
    let mut geometry = contract();
    let original = geometry.clone();

    assert!(matches!(
        read_geometry_contract_metadata(&field, &mut geometry),
        Err(error) if error.category == crate::ErrorCategory::InvalidPlan
    ));
    assert_eq!(geometry.encoding, original.encoding);
    assert_eq!(geometry.precision, original.precision);
}

#[test]
fn invalid_srid_and_geometry_types_are_rejected() {
    for field in [
        field_with(PLENORA_FIELD_ID_KEY, "not-a-u32"),
        field_with(PLENORA_SRID_KEY, "not-an-i32"),
        field_with(PLENORA_GEOMETRY_TYPES_KEY, "point,futuregeometry"),
        field_with(PLENORA_GEOMETRY_TYPES_KEY, "point,point"),
        field_with(PLENORA_GEOMETRY_TYPES_KEY, "polygon,point"),
        field_with(PLENORA_GEOMETRY_TYPES_KEY, "line_string"),
        field_with(PLENORA_GEOMETRY_TYPES_KEY, "LineString"),
    ] {
        assert!(matches!(
            read_geometry_contract_metadata(&field, &mut contract()),
            Err(error) if error.category == crate::ErrorCategory::InvalidPlan
        ));
    }
}

#[test]
fn geometry_type_metadata_uses_canonical_names_without_separators() {
    let mut geometry = contract();
    geometry.types_declaration = TypesDeclaration::Exact;
    geometry.geometry_types = vec![
        GeometryType::LineString,
        GeometryType::MultiPolygon,
        GeometryType::GeometryCollection,
    ];

    let field =
        with_geometry_contract_metadata(&Field::new("geometry", DataType::Binary, true), &geometry);

    assert_eq!(
        field
            .metadata()
            .get(PLENORA_GEOMETRY_TYPES_KEY)
            .map(String::as_str),
        Some("linestring,multipolygon,geometrycollection")
    );

    let mut decoded = contract();
    read_geometry_contract_metadata(&field, &mut decoded).unwrap();
    assert_eq!(decoded.geometry_types, geometry.geometry_types);
}

#[test]
fn emits_and_reads_complete_crs_and_type_contract() {
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(17),
        "geometry",
        ResolvedCrs::new(
            Some("EPSG:4326".to_owned()),
            crate::crs::CrsKind::Geographic,
            Some("GEOGCRS[\"WGS 84\"]".to_owned()),
        ),
        true,
    );
    geometry.geometry_types = vec![GeometryType::Point];
    geometry.types_declaration = TypesDeclaration::Exact;
    let field =
        with_geometry_contract_metadata(&Field::new("geometry", DataType::Binary, true), &geometry);
    let metadata = field.metadata();
    assert_eq!(
        metadata.get(PLENORA_FIELD_ID_KEY).map(String::as_str),
        Some("17")
    );
    assert_eq!(
        metadata.get(PLENORA_CRS_ID_KEY).map(String::as_str),
        Some("EPSG:4326")
    );
    assert_eq!(
        metadata.get(PLENORA_CRS_RESOLUTION_KEY).map(String::as_str),
        Some("resolved")
    );
    assert_eq!(
        metadata
            .get(PLENORA_CRS_DEFINITION_FORMAT_KEY)
            .map(String::as_str),
        Some("wkt2")
    );
    assert_eq!(
        metadata.get(PLENORA_AXIS_ORDER_KEY).map(String::as_str),
        Some("lat_lon")
    );
    assert_eq!(
        metadata
            .get(PLENORA_TYPES_DECLARATION_KEY)
            .map(String::as_str),
        Some("exact")
    );

    let mut decoded = contract();
    read_geometry_contract_metadata(&field, &mut decoded).unwrap();
    // Nell'identita' dichiarata, non in `field_id`: il numero letto dal
    // file e' un'identita', e `field_id` resta la posizione che la nostra
    // enumerazione produce. Prima finivano nello stesso campo, e per
    // difendere l'indice si rifiutava un file conforme.
    assert_eq!(decoded.identita_dichiarata, Some(17));
    assert_eq!(
        decoded.field_id,
        contract().field_id,
        "la posizione fisica non viene dal payload"
    );
    assert_eq!(decoded.crs, geometry.crs);
    assert_eq!(decoded.types_declaration, TypesDeclaration::Exact);
}

#[test]
fn schema_contract_version_is_emitted_and_future_versions_fail_closed() {
    let schema = Arc::new(arrow_schema::Schema::new(vec![Field::new(
        "geometry",
        DataType::Binary,
        true,
    )]));
    let schema = with_contract_version(schema);
    assert_eq!(
        schema
            .metadata()
            .get(PLENORA_CONTRACT_VERSION_KEY)
            .map(String::as_str),
        Some("1")
    );
    validate_contract_version(schema.as_ref()).unwrap();

    let future = arrow_schema::Schema::new_with_metadata(
        Vec::<Field>::new(),
        HashMap::from([(PLENORA_CONTRACT_VERSION_KEY.to_owned(), "2".to_owned())]),
    );
    assert!(validate_contract_version(&future).is_err());
}

#[test]
fn legacy_missing_type_declaration_is_preserved_as_absence() {
    let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
        ARROW_EXTENSION_NAME_KEY.to_owned(),
        GEOARROW_WKB_EXTENSION.to_owned(),
    )]));
    let schema = Arc::new(arrow_schema::Schema::new(vec![field.clone()]));
    let mut geometry = contract();

    read_geometry_contract_metadata(&field, &mut geometry).unwrap();
    assert_eq!(
        geometry.types_declaration,
        TypesDeclaration::LegacyUndeclared
    );

    let data_contract = crate::contract::DataContract::new(schema.clone(), Some(geometry));
    assert!(data_contract
        .schema
        .metadata()
        .get(PLENORA_CONTRACT_VERSION_KEY)
        .is_none());
    assert_eq!(data_contract.schema, schema);
}

#[test]
fn unresolved_crs_preserves_definition_format_and_axis_order() {
    let mut geometry = contract();
    geometry.crs = CrsResolution::DeclaredButUnresolved(RawCrs {
        definition: Some("GEOGCRS[\"unresolved\"]".to_owned()),
        authority_hint: Some("EPSG:4326".to_owned()),
        definition_format: Some(CrsDefinitionFormat::Wkt2),
        axis_order: AxisOrder::LatitudeLongitude,
    });
    let field =
        with_geometry_contract_metadata(&Field::new("geometry", DataType::Binary, true), &geometry);
    let mut decoded = contract();
    read_geometry_contract_metadata(&field, &mut decoded).unwrap();
    assert_eq!(decoded.crs, geometry.crs);
}

#[test]
fn unresolved_crs_roundtrips_with_authority_only() {
    let mut geometry = contract();
    geometry.crs =
        CrsResolution::DeclaredButUnresolved(RawCrs::from_authority_hint("EPSG:99999".to_owned()));
    let field =
        with_geometry_contract_metadata(&Field::new("geometry", DataType::Binary, true), &geometry);

    assert_eq!(
        field.metadata().get(PLENORA_CRS_ID_KEY).map(String::as_str),
        Some("EPSG:99999")
    );
    assert!(!field.metadata().contains_key(PLENORA_CRS_DEFINITION_KEY));
    assert!(!field
        .metadata()
        .contains_key(PLENORA_CRS_DEFINITION_FORMAT_KEY));

    let mut decoded = contract();
    read_geometry_contract_metadata(&field, &mut decoded).unwrap();
    assert_eq!(decoded.crs, geometry.crs);
}

#[test]
fn unresolved_crs_roundtrips_with_srid_only_without_synthesis() {
    let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
        (
            PLENORA_CRS_RESOLUTION_KEY.to_owned(),
            "declared_unresolved".to_owned(),
        ),
        (PLENORA_SRID_KEY.to_owned(), "4326".to_owned()),
    ]));
    let mut geometry = contract();

    read_geometry_contract_metadata(&field, &mut geometry).unwrap();

    assert_eq!(geometry.srid, Some(4326));
    let raw = geometry.crs.raw().unwrap();
    assert_eq!(raw.authority_hint, None);
    assert_eq!(raw.definition, None);
    assert_eq!(raw.definition_format, None);
    assert_eq!(raw.axis_order, AxisOrder::Unknown);

    let emitted =
        with_geometry_contract_metadata(&Field::new("geometry", DataType::Binary, true), &geometry);
    assert_eq!(
        emitted.metadata().get(PLENORA_SRID_KEY).map(String::as_str),
        Some("4326")
    );
    for key in [
        PLENORA_CRS_ID_KEY,
        PLENORA_CRS_DEFINITION_KEY,
        PLENORA_CRS_DEFINITION_FORMAT_KEY,
        PLENORA_AXIS_ORDER_KEY,
    ] {
        assert!(
            !emitted.metadata().contains_key(key),
            "chiave sintetizzata: {key}"
        );
    }
}

#[test]
fn resolved_crs_with_srid_only_is_rejected() {
    let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
        (PLENORA_CRS_RESOLUTION_KEY.to_owned(), "resolved".to_owned()),
        (PLENORA_SRID_KEY.to_owned(), "4326".to_owned()),
    ]));

    assert!(read_geometry_contract_metadata(&field, &mut contract()).is_err());
}

#[test]
fn unresolved_crs_with_structured_representation_requires_axis_order() {
    for key in [PLENORA_CRS_ID_KEY, PLENORA_CRS_DEFINITION_KEY] {
        let mut metadata = HashMap::from([
            (
                PLENORA_CRS_RESOLUTION_KEY.to_owned(),
                "declared_unresolved".to_owned(),
            ),
            (key.to_owned(), "EPSG:4326".to_owned()),
        ]);
        if key == PLENORA_CRS_DEFINITION_KEY {
            metadata.insert(
                PLENORA_CRS_DEFINITION_FORMAT_KEY.to_owned(),
                "wkt".to_owned(),
            );
        }
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(metadata);

        assert!(read_geometry_contract_metadata(&field, &mut contract()).is_err());
    }
}

#[test]
fn axis_order_without_structured_crs_is_rejected() {
    for (resolution, srid) in [("declared_unresolved", Some("4326")), ("missing", None)] {
        let mut metadata = HashMap::from([
            (PLENORA_CRS_RESOLUTION_KEY.to_owned(), resolution.to_owned()),
            (PLENORA_AXIS_ORDER_KEY.to_owned(), "lon_lat".to_owned()),
        ]);
        if let Some(srid) = srid {
            metadata.insert(PLENORA_SRID_KEY.to_owned(), srid.to_owned());
        }
        let field = Field::new("geometry", DataType::Binary, true).with_metadata(metadata);

        assert!(
            read_geometry_contract_metadata(&field, &mut contract()).is_err(),
            "`{resolution}` senza crs_id/definition ha accettato axis_order"
        );
    }
}

#[test]
fn unresolved_crs_without_any_declaration_is_rejected() {
    let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([
        (
            PLENORA_CRS_RESOLUTION_KEY.to_owned(),
            "declared_unresolved".to_owned(),
        ),
        (PLENORA_AXIS_ORDER_KEY.to_owned(), "unknown".to_owned()),
    ]));

    assert!(read_geometry_contract_metadata(&field, &mut contract()).is_err());
}

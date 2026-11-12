//! Fixture native: lo schema e il contratto nascono dalla stessa dichiarazione.
//! Compilato e provato anche senza GDAL; nessuna riparazione al bordo writer.
use arrow_schema::SchemaRef;
use plenora_io_model::contract::{DataContract, GeometryColumnContract};

pub fn declared(schema: &SchemaRef, geometry: GeometryColumnContract) -> DataContract {
    let schema = plenora_io_model::geometry::schema_from_native_fields(schema, &geometry);
    let contract = DataContract::new(schema, Some(geometry));
    plenora_io_model::primary::reconcile_schema_contract(&contract)
        .expect("fixture nativa coerente prima del writer");
    contract
}

#[test]
fn reader_wkt_without_authority_is_not_emitted_as_an_identifier() {
    use arrow_schema::Schema;
    use plenora_io_model::contract::{FieldId, GeometryType};
    use plenora_io_model::crs::{CrsKind, ResolvedCrs};
    use plenora_io_model::geometry::{
        with_contract_version, with_geometry_contract_metadata, GEO_CRS_KEY,
    };
    use std::sync::Arc;

    let wkt =
        r#"LOCAL_CS["site",LOCAL_DATUM["d",0],UNIT["metre",1],AXIS["X",EAST],AXIS["Y",NORTH]]"#;
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geometry",
        ResolvedCrs::new(None, CrsKind::Unknown, Some(wkt.to_owned())).stored_east_first(),
        true,
    );
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let old_field =
        with_geometry_contract_metadata(&driver_common::geometry_field("geometry", wkt), &geometry);
    let old = DataContract::new(
        with_contract_version(Arc::new(Schema::new(vec![old_field]))),
        Some(geometry.clone()),
    );
    assert!(old.geometries().is_err());

    // Questa e' la stessa emissione usata dal reader GDAL, non una fixture alternativa.
    let field = super::native_geometry_field(&geometry);
    assert!(!field.metadata().contains_key(GEO_CRS_KEY));
    let contract = DataContract::new(
        with_contract_version(Arc::new(Schema::new(vec![field]))),
        Some(geometry),
    );
    let read = contract.geometries().unwrap();
    assert_eq!(
        read[0].resolved_crs().unwrap().definition.as_deref(),
        Some(wkt)
    );
    assert_eq!(read[0].crs.id(), None);
}

#[test]
fn native_contracts_reconcile_and_project_without_gdal() {
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_core::{project_layer_contract, ReadRequest, WriteLayer, WritePlan};
    use plenora_io_model::contract::{
        CoordinateDimensions, FieldId, GeometryType, LayerContract, LayerId,
    };
    use plenora_io_model::crs::{CrsKind, ResolvedCrs};

    for dimensions in [
        CoordinateDimensions::Xy,
        CoordinateDimensions::Xyz,
        CoordinateDimensions::Xym,
        CoordinateDimensions::Xyzm,
    ] {
        for family in [
            GeometryType::Point,
            GeometryType::MultiPoint,
            GeometryType::MultiLineString,
            GeometryType::MultiPolygon,
        ] {
            let mut geometry = GeometryColumnContract::wkb_xy(
                FieldId(0),
                "geometry",
                ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
                true,
            );
            geometry.dimensions = dimensions;
            geometry.set_exact_geometry_types(vec![family]);
            geometry
                .native_metadata
                .insert("filegdb.geometry_field_count".to_owned(), "1".to_owned());
            let attribute = Field::new("label", DataType::Utf8, true).with_metadata(
                std::iter::once(("plenora.filegdb.ogr_field_type".to_owned(), "4".to_owned()))
                    .collect::<arrow_schema::Metadata>(),
            );
            let schema = Arc::new(Schema::new(vec![
                driver_common::geometry_field("geometry", "EPSG:3857"),
                attribute.clone(),
            ]));
            // La vecchia fixture e' rifiutata anche senza GDAL.
            let incomplete = DataContract::new(schema.clone(), Some(geometry.clone()));
            assert!(incomplete.geometries().is_err());
            let emitted = super::native_geometry_field(&geometry);
            let contract = declared(&schema, geometry);
            assert_eq!(contract.schema.field(0), &emitted);
            assert_eq!(contract.geometries().unwrap()[0].dimensions, dimensions);
            assert_eq!(
                contract.geometries().unwrap()[0].geometry_types,
                vec![family]
            );
            // Piano Rust esplicito e piano derivato dallo stesso Arrow (CLI/SDK).
            for candidate in [
                contract.clone(),
                DataContract::from_schema(contract.schema.clone()).unwrap(),
            ] {
                let plan = WritePlan {
                    layers: vec![WriteLayer {
                        name: "native".to_owned(),
                        contract: candidate,
                    }],
                };
                plenora_io_core::validate_write(
                    &super::DESCRIPTOR,
                    &plan,
                    16,
                    &std::collections::BTreeMap::default(),
                )
                .unwrap();
            }
            let layer = LayerContract {
                id: LayerId(0),
                name: "native".to_owned(),
                contract,
            };
            for indices in [vec![FieldId(0), FieldId(1)], vec![FieldId(1)], vec![]] {
                let request = ReadRequest {
                    projected_fields: Some(indices.clone()),
                    layer: LayerId(0),
                    projection_mode: plenora_io_core::request::ProjectionMode::Required,
                    pruning_predicate: None,
                    spatial_pruning_hint: None,
                    scope: plenora_io_core::request::ReadScope::default(),
                    batch_target: plenora_io_core::request::BatchTarget::default(),
                    cancellation: plenora_io_model::CancellationToken::default(),
                };
                let (_, projected) = project_layer_contract(&layer, &request).unwrap();
                projected.contract.geometries().unwrap();
                assert_eq!(
                    projected.contract.geometry.is_some(),
                    indices.contains(&FieldId(0))
                );
                if let Some(position) = indices.iter().position(|id| *id == FieldId(1)) {
                    assert_eq!(
                        projected.contract.schema.field(position).metadata(),
                        attribute.metadata()
                    );
                }
            }
        }
    }
}

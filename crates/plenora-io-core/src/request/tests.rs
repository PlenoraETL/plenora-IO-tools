//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use std::collections::HashMap;

use arrow_schema::{DataType, Field, Schema};
use plenora_io_model::crs::CrsResolution;

use super::*;

#[test]
fn target_bytes_reduces_rows_but_never_below_one() {
    let schema = Schema::new(vec![
        Field::new("name", DataType::Utf8, true),
        Field::new("value", DataType::Int64, false),
    ]);
    assert_eq!(
        effective_batch_rows(
            &schema,
            BatchTarget {
                target_bytes: 720,
                max_rows: 100,
            },
        ),
        10
    );
    assert_eq!(
        effective_batch_rows(
            &schema,
            BatchTarget {
                target_bytes: 0,
                max_rows: 0,
            },
        ),
        1
    );
}

#[test]
fn geometry_uses_a_conservative_variable_width_estimate() {
    let field = Field::new("geometry", DataType::Binary, true).with_metadata(HashMap::from([(
        plenora_io_model::geometry::ARROW_EXTENSION_NAME_KEY.to_owned(),
        plenora_io_model::geometry::GEOARROW_WKB_EXTENSION.to_owned(),
    )]));
    let schema = Schema::new(vec![field]);
    assert_eq!(
        effective_batch_rows(
            &schema,
            BatchTarget {
                target_bytes: 1024,
                max_rows: 100,
            },
        ),
        2
    );
}

#[test]
fn observed_batch_sizing_respects_both_limits() {
    let target = BatchTarget {
        target_bytes: 1_000,
        max_rows: 100,
    };
    assert_eq!(observed_batch_rows(20, 4_000, target), 5);
    assert_eq!(observed_batch_rows(20, 10, target), 100);
    assert_eq!(observed_batch_rows(20, usize::MAX, target), 1);
    assert_eq!(observed_batch_rows(0, 0, target), 100);
}

#[test]
fn exact_projection_is_deduplicated_ordered_and_fail_closed() {
    let source = LayerContract {
        id: LayerId(7),
        name: "source".to_owned(),
        contract: DataContract::new(
            Arc::new(Schema::new(vec![
                Field::new("a", DataType::Int64, false),
                Field::new("b", DataType::Utf8, true),
                Field::new("c", DataType::Float64, false),
            ])),
            None,
        ),
    };
    let request = ReadRequest {
        layer: source.id,
        projected_fields: Some(vec![FieldId(2), FieldId(0), FieldId(2)]),
        projection_mode: ProjectionMode::Required,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::Complete,
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    };
    let (indices, projected) = match project_layer_contract(&source, &request) {
        Ok(projected) => projected,
        Err(error) => panic!("projection valida rifiutata: {error}"),
    };
    assert_eq!(indices, vec![0, 2]);
    assert_eq!(projected.contract.schema.field(0).name(), "a");
    assert_eq!(projected.contract.schema.field(1).name(), "c");

    let invalid = ReadRequest {
        projected_fields: Some(vec![FieldId(3)]),
        ..request
    };
    assert!(project_layer_contract(&source, &invalid).is_err());

    let geometry_source = LayerContract {
        contract: DataContract::new(
            Arc::new(Schema::new(vec![
                Field::new("a", DataType::Int64, false),
                Field::new("geometry", DataType::Binary, true),
                Field::new("c", DataType::Float64, false),
            ])),
            Some(GeometryColumnContract::wkb_xy(
                FieldId(1),
                "geometry",
                CrsResolution::Missing,
                true,
            )),
        ),
        ..source
    };
    let geometry_request = ReadRequest {
        layer: geometry_source.id,
        projected_fields: Some(vec![FieldId(2), FieldId(1)]),
        projection_mode: ProjectionMode::Required,
        pruning_predicate: None,
        spatial_pruning_hint: None,
        scope: ReadScope::Complete,
        batch_target: BatchTarget::default(),
        cancellation: CancellationToken::default(),
    };
    let projected = match project_layer_contract(&geometry_source, &geometry_request) {
        Ok((_, projected)) => projected,
        Err(error) => panic!("projection geometrica valida rifiutata: {error}"),
    };
    assert_eq!(
        projected
            .contract
            .geometry
            .map(|geometry| geometry.field_id),
        Some(FieldId(0))
    );
}

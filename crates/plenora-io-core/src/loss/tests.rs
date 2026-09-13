//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};
use plenora_io_model::contract::{DataContract, FieldId, GeometryColumnContract, LayerId};
use plenora_io_model::crs::{CrsKind, RawCrs, ResolvedCrs};

use super::*;

fn layer_with_geometry(geometry: GeometryColumnContract) -> LayerContract {
    LayerContract {
        id: LayerId(0),
        name: "parcels".to_owned(),
        contract: DataContract::new(
            Arc::new(Schema::new(vec![Field::new(
                "geom",
                DataType::Binary,
                true,
            )])),
            Some(geometry),
        ),
    }
}

#[test]
fn observed_loss_promotes_assessment_and_stays_bounded() {
    let mut report = LossReport::default();
    for index in 0..(MAX_FIDELITY_REASONS + 10) {
        report.record(&format!("category-{index}"), 1);
    }
    let assessment =
        FidelityAssessment::for_format("test", Fidelity::Conditional).with_loss_report(&report);
    assert_eq!(assessment.level, Fidelity::Approximating);
    assert_eq!(assessment.ragioni_v1().len(), MAX_FIDELITY_REASONS);
    assert!(assessment
        .ragioni_v1()
        .iter()
        .any(|reason| reason.code == FidelityReasonCode::LossReported));
}

#[test]
fn lossless_assessment_has_no_reasons() {
    assert_eq!(
        FidelityAssessment::for_format("ipc", Fidelity::Lossless),
        FidelityAssessment::lossless()
    );
}

#[test]
fn definition_and_srid_disagreement_is_declared_once() {
    let definition = concat!(
        "PROJCS[\"Monte Mario / Italy zone 1\",",
        "GEOGCS[\"Monte Mario\",AUTHORITY[\"EPSG\",\"4265\"]],",
        "AUTHORITY[\"EPSG\",\"3003\"]]"
    );
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geom",
        ResolvedCrs::new(None, CrsKind::Projected, Some(definition.to_owned())),
        true,
    );
    geometry.srid = Some(4326);
    let layer = layer_with_geometry(geometry);
    let mut report = LossReport::default();

    declare_crs_inconsistency(&layer, &mut report);
    declare_crs_inconsistency(&layer, &mut report);

    assert_eq!(report.counts[INCONSISTENT_CRS_REPRESENTATIONS], 1);
    assert!(report
        .esempi_canonici()
        .next()
        .expect("un esempio")
        .context
        .contains("definition_epsg=3003"));
}

#[test]
fn unresolved_definition_and_authority_disagreement_is_declared() {
    let raw = RawCrs::new(
        "GEOGCS[\"WGS 84\",AUTHORITY[\"EPSG\",\"4326\"]]".to_owned(),
        Some("EPSG:3003".to_owned()),
    );
    let geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geom",
        CrsResolution::DeclaredButUnresolved(raw),
        true,
    );
    let layer = layer_with_geometry(geometry);
    let mut report = LossReport::default();

    declare_crs_inconsistency(&layer, &mut report);

    assert_eq!(report.counts[INCONSISTENT_CRS_REPRESENTATIONS], 1);
}

//! Le prove di [`super`], in un file loro.
//!
//! Restano un **modulo figlio**: vedono i privati del genitore
//! esattamente come quando stavano dentro di lui, e non allargano di
//! una riga la superficie pubblica del crate.

use std::sync::Arc;

/// Tetto di colonne dei test, dai limiti della pipeline.
///
/// Era `Limits::default().max_columns`: il tipo legacy non esiste piu' nel
/// percorso core (S4.e).
fn colonne_predefinite() -> usize {
    usize::try_from(plenora_io_model::budget::PipelineLimits::default().max_columns())
        .unwrap_or(usize::MAX)
}

use arrow_schema::{DataType, Field, Schema};
use plenora_io_model::contract::{DataContract, FieldId, GeometryColumnContract, GeometryType};
use plenora_io_model::crs::{CrsKind, ResolvedCrs};

use super::*;
use crate::descriptor::{
    CrsDerivation, CrsRepresentationCapabilities, Direction, Fidelity, FormatWriteCapabilities,
    ReadMode, ReaderConcurrency, Runtime, WriteMode, DBF_FIELD_NAMES, SCALAR_TYPES,
    WKB_XY_GEOMETRY,
};
use crate::request::WriteLayer;

/// I test di questo modulo verificano le capability, non le opzioni: il
/// descrittore di prova dichiara schema vuoto, quindi la mappa vuota e'
/// l'unica che lo soddisfa.
fn senza_opzioni() -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::new()
}

fn descriptor(crs: CrsWriteSupport) -> FormatDescriptor {
    FormatDescriptor::const_new(
        "test",
        Direction::Bidirectional,
        ReadMode::StreamingSequential,
        // I tre assi di INV-7: il descrittore di prova dichiara la
        // combinazione che tutti i driver reali dichiarano.
        crate::descriptor::NativeReadMode::StreamingSequential,
        crate::descriptor::DeliverySemantics::OperationAtomic,
        crate::descriptor::BufferingStrategy::AdaptiveMemoryThenDisk,
        crate::descriptor::DeterminismLevel::Semantic,
        Some(WriteMode::Streaming),
        Some(crate::descriptor::DeterminismLevel::Semantic),
        false,
        false,
        ReaderConcurrency::MultipleIndependentReaders,
        crate::descriptor::ProjectionSupport::None,
        crate::descriptor::PredicatePruningSupport::None,
        crate::descriptor::SpatialPruningSupport::None,
        crate::descriptor::CrsHandling::Embedded,
        Fidelity::Conditional,
        Runtime::PureRust,
        // `hostile_input_hardened`: un descrittore di prova non parla di
        // input ostile: dichiara il valore che non afferma niente.
        false,
        // `spec_version_supported`: un descrittore di prova non parla di
        // nessun formato reale, quindi non ne dichiara la versione.
        None,
        Some(FormatWriteCapabilities {
            field_names: DBF_FIELD_NAMES,
            allowed_types: SCALAR_TYPES,
            type_coercion: TypeCoercionPolicy::Reject,
            attributes: AttributeWriteSupport::All,
            geometry: WKB_XY_GEOMETRY,
            crs,
            crs_representations: CrsRepresentationCapabilities::new(
                CrsRepresentationState::Preserved,
                CrsRepresentationState::Preserved,
                CrsRepresentationState::Preserved,
            ),
            nullability: NullabilitySupport::Preserve,
            multi_layer: false,
            sink_path: crate::SinkPathConstraint::Free,
        }),
        plenora_io_model::format_options::SchemaOpzioniFormato::VUOTO,
        &["test"],
        1,
        1,
        1,
    )
}

fn plan(fields: Vec<Field>, geometry: Option<GeometryColumnContract>) -> WritePlan {
    WritePlan {
        layers: vec![WriteLayer {
            name: "layer".to_owned(),
            contract: DataContract {
                schema: Arc::new(Schema::new(fields)),
                geometry,
            },
        }],
    }
}

#[test]
fn rejects_field_name_before_writer_creation() {
    let p = plan(
        vec![Field::new("field_name_too_long", DataType::Utf8, true)],
        None,
    );
    let error = validate_write(
        &descriptor(CrsWriteSupport::None),
        &p,
        colonne_predefinite(),
        &senza_opzioni(),
    )
    .unwrap_err();
    assert_eq!(
        error.capability_reason,
        Some(CapabilityReason::FieldNameTooLong)
    );
}

#[test]
fn fixed_crs_requires_explicit_reprojection() {
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geom",
        ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None),
        true,
    );
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let p = plan(
        vec![Field::new("geom", DataType::Binary, true)],
        Some(geometry),
    );
    let error = validate_write(
        &descriptor(CrsWriteSupport::Fixed("OGC:CRS84")),
        &p,
        colonne_predefinite(),
        &senza_opzioni(),
    )
    .unwrap_err();
    assert_eq!(
        error.capability_reason,
        Some(CapabilityReason::ReprojectionRequired)
    );
}

#[test]
fn enforces_contract_column_limit() {
    let p = plan(
        vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Int64, false),
        ],
        None,
    );

    assert!(matches!(
        validate_write(&descriptor(CrsWriteSupport::None), &p, 1, &senza_opzioni()),
        Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
    ));
}

#[test]
fn attribute_none_rejects_every_non_geometry_field() {
    let descriptor = descriptor(CrsWriteSupport::None);
    let mut capabilities = descriptor.write_capabilities().unwrap();
    capabilities.attributes = AttributeWriteSupport::None;
    let descriptor = descriptor.con_write_capabilities(Some(capabilities));
    let p = plan(vec![Field::new("attribute", DataType::Utf8, false)], None);

    assert!(matches!(
        validate_write(&descriptor, &p, colonne_predefinite(), &senza_opzioni()),
        Err(error)
            if error.capability_reason == Some(CapabilityReason::TypeNotRepresentable)
    ));
}

#[test]
fn named_attribute_subset_accepts_only_the_declared_names() {
    static ALLOWED: &[&str] = &["name"];
    let descriptor = descriptor(CrsWriteSupport::None);
    let mut capabilities = descriptor.write_capabilities().unwrap();
    capabilities.attributes = AttributeWriteSupport::NamedSubset(ALLOWED);
    let descriptor = descriptor.con_write_capabilities(Some(capabilities));

    let accepted = plan(vec![Field::new("name", DataType::Utf8, false)], None);
    assert!(validate_write(
        &descriptor,
        &accepted,
        colonne_predefinite(),
        &senza_opzioni()
    )
    .is_ok());

    let rejected = plan(vec![Field::new("secret", DataType::Utf8, false)], None);
    assert!(matches!(
        validate_write(&descriptor, &rejected, colonne_predefinite(), &senza_opzioni()),
        Err(error)
            if error.capability_reason == Some(CapabilityReason::TypeNotRepresentable)
    ));
}

#[test]
fn no_nulls_rejects_nullable_contract_fields() {
    let descriptor = descriptor(CrsWriteSupport::None);
    let mut capabilities = descriptor.write_capabilities().unwrap();
    capabilities.nullability = NullabilitySupport::NoNulls;
    let descriptor = descriptor.con_write_capabilities(Some(capabilities));
    let p = plan(vec![Field::new("required", DataType::Utf8, true)], None);

    assert!(matches!(
        validate_write(&descriptor, &p, colonne_predefinite(), &senza_opzioni()),
        Err(error) if error.capability_reason == Some(CapabilityReason::Nullability)
    ));
}

#[test]
fn duplicate_geometry_types_are_rejected() {
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geom",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        true,
    );
    geometry.geometry_types = vec![GeometryType::Point, GeometryType::Point];
    let p = plan(
        vec![Field::new("geom", DataType::Binary, true)],
        Some(geometry),
    );

    assert!(matches!(
        validate_write(
            &descriptor(CrsWriteSupport::Embedded),
            &p,
            colonne_predefinite(), &senza_opzioni()),
        Err(error) if error.capability_reason == Some(CapabilityReason::MixedGeometry)
    ));
}

#[test]
fn inconsistent_crs_requires_independent_preservation_of_id_and_srid() {
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geom",
        ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None),
        true,
    );
    geometry.srid = Some(3003);
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let p = plan(
        vec![Field::new("geom", DataType::Binary, true)],
        Some(geometry),
    );

    assert!(validate_write(
        &descriptor(CrsWriteSupport::EmbeddedOptional),
        &p,
        colonne_predefinite(),
        &senza_opzioni()
    )
    .is_ok());

    let selecting = descriptor(CrsWriteSupport::Embedded);
    let mut capabilities = selecting.write_capabilities().unwrap();
    capabilities.crs_representations.srid =
        CrsRepresentationState::Derived(CrsDerivation::FromIdentifier);
    let selecting = selecting.con_write_capabilities(Some(capabilities));
    let error =
        validate_write(&selecting, &p, colonne_predefinite(), &senza_opzioni()).unwrap_err();
    assert_eq!(
        error.capability_reason,
        Some(CapabilityReason::CrsRepresentationsInconsistent)
    );
    assert_eq!(error.phase, plenora_io_model::ErrorPhase::Validate);
    assert_eq!(error.remote_effect, plenora_io_model::RemoteEffect::None);
    assert_eq!(error.retry, plenora_io_model::RetryDisposition::Never);
}

#[test]
fn inconsistent_definition_requires_independent_preservation() {
    let definition = concat!(
        "PROJCS[\"Monte Mario / Italy zone 1\",",
        "GEOGCS[\"Monte Mario\",AUTHORITY[\"EPSG\",\"4265\"]],",
        "AUTHORITY[\"EPSG\",\"3003\"]]"
    );
    let mut geometry = GeometryColumnContract::wkb_xy(
        FieldId(0),
        "geom",
        ResolvedCrs::new(
            Some("EPSG:3003".to_owned()),
            CrsKind::Projected,
            Some(definition.to_owned()),
        ),
        true,
    );
    geometry.srid = Some(4326);
    geometry.set_exact_geometry_types(vec![GeometryType::Point]);
    let p = plan(
        vec![Field::new("geom", DataType::Binary, true)],
        Some(geometry),
    );

    let preserving = descriptor(CrsWriteSupport::EmbeddedOptional);
    assert!(validate_write(&preserving, &p, colonne_predefinite(), &senza_opzioni()).is_ok());

    let selecting = descriptor(CrsWriteSupport::Embedded);
    let mut capabilities = selecting.write_capabilities().unwrap();
    capabilities.crs_representations.crs_definition =
        CrsRepresentationState::Derived(CrsDerivation::FromIdentifier);
    let selecting = selecting.con_write_capabilities(Some(capabilities));
    let error =
        validate_write(&selecting, &p, colonne_predefinite(), &senza_opzioni()).unwrap_err();
    assert_eq!(
        error.capability_reason,
        Some(CapabilityReason::CrsRepresentationsInconsistent)
    );
}

#[test]
fn known_crs_values_disagree_ignores_missing_values_and_order() {
    assert!(!known_crs_values_disagree([None, None, None]));
    assert!(!known_crs_values_disagree([Some(4_326), None, Some(4_326)]));
    assert!(known_crs_values_disagree([Some(4_326), None, Some(3_003)]));
    assert!(known_crs_values_disagree([Some(3_003), Some(4_326), None]));
}

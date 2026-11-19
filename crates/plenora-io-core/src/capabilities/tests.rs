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
use plenora_io_model::contract::{FieldId, GeometryColumnContract, GeometryType};
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
        &crate::dataset_files::FILE_SETS[4],
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
            contract: crate::test_contract::declared(Arc::new(Schema::new(fields)), geometry),
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
        Err(error) if error.category == plenora_io_model::ErrorCategory::Schema
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

/// Every physical Arrow variant belongs to exactly one public family.
/// The production match has no wildcard, so new Arrow variants also fail compilation.
#[test]
fn every_arrow_variant_has_one_family() {
    use arrow_schema::{IntervalUnit, TimeUnit, UnionFields, UnionMode};
    use ArrowTypeClass as C;
    use DataType as D;
    let field = Arc::new(Field::new("item", D::Int32, true));
    let cases = vec![
        (D::Null, C::Other),
        (D::Boolean, C::Boolean),
        (D::Int8, C::SignedInteger),
        (D::Int16, C::SignedInteger),
        (D::Int32, C::SignedInteger),
        (D::Int64, C::SignedInteger),
        (D::UInt8, C::UnsignedInteger),
        (D::UInt16, C::UnsignedInteger),
        (D::UInt32, C::UnsignedInteger),
        (D::UInt64, C::UnsignedInteger),
        (D::Float16, C::Floating),
        (D::Float32, C::Floating),
        (D::Float64, C::Floating),
        (D::Timestamp(TimeUnit::Second, None), C::Temporal),
        (D::Date32, C::Temporal),
        (D::Date64, C::Temporal),
        (D::Time32(TimeUnit::Second), C::Temporal),
        (D::Time64(TimeUnit::Nanosecond), C::Temporal),
        (D::Duration(TimeUnit::Second), C::Temporal),
        (D::Interval(IntervalUnit::YearMonth), C::Temporal),
        (D::Binary, C::Binary),
        (D::FixedSizeBinary(1), C::Binary),
        (D::LargeBinary, C::Binary),
        (D::BinaryView, C::Binary),
        (D::Utf8, C::Utf8),
        (D::LargeUtf8, C::Utf8),
        (D::Utf8View, C::Utf8),
        (D::List(field.clone()), C::Nested),
        (D::ListView(field.clone()), C::Nested),
        (D::FixedSizeList(field.clone(), 1), C::Nested),
        (D::LargeList(field.clone()), C::Nested),
        (D::LargeListView(field.clone()), C::Nested),
        (D::Struct(vec![field.clone()].into()), C::Nested),
        (D::Union(UnionFields::empty(), UnionMode::Sparse), C::Nested),
        (
            D::Dictionary(Box::new(D::Int32), Box::new(D::Utf8)),
            C::Other,
        ),
        (D::Decimal32(9, 0), C::Decimal),
        (D::Decimal64(18, 0), C::Decimal),
        (D::Decimal128(38, 0), C::Decimal),
        (D::Decimal256(76, 0), C::Decimal),
        (D::Map(field.clone(), false), C::Nested),
        (D::RunEndEncoded(field.clone(), field), C::Other),
    ];
    assert_eq!(cases.len(), 41);
    let mut seen = std::collections::HashSet::new();
    for (data_type, family) in cases {
        assert!(seen.insert(std::mem::discriminant(&data_type)));
        assert_eq!(arrow_type_class(&data_type), family, "{data_type:?}");
    }
}

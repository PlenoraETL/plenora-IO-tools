//! Validazione comune del `WritePlan` contro capability machine-readable
//! (ADR-IO 3). I driver possono aggiungere vincoli specifici, ma non saltare
//! questi controlli di base.

use std::collections::BTreeSet;

use arrow_schema::DataType;
use plenora_io_model::crs::{definition_authority_srid, CrsResolution};
use plenora_io_model::limits::Limits;
use plenora_io_model::{CapabilityReason, PlenoraIoError, Result};

use crate::descriptor::{
    ArrowTypeClass, AttributeWriteSupport, CrsRepresentationState, CrsWriteSupport,
    FormatDescriptor, NullabilitySupport, TextEncoding, TypeCoercionPolicy, ALL_GEOMETRY_TYPES,
};
use crate::request::WritePlan;

fn violation(
    driver: &'static str,
    field: Option<&str>,
    reason: CapabilityReason,
    detail: impl Into<String>,
) -> PlenoraIoError {
    PlenoraIoError::capability(driver, field.map(str::to_owned), reason, detail)
}

fn declared_crs_id(crs: &CrsResolution) -> Option<&str> {
    match crs {
        CrsResolution::Resolved(resolved) => resolved.id.as_deref(),
        CrsResolution::DeclaredButUnresolved(raw) => raw.authority_hint.as_deref(),
        CrsResolution::Missing => None,
    }
}

fn comparable_crs_representations(
    geometry: &plenora_io_model::contract::GeometryColumnContract,
) -> (Option<i64>, Option<i64>, Option<i64>) {
    let (definition, definition_format) = match &geometry.crs {
        CrsResolution::Resolved(resolved) => {
            (resolved.definition.as_deref(), resolved.definition_format)
        }
        CrsResolution::DeclaredButUnresolved(raw) => {
            (raw.definition.as_deref(), raw.definition_format)
        }
        CrsResolution::Missing => (None, None),
    };
    (
        declared_crs_id(&geometry.crs)
            .and_then(plenora_io_model::crs::authority_srid)
            .map(i64::from),
        geometry.srid.map(i64::from),
        definition
            .zip(definition_format)
            .and_then(|(value, format)| definition_authority_srid(value, format))
            .map(i64::from),
    )
}

fn crs_representations_are_inconsistent(
    geometry: &plenora_io_model::contract::GeometryColumnContract,
) -> bool {
    let (crs_id, srid, definition) = comparable_crs_representations(geometry);
    known_crs_values_disagree([crs_id, srid, definition])
}

pub(crate) fn known_crs_values_disagree(values: [Option<i64>; 3]) -> bool {
    let mut known = values.into_iter().flatten();
    let Some(first) = known.next() else {
        return false;
    };
    known.any(|value| value != first)
}

pub fn arrow_type_class(data_type: &DataType) -> ArrowTypeClass {
    use DataType::*;
    match data_type {
        Boolean => ArrowTypeClass::Boolean,
        Int8 | Int16 | Int32 | Int64 => ArrowTypeClass::SignedInteger,
        UInt8 | UInt16 | UInt32 | UInt64 => ArrowTypeClass::UnsignedInteger,
        Float16 | Float32 | Float64 => ArrowTypeClass::Floating,
        Utf8 | LargeUtf8 | Utf8View => ArrowTypeClass::Utf8,
        Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => ArrowTypeClass::Binary,
        Date32 | Date64 | Time32(_) | Time64(_) | Timestamp(_, _) | Duration(_) | Interval(_) => {
            ArrowTypeClass::Temporal
        }
        Decimal32(_, _) | Decimal64(_, _) | Decimal128(_, _) | Decimal256(_, _) => {
            ArrowTypeClass::Decimal
        }
        List(_)
        | ListView(_)
        | FixedSizeList(_, _)
        | LargeList(_)
        | LargeListView(_)
        | Struct(_)
        | Union(_, _)
        | Map(_, _) => ArrowTypeClass::Nested,
        _ => ArrowTypeClass::Other,
    }
}

/// Valida tutto ciò che è determinabile dal contratto statico. Vincoli che
/// richiedono valori (per esempio tipo geometrico ignoto fino al primo record)
/// restano una seconda guardia nel writer.
pub fn validate_write(
    descriptor: &FormatDescriptor,
    plan: &WritePlan,
    limits: &Limits,
) -> Result<()> {
    let driver = descriptor.id;
    let caps = descriptor.write_capabilities.as_ref().ok_or_else(|| {
        violation(
            driver,
            None,
            CapabilityReason::TypeNotRepresentable,
            "driver scrivibile senza capability dichiarate",
        )
    })?;

    if plan.layers.is_empty() {
        return Err(violation(
            driver,
            None,
            CapabilityReason::EmptyWritePlan,
            "WritePlan senza layer",
        ));
    }
    if !caps.multi_layer && plan.layers.len() != 1 {
        return Err(violation(
            driver,
            None,
            CapabilityReason::MultipleLayers,
            format!("atteso un layer, ricevuti {}", plan.layers.len()),
        ));
    }

    let mut layer_names = BTreeSet::new();
    for layer in &plan.layers {
        if layer.name.is_empty() || !layer_names.insert(layer.name.clone()) {
            return Err(violation(
                driver,
                None,
                CapabilityReason::DuplicateLayerName,
                format!("nome layer vuoto o duplicato: '{}'", layer.name),
            ));
        }
        if layer.contract.schema.fields().len() > limits.max_columns {
            return Err(PlenoraIoError::LimitExceeded(format!(
                "layer '{}' con {} colonne oltre il limite di {}",
                layer.name,
                layer.contract.schema.fields().len(),
                limits.max_columns
            )));
        }

        let geometry_name = layer
            .contract
            .geometry
            .as_ref()
            .map(|geometry| geometry.name.as_str());
        let mut normalized_names = BTreeSet::new();
        for field in layer.contract.schema.fields() {
            let name = field.name();
            if caps
                .field_names
                .max_bytes
                .is_some_and(|limit| name.len() > limit)
                || caps
                    .field_names
                    .max_chars
                    .is_some_and(|limit| name.chars().count() > limit)
            {
                return Err(violation(
                    driver,
                    Some(name),
                    CapabilityReason::FieldNameTooLong,
                    "nome oltre il limite del formato",
                ));
            }
            if caps.field_names.encoding == TextEncoding::Ascii && !name.is_ascii() {
                return Err(violation(
                    driver,
                    Some(name),
                    CapabilityReason::FieldNameEncoding,
                    "il formato richiede nomi ASCII",
                ));
            }
            if caps
                .field_names
                .reserved_names
                .iter()
                .any(|reserved| reserved.eq_ignore_ascii_case(name))
            {
                return Err(violation(
                    driver,
                    Some(name),
                    CapabilityReason::FieldNameCollision,
                    "nome riservato dal formato",
                ));
            }
            let normalized = if caps.field_names.case_sensitive {
                name.clone()
            } else {
                name.to_lowercase()
            };
            if !normalized_names.insert(normalized) {
                return Err(violation(
                    driver,
                    Some(name),
                    CapabilityReason::FieldNameCollision,
                    "collisione dopo la normalizzazione del formato",
                ));
            }

            if geometry_name == Some(name.as_str()) {
                continue;
            }
            match caps.attributes {
                AttributeWriteSupport::None => {
                    return Err(violation(
                        driver,
                        Some(name),
                        CapabilityReason::TypeNotRepresentable,
                        "il formato non rappresenta attributi",
                    ))
                }
                AttributeWriteSupport::NamedSubset(names)
                    if !names.iter().any(|allowed| *allowed == name) =>
                {
                    return Err(violation(
                        driver,
                        Some(name),
                        CapabilityReason::TypeNotRepresentable,
                        "attributo fuori dal sottoinsieme rappresentabile",
                    ))
                }
                AttributeWriteSupport::All
                | AttributeWriteSupport::NamedSubset(_)
                | AttributeWriteSupport::LossReported => {}
            }

            let class = arrow_type_class(field.data_type());
            if !caps.allowed_types.contains(&class)
                && caps.type_coercion == TypeCoercionPolicy::Reject
            {
                return Err(violation(
                    driver,
                    Some(name),
                    CapabilityReason::TypeNotRepresentable,
                    format!("tipo Arrow {class:?} non rappresentabile"),
                ));
            }
            if caps.nullability == NullabilitySupport::NoNulls && field.is_nullable() {
                return Err(violation(
                    driver,
                    Some(name),
                    CapabilityReason::Nullability,
                    "campo nullable non supportato",
                ));
            }
        }

        if let Some(geometry) = &layer.contract.geometry {
            if !caps.geometry.supported {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::GeometryNotSupported,
                    "geometria non supportata",
                ));
            }
            if !caps.geometry.encodings.contains(&geometry.encoding) {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::GeometryEncoding,
                    format!("encoding {:?} non supportato", geometry.encoding),
                ));
            }
            if !caps.geometry.dimensions.contains(&geometry.dimensions) {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::CoordinateDimensions,
                    format!("dimensioni {:?} non supportate", geometry.dimensions),
                ));
            }
            if !caps
                .geometry
                .spatial_semantics
                .contains(&geometry.spatial_semantics)
            {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::SpatialSemantics,
                    format!("semantica {:?} non supportata", geometry.spatial_semantics),
                ));
            }
            let declared_mixed =
                geometry.types_declaration == plenora_io_model::contract::TypesDeclaration::Mixed;
            if !caps.geometry.mixed_types && (declared_mixed || geometry.geometry_types.len() > 1) {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::MixedGeometry,
                    "il formato richiede un solo tipo geometrico",
                ));
            }
            let unique_geometry_types = geometry
                .geometry_types
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            if unique_geometry_types.len() != geometry.geometry_types.len() {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::MixedGeometry,
                    "il contratto geometrico contiene tipi duplicati",
                ));
            }
            let restricts_geometry_types = caps.geometry.geometry_types.len()
                != ALL_GEOMETRY_TYPES.len()
                || !ALL_GEOMETRY_TYPES
                    .iter()
                    .all(|geometry_type| caps.geometry.geometry_types.contains(geometry_type));
            if restricts_geometry_types
                && matches!(
                    geometry.types_declaration,
                    plenora_io_model::contract::TypesDeclaration::Unresolved
                        | plenora_io_model::contract::TypesDeclaration::LegacyUndeclared
                )
            {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::GeometryNotSupported,
                    "il formato richiede una dichiarazione preventiva dei tipi geometrici",
                ));
            }
            if let Some(unsupported) = geometry
                .geometry_types
                .iter()
                .find(|geometry_type| !caps.geometry.geometry_types.contains(geometry_type))
            {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::GeometryNotSupported,
                    format!("tipo geometrico {unsupported:?} non supportato"),
                ));
            }
            match caps.crs {
                CrsWriteSupport::Embedded if geometry.resolved_crs().is_none() => {
                    return Err(violation(
                        driver,
                        Some(&geometry.name),
                        CapabilityReason::CrsUnresolved,
                        "il formato richiede un CRS risolto",
                    ))
                }
                CrsWriteSupport::Fixed(expected) if geometry.crs.id() != Some(expected) => {
                    return Err(violation(
                        driver,
                        Some(&geometry.name),
                        CapabilityReason::ReprojectionRequired,
                        format!("richiesto {expected}, ricevuto {:?}", geometry.crs.id()),
                    ))
                }
                CrsWriteSupport::Embedded
                | CrsWriteSupport::EmbeddedOptional
                | CrsWriteSupport::Fixed(_)
                | CrsWriteSupport::None => {}
            }
            let (comparable_id, comparable_srid, comparable_definition) =
                comparable_crs_representations(geometry);
            let representations_are_preserved = comparable_id.is_none()
                || caps.crs_representations.crs_id == CrsRepresentationState::Preserved;
            let representations_are_preserved = representations_are_preserved
                && (comparable_srid.is_none()
                    || caps.crs_representations.srid == CrsRepresentationState::Preserved);
            let representations_are_preserved = representations_are_preserved
                && (comparable_definition.is_none()
                    || caps.crs_representations.crs_definition
                        == CrsRepresentationState::Preserved);
            if crs_representations_are_inconsistent(geometry) && !representations_are_preserved {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::CrsRepresentationsInconsistent,
                    format!(
                        "rappresentazioni CRS discordanti non preservabili indipendentemente: \
                         crs_id={:?}, srid={:?}, crs_definition={:?}",
                        caps.crs_representations.crs_id,
                        caps.crs_representations.srid,
                        caps.crs_representations.crs_definition
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, Schema};
    use plenora_io_model::contract::{DataContract, FieldId, GeometryColumnContract, GeometryType};
    use plenora_io_model::crs::{CrsKind, ResolvedCrs};

    use super::*;
    use crate::descriptor::{
        CrsRepresentationCapabilities, Direction, Fidelity, FormatWriteCapabilities, ReadMode,
        ReaderConcurrency, Runtime, WriteMode, DBF_FIELD_NAMES, SCALAR_TYPES, WKB_XY_GEOMETRY,
    };
    use crate::request::WriteLayer;

    fn descriptor(crs: CrsWriteSupport) -> FormatDescriptor {
        FormatDescriptor {
            id: "test",
            direction: Direction::Bidirectional,
            read_mode: ReadMode::StreamingSequential,
            read_determinism: crate::descriptor::DeterminismLevel::Semantic,
            write_mode: Some(WriteMode::Streaming),
            write_determinism: Some(crate::descriptor::DeterminismLevel::Semantic),
            multi_layer: false,
            multi_file: false,
            reader_concurrency: ReaderConcurrency::MultipleIndependentReaders,
            projection_support: crate::descriptor::ProjectionSupport::None,
            predicate_pruning_support: crate::descriptor::PredicatePruningSupport::None,
            spatial_pruning_support: crate::descriptor::SpatialPruningSupport::None,
            crs_handling: crate::descriptor::CrsHandling::Embedded,
            fidelity_class: Fidelity::Conditional,
            runtime: Runtime::PureRust,
            write_capabilities: Some(FormatWriteCapabilities {
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
            }),
            semantic_version: 1,
            driver_version: 1,
            descriptor_version: 1,
        }
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
        let error =
            validate_write(&descriptor(CrsWriteSupport::None), &p, &Limits::default()).unwrap_err();
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
            &Limits::default(),
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
        let limits = Limits {
            max_columns: 1,
            ..Limits::default()
        };
        assert!(matches!(
            validate_write(&descriptor(CrsWriteSupport::None), &p, &limits),
            Err(error) if error.code == plenora_io_model::IoErrorCode::LimitExceeded
        ));
    }

    #[test]
    fn attribute_none_rejects_every_non_geometry_field() {
        let mut descriptor = descriptor(CrsWriteSupport::None);
        let mut capabilities = descriptor.write_capabilities.unwrap();
        capabilities.attributes = AttributeWriteSupport::None;
        descriptor.write_capabilities = Some(capabilities);
        let p = plan(vec![Field::new("attribute", DataType::Utf8, false)], None);

        assert!(matches!(
            validate_write(&descriptor, &p, &Limits::default()),
            Err(error)
                if error.capability_reason == Some(CapabilityReason::TypeNotRepresentable)
        ));
    }

    #[test]
    fn named_attribute_subset_accepts_only_the_declared_names() {
        static ALLOWED: &[&str] = &["name"];
        let mut descriptor = descriptor(CrsWriteSupport::None);
        let mut capabilities = descriptor.write_capabilities.unwrap();
        capabilities.attributes = AttributeWriteSupport::NamedSubset(ALLOWED);
        descriptor.write_capabilities = Some(capabilities);

        let accepted = plan(vec![Field::new("name", DataType::Utf8, false)], None);
        assert!(validate_write(&descriptor, &accepted, &Limits::default()).is_ok());

        let rejected = plan(vec![Field::new("secret", DataType::Utf8, false)], None);
        assert!(matches!(
            validate_write(&descriptor, &rejected, &Limits::default()),
            Err(error)
                if error.capability_reason == Some(CapabilityReason::TypeNotRepresentable)
        ));
    }

    #[test]
    fn no_nulls_rejects_nullable_contract_fields() {
        let mut descriptor = descriptor(CrsWriteSupport::None);
        let mut capabilities = descriptor.write_capabilities.unwrap();
        capabilities.nullability = NullabilitySupport::NoNulls;
        descriptor.write_capabilities = Some(capabilities);
        let p = plan(vec![Field::new("required", DataType::Utf8, true)], None);

        assert!(matches!(
            validate_write(&descriptor, &p, &Limits::default()),
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
                &Limits::default()
            ),
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
            &Limits::default()
        )
        .is_ok());

        let mut selecting = descriptor(CrsWriteSupport::Embedded);
        let mut capabilities = selecting.write_capabilities.unwrap();
        capabilities.crs_representations.srid = CrsRepresentationState::Derived;
        selecting.write_capabilities = Some(capabilities);
        let error = validate_write(&selecting, &p, &Limits::default()).unwrap_err();
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
        assert!(validate_write(&preserving, &p, &Limits::default()).is_ok());

        let mut selecting = descriptor(CrsWriteSupport::Embedded);
        let mut capabilities = selecting.write_capabilities.unwrap();
        capabilities.crs_representations.crs_definition = CrsRepresentationState::Derived;
        selecting.write_capabilities = Some(capabilities);
        let error = validate_write(&selecting, &p, &Limits::default()).unwrap_err();
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
}

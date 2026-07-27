//! Validazione comune del `WritePlan` contro capability machine-readable
//! (ADR-IO 3). I driver possono aggiungere vincoli specifici, ma non saltare
//! questi controlli di base.

use std::collections::BTreeSet;

use arrow_schema::DataType;
use plenora_core::limits::Limits;
use plenora_core::{CapabilityReason, PlenoraError, Result};

use crate::descriptor::{
    ArrowTypeClass, AttributeWriteSupport, CrsWriteSupport, FormatDescriptor, NullabilitySupport,
    TextEncoding, TypeCoercionPolicy,
};
use crate::request::WritePlan;

fn violation(
    driver: &'static str,
    field: Option<&str>,
    reason: CapabilityReason,
    detail: impl Into<String>,
) -> PlenoraError {
    PlenoraError::Capability {
        driver,
        field: field.map(str::to_owned),
        reason,
        detail: detail.into(),
    }
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
            return Err(PlenoraError::LimitExceeded(format!(
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
            if !caps.geometry.mixed_types && geometry.geometry_types.len() > 1 {
                return Err(violation(
                    driver,
                    Some(&geometry.name),
                    CapabilityReason::MixedGeometry,
                    "il formato richiede un solo tipo geometrico",
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
                CrsWriteSupport::Embedded | CrsWriteSupport::Fixed(_) | CrsWriteSupport::None => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_schema::{DataType, Field, Schema};
    use plenora_core::contract::{DataContract, FieldId, GeometryColumnContract};
    use plenora_core::crs::{CrsKind, ResolvedCrs};

    use super::*;
    use crate::descriptor::{
        Direction, Fidelity, FormatWriteCapabilities, ReadMode, ReaderConcurrency, Runtime,
        WriteMode, DBF_FIELD_NAMES, SCALAR_TYPES, WKB_XY_GEOMETRY,
    };
    use crate::request::WriteLayer;

    fn descriptor(crs: CrsWriteSupport) -> FormatDescriptor {
        FormatDescriptor {
            id: "test",
            direction: Direction::Bidirectional,
            read_mode: ReadMode::StreamingSequential,
            write_mode: Some(WriteMode::Streaming),
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
        assert!(matches!(
            error,
            PlenoraError::Capability {
                reason: CapabilityReason::FieldNameTooLong,
                ..
            }
        ));
    }

    #[test]
    fn fixed_crs_requires_explicit_reprojection() {
        let geometry = GeometryColumnContract::wkb_xy(
            FieldId(0),
            "geom",
            ResolvedCrs {
                id: Some("EPSG:3857".to_owned()),
                kind: CrsKind::Projected,
                definition: None,
            },
            true,
        );
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
        assert!(matches!(
            error,
            PlenoraError::Capability {
                reason: CapabilityReason::ReprojectionRequired,
                ..
            }
        ));
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
            Err(PlenoraError::LimitExceeded(_))
        ));
    }
}

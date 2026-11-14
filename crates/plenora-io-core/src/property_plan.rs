//! Inventario dichiarativo: ogni chiave dell'ingresso ha una regola esplicita.
use arrow_schema::{Field, Schema};
use plenora_io_model::geometry::{PLENORA_ENCODING_KEY, PLENORA_SRID_KEY};
use serde::{Deserialize, Serialize};

/// Le proprietà richieste dalla qualifica, con la chiave del vocabolario.
pub const PROPERTIES: &[(&str, &str)] = &[
    ("contract_version", "plenora.contract.version"),
    ("field_id", "plenora.field_id"),
    ("types_declaration", "plenora.geometry.types_declaration"),
    ("geometry_types", "plenora.geometry.types"),
    ("dimensions", "plenora.geometry.dimensions"),
    ("encoding", "plenora.geometry.encoding"),
    (
        "geometry_or_geography",
        "plenora.geometry.spatial_semantics",
    ),
    ("precision", "plenora.geometry.precision"),
    ("srid", "plenora.geometry.srid"),
    ("crs_id", "plenora.geometry.crs_id"),
    ("crs_resolution", "plenora.geometry.crs_resolution"),
    ("crs_definition", "plenora.geometry.crs_definition"),
    (
        "crs_definition_format",
        "plenora.geometry.crs_definition_format",
    ),
    ("axis_order", "plenora.geometry.axis_order"),
    ("native_metadata", "plenora.geometry.native."),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Preserved,
    Unassessed,
    Reconstructed,
    Removed,
    EwkbToWkb,
    SridRemoved,
}

/// Localizzazione strutturale completa, indipendente dagli esempi bounded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyEffect {
    pub field_index: Option<usize>,
    pub field_id: Option<u32>,
    /// Indice nella sequenza lessicografica delle chiavi originali, mai il payload.
    pub key_index: usize,
    /// Solo chiavi del vocabolario chiuso; le chiavi opache restano strutturali.
    pub property: Option<String>,
    pub rule: Rule,
}

#[must_use]
pub fn metadata_rule(driver: &str, key: &str, value: &str, convert_ewkb: bool) -> Rule {
    if convert_ewkb && key == PLENORA_ENCODING_KEY && value == "ewkb" {
        return Rule::EwkbToWkb;
    }
    if convert_ewkb && key == PLENORA_SRID_KEY {
        return Rule::SridRemoved;
    }
    if driver != "gpkg" {
        return if driver == "ipc" {
            Rule::Preserved
        } else {
            Rule::Unassessed
        };
    }
    if matches!(
        key,
        "plenora.field_id" | "gpkg.declared_type" | "gpkg.primary_key"
    ) {
        return Rule::Preserved;
    }
    if PROPERTIES.iter().any(|(_, candidate)| *candidate == key)
        || matches!(key, "ARROW:extension:name" | "plenora.primary_geometry")
    {
        Rule::Reconstructed
    } else {
        Rule::Removed
    }
}

#[must_use]
pub fn plan_metadata(
    schema: &Schema,
    driver: &str,
    converted_field: Option<usize>,
) -> Vec<PropertyEffect> {
    let mut effects = Vec::new();
    for (field_index, metadata) in std::iter::once((None, schema.metadata())).chain(
        schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, f)| (Some(i), f.metadata())),
    ) {
        let mut keys: Vec<_> = metadata.keys().collect();
        keys.sort_unstable();
        for (key_index, key) in keys.into_iter().enumerate() {
            effects.push(PropertyEffect {
                field_index,
                field_id: field_index.and_then(|i| {
                    schema
                        .field(i)
                        .metadata()
                        .get("plenora.field_id")
                        .and_then(|id| id.parse().ok())
                }),
                key_index,
                property: PROPERTIES
                    .iter()
                    .find(|(_, candidate)| {
                        *candidate == key
                            || (candidate.ends_with('.') && key.starts_with(candidate))
                    })
                    .map(|(name, _)| (*name).to_owned()),
                rule: metadata_rule(
                    driver,
                    key,
                    &metadata[key],
                    converted_field.is_some() && converted_field == field_index,
                ),
            });
        }
    }
    effects
}

/// La stessa regola governa la modifica dello schema e l'inventario del piano.
#[must_use]
pub fn field_in_wkb(field: &Field) -> Field {
    let metadata: std::collections::HashMap<String, String> = field
        .metadata()
        .iter()
        .filter_map(|(key, value)| match metadata_rule("", key, value, true) {
            Rule::SridRemoved => None,
            Rule::EwkbToWkb => Some((key.clone(), "wkb".to_owned())),
            _ => Some((key.clone(), value.clone())),
        })
        .collect();
    field.clone().with_metadata(metadata)
}

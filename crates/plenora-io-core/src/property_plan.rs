//! Inventario dichiarativo: ogni chiave dell'ingresso ha una regola esplicita.
use arrow_schema::{Field, Schema};
use plenora_io_model::geometry::{PLENORA_ENCODING_KEY, PLENORA_SRID_KEY};
use serde::{Deserialize, Serialize};

/// Qualification properties, with GPKG write/read rules and opaque fallback.
///
/// Reconstructed declares catalog/native reconstruction, not value inequality.
/// Only logical identity and contract version promise metadata preservation.
pub const PROPERTIES: &[(&str, &str, Rule, Rule)] = &[
    (
        "contract_version",
        "plenora.contract.version",
        Rule::Preserved,
        Rule::Preserved,
    ),
    (
        "field_id",
        "plenora.field_id",
        Rule::Preserved,
        Rule::Preserved,
    ),
    (
        "types_declaration",
        "plenora.geometry.types_declaration",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "geometry_types",
        "plenora.geometry.types",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "dimensions",
        "plenora.geometry.dimensions",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "encoding",
        "plenora.geometry.encoding",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "geometry_or_geography",
        "plenora.geometry.spatial_semantics",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "precision",
        "plenora.geometry.precision",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "srid",
        "plenora.geometry.srid",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "crs_id",
        "plenora.geometry.crs_id",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "crs_resolution",
        "plenora.geometry.crs_resolution",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "crs_definition",
        "plenora.geometry.crs_definition",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "crs_definition_format",
        "plenora.geometry.crs_definition_format",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "axis_order",
        "plenora.geometry.axis_order",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "native_metadata",
        "plenora.geometry.native.",
        Rule::Removed,
        Rule::Preserved,
    ),
    ("", "gpkg.declared_type", Rule::Preserved, Rule::Preserved),
    ("", "gpkg.primary_key", Rule::Preserved, Rule::Preserved),
    (
        "",
        "ARROW:extension:name",
        Rule::Reconstructed,
        Rule::Preserved,
    ),
    (
        "",
        "ARROW:extension:metadata",
        Rule::Removed,
        Rule::Preserved,
    ),
    (
        "",
        "plenora.primary_geometry",
        Rule::Removed,
        Rule::Preserved,
    ),
    ("", "gpkg.schema_normalized", Rule::Removed, Rule::Preserved),
    ("", "crs", Rule::Removed, Rule::Preserved),
    ("", "geo", Rule::Removed, Rule::Preserved),
    ("", "", Rule::Removed, Rule::Rejected),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Preserved,
    Rejected,
    Reconstructed,
    Removed,
    EwkbToWkb,
    SridRemoved,
    SridToCatalog,
}

const METADATA_REMOVED: &str = "gpkg_metadata_not_preserved";
const GEOMETRY_NORMALIZED: &str = "geometry_representation_normalized";

impl Rule {
    const fn loss_category(self) -> Option<&'static str> {
        match self {
            Self::Removed => Some(METADATA_REMOVED),
            Self::EwkbToWkb | Self::SridRemoved | Self::SridToCatalog => Some(GEOMETRY_NORMALIZED),
            _ => None,
        }
    }
}

/// Localizzazione strutturale completa, indipendente dagli esempi bounded.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
        return if driver == "gpkg" {
            Rule::SridToCatalog
        } else {
            Rule::SridRemoved
        };
    }
    // Outside GPKG this plan governs the representation adapter only;
    // it does not claim that a different format preserves Arrow metadata.
    if driver != "gpkg" {
        return Rule::Preserved;
    }
    PROPERTIES
        .iter()
        .find(|(_, pattern, _, _)| matches_key(pattern, key))
        .map_or(Rule::Rejected, |entry| entry.2)
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
                    .find(|(_, pattern, _, _)| matches_key(pattern, key))
                    .filter(|(name, _, _, _)| !name.is_empty())
                    .map(|(name, _, _, _)| (*name).to_owned()),
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
            Rule::SridRemoved | Rule::SridToCatalog => None,
            Rule::EwkbToWkb => Some((key.clone(), "wkb".to_owned())),
            _ => Some((key.clone(), value.clone())),
        })
        .collect();
    field.clone().with_metadata(metadata)
}

fn matches_key(pattern: &str, key: &str) -> bool {
    pattern.is_empty() || pattern == key || (pattern.ends_with('.') && key.starts_with(pattern))
}

/// Complete structural plan. No input name, metadata value or digest is carried.
/// For GPKG this covers format metadata; for other sinks it covers the EWKB adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyPlan {
    driver: String,
    effects: Vec<PropertyEffect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Write,
    SourceWrite,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyChange {
    pub layer_index: u64,
    pub phase: Phase,
    pub effect: PropertyEffect,
}

/// Reserved within the existing 12 KiB loss section. Never truncated.
pub const MAX_PROPERTY_REPORT_BYTES: usize = 8192;

impl PropertyPlan {
    #[must_use]
    pub fn new(schema: &Schema, driver: &str, converted_field: Option<usize>) -> Self {
        Self {
            driver: driver.to_owned(),
            effects: plan_metadata(schema, driver, converted_field),
        }
    }

    #[must_use]
    pub fn effects(&self) -> &[PropertyEffect] {
        &self.effects
    }

    /// Validate persisted plans before exposing any diagnostic text.
    /// # Errors
    /// Rejects a driver mismatch, invalid locations or non-vocabulary properties.
    pub fn validate(&self, driver: &str, fields: usize) -> plenora_io_model::Result<()> {
        let mut next = std::collections::BTreeMap::new();
        let mut identities = std::collections::BTreeMap::new();
        if self.driver != driver
            || self.effects.iter().any(|effect| {
                effect.field_index.is_some_and(|i| i >= fields)
                    || effect
                        .property
                        .as_ref()
                        .is_some_and(|p| !PROPERTIES.iter().any(|(name, _, _, _)| name == p))
                    || {
                        let index = next.entry(effect.field_index).or_insert(0);
                        let invalid = effect.key_index != *index;
                        *index += 1;
                        invalid
                    }
                    || (effect.field_index.is_none() && effect.field_id.is_some())
                    || identities
                        .insert(effect.field_index, effect.field_id)
                        .is_some_and(|id| id != effect.field_id)
                    || (effect.field_index.is_some()
                        && effect.property.as_deref() == Some("field_id")
                        && effect.field_id.is_none())
                    || !valid_rule(driver, effect)
                    || effect.rule == Rule::Rejected
                    || (driver == "gpkg" && effect.rule == Rule::SridRemoved)
            })
        {
            return Err(plenora_io_model::PlenoraIoError::contratto_redatto(
                &plenora_io_model::PublicMessage::Curated("piano delle proprieta non valido"),
            ));
        }
        Ok(())
    }

    /// Bind a supplied plan to the actual schema before any writer effect.
    /// Only the known EWKB adapter may have changed encoding and removed SRID.
    /// # Errors
    /// Rejects missing, invented or mislocalized effects.
    pub fn validate_schema(&self, driver: &str, schema: &Schema) -> plenora_io_model::Result<()> {
        self.validate(driver, schema.fields().len())?;
        let converted: Vec<_> = self
            .effects
            .iter()
            .filter(|e| e.rule == Rule::EwkbToWkb)
            .collect();
        let converted_field = converted.first().and_then(|e| e.field_index);
        let mut fields: Vec<_> = schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        if let Some(index) = converted_field {
            let mut metadata = fields[index].metadata().clone();
            if converted.len() != 1
                || metadata.get(PLENORA_ENCODING_KEY).map(String::as_str) != Some("wkb")
            {
                return Err(invalid_plan());
            }
            metadata.insert(PLENORA_ENCODING_KEY.to_owned(), "ewkb".to_owned());
            if self.effects.iter().any(|e| {
                e.field_index == Some(index)
                    && matches!(e.rule, Rule::SridRemoved | Rule::SridToCatalog)
            }) {
                metadata.insert(PLENORA_SRID_KEY.to_owned(), "0".to_owned());
            }
            fields[index] = fields[index].clone().with_metadata(metadata);
        }
        let original = Schema::new_with_metadata(fields, schema.metadata().clone());
        if self.effects != plan_metadata(&original, driver, converted_field) {
            return Err(invalid_plan());
        }
        Ok(())
    }

    /// A complete report, independent of the bounded illustrative examples.
    /// # Errors
    /// Rejects reports exceeding the complete-inventory byte budget.
    pub fn report(
        &self,
        layer_index: u64,
        phase: Phase,
    ) -> plenora_io_model::Result<crate::LossReport> {
        let mut report = crate::LossReport::default();
        for effect in &self.effects {
            if effect.rule == Rule::Preserved {
                continue;
            }
            report.property_changes.push(PropertyChange {
                layer_index,
                phase,
                effect: effect.clone(),
            });
            let rule = effect.rule;
            let Some(category) = rule.loss_category() else {
                continue;
            };
            // Persisted write provenance is visible on read, but is not counted
            // a second time as a new loss incurred by that read.
            if phase == Phase::Write {
                report.record(category, 1);
                report.add_example(crate::loss::LossExample {
                    category: category.to_owned(),
                    posizione: crate::loss::Posizione {
                        layer_index: Some(layer_index),
                        field_index: effect.field_index.map(|i| i as u64),
                        type_class: None,
                    },
                    context: format!(
                        "scope={} key_index={} field_id={} {} phase=write",
                        if effect.field_index.is_some() {
                            "field"
                        } else {
                            "schema"
                        },
                        effect.key_index,
                        effect.field_id.map_or_else(|| "absent".to_owned(), |id| id.to_string()),
                        match effect.rule {
                            Rule::EwkbToWkb => "encoding=ewkb_to_iso_wkb byte_order=little_endian coordinates=bitwise",
                            Rule::SridToCatalog => "srid=gpkg_catalog payload_srid=removed",
                            Rule::SridRemoved => "srid=removed crs=validated",
                            _ => "rule=removed",
                        }
                    ),
                });
            }
        }
        report.validate_property_budget()?;
        Ok(report)
    }
}

fn invalid_plan() -> plenora_io_model::PlenoraIoError {
    plenora_io_model::PlenoraIoError::contratto_redatto(&plenora_io_model::PublicMessage::Curated(
        "piano delle proprieta non valido",
    ))
}

fn valid_rule(driver: &str, effect: &PropertyEffect) -> bool {
    match effect.rule {
        Rule::EwkbToWkb => {
            effect.field_index.is_some() && effect.property.as_deref() == Some("encoding")
        }
        Rule::SridRemoved | Rule::SridToCatalog => {
            effect.field_index.is_some()
                && effect.property.as_deref() == Some("srid")
                && (driver == "gpkg") == (effect.rule == Rule::SridToCatalog)
        }
        _ if driver != "gpkg" => effect.rule == Rule::Preserved,
        _ => PROPERTIES.iter().any(|(name, _, rule, _)| {
            effect.property.as_deref() == (!name.is_empty()).then_some(*name)
                && effect.rule == *rule
        }),
    }
}

/// The writer takes SQL/contract metadata from this projection of the same rules
/// that produced the original-input report. Opaque keys never reach the codec.
#[must_use]
pub fn gpkg_write_schema(schema: &Schema) -> Schema {
    let filter = |metadata: &arrow_schema::Metadata| -> arrow_schema::Metadata {
        metadata
            .iter()
            .filter(|(key, value)| metadata_rule("gpkg", key, value, false) != Rule::Removed)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    };
    Schema::new_with_metadata(
        schema
            .fields()
            .iter()
            .map(|f| f.as_ref().clone().with_metadata(filter(f.metadata())))
            .collect::<Vec<_>>(),
        filter(schema.metadata()),
    )
}

/// New native metadata emitted by the reader must be classified in the table.
/// External files do not manufacture a history of missing Arrow source metadata.
/// # Errors
/// Rejects metadata emitted outside the declared reader vocabulary.
pub fn validate_gpkg_read_schema(schema: &Schema) -> plenora_io_model::Result<()> {
    for metadata in
        std::iter::once(schema.metadata()).chain(schema.fields().iter().map(|f| f.metadata()))
    {
        for key in metadata.keys() {
            let rule = PROPERTIES
                .iter()
                .find(|(_, pattern, _, _)| matches_key(pattern, key))
                .map_or(Rule::Rejected, |entry| entry.3);
            if rule == Rule::Rejected {
                return Err(plenora_io_model::PlenoraIoError::contratto_redatto(
                    &plenora_io_model::PublicMessage::Curated(
                        "metadato di lettura non censito nel piano GeoPackage",
                    ),
                ));
            }
        }
    }
    Ok(())
}

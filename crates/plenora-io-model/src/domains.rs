//! Typed attribute domains and subtype metadata. JSON is validated at both borders.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{PlenoraIoError, PublicMessage, Result};

pub const ALIAS_KEY: &str = "plenora.field.alias";
pub const DOMAIN_KEY: &str = "plenora.field.domain";
pub const DOMAINS_KEY: &str = "plenora.dataset.domains";
pub const SUBTYPES_KEY: &str = "plenora.layer.subtypes";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeType {
    Int32,
    Int64,
    Float64,
    Utf8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Code {
    Int32(i32),
    Int64(i64),
    Float64(f64),
    Utf8(String),
}

impl Code {
    #[must_use]
    pub const fn code_type(&self) -> CodeType {
        match self {
            Self::Int32(_) => CodeType::Int32,
            Self::Int64(_) => CodeType::Int64,
            Self::Float64(_) => CodeType::Float64,
            Self::Utf8(_) => CodeType::Utf8,
        }
    }
    fn valid(&self) -> bool {
        match self {
            Self::Float64(value) => value.is_finite(),
            Self::Utf8(value) => !value.contains('\0'),
            _ => true,
        }
    }

    fn same_type(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodedValue {
    pub code: Code,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Domain {
    Coded {
        code_type: CodeType,
        values: Vec<CodedValue>,
        #[serde(default, skip_serializing_if = "DomainAttributes::is_default")]
        attributes: DomainAttributes,
    },
    Range {
        min: Code,
        max: Code,
        min_inclusive: bool,
        max_inclusive: bool,
        #[serde(default, skip_serializing_if = "DomainAttributes::is_default")]
        attributes: DomainAttributes,
    },
}

impl Domain {
    #[must_use]
    pub const fn attributes(&self) -> &DomainAttributes {
        match self {
            Self::Coded { attributes, .. } | Self::Range { attributes, .. } => attributes,
        }
    }
}

/// What a split of the feature does to the field value (GDAL `OGRFieldDomainSplitPolicy`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitPolicy {
    #[default]
    DefaultValue,
    Duplicate,
    GeometryRatio,
}

/// What a merge of features does to the field value (GDAL `OGRFieldDomainMergePolicy`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergePolicy {
    #[default]
    DefaultValue,
    Sum,
    GeometryWeighted,
}

/// The narrower native field type of the domain (GDAL `OGRFieldSubType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldSubtype {
    Boolean,
    Int16,
    Float32,
    Json,
    Uuid,
}

/// Native attributes shared by every domain kind; the defaults are not serialized.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainAttributes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_subtype: Option<FieldSubtype>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub split_policy: SplitPolicy,
    #[serde(default, skip_serializing_if = "is_default")]
    pub merge_policy: MergePolicy,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

impl DomainAttributes {
    #[must_use]
    pub fn is_default(&self) -> bool {
        is_default(self)
    }
}

pub type Domains = BTreeMap<String, Domain>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subtype {
    pub code: i32,
    pub name: String,
    pub domains: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subtypes {
    pub field: String,
    pub default: Option<i32>,
    pub values: Vec<Subtype>,
}

fn invalid() -> PlenoraIoError {
    PlenoraIoError::schema_redatto(&PublicMessage::Curated(
        "metadati di dominio o sottotipo non validi",
    ))
}

fn name(value: &str) -> bool {
    !value.is_empty() && !value.contains('\0')
}

/// # Errors
/// Invalid, inconsistent or non-finite domain values, without input in diagnostics.
pub fn validate_domains(domains: &Domains) -> Result<()> {
    for (key, domain) in domains {
        if !name(key) {
            return Err(invalid());
        }
        if domain
            .attributes()
            .description
            .as_ref()
            .is_some_and(|value| !name(value))
        {
            return Err(invalid());
        }
        match domain {
            Domain::Coded {
                code_type, values, ..
            } => {
                let mut codes = BTreeSet::new();
                for value in values {
                    if !value.code.valid()
                        || value.code.code_type() != *code_type
                        || values
                            .first()
                            .is_some_and(|first| !first.code.same_type(&value.code))
                        || !codes.insert(serde_json::to_string(&value.code).map_err(|_| invalid())?)
                        || value.description.as_ref().is_some_and(|v| v.contains('\0'))
                    {
                        return Err(invalid());
                    }
                }
            }
            Domain::Range { min, max, .. } => {
                let ordered = match (min, max) {
                    (Code::Int32(a), Code::Int32(b)) => a <= b,
                    (Code::Int64(a), Code::Int64(b)) => a <= b,
                    (Code::Float64(a), Code::Float64(b)) => a <= b,
                    _ => false,
                };
                if !min.valid() || !max.valid() || !ordered {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}

/// # Errors
/// Malformed domain metadata.
pub fn decode_domains(value: &str) -> Result<Domains> {
    let domains = crate::json::from_str(value).map_err(|_| invalid())?;
    validate_domains(&domains)?;
    Ok(domains)
}

/// # Errors
/// Invalid domain metadata.
pub fn encode_domains(domains: &Domains) -> Result<String> {
    validate_domains(domains)?;
    serde_json::to_string(domains).map_err(|_| invalid())
}

/// # Errors
/// Malformed subtype metadata, duplicate codes or an absent default code.
pub fn decode_subtypes(value: &str) -> Result<Subtypes> {
    let subtypes: Subtypes = crate::json::from_str(value).map_err(|_| invalid())?;
    let mut codes = BTreeSet::new();
    if !name(&subtypes.field)
        || subtypes.values.iter().any(|s| {
            !name(&s.name)
                || !codes.insert(s.code)
                || s.domains.iter().any(|(f, d)| !name(f) || !name(d))
        })
        || subtypes.default.is_some_and(|code| !codes.contains(&code))
    {
        return Err(invalid());
    }
    Ok(subtypes)
}

/// # Errors
/// Invalid subtype metadata.
pub fn encode_subtypes(subtypes: &Subtypes) -> Result<String> {
    let value = serde_json::to_string(subtypes).map_err(|_| invalid())?;
    decode_subtypes(&value)?;
    Ok(value)
}

/// # Errors
/// Invalid placement, encoding or field reference in registered attribute metadata.
pub fn validate_schema(schema: &arrow_schema::Schema) -> Result<()> {
    for key in [ALIAS_KEY, DOMAIN_KEY] {
        if schema.metadata().contains_key(key) {
            return Err(invalid());
        }
    }
    if schema
        .metadata()
        .get("plenora.layer.alias")
        .is_some_and(|value| !name(value))
    {
        return Err(invalid());
    }
    if let Some(value) = schema.metadata().get(DOMAINS_KEY) {
        decode_domains(value)?;
    }
    if let Some(value) = schema.metadata().get(SUBTYPES_KEY) {
        let subtypes = decode_subtypes(value)?;
        let field = schema
            .field_with_name(&subtypes.field)
            .map_err(|_| invalid())?;
        if !matches!(
            field.data_type(),
            arrow_schema::DataType::Int32 | arrow_schema::DataType::Int64
        ) {
            return Err(invalid());
        }
    }
    for field in schema.fields() {
        for key in [ALIAS_KEY, DOMAIN_KEY] {
            if field.metadata().get(key).is_some_and(|value| !name(value)) {
                return Err(invalid());
            }
        }
        if [DOMAINS_KEY, SUBTYPES_KEY]
            .iter()
            .any(|key| field.metadata().contains_key(key))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

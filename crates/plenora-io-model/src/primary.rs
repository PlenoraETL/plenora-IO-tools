//! Arrow Primary Geometry 1.0: risoluzione prima del completamento, emissione
//! canonica e conservazione per identità. Gli ID wire non sono indici.
use std::sync::Arc;

use arrow_schema::{Schema, SchemaRef};

use crate::{PlenoraIoError, PublicMessage, Result};

pub const KEY: &str = "plenora.primary_geometry";
const ID: &str = "plenora.field_id";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrimaryState {
    Absent,
    None,
    Id(String),
}

fn invalid() -> PlenoraIoError {
    PlenoraIoError::schema_redatto(&PublicMessage::Curated(
        "plenora.primary_geometry non conforme allo schema ricevuto",
    ))
}

fn geometry(field: &arrow_schema::Field) -> bool {
    field
        .metadata()
        .get("ARROW:extension:name")
        .map(String::as_str)
        == Some("geoarrow.wkb")
}

impl PrimaryState {
    /// Legge la dichiarazione senza completare gli ID.
    /// # Errors
    /// Chiave di campo, grammatica o riferimento non conforme (PRIM-003).
    pub fn read(schema: &Schema) -> Result<Self> {
        if schema
            .fields()
            .iter()
            .any(|f| f.metadata().contains_key(KEY))
        {
            return Err(invalid());
        }
        let Some(value) = schema.metadata().get(KEY) else {
            return Ok(Self::Absent);
        };
        if !schema.fields().iter().any(|f| geometry(f)) {
            return Err(invalid());
        }
        if value == "none" {
            return Ok(Self::None);
        }
        if value.is_empty()
            || !value.bytes().all(|b| b.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
            || !schema
                .fields()
                .iter()
                .any(|f| geometry(f) && f.metadata().get(ID) == Some(value))
        {
            return Err(invalid());
        }
        Ok(Self::Id(value.clone()))
    }

    #[must_use]
    pub fn position(&self, schema: &Schema) -> Option<usize> {
        match self {
            Self::None => None,
            Self::Id(id) => schema
                .fields()
                .iter()
                .position(|f| geometry(f) && f.metadata().get(ID) == Some(id)),
            Self::Absent => {
                let mut positions = schema
                    .fields()
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| geometry(f));
                let (position, _) = positions.next()?;
                positions.next().is_none().then_some(position)
            }
        }
    }
}

/// Emette lo stato semantico già risolto; `None` non promuove l'unico campo.
/// # Errors
/// Posizione primaria inesistente, non geometrica o priva di ID in multi.
pub fn emit(schema: &Schema, primary: Option<usize>) -> Result<SchemaRef> {
    let count = schema.fields().iter().filter(|f| geometry(f)).count();
    let mut metadata = schema.metadata().clone();
    metadata.remove(KEY);
    if let Some(position) = primary {
        let field = schema
            .fields()
            .get(position)
            .filter(|f| geometry(f))
            .ok_or_else(invalid)?;
        if count > 1 {
            metadata.insert(
                KEY.to_owned(),
                field.metadata().get(ID).ok_or_else(invalid)?.clone(),
            );
        }
    } else if count == 1 {
        metadata.insert(KEY.to_owned(), "none".to_owned());
    }
    Ok(Arc::new(Schema::new_with_metadata(
        schema.fields().clone(),
        metadata,
    )))
}

/// Verifica la coerenza del piano Rust, prima di effetti o normalizzazione.
/// # Errors
/// Dichiarazione wire invalida o primaria interna fuori schema/discorde.
pub fn validate_contract(contract: &crate::contract::DataContract) -> Result<()> {
    reconcile_schema_contract(contract).map(|_| ())
}

/// Unico bordo di riconciliazione: lo schema ricevuto e' l'autorita'.
/// # Errors
/// Vocabolario invalido o contratto interno discorde, prima di completamenti.
pub fn reconcile_schema_contract(
    contract: &crate::contract::DataContract,
) -> Result<Vec<crate::contract::GeometryColumnContract>> {
    use crate::contract::{FieldId, GeometryColumnContract};
    crate::vocabolario::verifica_schema(&contract.schema)?;
    let state = PrimaryState::read(&contract.schema)?;
    if state.position(&contract.schema) != contract.geometry.as_ref().map(|g| g.field_id.0 as usize)
    {
        return Err(invalid());
    }
    let mut geometries = Vec::new();
    for (index, field) in contract.schema.fields().iter().enumerate() {
        if !crate::geometry::is_geometry_field(field) {
            continue;
        }
        let index = u32::try_from(index).map_err(|_| invalid())?;
        let mut declared = GeometryColumnContract::wkb_passthrough(
            FieldId(index),
            field.name(),
            crate::crs::CrsResolution::Missing,
            field.is_nullable(),
        );
        crate::geometry::read_geometry_contract_metadata(field, &mut declared)?;
        if let Some(internal) = contract.geometry.as_ref().filter(|g| g.field_id.0 == index) {
            reconcile_geometry_properties(field, internal)?;
            crate::geometry::reconcile_crs_declarations(&internal.crs, &declared.crs)
                .map_err(|_| invalid())?;
            // Serializza separatamente su un campo pulito: include identita',
            // encoding, dimensioni, precisione, tipi, CRS/assi, SRID e nativi.
            let clean = arrow_schema::Field::new(
                field.name(),
                field.data_type().clone(),
                field.is_nullable(),
            );
            if internal.name != *field.name()
                || internal.nullable != field.is_nullable()
                || (matches!(
                    internal.types_declaration,
                    crate::contract::TypesDeclaration::Unresolved
                        | crate::contract::TypesDeclaration::LegacyUndeclared
                ) && !internal.geometry_types.is_empty())
                || crate::geometry::with_geometry_contract_metadata(&clean, internal)
                    != crate::geometry::with_geometry_contract_metadata(&clean, &declared)
            {
                return Err(invalid());
            }
            declared.scansione_completa = internal.scansione_completa;
        }
        geometries.push(declared);
    }
    Ok(geometries)
}

/// Confronta le proprieta' dichiarate da entrambe le fonti prima di completarle.
/// La tabella e' condivisa con la riconciliazione dei piani Rust.
/// # Errors
/// Due valori risolti discordi producono `schema`; gli ignoti non vincolano.
pub fn reconcile_geometry_properties(
    field: &arrow_schema::Field,
    native: &crate::contract::GeometryColumnContract,
) -> Result<()> {
    use crate::geometry::{self, GEO_CRS_KEY, PLENORA_CRS_RESOLUTION_KEY, PLENORA_NATIVE_PREFIX};
    let clean =
        arrow_schema::Field::new(field.name(), field.data_type().clone(), field.is_nullable());
    let expected = geometry::with_geometry_contract_metadata(&clean, native);
    for (key, unresolved) in GEOMETRY_PROPERTIES {
        if matches!(
            *key,
            "plenora.geometry.crs_id"
                | "plenora.geometry.crs_resolution"
                | "plenora.geometry.crs_definition"
                | "plenora.geometry.crs_definition_format"
                | "plenora.geometry.axis_order"
        ) {
            // CRS declarations have a semantic comparator below (including aliases).
            continue;
        }
        if let (Some(a), Some(b)) = (field.metadata().get(key), expected.metadata().get(key)) {
            if a != b && !unresolved.contains(&a.as_str()) && !unresolved.contains(&b.as_str()) {
                return Err(invalid());
            }
        }
    }
    for (key, value) in expected.metadata() {
        if key.starts_with(PLENORA_NATIVE_PREFIX)
            && field.metadata().get(key).is_some_and(|v| v != value)
        {
            return Err(invalid());
        }
    }
    if field.metadata().contains_key(PLENORA_CRS_RESOLUTION_KEY)
        || field.metadata().contains_key(GEO_CRS_KEY)
        || field
            .metadata()
            .get("ARROW:extension:metadata")
            .is_some_and(|s| {
                serde_json::from_str::<serde_json::Value>(s).is_ok_and(|v| v.get("crs").is_some())
            })
    {
        let mut declared = crate::contract::GeometryColumnContract::wkb_passthrough(
            native.field_id,
            field.name(),
            crate::crs::CrsResolution::Missing,
            field.is_nullable(),
        );
        geometry::read_geometry_contract_metadata(field, &mut declared)?;
        reconcile_geometry_crs(&native.crs, &declared.crs)?;
    }
    Ok(())
}

/// Cross-source CRS completion; validation of declarations within one field stays strict.
/// # Errors
/// Two resolved, contradictory declarations produce `schema`.
pub fn reconcile_geometry_crs(
    first: &crate::crs::CrsResolution,
    second: &crate::crs::CrsResolution,
) -> Result<crate::crs::CrsResolution> {
    use crate::crs::CrsResolution::{Missing, Resolved};
    match (first, second) {
        (Resolved(_), Resolved(_)) => {
            crate::geometry::reconcile_crs_declarations(first, second).map_err(|_| invalid())
        }
        (Resolved(_), _) | (_, Missing) => Ok(first.clone()),
        _ => Ok(second.clone()),
    }
}

// Every absent property is unresolved. Listed tokens are the additional unresolved
// wire values. `native` precision, `mixed` types and `other` axes are declarations,
// not unknowns. CRS components are merged semantically, including unknown axes.
const GEOMETRY_PROPERTIES: &[(&str, &[&str])] = &[
    ("plenora.geometry.encoding", &[]),
    ("plenora.geometry.dimensions", &["unknown"]),
    ("plenora.geometry.spatial_semantics", &[]),
    ("plenora.geometry.srid", &[]),
    ("plenora.geometry.precision", &[]),
    ("plenora.geometry.types", &["unknown"]),
    ("plenora.geometry.types_declaration", &["unresolved"]),
    ("plenora.geometry.crs_id", &[]),
    (
        "plenora.geometry.crs_resolution",
        &["missing", "declared_unresolved"],
    ),
    ("plenora.geometry.crs_definition", &[]),
    ("plenora.geometry.crs_definition_format", &[]),
    ("plenora.geometry.axis_order", &["unknown"]),
    ("plenora.geometry.native.", &[]),
];

#[cfg(test)]
mod property_tests;

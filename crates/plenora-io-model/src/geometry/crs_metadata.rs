//! Un solo percorso per le dichiarazioni CRS canoniche, legacy e `GeoArrow`.

use super::{CrsResolution, ResolvedCrs, GEO_CRS_KEY};
use crate::crs::{AxisOrder, CrsDefinitionFormat, CrsKind, RawCrs};
use crate::{PlenoraIoError, PublicMessage, Result};
use arrow_schema::Field;

fn incoerente() -> PlenoraIoError {
    PlenoraIoError::crs_redatto(&PublicMessage::Curated(
        "dichiarazioni CRS del campo incoerenti o non interpretabili",
    ))
}

/// Confronta dichiarazioni indipendenti senza scegliere quella piu' debole.
///
/// # Errors
/// Rifiuta stati, identificatori, definizioni o assi contraddittori.
pub fn reconcile_crs_declarations(
    first: &CrsResolution,
    second: &CrsResolution,
) -> Result<CrsResolution> {
    match (first, second) {
        (CrsResolution::Missing, CrsResolution::Missing) => Ok(first.clone()),
        (CrsResolution::Resolved(a), CrsResolution::Resolved(b)) => {
            if a.id
                .as_ref()
                .zip(b.id.as_ref())
                .is_some_and(|(a, b)| a != b)
                || a.definition
                    .as_ref()
                    .zip(b.definition.as_ref())
                    .is_some_and(|(a, b)| a != b)
                || (a.axis_order != AxisOrder::Unknown
                    && b.axis_order != AxisOrder::Unknown
                    && a.axis_order != b.axis_order)
                || (a.kind != CrsKind::Unknown && b.kind != CrsKind::Unknown && a.kind != b.kind)
            {
                return Err(incoerente());
            }
            let mut merged = a.clone();
            if merged.kind == CrsKind::Unknown {
                merged.kind = b.kind;
            }
            if merged.id.is_none() {
                merged.id.clone_from(&b.id);
            }
            if merged.definition.is_none() {
                merged.definition.clone_from(&b.definition);
                merged.definition_format = b.definition_format;
            }
            if merged.axis_order == AxisOrder::Unknown {
                merged.axis_order = b.axis_order;
            }
            if let (Some(id), Some(definition), Some(format)) =
                (&merged.id, &merged.definition, merged.definition_format)
            {
                if crate::crs_definizione::contraddice(Some(id), definition, format)
                    .map_err(|_| incoerente())?
                {
                    return Err(incoerente());
                }
            }
            Ok(merged.into())
        }
        _ if first == second => Ok(first.clone()),
        _ => Err(incoerente()),
    }
}

pub(super) fn read(
    field: &Field,
    canonical: Option<&CrsResolution>,
) -> Result<Option<CrsResolution>> {
    let mut result = canonical.cloned();
    let mut add = |declaration: CrsResolution| -> Result<()> {
        result = Some(match &result {
            Some(previous) => reconcile_crs_declarations(previous, &declaration)?,
            None => declaration,
        });
        Ok(())
    };
    if let Some(id) = field.metadata().get(GEO_CRS_KEY) {
        add(authority(id)?)?;
    }
    if let Some(text) = field.metadata().get("ARROW:extension:metadata") {
        crate::crs_definizione::verifica_dimensione_definizione(text, None)?;
        let object: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
            crate::json::from_str(text).map_err(|_| incoerente())?;
        if let Some(crs) = object.get("crs") {
            let kind = object
                .get("crs_type")
                .map(|v| serde_json::from_str::<String>(v.get()).map_err(|_| incoerente()))
                .transpose()?;
            add(extension(crs.get(), kind.as_deref())?)?;
        }
    }
    Ok(result)
}

fn authority(id: &str) -> Result<CrsResolution> {
    if id.trim().is_empty() {
        return Err(incoerente());
    }
    if crate::crs_definizione::autorita_locale(id) {
        return Ok(CrsResolution::DeclaredButUnresolved(
            RawCrs::from_authority_hint(id.to_owned()),
        ));
    }
    Ok(ResolvedCrs::new(
        Some(id.to_owned()),
        crate::crs::crs_kind_for_authority_id(id),
        None,
    )
    .into())
}

fn extension(text: &str, kind: Option<&str>) -> Result<CrsResolution> {
    if text == "null" {
        return Ok(CrsResolution::Missing);
    }
    if kind == Some("authority_code") {
        let id: String = crate::json::from_str(text).map_err(|_| incoerente())?;
        return authority(&id);
    }
    let (definition, format) = match kind {
        None | Some("projjson") if text.starts_with('{') => {
            (text.to_owned(), CrsDefinitionFormat::Projjson)
        }
        Some("wkt2:2019") => (
            serde_json::from_str::<String>(text).map_err(|_| incoerente())?,
            CrsDefinitionFormat::Wkt2,
        ),
        _ => return Err(incoerente()),
    };
    let ids =
        crate::crs_definizione::identificatori(&definition, format).map_err(|_| incoerente())?;
    let id = ids.first().and_then(|(a, c)| match (a, c) {
        (
            crate::crs_definizione::TestoDefinizione::Unicode(a),
            crate::crs_definizione::TestoDefinizione::Unicode(c),
        ) => Some(format!("{a}:{c}")),
        _ => None,
    });
    if crate::crs_definizione::contraddice(id.as_deref(), &definition, format)
        .map_err(|_| incoerente())?
    {
        return Err(incoerente());
    }
    if id
        .as_deref()
        .is_none_or(crate::crs_definizione::autorita_locale)
    {
        return Ok(CrsResolution::DeclaredButUnresolved(RawCrs::new(
            definition, id,
        )));
    }
    let crs_kind = crate::crs::crs_kind_for(id.as_deref(), Some(&definition), "arrow")?;
    let mut crs = ResolvedCrs::new(id, crs_kind, Some(definition)).stored_east_first();
    crs.definition_format = Some(format);
    Ok(crs.into())
}

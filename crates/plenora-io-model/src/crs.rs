//! Contratto CRS del bordo (scheletro Fase 0). Nessuna riproiezione qui: la
//! trasformazione è lo step `geo.reproject` di data-tools.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrsKind {
    Geographic,
    Projected,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisOrder {
    LongitudeLatitude,
    LatitudeLongitude,
    EastingNorthing,
    NorthingEasting,
    Other,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CrsDefinitionFormat {
    Wkt,
    Wkt2,
    Projjson,
}

/// CRS risolto associato a una colonna geometria.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResolvedCrs {
    /// Identificatore leggibile, es. "EPSG:4326" / "OGC:CRS84".
    pub id: Option<String>,
    pub kind: CrsKind,
    /// Ordine assi dichiarato dall'autorità/formato, mai canonicalizzato.
    pub axis_order: AxisOrder,
    /// Rappresentazione sorgente (WKT o PROJJSON), se disponibile.
    pub definition: Option<String>,
    /// Formato della definizione sorgente. Obbligatorio quando `definition`
    /// è presente; il costruttore lo ricava dalla grammatica riconoscibile.
    pub definition_format: Option<CrsDefinitionFormat>,
}

impl ResolvedCrs {
    pub fn new(id: Option<String>, kind: CrsKind, definition: Option<String>) -> Self {
        let axis_order = axis_order_for(id.as_deref(), kind);
        let definition_format = definition.as_deref().map(definition_format);
        Self {
            id,
            kind,
            axis_order,
            definition,
            definition_format,
        }
    }

    #[must_use]
    pub fn with_definition_format(mut self, format: CrsDefinitionFormat) -> Self {
        self.definition_format = self.definition.as_ref().map(|_| format);
        self
    }

    pub fn wgs84() -> Self {
        Self::new(Some("OGC:CRS84".to_owned()), CrsKind::Geographic, None)
    }
}

pub(crate) fn definition_format(definition: &str) -> CrsDefinitionFormat {
    let definition = definition.trim_start();
    if definition.starts_with('{') {
        CrsDefinitionFormat::Projjson
    } else if [
        "GEOGCRS[",
        "PROJCRS[",
        "GEODCRS[",
        "VERTCRS[",
        "COMPOUNDCRS[",
        "BOUNDCRS[",
        "ENGCRS[",
        "PARAMETRICCRS[",
        "TIMECRS[",
    ]
    .iter()
    .any(|prefix| definition.starts_with(prefix))
    {
        CrsDefinitionFormat::Wkt2
    } else {
        CrsDefinitionFormat::Wkt
    }
}

pub fn axis_order_for(id: Option<&str>, kind: CrsKind) -> AxisOrder {
    match id {
        Some(id) if id.eq_ignore_ascii_case("OGC:CRS84") => AxisOrder::LongitudeLatitude,
        Some(id) if id.eq_ignore_ascii_case("EPSG:4326") => AxisOrder::LatitudeLongitude,
        _ if kind == CrsKind::Projected => AxisOrder::EastingNorthing,
        _ => AxisOrder::Unknown,
    }
}

/// Estrae il codice numerico da un identificatore di autorità EPSG.
///
/// Gli altri namespace non sono interpretati come SRID: una loro eventuale
/// equivalenza richiede un resolver CRS, non una regola sintattica al bordo.
pub fn authority_srid(value: &str) -> Option<u32> {
    let (authority, code) = value.split_once(':')?;
    authority
        .eq_ignore_ascii_case("EPSG")
        .then(|| code.parse::<u32>().ok())
        .flatten()
}

/// Estrae l'identificatore EPSG della definizione CRS soltanto quando è
/// dichiarato alla radice. Gli identificatori dei CRS base annidati non
/// descrivono necessariamente il CRS esterno (per esempio un `PROJCS` EPSG:3003
/// contiene un `GEOGCS` EPSG:4326) e vengono quindi ignorati.
///
/// Questa funzione non è un resolver di equivalenza: se la definizione non
/// porta un identificatore EPSG radice, il bordo non deduce nulla.
pub fn definition_authority_srid(definition: &str, format: CrsDefinitionFormat) -> Option<u32> {
    match format {
        CrsDefinitionFormat::Projjson => projjson_root_epsg(definition),
        CrsDefinitionFormat::Wkt | CrsDefinitionFormat::Wkt2 => wkt_root_epsg(definition),
    }
}

fn projjson_root_epsg(definition: &str) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_str(definition).ok()?;
    let id = value.as_object()?.get("id")?.as_object()?;
    let authority = id.get("authority")?.as_str()?;
    if !authority.eq_ignore_ascii_case("EPSG") {
        return None;
    }
    match id.get("code")? {
        serde_json::Value::Number(code) => u32::try_from(code.as_u64()?).ok(),
        serde_json::Value::String(code) => code.parse().ok(),
        _ => None,
    }
}

fn wkt_root_epsg(definition: &str) -> Option<u32> {
    let upper = definition.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let mut depth = 0_u32;
    let mut quoted = false;
    let mut index = 0_usize;
    let mut root_code = None;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => quoted = !quoted,
            b'[' if !quoted => depth = depth.checked_add(1)?,
            b']' if !quoted => depth = depth.checked_sub(1)?,
            _ if !quoted && depth == 1 => {
                for marker in ["AUTHORITY[", "ID["] {
                    if upper[index..].starts_with(marker) {
                        let tail = &definition[index + marker.len()..];
                        if let Some(code) = parse_epsg_wkt_identifier(tail) {
                            // Più identificatori EPSG alla radice sono
                            // ambigui: in quel caso non scegliamo.
                            if root_code.replace(code).is_some() {
                                return None;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }
    root_code
}

fn parse_epsg_wkt_identifier(tail: &str) -> Option<u32> {
    let mut parts = tail.splitn(3, ',');
    let authority = parts.next()?.trim().trim_matches(['"', '\'']);
    if !authority.eq_ignore_ascii_case("EPSG") {
        return None;
    }
    let code = parts
        .next()?
        .trim()
        .trim_matches(['"', '\''])
        .split(|character: char| !character.is_ascii_digit())
        .next()?;
    code.parse().ok()
}

/// Rappresentazione CRS presente nella sorgente ma non risolta in modo
/// affidabile. È conservata per diagnostica e round-trip metadata, mai usata
/// come CRS operativo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawCrs {
    /// Rappresentazione sorgente, se dichiarata. Un identificatore di autorità
    /// non risolvibile è sufficiente a distinguere questo stato da `Missing`.
    pub definition: Option<String>,
    pub authority_hint: Option<String>,
    /// Presente se e solo se `definition` è presente.
    pub definition_format: Option<CrsDefinitionFormat>,
    pub axis_order: AxisOrder,
}

impl RawCrs {
    #[must_use]
    pub fn new(definition: String, authority_hint: Option<String>) -> Self {
        let definition_format = Some(definition_format(&definition));
        let axis_order = axis_order_for(authority_hint.as_deref(), CrsKind::Unknown);
        Self {
            definition: Some(definition),
            authority_hint,
            definition_format,
            axis_order,
        }
    }

    /// Conserva un identificatore dichiarato che il bordo non ha risolto.
    /// Non sintetizza una definizione testuale né rende il CRS operativo.
    #[must_use]
    pub fn from_authority_hint(authority_hint: String) -> Self {
        let axis_order = axis_order_for(Some(&authority_hint), CrsKind::Unknown);
        Self {
            definition: None,
            authority_hint: Some(authority_hint),
            definition_format: None,
            axis_order,
        }
    }
}

/// Stato esplicito della risoluzione CRS (ADR-IO 4). Evita di rappresentare
/// `unknown` come se fosse un [`ResolvedCrs`] valido.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CrsResolution {
    Resolved(ResolvedCrs),
    DeclaredButUnresolved(RawCrs),
    Missing,
}

impl CrsResolution {
    pub fn resolved(crs: ResolvedCrs) -> Self {
        Self::Resolved(crs)
    }

    pub fn as_resolved(&self) -> Option<&ResolvedCrs> {
        match self {
            Self::Resolved(crs) => Some(crs),
            Self::DeclaredButUnresolved(_) | Self::Missing => None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.as_resolved().and_then(|crs| crs.id.as_deref())
    }

    pub fn definition(&self) -> Option<&str> {
        self.as_resolved().and_then(|crs| crs.definition.as_deref())
    }

    pub fn raw(&self) -> Option<&RawCrs> {
        match self {
            Self::DeclaredButUnresolved(raw) => Some(raw),
            Self::Resolved(_) | Self::Missing => None,
        }
    }
}

impl From<ResolvedCrs> for CrsResolution {
    fn from(value: ResolvedCrs) -> Self {
        Self::Resolved(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crs84_and_epsg_4326_keep_distinct_axis_orders() {
        let crs84 = ResolvedCrs::wgs84();
        let epsg4326 = ResolvedCrs::new(Some("EPSG:4326".to_owned()), CrsKind::Geographic, None);

        assert_eq!(crs84.axis_order, AxisOrder::LongitudeLatitude);
        assert_eq!(epsg4326.axis_order, AxisOrder::LatitudeLongitude);
        assert_ne!(crs84, epsg4326);
    }

    #[test]
    fn projected_crs_declares_easting_northing() {
        let crs = ResolvedCrs::new(Some("EPSG:3857".to_owned()), CrsKind::Projected, None);
        assert_eq!(crs.axis_order, AxisOrder::EastingNorthing);
    }

    #[test]
    fn raw_resolution_is_not_operational() {
        let raw = RawCrs::new("LOCAL_CS[\"private\"]".to_owned(), Some("LOCAL".to_owned()));
        let resolution = CrsResolution::DeclaredButUnresolved(raw.clone());
        assert_eq!(resolution.raw(), Some(&raw));
        assert!(resolution.as_resolved().is_none());
    }

    #[test]
    fn unresolved_authority_does_not_require_or_invent_a_definition() {
        let raw = RawCrs::from_authority_hint("EPSG:99999".to_owned());
        let resolution = CrsResolution::DeclaredButUnresolved(raw.clone());

        assert_eq!(raw.authority_hint.as_deref(), Some("EPSG:99999"));
        assert_eq!(raw.definition, None);
        assert_eq!(raw.definition_format, None);
        assert_eq!(raw.axis_order, AxisOrder::Unknown);
        assert!(resolution.as_resolved().is_none());
    }

    #[test]
    fn authority_srid_only_resolves_numeric_epsg_ids() {
        assert_eq!(authority_srid("EPSG:4326"), Some(4326));
        assert_eq!(authority_srid("epsg:3003"), Some(3003));
        assert_eq!(authority_srid("OGC:CRS84"), None);
        assert_eq!(authority_srid("EPSG:not-a-code"), None);
        assert_eq!(authority_srid("EPSG:4326:extra"), None);
    }

    #[test]
    fn definition_format_is_explicit_and_not_inferred_by_consumers() {
        let wkt = ResolvedCrs::new(
            None,
            CrsKind::Geographic,
            Some("GEOGCS[\"WGS 84\"]".to_owned()),
        );
        let wkt2 = ResolvedCrs::new(
            None,
            CrsKind::Geographic,
            Some("GEOGCRS[\"WGS 84\"]".to_owned()),
        );
        let projjson = ResolvedCrs::new(
            None,
            CrsKind::Geographic,
            Some("{\"type\":\"GeographicCRS\"}".to_owned()),
        );
        assert_eq!(wkt.definition_format, Some(CrsDefinitionFormat::Wkt));
        assert_eq!(wkt2.definition_format, Some(CrsDefinitionFormat::Wkt2));
        assert_eq!(
            projjson.definition_format,
            Some(CrsDefinitionFormat::Projjson)
        );
    }

    #[test]
    fn definition_epsg_uses_root_identifier_not_nested_base_crs() {
        let projected = concat!(
            "PROJCS[\"Monte Mario / Italy zone 1\",",
            "GEOGCS[\"Monte Mario\",AUTHORITY[\"EPSG\",\"4265\"]],",
            "AUTHORITY[\"EPSG\",\"3003\"]]"
        );
        assert_eq!(
            definition_authority_srid(projected, CrsDefinitionFormat::Wkt),
            Some(3003)
        );

        let wkt2 = concat!(
            "PROJCRS[\"WGS 84 / UTM zone 32N\",",
            "BASEGEOGCRS[\"WGS 84\",ID[\"EPSG\",4326]],",
            "ID[\"EPSG\",32632]]"
        );
        assert_eq!(
            definition_authority_srid(wkt2, CrsDefinitionFormat::Wkt2),
            Some(32632)
        );
    }

    #[test]
    fn definition_epsg_reads_projjson_root_id_only() {
        let definition = r#"{
            "type":"ProjectedCRS",
            "base_crs":{"id":{"authority":"EPSG","code":4326}},
            "id":{"authority":"EPSG","code":3003}
        }"#;
        assert_eq!(
            definition_authority_srid(definition, CrsDefinitionFormat::Projjson),
            Some(3003)
        );
        assert_eq!(
            definition_authority_srid(
                r#"{"id":{"authority":"OGC","code":"CRS84"}}"#,
                CrsDefinitionFormat::Projjson
            ),
            None
        );
    }

    #[test]
    fn definition_epsg_rejects_ambiguous_or_nested_only_ids() {
        assert_eq!(
            definition_authority_srid(
                "GEOGCS[\"unnamed\",DATUM[\"x\",AUTHORITY[\"EPSG\",\"6326\"]]]",
                CrsDefinitionFormat::Wkt
            ),
            None
        );
        assert_eq!(
            definition_authority_srid(
                "GEOGCS[\"x\",AUTHORITY[\"EPSG\",\"4326\"],ID[\"EPSG\",4326]]",
                CrsDefinitionFormat::Wkt
            ),
            None
        );
    }
}

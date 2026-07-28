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

fn definition_format(definition: &str) -> CrsDefinitionFormat {
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

/// Rappresentazione CRS presente nella sorgente ma non risolta in modo
/// affidabile. È conservata per diagnostica e round-trip metadata, mai usata
/// come CRS operativo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawCrs {
    pub definition: String,
    pub authority_hint: Option<String>,
    pub definition_format: CrsDefinitionFormat,
    pub axis_order: AxisOrder,
}

impl RawCrs {
    #[must_use]
    pub fn new(definition: String, authority_hint: Option<String>) -> Self {
        let definition_format = definition_format(&definition);
        let axis_order = axis_order_for(authority_hint.as_deref(), CrsKind::Unknown);
        Self {
            definition,
            authority_hint,
            definition_format,
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
}

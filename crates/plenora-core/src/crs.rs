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

/// CRS risolto associato a una colonna geometria.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResolvedCrs {
    /// Identificatore leggibile, es. "EPSG:4326" / "OGC:CRS84".
    pub id: Option<String>,
    pub kind: CrsKind,
    /// Rappresentazione sorgente (WKT o PROJJSON), se disponibile.
    pub definition: Option<String>,
}

impl ResolvedCrs {
    pub fn wgs84() -> Self {
        Self {
            id: Some("OGC:CRS84".to_owned()),
            kind: CrsKind::Geographic,
            definition: None,
        }
    }
}

/// Rappresentazione CRS presente nella sorgente ma non risolta in modo
/// affidabile. È conservata per diagnostica e round-trip metadata, mai usata
/// come CRS operativo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawCrs {
    pub definition: String,
    pub authority_hint: Option<String>,
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
}

impl From<ResolvedCrs> for CrsResolution {
    fn from(value: ResolvedCrs) -> Self {
        Self::Resolved(value)
    }
}

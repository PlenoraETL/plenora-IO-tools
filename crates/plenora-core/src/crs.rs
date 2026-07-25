//! Contratto CRS del bordo (scheletro Fase 0). Nessuna riproiezione qui: la
//! trasformazione è lo step `geo.reproject` di data-tools.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrsKind {
    Geographic,
    Projected,
    Unknown,
}

/// CRS risolto associato a una colonna geometria.
#[derive(Clone, Debug)]
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

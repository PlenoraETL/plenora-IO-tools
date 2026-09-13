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

    #[must_use]
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

#[must_use]
pub fn axis_order_for(id: Option<&str>, kind: CrsKind) -> AxisOrder {
    match id {
        Some(id) if id.eq_ignore_ascii_case("OGC:CRS84") => AxisOrder::LongitudeLatitude,
        Some(id) if id.eq_ignore_ascii_case("EPSG:4326") => AxisOrder::LatitudeLongitude,
        _ if kind == CrsKind::Projected => AxisOrder::EastingNorthing,
        _ => AxisOrder::Unknown,
    }
}

/// Classifica gli identificatori di autorità il cui tipo CRS è noto senza
/// risolvere una definizione esterna.
///
/// Gli altri identificatori restano sconosciuti: dedurre il tipo dal solo
/// codice richiederebbe un resolver CRS, che non appartiene al bordo I/O.
#[must_use]
pub const fn crs_kind_for_authority_id(id: &str) -> CrsKind {
    if id.eq_ignore_ascii_case("OGC:CRS84") || id.eq_ignore_ascii_case("EPSG:4326") {
        CrsKind::Geographic
    } else {
        CrsKind::Unknown
    }
}

/// Estrae il codice numerico da un identificatore di autorità EPSG.
///
/// Gli altri namespace non sono interpretati come SRID: una loro eventuale
/// equivalenza richiede un resolver CRS, non una regola sintattica al bordo.
#[must_use]
pub fn authority_srid(value: &str) -> Option<u32> {
    let (authority, code) = value.split_once(':')?;
    authority
        .eq_ignore_ascii_case("EPSG")
        .then(|| code.parse::<u32>().ok())
        .flatten()
}

/// Estrae l'identificatore EPSG della definizione CRS soltanto quando è
/// dichiarato alla radice.
///
/// Gli identificatori dei CRS base annidati non descrivono necessariamente il
/// CRS esterno (per esempio un `PROJCS` EPSG:3003 contiene un `GEOGCS`
/// EPSG:4326) e vengono quindi ignorati.
///
/// Questa funzione non è un resolver di equivalenza: se la definizione non
/// porta un identificatore EPSG radice, il bordo non deduce nulla.
#[must_use]
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
                    // Il confronto e' **sui byte**, non sulla stringa.
                    //
                    // `index` scorre `bytes`, quindi puo' cadere dentro un
                    // carattere multi-byte; `upper[index..]` in quel caso
                    // panicava — trovato dal fuzzer su un WKT contenente un
                    // ideogramma. Un `&[u8]` non ha confini di carattere da
                    // rispettare, e il marcatore e' ASCII: nessun byte ASCII
                    // compare mai dentro una sequenza UTF-8 multi-byte, quindi
                    // se il confronto riesce `index` e' per forza un confine.
                    if bytes[index..].starts_with(marker.as_bytes()) {
                        // `get` invece dell'indicizzazione: la deduzione sopra
                        // e' vera, ma affidarle un panic significa che se un
                        // giorno smettesse di esserlo il difetto tornerebbe
                        // ad abortire il processo invece di dare `None`.
                        let Some(tail) = definition.get(index + marker.len()..) else {
                            continue;
                        };
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

/// Stato esplicito della risoluzione CRS (`PRODUCT.md § CRS`). Evita di rappresentare
/// `unknown` come se fosse un [`ResolvedCrs`] valido.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CrsResolution {
    Resolved(ResolvedCrs),
    /// Serializzato `declared_unresolved`, non `declared_but_unresolved`.
    ///
    /// La grafia non e' una preferenza: `ARROW-VOCABULARY-1.0 §3` chiude
    /// `plenora.geometry.crs_resolution` su `resolved`, `declared_unresolved` e
    /// `missing`, e i metadati Arrow che scriviamo la usano gia'. Il JSON della
    /// busta rendeva `declared_but_unresolved` perche' era quella che serde
    /// deriva dal nome della variante, e il risultato erano **due grafie per lo
    /// stesso stato dentro lo stesso prodotto**: chi confrontava le due viste
    /// doveva tradurre, e una traduzione non scritta prima o poi si sbaglia.
    ///
    /// Il nome Rust resta `DeclaredButUnresolved` perche' li' si legge meglio;
    /// a viaggiare e' la grafia del contratto.
    #[serde(rename = "declared_unresolved")]
    DeclaredButUnresolved(RawCrs),
    Missing,
}

impl CrsResolution {
    #[must_use]
    pub const fn resolved(crs: ResolvedCrs) -> Self {
        Self::Resolved(crs)
    }

    #[must_use]
    pub const fn as_resolved(&self) -> Option<&ResolvedCrs> {
        match self {
            Self::Resolved(crs) => Some(crs),
            Self::DeclaredButUnresolved(_) | Self::Missing => None,
        }
    }

    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.as_resolved().and_then(|crs| crs.id.as_deref())
    }

    #[must_use]
    pub fn definition(&self) -> Option<&str> {
        self.as_resolved().and_then(|crs| crs.definition.as_deref())
    }

    #[must_use]
    pub const fn raw(&self) -> Option<&RawCrs> {
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
mod tests;

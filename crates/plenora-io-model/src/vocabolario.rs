//! La verifica dello schema Arrow in ingresso (REJ-001, REJ-002).
//!
//! Classi e ordine di Arrow Geometry Semantics 1.0: versione del contratto,
//! vocabolario, CRS, supporto. I valori si verificano dopo, riga per riga.
//!
//! Ogni classe ha una categoria sola: versione assente o malformata e ogni
//! difetto di vocabolario `schema`, versione ben formata ma non supportata e
//! limiti di questo componente `unsupported`, ogni difetto di una chiave CRS,
//! anche l'assenza, `crs`. La prima classe difettosa decide, nell'ordine.

use arrow_schema::{DataType, Field, Schema};

use crate::crs::CrsDefinitionFormat;
use crate::crs_definizione;
use crate::error::{PlenoraIoError, PublicMessage, Result};

const VERSIONE: &str = "plenora.contract.version";
const ESTENSIONE: &str = "ARROW:extension:name";
const GEOARROW_WKB: &str = "geoarrow.wkb";
const FIELD_ID: &str = "plenora.field_id";
const PREFISSO_GEOMETRIA: &str = "plenora.geometry.";
const PREFISSO_NATIVO: &str = "plenora.geometry.native.";

const OBBLIGATORIE: [&str; 7] = [
    FIELD_ID,
    "plenora.geometry.encoding",
    "plenora.geometry.dimensions",
    "plenora.geometry.spatial_semantics",
    "plenora.geometry.precision",
    "plenora.geometry.types_declaration",
    "plenora.geometry.crs_resolution",
];

const VALORI_CHIUSI: [(&str, &[&str]); 8] = [
    ("plenora.geometry.encoding", &["wkb", "ewkb"]),
    (
        "plenora.geometry.dimensions",
        &["xy", "xyz", "xym", "xyzm", "unknown"],
    ),
    (
        "plenora.geometry.spatial_semantics",
        &["geometry", "geography"],
    ),
    (
        "plenora.geometry.precision",
        &["float64", "float32", "native"],
    ),
    (
        "plenora.geometry.types_declaration",
        &["exact", "mixed", "unresolved"],
    ),
    (
        "plenora.geometry.crs_resolution",
        &["resolved", "declared_unresolved", "missing"],
    ),
    (
        "plenora.geometry.crs_definition_format",
        &["wkt", "wkt2", "projjson"],
    ),
    (
        "plenora.geometry.axis_order",
        &[
            "lon_lat",
            "lat_lon",
            "easting_northing",
            "northing_easting",
            "other",
            "unknown",
        ],
    ),
];

/// Le chiavi il cui ogni difetto, assenza compresa, e' della classe CRS.
const CHIAVI_CRS: [&str; 5] = [
    "plenora.geometry.crs_resolution",
    "plenora.geometry.crs_id",
    "plenora.geometry.crs_definition",
    "plenora.geometry.crs_definition_format",
    "plenora.geometry.axis_order",
];

const ALTRE_CHIAVI: [&str; 4] = [
    "plenora.geometry.srid",
    "plenora.geometry.types",
    "plenora.geometry.crs_id",
    "plenora.geometry.crs_definition",
];

const TIPI_CANONICI: [&str; 16] = [
    "point",
    "linestring",
    "polygon",
    "multipoint",
    "multilinestring",
    "multipolygon",
    "geometrycollection",
    "circularstring",
    "compoundcurve",
    "curvepolygon",
    "multicurve",
    "multisurface",
    "polyhedralsurface",
    "tin",
    "triangle",
    "unknown",
];

const VERSIONE_ASSENTE_O_MALFORMATA: PublicMessage = PublicMessage::Curated(
    "versione del contratto assente o malformata in uno schema con metadati plenora",
);
const VERSIONE_NON_SUPPORTATA: PublicMessage =
    PublicMessage::Curated("versione del contratto non supportata");
const FIELD_ID_NON_CANONICO: PublicMessage =
    PublicMessage::Curated("plenora.field_id non decimale canonico");
const GEOMETRIA_SU_NON_GEOMETRIA: PublicMessage =
    PublicMessage::Curated("chiave plenora.geometry su un campo senza estensione geoarrow.wkb");
const GEOMETRIA_NON_BINARIA: PublicMessage =
    PublicMessage::Curated("campo geoarrow.wkb con tipo Arrow non binario");
const CHIAVE_OBBLIGATORIA: PublicMessage =
    PublicMessage::Curated("campo geometria senza una chiave obbligatoria del vocabolario");
const VALORE_FUORI_VOCABOLARIO: PublicMessage =
    PublicMessage::Curated("valore fuori dal vocabolario chiuso");
const SRID_NON_VALIDO: PublicMessage =
    PublicMessage::Curated("plenora.geometry.srid non intero a 32 bit");
const TIPI_NON_VALIDI: PublicMessage =
    PublicMessage::Curated("plenora.geometry.types incoerente con la dichiarazione o non canonico");
const CHIAVE_SCONOSCIUTA: PublicMessage =
    PublicMessage::Curated("chiave plenora.geometry sconosciuta");
const FIELD_ID_RIPETUTO: PublicMessage =
    PublicMessage::Curated("plenora.field_id ripetuto nello schema");
const CRS: PublicMessage = PublicMessage::Curated("metadato CRS non valido");
const DEFINIZIONE_MALFORMATA: PublicMessage =
    PublicMessage::Curated("definizione CRS non ben formata nel formato dichiarato");
const DEFINIZIONE_CONTRADDICE: PublicMessage =
    PublicMessage::Curated("definizione CRS che descrive un CRS diverso da crs_id");
const AUTORITA_LOCALE: PublicMessage =
    PublicMessage::Curated("crs_id di un catalogo locale dichiarato risolto");

/// Verifica lo schema nelle classi e nell'ordine di REJ-002 fino al supporto.
///
/// Uno schema senza versione e senza alcuna chiave `plenora.*` e' un Arrow
/// estraneo al contratto: non si rifiuta qui, e lo legge il driver come prima.
///
/// # Errors
/// Il primo difetto nell'ordine versione, vocabolario, CRS, supporto, con la
/// categoria della sua classe e senza valori dello schema nel messaggio.
pub fn verifica_schema(schema: &Schema) -> Result<()> {
    match schema.metadata().get(VERSIONE) {
        None if !porta_metadati_plenora(schema) => return Ok(()),
        None => {
            return Err(PlenoraIoError::schema_redatto(
                &VERSIONE_ASSENTE_O_MALFORMATA,
            ))
        }
        Some(v) if !intero_positivo(v) => {
            return Err(PlenoraIoError::schema_redatto(
                &VERSIONE_ASSENTE_O_MALFORMATA,
            ))
        }
        Some(v) if v != "1" => {
            return Err(PlenoraIoError::non_supportato_redatto(
                &VERSIONE_NON_SUPPORTATA,
            ))
        }
        Some(_) => {}
    }
    vocabolario(schema)?;
    for campo in schema.fields().iter().filter(|f| geometria(f)) {
        crs(campo)?;
    }
    Ok(())
}

fn porta_metadati_plenora(schema: &Schema) -> bool {
    schema.metadata().keys().any(|k| k.starts_with("plenora."))
        || schema
            .fields()
            .iter()
            .any(|f| f.metadata().keys().any(|k| k.starts_with("plenora.")))
}

fn geometria(campo: &Field) -> bool {
    campo
        .metadata()
        .get(ESTENSIONE)
        .is_some_and(|v| v == GEOARROW_WKB)
}

/// `0|[1-9][0-9]*`.
fn decimale(testo: &str) -> bool {
    testo == "0" || intero_positivo(testo)
}

/// `[1-9][0-9]*`.
fn intero_positivo(testo: &str) -> bool {
    let mut cifre = testo.bytes();
    matches!(cifre.next(), Some(b'1'..=b'9')) && cifre.all(|b| b.is_ascii_digit())
}

/// `0|-?[1-9][0-9]*` dentro i 32 bit con segno.
fn srid(testo: &str) -> bool {
    let assoluto = testo.strip_prefix('-').unwrap_or(testo);
    (testo == "0" || intero_positivo(assoluto)) && testo.parse::<i32>().is_ok()
}

/// `[A-Za-z][A-Za-z0-9_.-]*:[^\s:]+`.
fn crs_id(testo: &str) -> bool {
    let Some((autorita, codice)) = testo.split_once(':') else {
        return false;
    };
    let mut lettere = autorita.chars();
    lettere.next().is_some_and(|c| c.is_ascii_alphabetic())
        && lettere.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && !codice.is_empty()
        // `\s` di Python, non `White_Space`: U+001C..U+001F sono spazi anche
        // per la grammatica (GEO-016).
        && !codice
            .chars()
            .any(|c| crate::crs_definizione::spazio_python(c) || c == ':')
}

fn chiusa(chiave: &str, valore: &str) -> bool {
    VALORI_CHIUSI
        .iter()
        .find(|(k, _)| *k == chiave)
        .is_none_or(|(_, ammessi)| ammessi.contains(&valore))
}

fn vocabolario(schema: &Schema) -> Result<()> {
    let errore = || PlenoraIoError::schema_redatto(&TIPI_NON_VALIDI);
    let mut identita = Vec::new();
    for campo in schema.fields() {
        let md = campo.metadata();
        if let Some(id) = md.get(FIELD_ID) {
            if !decimale(id) {
                return Err(PlenoraIoError::schema_redatto(&FIELD_ID_NON_CANONICO));
            }
            identita.push(id.as_str());
        }
        if !geometria(campo) {
            if md.keys().any(|k| k.starts_with(PREFISSO_GEOMETRIA)) {
                return Err(PlenoraIoError::schema_redatto(&GEOMETRIA_SU_NON_GEOMETRIA));
            }
            continue;
        }
        if !matches!(campo.data_type(), DataType::Binary | DataType::LargeBinary) {
            return Err(PlenoraIoError::schema_redatto(&GEOMETRIA_NON_BINARIA));
        }
        if OBBLIGATORIE
            .iter()
            .any(|k| !CHIAVI_CRS.contains(k) && !md.contains_key(k))
        {
            return Err(PlenoraIoError::schema_redatto(&CHIAVE_OBBLIGATORIA));
        }
        let nota = |k: &str| {
            OBBLIGATORIE.contains(&k)
                || VALORI_CHIUSI.iter().any(|(c, _)| *c == k)
                || ALTRE_CHIAVI.contains(&k)
        };
        if md.keys().any(|k| {
            k.starts_with(PREFISSO_GEOMETRIA) && !k.starts_with(PREFISSO_NATIVO) && !nota(k)
        }) {
            return Err(PlenoraIoError::schema_redatto(&CHIAVE_SCONOSCIUTA));
        }
        if md
            .iter()
            .any(|(k, v)| !CHIAVI_CRS.contains(&k.as_str()) && !chiusa(k, v))
        {
            return Err(PlenoraIoError::schema_redatto(&VALORE_FUORI_VOCABOLARIO));
        }
        if md.get("plenora.geometry.srid").is_some_and(|v| !srid(v)) {
            return Err(PlenoraIoError::schema_redatto(&SRID_NON_VALIDO));
        }
        let dichiarazione = md
            .get("plenora.geometry.types_declaration")
            .map(String::as_str);
        match md.get("plenora.geometry.types").map(String::as_str) {
            Some("") => return Err(PlenoraIoError::schema_redatto(&CHIAVE_SCONOSCIUTA)),
            None if dichiarazione == Some("exact") => return Err(errore()),
            Some(_) if dichiarazione == Some("unresolved") => return Err(errore()),
            Some(tipi) => {
                let mut precedente = None;
                for tipo in tipi.split(',') {
                    let posizione = TIPI_CANONICI
                        .iter()
                        .position(|t| *t == tipo)
                        .ok_or_else(errore)?;
                    if precedente.is_some_and(|p| p >= posizione) {
                        return Err(errore());
                    }
                    precedente = Some(posizione);
                }
            }
            None => {}
        }
    }
    let mut ordinate = identita.clone();
    ordinate.sort_unstable();
    ordinate.dedup();
    if ordinate.len() != identita.len() {
        return Err(PlenoraIoError::schema_redatto(&FIELD_ID_RIPETUTO));
    }
    Ok(())
}

fn crs(campo: &Field) -> Result<()> {
    let errore = || PlenoraIoError::crs_redatto(&CRS);
    let md = campo.metadata();
    let Some(risoluzione) = md
        .get("plenora.geometry.crs_resolution")
        .map(String::as_str)
    else {
        return Err(errore());
    };
    if md
        .iter()
        .any(|(k, v)| CHIAVI_CRS.contains(&k.as_str()) && !chiusa(k, v))
    {
        return Err(errore());
    }
    let definizione = md
        .get("plenora.geometry.crs_definition")
        .map(String::as_str);
    if definizione == Some("") {
        return Err(errore());
    }
    let id = md.get("plenora.geometry.crs_id").map(String::as_str);
    if id.is_some_and(|v| !crs_id(v)) {
        return Err(errore());
    }
    let formato = md
        .get("plenora.geometry.crs_definition_format")
        .map(String::as_str);
    let asse = md.get("plenora.geometry.axis_order");
    if definizione.is_some() != formato.is_some() {
        return Err(errore());
    }
    if risoluzione == "missing" {
        return if id.is_some() || definizione.is_some() || formato.is_some() || asse.is_some() {
            Err(errore())
        } else {
            Ok(())
        };
    }
    if id.is_none() && definizione.is_none() || asse.is_none() {
        return Err(errore());
    }
    if risoluzione == "resolved" && id.is_some_and(crs_definizione::autorita_locale) {
        return Err(PlenoraIoError::crs_redatto(&AUTORITA_LOCALE));
    }
    if let (Some(testo), Some(formato)) = (definizione, formato) {
        let formato = match formato {
            "wkt" => CrsDefinitionFormat::Wkt,
            "wkt2" => CrsDefinitionFormat::Wkt2,
            _ => CrsDefinitionFormat::Projjson,
        };
        // Il tetto unico delle definizioni, prima del lettore: oltre il tetto il
        // lettore stretto la dice malformata, e fino alla rilettura 10 una
        // definizione di 1 MiB e un byte usciva `crs` «non ben formata» invece
        // di `limit_exceeded`. E' lo stesso ordine della lettura dei metadati
        // del campo (`geometry::read_geometry_contract_metadata`).
        crs_definizione::verifica_dimensione_definizione(testo, None)?;
        match crs_definizione::contraddice(id, testo, formato) {
            Err(crs_definizione::Malformata) => {
                return Err(PlenoraIoError::crs_redatto(&DEFINIZIONE_MALFORMATA))
            }
            Ok(true) => return Err(PlenoraIoError::crs_redatto(&DEFINIZIONE_CONTRADDICE)),
            Ok(false) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

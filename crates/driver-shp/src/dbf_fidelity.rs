//! Confine raw DBF: la decodifica permissiva interna di dbase non e' mai
//! l'autorita' per nomi, Character o numeri ASCII. Tutti sono verificati e
//! ricostruiti dai byte, prima dell'inferenza e prima di emettere una riga.
use super::{err, DbfExactIntegerRows, DbfFieldLayout};
use plenora_io_model::{PlenoraIoError, PublicMessage, Result};
use shapefile::dbase::{FieldValue, Record};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub enum TextEncoding {
    Ascii,
    Latin1,
    Encoded(&'static encoding_rs::Encoding),
}

impl TextEncoding {
    pub(super) fn from_path(path: &Path) -> Result<Self> {
        let mut header = [0; 32];
        File::open(path)
            .map_err(|_| codepage_error())?
            .read_exact(&mut header)
            .map_err(|_| codepage_error())?;
        let cpg = match File::open(path.with_extension("cpg")) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(129)
                    .read_to_end(&mut bytes)
                    .map_err(|_| codepage_error())?;
                if bytes.len() > 128 {
                    return Err(codepage_error());
                }
                Some(String::from_utf8(bytes).map_err(|_| codepage_error())?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(codepage_error()),
        };
        if let Some(cpg) = cpg {
            return match cpg
                .trim()
                .trim_start_matches('\u{feff}')
                .to_ascii_uppercase()
                .as_str()
            {
                "UTF-8" | "UTF8" | "65001" => Ok(Self::Encoded(encoding_rs::UTF_8)),
                "ISO-8859-1" | "LATIN1" | "8859-1" | "28591" => Ok(Self::Latin1),
                "ASCII" | "US-ASCII" | "20127" => Ok(Self::Ascii),
                name => {
                    let number = name
                        .strip_prefix("WINDOWS-")
                        .or_else(|| name.strip_prefix("CP"))
                        .unwrap_or(name);
                    Self::windows(number)
                }
            };
        }
        match header[29] {
            // Nessuna codepage: solo ASCII e' non ambiguo.
            0x00 => Ok(Self::Ascii),
            0xf0 => Ok(Self::Encoded(encoding_rs::UTF_8)),
            // Convenzione ESRI/GDAL per LDID 87: ISO-8859-1, non CP1252.
            0x57 => Ok(Self::Latin1),
            0x03 => Self::windows("1252"),
            0xc8 => Self::windows("1250"),
            0xc9 => Self::windows("1251"),
            0xca => Self::windows("1254"),
            0xcb => Self::windows("1253"),
            _ => Err(codepage_error()),
        }
    }

    fn windows(number: &str) -> Result<Self> {
        Ok(Self::Encoded(match number {
            "1250" => encoding_rs::WINDOWS_1250,
            "1251" => encoding_rs::WINDOWS_1251,
            "1252" => encoding_rs::WINDOWS_1252,
            "1253" => encoding_rs::WINDOWS_1253,
            "1254" => encoding_rs::WINDOWS_1254,
            "1255" => encoding_rs::WINDOWS_1255,
            "1256" => encoding_rs::WINDOWS_1256,
            "1257" => encoding_rs::WINDOWS_1257,
            "1258" => encoding_rs::WINDOWS_1258,
            _ => return Err(codepage_error()),
        }))
    }

    pub(super) fn decode(self, bytes: &[u8]) -> Result<String> {
        match self {
            Self::Ascii if bytes.is_ascii() => Ok(bytes.iter().map(|b| char::from(*b)).collect()),
            Self::Ascii => Err(codepage_error()),
            Self::Latin1 => Ok(bytes.iter().map(|b| char::from(*b)).collect()),
            Self::Encoded(encoding) => encoding
                .decode_without_bom_handling_and_without_replacement(bytes)
                .map(Cow::into_owned)
                .ok_or_else(codepage_error),
        }
    }
}

fn codepage_error() -> PlenoraIoError {
    err(&PublicMessage::Curated(
        "DBF: codepage sconosciuta o testo non decodificabile esattamente",
    ))
}

pub fn restore_names(
    path: &Path,
    fields: &mut [DbfFieldLayout],
    encoding: TextEncoding,
) -> Result<()> {
    let mut file = File::open(path).map_err(|_| codepage_error())?;
    file.seek(SeekFrom::Start(32))
        .map_err(|_| codepage_error())?;
    let mut seen = BTreeSet::new();
    for field in fields {
        let mut descriptor = [0; 32];
        file.read_exact(&mut descriptor)
            .map_err(|_| codepage_error())?;
        let end = descriptor[..11].iter().position(|b| *b == 0).unwrap_or(11);
        let name = encoding.decode(&descriptor[..end])?;
        if name.is_empty() || !seen.insert(name.to_ascii_uppercase()) {
            return Err(err(&PublicMessage::Curated(
                "DBF: nomi campo vuoti o duplicati",
            )));
        }
        field.name = name;
    }
    Ok(())
}

pub fn reader(
    path: &Path,
) -> std::result::Result<shapefile::dbase::Reader<BufReader<File>>, shapefile::dbase::Error> {
    shapefile::dbase::ReaderBuilder::new()
        // I risultati Character e i nomi vengono sostituiti dai byte originali.
        // La scelta esplicita evita fallback diversi secondo le feature dbase.
        .with_encoding(shapefile::dbase::encoding::UnicodeLossy)
        .with_options(
            shapefile::dbase::ReadingOptions::default()
                .character_trim(shapefile::dbase::TrimOption::End),
        )
        .open(path)
}

/// Forma decimale canonica (segno, cifre significative, esponente). Nessuna
/// conversione numerica delle cifre: funziona anche oltre i128 e con esponenti.
fn decimal_key(text: &str) -> Option<(bool, String, i32)> {
    let negative = text.starts_with('-');
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    let mut parts = unsigned.split(['e', 'E']);
    let mantissa = parts.next()?;
    let exponent = parts
        .next()
        .map_or(Some(0), |part| part.parse::<i32>().ok())?;
    if parts.next().is_some() {
        return None;
    }
    let mut digits = String::new();
    let mut scale = 0_i32;
    let mut dot = false;
    for byte in mantissa.bytes() {
        match byte {
            b'.' if !dot => dot = true,
            b'0'..=b'9' => {
                digits.push(char::from(byte));
                if dot {
                    scale = scale.checked_add(1)?;
                }
            }
            _ => return None,
        }
    }
    if digits.is_empty() {
        return None;
    }
    let mut digits = digits.trim_start_matches('0').to_owned();
    let mut exponent = exponent.checked_sub(scale)?;
    while digits.ends_with('0') {
        digits.pop();
        exponent = exponent.checked_add(1)?;
    }
    if digits.is_empty() {
        exponent = 0;
    }
    Some((negative, digits, exponent))
}

fn numeric(raw: &[u8]) -> Result<Option<f64>> {
    let invalid = || {
        err(&PublicMessage::Curated(
            "DBF: numero non rappresentabile senza perdita decimale",
        ))
    };
    let text = std::str::from_utf8(raw)
        .map_err(|_| invalid())?
        .trim_matches(' ');
    if text.is_empty() || text.bytes().all(|b| b == b'*') {
        return Ok(None);
    }
    let value = text.parse::<f64>().map_err(|_| invalid())?;
    let original = decimal_key(text).ok_or_else(invalid)?;
    if !value.is_finite() || Some(original) != decimal_key(&value.to_string()) {
        return Err(invalid());
    }
    Ok(Some(value))
}

impl DbfExactIntegerRows {
    fn raw_value(&self, field: &DbfFieldLayout) -> Result<Option<FieldValue>> {
        let end = field
            .offset
            .checked_add(field.width)
            .ok_or_else(|| err(&PublicMessage::Curated("overflow offset campo DBF")))?;
        let raw = self
            .buffer
            .get(field.offset..end)
            .ok_or_else(|| err(&PublicMessage::Curated("campo DBF fuori dal record")))?;
        match field.field_type {
            b'C' => {
                if raw.contains(&0) {
                    return Err(err(&PublicMessage::Curated("DBF: NUL in campo Character")));
                }
                let text = self.layout.encoding.decode(raw)?;
                let text = text.trim_end_matches(' ');
                Ok(Some(FieldValue::Character(
                    (!text.is_empty()).then(|| text.to_owned()),
                )))
            }
            b'N' | b'F' if field.exact_integer_slot.is_none() => {
                numeric(raw).map(|number| Some(FieldValue::Numeric(number)))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn validate_raw_fields(&self) -> Result<()> {
        for field in &self.layout.fields {
            self.raw_value(field)?;
        }
        Ok(())
    }

    pub(super) fn restore_fields(&self, record: &mut Record) -> Result<()> {
        // Ricostruire evita sovrascritture durante la rinomina fra nomi
        // provvisori e definitivi. leggi_descrittori_dbf verifica i primi,
        // restore_names verifica anche l'unicita' dei nomi esatti.
        let mut restored = Record::default();
        for field in &self.layout.fields {
            let value = self
                .raw_value(field)?
                .or_else(|| record.get(&field.source_name).cloned())
                .ok_or_else(|| err(&PublicMessage::Curated("DBF: campo dichiarato assente")))?;
            restored.insert(field.name.clone(), value);
        }
        *record = restored;
        Ok(())
    }
}

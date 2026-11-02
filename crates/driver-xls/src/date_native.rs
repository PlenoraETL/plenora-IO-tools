//! Date native: millisecondi Excel con round-trip binary64 esatto; Arrow in microsecondi.
use super::{
    err, ArrayRef, BTreeMap, BTreeSet, BufReader, OperationBudget, PlenoraIoError, PublicMessage,
    Result,
};
use arrow_array::{
    Date32Array, DurationMicrosecondArray, Int64Array, Time64MicrosecondArray,
    TimestampMicrosecondArray,
};
use arrow_schema::{DataType, TimeUnit};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeKind {
    Date,
    Timestamp,
    Duration,
    Time,
}
impl NativeKind {
    pub const fn data_type(self) -> DataType {
        match self {
            Self::Time => DataType::Time64(TimeUnit::Microsecond),
            Self::Date => DataType::Date32,
            Self::Timestamp => DataType::Timestamp(TimeUnit::Microsecond, None),
            Self::Duration => DataType::Duration(TimeUnit::Microsecond),
        }
    }
}

pub fn convert(
    value: calamine::ExcelDateTime,
    epoch: bool,
    date_only: bool,
    time_only: bool,
) -> Result<(NativeKind, i64)> {
    let serial = value.as_f64();
    let limit = if epoch && !value.is_duration() {
        2_957_004.0
    } else {
        2_958_466.0
    };
    if !serial.is_finite() || serial < 0.0 || serial >= limit {
        return Err(err(&PublicMessage::Curated(
            "seriale temporale XLSX negativo o fuori dominio",
        )));
    }
    let pure_time = time_only && serial < 1.0;
    if !pure_time
        && !value.is_duration()
        && !epoch
        && serial < 61.0
        && !(1.0..60.0).contains(&serial)
    {
        return Err(err(&PublicMessage::Curated(
            "seriale XLSX ambiguo: giorno zero o 29/02/1900 inesistente",
        )));
    }
    let milliseconds = excel_milliseconds(serial)?;
    let ticks = milliseconds * 1000;
    if pure_time {
        return Ok((NativeKind::Time, ticks));
    }
    if value.is_duration() {
        return Ok((NativeKind::Duration, ticks));
    }
    let offset = if epoch {
        24_107
    } else if serial < 60.0 {
        25_568
    } else {
        25_569
    };
    if date_only && ticks % 86_400_000_000 == 0 {
        Ok((NativeKind::Date, ticks / 86_400_000_000 - offset))
    } else {
        Ok((NativeKind::Timestamp, ticks - offset * 86_400_000_000))
    }
}

const MILLISECONDS_PER_DAY: u128 = 86_400_000;

// Quoziente arrotondato al piu' vicino, parita' in caso di meta' esatta.
const fn round_half_even(numerator: u128, denominator: u128) -> u128 {
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    quotient
        + (remainder > denominator / 2
            || (remainder * 2 == denominator && !quotient.is_multiple_of(2))) as u128
}

fn excel_milliseconds(serial: f64) -> Result<i64> {
    // Chiamato solo nel dominio non negativo < 2^22 giorni: prodotto < 2^80,
    // shift positivo; anche i subnormali si arrotondano senza shift fuori tipo.
    let bits = serial.to_bits();
    let exponent = (bits >> 52) & 2047;
    let mantissa = (bits & ((1_u64 << 52) - 1)) | if exponent == 0 { 0 } else { 1_u64 << 52 };
    let numerator = u128::from(mantissa) * MILLISECONDS_PER_DAY;
    let shift = 1075 - exponent.max(1);
    let milliseconds = if shift >= 128 {
        0
    } else {
        round_half_even(numerator, 1_u128 << shift)
    };
    let canonical = f64::from_bits(millisecond_serial_bits(milliseconds));
    if canonical.to_bits() != bits
        && !(14..=16).any(|precision| {
            format!("{canonical:.precision$e}")
                .parse::<f64>()
                .is_ok_and(|v| v.to_bits() == bits)
        })
    {
        return Err(err(&PublicMessage::Curated(
            "seriale XLSX non canonico al millisecondo: round-trip binary64 rifiutato",
        )));
    }
    i64::try_from(milliseconds)
        .map_err(|_| err(&PublicMessage::Curated("seriale XLSX fuori dominio")))
}

// Rapporto ms/giorno -> binary64, solo aritmetica intera e round-half-even.
// Nel dominio Excel ms < 2^48: il numeratore normalizzato << 52 sta in u128.
fn millisecond_serial_bits(milliseconds: u128) -> u64 {
    if milliseconds == 0 {
        return 0;
    }
    let mut numerator = milliseconds;
    let mut denominator = MILLISECONDS_PER_DAY;
    let mut exponent = 1023_u64;
    while numerator < denominator {
        numerator *= 2;
        exponent -= 1;
    }
    while numerator >= denominator * 2 {
        denominator *= 2;
        exponent += 1;
    }
    let significand = round_half_even(numerator << 52, denominator);
    // L'addizione propaga anche l'eventuale riporto all'esponente.
    (exponent << 52) + u64::try_from(significand - (1_u128 << 52)).unwrap_or(0)
}

pub fn restore(array: ArrayRef, datatype: &DataType) -> Result<ArrayRef> {
    if !matches!(
        datatype,
        DataType::Date32 | DataType::Timestamp(_, _) | DataType::Duration(_) | DataType::Time64(_)
    ) {
        return Ok(array);
    }
    let values = array
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| err(&PublicMessage::Curated("spool temporale XLSX non valido")))?;
    Ok(match datatype {
        DataType::Date32 => Arc::new(Date32Array::from(
            values
                .iter()
                .map(|v| v.map(i32::try_from).transpose())
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| err(&PublicMessage::Curated("data XLSX fuori dominio")))?,
        )),
        DataType::Time64(_) => Arc::new(Time64MicrosecondArray::from_iter(values.iter())),
        DataType::Timestamp(_, _) => Arc::new(TimestampMicrosecondArray::from_iter(values.iter())),
        _ => Arc::new(DurationMicrosecondArray::from_iter(values.iter())),
    })
}

#[derive(Default)]
pub struct NativeStyles {
    dates: BTreeSet<(u32, u32)>,
    times: BTreeSet<(u32, u32)>,
    pub epoch: bool,
}
impl NativeStyles {
    pub fn date_only(&self, row: u32, col: u32) -> bool {
        self.dates.contains(&(row, col))
    }
    pub fn time_only(&self, row: u32, col: u32) -> bool {
        self.times.contains(&(row, col))
    }
    pub fn temporal(
        &self,
        data: &calamine::Data,
        row: u32,
        col: u32,
    ) -> Option<calamine::ExcelDateTime> {
        match data {
            calamine::Data::DateTime(value) => Some(*value),
            calamine::Data::Float(value)
                if self.date_only(row, col) || self.time_only(row, col) =>
            {
                Some(calamine::ExcelDateTime::new(
                    *value,
                    calamine::ExcelDateTimeType::DateTime,
                    self.epoch,
                ))
            }
            _ => None,
        }
    }
    pub fn read(path: &std::path::Path, sheet: &str, budget: &OperationBudget) -> Result<Self> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?).map_err(|_| xml_error())?;
        let mut result = Self::default();
        let mut relation = String::new();
        scan(&mut zip, "xl/workbook.xml", budget, |tag, attrs| {
            if tag == "workbook/sheets/sheet" && attrs.get("name").is_some_and(|v| v == sheet) {
                relation = attrs.get("r:id").cloned().unwrap_or_default();
            }
            if tag == "workbook/workbookPr" {
                result.epoch = match attrs.get("date1904").map(String::as_str) {
                    Some("1" | "true") => true,
                    None | Some("0" | "false") => false,
                    _ => return Err(xml_error()),
                };
            }
            Ok(())
        })?;
        let mut target = String::new();
        scan(
            &mut zip,
            "xl/_rels/workbook.xml.rels",
            budget,
            |tag, attrs| {
                if tag == "Relationships/Relationship" && attrs.get("Id") == Some(&relation) {
                    target = attrs.get("Target").cloned().unwrap_or_default();
                }
                Ok(())
            },
        )?;
        let mut formats = BTreeMap::new();
        let mut styles = Vec::new();
        if zip.index_for_name("xl/styles.xml").is_none() {
            return Ok(result);
        }
        scan(&mut zip, "xl/styles.xml", budget, |tag, attrs| {
            match tag {
                "styleSheet/numFmts/numFmt" => {
                    formats.insert(
                        attrs.get("numFmtId").cloned().unwrap_or_default(),
                        attrs.get("formatCode").cloned().unwrap_or_default(),
                    );
                }
                "styleSheet/cellXfs/xf" => {
                    let id = attrs.get("numFmtId").map_or("0", String::as_str);
                    styles.push(formats.get(id).map_or_else(
                        || (builtin_date(id), builtin_time(id)),
                        |code| (only_date(code), only_time(code)),
                    ));
                }
                _ => {}
            }
            Ok(())
        })?;
        let part = if target.starts_with('/') {
            target.trim_start_matches('/').to_owned()
        } else {
            format!("xl/{target}")
        };
        scan(&mut zip, &part, budget, |tag, attrs| {
            if tag == "worksheet/sheetData/row/c" {
                let (date, time) = attrs
                    .get("s")
                    .and_then(|v| v.parse::<usize>().ok())
                    .and_then(|i| styles.get(i))
                    .copied()
                    .unwrap_or((false, false));
                if !date && !time {
                    return Ok(());
                }
                let reference = attrs.get("r").ok_or_else(xml_error)?;
                let mut col = 0_u32;
                let mut row = 0_u32;
                for c in reference.bytes() {
                    if c.is_ascii_uppercase() {
                        col = col
                            .checked_mul(26)
                            .and_then(|n| n.checked_add(u32::from(c - b'A') + 1))
                            .ok_or_else(xml_error)?;
                    } else if c.is_ascii_digit() {
                        row = row
                            .checked_mul(10)
                            .and_then(|n| n.checked_add(u32::from(c - b'0')))
                            .ok_or_else(xml_error)?;
                    } else {
                        return Err(xml_error());
                    }
                }
                let position = (
                    row.checked_sub(1).ok_or_else(xml_error)?,
                    col.checked_sub(1).ok_or_else(xml_error)?,
                );
                if date {
                    result.dates.insert(position);
                }
                if time {
                    result.times.insert(position);
                }
            }
            Ok(())
        })?;
        Ok(result)
    }
}

// Le sezioni condizionali possono scegliere formati diversi: Date32 solo
// quando tutte sono prive di token orari. Letterali e escape non sono token.
fn only_date(code: &str) -> bool {
    let mut quoted = false;
    let mut escaped = false;
    let mut bracket = false;
    let mut date = false;
    for c in code.chars().flat_map(char::to_lowercase) {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '"' {
            quoted = !quoted;
            continue;
        }
        if quoted {
            continue;
        }
        if matches!(c, '\\' | '_' | '*') {
            escaped = true;
            continue;
        }
        if c == '[' {
            bracket = true;
            continue;
        }
        if c == ']' {
            bracket = false;
            continue;
        }
        if bracket {
            continue;
        }
        if matches!(c, 'h' | 's' | 'a') {
            return false;
        }
        date |= matches!(c, 'y' | 'd');
    }
    date
}
fn xml_error() -> PlenoraIoError {
    err(&PublicMessage::Curated(
        "metadati temporali XML XLSX non validi",
    ))
}
fn scan<F>(
    zip: &mut zip::ZipArchive<std::fs::File>,
    part: &str,
    budget: &OperationBudget,
    mut visit: F,
) -> Result<()>
where
    F: FnMut(&str, &BTreeMap<String, String>) -> Result<()>,
{
    let entry = zip.by_name(part).map_err(|_| xml_error())?;
    let mut reader = quick_xml::NsReader::from_reader(quick_xml::encoding::DecodingReader::new(
        BufReader::new(entry),
    ));
    let mut buf = Vec::new();
    let mut stack = Vec::new();
    loop {
        budget.context().ensure_active()?;
        buf.clear();
        let event = reader.read_event_into(&mut buf).map_err(|_| xml_error())?;
        let empty = matches!(event, quick_xml::events::Event::Empty(_));
        match event {
            quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let expected_ns = if part == "xl/_rels/workbook.xml.rels" {
                    "http://schemas.openxmlformats.org/package/2006/relationships"
                } else {
                    "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                };
                let valid = matches!(ns, quick_xml::name::ResolveResult::Bound(n) if n.as_ref() == expected_ns);
                stack.push(if valid {
                    local.as_ref().to_owned()
                } else {
                    String::new()
                });
                let mut attrs = BTreeMap::new();
                for a in e.attributes() {
                    let a = a.map_err(|_| xml_error())?;
                    let (ns, local) = reader.resolver().resolve_attribute(a.key);
                    let key = match ns {
                        quick_xml::name::ResolveResult::Unbound => local.as_ref().to_owned(),
                        quick_xml::name::ResolveResult::Bound(n) if n.as_ref() == "http://schemas.openxmlformats.org/officeDocument/2006/relationships" => format!("r:{}", local.as_ref()),
                        _ => continue,
                    };
                    attrs.insert(
                        key,
                        a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|_| xml_error())?
                            .into_owned(),
                    );
                }
                visit(&stack.join("/"), &attrs)?;
                if empty {
                    stack.pop();
                }
            }
            quick_xml::events::Event::End(_) => {
                stack.pop();
            }
            quick_xml::events::Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn builtin_time(id: &str) -> bool {
    matches!(
        id,
        "18" | "19" | "20" | "21" | "32" | "33" | "45" | "47" | "55" | "56"
    )
}

fn only_time(code: &str) -> bool {
    let mut quoted = false;
    let mut escaped = false;
    let mut bracket = false;
    let mut bracket_token = String::new();
    let mut time = false;
    for c in code.chars().flat_map(char::to_lowercase) {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '"' {
            quoted = !quoted;
            continue;
        }
        if quoted {
            continue;
        }
        if matches!(c, '\\' | '_' | '*') {
            escaped = true;
            continue;
        }
        if c == '[' {
            bracket = true;
            bracket_token.clear();
            continue;
        }
        if c == ']' {
            if matches!(bracket_token.as_str(), "h" | "hh" | "m" | "mm" | "s" | "ss") {
                return false;
            }
            bracket = false;
            continue;
        }
        if bracket {
            // Colori e codici locali (es. [$-en-US]) non sono durate.
            bracket_token.push(c);
            continue;
        }
        if matches!(c, 'y' | 'd') {
            return false;
        }
        time |= matches!(c, 'h' | 's');
    }
    time
}

fn builtin_date(id: &str) -> bool {
    matches!(
        id,
        "14" | "15"
            | "16"
            | "17"
            | "27"
            | "28"
            | "29"
            | "30"
            | "31"
            | "34"
            | "35"
            | "36"
            | "50"
            | "51"
            | "52"
            | "53"
            | "54"
            | "57"
            | "58"
    )
}

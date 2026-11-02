//! Protezione dei valori dalle conversioni implicite dell'affinita' SQLite.
use super::{err, BorrowedSqlValue, Field, PublicMessage, Result, DECLARED_TYPE};

/// Gli altri abbinamenti ammessi sono Int64 -> INTEGER/NUMERIC,
/// Float64 -> REAL (zero negativo escluso dal binder), Utf8 -> TEXT e
/// Binary -> BLOB. Solo DATE/DATETIME possono convertire un testo in numero.
pub fn validate_value(field: &Field, value: &BorrowedSqlValue<'_>) -> Result<()> {
    if field
        .metadata()
        .get(DECLARED_TYPE)
        .is_some_and(|t| t.eq_ignore_ascii_case("DATE") || t.eq_ignore_ascii_case("DATETIME"))
    {
        if let BorrowedSqlValue::Text(text) = value {
            if is_sqlite_numeric_literal(text) {
                return Err(err(&PublicMessage::Curated(
                    "testo numerico non conservabile con affinita' DATE/DATETIME GeoPackage",
                )));
            }
        }
    }
    Ok(())
}

/// Grammatica decimale dell'affinita' SQLite, non quella di CAST (prefissi),
/// Rust (Inf/NaN) o dei letterali SQL (0x, underscore). Nessuna conversione
/// floating point: anche overflow e underflow convertono il testo.
fn is_sqlite_numeric_literal(text: &str) -> bool {
    // SQLite termina la scansione numerica al primo NUL, anche se il TEXT
    // contiene altri byte. Anche quel suffisso andrebbe perso nell'INSERT.
    let mut bytes = text
        .split('\0')
        .next()
        .unwrap_or_default()
        .trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000b}' | '\u{000c}'))
        .bytes()
        .peekable();
    if matches!(bytes.peek(), Some(b'+' | b'-')) {
        bytes.next();
    }
    let mut digits = 0;
    while bytes.peek().is_some_and(u8::is_ascii_digit) {
        bytes.next();
        digits += 1;
    }
    if bytes.peek() == Some(&b'.') {
        bytes.next();
        while bytes.peek().is_some_and(u8::is_ascii_digit) {
            bytes.next();
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if matches!(bytes.peek(), Some(b'e' | b'E')) {
        bytes.next();
        if matches!(bytes.peek(), Some(b'+' | b'-')) {
            bytes.next();
        }
        let mut exponent_digits = 0;
        while bytes.peek().is_some_and(u8::is_ascii_digit) {
            bytes.next();
            exponent_digits += 1;
        }
        if exponent_digits == 0 {
            return false;
        }
    }
    bytes.next().is_none()
}

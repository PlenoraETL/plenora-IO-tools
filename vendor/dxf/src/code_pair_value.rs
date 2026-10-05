use std::fmt;
use std::fmt::{Debug, Display, Formatter};

/// Contains the data portion of a `CodePair`.
#[derive(PartialEq)]
#[cfg_attr(feature = "serialize", derive(serde::Serialize, serde::Deserialize))]
pub enum CodePairValue {
    Boolean(i16),
    Integer(i32),
    Long(i64),
    Short(i16),
    Double(f64),
    Str(String),
    Binary(Vec<u8>),
}

impl Clone for CodePairValue {
    fn clone(&self) -> Self {
        match self {
            CodePairValue::Boolean(b) => CodePairValue::Boolean(*b),
            CodePairValue::Integer(i) => CodePairValue::Integer(*i),
            CodePairValue::Long(l) => CodePairValue::Long(*l),
            CodePairValue::Short(s) => CodePairValue::Short(*s),
            CodePairValue::Double(d) => CodePairValue::Double(*d),
            CodePairValue::Str(ref s) => CodePairValue::Str(String::from(s.as_str())),
            CodePairValue::Binary(ref b) => CodePairValue::Binary(b.clone()),
        }
    }
}

impl Debug for CodePairValue {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            CodePairValue::Boolean(s) => write!(f, "{s}"),
            CodePairValue::Integer(i) => write!(f, "{i: >9}"),
            CodePairValue::Long(l) => write!(f, "{l}"),
            CodePairValue::Short(s) => write!(f, "{s: >6}"),
            CodePairValue::Double(d) => write!(f, "{}", format_f64(*d)),
            CodePairValue::Str(ref s) => write!(f, "{s}"),
            CodePairValue::Binary(ref b) => {
                let mut line = String::new();
                for s in b {
                    line.push_str(&format!("{s:02X}"));
                }
                write!(f, "{line}")
            }
        }
    }
}

impl Display for CodePairValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}") // fall back to debug
    }
}

pub(crate) fn escape_control_characters(val: &str) -> String {
    fn needs_escaping(c: char) -> bool {
        let c = c as u32;
        c <= 0x1F || c == 0x5E
    }

    let mut result = String::from("");
    for c in val.chars() {
        if needs_escaping(c) {
            result.push('^');
            match c as u8 {
                0x00 => result.push('@'),
                0x01 => result.push('A'),
                0x02 => result.push('B'),
                0x03 => result.push('C'),
                0x04 => result.push('D'),
                0x05 => result.push('E'),
                0x06 => result.push('F'),
                0x07 => result.push('G'),
                0x08 => result.push('H'),
                0x09 => result.push('I'),
                0x0A => result.push('J'),
                0x0B => result.push('K'),
                0x0C => result.push('L'),
                0x0D => result.push('M'),
                0x0E => result.push('N'),
                0x0F => result.push('O'),
                0x10 => result.push('P'),
                0x11 => result.push('Q'),
                0x12 => result.push('R'),
                0x13 => result.push('S'),
                0x14 => result.push('T'),
                0x15 => result.push('U'),
                0x16 => result.push('V'),
                0x17 => result.push('W'),
                0x18 => result.push('X'),
                0x19 => result.push('Y'),
                0x1A => result.push('Z'),
                0x1B => result.push('['),
                0x1C => result.push('\\'),
                0x1D => result.push(']'),
                0x1E => result.push('^'),
                0x1F => result.push('_'),
                0x5E => result.push(' '),
                _ => panic!("this should never happen"),
            }
        } else {
            result.push(c);
        }
    }

    result
}

pub(crate) fn escape_unicode_to_ascii(val: &str) -> String {
    let mut result = String::from("");

    for c in val.chars() {
        let b = c as u32;
        if b >= 128 {
            result.push_str(&format!("\\U+{b:04X}"));
        } else {
            result.push(c);
        }
    }

    result
}

/// Il carattere che una sequenza `^x` rappresenta, se `x` e' una di quelle
/// che il DXF definisce.
fn carattere_caret(c: char) -> Option<char> {
    let codice = match c {
        '@' => 0x00,
        'A'..='Z' => u32::from(c) - u32::from('A') + 0x01,
        '[' => 0x1B,
        '\\' => 0x1C,
        ']' => 0x1D,
        '^' => 0x1E,
        '_' => 0x1F,
        ' ' => u32::from('^'),
        _ => return None,
    };
    char::from_u32(codice)
}

/// Decodifica un valore di testo del DXF, in un solo passaggio da sinistra a
/// destra: le sequenze `^x` e, se `unicode`, le `\U+XXXX` del DXF ASCII.
///
/// Upstream le decodificava in due passaggi -- prima `\U+`, poi `^` -- e
/// ciascuno perdeva qualcosa: una `\U+` non valida diventava `?`, una barra
/// negli ultimi sei caratteri spariva con cio' che seguiva, un `^` finale
/// spariva, un `^` seguito da un carattere fuori tabella diventava quel
/// carattere troncato a un byte, e un `^` prodotto da `\U+005E` veniva poi
/// letto come inizio di una sequenza. Qui:
///
/// * `\\` e' una barra letterale di MTEXT, e passa intatta: la barra che
///   segue non apre una sequenza;
/// * `\U+` seguita da quattro cifre esadecimali di un carattere valido si
///   decodifica; seguita da altro, o da un surrogato, rende il valore
///   illeggibile (`None`);
/// * ogni altra barra, e ogni `^x` fuori tabella, resta com'e';
/// * con `consenti_coda`, una sequenza aperta alla fine del valore -- `^`, `\`,
///   `\U`, `\U+` con meno di quattro cifre -- e' restituita a parte come
///   coda grezza: e' il frammento di un MTEXT che continua nel gruppo
///   seguente. Senza, `^`, `\` e `\U` finali sono letterali, e una `\U+`
///   incompleta e' un errore.
pub(crate) fn decodifica_testo(
    val: &str,
    unicode: bool,
    consenti_coda: bool,
) -> Option<(String, String)> {
    let c: Vec<char> = val.chars().collect();
    let n = c.len();
    let coda = |da: usize| c[da..].iter().collect::<String>();
    let mut uscita = String::with_capacity(val.len());
    let mut i = 0;
    while i < n {
        match c[i] {
            '^' => {
                if i + 1 == n {
                    if consenti_coda {
                        return Some((uscita, coda(i)));
                    }
                    uscita.push('^');
                    i += 1;
                    continue;
                }
                match carattere_caret(c[i + 1]) {
                    Some(decodificato) => uscita.push(decodificato),
                    None => {
                        uscita.push('^');
                        uscita.push(c[i + 1]);
                    }
                }
                i += 2;
            }
            '\\' if unicode => {
                if i + 1 == n {
                    if consenti_coda {
                        return Some((uscita, coda(i)));
                    }
                    uscita.push('\\');
                    i += 1;
                    continue;
                }
                if c[i + 1] == '\\' {
                    uscita.push('\\');
                    uscita.push('\\');
                    i += 2;
                    continue;
                }
                if c[i + 1] == 'U' {
                    if i + 2 == n {
                        if consenti_coda {
                            return Some((uscita, coda(i)));
                        }
                        uscita.push('\\');
                        uscita.push('U');
                        i += 2;
                        continue;
                    }
                    if c[i + 2] == '+' {
                        let disponibili = (n - (i + 3)).min(4);
                        let cifre = &c[i + 3..i + 3 + disponibili];
                        if !cifre.iter().all(char::is_ascii_hexdigit) {
                            return None;
                        }
                        if disponibili < 4 {
                            return if consenti_coda {
                                Some((uscita, coda(i)))
                            } else {
                                None
                            };
                        }
                        let esadecimale: String = cifre.iter().collect();
                        let codice = u32::from_str_radix(&esadecimale, 16).ok()?;
                        uscita.push(char::from_u32(codice)?);
                        i += 7;
                        continue;
                    }
                }
                uscita.push('\\');
                i += 1;
            }
            altro => {
                uscita.push(altro);
                i += 1;
            }
        }
    }
    Some((uscita, String::new()))
}

/// Il solo `\U+` di un valore intero, senza coda: usato dalle prove.
#[cfg(test)]
pub(crate) fn un_escape_ascii_to_unicode(val: &str) -> Option<String> {
    decodifica_testo(val, true, false).map(|(testo, _)| testo)
}

/// Formats an `f64` value with up to 12 digits of precision, ensuring at least one trailing digit after the decimal.
fn format_f64(val: f64) -> String {
    // format with 12 digits of precision
    let mut val = format!("{val:.12}");

    // trim trailing zeros
    while val.ends_with('0') {
        val.pop();
    }

    // ensure it doesn't end with a decimal
    if val.ends_with('.') {
        val.push('0');
    }

    val
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_control_characters() {
        assert_eq!("a^G^ ^^ b", escape_control_characters("a\u{7}^\u{1E} b"));
    }

    #[test]
    fn test_unicode_escape_1() {
        // values in the middle of a string
        assert_eq!(
            "Rep\\U+00E8re pi\\U+00E8ce",
            escape_unicode_to_ascii("Repère pièce")
        );

        // value is the entire string
        assert_eq!("\\U+4F60\\U+597D", escape_unicode_to_ascii("你好"));
    }

    #[test]
    fn test_unicode_escape_2() {
        assert_eq!(
            "\\U+0410\\U+0430\\U+042F\\U+044F",
            escape_unicode_to_ascii("АаЯя")
        );
    }

    #[test]
    fn test_ascii_unescape() {
        // values in the middle of the string
        assert_eq!(
            "Repère pièce",
            un_escape_ascii_to_unicode("Rep\\U+00E8re pi\\U+00E8ce").unwrap()
        );

        // value is entire string
        assert_eq!(
            "你好",
            un_escape_ascii_to_unicode("\\U+4F60\\U+597D").unwrap()
        );
    }

    #[test]
    fn test_display_boolean() {
        assert_eq!("0", format!("{}", CodePairValue::Boolean(0)));
        assert_eq!("1", format!("{}", CodePairValue::Boolean(1)));
        assert_eq!("2", format!("{}", CodePairValue::Boolean(2)));
    }

    #[test]
    fn test_display_integer() {
        assert_eq!("        0", format!("{}", CodePairValue::Integer(0)));
        assert_eq!("      500", format!("{}", CodePairValue::Integer(500)));
        assert_eq!("     -500", format!("{}", CodePairValue::Integer(-500)));
    }

    #[test]
    fn test_display_long() {
        assert_eq!("0", format!("{}", CodePairValue::Long(0)));
        assert_eq!("500", format!("{}", CodePairValue::Long(500)));
        assert_eq!("-500", format!("{}", CodePairValue::Long(-500)));
    }

    #[test]
    fn test_display_short() {
        assert_eq!("     0", format!("{}", CodePairValue::Short(0)));
        assert_eq!("   500", format!("{}", CodePairValue::Short(500)));
        assert_eq!("  -500", format!("{}", CodePairValue::Short(-500)));
    }

    #[test]
    fn test_display_double() {
        assert_eq!("0.0", format!("{}", CodePairValue::Double(0.0)));
        assert_eq!("1.0", format!("{}", CodePairValue::Double(1.0)));
        assert_eq!("3.5", format!("{}", CodePairValue::Double(3.5)));
        assert_eq!("-3.5", format!("{}", CodePairValue::Double(-3.5)));
        assert_eq!(
            "1000000000000.0",
            format!("{}", CodePairValue::Double(1e12))
        );
    }

    #[test]
    fn test_display_str() {
        assert_eq!("", format!("{}", CodePairValue::Str("".to_string())));
        assert_eq!(
            "some text",
            format!("{}", CodePairValue::Str("some text".to_string()))
        );
    }

    #[test]
    fn test_display_binary() {
        assert_eq!("", format!("{}", CodePairValue::Binary(vec![])));
        assert_eq!("01", format!("{}", CodePairValue::Binary(vec![0x01])));
        assert_eq!(
            "01020304",
            format!("{}", CodePairValue::Binary(vec![0x01, 0x02, 0x03, 0x04]))
        );
    }
}

//! Gli identificatori di primo livello di una definizione CRS e le autorita'
//! dei cataloghi locali (Arrow Geometry Semantics 1.0, GEO-005 e GEO-020).
//!
//! Il bordo non risolve CRS: legge soltanto gli identificatori che la
//! definizione dichiara alla propria radice e ne verifica la buona forma. Un
//! identificatore annidato (l'`AUTHORITY` del `GEOGCS` dentro un `PROJCS`) non
//! soddisfa e non contraddice mai l'identita' del campo.

use crate::crs::CrsDefinitionFormat;

/// La profondita' di annidamento oltre la quale una definizione e' malformata,
/// la stessa del contratto per la geometria.
const PROFONDITA_MASSIMA: usize = 32;

/// Una definizione che non e' ben formata nel formato che dichiara.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformata;

const RADICI_WKT1: [&str; 7] = [
    "GEOGCS",
    "PROJCS",
    "GEOCCS",
    "VERT_CS",
    "COMPD_CS",
    "LOCAL_CS",
    "FITTED_CS",
];
const RADICI_WKT2: [&str; 15] = [
    "GEOGCRS",
    "GEOGRAPHICCRS",
    "GEODCRS",
    "GEODETICCRS",
    "PROJCRS",
    "PROJECTEDCRS",
    "VERTCRS",
    "VERTICALCRS",
    "COMPOUNDCRS",
    "ENGCRS",
    "ENGINEERINGCRS",
    "BOUNDCRS",
    "DERIVEDPROJCRS",
    "TIMECRS",
    "PARAMETRICCRS",
];

/// Le autorita' di un catalogo locale (GEO-020): i loro codici identificano un
/// CRS soltanto dentro l'installazione che li ha scritti. Lista chiusa: un nome
/// si aggiunge solo con una versione del contratto.
const AUTORITA_LOCALI: [&str; 7] = [
    "POSTGIS", "MYSQL", "MARIADB", "MSSQL", "ORACLE", "DB2", "SQLITE",
];

/// `crs_id` nomina l'autorita' di un catalogo locale.
#[must_use]
pub fn autorita_locale(crs_id: &str) -> bool {
    crs_id.split_once(':').is_some_and(|(autorita, _)| {
        AUTORITA_LOCALI
            .iter()
            .any(|locale| autorita.eq_ignore_ascii_case(locale))
    })
}

/// La definizione contraddice `crs_id`: porta almeno un identificatore di
/// primo livello dell'autorita' di `crs_id` e nessuno uguale a `crs_id`.
///
/// # Errors
/// [`Malformata`] se la definizione non e' ben formata nel formato dichiarato,
/// anche quando `crs_id` manca: la buona forma non dipende dall'identita'.
pub fn contraddice(
    crs_id: Option<&str>,
    testo: &str,
    formato: CrsDefinitionFormat,
) -> Result<bool, Malformata> {
    let coppie = identificatori(testo, formato)?;
    let Some((autorita, codice)) = crs_id.and_then(|id| id.split_once(':')) else {
        return Ok(false);
    };
    let stessa: Vec<_> = coppie
        .iter()
        .filter(|(a, _)| a.eq_ignore_ascii_case(autorita))
        .collect();
    Ok(!stessa.is_empty() && !stessa.iter().any(|(_, c)| c == codice))
}

/// Le coppie (autorita', codice) di primo livello della definizione.
///
/// # Errors
/// [`Malformata`] se la definizione non e' ben formata nel formato dichiarato.
pub fn identificatori(
    testo: &str,
    formato: CrsDefinitionFormat,
) -> Result<Vec<(String, String)>, Malformata> {
    match formato {
        CrsDefinitionFormat::Projjson => identificatori_projjson(testo),
        CrsDefinitionFormat::Wkt => {
            identificatori_wkt(&radice_wkt(testo, &RADICI_WKT1)?, "AUTHORITY")
        }
        CrsDefinitionFormat::Wkt2 => identificatori_wkt(&radice_wkt(testo, &RADICI_WKT2)?, "ID"),
    }
}

fn identificatori_projjson(testo: &str) -> Result<Vec<(String, String)>, Malformata> {
    let radice: serde_json::Value = crate::json::from_str(testo).map_err(|_| Malformata)?;
    let radice = radice.as_object().ok_or(Malformata)?;
    let trovati: Vec<&serde_json::Value> = match (radice.get("id"), radice.get("ids")) {
        (Some(_), Some(_)) => return Err(Malformata),
        (Some(id), None) => vec![id],
        (None, Some(ids)) => ids.as_array().ok_or(Malformata)?.iter().collect(),
        (None, None) => Vec::new(),
    };
    trovati
        .into_iter()
        .map(|voce| {
            let voce = voce.as_object().ok_or(Malformata)?;
            let autorita = voce
                .get("authority")
                .and_then(serde_json::Value::as_str)
                .ok_or(Malformata)?;
            let codice = match voce.get("code").ok_or(Malformata)? {
                serde_json::Value::String(testo) => testo.clone(),
                serde_json::Value::Number(numero) if numero.is_i64() || numero.is_u64() => {
                    numero.to_string()
                }
                _ => return Err(Malformata),
            };
            Ok((autorita.to_owned(), codice))
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum Token<'a> {
    Apre(u8),
    Chiude(u8),
    Virgola,
    Stringa(String),
    Numero(&'a str),
    Parola(&'a str),
}

fn token_wkt(testo: &str) -> Result<Vec<Token<'_>>, Malformata> {
    let byte = testo.as_bytes();
    let mut token = Vec::new();
    let mut i = 0;
    while i < byte.len() {
        let c = testo[i..].chars().next().ok_or(Malformata)?;
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        match byte[i] {
            b'[' | b'(' => {
                token.push(Token::Apre(byte[i]));
                i += 1;
            }
            b']' | b')' => {
                token.push(Token::Chiude(byte[i]));
                i += 1;
            }
            b',' => {
                token.push(Token::Virgola);
                i += 1;
            }
            b'"' => {
                let mut valore = String::new();
                let mut j = i + 1;
                loop {
                    match byte.get(j) {
                        None => return Err(Malformata),
                        Some(b'"') if byte.get(j + 1) == Some(&b'"') => {
                            valore.push('"');
                            j += 2;
                        }
                        Some(b'"') => break,
                        Some(_) => {
                            let carattere = testo[j..].chars().next().ok_or(Malformata)?;
                            valore.push(carattere);
                            j += carattere.len_utf8();
                        }
                    }
                }
                token.push(Token::Stringa(valore));
                i = j + 1;
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let fine = fine_numero(byte, i).ok_or(Malformata)?;
                token.push(Token::Numero(&testo[i..fine]));
                i = fine;
            }
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let mut j = i + 1;
                while byte
                    .get(j)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                {
                    j += 1;
                }
                token.push(Token::Parola(&testo[i..j]));
                i = j;
            }
            _ => return Err(Malformata),
        }
    }
    Ok(token)
}

/// La fine di un numero `[+-]?(cifre[.cifre*]|.cifre)([eE][+-]?cifre)?` che
/// comincia in `inizio`, oppure `None` se li' non comincia un numero.
fn fine_numero(byte: &[u8], inizio: usize) -> Option<usize> {
    let cifre = |da: usize| {
        byte.get(da..).map_or(0, |resto| {
            resto.iter().take_while(|b| b.is_ascii_digit()).count()
        })
    };
    let mut i = inizio;
    if matches!(byte.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let intere = cifre(i);
    i += intere;
    if intere > 0 {
        if byte.get(i) == Some(&b'.') {
            i += 1;
            i += cifre(i);
        }
    } else {
        if byte.get(i) != Some(&b'.') {
            return None;
        }
        let decimali = cifre(i + 1);
        if decimali == 0 {
            return None;
        }
        i += 1 + decimali;
    }
    if matches!(byte.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(byte.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let esponente = cifre(j);
        if esponente > 0 {
            i = j + esponente;
        }
    }
    Some(i)
}

#[derive(Debug)]
struct Nodo {
    parola: String,
    voci: Vec<Voce>,
}

#[derive(Debug)]
enum Voce {
    Nodo(Nodo),
    Stringa(String),
    Numero(String),
    Parola,
}

fn radice_wkt(testo: &str, ammesse: &[&str]) -> Result<Nodo, Malformata> {
    let token = token_wkt(testo)?;
    let mut posizione = 0;
    let radice = nodo(&token, &mut posizione, 0)?;
    if posizione != token.len()
        || !ammesse
            .iter()
            .any(|r| radice.parola.eq_ignore_ascii_case(r))
    {
        return Err(Malformata);
    }
    Ok(radice)
}

fn nodo(token: &[Token<'_>], posizione: &mut usize, profondita: usize) -> Result<Nodo, Malformata> {
    if profondita > PROFONDITA_MASSIMA {
        return Err(Malformata);
    }
    let (Some(Token::Parola(parola)), Some(Token::Apre(apertura))) =
        (token.get(*posizione), token.get(*posizione + 1))
    else {
        return Err(Malformata);
    };
    let chiusura = if *apertura == b'[' { b']' } else { b')' };
    *posizione += 2;
    let mut voci = Vec::new();
    loop {
        match (token.get(*posizione), token.get(*posizione + 1)) {
            (Some(Token::Parola(_)), Some(Token::Apre(_))) => {
                voci.push(Voce::Nodo(nodo(token, posizione, profondita + 1)?));
            }
            (Some(Token::Stringa(valore)), _) => {
                voci.push(Voce::Stringa(valore.clone()));
                *posizione += 1;
            }
            (Some(Token::Numero(valore)), _) => {
                voci.push(Voce::Numero((*valore).to_owned()));
                *posizione += 1;
            }
            (Some(Token::Parola(_)), _) => {
                voci.push(Voce::Parola);
                *posizione += 1;
            }
            _ => return Err(Malformata),
        }
        match token.get(*posizione) {
            Some(Token::Chiude(c)) if *c == chiusura => {
                *posizione += 1;
                return Ok(Nodo {
                    parola: (*parola).to_owned(),
                    voci,
                });
            }
            Some(Token::Virgola) => {
                *posizione += 1;
            }
            _ => return Err(Malformata),
        }
    }
}

fn identificatori_wkt(radice: &Nodo, parola: &str) -> Result<Vec<(String, String)>, Malformata> {
    let mut coppie = Vec::new();
    for voce in &radice.voci {
        let Voce::Nodo(figlio) = voce else { continue };
        if !figlio.parola.eq_ignore_ascii_case(parola) {
            continue;
        }
        let Some(Voce::Stringa(autorita)) = figlio.voci.first() else {
            return Err(Malformata);
        };
        let codice = match figlio.voci.get(1) {
            Some(Voce::Stringa(codice)) => codice.clone(),
            Some(Voce::Numero(numero)) => codice_numerico(numero)?,
            _ => return Err(Malformata),
        };
        coppie.push((autorita.clone(), codice));
    }
    Ok(coppie)
}

/// Un codice numerico scritto come intero decimale (`+0043` vale `43`).
fn codice_numerico(numero: &str) -> Result<String, Malformata> {
    let cifre = numero.strip_prefix('+').unwrap_or(numero);
    if cifre.is_empty() || !cifre.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Malformata);
    }
    let senza_zeri = cifre.trim_start_matches('0');
    Ok(if senza_zeri.is_empty() {
        "0".to_owned()
    } else {
        senza_zeri.to_owned()
    })
}

#[cfg(test)]
mod tests;

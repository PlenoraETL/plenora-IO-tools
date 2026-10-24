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

mod projjson;

/// Un'autorita' o un codice letti da una definizione.
///
/// Un PROJJSON puo' portare una stringa con un surrogato isolato (`"\ud800"`):
/// e' JSON ben formato per l'oracolo (`json.loads` la conserva), e nessuna
/// stringa Rust la rappresenta. Non si sostituisce con un carattere qualunque:
/// un testo cosi' non e' uguale a nessun `crs_id`, che e' sempre Unicode
/// valido, e una sostituzione potrebbe renderlo uguale a uno.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestoDefinizione {
    Unicode(String),
    SurrogatiIsolati,
}

impl TestoDefinizione {
    /// Uguale, carattere per carattere, al testo dato.
    #[must_use]
    pub fn uguale_a(&self, altro: &str) -> bool {
        matches!(self, Self::Unicode(testo) if testo == altro)
    }

    /// La stessa autorita' di `altro` come la confronta l'oracolo:
    /// `pair[0].upper() == authority.upper()`. La mappatura e' quella completa
    /// di Unicode, con i casi speciali (la s lunga U+017F diventa `S`, la
    /// legatura U+FB01 diventa `FI`), che Rust e Python applicano entrambi; un
    /// carattere introdotto in una versione di Unicode che l'uno conosce e
    /// l'altro no puo' divergere, ed e' un limite dichiarato.
    #[must_use]
    pub fn stessa_autorita(&self, altro: &str) -> bool {
        matches!(self, Self::Unicode(testo) if testo.to_uppercase() == altro.to_uppercase())
    }
}

/// Uno spazio per l'oracolo: `str.isspace()` di Python, che e' anche il `\s`
/// delle sue espressioni regolari su `str` (la grammatica di `crs_id`, lo
/// spoglio del WKT).
///
/// E' il `White_Space` di Unicode -- `char::is_whitespace` -- piu' i quattro
/// separatori U+001C..U+001F, che Python conta fra gli spazi (classe
/// bidirezionale `B` o `S`) e Unicode no. L'elenco e' chiuso e scritto per
/// esteso, invece di dipendere dalle tabelle Unicode di una versione di Rust:
/// e' quello di Python 3.11 (Unicode 14), verificato su tutto lo spazio dei
/// punti di codice.
#[must_use]
pub const fn spazio_python(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | '\u{1c}'..='\u{20}'
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

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
    // `authority.upper() in LOCAL_AUTHORITIES`, come l'oracolo: la mappatura
    // completa, non solo ASCII.
    crs_id.split_once(':').is_some_and(|(autorita, _)| {
        let maiuscola = autorita.to_uppercase();
        AUTORITA_LOCALI.iter().any(|locale| maiuscola == *locale)
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
        .filter(|(a, _)| a.stessa_autorita(autorita))
        .collect();
    Ok(!stessa.is_empty() && !stessa.iter().any(|(_, c)| c.uguale_a(codice)))
}

/// Le coppie (autorita', codice) di primo livello della definizione.
///
/// # Errors
/// [`Malformata`] se la definizione non e' ben formata nel formato dichiarato.
pub fn identificatori(
    testo: &str,
    formato: CrsDefinitionFormat,
) -> Result<Vec<(TestoDefinizione, TestoDefinizione)>, Malformata> {
    let wkt = |coppie: Vec<(String, String)>| {
        coppie
            .into_iter()
            .map(|(a, c)| (TestoDefinizione::Unicode(a), TestoDefinizione::Unicode(c)))
            .collect()
    };
    match formato {
        CrsDefinitionFormat::Projjson => projjson::identificatori(testo),
        CrsDefinitionFormat::Wkt => Ok(wkt(identificatori_wkt(
            &radice_wkt(testo, &RADICI_WKT1)?,
            "AUTHORITY",
        )?)),
        CrsDefinitionFormat::Wkt2 => Ok(wkt(identificatori_wkt(
            &radice_wkt(testo, &RADICI_WKT2)?,
            "ID",
        )?)),
    }
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
        // Lo spazio dell'oracolo (`\s*` e `str.strip()`), non `White_Space`:
        // anche U+001C..U+001F separano i token.
        if spazio_python(c) {
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
    Parola(String),
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
            (Some(Token::Parola(valore)), _) => {
                voci.push(Voce::Parola((*valore).to_owned()));
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

/// Che cosa la definizione dice degli assi delle prime due ordinate, letti
/// alla radice (GEO-001).
///
/// Non dice in che **ordine** i valori le memorizzano: l'ordine con cui la
/// definizione elenca gli assi e' quello del registro (EPSG:4326 elenca la
/// latitudine per prima), e GEO-002 vieta di ricavarne la dichiarazione. Dice
/// soltanto se gli assi sono una coppia est/nord, l'unica cosa che serve a un
/// formato che memorizza per prima l'ascissa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssiDichiarati {
    /// Un asse verso est e uno verso nord, in qualunque ordine. Anche il WKT 1
    /// di un `GEOGCS` o `PROJCS` senza `AXIS`, che per la specifica OGC
    /// 01-009 vale est poi nord, fuori dalle proiezioni polari.
    EstNord,
    /// Assi che non sono una coppia est/nord: lungo i meridiani di una
    /// proiezione polare (EPSG:3031), verso ovest o sud, geocentrici.
    Altri,
    /// La definizione non dice gli assi della radice: un CRS composto o
    /// vincolato, un WKT 2 o un PROJJSON senza sistema di coordinate.
    Ignoti,
}

const RADICI_CON_ASSI_WKT2: [&str; 6] = [
    "GEOGCRS",
    "GEOGRAPHICCRS",
    "GEODCRS",
    "GEODETICCRS",
    "PROJCRS",
    "PROJECTEDCRS",
];

/// Gli assi della radice della definizione.
///
/// # Errors
/// [`Malformata`] se la definizione non e' ben formata nel formato dichiarato.
pub fn assi_dichiarati(
    testo: &str,
    formato: CrsDefinitionFormat,
) -> Result<AssiDichiarati, Malformata> {
    let (ammesse, con_assi, wkt1): (&[&str], &[&str], bool) = match formato {
        CrsDefinitionFormat::Projjson => return assi_projjson(testo),
        CrsDefinitionFormat::Wkt => (&RADICI_WKT1, &["GEOGCS", "PROJCS"], true),
        CrsDefinitionFormat::Wkt2 => (&RADICI_WKT2, &RADICI_CON_ASSI_WKT2, false),
    };
    let radice = radice_wkt(testo, ammesse)?;
    if !con_assi
        .iter()
        .any(|r| radice.parola.eq_ignore_ascii_case(r))
    {
        return Ok(AssiDichiarati::Ignoti);
    }
    // Senza `AXIS` il WKT 1 vale est poi nord (OGC 01-009), ma il default non
    // descrive una proiezione polare; il WKT 2 non ha default.
    Ok(assi_wkt(&radice).unwrap_or_else(|| {
        if !wkt1 {
            AssiDichiarati::Ignoti
        } else if proiezione_polare_wkt1(&radice) {
            AssiDichiarati::Altri
        } else {
            AssiDichiarati::EstNord
        }
    }))
}

/// Le prime due coppie (direzione, legato a un meridiano) formano una coppia
/// est/nord? `None` se non ci sono assi; un asse solo non e' una coppia.
fn coppia_est_nord<'a>(assi: impl Iterator<Item = (&'a str, bool)>) -> Option<AssiDichiarati> {
    let coppie: Vec<(&str, bool)> = assi.take(2).collect();
    let [(prima, meridiano_1), (seconda, meridiano_2)] = coppie.as_slice() else {
        return (!coppie.is_empty()).then_some(AssiDichiarati::Altri);
    };
    let est_nord =
        |a: &str, b: &str| a.eq_ignore_ascii_case("east") && b.eq_ignore_ascii_case("north");
    Some(
        if !meridiano_1 && !meridiano_2 && (est_nord(prima, seconda) || est_nord(seconda, prima)) {
            AssiDichiarati::EstNord
        } else {
            AssiDichiarati::Altri
        },
    )
}

/// Gli `AXIS` figli diretti della radice; `None` se non ce ne sono.
fn assi_wkt(radice: &Nodo) -> Option<AssiDichiarati> {
    let assi = radice.voci.iter().filter_map(|voce| match voce {
        Voce::Nodo(figlio) if figlio.parola.eq_ignore_ascii_case("AXIS") => {
            let direzione = match figlio.voci.get(1) {
                Some(Voce::Parola(direzione)) => direzione.as_str(),
                // Un asse senza direzione leggibile non e' est ne' nord.
                _ => "",
            };
            let meridiano = figlio
                .voci
                .iter()
                .any(|v| matches!(v, Voce::Nodo(n) if n.parola.eq_ignore_ascii_case("MERIDIAN")));
            Some((direzione, meridiano))
        }
        _ => None,
    });
    coppia_est_nord(assi)
}

/// Una proiezione WKT 1 polare per nome (`Polar_Stereographic`,
/// `Stereographic_South_Pole`) o per `latitude_of_origin`/`latitude_of_center`
/// di +-90 gradi. E' un indizio, non una risoluzione: una polare che non lo
/// dichiara resta est/nord, come il default del formato. Limite dichiarato.
fn proiezione_polare_wkt1(radice: &Nodo) -> bool {
    radice.voci.iter().any(|voce| {
        let Voce::Nodo(figlio) = voce else {
            return false;
        };
        let nome = match figlio.voci.first() {
            Some(Voce::Stringa(nome)) => nome.to_ascii_lowercase(),
            _ => return false,
        };
        if figlio.parola.eq_ignore_ascii_case("PROJECTION") {
            return nome.contains("polar") || nome.contains("pole");
        }
        figlio.parola.eq_ignore_ascii_case("PARAMETER")
            && (nome == "latitude_of_origin" || nome == "latitude_of_center")
            && matches!(
                figlio.voci.get(1),
                Some(Voce::Numero(valore)) if novanta(valore)
            )
    })
}

/// Il numero vale esattamente +-90 in decimale (`90`, `-90.000`), confrontato
/// sul testo; una forma esponenziale non e' riconosciuta.
fn novanta(valore: &str) -> bool {
    let senza_segno = valore.strip_prefix(['+', '-']).unwrap_or(valore);
    let (intera, frazione) = senza_segno.split_once('.').unwrap_or((senza_segno, ""));
    intera.trim_start_matches('0') == "90" && frazione.bytes().all(|b| b == b'0')
}

// Limite dichiarato (docs/PRODUCT.md, «CRS: PROJJSON»): qui si legge ancora con
// `serde_json`, che rifiuta un numero oltre `f64`, un surrogato isolato e piu' di
// 128 livelli. Il chiamante tratta il rifiuto come assi ignoti, quindi `unknown`:
// un'informazione in meno, mai un ordine sbagliato.
fn assi_projjson(testo: &str) -> Result<AssiDichiarati, Malformata> {
    let radice: serde_json::Value = crate::json::from_str(testo).map_err(|_| Malformata)?;
    let radice = radice.as_object().ok_or(Malformata)?;
    let tipo = radice.get("type").and_then(serde_json::Value::as_str);
    if !matches!(tipo, Some("GeographicCRS" | "GeodeticCRS" | "ProjectedCRS")) {
        return Ok(AssiDichiarati::Ignoti);
    }
    let Some(assi) = radice
        .get("coordinate_system")
        .and_then(|cs| cs.get("axis"))
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(AssiDichiarati::Ignoti);
    };
    let assi = assi.iter().map(|asse| {
        (
            asse.get("direction")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
            asse.get("meridian").is_some(),
        )
    });
    Ok(coppia_est_nord(assi).unwrap_or(AssiDichiarati::Ignoti))
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

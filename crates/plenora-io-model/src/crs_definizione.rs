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

/// Il tetto sui byte di **ogni** definizione CRS che entra nel lettore stretto.
///
/// Vale da qualunque sorgente: il `.prj` di uno Shapefile, il `GEODATA` di un
/// DXF, la riga di `gpkg_spatial_ref_sys`, il `crs` di `GeoParquet`, il
/// metadato di un campo Arrow IPC, la definizione di `FileGDB`.
///
/// Una definizione reale e' di qualche KiB; il lettore stretto tiene in memoria
/// un token per ogni elemento, qualche decina di byte per byte d'ingresso nel
/// caso peggiore (`,,,,`), quindi un MiB costa qualche decina di MB.
/// Fino al 24/10 il `.prj` si leggeva intero, senza tetto, e le definizioni di
/// un metadato erano limitate solo dai byte del metadato: una riga di
/// `gpkg_spatial_ref_sys` di 8 MiB costava 700 MB di picco, una di 32 MiB
/// esauriva la scadenza. Ora la regola e' una: oltre il tetto e'
/// `limit_exceeded` ([`verifica_dimensione_definizione`], chiamata da
/// [`crate::crs::crs_kind_for`], da [`crate::crs::verifica_crs_emesso`] e dalla
/// lettura dei metadati Arrow), e il lettore stretto non apre un testo piu'
/// lungo (lo tratta come malformato, una difesa che non decide da sola).
pub const MAX_BYTE_DEFINIZIONE_CRS: usize = 1 << 20;

/// La definizione sta nel tetto [`MAX_BYTE_DEFINIZIONE_CRS`].
///
/// # Errors
///
/// `limit_exceeded`, con il driver se dato, se lo supera.
pub fn verifica_dimensione_definizione(
    testo: &str,
    driver: Option<&'static str>,
) -> crate::Result<()> {
    if testo.len() <= MAX_BYTE_DEFINIZIONE_CRS {
        return Ok(());
    }
    let mut errore = crate::PlenoraIoError::limite_redatto(&crate::PublicMessage::CuratedWith(
        "la definizione del CRS supera il tetto, byte",
        crate::NumeroStrutturale::Limite(MAX_BYTE_DEFINIZIONE_CRS as u64),
    ));
    errore.driver = driver.map(str::to_owned);
    Err(errore)
}

/// Il testo senza un BOM UTF-8 iniziale (uno solo).
///
/// Il BOM non fa parte del WKT ne' del JSON: un `.prj` o una riga di
/// `gpkg_spatial_ref_sys` scritti da un editor che lo antepone si leggevano
/// malformati. Lo tolgono i driver all'ingresso, cosi' la definizione emessa
/// e' quella senza BOM, la stessa che il lettore stretto legge; un secondo BOM
/// resta, e la definizione resta malformata.
#[must_use]
pub fn senza_bom(mut testo: String) -> String {
    if testo.starts_with('\u{feff}') {
        testo.replace_range(..'\u{feff}'.len_utf8(), "");
    }
    testo
}

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

    /// La stessa autorita' di `altro`, ASCII case-insensitive come vuole
    /// GEO-005: una lettera non ASCII resta se stessa (`EPſG` non e' `EPSG`).
    /// L'oracolo al pin usa le maiuscole Unicode e qui diverge; la correzione
    /// dell'oracolo e' plenora-contracts #53.
    #[must_use]
    pub fn stessa_autorita(&self, altro: &str) -> bool {
        matches!(self, Self::Unicode(testo) if testo.eq_ignore_ascii_case(altro))
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
    // ASCII case-insensitive (GEO-020): `poſtgis` non e' un catalogo locale.
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

/// La grammatica WKT in cui il testo e' ben formato: WKT 2, poi WKT 1. Le
/// radici delle due grammatiche sono disgiunte, quindi l'ordine non sceglie
/// fra due letture valide. `None` se il testo non e' ben formato in nessuna.
pub(crate) fn grammatica_wkt(testo: &str) -> Option<CrsDefinitionFormat> {
    if radice_wkt(testo, &RADICI_WKT2).is_ok() {
        Some(CrsDefinitionFormat::Wkt2)
    } else if radice_wkt(testo, &RADICI_WKT1).is_ok() {
        Some(CrsDefinitionFormat::Wkt)
    } else {
        None
    }
}

/// Il testo **comincia** come un WKT: a meno degli spazi, una parola seguita
/// da `[` o `(`.
///
/// Sono le stesse due condizioni con cui il lettore stretto apre
/// la radice, quindi un testo che non le soddisfa non e' mai un WKT ben formato,
/// e uno che le soddisfa e' un WKT -- ben formato o no. Serve a chi riceve
/// definizioni di piu' forme (il `GEODATA` di un DXF: WKT, XML, `EPSG:<codice>`)
/// per sapere quali vanno alla regola di [`identificatore_wkt`].
#[must_use]
pub fn sembra_wkt(testo: &str) -> bool {
    let resto = testo.trim_start_matches(spazio_python);
    let inizio_parola = resto
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_');
    let dopo_parola = resto.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
    inizio_parola
        && dopo_parola
            .trim_start_matches(spazio_python)
            .starts_with(['[', '('])
}

/// Perche' una definizione WKT non da' un identificatore e la lettura si
/// ferma. Ogni variante e' un errore `crs`, con o senza `--assume-crs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RifiutoIdentita {
    /// Il testo non e' un WKT ben formato, ne' 1 ne' 2, oppure un nodo
    /// identificatore della radice (`AUTHORITY` o `ID`) e' malformato: che cosa
    /// la radice dichiari non e' decidibile.
    Malformata,
    /// La radice dichiara piu' identificatori diversi (GEO-005).
    Ambigua,
    /// `--assume-crs` su un `BOUNDCRS`, che lega un CRS sorgente a uno di
    /// destinazione e non ha un'identita' propria da nominare.
    Legata,
    /// `--assume-crs` diverso da cio' che la definizione dichiara, o da uno dei
    /// due nomi di WGS 84 sulla forma ESRI riconosciuta.
    Contraddetta,
}

impl RifiutoIdentita {
    /// L'errore `crs` del driver, con un messaggio curato che non riporta ne'
    /// la definizione ne' gli identificatori.
    #[must_use]
    pub fn errore(self, driver: &'static str) -> crate::PlenoraIoError {
        let messaggio = match self {
            Self::Malformata => {
                "definizione WKT del CRS malformata: l'identificatore della radice non e' decidibile"
            }
            Self::Ambigua => "la radice della definizione WKT del CRS dichiara identificatori discordi",
            Self::Legata => {
                "--assume-crs non nomina un BOUNDCRS: la definizione lega due CRS e non ne ha uno proprio"
            }
            Self::Contraddetta => "--assume-crs contraddice il CRS della definizione WKT",
        };
        let mut errore =
            crate::PlenoraIoError::crs_redatto(&crate::PublicMessage::Curated(messaggio));
        errore.driver = Some(driver.to_owned());
        errore
    }
}

/// Che cosa la radice di un WKT ben formato dice della propria identita'.
#[derive(Debug, PartialEq, Eq)]
enum Identita {
    /// `BOUNDCRS`.
    Legata,
    /// Una sola dichiarazione (autorita', codice), nella grafia del file.
    Dichiarata(String, String),
    /// Piu' dichiarazioni diverse.
    Ambigua,
    /// Nessuna dichiarazione, e la forma ESRI di WGS 84.
    Wgs84Esri,
    /// Nessuna dichiarazione, e nessuna forma riconosciuta.
    Anonima,
}

/// L'identificatore che i driver danno a una definizione WKT, con o senza
/// `--assume-crs`: la **regola unica** dello Shapefile (`.prj`) e del DXF
/// (`GEODATA` in WKT).
///
/// `Ok(None)` e' un CRS dichiarato e non risolto. Fino al 24/10 ciascun driver decideva con piu' letture parziali del testo
/// (una per la radice, una per l'altra parola chiave, una per la forma ESRI,
/// una per il `BOUNDCRS`), e ogni rilettura ne trovava una che diceva una cosa
/// diversa dalle altre: un'etichetta sbagliata senza errore. Qui la lettura e'
/// una, sull'albero del lettore stretto, e decide cosi':
///
/// 1. il testo si legge con la grammatica WKT 1 o WKT 2 in cui e' ben formato;
///    malformato in entrambe e' [`RifiutoIdentita::Malformata`];
/// 2. le **dichiarazioni** sono gli identificatori della radice con la parola
///    della grammatica (`AUTHORITY` in WKT 1, `ID` in WKT 2) **piu'** quelli con
///    la parola dell'altra: per GEO-005 questi ultimi non sono identificatori,
///    ma chi legge il file li legge come tali. Un nodo malformato dell'una o
///    dell'altra parola rende la radice indecidibile: `Malformata`;
/// 3. una radice `BOUNDCRS` non si risolve mai: senza opzione resta non
///    risolta, con l'opzione e' [`RifiutoIdentita::Legata`];
/// 4. due dichiarazioni diverse (autorita' ASCII case-insensitive, codice come
///    testo) sono [`RifiutoIdentita::Ambigua`], con o senza opzione;
/// 5. una dichiarazione: senza opzione si risolve solo un `EPSG` con codice
///    intero canonico, come `EPSG:<codice>` (le altre autorita', `OGC:CRS84`
///    compreso, restano non risolte); con l'opzione, l'opzione deve essere
///    uguale alla dichiarazione, e l'identificatore ha la grafia del file;
/// 6. nessuna dichiarazione: la forma ESRI **intera** di WGS 84
///    (`wgs84_esri`: nomi, ellissoide, meridiano e unita' canonici) e'
///    `EPSG:4326` senza opzione, e con l'opzione ammette solo `EPSG:4326` o
///    `OGC:CRS84`; ogni altra definizione resta non risolta senza opzione, e
///    con l'opzione l'opzione la nomina. Il driver poi confronta il genere
///    ([`crate::crs::crs_kind_for`]): un identificatore che il bordo sa
///    geografico su una radice proiettata, o `EPSG:3857` su una geografica, e'
///    `crs`. Resta il rischio residuo dichiarato: il bordo non e' un resolver,
///    e non sa dire se un `PROJCS` anonimo e' davvero il proiettato che
///    l'opzione nomina (`EPSG:3003` su un UTM).
///
/// # Errors
/// [`RifiutoIdentita`], nei casi elencati.
pub fn identificatore_wkt(
    testo: &str,
    assunto: Option<&str>,
) -> Result<Option<String>, RifiutoIdentita> {
    let identita = identita_wkt(testo).map_err(|Malformata| RifiutoIdentita::Malformata)?;
    let nomina = |assunto: &str, autorita: &str, codice: &str| {
        assunto
            .split_once(':')
            .is_some_and(|(a, c)| a.eq_ignore_ascii_case(autorita) && c == codice)
    };
    match (identita, assunto) {
        (Identita::Ambigua, _) => Err(RifiutoIdentita::Ambigua),
        (Identita::Legata | Identita::Anonima, None) => Ok(None),
        (Identita::Legata, Some(_)) => Err(RifiutoIdentita::Legata),
        (Identita::Dichiarata(autorita, codice), None) => Ok(autorita
            .eq_ignore_ascii_case("EPSG")
            .then(|| crate::crs::codice_intero_canonico(&codice))
            .flatten()
            .and_then(|codice| u32::try_from(codice).ok())
            .map(|codice| format!("EPSG:{codice}"))),
        (Identita::Dichiarata(autorita, codice), Some(assunto)) => {
            if nomina(assunto, &autorita, &codice) {
                Ok(Some(format!("{autorita}:{codice}")))
            } else {
                Err(RifiutoIdentita::Contraddetta)
            }
        }
        (Identita::Wgs84Esri, None) => Ok(Some("EPSG:4326".to_owned())),
        (Identita::Wgs84Esri, Some(assunto)) => {
            if nomina(assunto, "EPSG", "4326") || nomina(assunto, "OGC", "CRS84") {
                Ok(Some(assunto.to_owned()))
            } else {
                Err(RifiutoIdentita::Contraddetta)
            }
        }
        (Identita::Anonima, Some(assunto)) => Ok(Some(assunto.to_owned())),
    }
}

fn identita_wkt(testo: &str) -> Result<Identita, Malformata> {
    let (radice, propria, altra) = match radice_wkt(testo, &RADICI_WKT1) {
        Ok(radice) => (radice, "AUTHORITY", "ID"),
        Err(Malformata) => (radice_wkt(testo, &RADICI_WKT2)?, "ID", "AUTHORITY"),
    };
    // Ogni nodo dell'una e dell'altra parola si legge (uno malformato rende la
    // radice indecidibile), e ciascuno si confronta solo con il primo: la
    // prima dichiarazione diversa basta a dire `Ambigua`. Fino al 24/10 ogni
    // nodo si cercava fra tutti quelli gia' visti, quadratico nel numero.
    let mut dichiarate = identificatori_wkt(&radice, propria)?
        .into_iter()
        .chain(identificatori_wkt(&radice, altra)?);
    if radice.parola.eq_ignore_ascii_case("BOUNDCRS") {
        return Ok(Identita::Legata);
    }
    let Some((autorita, codice)) = dichiarate.next() else {
        return Ok(if wgs84_esri(&radice) {
            Identita::Wgs84Esri
        } else {
            Identita::Anonima
        });
    };
    let diversa = dichiarate.any(|(a, c)| !(a.eq_ignore_ascii_case(&autorita) && c == codice));
    Ok(if diversa {
        Identita::Ambigua
    } else {
        Identita::Dichiarata(autorita, codice)
    })
}

/// La forma ESRI WKT 1 di WGS 84 senza autorita', **intera**: quella che il
/// writer Shapefile emette e quella di `ArcGIS` e GDAL (`GCS_WGS_1984`,
/// `D_WGS_1984`).
///
/// La radice e' `GEOGCS[nome, DATUM, PRIMEM, UNIT]` e nient'altro; il `DATUM`
/// e' `[nome, SPHEROID]`; lo `SPHEROID` e' `[nome, 6378137, 298.257223563]`; il
/// `PRIMEM` e' `["Greenwich", 0]`; lo `UNIT` e' `["degree", 0.0174532925199433]`.
/// I nomi sono interi, ASCII case-insensitive: `WGS 84` o `GCS_WGS_1984` per la
/// radice, `WGS_1984` o `D_WGS_1984` per il datum, `WGS 84` o `WGS_1984` per
/// l'ellissoide. I numeri si confrontano come decimali esatti
/// ([`stesso_decimale`]): `6378137.0` e' `6378137`, `0.0174532925199433` non
/// ammette un'altra cifra, una forma esponenziale non e' riconosciuta.
///
/// Fino al 24/10 bastavano i due nomi, e `GEOGCS["WGS 84",DATUM["WGS_1984",
/// SPHEROID["GRS 1980",...]],PRIMEM["Paris",2.337],UNIT["grad",...]]` diventava
/// `EPSG:4326`, come un `DATUM` senza ellissoide. Un nodo in piu' (`AXIS`,
/// `TOWGS84`, un `AUTHORITY` annidato) toglie la forma: la deduzione vale solo
/// dove la definizione non puo' dire altro. Una forma non riconosciuta resta
/// anonima: non risolta senza opzione, e l'opzione la nomina.
fn wgs84_esri(radice: &Nodo) -> bool {
    let chiamato = |nome: &str, nomi: &[&str]| nomi.iter().any(|n| nome.eq_ignore_ascii_case(n));
    let e = |nodo: &Nodo, parola: &str| nodo.parola.eq_ignore_ascii_case(parola);
    let [Voce::Stringa(nome), Voce::Nodo(datum), Voce::Nodo(primem), Voce::Nodo(unit)] =
        radice.voci.as_slice()
    else {
        return false;
    };
    let [Voce::Stringa(nome_datum), Voce::Nodo(sferoide)] = datum.voci.as_slice() else {
        return false;
    };
    let [Voce::Stringa(nome_sferoide), Voce::Numero(semiasse), Voce::Numero(appiattimento)] =
        sferoide.voci.as_slice()
    else {
        return false;
    };
    let [Voce::Stringa(nome_primem), Voce::Numero(longitudine)] = primem.voci.as_slice() else {
        return false;
    };
    let [Voce::Stringa(nome_unit), Voce::Numero(radianti)] = unit.voci.as_slice() else {
        return false;
    };
    e(radice, "GEOGCS")
        && e(datum, "DATUM")
        && e(sferoide, "SPHEROID")
        && e(primem, "PRIMEM")
        && e(unit, "UNIT")
        && chiamato(nome, &["WGS 84", "GCS_WGS_1984"])
        && chiamato(nome_datum, &["WGS_1984", "D_WGS_1984"])
        && chiamato(nome_sferoide, &["WGS 84", "WGS_1984"])
        && chiamato(nome_primem, &["Greenwich"])
        && chiamato(nome_unit, &["degree"])
        && stesso_decimale(semiasse, "6378137")
        && stesso_decimale(appiattimento, "298.257223563")
        && stesso_decimale(longitudine, "0")
        && stesso_decimale(radianti, "0.0174532925199433")
}

/// Due numeri WKT valgono lo stesso decimale, esattamente: segno, cifre intere
/// senza zeri iniziali e cifre decimali senza zeri finali uguali (`-0` vale
/// `0`). Un numero con esponente non e' uguale a niente: nessuna conversione in
/// virgola mobile decide un'identita'.
fn stesso_decimale(testo: &str, canonico: &str) -> bool {
    fn normale(testo: &str) -> Option<(bool, &str, &str)> {
        let (negativo, cifre) = match testo.as_bytes().first() {
            Some(b'-') => (true, &testo[1..]),
            Some(b'+') => (false, &testo[1..]),
            _ => (false, testo),
        };
        let (intera, frazione) = cifre.split_once('.').unwrap_or((cifre, ""));
        if !intera
            .bytes()
            .chain(frazione.bytes())
            .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let intera = intera.trim_start_matches('0');
        let frazione = frazione.trim_end_matches('0');
        Some((
            negativo && !(intera.is_empty() && frazione.is_empty()),
            intera,
            frazione,
        ))
    }
    matches!((normale(testo), normale(canonico)), (Some(a), Some(b)) if a == b)
}

/// Il genere del CRS che la definizione descrive, dalla parola della **radice**
/// letta con il lettore stretto, o dal `type` della radice di un PROJJSON.
///
/// `GEOGCS`, `GEOGCRS`, `GEOGRAPHICCRS` (`GeographicCRS`) sono geografici;
/// `PROJCS`, `PROJCRS`, `PROJECTEDCRS` (`ProjectedCRS`) sono proiettati; un
/// `GEODCRS` o `GEODETICCRS` (`GeodeticCRS`) e' geografico solo con un
/// `CS[ellipsoidal,...]` figlio diretto (`coordinate_system.subtype`
/// `ellipsoidal`), perche' altrimenti e' geocentrico. Ogni altra radice --
/// `GEOCCS`, `BOUNDCRS`, `COMPOUNDCRS`, `COMPD_CS`, `DERIVEDPROJCRS`, verticali,
/// ingegneristici -- ha genere ignoto: quello di un CRS composto o vincolato
/// non e' quello di una sua parte. `None` per un PROJJSON senza `type`, che
/// del genere non dice niente: decide l'identificatore.
///
/// Fino al 24/10 i driver cercavano `PROJCS[` e `GEOGCS[` **nel testo**: un
/// `PROJECTEDCRS[`, `projcrs [` o `PROJCS(` usciva geografico per il
/// `BASEGEOGCRS[` annidato, e con gli assi est/nord diventava `lon_lat`.
///
/// # Errors
/// [`Malformata`] se la definizione non e' ben formata nel formato dichiarato.
pub fn genere_radice(
    testo: &str,
    formato: CrsDefinitionFormat,
) -> Result<Option<crate::crs::CrsKind>, Malformata> {
    use crate::crs::CrsKind;
    let parola_di = |radice: &Nodo, elenco: &[&str]| {
        elenco
            .iter()
            .any(|parola| radice.parola.eq_ignore_ascii_case(parola))
    };
    let (geografiche, proiettate, radice): (&[&str], &[&str], Nodo) = match formato {
        CrsDefinitionFormat::Projjson => return genere_projjson(testo),
        CrsDefinitionFormat::Wkt => (&["GEOGCS"], &["PROJCS"], radice_wkt(testo, &RADICI_WKT1)?),
        CrsDefinitionFormat::Wkt2 => (
            &["GEOGCRS", "GEOGRAPHICCRS"],
            &["PROJCRS", "PROJECTEDCRS"],
            radice_wkt(testo, &RADICI_WKT2)?,
        ),
    };
    let geodetica_ellissoidale = parola_di(&radice, &["GEODCRS", "GEODETICCRS"])
        && radice.voci.iter().any(|voce| {
            matches!(voce, Voce::Nodo(cs) if cs.parola.eq_ignore_ascii_case("CS")
                && matches!(cs.voci.first(), Some(Voce::Parola(tipo)) if tipo.eq_ignore_ascii_case("ellipsoidal")))
        });
    Ok(Some(
        if parola_di(&radice, geografiche) || geodetica_ellissoidale {
            CrsKind::Geographic
        } else if parola_di(&radice, proiettate) {
            CrsKind::Projected
        } else {
            CrsKind::Unknown
        },
    ))
}

/// La radice e' un `BOUNDCRS` (WKT 2) o un `BoundCRS` (PROJJSON): lega un CRS
/// sorgente a uno di destinazione e non ha un genere proprio. `false` per un
/// testo malformato.
#[must_use]
pub fn radice_legata(testo: &str, formato: CrsDefinitionFormat) -> bool {
    match formato {
        CrsDefinitionFormat::Projjson => {
            testo.len() <= MAX_BYTE_DEFINIZIONE_CRS
                && crate::json::from_str::<serde_json::Value>(testo).is_ok_and(|radice| {
                    radice.get("type").and_then(serde_json::Value::as_str) == Some("BoundCRS")
                })
        }
        CrsDefinitionFormat::Wkt => false,
        CrsDefinitionFormat::Wkt2 => radice_wkt(testo, &RADICI_WKT2)
            .is_ok_and(|radice| radice.parola.eq_ignore_ascii_case("BOUNDCRS")),
    }
}

// Lo stesso limite dichiarato di `assi_projjson` (docs/PRODUCT.md, «CRS:
// PROJJSON»): un documento che `serde_json` rifiuta e' trattato come
// malformato, e il chiamante ne fa un genere ignoto.
fn genere_projjson(testo: &str) -> Result<Option<crate::crs::CrsKind>, Malformata> {
    use crate::crs::CrsKind;
    if testo.len() > MAX_BYTE_DEFINIZIONE_CRS {
        return Err(Malformata);
    }
    let radice: serde_json::Value = crate::json::from_str(testo).map_err(|_| Malformata)?;
    let radice = radice.as_object().ok_or(Malformata)?;
    let ellissoidale = radice
        .get("coordinate_system")
        .and_then(|cs| cs.get("subtype"))
        .and_then(serde_json::Value::as_str)
        == Some("ellipsoidal");
    Ok(radice.get("type").map(|tipo| match tipo.as_str() {
        Some("GeographicCRS") => CrsKind::Geographic,
        Some("GeodeticCRS") if ellissoidale => CrsKind::Geographic,
        Some("ProjectedCRS") => CrsKind::Projected,
        _ => CrsKind::Unknown,
    }))
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
    // La difesa del lettore: oltre il tetto non si apre. A dire
    // `limit_exceeded` e' `verifica_dimensione_definizione`, prima.
    if testo.len() > MAX_BYTE_DEFINIZIONE_CRS {
        return Err(Malformata);
    }
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
    if testo.len() > MAX_BYTE_DEFINIZIONE_CRS {
        return Err(Malformata);
    }
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

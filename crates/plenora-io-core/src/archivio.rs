//! ZIP non fidati: preflight centrale senza allocazioni proporzionali ai conteggi,
//! estrazione a blocchi, conteggio reale e CRC.
//!
//! # Le tre regole uniche
//!
//! * **Nomi**: un nome e' una sequenza di componenti separate da `/`; ogni
//!   componente e' portabile (`portabile`) e si confronta per chiave
//!   (`dataset_files::name_key`). Voci e prefissi si confrontano componente
//!   per componente (`key`, `sotto`), mai tagliando per lunghezza in byte un
//!   testo trasformato. Nella vista ogni file nasce con `create_new`: un alias
//!   del filesystem che `name_key` non vede e' un errore, mai una sovrascrittura.
//! * **Errori**: ogni rifiuto passa da una riga di `error_table::Situation`
//!   (archivio non valido, voce assente, nome non portabile, annidato, metodo
//!   non supportato, nomi ambigui) o da `superato`, che porta in
//!   `details.quota` il nome stabile del limite vincolante.
//! * **Limiti**: `REGOLE` lega ogni limite d'archivio alla quota della pipeline
//!   che lo stringe; `vincolo` rende il valore effettivo e chi lo impone.
use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{ReadOptions, Sorgente};
use plenora_io_model::error_table::Situation;
use plenora_io_model::quota::Quota;
use plenora_io_model::{
    ErrorCategory, ErrorPhase, IoErrorCode, NumeroStrutturale, PlenoraIoError, PublicMessage,
    RemoteEffect, Result, RetryDisposition,
};

/// I tre limiti propri di un archivio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaArchivio {
    Voci,
    ByteDecompressi,
    Rapporto,
}

/// Una riga della tabella dei limiti d'archivio.
#[derive(Clone, Copy, Debug)]
pub struct RegolaQuotaArchivio {
    pub quota: QuotaArchivio,
    /// Nome stabile, in `details.quota` quando e' il limite vincolante.
    pub nome: &'static str,
    pub flag_cli: &'static str,
    pub campo_sdk: &'static str,
    pub default: u64,
    /// La quota della pipeline che stringe il limite: vale il minimo dei due,
    /// e `archive-list` lo dichiara in `limits`.
    pub pipeline: Quota,
}

/// La tabella unica: CLI, SDK, validazione, errori e documentazione la leggono.
pub const REGOLE: [RegolaQuotaArchivio; 3] = [
    RegolaQuotaArchivio {
        quota: QuotaArchivio::Voci,
        nome: "archive_max_entries",
        flag_cli: "--archive-max-entries",
        campo_sdk: "max_voci",
        default: 10_000,
        pipeline: Quota::InputEntries,
    },
    RegolaQuotaArchivio {
        quota: QuotaArchivio::ByteDecompressi,
        nome: "archive_max_bytes",
        flag_cli: "--archive-max-bytes",
        campo_sdk: "max_byte_decompressi",
        default: 512 * 1024 * 1024,
        pipeline: Quota::InputBytes,
    },
    RegolaQuotaArchivio {
        quota: QuotaArchivio::Rapporto,
        nome: "archive_max_ratio",
        flag_cli: "--archive-max-ratio",
        campo_sdk: "max_rapporto",
        default: 100,
        pipeline: Quota::DecompressionRatio,
    },
];

impl QuotaArchivio {
    #[must_use]
    pub const fn regola(self) -> RegolaQuotaArchivio {
        REGOLE[self as usize]
    }
    fn non_valida(self) -> PlenoraIoError {
        Situation::InvalidQuota.error_with(&PublicMessage::CuratedPair(
            self.regola().nome,
            "deve essere un intero positivo",
        ))
    }
}

/// Limiti cumulativi applicati anche alle voci non selezionate. Si costruiscono
/// solo validati: zero e' rifiutato con l'errore delle quote (`CLI_USAGE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitiArchivio {
    valori: [u64; 3],
}
impl Default for LimitiArchivio {
    fn default() -> Self {
        Self {
            valori: [REGOLE[0].default, REGOLE[1].default, REGOLE[2].default],
        }
    }
}
impl LimitiArchivio {
    /// # Errors
    /// Un valore zero.
    pub fn nuovi(max_voci: u64, max_byte_decompressi: u64, max_rapporto: u64) -> Result<Self> {
        Self::default()
            .con(QuotaArchivio::Voci, max_voci)?
            .con(QuotaArchivio::ByteDecompressi, max_byte_decompressi)?
            .con(QuotaArchivio::Rapporto, max_rapporto)
    }
    /// # Errors
    /// Un valore zero.
    pub fn con(mut self, quota: QuotaArchivio, valore: u64) -> Result<Self> {
        if valore == 0 {
            return Err(quota.non_valida());
        }
        self.valori[quota as usize] = valore;
        Ok(self)
    }
    #[must_use]
    pub const fn valore(&self, quota: QuotaArchivio) -> u64 {
        self.valori[quota as usize]
    }
}

/// Il limite effettivo e il nome di chi lo impone: la riduzione a una quota
/// della pipeline e' dichiarata, non silenziosa.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Vincolo {
    pub quota: &'static str,
    pub limit: u64,
    pub bound_by: &'static str,
}
/// Il minimo fra limite d'archivio e quota della pipeline, con il suo nome.
#[must_use]
pub fn vincolo(
    quota: QuotaArchivio,
    limiti: &LimitiArchivio,
    opts: Option<&ReadOptions>,
) -> Vincolo {
    let regola = quota.regola();
    let proprio = limiti.valore(quota);
    let pipeline = opts.map(|o| {
        let l = o.budget().context().limits();
        match quota {
            QuotaArchivio::Voci => l.max_input_entries(),
            QuotaArchivio::ByteDecompressi => l.max_input_bytes(),
            QuotaArchivio::Rapporto => l.decompression_ratio(),
        }
    });
    match pipeline {
        Some(p) if p < proprio => Vincolo {
            quota: regola.nome,
            limit: p,
            bound_by: regola.pipeline.rule().name,
        },
        _ => Vincolo {
            quota: regola.nome,
            limit: proprio,
            bound_by: regola.nome,
        },
    }
}
fn superato(v: Vincolo, observed: u64) -> PlenoraIoError {
    let mut error = PlenoraIoError::redatto(
        IoErrorCode::LimitExceeded,
        ErrorCategory::ResourceLimit,
        ErrorPhase::Read,
        RemoteEffect::None,
        RetryDisposition::Never,
        &PublicMessage::CuratedBetween(
            v.bound_by,
            NumeroStrutturale::Conteggio(observed),
            "oltre il limite di",
            NumeroStrutturale::Limite(v.limit),
        ),
    );
    error.quota_details = Some(Box::new(plenora_io_model::error::QuotaExceeded {
        quota: v.bound_by.to_owned(),
        observed,
        limit: v.limit,
    }));
    error
}
fn verifica(v: Vincolo, observed: u64) -> Result<()> {
    if observed > v.limit {
        return Err(superato(v, observed));
    }
    Ok(())
}
/// Il rapporto osservato, per eccesso; senza byte compressi e' illimitato.
const fn rapporto_di(expanded: u64, compressed: u64) -> u64 {
    if expanded == 0 {
        0
    } else if compressed == 0 {
        u64::MAX
    } else {
        expanded.div_ceil(compressed)
    }
}

/// Metadati della directory centrale. Le dimensioni sono **dichiarate**: CRC e
/// lunghezza reale si verificano solo estraendo, e il contratto lo dice nel
/// nome dei campi.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct VoceArchivio {
    #[serde(rename = "name")]
    pub nome: String,
    #[serde(rename = "declared_compressed_bytes")]
    pub byte_compressi: u64,
    #[serde(rename = "declared_uncompressed_bytes")]
    pub byte_decompressi: u64,
    pub directory: bool,
}

pub(crate) trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

fn invalid() -> PlenoraIoError {
    let mut error = Situation::InvalidArchive.error();
    error.driver = Some("zip".to_owned());
    error
}
fn active(opts: Option<&ReadOptions>) -> Result<()> {
    opts.map_or(Ok(()), ReadOptions::ensure_active)
}
fn exact(reader: &mut dyn Read, bytes: &mut [u8]) -> Result<()> {
    reader.read_exact(bytes).map_err(|_| invalid())
}
const fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
const fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
const fn u64le(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(invalid)
}

// --- la regola unica dei nomi ---------------------------------------------

/// Dispositivi Windows riservati, con o senza estensione e spazi finali.
const DISPOSITIVI: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
const CIFRE_DISPOSITIVO: [&str; 13] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "\u{b9}", "\u{b2}", "\u{b3}",
];
fn dispositivo(part: &str) -> bool {
    let base = part
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    DISPOSITIVI.contains(&base.as_str())
        || ["COM", "LPT"].iter().any(|p| {
            base.strip_prefix(p)
                .is_some_and(|n| CIFRE_DISPOSITIVO.contains(&n))
        })
}
/// Il predicato unico: include ADS e alias Windows anche su Unix.
fn portabile(nome: &str) -> bool {
    let trimmed = nome.strip_suffix('/').unwrap_or(nome);
    !(trimmed.is_empty() || nome.contains(['\\', ':', '\0']) || nome.starts_with('/'))
        && trimmed.split('/').all(|part| {
            !(part.is_empty()
                || matches!(part, "." | "..")
                || part.ends_with(['.', ' '])
                || part
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
                || dispositivo(part))
        })
}
/// Nome scelto dal chiamante: sorgente in memoria o voce richiesta.
/// # Errors
/// `CLI_USAGE` (`NonPortableSourceName`): assoluto, ambiguo o non portabile.
pub fn valida_nome(nome: &str) -> Result<()> {
    if portabile(nome) {
        Ok(())
    } else {
        Err(Situation::NonPortableSourceName.error())
    }
}
/// La chiave di un nome: una chiave per componente.
fn key(nome: &str) -> Vec<Vec<u8>> {
    nome.trim_end_matches('/')
        .split('/')
        .map(|c| crate::dataset_files::name_key(std::ffi::OsStr::new(c)))
        .collect()
}
/// Il resto di `nome`, testo originale, se le sue prime componenti hanno le
/// chiavi di `radice` e ne segue almeno un'altra.
fn sotto<'a>(nome: &'a str, radice: &[Vec<u8>]) -> Option<&'a str> {
    let mut parti = nome.trim_end_matches('/').splitn(radice.len() + 1, '/');
    for attesa in radice {
        let componente = parti.next()?;
        if crate::dataset_files::name_key(std::ffi::OsStr::new(componente)) != *attesa {
            return None;
        }
    }
    parti.next()
}

// --- nomi nella directory centrale: UTF-8, Info-ZIP 0x7075, CP437 ----------

/// CP437 da 0x80 a 0xFF (APPNOTE, appendice D).
const CP437: [char; 128] = [
    '\u{c7}', '\u{fc}', '\u{e9}', '\u{e2}', '\u{e4}', '\u{e0}', '\u{e5}', '\u{e7}', '\u{ea}',
    '\u{eb}', '\u{e8}', '\u{ef}', '\u{ee}', '\u{ec}', '\u{c4}', '\u{c5}', '\u{c9}', '\u{e6}',
    '\u{c6}', '\u{f4}', '\u{f6}', '\u{f2}', '\u{fb}', '\u{f9}', '\u{ff}', '\u{d6}', '\u{dc}',
    '\u{a2}', '\u{a3}', '\u{a5}', '\u{20a7}', '\u{192}', '\u{e1}', '\u{ed}', '\u{f3}', '\u{fa}',
    '\u{f1}', '\u{d1}', '\u{aa}', '\u{ba}', '\u{bf}', '\u{2310}', '\u{ac}', '\u{bd}', '\u{bc}',
    '\u{a1}', '\u{ab}', '\u{bb}', '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}',
    '\u{2561}', '\u{2562}', '\u{2556}', '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}',
    '\u{255c}', '\u{255b}', '\u{2510}', '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}',
    '\u{253c}', '\u{255e}', '\u{255f}', '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}',
    '\u{2550}', '\u{256c}', '\u{2567}', '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}',
    '\u{2552}', '\u{2553}', '\u{256b}', '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}',
    '\u{258c}', '\u{2590}', '\u{2580}', '\u{3b1}', '\u{df}', '\u{393}', '\u{3c0}', '\u{3a3}',
    '\u{3c3}', '\u{b5}', '\u{3c4}', '\u{3a6}', '\u{398}', '\u{3a9}', '\u{3b4}', '\u{221e}',
    '\u{3c6}', '\u{3b5}', '\u{2229}', '\u{2261}', '\u{b1}', '\u{2265}', '\u{2264}', '\u{2320}',
    '\u{2321}', '\u{f7}', '\u{2248}', '\u{b0}', '\u{2219}', '\u{b7}', '\u{221a}', '\u{207f}',
    '\u{b2}', '\u{25a0}', '\u{a0}',
];
/// Bit 11: UTF-8. Altrimenti il campo Info-ZIP 0x7075, se il suo CRC
/// corrisponde al nome grezzo; altrimenti CP437, come prescrive APPNOTE.
/// Un nome scritto in CP850 da Esplora risorse si legge con i glifi CP437
/// (limite dichiarato: il codepage OEM non e' scritto nell'archivio).
fn decodifica_nome(raw: &[u8], flags: u16, unicode: Option<&[u8]>) -> Result<String> {
    if flags & 0x0800 != 0 {
        return String::from_utf8(raw.to_vec()).map_err(|_| invalid());
    }
    if let Some(field) = unicode {
        if field.len() >= 5 && field[0] == 1 && u32le(&field[1..]) == crc32fast::hash(raw) {
            return String::from_utf8(field[5..].to_vec()).map_err(|_| invalid());
        }
    }
    Ok(raw
        .iter()
        .map(|&b| {
            if b < 0x80 {
                char::from(b)
            } else {
                CP437[usize::from(b - 0x80)]
            }
        })
        .collect())
}

/// I campi extra che servono: ZIP64 (0x0001) e nome Unicode Info-ZIP (0x7075).
fn campi_extra<'a>(extra: &'a [u8], mut sizes: [&mut u64; 3]) -> Result<Option<&'a [u8]>> {
    let mut unicode = None;
    let mut remaining = extra;
    while !remaining.is_empty() {
        if remaining.len() < 4 {
            return Err(invalid());
        }
        let tag = u16le(remaining);
        let len = usize::from(u16le(&remaining[2..]));
        let field = remaining.get(4..4 + len).ok_or_else(invalid)?;
        if tag == 1 {
            let mut values = field;
            for value in &mut sizes {
                if **value == u64::from(u32::MAX) {
                    **value = u64le(values.get(..8).ok_or_else(invalid)?);
                    values = &values[8..];
                }
            }
        } else if tag == 0x7075 {
            unicode = Some(field);
        }
        remaining = &remaining[4 + len..];
    }
    Ok(unicode)
}

struct Entry {
    public: VoceArchivio,
    raw_name: Vec<u8>,
    offset: u64,
    /// Il primo byte che non appartiene piu' alla voce: l'inizio della voce
    /// successiva o della directory centrale. Rende le voci disgiunte.
    end: u64,
    method: u16,
    flags: u16,
    crc: u32,
}
struct Archive {
    reader: Box<dyn ReadSeek>,
    entries: Vec<Entry>,
    _lease: Option<plenora_io_model::budget::InternalMemoryLease>,
}
fn source_reader(source: &Sorgente) -> Result<Box<dyn ReadSeek>> {
    match source {
        Sorgente::Percorso(path) => {
            if !std::fs::symlink_metadata(path)?.is_file() {
                return Err(Situation::DirectoryInsteadOfFile.error());
            }
            Ok(Box::new(std::fs::File::open(path)?))
        }
        Sorgente::Byte { dati, .. } => Ok(Box::new(std::io::Cursor::new(Arc::clone(dati)))),
        Sorgente::Archivio { .. } => Err(Situation::NestedArchive.error()),
    }
}

// Seek backwards in a fixed-size stack buffer, never allocating for the ZIP comment.
fn directory(reader: &mut dyn ReadSeek, opts: Option<&ReadOptions>) -> Result<(u64, u64, u64)> {
    let length = reader.seek(SeekFrom::End(0))?;
    let mut h = [0_u8; 22];
    let start = length.saturating_sub(65_557);
    for offset in (start..=length.saturating_sub(22)).rev() {
        active(opts)?;
        reader.seek(SeekFrom::Start(offset))?;
        exact(reader, &mut h)?;
        if &h[..4] != b"PK\x05\x06" || add(offset, 22 + u64::from(u16le(&h[20..])))? != length {
            continue;
        }
        if u16le(&h[4..]) != 0 || u16le(&h[6..]) != 0 || u16le(&h[8..]) != u16le(&h[10..]) {
            return Err(invalid());
        }
        let (count, size, central) = (
            u64::from(u16le(&h[10..])),
            u64::from(u32le(&h[12..])),
            u64::from(u32le(&h[16..])),
        );
        if count != 65_535 && size != u64::from(u32::MAX) && central != u64::from(u32::MAX) {
            if add(central, size)? != offset {
                return Err(invalid());
            }
            return Ok((count, central, size));
        }
        let mut locator = [0; 20];
        reader.seek(SeekFrom::Start(offset.checked_sub(20).ok_or_else(invalid)?))?;
        exact(reader, &mut locator)?;
        if &locator[..4] != b"PK\x06\x07" || u32le(&locator[4..]) != 0 || u32le(&locator[16..]) != 1
        {
            return Err(invalid());
        }
        let zoffset = u64le(&locator[8..]);
        reader.seek(SeekFrom::Start(zoffset))?;
        let mut z = [0; 56];
        exact(reader, &mut z)?;
        if &z[..4] != b"PK\x06\x06"
            || u64le(&z[4..]) < 44
            || add(zoffset, add(12, u64le(&z[4..]))?)? != offset - 20
            || u32le(&z[16..]) != 0
            || u32le(&z[20..]) != 0
            || u64le(&z[24..]) != u64le(&z[32..])
        {
            return Err(invalid());
        }
        let (count, size, central) = (u64le(&z[32..]), u64le(&z[40..]), u64le(&z[48..]));
        if add(central, size)? != zoffset {
            return Err(invalid());
        }
        return Ok((count, central, size));
    }
    Err(invalid())
}

impl Archive {
    #[allow(clippy::too_many_lines)] // Two bounded central passes share offsets and identity sets.
    fn open(
        source: &Sorgente,
        limits: &LimitiArchivio,
        opts: Option<&ReadOptions>,
    ) -> Result<Self> {
        active(opts)?;
        let mut reader = source_reader(source)?;
        let length = reader.seek(SeekFrom::End(0))?;
        if let Some(o) = opts {
            if length > o.max_input_bytes() {
                return Err(Quota::InputBytes.exceeded_measured(
                    ErrorPhase::Read,
                    length,
                    o.max_input_bytes(),
                ));
            }
        }
        let (count, central, size) = directory(&mut *reader, opts)?;
        verifica(vincolo(QuotaArchivio::Voci, limits, opts), count)?;
        let byte = vincolo(QuotaArchivio::ByteDecompressi, limits, opts);
        let rapporto = vincolo(QuotaArchivio::Rapporto, limits, opts);
        // First pass: fixed headers only. No Vec, name, decoder, or parser allocation.
        let end = add(central, size)?;
        let mut pos = central;
        for _ in 0..count {
            active(opts)?;
            if add(pos, 46)? > end {
                return Err(invalid());
            }
            reader.seek(SeekFrom::Start(pos))?;
            let mut h = [0; 46];
            exact(&mut *reader, &mut h)?;
            if &h[..4] != b"PK\x01\x02" {
                return Err(invalid());
            }
            pos = add(
                pos,
                46 + u64::from(u16le(&h[28..]))
                    + u64::from(u16le(&h[30..]))
                    + u64::from(u16le(&h[32..])),
            )?;
            if pos > end {
                return Err(invalid());
            }
        }
        if pos != end {
            return Err(invalid());
        }
        let metadata_lease = opts
            .map(|o| {
                o.budget().context().lease_memory_internal(
                    size.saturating_mul(5)
                        .saturating_add(count.saturating_mul(512))
                        .saturating_add(65_536),
                )
            })
            .transpose()?;
        let mut entries: Vec<Entry> = Vec::new();
        let mut names = BTreeSet::new();
        let mut files = BTreeSet::new();
        let mut directories = BTreeSet::new();
        let mut total = 0_u64;
        pos = central;
        for _ in 0..count {
            active(opts)?;
            reader.seek(SeekFrom::Start(pos))?;
            let mut h = [0; 46];
            exact(&mut *reader, &mut h)?;
            let name_len = usize::from(u16le(&h[28..]));
            let extra_len = usize::from(u16le(&h[30..]));
            let mut raw_name = vec![0; name_len];
            exact(&mut *reader, &mut raw_name)?;
            let flags = u16le(&h[8..]);
            let method = u16le(&h[10..]);
            if flags & !0x080e != 0 || !matches!(method, 0 | 8) {
                return Err(Situation::UnsupportedArchiveFeature.error());
            }
            let mode = u32le(&h[38..]) >> 16;
            if (mode & 0xf000 != 0 && mode & 0xf000 != 0x8000 && mode & 0xf000 != 0x4000)
                || u16le(&h[34..]) != 0
            {
                return Err(invalid());
            }
            let mut compressed = u64::from(u32le(&h[20..]));
            let mut decompressed = u64::from(u32le(&h[24..]));
            let mut offset = u64::from(u32le(&h[42..]));
            // Extra fields are bounded by the u16 wire length; ZIP64 sizes precede allocation of payloads.
            let mut extra = vec![0; extra_len];
            exact(&mut *reader, &mut extra)?;
            let unicode = campi_extra(&extra, [&mut decompressed, &mut compressed, &mut offset])?;
            let nome = decodifica_nome(&raw_name, flags, unicode)?;
            if !portabile(&nome) {
                return Err(invalid());
            }
            let identity = key(&nome);
            if !names.insert(identity.clone()) {
                return Err(Situation::AmbiguousArchiveNames.error());
            }
            let directory = nome.ends_with('/');
            for depth in 1..identity.len() {
                directories.insert(identity[..depth].to_vec());
            }
            if directory {
                directories.insert(identity);
            } else {
                files.insert(identity);
            }
            // Per voce e sull'archivio intero: ogni byte dichiarato conta,
            // anche quello delle voci che nessuno selezionera'.
            total = total.saturating_add(decompressed);
            verifica(byte, total)?;
            verifica(rapporto, rapporto_di(decompressed, compressed))?;
            verifica(rapporto, rapporto_di(total, length))?;
            if add(offset, 30)? > central || (directory && decompressed != 0) {
                return Err(invalid());
            }
            entries.push(Entry {
                public: VoceArchivio {
                    nome,
                    byte_compressi: compressed,
                    byte_decompressi: decompressed,
                    directory,
                },
                raw_name,
                offset,
                end: central,
                method,
                flags,
                crc: u32le(&h[16..]),
            });
            pos = add(
                pos,
                46 + name_len as u64 + extra_len as u64 + u64::from(u16le(&h[32..])),
            )?;
        }
        if files.iter().any(|f| directories.contains(f)) {
            return Err(Situation::AmbiguousArchiveNames.error());
        }
        // Voci disgiunte: nessuna puo' cominciare dentro i dati di un'altra.
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by_key(|&i| entries[i].offset);
        for pair in order.windows(2) {
            let (a, b) = (&entries[pair[0]], &entries[pair[1]]);
            let minimum = add(
                add(a.offset, 30 + a.raw_name.len() as u64)?,
                a.public.byte_compressi,
            )?;
            if minimum > b.offset {
                return Err(invalid());
            }
            let next = b.offset;
            entries[pair[0]].end = next;
        }
        Ok(Self {
            reader,
            entries,
            _lease: metadata_lease,
        })
    }

    /// Estrae la voce `index`; con `fino_a` minore della voce si ferma prima,
    /// senza verificare il CRC: serve solo a riconoscere un prefisso.
    fn extract(
        &mut self,
        index: usize,
        limits: &LimitiArchivio,
        opts: Option<&ReadOptions>,
        fino_a: u64,
        mut write: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        active(opts)?;
        let entry = &self.entries[index];
        self.reader.seek(SeekFrom::Start(entry.offset))?;
        let mut h = [0; 30];
        exact(&mut *self.reader, &mut h)?;
        if &h[..4] != b"PK\x03\x04"
            || u16le(&h[6..]) != entry.flags
            || u16le(&h[8..]) != entry.method
        {
            return Err(invalid());
        }
        let mut name = vec![0; usize::from(u16le(&h[26..]))];
        exact(&mut *self.reader, &mut name)?;
        if name != entry.raw_name {
            return Err(invalid());
        }
        let start = add(
            entry.offset,
            30 + name.len() as u64 + u64::from(u16le(&h[28..])),
        )?;
        if add(start, entry.public.byte_compressi)? > entry.end {
            return Err(invalid());
        }
        self.reader.seek(SeekFrom::Start(start))?;
        let raw = (&mut self.reader).take(entry.public.byte_compressi);
        let mut decoded: Box<dyn Read + '_> = if entry.method == 8 {
            Box::new(flate2::read::DeflateDecoder::new(raw))
        } else {
            Box::new(raw)
        };
        let byte = vincolo(QuotaArchivio::ByteDecompressi, limits, opts);
        let rapporto = vincolo(QuotaArchivio::Rapporto, limits, opts);
        let declared = entry.public.byte_decompressi;
        let ceiling = declared
            .min(byte.limit)
            .min(entry.public.byte_compressi.saturating_mul(rapporto.limit));
        let mut total = 0_u64;
        let mut crc = crc32fast::Hasher::new();
        let mut buffer = [0; 8192];
        loop {
            active(opts)?;
            if total >= fino_a {
                return Ok(());
            }
            // At the declared limit, read only one sentinel byte, never allocate for excess output.
            let capacity = usize::try_from(
                ceiling
                    .saturating_sub(total)
                    .saturating_add(1)
                    .min(buffer.len() as u64),
            )
            .map_err(|_| invalid())?;
            let n = decoded
                .read(&mut buffer[..capacity])
                .map_err(|_| invalid())?;
            if n == 0 {
                break;
            }
            total = total.saturating_add(n as u64);
            if total > ceiling {
                // Oltre la dimensione dichiarata e' una voce che mente (formato);
                // oltre un limite e' quel limite, con il suo nome.
                return Err(if total > byte.limit {
                    superato(byte, total)
                } else if total > declared {
                    invalid()
                } else {
                    superato(rapporto, rapporto_di(total, entry.public.byte_compressi))
                });
            }
            crc.update(&buffer[..n]);
            write(&buffer[..n])?;
        }
        if total != declared || crc.finalize() != entry.crc {
            return Err(invalid());
        }
        Ok(())
    }

    fn voce(&self, voce: &str) -> Result<usize> {
        let wanted = key(voce);
        self.entries
            .iter()
            .position(|e| key(&e.public.nome) == wanted && !e.public.directory)
            .ok_or_else(|| Situation::ArchiveEntryMissing.error())
    }
}

/// Elenca un ZIP dopo il controllo delle intestazioni, senza decompressione.
/// # Errors
/// Formato, nomi, annidamento o limiti non validi. Non include dati negli errori.
pub fn elenca_archivio(src: &Sorgente, limiti: &LimitiArchivio) -> Result<Vec<VoceArchivio>> {
    Ok(Archive::open(src, limiti, None)?
        .entries
        .into_iter()
        .map(|e| e.public)
        .collect())
}

/// Come `elenca_archivio`, con cancellazione, deadline e quote della pipeline.
/// # Errors
/// Errori di archivio, cancellazione e deadline.
pub fn elenca_archivio_con_opzioni(
    src: &Sorgente,
    limiti: &LimitiArchivio,
    opts: &ReadOptions,
) -> Result<Vec<VoceArchivio>> {
    Ok(Archive::open(src, limiti, Some(opts))?
        .entries
        .into_iter()
        .map(|e| e.public)
        .collect())
}

/// Estrae una voce in memoria dopo il preflight di tutto l'archivio. I driver
/// non la usano: leggono le voci da una vista su disco con quota di spill.
/// # Errors
/// Errori di archivio, quota, cancellazione, voce assente, CRC o dimensione falsa.
pub fn leggi_voce(
    src: &Sorgente,
    voce: &str,
    limiti: &LimitiArchivio,
    opts: &ReadOptions,
) -> Result<Vec<u8>> {
    valida_nome(voce)?;
    let mut archive = Archive::open(src, limiti, Some(opts))?;
    let index = archive.voce(voce)?;
    let size = archive.entries[index].public.byte_decompressi;
    let _lease = opts.budget().context().lease_memory_internal(size.max(1))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(usize::try_from(size).map_err(|_| invalid())?)
        .map_err(|_| Quota::MemoryBytes.exceeded(ErrorPhase::Read))?;
    archive.extract(index, limiti, Some(opts), u64::MAX, |bytes| {
        output.extend_from_slice(bytes);
        Ok(())
    })?;
    Ok(output)
}

/// I primi `n` byte di una sorgente, per riconoscerne la serializzazione dai
/// byte e non dal nome.
/// # Errors
/// Errori di archivio o di I/O.
pub fn prefisso(src: &Sorgente, limiti: &LimitiArchivio, n: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    match src {
        Sorgente::Percorso(path) => {
            std::fs::File::open(path)?
                .take(n as u64)
                .read_to_end(&mut out)?;
        }
        Sorgente::Byte { dati, .. } => out.extend_from_slice(&dati[..n.min(dati.len())]),
        Sorgente::Archivio { archivio, voce } => {
            let mut archive = Archive::open(archivio, limiti, None)?;
            let index = archive.voce(voce)?;
            archive.extract(index, limiti, None, n as u64, |bytes| {
                out.extend_from_slice(&bytes[..bytes.len().min(n - out.len())]);
                Ok(())
            })?;
        }
    }
    Ok(out)
}

/// Il documento di un KMZ: il primo `.kml` alla radice in ordine di directory
/// centrale (KML 2.2, KMZ), e quanti altri `.kml` l'archivio contiene.
/// # Errors
/// Archivio non valido o annidato; `SOURCE_NOT_FOUND` senza `.kml` alla radice.
pub fn documento_kmz(
    src: &Sorgente,
    limiti: &LimitiArchivio,
    opts: &ReadOptions,
) -> Result<(String, u64)> {
    let archive = Archive::open(src, limiti, Some(opts))?;
    let tutti: Vec<&VoceArchivio> = archive
        .entries
        .iter()
        .map(|e| &e.public)
        .filter(|e| {
            !e.directory
                && Path::new(&e.nome)
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("kml"))
        })
        .collect();
    let documento = tutti
        .iter()
        .find(|e| !e.nome.contains('/'))
        .ok_or_else(|| Situation::ArchiveEntryMissing.error())?;
    Ok((documento.nome.clone(), tutti.len() as u64 - 1))
}

/// Vista temporanea privata nella directory di spill, mantenuta viva anche dai
/// reader dopo il drop del dataset. Dopo un `SIGKILL` resta come directory
/// `plenora-source-*` nella directory di spill (limite dichiarato).
#[derive(Debug)]
pub(crate) struct Vista {
    // Tenuta per il suo Drop: la directory vive quanto l'ultimo `Arc`.
    #[allow(dead_code)]
    directory: tempfile::TempDir,
    _spill: std::sync::Mutex<Vec<plenora_io_model::budget::SpillLease>>,
}

fn crea_nuovo(path: &Path) -> Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Situation::AmbiguousArchiveNames.error()
            } else {
                e.into()
            }
        })
}

/// Le voci da estrarre e la loro destinazione relativa alla vista.
///
/// Una voce con discendenti (`x.gdb`, `x.shp.d`) porta il sottoalbero; una
/// voce file porta se stessa e i membri dell'insieme fisico che stanno nella
/// sua cartella, tutti sotto il nome richiesto.
fn seleziona(
    archive: &Archive,
    voce: &str,
    files: &crate::dataset_files::FileSet,
) -> Result<Vec<(usize, PathBuf)>> {
    let radice = key(voce);
    let base = Path::new(voce.trim_end_matches('/'));
    let discendenti: Vec<(usize, &str)> = archive
        .entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| sotto(&e.public.nome, &radice).map(|rest| (i, rest)))
        .collect();
    if !discendenti.is_empty() {
        return discendenti
            .into_iter()
            .map(|(i, rest)| {
                if !files.tree {
                    return Ok((i, base.join(rest)));
                }
                // Only physical FileGDB members belong to the private view.
                if rest.contains('/') {
                    return Err(invalid());
                }
                let canonical = crate::filegdb_names::parse(rest)
                    .map_or_else(|| rest.to_owned(), |m| m.canonical);
                Ok((i, base.join(canonical)))
            })
            .collect();
    }
    let principale = archive.voce(voce)?;
    let cartella = base.parent().unwrap_or_else(|| Path::new(""));
    let membri: Vec<Vec<Vec<u8>>> = files
        .members
        .iter()
        .filter_map(|m| m.path(base).to_str().map(|p| key(&p.replace('\\', "/"))))
        .collect();
    Ok(archive
        .entries
        .iter()
        .enumerate()
        .filter(|(i, e)| {
            *i == principale || (!e.public.directory && membri.contains(&key(&e.public.nome)))
        })
        .map(|(i, e)| {
            let nome = if i == principale {
                base.file_name()
            } else {
                Path::new(&e.public.nome).file_name()
            };
            (i, cartella.join(nome.unwrap_or_default()))
        })
        .collect())
}

/// Il percorso fisico di una sorgente: invariato per un percorso, una vista
/// privata con quota di spill per byte in memoria e voci d'archivio.
/// # Errors
/// Nome non portabile, archivio, quote, cancellazione o I/O.
pub(crate) fn prepara_percorso(
    source: Sorgente,
    files: &crate::dataset_files::FileSet,
    opts: &ReadOptions,
) -> Result<(PathBuf, Option<Arc<Vista>>)> {
    let (nome, contenuto) = match source {
        Sorgente::Percorso(path) => return Ok((path, None)),
        Sorgente::Byte { nome, dati } => (nome, Ok(dati)),
        Sorgente::Archivio { archivio, voce } => (voce, Err(archivio)),
    };
    valida_nome(&nome)?;
    active(Some(opts))?;
    let directory = tempfile::Builder::new()
        .prefix("plenora-source-")
        .tempdir_in(crate::driver::spool::spill_directory()?)?;
    let mut leases = Vec::new();
    match contenuto {
        Ok(dati) => {
            let length = dati.len() as u64;
            if length > opts.max_input_bytes() {
                return Err(Quota::InputBytes.exceeded_measured(
                    ErrorPhase::Read,
                    length,
                    opts.max_input_bytes(),
                ));
            }
            leases.push(opts.budget().context().lease_spill(length.max(1))?);
            let mut output = crea_nuovo(&directory.path().join(&nome))?;
            for block in dati.chunks(8192) {
                active(Some(opts))?;
                std::io::Write::write_all(&mut output, block)?;
            }
        }
        Err(archivio) => {
            let limits = opts.limiti_archivio;
            let mut archive = Archive::open(&archivio, &limits, Some(opts))?;
            let selected = seleziona(&archive, &nome, files)?;
            let total = selected.iter().fold(0_u64, |n, (i, _)| {
                n.saturating_add(archive.entries[*i].public.byte_decompressi)
            });
            verifica(
                vincolo(QuotaArchivio::ByteDecompressi, &limits, Some(opts)),
                total,
            )?;
            verifica(
                vincolo(QuotaArchivio::Voci, &limits, Some(opts)),
                selected.len() as u64,
            )?;
            leases.push(opts.budget().context().lease_spill(total.max(1))?);
            for (index, relative) in selected {
                active(Some(opts))?;
                let destination = directory.path().join(relative);
                if archive.entries[index].public.directory {
                    std::fs::create_dir_all(destination)?;
                    continue;
                }
                let mut output = crea_nuovo(&destination)?;
                archive.extract(index, &limits, Some(opts), u64::MAX, |bytes| {
                    std::io::Write::write_all(&mut output, bytes)?;
                    Ok(())
                })?;
            }
        }
    }
    Ok((
        directory.path().join(nome.trim_end_matches('/')),
        Some(Arc::new(Vista {
            directory,
            _spill: std::sync::Mutex::new(leases),
        })),
    ))
}

#[cfg(test)]
mod tests;

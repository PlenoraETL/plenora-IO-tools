//! ZIP non fidati: preflight centrale senza allocazioni proporzionali ai conteggi,
//! estrazione a blocchi, conteggio reale e CRC. Nessun nome raggiunge il filesystem.
use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use crate::{ReadOptions, Source};
use plenora_io_model::{PlenoraIoError, PublicMessage, Result};

/// Limiti cumulativi applicati anche alle voci non selezionate.
#[derive(Clone, Copy, Debug)]
pub struct LimitiArchivio {
    pub max_voci: u32,
    pub max_byte_decompressi: u64,
    pub max_rapporto: u32,
}
impl Default for LimitiArchivio {
    fn default() -> Self {
        Self {
            max_voci: 10_000,
            max_byte_decompressi: 512 * 1024 * 1024,
            max_rapporto: 100,
        }
    }
}
/// Metadati verificati della directory centrale; non attestano ancora il CRC.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct VoceArchivio {
    pub nome: String,
    pub byte_compressi: u64,
    pub byte_decompressi: u64,
    pub directory: bool,
}

pub(crate) trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

fn invalid() -> PlenoraIoError {
    PlenoraIoError::formato_redatto("zip", &PublicMessage::Curated("archivio non valido"))
}
fn limit() -> PlenoraIoError {
    PlenoraIoError::limite_redatto(&PublicMessage::Curated("limite archivio superato"))
}
fn unsupported() -> PlenoraIoError {
    PlenoraIoError::non_supportato_redatto(&PublicMessage::Curated(
        "archivio annidato, cifrato o metodo ZIP non supportato",
    ))
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

/// Validazione portabile: include ADS e alias Windows, anche su Unix.
/// # Errors
/// Nome assoluto, ambiguo, non portabile o con attraversamento di directory.
pub fn valida_nome(nome: &str) -> Result<()> {
    let trimmed = nome.strip_suffix('/').unwrap_or(nome);
    if trimmed.is_empty() || nome.contains(['\\', ':', '\0']) || nome.starts_with('/') {
        return Err(invalid());
    }
    for part in trimmed.split('/') {
        let device = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if part.is_empty()
            || matches!(part, "." | "..")
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
            || matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (device.len() == 4
                && (device.starts_with("COM") || device.starts_with("LPT"))
                && device.as_bytes()[3].is_ascii_digit())
        {
            return Err(invalid());
        }
    }
    Ok(())
}
fn key(nome: &str) -> Vec<u8> {
    crate::dataset_files::name_key(std::ffi::OsStr::new(nome.trim_end_matches('/')))
}

struct Entry {
    public: VoceArchivio,
    offset: u64,
    method: u16,
    flags: u16,
    crc: u32,
}
struct Archive {
    reader: Box<dyn ReadSeek>,
    entries: Vec<Entry>,
    central: u64,
    _lease: Option<plenora_io_model::budget::InternalMemoryLease>,
}
fn source_reader(source: &Source) -> Result<Box<dyn ReadSeek>> {
    match source {
        Source::Path(path) => {
            if !std::fs::symlink_metadata(path)?.is_file() {
                return Err(invalid());
            }
            Ok(Box::new(std::fs::File::open(path)?))
        }
        Source::Bytes { dati, .. } => Ok(Box::new(std::io::Cursor::new(Arc::clone(dati)))),
        Source::Archivio { .. } => Err(unsupported()),
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
    #[allow(clippy::too_many_lines)] // Two bounded central passes share offsets and identity sets.
    fn open(source: &Source, limits: &LimitiArchivio, opts: Option<&ReadOptions>) -> Result<Self> {
        active(opts)?;
        let mut reader = source_reader(source)?;
        if let Some(opts) = opts {
            if reader.seek(SeekFrom::End(0))? > opts.max_input_bytes() {
                return Err(limit());
            }
        }
        let (count, central, size) = directory(&mut *reader, opts)?;
        if count
            > opts.map_or_else(
                || u64::from(limits.max_voci),
                |o| u64::from(limits.max_voci).min(o.max_input_entries()),
            )
        {
            return Err(limit());
        }
        // First pass: fixed headers only. No Vec, name, decoder, or parser allocation.
        let end = add(central, size)?;
        let mut pos = central;
        let mut total = 0_u64;
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
        let mut entries = Vec::new();
        let mut names = BTreeSet::new();
        let mut files = BTreeSet::new();
        let mut directories = BTreeSet::new();
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
            let nome = String::from_utf8(raw_name).map_err(|_| invalid())?;
            valida_nome(&nome)?;
            let identity = key(&nome);
            if !names.insert(identity.clone()) {
                return Err(invalid());
            }
            let directory = nome.ends_with('/');
            if directory {
                directories.insert(identity.clone());
            } else {
                files.insert(identity.clone());
            }
            for (index, _) in nome.match_indices('/') {
                directories.insert(key(&nome[..index]));
            }
            let flags = u16le(&h[8..]);
            let method = u16le(&h[10..]);
            if flags & !0x080e != 0 || !matches!(method, 0 | 8) {
                return Err(unsupported());
            }
            let mode = u32le(&h[38..]) >> 16;
            if mode & 0xf000 != 0 && mode & 0xf000 != 0x8000 && mode & 0xf000 != 0x4000 {
                return Err(invalid());
            }
            let mut compressed = u64::from(u32le(&h[20..]));
            let mut decompressed = u64::from(u32le(&h[24..]));
            let mut offset = u64::from(u32le(&h[42..]));
            if u16le(&h[34..]) != 0 {
                return Err(invalid());
            }
            // Extra fields are bounded by the u16 wire length; ZIP64 sizes precede allocation of payloads.
            let mut extra = vec![0; extra_len];
            exact(&mut *reader, &mut extra)?;
            let mut remaining = extra.as_slice();
            while !remaining.is_empty() {
                if remaining.len() < 4 {
                    return Err(invalid());
                }
                let tag = u16le(remaining);
                let len = usize::from(u16le(&remaining[2..]));
                let field = remaining.get(4..4 + len).ok_or_else(invalid)?;
                if tag == 1 {
                    let mut values = field;
                    for value in [&mut decompressed, &mut compressed, &mut offset] {
                        if *value == u64::from(u32::MAX) {
                            *value = u64le(values.get(..8).ok_or_else(invalid)?);
                            values = &values[8..];
                        }
                    }
                }
                remaining = &remaining[4 + len..];
            }
            total = total.checked_add(decompressed).ok_or_else(limit)?;
            if total
                > opts.map_or(limits.max_byte_decompressi, |o| {
                    limits.max_byte_decompressi.min(o.max_input_bytes())
                })
                || decompressed > compressed.saturating_mul(u64::from(limits.max_rapporto))
            {
                return Err(limit());
            }
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
                offset,
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
            return Err(invalid());
        }
        Ok(Self {
            reader,
            entries,
            central,
            _lease: metadata_lease,
        })
    }

    fn extract(
        &mut self,
        index: usize,
        limits: &LimitiArchivio,
        opts: Option<&ReadOptions>,
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
        if name != entry.public.nome.as_bytes() {
            return Err(invalid());
        }
        let start = add(
            entry.offset,
            30 + name.len() as u64 + u64::from(u16le(&h[28..])),
        )?;
        if add(start, entry.public.byte_compressi)? > self.central {
            return Err(invalid());
        }
        self.reader.seek(SeekFrom::Start(start))?;
        let raw = (&mut self.reader).take(entry.public.byte_compressi);
        let mut decoded: Box<dyn Read + '_> = if entry.method == 8 {
            Box::new(flate2::read::DeflateDecoder::new(raw))
        } else {
            Box::new(raw)
        };
        let ceiling = entry
            .public
            .byte_decompressi
            .min(limits.max_byte_decompressi)
            .min(
                entry
                    .public
                    .byte_compressi
                    .saturating_mul(u64::from(limits.max_rapporto)),
            );
        let mut total = 0_u64;
        let mut crc = crc32fast::Hasher::new();
        let mut buffer = [0; 8192];
        loop {
            active(opts)?;
            // At the declared limit, read only one sentinel byte, never allocate for excess output.
            let capacity = usize::try_from(
                ceiling
                    .saturating_sub(total)
                    .saturating_add(1)
                    .min(buffer.len() as u64),
            )
            .map_err(|_| limit())?;
            let n = decoded
                .read(&mut buffer[..capacity])
                .map_err(|_| invalid())?;
            if n == 0 {
                break;
            }
            total = total.checked_add(n as u64).ok_or_else(limit)?;
            if total > ceiling {
                return Err(limit());
            }
            crc.update(&buffer[..n]);
            write(&buffer[..n])?;
        }
        if total != entry.public.byte_decompressi || crc.finalize() != entry.crc {
            return Err(invalid());
        }
        Ok(())
    }
}

/// Elenca un ZIP dopo il controllo delle intestazioni, senza decompressione.
/// # Errors
/// Formato, nomi, profondita' o limiti non validi. Non include dati negli errori.
pub fn elenca_archivio(src: &Source, limiti: &LimitiArchivio) -> Result<Vec<VoceArchivio>> {
    Ok(Archive::open(src, limiti, None)?
        .entries
        .into_iter()
        .map(|e| e.public)
        .collect())
}

/// Come `elenca_archivio`, con cancellazione e deadline della pipeline.
/// # Errors
/// Errori di archivio, cancellazione e deadline.
pub fn elenca_archivio_con_opzioni(
    src: &Source,
    limiti: &LimitiArchivio,
    opts: &ReadOptions,
) -> Result<Vec<VoceArchivio>> {
    Ok(Archive::open(src, limiti, Some(opts))?
        .entries
        .into_iter()
        .map(|e| e.public)
        .collect())
}

/// Estrae una voce in memoria dopo il preflight di tutto l'archivio.
/// # Errors
/// Errori di archivio, quota, cancellazione, voce assente, CRC o dimensione falsa.
pub fn leggi_voce(
    src: &Source,
    voce: &str,
    limiti: &LimitiArchivio,
    opts: &ReadOptions,
) -> Result<Vec<u8>> {
    valida_nome(voce)?;
    let mut archive = Archive::open(src, limiti, Some(opts))?;
    let index = archive
        .entries
        .iter()
        .position(|e| key(&e.public.nome) == key(voce) && !e.public.directory)
        .ok_or_else(invalid)?;
    let size = archive.entries[index].public.byte_decompressi;
    let _lease = opts.budget().context().lease_memory_internal(size.max(1))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(usize::try_from(size).map_err(|_| limit())?)
        .map_err(|_| limit())?;
    archive.extract(index, limiti, Some(opts), |bytes| {
        output.extend_from_slice(bytes);
        Ok(())
    })?;
    Ok(output)
}

/// Vista temporanea privata, mantenuta viva anche dai reader dopo il drop del dataset.
pub(crate) struct Vista {
    pub _directory: tempfile::TempDir,
    _spill: std::sync::Mutex<Vec<plenora_io_model::budget::SpillLease>>,
}

#[allow(clippy::too_many_lines)] // One RAII scope owns partial files and quota on all error paths.
pub(crate) fn prepara_percorso(
    source: Source,
    files: &crate::dataset_files::FileSet,
    opts: &ReadOptions,
) -> Result<(std::path::PathBuf, Option<Arc<Vista>>)> {
    if let Source::Path(path) = source {
        return Ok((path, None));
    }
    active(Some(opts))?;
    let directory = tempfile::Builder::new()
        .prefix("plenora-source-")
        .tempdir()?;
    let mut leases = Vec::new();
    let path = match source {
        Source::Bytes { nome, dati } => {
            valida_nome(&nome)?;
            if dati.len() as u64 > opts.max_input_bytes() {
                return Err(limit());
            }
            leases.push(
                opts.budget()
                    .context()
                    .lease_spill((dati.len() as u64).max(1))?,
            );
            let path = directory.path().join(nome);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut output = std::fs::File::create(&path)?;
            for block in dati.chunks(8192) {
                active(Some(opts))?;
                std::io::Write::write_all(&mut output, block)?;
            }
            path
        }
        Source::Archivio { archivio, voce } => {
            valida_nome(&voce)?;
            let limits = opts.limiti_archivio;
            let mut archive = Archive::open(&archivio, &limits, Some(opts))?;
            let wanted = key(&voce);
            let mut selected = Vec::new();
            let main = std::path::Path::new(&voce);
            let mut members: Vec<_> = files.members.iter().map(|m| m.path(main)).collect();
            members.push(main.to_owned());
            let member_keys: Vec<_> = members
                .iter()
                .filter_map(|p| p.to_str())
                .map(|p| key(&p.replace('\\', "/")))
                .collect();
            let prefix = format!("{}/", voce.trim_end_matches('/')).to_lowercase();
            for (index, entry) in archive.entries.iter().enumerate() {
                let entry_key = key(&entry.public.nome);
                let relevant = if files.tree {
                    entry.public.nome.to_lowercase().starts_with(&prefix)
                } else {
                    member_keys.contains(&entry_key)
                };
                if relevant {
                    selected.push(index);
                }
            }
            if selected.is_empty()
                || (!files.tree
                    && !selected
                        .iter()
                        .any(|i| key(&archive.entries[*i].public.nome) == wanted))
            {
                return Err(invalid());
            }
            let total = selected.iter().try_fold(0_u64, |n, i| {
                n.checked_add(archive.entries[*i].public.byte_decompressi)
                    .ok_or_else(limit)
            })?;
            if total > opts.max_input_bytes() || selected.len() as u64 > opts.max_input_entries() {
                return Err(limit());
            }
            leases.push(opts.budget().context().lease_spill(total.max(1))?);
            let mut resolved = directory.path().join(&voce);
            for index in selected {
                active(Some(opts))?;
                let entry = &archive.entries[index].public;
                let mut destination = directory.path().join(&entry.nome);
                if files.tree {
                    let relative = entry
                        .nome
                        .get(prefix.len()..)
                        .ok_or_else(invalid)?
                        .trim_end_matches('/');
                    // Only physical FileGDB members belong to the private view.
                    if relative.contains('/') {
                        return Err(invalid());
                    }
                    let canonical = crate::filegdb_names::parse(relative)
                        .map_or_else(|| relative.to_owned(), |member| member.canonical);
                    destination = directory.path().join(&voce).join(canonical);
                } else if key(&entry.nome) == wanted {
                    resolved.clone_from(&destination);
                }
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if entry.directory {
                    std::fs::create_dir_all(destination)?;
                    continue;
                }
                let mut output = std::fs::File::create(destination)?;
                archive.extract(index, &limits, Some(opts), |bytes| {
                    std::io::Write::write_all(&mut output, bytes)?;
                    Ok(())
                })?;
            }
            resolved
        }
        Source::Path(_) => return Err(invalid()),
    };
    Ok((
        path,
        Some(Arc::new(Vista {
            _directory: directory,
            _spill: std::sync::Mutex::new(leases),
        })),
    ))
}

pub(crate) fn leggi_custodita(
    src: &Source,
    voce: &str,
    limiti: &LimitiArchivio,
    opts: &ReadOptions,
) -> Result<(Arc<[u8]>, plenora_io_model::budget::InternalMemoryLease)> {
    valida_nome(voce)?;
    let mut archive = Archive::open(src, limiti, Some(opts))?;
    let index = archive
        .entries
        .iter()
        .position(|e| key(&e.public.nome) == key(voce) && !e.public.directory)
        .ok_or_else(invalid)?;
    let size = archive.entries[index].public.byte_decompressi;
    let mut lease = opts
        .budget()
        .context()
        .lease_memory_internal(size.checked_mul(2).ok_or_else(limit)?.max(1))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(usize::try_from(size).map_err(|_| limit())?)
        .map_err(|_| limit())?;
    archive.extract(index, limiti, Some(opts), |bytes| {
        output.extend_from_slice(bytes);
        Ok(())
    })?;
    let data = Arc::from(output);
    lease.shrink_to(size.max(1))?;
    Ok((data, lease))
}

#[cfg(test)]
mod tests;

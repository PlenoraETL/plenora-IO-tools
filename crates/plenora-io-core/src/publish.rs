//! Publish atomico condiviso (`ENGINEERING.md § Pipeline di scrittura`).
//!
//! Profilo v1 di default: `AtomicPublish`; `durable` attiva
//! `DurableAtomicPublish`, che sincronizza file e directory dove la
//! piattaforma lo consente e segnala esplicitamente quando la durabilità del
//! nome pubblicato non può essere confermata.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use plenora_io_model::{NumeroStrutturale, PublicMessage};
use plenora_io_model::{PlenoraIoError, RemoteEffect, Result, RetryDisposition};
use tempfile::{NamedTempFile, TempDir};

/// Esito del publish (`ENGINEERING.md § Pipeline di scrittura`): un errore di `fsync` **dopo** il rename lascia
/// l'output già visibile ma senza conferma di durabilità.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishOutcome {
    Published,
    PublishedButDurabilityUnconfirmed,
}

/// Lifecycle comune di un output a file singolo.
///
/// Incapsula staging, destinazione, profilo durable e limite fisico. Il publish
/// è una transizione terminale: dopo il primo tentativo lo staging non può
/// essere riutilizzato o pubblicato una seconda volta.
pub struct StagedFile {
    temp: Option<NamedTempFile>,
    destination: PathBuf,
    durable: bool,
    max_output_bytes: u64,
}

impl StagedFile {
    /// Prepara uno staging file adiacente alla destinazione.
    ///
    /// # Errors
    ///
    /// Restituisce un errore di I/O se lo staging non è creabile nella
    /// directory di destinazione.
    pub fn new(destination: &Path, durable: bool, max_output_bytes: u64) -> Result<Self> {
        Ok(Self {
            temp: Some(create_staged_file(destination)?),
            destination: destination.to_owned(),
            durable,
            max_output_bytes,
        })
    }

    /// Come [`StagedFile::new`], conservando il suffisso richiesto dal
    /// formato.
    ///
    /// # Errors
    ///
    /// Restituisce un errore di I/O se lo staging non è creabile nella
    /// directory di destinazione.
    pub fn with_suffix(
        destination: &Path,
        suffix: &str,
        durable: bool,
        max_output_bytes: u64,
    ) -> Result<Self> {
        Ok(Self {
            temp: Some(create_staged_file_with_suffix(destination, suffix)?),
            destination: destination.to_owned(),
            durable,
            max_output_bytes,
        })
    }

    /// Percorso dello staging file.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Contract`] se lo staging è già stato
    /// consumato da [`StagedFile::publish`].
    pub fn path(&self) -> Result<&Path> {
        self.temp
            .as_ref()
            .map(NamedTempFile::path)
            .ok_or_else(Self::terminal_state_error)
    }

    /// Riapre lo staging file come handle indipendente.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Contract`] se lo staging è già stato
    /// consumato, o l'errore di I/O della riapertura.
    pub fn reopen(&self) -> Result<File> {
        Ok(self
            .temp
            .as_ref()
            .ok_or_else(Self::terminal_state_error)?
            .reopen()?)
    }

    /// Handle mutabile sullo staging file.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Contract`] se lo staging è già stato
    /// consumato da [`StagedFile::publish`].
    pub fn as_file_mut(&mut self) -> Result<&mut File> {
        Ok(self
            .temp
            .as_mut()
            .ok_or_else(Self::terminal_state_error)?
            .as_file_mut())
    }

    /// Transizione terminale: pubblica lo staging sulla destinazione.
    ///
    /// # Errors
    ///
    /// Restituisce [`PlenoraIoError::Contract`] se lo staging è già stato
    /// consumato, [`PlenoraIoError::LimitExceeded`] se l'output supera il
    /// limite fisico, [`PlenoraIoError::OutputExists`] se la destinazione
    /// esiste già, o l'errore di I/O di `fsync`/rename.
    pub fn publish(&mut self) -> Result<(u64, PublishOutcome)> {
        let temp = self.temp.take().ok_or_else(Self::terminal_state_error)?;
        publish_file_atomic_limited(temp, &self.destination, self.durable, self.max_output_bytes)
    }

    fn terminal_state_error() -> PlenoraIoError {
        PlenoraIoError::contratto_redatto(&PublicMessage::Curated(
            "staging file non disponibile dopo la transizione terminale",
        ))
    }
}

/// Crea uno staging file sullo stesso filesystem della destinazione.
///
/// Tutti i writer a file singolo devono passare da qui: in questo modo un
/// percorso relativo e uno assoluto risolvono il parent con la stessa semantica
/// e il successivo rename atomico non dipende dalla directory temporanea di
/// sistema.
///
/// # Errors
///
/// Restituisce un errore di I/O se il file temporaneo non è creabile nella
/// directory padre della destinazione.
pub fn create_staged_file(dest: &Path) -> Result<NamedTempFile> {
    Ok(NamedTempFile::new_in(destination_parent(dest))?)
}

/// Come [`create_staged_file`], mantenendo il suffisso richiesto da librerie che
/// riconoscono il formato dal nome del file temporaneo.
///
/// # Errors
///
/// Gli stessi di [`create_staged_file`].
pub fn create_staged_file_with_suffix(dest: &Path, suffix: &str) -> Result<NamedTempFile> {
    Ok(tempfile::Builder::new()
        .suffix(suffix)
        .tempfile_in(destination_parent(dest))?)
}

/// Crea una staging directory adiacente alla destinazione del dataset.
///
/// # Errors
///
/// Restituisce un errore di I/O se la directory temporanea non è creabile
/// nella directory padre della destinazione.
pub fn create_staged_dir(dest: &Path) -> Result<TempDir> {
    Ok(tempfile::Builder::new().tempdir_in(destination_parent(dest))?)
}

/// Pubblica un file singolo in modo atomico e no-clobber.
///
/// # Errors
///
/// Restituisce [`PlenoraIoError::OutputExists`] se la destinazione esiste già,
/// [`PlenoraIoError::Unsupported`] se staging e destinazione non sono sullo
/// stesso filesystem, o l'errore di I/O di `fsync`/rename.
pub fn publish_file_atomic(
    temp: NamedTempFile,
    dest: &Path,
    durable: bool,
) -> Result<(u64, PublishOutcome)> {
    ensure_destination_absent(dest)?;
    ensure_same_filesystem(temp.path(), destination_parent(dest))?;
    // 1. fsync del file, prima del rename.
    if durable {
        temp.as_file().sync_all()?;
    }
    let bytes = temp.as_file().metadata()?.len();
    // 3. rename atomico no-clobber.
    temp.persist_noclobber(dest)
        .map_err(|error| publish_rename_error(error.error, dest))?;
    // 4. fsync della directory padre, dopo il rename.
    Ok((bytes, finalize_durability(dest, durable, true)))
}

/// Variante bounded: verifica la dimensione del tempfile prima del rename, così
/// un superamento non rende mai visibile l'output.
///
/// # Errors
///
/// Restituisce [`PlenoraIoError::LimitExceeded`] se lo staging supera
/// `max_output_bytes`; per il resto gli stessi errori di
/// [`publish_file_atomic`].
pub fn publish_file_atomic_limited(
    temp: NamedTempFile,
    dest: &Path,
    durable: bool,
    max_output_bytes: u64,
) -> Result<(u64, PublishOutcome)> {
    let bytes = temp.as_file().metadata()?.len();
    if bytes > max_output_bytes {
        return Err(PlenoraIoError::limite_redatto(
            &PublicMessage::CuratedBetween(
                "output da",
                NumeroStrutturale::Conteggio(bytes),
                "byte oltre il limite di",
                NumeroStrutturale::Limite(max_output_bytes),
            ),
        ));
    }
    publish_file_atomic(temp, dest, durable)
}

/// Pubblica una directory-dataset (multi-file / multi-layer) con un unico rename
/// atomico (staging dir -> destinazione), sullo stesso filesystem.
///
/// # Errors
///
/// Restituisce [`PlenoraIoError::OutputExists`] se la destinazione esiste già,
/// [`PlenoraIoError::Unsupported`] se staging e destinazione non sono sullo
/// stesso filesystem o se il tree di staging contiene voci non regolari
/// (symlink), o l'errore di I/O di `fsync`/rename.
pub fn publish_dir_atomic(staging: &Path, dest: &Path, durable: bool) -> Result<PublishOutcome> {
    ensure_destination_absent(dest)?;
    ensure_same_filesystem(staging, destination_parent(dest))?;
    // La validazione dell'intero tree (incluso il rifiuto dei symlink) è
    // indipendente da `durable`; in quel profilo sincronizza anche ciò che la
    // piattaforma permette e conserva se le directory non sono confermabili.
    let staging_durability_confirmed = prepare_tree(staging, durable)?;
    // 3. rename atomico e autorevolmente no-clobber.
    rename_noclobber(staging, dest)?;
    // 4. fsync della directory padre, dopo il rename.
    Ok(finalize_durability(
        dest,
        durable,
        staging_durability_confirmed,
    ))
}

/// Pubblica un set di file sciolti nell'ordine fornito.
///
/// La modalità è deliberatamente più debole del rename di directory: i
/// companion possono diventare visibili uno alla volta, quindi il marker
/// principale va passato per ultimo. Tutti i controlli e gli `fsync`
/// pre-publish avvengono prima del primo rename.
///
/// # Errors
///
/// Restituisce [`PlenoraIoError::Unsupported`] se il set è vuoto, se non usa
/// una sola staging e una sola destinazione, se un file di staging non è
/// regolare o se staging e destinazione non sono sullo stesso filesystem;
/// [`PlenoraIoError::LimitExceeded`] se i byte totali superano il limite o
/// vanno in overflow; [`PlenoraIoError::OutputExists`] se una destinazione
/// esiste già; l'errore di I/O di `fsync`/rename.
pub fn publish_files_ordered_limited(
    files: &[(PathBuf, PathBuf)],
    durable: bool,
    max_output_bytes: u64,
) -> Result<(u64, PublishOutcome)> {
    let Some((first_source, first_destination)) = files.first() else {
        return Err(PlenoraIoError::non_supportato_redatto(
            &PublicMessage::Curated("set di publish vuoto"),
        ));
    };
    let source_parent_path = first_source.parent();
    let destination_parent_path = first_destination.parent();
    let mut bytes = 0_u64;

    // Preflight completo prima di rendere visibile qualunque companion.
    for (source, destination) in files {
        if source.parent() != source_parent_path || destination.parent() != destination_parent_path
        {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated(
                    "il set di publish deve usare una sola staging e una sola destinazione",
                ),
            ));
        }
        let metadata = std::fs::symlink_metadata(source)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("file di staging non regolare"),
            ));
        }
        ensure_destination_absent(destination)?;
        bytes = bytes.checked_add(metadata.len()).ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated(
                "overflow nel conteggio dell'output",
            ))
        })?;
    }
    if bytes > max_output_bytes {
        return Err(PlenoraIoError::limite_redatto(
            &PublicMessage::CuratedBetween(
                "output da",
                NumeroStrutturale::Conteggio(bytes),
                "byte oltre il limite di",
                NumeroStrutturale::Limite(max_output_bytes),
            ),
        ));
    }
    ensure_same_filesystem(first_source, destination_parent(first_destination))?;

    let staging_durability_confirmed = if durable {
        for (source, _) in files {
            sync_file(source)?;
        }
        sync_dir(source_parent_path.unwrap_or_else(|| Path::new(".")))?
    } else {
        true
    };

    commit_ordered_renames(files, rollback_rename)?;
    Ok((
        bytes,
        finalize_durability(first_destination, durable, staging_durability_confirmed),
    ))
}

/// Il rename di rollback in produzione.
///
/// Volutamente **non** no-clobber: rimette il file al proprio staging, che e'
/// una posizione che nessun altro vede, quindi la simmetrica riporta il file
/// da dove era partito senza mai sovrascrivere qualcosa «in uso».
fn rollback_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

/// I rename, in ordine, con rollback dei companion gia' spostati.
///
/// # Perche' il rollback arriva da un parametro
///
/// Il set loose **non e' crash-atomic** — per quello esiste il
/// directory-dataset — ma un errore osservabile *durante* il publish produce un
/// tentativo di rollback, e l'esito di quel tentativo cambia cio' che l'errore
/// dichiara al chiamante:
///
/// * `RemoteEffect::None` — rollback completo, nessun companion visibile;
/// * `RemoteEffect::Partial` con `RetryDisposition::RequiresRecovery` — il
///   rollback e' fallito su almeno un file: il filesystem **puo'** contenere
///   companion pubblicati, e una retry cieca non e' sicura.
///
/// Il secondo ramo era, fino a questa revisione, provato soltanto su un errore
/// sintetico: la sonda costruiva un `PlenoraIoError` e gli applicava
/// `with_effect`, cioe' verificava che `with_effect` faccia il proprio lavoro —
/// non che questo ciclo lo chiami, non che lo chiami nel caso giusto, e
/// soprattutto non che l'affermazione «puo' restare visibile» sia **vera**. Un
/// rollback che fallisce non si ottiene da un filesystem reale con i permessi:
/// avanti e indietro attraversano le stesse due directory, quindi togliere il
/// diritto al ritorno lo toglie anche all'andata.
///
/// Il parametro e' il seam minimo che rende quel ramo osservabile per intero,
/// destinazione rimasta sul disco compresa. La produzione passa sempre
/// [`rollback_rename`].
fn commit_ordered_renames(
    files: &[(PathBuf, PathBuf)],
    rollback: fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    let mut committed: Vec<(&PathBuf, &PathBuf)> = Vec::with_capacity(files.len());
    for (source, destination) in files {
        if let Err(error) = rename_noclobber(source, destination) {
            let mut rollback_failed = false;
            for (committed_source, committed_destination) in committed.iter().rev() {
                if rollback(committed_destination, committed_source).is_err() {
                    rollback_failed = true;
                }
            }
            return Err(if rollback_failed {
                error.with_effect(RemoteEffect::Partial, RetryDisposition::RequiresRecovery)
            } else {
                error
            });
        }
        committed.push((source, destination));
    }
    Ok(())
}

fn destination_parent(dest: &Path) -> &Path {
    dest.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn ensure_destination_absent(dest: &Path) -> Result<()> {
    match std::fs::symlink_metadata(dest) {
        Ok(_) => Err(PlenoraIoError::destinazione_esistente()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PlenoraIoError::Io(error)),
    }
}

fn publish_rename_error(error: std::io::Error, dest: &Path) -> PlenoraIoError {
    if error.kind() == std::io::ErrorKind::AlreadyExists || std::fs::symlink_metadata(dest).is_ok()
    {
        PlenoraIoError::destinazione_esistente()
    } else {
        PlenoraIoError::Io(error)
    }
}

fn rename_noclobber(source: &Path, destination: &Path) -> Result<()> {
    rename_noclobber_os(source, destination)
        .map_err(|error| publish_rename_error(error, destination))
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn rename_noclobber_os(source: &Path, destination: &Path) -> std::io::Result<()> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};

    // Linux/Android usano renameat2(RENAME_NOREPLACE); sulle piattaforme Apple
    // rustix traduce la stessa API in renameatx_np(RENAME_EXCL). Entrambe le
    // primitive sono atomiche anche per directory e rifiutano un nome apparso
    // dopo il preflight.
    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE).map_err(Into::into)
}

#[cfg(windows)]
fn rename_noclobber_os(source: &Path, destination: &Path) -> std::io::Result<()> {
    atomicwrites::move_atomic(source, destination)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
)))]
fn rename_noclobber_os(source: &Path, destination: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "publish directory no-clobber non supportato su questa piattaforma",
        ));
    }
    std::fs::hard_link(source, destination)?;
    std::fs::remove_file(source)
}

fn ensure_same_filesystem(staging: &Path, destination_parent: &Path) -> Result<()> {
    if same_filesystem(staging, destination_parent)? {
        return Ok(());
    }
    Err(PlenoraIoError::non_supportato_redatto(
        &PublicMessage::Curated(
            "publish cross-filesystem vietato: staging e destinazione sono su filesystem diversi",
        ),
    ))
}

#[cfg(unix)]
fn same_filesystem(left: &Path, right: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    Ok(std::fs::metadata(left)?.dev() == std::fs::metadata(right)?.dev())
}

#[cfg(windows)]
fn same_filesystem(left: &Path, right: &Path) -> std::io::Result<bool> {
    Ok(windows_volume_root(left)? == windows_volume_root(right)?)
}

#[cfg(windows)]
fn windows_volume_root(path: &Path) -> std::io::Result<String> {
    use std::path::Component;

    let canonical = std::fs::canonicalize(path)?;
    match canonical.components().next() {
        Some(Component::Prefix(prefix)) => Ok(prefix.as_os_str().to_string_lossy().to_lowercase()),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "percorso Windows senza volume",
        )),
    }
}

#[cfg(not(any(unix, windows)))]
fn same_filesystem(_left: &Path, _right: &Path) -> std::io::Result<bool> {
    // Non esiste un identificatore portabile del filesystem: su piattaforme
    // diverse da Unix/Windows resta autorevole il fallimento atomico di rename.
    Ok(true)
}

fn prepare_tree(path: &Path, durable: bool) -> std::io::Result<bool> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "symlink nella staging",
        ));
    }
    if metadata.is_file() {
        if durable {
            sync_file(path)?;
        }
        return Ok(true);
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "elemento staging non regolare",
        ));
    }
    let mut durability_confirmed = true;
    for entry in std::fs::read_dir(path)? {
        durability_confirmed &= prepare_tree(&entry?.path(), durable)?;
    }
    if durable {
        durability_confirmed &= sync_dir(path)?;
    }
    Ok(durability_confirmed)
}

fn sync_file(path: &Path) -> std::io::Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()
}

fn finalize_durability(
    dest: &Path,
    durable: bool,
    staging_durability_confirmed: bool,
) -> PublishOutcome {
    if !durable {
        return PublishOutcome::Published;
    }
    match sync_dir(destination_parent(dest)) {
        Ok(true) if staging_durability_confirmed => PublishOutcome::Published,
        Ok(_) | Err(_) => PublishOutcome::PublishedButDurabilityUnconfirmed,
    }
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<bool> {
    std::fs::File::open(dir)?.sync_all()?;
    Ok(true)
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> std::io::Result<bool> {
    // Il fsync di directory non è disponibile in modo portabile su Windows:
    // il publish prosegue ma l'esito deve restare non confermato.
    Ok(false)
}

#[cfg(test)]
mod tests;

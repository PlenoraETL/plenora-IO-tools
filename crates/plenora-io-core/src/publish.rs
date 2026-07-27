//! Publish atomico condiviso (ADR-IO 2). Profilo v1 di default: `AtomicPublish`;
//! `durable` attiva `DurableAtomicPublish`: sincronizza file e directory dove
//! la piattaforma lo consente e segnala esplicitamente quando la durabilità del
//! nome pubblicato non può essere confermata.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use plenora_io_model::{PlenoraIoError, Result};
use tempfile::{NamedTempFile, TempDir};

/// Esito del publish (ADR-IO 2): un errore di `fsync` **dopo** il rename lascia
/// l'output già visibile ma senza conferma di durabilità.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishOutcome {
    Published,
    PublishedButDurabilityUnconfirmed,
}

/// Crea uno staging file sullo stesso filesystem della destinazione.
///
/// Tutti i writer a file singolo devono passare da qui: in questo modo un
/// percorso relativo e uno assoluto risolvono il parent con la stessa semantica
/// e il successivo rename atomico non dipende dalla directory temporanea di
/// sistema.
pub fn create_staged_file(dest: &Path) -> Result<NamedTempFile> {
    Ok(NamedTempFile::new_in(destination_parent(dest))?)
}

/// Come [`create_staged_file`], mantenendo il suffisso richiesto da librerie che
/// riconoscono il formato dal nome del file temporaneo.
pub fn create_staged_file_with_suffix(dest: &Path, suffix: &str) -> Result<NamedTempFile> {
    Ok(tempfile::Builder::new()
        .suffix(suffix)
        .tempfile_in(destination_parent(dest))?)
}

/// Crea una staging directory adiacente alla destinazione del dataset.
pub fn create_staged_dir(dest: &Path) -> Result<TempDir> {
    Ok(tempfile::Builder::new().tempdir_in(destination_parent(dest))?)
}

/// Pubblica un file singolo in modo atomico e no-clobber.
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
pub fn publish_file_atomic_limited(
    temp: NamedTempFile,
    dest: &Path,
    durable: bool,
    max_output_bytes: u64,
) -> Result<(u64, PublishOutcome)> {
    let bytes = temp.as_file().metadata()?.len();
    if bytes > max_output_bytes {
        return Err(PlenoraIoError::LimitExceeded(format!(
            "output da {bytes} byte oltre il limite di {max_output_bytes}"
        )));
    }
    publish_file_atomic(temp, dest, durable)
}

/// Pubblica una directory-dataset (multi-file / multi-layer) con un unico rename
/// atomico (staging dir -> destinazione), sullo stesso filesystem.
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

/// Pubblica un set di file sciolti nell'ordine fornito. La modalità è
/// deliberatamente più debole del rename di directory: i companion possono
/// diventare visibili uno alla volta, quindi il marker principale va passato
/// per ultimo. Tutti i controlli e gli `fsync` pre-publish avvengono prima del
/// primo rename.
pub fn publish_files_ordered_limited(
    files: &[(PathBuf, PathBuf)],
    durable: bool,
    max_output_bytes: u64,
) -> Result<(u64, PublishOutcome)> {
    let Some((first_source, first_destination)) = files.first() else {
        return Err(PlenoraIoError::Unsupported(
            "set di publish vuoto".to_owned(),
        ));
    };
    let source_parent_path = first_source.parent();
    let destination_parent_path = first_destination.parent();
    let mut bytes = 0_u64;

    // Preflight completo prima di rendere visibile qualunque companion.
    for (source, destination) in files {
        if source.parent() != source_parent_path || destination.parent() != destination_parent_path
        {
            return Err(PlenoraIoError::Unsupported(
                "il set di publish deve usare una sola staging e una sola destinazione".to_owned(),
            ));
        }
        let metadata = std::fs::symlink_metadata(source)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(PlenoraIoError::Unsupported(format!(
                "file di staging non regolare: {}",
                source.display()
            )));
        }
        ensure_destination_absent(destination)?;
        bytes = bytes.checked_add(metadata.len()).ok_or_else(|| {
            PlenoraIoError::LimitExceeded("overflow nel conteggio dell'output".to_owned())
        })?;
    }
    if bytes > max_output_bytes {
        return Err(PlenoraIoError::LimitExceeded(format!(
            "output da {bytes} byte oltre il limite di {max_output_bytes}"
        )));
    }
    ensure_same_filesystem(first_source, destination_parent(first_destination))?;

    let mut staging_durability_confirmed = true;
    if durable {
        for (source, _) in files {
            sync_file(source)?;
        }
        staging_durability_confirmed =
            sync_dir(source_parent_path.unwrap_or_else(|| Path::new(".")))?;
    }

    for (source, destination) in files {
        rename_noclobber(source, destination)?;
    }
    Ok((
        bytes,
        finalize_durability(first_destination, durable, staging_durability_confirmed),
    ))
}

fn destination_parent(dest: &Path) -> &Path {
    dest.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn ensure_destination_absent(dest: &Path) -> Result<()> {
    match std::fs::symlink_metadata(dest) {
        Ok(_) => Err(PlenoraIoError::OutputExists(dest.display().to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PlenoraIoError::Io(error)),
    }
}

fn publish_rename_error(error: std::io::Error, dest: &Path) -> PlenoraIoError {
    if error.kind() == std::io::ErrorKind::AlreadyExists || std::fs::symlink_metadata(dest).is_ok()
    {
        PlenoraIoError::OutputExists(dest.display().to_string())
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
    Err(PlenoraIoError::Unsupported(format!(
        "publish cross-filesystem vietato: staging '{}' e destinazione '{}' sono su filesystem diversi",
        staging.display(),
        destination_parent.display()
    )))
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
            format!("percorso Windows senza volume: {}", canonical.display()),
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
            format!("symlink nella staging: {}", path.display()),
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
            format!("elemento staging non regolare: {}", path.display()),
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
mod tests {
    use std::io::Write;

    use super::*;

    fn expected_durable_outcome() -> PublishOutcome {
        if cfg!(unix) {
            PublishOutcome::Published
        } else {
            PublishOutcome::PublishedButDurabilityUnconfirmed
        }
    }

    #[test]
    fn staging_helpers_use_destination_parent_and_requested_suffix() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("dataset.gpkg");
        let file = create_staged_file_with_suffix(&destination, ".gpkg").unwrap();
        assert_eq!(file.path().parent(), Some(directory.path()));
        assert_eq!(
            file.path().extension().and_then(|value| value.to_str()),
            Some("gpkg")
        );

        let staging = create_staged_dir(&destination).unwrap();
        assert_eq!(staging.path().parent(), Some(directory.path()));
    }

    #[test]
    fn output_limit_is_checked_before_publish() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("output.bin");
        let mut temp = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        temp.write_all(&[0_u8; 8]).unwrap();
        let result = publish_file_atomic_limited(temp, &destination, false, 7);
        assert!(matches!(result, Err(PlenoraIoError::LimitExceeded(_))));
        assert!(!destination.exists());
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        windows
    ))]
    #[test]
    fn directory_dataset_is_published_with_one_rename() {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
        std::fs::write(staging.path().join("data.shp"), b"shape").unwrap();
        std::fs::create_dir(staging.path().join("nested")).unwrap();
        std::fs::write(staging.path().join("nested").join("index"), b"index").unwrap();
        let destination = root.path().join("dataset.shp.d");

        let outcome = publish_dir_atomic(staging.path(), &destination, true).unwrap();

        assert_eq!(outcome, expected_durable_outcome());
        assert_eq!(
            std::fs::read(destination.join("data.shp")).unwrap(),
            b"shape"
        );
        assert_eq!(
            std::fs::read(destination.join("nested").join("index")).unwrap(),
            b"index"
        );
    }

    #[test]
    fn directory_publish_is_no_clobber() {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
        std::fs::write(staging.path().join("data.shp"), b"new").unwrap();
        let destination = root.path().join("dataset.shp.d");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("sentinel"), b"existing").unwrap();

        let result = publish_dir_atomic(staging.path(), &destination, false);

        assert!(matches!(result, Err(PlenoraIoError::OutputExists(_))));
        assert_eq!(
            std::fs::read(destination.join("sentinel")).unwrap(),
            b"existing"
        );
        assert!(!destination.join("data.shp").exists());
    }

    #[test]
    fn atomic_noclobber_refuses_a_file_created_after_preflight() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("staged.dbf");
        let destination = root.path().join("dataset.dbf");
        std::fs::write(&source, b"new").unwrap();
        ensure_destination_absent(&destination).unwrap();

        std::fs::write(&destination, b"concurrent").unwrap();
        let result = rename_noclobber(&source, &destination);

        assert!(matches!(result, Err(PlenoraIoError::OutputExists(_))));
        assert_eq!(std::fs::read(&destination).unwrap(), b"concurrent");
        assert_eq!(std::fs::read(&source).unwrap(), b"new");
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_vendor = "apple",
        windows
    ))]
    #[test]
    fn atomic_noclobber_refuses_a_directory_created_after_preflight() {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
        std::fs::write(staging.path().join("data"), b"new").unwrap();
        let destination = root.path().join("dataset");
        ensure_destination_absent(&destination).unwrap();

        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("sentinel"), b"concurrent").unwrap();
        let result = rename_noclobber(staging.path(), &destination);

        assert!(matches!(result, Err(PlenoraIoError::OutputExists(_))));
        assert_eq!(
            std::fs::read(destination.join("sentinel")).unwrap(),
            b"concurrent"
        );
        assert_eq!(std::fs::read(staging.path().join("data")).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn directory_publish_rejects_symlinks_even_when_not_durable() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
        let target = root.path().join("outside");
        std::fs::write(&target, b"outside").unwrap();
        symlink(&target, staging.path().join("link")).unwrap();
        let destination = root.path().join("dataset");

        let result = publish_dir_atomic(staging.path(), &destination, false);

        assert!(matches!(
            result,
            Err(PlenoraIoError::Io(error))
                if error.kind() == std::io::ErrorKind::InvalidInput
        ));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(target).unwrap(), b"outside");
    }

    #[test]
    fn loose_set_preflight_fails_before_first_rename() {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
        let first_source = staging.path().join("data.dbf");
        std::fs::write(&first_source, b"dbf").unwrap();
        let files = vec![
            (first_source, root.path().join("data.dbf")),
            (
                staging.path().join("missing.shp"),
                root.path().join("data.shp"),
            ),
        ];

        assert!(publish_files_ordered_limited(&files, false, u64::MAX).is_err());
        assert!(!root.path().join("data.dbf").exists());
        assert!(!root.path().join("data.shp").exists());
    }

    #[test]
    fn loose_set_durable_publish_preserves_ordered_files() {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
        let source_dbf = staging.path().join("data.dbf");
        let source_shp = staging.path().join("data.shp");
        std::fs::write(&source_dbf, b"dbf").unwrap();
        std::fs::write(&source_shp, b"shape").unwrap();
        let destination_dbf = root.path().join("dataset.dbf");
        let destination_shp = root.path().join("dataset.shp");
        let files = vec![
            (source_dbf, destination_dbf.clone()),
            (source_shp, destination_shp.clone()),
        ];

        let (bytes, outcome) = publish_files_ordered_limited(&files, true, u64::MAX).unwrap();

        assert_eq!(bytes, 8);
        assert_eq!(outcome, expected_durable_outcome());
        assert_eq!(std::fs::read(destination_dbf).unwrap(), b"dbf");
        assert_eq!(std::fs::read(destination_shp).unwrap(), b"shape");
    }

    #[cfg(windows)]
    #[test]
    fn durable_file_publish_reports_unconfirmed_parent_directory() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("output.bin");
        let mut temp = NamedTempFile::new_in(root.path()).unwrap();
        temp.write_all(b"durable").unwrap();

        let (_, outcome) = publish_file_atomic(temp, &destination, true).unwrap();

        assert_eq!(outcome, PublishOutcome::PublishedButDurabilityUnconfirmed);
        assert_eq!(std::fs::read(destination).unwrap(), b"durable");
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn cross_filesystem_publish_is_rejected_before_any_output_is_visible() {
        let Some(cross_root) = std::env::var_os("PLENORA_CROSS_FS_TEST_ROOT") else {
            return;
        };
        let source_root = tempfile::tempdir_in(cross_root).unwrap();
        let destination_root = tempfile::tempdir().unwrap();
        assert!(
            !same_filesystem(source_root.path(), destination_root.path()).unwrap(),
            "PLENORA_CROSS_FS_TEST_ROOT deve indicare un filesystem distinto"
        );

        let staging = tempfile::Builder::new()
            .prefix("directory-")
            .tempdir_in(source_root.path())
            .unwrap();
        std::fs::write(staging.path().join("data"), b"directory").unwrap();
        let directory_destination = destination_root.path().join("dataset");
        let directory_result = publish_dir_atomic(staging.path(), &directory_destination, false);
        assert!(matches!(
            directory_result,
            Err(PlenoraIoError::Unsupported(message))
                if message.contains("cross-filesystem")
        ));
        assert!(staging.path().join("data").exists());
        assert!(!directory_destination.exists());

        let mut temp = NamedTempFile::new_in(source_root.path()).unwrap();
        temp.write_all(b"single-file").unwrap();
        let file_destination = destination_root.path().join("output.bin");
        let file_result = publish_file_atomic(temp, &file_destination, false);
        assert!(matches!(
            file_result,
            Err(PlenoraIoError::Unsupported(message))
                if message.contains("cross-filesystem")
        ));
        assert!(!file_destination.exists());

        let loose_source = source_root.path().join("data.shp");
        std::fs::write(&loose_source, b"shape").unwrap();
        let loose_destination = destination_root.path().join("data.shp");
        let loose_result = publish_files_ordered_limited(
            &[(loose_source.clone(), loose_destination.clone())],
            false,
            u64::MAX,
        );
        assert!(matches!(
            loose_result,
            Err(PlenoraIoError::Unsupported(message))
                if message.contains("cross-filesystem")
        ));
        assert!(loose_source.exists());
        assert!(!loose_destination.exists());
    }
}

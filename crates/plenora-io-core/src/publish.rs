//! Publish atomico condiviso (ADR-IO 2). Profilo v1 di default: `AtomicPublish`;
//! `durable` attiva `DurableAtomicPublish` con la sequenza fsync completa:
//! fsync file/staging -> rename -> fsync directory padre della destinazione.

use std::fs::File;
use std::path::{Path, PathBuf};

use plenora_core::{PlenoraError, Result};
use tempfile::NamedTempFile;

/// Esito del publish (ADR-IO 2): un errore di `fsync` **dopo** il rename lascia
/// l'output già visibile ma senza conferma di durabilità.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishOutcome {
    Published,
    PublishedButDurabilityUnconfirmed,
}

/// Pubblica un file singolo in modo atomico e no-clobber.
pub fn publish_file_atomic(
    temp: NamedTempFile,
    dest: &Path,
    durable: bool,
) -> Result<(u64, PublishOutcome)> {
    if dest.exists() {
        return Err(PlenoraError::OutputExists(dest.display().to_string()));
    }
    // 1. fsync del file, prima del rename.
    if durable {
        temp.as_file().sync_all()?;
    }
    let bytes = temp.as_file().metadata()?.len();
    // 3. rename atomico no-clobber.
    temp.persist_noclobber(dest)
        .map_err(|e| PlenoraError::Io(e.error))?;
    // 4. fsync della directory padre, dopo il rename.
    Ok((bytes, finalize_durability(dest, durable)))
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
        return Err(PlenoraError::LimitExceeded(format!(
            "output da {bytes} byte oltre il limite di {max_output_bytes}"
        )));
    }
    publish_file_atomic(temp, dest, durable)
}

/// Pubblica una directory-dataset (multi-file / multi-layer) con un unico rename
/// atomico (staging dir -> destinazione), sullo stesso filesystem.
pub fn publish_dir_atomic(staging: &Path, dest: &Path, durable: bool) -> Result<PublishOutcome> {
    if dest.exists() {
        return Err(PlenoraError::OutputExists(dest.display().to_string()));
    }
    // 1-2. fsync di tutti i file e delle directory della staging prima del
    // rename. Qualunque errore pre-publish è fail-closed.
    if durable {
        sync_tree(staging)?;
    }
    // 3. rename.
    std::fs::rename(staging, dest)?;
    // 4. fsync della directory padre, dopo il rename.
    Ok(finalize_durability(dest, durable))
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
        return Err(PlenoraError::Unsupported("set di publish vuoto".to_owned()));
    };
    let source_parent = first_source.parent();
    let destination_parent = first_destination.parent();
    let mut bytes = 0_u64;

    // Preflight completo prima di rendere visibile qualunque companion.
    for (source, destination) in files {
        if source.parent() != source_parent || destination.parent() != destination_parent {
            return Err(PlenoraError::Unsupported(
                "il set di publish deve usare una sola staging e una sola destinazione".to_owned(),
            ));
        }
        let metadata = std::fs::symlink_metadata(source)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(PlenoraError::Unsupported(format!(
                "file di staging non regolare: {}",
                source.display()
            )));
        }
        if destination.exists() {
            return Err(PlenoraError::OutputExists(
                destination.display().to_string(),
            ));
        }
        bytes = bytes.checked_add(metadata.len()).ok_or_else(|| {
            PlenoraError::LimitExceeded("overflow nel conteggio dell'output".to_owned())
        })?;
    }
    if bytes > max_output_bytes {
        return Err(PlenoraError::LimitExceeded(format!(
            "output da {bytes} byte oltre il limite di {max_output_bytes}"
        )));
    }

    if durable {
        for (source, _) in files {
            File::open(source)?.sync_all()?;
        }
        if let Some(staging) = source_parent {
            fsync_dir(staging)?;
        }
    }

    for (source, destination) in files {
        std::fs::rename(source, destination)?;
    }
    Ok((bytes, finalize_durability(first_destination, durable)))
}

fn sync_tree(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("symlink nella staging: {}", path.display()),
        ));
    }
    if metadata.is_file() {
        return File::open(path)?.sync_all();
    }
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("elemento staging non regolare: {}", path.display()),
        ));
    }
    for entry in std::fs::read_dir(path)? {
        sync_tree(&entry?.path())?;
    }
    fsync_dir(path)
}

fn finalize_durability(dest: &Path, durable: bool) -> PublishOutcome {
    if !durable {
        return PublishOutcome::Published;
    }
    match dest.parent().map(fsync_dir).unwrap_or(Ok(())) {
        Ok(()) => PublishOutcome::Published,
        Err(_) => PublishOutcome::PublishedButDurabilityUnconfirmed,
    }
}

#[cfg(unix)]
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn fsync_dir(_dir: &Path) -> std::io::Result<()> {
    // Il fsync di directory non è disponibile in modo portabile su Windows:
    // la durabilità del nome si affida alle garanzie del filesystem.
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn output_limit_is_checked_before_publish() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("output.bin");
        let mut temp = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        temp.write_all(&[0_u8; 8]).unwrap();
        let result = publish_file_atomic_limited(temp, &destination, false, 7);
        assert!(matches!(result, Err(PlenoraError::LimitExceeded(_))));
        assert!(!destination.exists());
    }

    #[test]
    fn directory_dataset_is_published_with_one_rename() {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::Builder::new().tempdir_in(root.path()).unwrap();
        std::fs::write(staging.path().join("data.shp"), b"shape").unwrap();
        std::fs::create_dir(staging.path().join("nested")).unwrap();
        std::fs::write(staging.path().join("nested").join("index"), b"index").unwrap();
        let destination = root.path().join("dataset.shp.d");

        let outcome = publish_dir_atomic(staging.path(), &destination, true).unwrap();

        assert_eq!(outcome, PublishOutcome::Published);
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

        assert!(matches!(result, Err(PlenoraError::OutputExists(_))));
        assert_eq!(
            std::fs::read(destination.join("sentinel")).unwrap(),
            b"existing"
        );
        assert!(!destination.join("data.shp").exists());
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
        assert_eq!(outcome, PublishOutcome::Published);
        assert_eq!(std::fs::read(destination_dbf).unwrap(), b"dbf");
        assert_eq!(std::fs::read(destination_shp).unwrap(), b"shape");
    }
}

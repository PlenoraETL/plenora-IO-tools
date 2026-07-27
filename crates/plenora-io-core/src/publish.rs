//! Publish atomico condiviso (ADR-IO 2). Profilo v1 di default: `AtomicPublish`;
//! `durable` attiva `DurableAtomicPublish` con la sequenza fsync completa:
//! fsync file/staging -> rename -> fsync directory padre della destinazione.

use std::path::Path;

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
/// atomico (staging dir -> destinazione), sullo stesso filesystem. I singoli
/// file nella staging sono già stati fsyncati dal driver (passo 1).
pub fn publish_dir_atomic(staging: &Path, dest: &Path, durable: bool) -> Result<PublishOutcome> {
    if dest.exists() {
        return Err(PlenoraError::OutputExists(dest.display().to_string()));
    }
    // 2. fsync della staging directory, prima del rename.
    if durable {
        let _ = fsync_dir(staging);
    }
    // 3. rename.
    std::fs::rename(staging, dest)?;
    // 4. fsync della directory padre, dopo il rename.
    Ok(finalize_durability(dest, durable))
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
}

//! Un file logico riapribile, su disco o in memoria, con ownership della quota.
use crate::{ReadOptions, Source};
use plenora_io_model::budget::{InternalMemoryLease, SourceEntry};
use plenora_io_model::{PlenoraIoError, PublicMessage, Result};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct SourceData {
    nome: PathBuf,
    dati: Option<Arc<[u8]>>,
    _lease: Option<Arc<Mutex<InternalMemoryLease>>>,
}
impl std::ops::Deref for SourceData {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.nome
    }
}
impl From<PathBuf> for SourceData {
    fn from(nome: PathBuf) -> Self {
        Self {
            nome,
            dati: None,
            _lease: None,
        }
    }
}
impl From<&Path> for SourceData {
    fn from(nome: &Path) -> Self {
        nome.to_owned().into()
    }
}
impl From<&PathBuf> for SourceData {
    fn from(nome: &PathBuf) -> Self {
        nome.clone().into()
    }
}
impl From<&Self> for SourceData {
    fn from(source: &Self) -> Self {
        source.clone()
    }
}
impl SourceData {
    /// # Errors
    /// Errore I/O sul file fisico.
    pub fn open(&self) -> std::io::Result<SourceReader> {
        Ok(match &self.dati {
            Some(dati) => SourceReader::Memory(std::io::Cursor::new(Arc::clone(dati))),
            None => SourceReader::File(std::fs::File::open(&self.nome)?),
        })
    }
    /// # Errors
    /// Errore I/O sui metadati fisici.
    pub fn length(&self) -> std::io::Result<u64> {
        self.dati.as_ref().map_or_else(
            || std::fs::metadata(&self.nome).map(|m| m.len()),
            |d| Ok(d.len() as u64),
        )
    }
    #[must_use]
    pub const fn in_memory(&self) -> bool {
        self.dati.is_some()
    }
}
pub enum SourceReader {
    File(std::fs::File),
    Memory(std::io::Cursor<Arc<[u8]>>),
}
impl Read for SourceReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::File(file) => file.read(bytes),
            Self::Memory(data) => data.read(bytes),
        }
    }
}
impl Seek for SourceReader {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::File(file) => file.seek(position),
            Self::Memory(data) => data.seek(position),
        }
    }
}
impl Source {
    /// Preflight dei file singoli, senza materializzare le sorgenti in memoria.
    /// # Errors
    /// Errori di struttura, archivio, quota, cancellazione e I/O.
    pub fn into_data_observed(
        self,
        descriptor: &crate::FormatDescriptor,
        opts: &mut ReadOptions,
        validate: impl Fn(&SourceData, &ReadOptions) -> Result<()>,
    ) -> Result<SourceData> {
        if let Self::Path(_) = &self {
            let path = crate::driver::preflight_source_validated(
                descriptor,
                self,
                opts,
                &|path, opts| validate(&path.into(), opts),
            )?;
            return Ok(path.into());
        }
        opts.ensure_active()?;
        let (nome, dati, lease) = match self {
            Self::Bytes { nome, dati } => {
                crate::archivio::valida_nome(&nome)?;
                let lease = opts
                    .budget()
                    .context()
                    .lease_memory_internal((dati.len() as u64).max(1))?;
                (nome, dati, lease)
            }
            Self::Archivio { archivio, voce } => {
                let (bytes, lease) = crate::archivio::leggi_custodita(
                    &archivio,
                    &voce,
                    &opts.limiti_archivio,
                    opts,
                )?;
                (voce, bytes, lease)
            }
            Self::Path(_) => {
                return Err(PlenoraIoError::non_supportato_redatto(
                    &PublicMessage::Curated("sorgente non valida"),
                ))
            }
        };
        let permit = opts.take_input_permit().ok_or_else(|| {
            PlenoraIoError::limite_redatto(&PublicMessage::Curated("permit input gia' speso"))
        })?;
        opts.budget()
            .context()
            .note_entry_visited(&SourceEntry::file(nome.as_bytes(), dati.len() as u64, None))?;
        opts.budget().context().observe_input(permit)?;
        let prepared = SourceData {
            nome: nome.into(),
            dati: Some(dati),
            _lease: Some(Arc::new(Mutex::new(lease))),
        };
        validate(&prepared, opts)?;
        plenora_io_model::format_options::valida_opzioni(
            descriptor.id(),
            descriptor.format_options(),
            &opts.format_options,
            plenora_io_model::format_options::FaseOpzione::Lettura,
        )?;
        Ok(prepared)
    }
}

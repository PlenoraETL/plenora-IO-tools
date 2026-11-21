//! Le sorgenti che non sono un percorso, senza rompere [`Source`].
//!
//! `Source` resta l'enum di sempre, con la sola variante `Path`: aggiungere
//! varianti a un enum esaustivo romperebbe ogni `match` e ogni `let
//! Source::Path(p) = s` dei consumatori. Il nome logico viaggia in
//! `Source::Path`; i byte dietro quel nome viaggiano nelle opzioni di lettura,
//! dove li mette [`Sorgente::per_lettura`] e da dove li riprende il preflight
//! comune (`ReadOptions::sorgente_di`). Nessun driver li vede: per costruzione
//! leggono tutti dalla stessa via.
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{ReadOptions, Source};
use plenora_io_model::Result;

/// Una sorgente di lettura: percorso, byte in memoria o voce di uno ZIP.
///
/// Gli archivi annidati si rappresentano e si rifiutano (`UNSUPPORTED`) con la
/// stessa regola in ogni superficie.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Sorgente {
    Percorso(PathBuf),
    Byte { nome: String, dati: Arc<[u8]> },
    Archivio { archivio: Box<Self>, voce: String },
}

macro_rules! da_percorso {
    ($($t:ty),+) => {$(
        impl From<$t> for Sorgente {
            fn from(path: $t) -> Self {
                Self::Percorso(PathBuf::from(path))
            }
        }
    )+};
}
da_percorso!(PathBuf, &Path, &PathBuf, String, &str, OsString, &OsStr);
impl From<Source> for Sorgente {
    fn from(source: Source) -> Self {
        let Source::Path(path) = source;
        Self::Percorso(path)
    }
}

impl Sorgente {
    /// Byte in memoria; `nome` decide il formato e il nome nella vista.
    #[must_use]
    pub fn byte(nome: impl Into<String>, dati: impl Into<Arc<[u8]>>) -> Self {
        Self::Byte {
            nome: nome.into(),
            dati: dati.into(),
        }
    }
    /// La voce `voce` di questo archivio.
    #[must_use]
    pub fn voce(self, voce: impl Into<String>) -> Self {
        Self::Archivio {
            archivio: Box::new(self),
            voce: voce.into(),
        }
    }
    /// Il nome logico: il percorso, il nome dei byte o la voce.
    #[must_use]
    pub fn nome(&self) -> &Path {
        match self {
            Self::Percorso(path) => path,
            Self::Byte { nome, .. } => Path::new(nome),
            Self::Archivio { voce, .. } => Path::new(voce),
        }
    }
    /// La `Source` da passare a un driver con queste opzioni: il nome logico,
    /// mentre i byte restano nelle opzioni fino al preflight.
    #[must_use]
    pub fn per_lettura(self, opts: &mut ReadOptions) -> Source {
        let nome = self.nome().to_owned();
        opts.sorgente = match self {
            Self::Percorso(_) => None,
            altro => Some(altro),
        };
        Source::Path(nome)
    }
}

impl ReadOptions {
    /// La sorgente che `source` nomina: quella affidata da
    /// [`Sorgente::per_lettura`] se il nome coincide, altrimenti il percorso.
    ///
    /// # Errors
    /// Un nome diverso da quello affidato: le opzioni sono di un'altra lettura.
    pub fn sorgente_di(&mut self, source: Source) -> Result<Sorgente> {
        let Source::Path(path) = source;
        match self.sorgente.take() {
            None => Ok(Sorgente::Percorso(path)),
            Some(sorgente) if sorgente.nome() == path => Ok(sorgente),
            Some(_) => Err(plenora_io_model::error_table::Situation::InvalidConfiguration.error()),
        }
    }
}

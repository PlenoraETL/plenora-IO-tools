//! Insiemi fisici dei dataset e risoluzione indipendente dalle maiuscole.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use plenora_io_model::{PlenoraIoError, PublicMessage, Result};

/// Relazione del companion con il principale.
#[derive(Clone, Copy, Debug)]
pub enum Naming {
    Extension(&'static str),
    Suffix(&'static str),
}

/// Politica di lettura; ogni membro presente resta soggetto al preflight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadMember {
    Required,
    Optional,
    Foreign,
    Forbidden,
}

#[derive(Clone, Copy, Debug)]
pub struct Member {
    pub naming: Naming,
    pub read: ReadMember,
    pub written: bool,
}

/// Il principale e' sempre obbligatorio; gli alberi includono ogni discendente.
#[derive(Clone, Copy, Debug)]
pub struct FileSet {
    pub members: &'static [Member],
    pub tree: bool,
    pub directory_main: Option<&'static str>,
    pub staging_suffixes: &'static [&'static str],
}

pub const FILE_SETS: [FileSet; 10] = [
    CSV, SHP, GPKG, FILEGDB, SINGLE, SINGLE, SINGLE, SINGLE, SINGLE, SINGLE,
];

impl Member {
    #[must_use]
    pub fn path(self, main: &Path) -> PathBuf {
        member_path(main, self.naming)
    }
}

/// Risolve un membro obbligatorio.
///
/// # Errors
/// Assenza, ambiguita' o errore I/O.
pub fn required(path: &Path) -> Result<PathBuf> {
    resolve_exact(path)?.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound).into())
}

/// Conserva la grafia dell'estensione principale anche sui companion.
#[must_use]
pub fn output_companion(path: &Path, extension: &str) -> PathBuf {
    if extension == "shp" {
        return path.to_owned();
    }
    let pattern = path.extension().unwrap_or_default().as_encoded_bytes();
    let mut ext = extension.as_bytes().to_vec();
    for (index, byte) in ext.iter_mut().enumerate() {
        if pattern.get(index).is_some_and(u8::is_ascii_uppercase) {
            byte.make_ascii_uppercase();
        }
    }
    path.with_extension(String::from_utf8_lossy(&ext).as_ref())
}

const fn extension(name: &'static str, read: ReadMember, written: bool) -> Member {
    Member {
        naming: Naming::Extension(name),
        read,
        written,
    }
}

pub const SINGLE: FileSet = FileSet {
    members: &[],
    tree: false,
    directory_main: None,
    staging_suffixes: &[],
};
pub const CSV: FileSet = FileSet {
    members: &[
        extension("prj", ReadMember::Foreign, false),
        extension("csvt", ReadMember::Foreign, false),
    ],
    tree: false,
    directory_main: None,
    staging_suffixes: &[],
};
pub const SHP: FileSet = FileSet {
    members: &[
        extension("dbf", ReadMember::Required, true),
        extension("shx", ReadMember::Optional, true),
        extension("prj", ReadMember::Optional, true),
        extension("cpg", ReadMember::Optional, true),
        extension("qix", ReadMember::Foreign, false),
        extension("sbn", ReadMember::Foreign, false),
        extension("sbx", ReadMember::Foreign, false),
        extension("shp.xml", ReadMember::Foreign, false),
    ],
    tree: false,
    directory_main: Some("data.shp"),
    staging_suffixes: &[],
};
pub const GPKG: FileSet = FileSet {
    members: &[
        Member {
            naming: Naming::Suffix("-wal"),
            read: ReadMember::Forbidden,
            written: false,
        },
        Member {
            naming: Naming::Suffix("-shm"),
            read: ReadMember::Forbidden,
            written: false,
        },
        Member {
            naming: Naming::Suffix("-journal"),
            read: ReadMember::Forbidden,
            written: false,
        },
    ],
    tree: false,
    directory_main: None,
    staging_suffixes: &[],
};
pub const FILEGDB_STAGING_MARKER: &str = ".plenora-tmp-";

pub const FILEGDB: FileSet = FileSet {
    members: &[],
    tree: true,
    directory_main: None,
    staging_suffixes: &[".gdb", ".lock"],
};

/// Costruisce un nome logico, prima della ricerca del percorso effettivo.
#[must_use]
pub fn member_path(main: &Path, naming: Naming) -> PathBuf {
    match naming {
        Naming::Extension(ext) => main.with_extension(ext),
        Naming::Suffix(suffix) => {
            let mut name = main.as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        }
    }
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn same_name(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    name_key(a) == name_key(b)
}

pub(crate) fn name_key(name: &std::ffi::OsStr) -> Vec<u8> {
    name.to_str().map_or_else(
        || name.as_encoded_bytes().to_ascii_lowercase(),
        |s| s.to_lowercase().into_bytes(),
    )
}

fn ambiguous() -> PlenoraIoError {
    PlenoraIoError::formato_redatto(
        "dataset",
        &PublicMessage::Curated(
            "insieme del dataset ambiguo: piu' nomi differiscono solo per maiuscole",
        ),
    )
}

pub(crate) fn note_name(
    names: &mut std::collections::BTreeSet<Vec<u8>>,
    name: &std::ffi::OsStr,
) -> Result<()> {
    if !names.insert(name_key(name)) {
        return Err(ambiguous());
    }
    Ok(())
}

/// Il seam di enumerazione consente di provare collisioni anche su Windows.
fn select_name(
    wanted: &std::ffi::OsStr,
    names: impl IntoIterator<Item = std::io::Result<OsString>>,
) -> Result<Option<OsString>> {
    let mut found = None;
    let mut collision = false;
    for name in names {
        let name = name?;
        if wanted == name {
            return Ok(Some(name));
        }
        if same_name(wanted, &name) {
            collision |= found.is_some();
            found = Some(name);
        }
    }
    if collision {
        return Err(ambiguous());
    }
    Ok(found)
}

/// Risolve un membro senza seguire link; la grafia esatta ha precedenza.
///
/// # Errors
/// Ambiguita' tipizzata o errore di enumerazione della directory.
pub fn resolve(path: &Path) -> Result<Option<PathBuf>> {
    let Some(name) = path.file_name() else {
        return Ok(None);
    };
    let names = std::fs::read_dir(parent(path))?.map(|entry| entry.map(|e| e.file_name()));
    Ok(select_name(name, names)?.map(|name| parent(path).join(name)))
}

/// Companion effettivo; se assente conserva il nome logico per l'errore I/O.
///
/// # Errors
/// Ambiguita' o errore di enumerazione.
pub fn companion(main: &Path, ext: &'static str) -> Result<PathBuf> {
    let logical = member_path(main, Naming::Extension(ext));
    Ok(resolve_extension(main, ext)?.unwrap_or(logical))
}

// SQLite opens suffixes using the native filesystem's case rules.
/// Resolves a physical path using the filesystem's own case rules.
///
/// # Errors
/// Propagates filesystem errors other than absence.
pub fn resolve_exact(path: &Path) -> Result<Option<PathBuf>> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(Some(path.to_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn resolve_extension(main: &Path, extension: &str) -> Result<Option<PathBuf>> {
    let logical = main.with_extension(extension);
    let path = logical.as_path();
    let mut prefix = main.file_stem().unwrap_or_default().to_owned();
    prefix.push(".");
    if let Some(path) = resolve_exact(path)? {
        return Ok(Some(path));
    }
    let names = std::fs::read_dir(parent(path))?.filter_map(|entry| match entry {
        Ok(entry)
            if entry
                .file_name()
                .as_encoded_bytes()
                .starts_with(prefix.as_encoded_bytes()) =>
        {
            Some(Ok(entry.file_name()))
        }
        Ok(_) => None,
        Err(error) => Some(Err(error)),
    });
    Ok(select_name(path.file_name().unwrap_or_default(), names)?
        .map(|name| parent(path).join(name)))
}

fn resolve_member(path: &Path, naming: Naming) -> Result<Option<PathBuf>> {
    let logical = member_path(path, naming);
    match naming {
        Naming::Suffix(_) => resolve_exact(&logical),
        Naming::Extension(ext) => resolve_extension(path, ext),
    }
}

impl FileSet {
    /// Risolve l'insieme letto, compreso il contenitore Shapefile.
    ///
    /// # Errors
    /// Membro assente, ambiguo o errore I/O.
    pub fn input(self, path: &Path) -> Result<Vec<PathBuf>> {
        self.input_validated(path, |_| Ok(()))
    }

    pub(crate) fn input_validated(
        self,
        path: &Path,
        validate: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<Vec<PathBuf>> {
        // Reject directory links in the lexical path before any native decoder opens it.
        // Checking only the final component would allow a link to bypass the footprint.
        for ancestor in path.ancestors().filter(|part| !part.as_os_str().is_empty()) {
            if std::fs::symlink_metadata(ancestor)?
                .file_type()
                .is_symlink()
            {
                return Err(PlenoraIoError::non_supportato_redatto(
                    &PublicMessage::Curated("symlink di sorgente non ammesso in fase read"),
                ));
            }
        }
        let main = required(path)?;
        let metadata = std::fs::symlink_metadata(&main)?;
        let shapefile_directory = self.directory_main.is_some()
            && main.file_name().is_some_and(|name| {
                name.as_encoded_bytes()
                    .to_ascii_lowercase()
                    .ends_with(b".shp.d")
            });
        if metadata.is_dir() && !self.tree && !shapefile_directory {
            return Err(plenora_io_model::error_table::Situation::DirectoryInsteadOfFile.error());
        }
        if metadata.file_type().is_symlink() {
            return Err(PlenoraIoError::non_supportato_redatto(
                &PublicMessage::Curated("symlink non ammesso nella sorgente"),
            ));
        }
        validate(&main)?;
        if metadata.is_file() && metadata.len() == 0 {
            return Err(plenora_io_model::error_table::Situation::InvalidContent.error());
        }
        let paths = self.paths(&main)?;
        if paths[0].is_dir() {
            if let Some(main) = self.directory_main {
                let logical = paths[0].join(main);
                let actual = resolve(&logical)?.unwrap_or(logical);
                self.paths(&actual)?;
            }
        }
        Ok(paths)
    }
    /// Enumera principale e companion presenti, esigendo quelli obbligatori.
    ///
    /// # Errors
    /// Ambiguita', membro obbligatorio assente o errore I/O.
    pub fn paths(self, path: &Path) -> Result<Vec<PathBuf>> {
        let main = resolve_exact(path)?.unwrap_or_else(|| path.to_owned());
        let mut paths = vec![main.clone()];
        if main.is_dir() {
            return Ok(paths);
        }
        for member in self.members {
            match resolve_member(&main, member.naming)? {
                Some(path) => paths.push(path),
                None if member.read == ReadMember::Required => {
                    return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
                }
                None => {}
            }
        }
        Ok(paths)
    }

    /// No-clobber dell'insieme, inclusi i membri non prodotti dal writer.
    ///
    /// # Errors
    /// `OUTPUT_EXISTS` per qualunque grafia presente; errore I/O altrimenti.
    pub fn ensure_absent(self, path: &Path) -> Result<()> {
        self.ensure_absent_except_staging(path, &[])
    }

    /// Verifica il publish escludendo solo gli staging posseduti dal writer.
    ///
    /// # Errors
    /// `OUTPUT_EXISTS` per membri o staging estranei; errore I/O.
    pub fn ensure_absent_except_staging(self, path: &Path, owned: &[&Path]) -> Result<()> {
        if self.directory_main.is_some()
            && path.file_name().is_some_and(|n| {
                n.as_encoded_bytes()
                    .to_ascii_lowercase()
                    .ends_with(b".shp.d")
            })
        {
            return SINGLE.ensure_absent(path);
        }
        if resolve_exact(path)?.is_some() {
            return Err(PlenoraIoError::destinazione_esistente());
        }
        for member in self.members {
            if resolve_member(path, member.naming)?.is_some() {
                return Err(PlenoraIoError::destinazione_esistente());
            }
        }
        let mut prefix = path.file_stem().unwrap_or_default().to_owned();
        prefix.push(FILEGDB_STAGING_MARKER);
        let prefix = name_key(&prefix);
        for entry in std::fs::read_dir(parent(path))? {
            let entry = entry?;
            let name = entry.file_name();
            let lower = name_key(&name);
            let staging = lower.starts_with(&prefix)
                && self
                    .staging_suffixes
                    .iter()
                    .any(|s| lower.ends_with(s.as_bytes()));
            if staging && !owned.contains(&entry.path().as_path()) {
                return Err(PlenoraIoError::destinazione_esistente());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

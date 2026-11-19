//! `FileGDB` physical names: one ASCII case-folded identity on every filesystem.
use std::path::{Path, PathBuf};

/// Parsed physical member; named attribute indexes retain their full suffix.
#[derive(Debug, PartialEq, Eq)]
pub struct Member {
    pub id: Option<u32>,
    pub extension: String,
    pub canonical: String,
}

/// Physical dependencies shared by discovery, validation and native readers.
/// The catalog decoder supplies the IDs of shared native schema tables.
#[derive(Clone, Copy, Debug)]
pub enum Operation<'a> {
    Catalog,
    Object { id: u64, catalog: &'a [u64] },
}

/// Return only the files belonging to this operation, without opening siblings.
/// # Errors
/// Directory enumeration, missing required members or ambiguous identities.
pub fn required(
    directory: &Path,
    operation: Operation<'_>,
) -> std::io::Result<Vec<(PathBuf, Member)>> {
    let mut members = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let Some(member) = entry.file_name().to_str().and_then(parse) else {
            continue;
        };
        let selected = match operation {
            Operation::Catalog => member.id == Some(1),
            Operation::Object { id, catalog } => member.id.is_some_and(|owner| {
                owner == 1 || u64::from(owner) == id || catalog.contains(&u64::from(owner))
            }),
        };
        if selected
            && members
                .insert(member.canonical.clone(), (entry.path(), member))
                .is_some()
        {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
    let id = match operation {
        Operation::Catalog => 1,
        Operation::Object { id, .. } => id,
    };
    for extension in ["gdbtable", "gdbtablx"] {
        if !members.contains_key(&format!("a{id:08x}.{extension}")) {
            return Err(std::io::ErrorKind::NotFound.into());
        }
    }
    Ok(members.into_values().collect())
}

/// Parse a basename, never a path. Unknown extensions remain associated with
/// their table so they cannot bypass local ownership checks.
#[must_use]
pub fn parse(name: &str) -> Option<Member> {
    let canonical = name.to_ascii_lowercase();
    if matches!(canonical.as_str(), "gdb" | "timestamps") {
        return Some(Member {
            id: None,
            extension: canonical.clone(),
            canonical,
        });
    }
    let (stem, suffix) = canonical.split_once('.')?;
    if stem.len() != 9
        || !stem.starts_with('a')
        || !stem.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        || !suffix.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
    {
        return None;
    }
    let id = u32::from_str_radix(&stem[1..], 16).ok()?;
    let extension = suffix.rsplit('.').next()?.to_owned();
    Some(Member {
        id: Some(id),
        extension,
        canonical,
    })
}

/// Resolve by parsed identity, rejecting aliases even when an exact name exists.
/// # Errors
/// Missing member, ambiguous identity or directory I/O failure.
pub fn resolve(directory: &Path, name: &str) -> std::io::Result<PathBuf> {
    let wanted = parse(name).ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut found = None;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .and_then(parse)
            .is_some_and(|m| m == wanted)
        {
            if found.is_some() {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            found = Some(entry.path());
        }
    }
    found.ok_or_else(|| std::io::ErrorKind::NotFound.into())
}

#[cfg(test)]
mod tests;

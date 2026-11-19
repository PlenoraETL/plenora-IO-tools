//! `FileGDB` physical names: one ASCII case-folded identity on every filesystem.
use std::path::{Path, PathBuf};

/// Parsed physical member; named attribute indexes retain their full suffix.
#[derive(Debug, PartialEq, Eq)]
pub struct Member {
    pub id: Option<u32>,
    pub extension: String,
    pub canonical: String,
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

//! Passive, bounded recent-worktree metadata. Reading/forgetting a bookmark
//! never probes the repository, grants trust, removes drafts or deletes files.
//! Schema 1's existing transactional settings store holds the versioned record;
//! no database migration or incompatible change to older drafts is needed.

use std::fmt::Write as _;
use std::path::Path;

use srcmgr_domain::{EncodedPath, NativePath, PathError};

use crate::{ProfileStore, StoreError};

pub const RECENT_REPOSITORY_LIMIT: usize = 32;
const KEY: &str = "workspace.recents.v1";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 65536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentRepository {
    pub backend: String,
    pub path: EncodedPath,
}

impl RecentRepository {
    pub fn native_path(&self) -> Result<NativePath, PathError> {
        NativePath::from_encoded(self.path.clone())
    }
}

impl ProfileStore {
    /// Most recently explicitly opened first. Foreign-platform paths remain
    /// tagged metadata: callers can show them unavailable rather than opening
    /// a different local path. The repository filesystem is never consulted.
    pub fn recent_repositories(&self) -> Result<Vec<RecentRepository>, StoreError> {
        let Some(value) = self.get_setting_bounded(KEY, MAX_BYTES)? else {
            return Ok(Vec::new());
        };
        let mut entries = Vec::new();
        for line in value.lines() {
            if entries.len() == RECENT_REPOSITORY_LIMIT {
                return Err(StoreError::InvalidMetadata);
            }
            let fields: Vec<_> = line.splitn(4, '\t').collect();
            if fields.len() != 3 || !valid_backend(fields[0]) {
                return Err(StoreError::InvalidMetadata);
            }
            let path = decode_path(fields[1], fields[2])?;
            let entry = RecentRepository {
                backend: fields[0].to_owned(),
                path,
            };
            if entries.contains(&entry) {
                return Err(StoreError::InvalidMetadata);
            }
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Called only after the application explicitly opens/validates this
    /// canonical worktree. This record is navigation history, never authority.
    pub fn remember_repository(
        &mut self,
        backend: &str,
        canonical_worktree: &Path,
    ) -> Result<(), StoreError> {
        if !valid_backend(backend) {
            return Err(StoreError::InvalidMetadata);
        }
        let entry = RecentRepository {
            backend: backend.to_owned(),
            path: NativePath::new(canonical_worktree.as_os_str()).encoded(),
        };
        let mut entries = self.recent_repositories()?;
        entries.retain(|old| old != &entry);
        entries.insert(0, entry);
        entries.truncate(RECENT_REPOSITORY_LIMIT);
        self.write_recents(&entries)
    }

    /// Removes navigation metadata only. Does not access the referenced path.
    pub fn forget_repository(&mut self, entry: &RecentRepository) -> Result<(), StoreError> {
        let mut entries = self.recent_repositories()?;
        let before = entries.len();
        entries.retain(|old| old != entry);
        if entries.len() == before {
            return Ok(());
        }
        self.write_recents(&entries)
    }

    /// Replace one explicitly reviewed bookmark, preserving unrelated entries
    /// and all drafts/trust/navigation. The caller validates the native target;
    /// storage itself performs no filesystem probe or repository operation.
    pub fn relocate_repository(&mut self, expected: &RecentRepository, target: &Path) -> Result<Vec<RecentRepository>, StoreError> {
        let mut entries = self.recent_repositories()?;
        if !entries.contains(expected) { return Err(StoreError::InvalidMetadata); }
        let replacement = RecentRepository { backend: expected.backend.clone(), path: NativePath::new(target.as_os_str()).encoded() };
        if replacement == *expected { return Ok(entries); }
        // An already-bookmarked destination is one record, not a merge of
        // repository state. Keep the source's position after deduplication.
        entries.retain(|entry| entry == expected || *entry != replacement);
        let position = entries.iter().position(|entry| entry == expected).ok_or(StoreError::InvalidMetadata)?;
        entries[position] = replacement;
        self.write_recents(&entries)?;
        Ok(entries)
    }

    fn write_recents(&mut self, entries: &[RecentRepository]) -> Result<(), StoreError> {
        let mut value = String::new();
        for entry in entries {
            write!(value, "{}\t{}", entry.backend, encode_path(&entry.path)?).expect("write String");
            value.push('\n');
            if value.len() > MAX_BYTES {
                return Err(StoreError::InvalidMetadata);
            }
        }
        self.put_setting(KEY, &value)
    }
}

pub(crate) fn encode_path(path: &EncodedPath) -> Result<String, StoreError> {
    let bytes = match path {
        EncodedPath::UnixBytes(bytes) => bytes.clone(),
        EncodedPath::WindowsUtf16(units) => units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
    };
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES { return Err(StoreError::InvalidMetadata); }
    let mut value = format!("{}\t", path.tag());
    for byte in bytes { write!(value, "{byte:02x}").expect("write String"); }
    Ok(value)
}

pub(crate) fn decode_path(tag: &str, value: &str) -> Result<EncodedPath, StoreError> {
    let bytes = unhex(value)?;
    match tag {
        "unix-bytes" => Ok(EncodedPath::UnixBytes(bytes)),
        "windows-utf16" if bytes.len().is_multiple_of(2) => Ok(EncodedPath::WindowsUtf16(bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect())),
        _ => Err(StoreError::InvalidMetadata),
    }
}

fn valid_backend(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn unhex(value: &str) -> Result<Vec<u8>, StoreError> {
    if value.is_empty() || !value.len().is_multiple_of(2) || value.len() > MAX_PATH_BYTES * 2 {
        return Err(StoreError::InvalidMetadata);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = char::from(pair[0])
                .to_digit(16)
                .ok_or(StoreError::InvalidMetadata)?;
            let low = char::from(pair[1])
                .to_digit(16)
                .ok_or(StoreError::InvalidMetadata)?;
            Ok((high * 16 + low) as u8)
        })
        .collect()
}

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::profile::write_atomic;

const HEADER: &str = "minicon-srcmgr trust v1";

/// Why a repository may not run Git right now, or why a grant cannot be made.
#[derive(Debug)]
pub enum TrustError {
    /// No grant exists for this repository.
    Untrusted,
    /// A grant exists, but the execution-relevant files changed after it.
    Changed,
    /// The repository config includes other files. Their content is not
    /// reviewed by this product, so no grant is made.
    IncludesPresent,
    /// Execution-relevant metadata is a link or not a plain file.
    UnsafeMetadata(&'static str),
    /// The grant file is not in the expected format.
    Malformed { line: usize },
    /// The repository path cannot be written as UTF-8 in the grant file.
    UnrepresentablePath,
    Io(io::Error),
}

impl fmt::Display for TrustError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Untrusted => formatter.write_str("this repository is not trusted yet"),
            Self::Changed => formatter.write_str(
                "repository configuration or hooks changed since trust was granted; review and grant again",
            ),
            Self::IncludesPresent => formatter.write_str(
                "the repository config includes other files; review them manually, this product will not trust that configuration",
            ),
            Self::UnsafeMetadata(what) => write!(formatter, "unsafe repository metadata: {what}"),
            Self::Malformed { line } => {
                write!(formatter, "trust file line {line} is malformed; no grants were used")
            }
            Self::UnrepresentablePath => formatter.write_str("the repository path cannot be stored as UTF-8"),
            Self::Io(_) => formatter.write_str("reading or writing the trust file failed"),
        }
    }
}

impl std::error::Error for TrustError {}

/// One grant: the canonical `.git` directory and the fingerprint of its
/// execution-relevant state when the user granted trust.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Grant {
    pub repository: PathBuf,
    pub fingerprint: String,
}

/// The durable set of grants for one profile.
#[derive(Debug)]
pub struct TrustStore {
    path: PathBuf,
    grants: Vec<Grant>,
}

impl TrustStore {
    /// Opens the store at `path`. A missing file is an empty store; any other
    /// content that does not parse makes the whole store unusable, so no
    /// grant is ever guessed.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, TrustError> {
        let path = path.into();
        let grants = match fs::read_to_string(&path) {
            Ok(text) => parse(&text)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(TrustError::Io(error)),
        };
        Ok(Self { path, grants })
    }

    #[must_use]
    pub fn grants(&self) -> &[Grant] {
        &self.grants
    }

    /// Records a grant, replacing any earlier grant for the same repository.
    pub fn grant(&mut self, repository: PathBuf, fingerprint: String) {
        self.grants.retain(|existing| existing.repository != repository);
        self.grants.push(Grant {
            repository,
            fingerprint,
        });
    }

    /// Removes the grant for `repository`, if any. Returns whether one existed.
    pub fn revoke(&mut self, repository: &Path) -> bool {
        let before = self.grants.len();
        self.grants.retain(|existing| existing.repository != repository);
        before != self.grants.len()
    }

    /// Writes the store atomically.
    pub fn save(&self) -> Result<(), TrustError> {
        let mut text = String::from(HEADER);
        text.push('\n');
        for grant in &self.grants {
            let path = grant.repository.to_str().ok_or(TrustError::UnrepresentablePath)?;
            text.push_str(&hex(path.as_bytes()));
            text.push(' ');
            text.push_str(&grant.fingerprint);
            text.push('\n');
        }
        write_atomic(&self.path, text.as_bytes()).map_err(TrustError::Io)
    }

    /// Confirms that `repository` has a grant whose fingerprint still matches
    /// the files on disk. Reads metadata only; Git is never run here.
    pub fn verify(&self, repository: &Path, worktree: &Path) -> Result<(), TrustError> {
        let grant = self
            .grants
            .iter()
            .find(|grant| grant.repository == repository)
            .ok_or(TrustError::Untrusted)?;
        let current = fingerprint(repository, worktree)?;
        if current == grant.fingerprint {
            Ok(())
        } else {
            Err(TrustError::Changed)
        }
    }
}

/// Hashes the files that can make Git execute configured programs: the
/// repository config, `info/attributes`, the worktree `.gitattributes`, and
/// every regular file in `hooks/`. A config that includes other files is
/// refused, because the included content would be outside this fingerprint.
pub fn fingerprint(git_dir: &Path, worktree: &Path) -> Result<String, TrustError> {
    let config = read_plain_file(&git_dir.join("config"))?.unwrap_or_default();
    if config
        .split(|byte| *byte == b'\n')
        .any(|line| line.trim_ascii_start().to_ascii_lowercase().starts_with(b"[include"))
    {
        return Err(TrustError::IncludesPresent);
    }

    let mut hasher = Sha256::new();
    feed(&mut hasher, "config", &config);
    feed_optional(&mut hasher, "info/attributes", &git_dir.join("info").join("attributes"))?;
    feed_optional(&mut hasher, "worktree/.gitattributes", &worktree.join(".gitattributes"))?;

    let hooks = git_dir.join("hooks");
    match fs::symlink_metadata(&hooks) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            let mut entries = fs::read_dir(&hooks)
                .map_err(TrustError::Io)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(TrustError::Io)?;
            entries.sort();
            for path in entries {
                let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
                let content = read_plain_file(&path)?
                    .ok_or(TrustError::UnsafeMetadata("a hook disappeared while reading"))?;
                feed(&mut hasher, &format!("hooks/{name}"), &content);
            }
        }
        Ok(_) => return Err(TrustError::UnsafeMetadata("hooks must be a plain directory")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(TrustError::Io(error)),
    }

    Ok(hex(&hasher.finalize()))
}

fn feed_optional(hasher: &mut Sha256, label: &str, path: &Path) -> Result<(), TrustError> {
    match read_plain_file(path)? {
        Some(content) => feed(hasher, label, &content),
        None => feed(hasher, label, b"<absent>"),
    }
    Ok(())
}

fn feed(hasher: &mut Sha256, label: &str, content: &[u8]) {
    hasher.update(format!("{label}\0{}\0", content.len()).as_bytes());
    hasher.update(content);
}

fn read_plain_file(path: &Path) -> Result<Option<Vec<u8>>, TrustError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => fs::read(path).map(Some).map_err(TrustError::Io),
        Ok(_) => Err(TrustError::UnsafeMetadata("metadata is a link or not a regular file")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(TrustError::Io(error)),
    }
}

fn parse(text: &str) -> Result<Vec<Grant>, TrustError> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(TrustError::Malformed { line: 1 });
    }
    let mut grants = Vec::new();
    for (index, line) in lines.enumerate() {
        let line_number = index + 2;
        let mut fields = line.split(' ');
        let (Some(encoded), Some(fingerprint), None) = (fields.next(), fields.next(), fields.next()) else {
            return Err(TrustError::Malformed { line: line_number });
        };
        let bytes = unhex(encoded).ok_or(TrustError::Malformed { line: line_number })?;
        let path = String::from_utf8(bytes).map_err(|_| TrustError::Malformed { line: line_number })?;
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(TrustError::Malformed { line: line_number });
        }
        grants.push(Grant {
            repository: PathBuf::from(path),
            fingerprint: fingerprint.to_owned(),
        });
    }
    Ok(grants)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(text.get(index..index + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_arbitrary_bytes() {
        let bytes = [0_u8, 1, 0x7f, 0xff, b'a'];
        assert_eq!(unhex(&hex(&bytes)), Some(bytes.to_vec()));
    }

    #[test]
    fn malformed_header_is_refused_as_a_whole() {
        assert!(matches!(parse("other\n"), Err(TrustError::Malformed { line: 1 })));
    }

    #[test]
    fn malformed_grant_line_is_refused() {
        let text = format!("{HEADER}\nnot-hex {}\n", "a".repeat(64));
        assert!(matches!(parse(&text), Err(TrustError::Malformed { line: 2 })));
    }
}

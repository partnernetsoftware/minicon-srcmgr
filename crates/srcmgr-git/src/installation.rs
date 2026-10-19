use std::path::{Path, PathBuf};

use minicon_core::version::Version;

use crate::error::GitError;
use crate::runner::{self, Limits};

/// The oldest Git this product supports (PRD 02). Newer optional behavior is
/// probed separately; a version number alone never proves a capability.
pub const MINIMUM_GIT: Version = Version {
    major: 2,
    minor: 43,
    patch: 0,
};

/// A selected Git executable and the version it reported.
///
/// Discovery and the choice of executable belong to the caller. The runner
/// never searches the current directory or a repository for Git.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitInstallation {
    executable: PathBuf,
    version: Version,
}

impl GitInstallation {
    /// Runs `<executable> --version` outside any repository and records the
    /// reported version.
    pub fn probe(executable: impl Into<PathBuf>) -> Result<Self, GitError> {
        let executable = executable.into();
        let output = runner::run(&executable, None, &["--version"], None, Limits::PROBE)?;
        let version = parse_version(&output.stdout)?;
        Ok(Self { executable, version })
    }

    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    #[must_use]
    pub const fn version(&self) -> Version {
        self.version
    }

    /// Refuses an installation below [`MINIMUM_GIT`]. Commands that depend on
    /// it must not run on an older Git; the caller reports the requirement.
    pub fn require_supported(&self) -> Result<(), GitError> {
        if self.version < MINIMUM_GIT {
            return Err(GitError::UnsupportedVersion {
                found: self.version,
                minimum: MINIMUM_GIT,
            });
        }
        Ok(())
    }
}

/// Reads `git version 2.42.0.windows.2` and keeps the numeric `X.Y.Z` part.
/// Vendor suffixes are ignored; a report without three numbers is refused.
pub fn parse_version(report: &[u8]) -> Result<Version, GitError> {
    let text = std::str::from_utf8(report).map_err(|_| GitError::UnrecognizedVersion)?;
    let word = text
        .trim()
        .strip_prefix("git version ")
        .and_then(|rest| rest.split_whitespace().next())
        .ok_or(GitError::UnrecognizedVersion)?;
    let numeric: Vec<&str> = word.split('.').take(3).collect();
    if numeric.len() != 3 {
        return Err(GitError::UnrecognizedVersion);
    }
    Version::parse(&numeric.join(".")).ok_or(GitError::UnrecognizedVersion)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_suffix_is_ignored() {
        let version = parse_version(b"git version 2.42.0.windows.2\n").expect("version");
        assert_eq!(version, Version { major: 2, minor: 42, patch: 0 });
    }

    #[test]
    fn plain_release_is_read() {
        let version = parse_version(b"git version 2.51.1\n").expect("version");
        assert_eq!(version, Version { major: 2, minor: 51, patch: 1 });
    }

    #[test]
    fn unrecognized_report_is_refused() {
        assert!(matches!(parse_version(b"hello"), Err(GitError::UnrecognizedVersion)));
        assert!(matches!(parse_version(b"git version 2.x"), Err(GitError::UnrecognizedVersion)));
    }

    #[test]
    fn floor_is_enforced_exactly() {
        let below = GitInstallation {
            executable: PathBuf::from("git"),
            version: Version { major: 2, minor: 42, patch: 9 },
        };
        assert!(matches!(below.require_supported(), Err(GitError::UnsupportedVersion { .. })));
        let at_floor = GitInstallation {
            executable: PathBuf::from("git"),
            version: MINIMUM_GIT,
        };
        assert!(at_floor.require_supported().is_ok());
    }
}

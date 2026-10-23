//! Explicit local initialization. Preparation is filesystem-only; execution
//! reserves new metadata and never reinitializes an existing `.git` entry.
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use minicon_platform::filesystem::{FileIdentity, metadata_is_link_like, path_identity};

use crate::runner::{self, Limits};
use crate::{GitError, GitInstallation};

#[derive(Debug)]
pub struct InitializationTarget {
    path: PathBuf,
    parent_identity: FileIdentity,
    directory_identity: Option<FileIdentity>,
    entries: Vec<OsString>,
    more_entries: bool,
}

impl InitializationTarget {
    /// Does not create directories, invoke Git or traverse existing contents.
    pub fn prepare(path: &Path) -> Result<Self, GitError> {
        let absolute = std::path::absolute(path).map_err(GitError::Io)?;
        let name = absolute.file_name().ok_or(GitError::InitializationTarget("choose a directory below an existing parent"))?;
        let parent = fs::canonicalize(absolute.parent().ok_or(GitError::InitializationTarget("target has no parent"))?).map_err(GitError::Io)?;
        let path = parent.join(name);
        let directory_identity = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata_is_link_like(&metadata) => Some(path_identity(&path).map_err(GitError::Io)?),
            Ok(_) => return Err(GitError::InitializationTarget("target must be a plain directory or a new directory")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(GitError::Io(error)),
        };
        if directory_identity.is_some() { require_absent(&path.join(".git"))?; }
        let mut entries = Vec::new();
        let mut more_entries = false;
        if directory_identity.is_some() {
            for entry in fs::read_dir(&path).map_err(GitError::Io)?.take(51) {
                let entry = entry.map_err(GitError::Io)?;
                if entries.len() == 50 { more_entries = true; break; }
                entries.push(entry.file_name());
            }
            entries.sort();
        }
        Ok(Self { path, parent_identity: path_identity(&parent).map_err(GitError::Io)?, directory_identity, entries, more_entries })
    }

    pub fn path(&self) -> &Path { &self.path }
    pub fn is_new_directory(&self) -> bool { self.directory_identity.is_none() }
    pub fn entries(&self) -> &[OsString] { &self.entries }
    pub fn has_more_entries(&self) -> bool { self.more_entries }

    /// Consumes the reviewed target. Failure may leave recognizable partial
    /// metadata; callers must report it and never automatically delete/retry.
    pub fn execute(self, git: &GitInstallation, branch: &str) -> Result<PathBuf, GitError> {
        git.require_supported()?;
        self.path.to_str().ok_or(GitError::UnrepresentableName)?;
        // Full ref validation avoids check-ref-format --branch's revision
        // shorthand expansion and requires no repository discovery.
        if branch.is_empty() || branch.starts_with('-') || branch == "HEAD" || branch.len() > 255 {
            return Err(GitError::InvalidName);
        }
        runner::run(git.executable(), None, &["check-ref-format".to_owned(), format!("refs/heads/{branch}")], None, Limits::METADATA)?;
        let parent = self.path.parent().ok_or(GitError::InitializationTarget("target has no parent"))?;
        if !self.parent_identity.same_object(path_identity(parent).map_err(GitError::Io)?) {
            return Err(GitError::InitializationTarget("target parent changed after review; review again"));
        }
        match self.directory_identity {
            Some(identity) => {
                let metadata = fs::symlink_metadata(&self.path).map_err(GitError::Io)?;
                if metadata_is_link_like(&metadata) || !metadata.is_dir() || !identity.same_object(path_identity(&self.path).map_err(GitError::Io)?) {
                    return Err(GitError::InitializationTarget("target directory changed after review; review again"));
                }
            }
            None => fs::create_dir(&self.path).map_err(GitError::Io)?,
        }
        // Atomic reservation refuses a file, symlink or competing init. Never
        // pass existing metadata to git init, which would otherwise reinit it.
        fs::create_dir(self.path.join(".git")).map_err(GitError::Io)?;
        let arguments = vec![
            OsString::from("-c"), "init.defaultRefFormat=files".into(),
            "-c".into(), "core.bare=false".into(),
            format!("--git-dir={}", self.path.join(".git").to_str().ok_or(GitError::UnrepresentableName)?).into(),
            format!("--work-tree={}", self.path.to_str().ok_or(GitError::UnrepresentableName)?).into(),
            "init".into(), "--quiet".into(), "--template=".into(), "--object-format=sha1".into(), format!("--initial-branch={branch}").into(),
        ];
        runner::run(git.executable(), None, &arguments, None, Limits::MUTATION)?;
        Ok(self.path)
    }
}

fn require_absent(path: &Path) -> Result<(), GitError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(GitError::Io(error)),
        Ok(_) => Err(GitError::InitializationTarget("a .git entry already exists; use Open instead of Create")),
    }
}

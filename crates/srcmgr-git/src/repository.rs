use std::fs;
use std::path::{Path, PathBuf};

use srcmgr_domain::{BackendKind, Generation, RepositoryId, StatusSnapshot};

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::runner::{self, Limits};
use crate::status::parse_status;

/// Where a worktree's repository lives, and the identity used to coordinate
/// writers across linked worktrees (PRD 02, ARCH-03).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryIdentity {
    pub id: RepositoryId,
    pub worktree_root: PathBuf,
    pub common_dir: PathBuf,
}

/// Identifies the repository at `worktree`.
///
/// Only a `.git` directory directly inside `worktree` is accepted. A `.git`
/// file, a link, or a parent-directory repository is refused rather than
/// followed, because following it would point Git at metadata the user has
/// not reviewed (PRD 11). Git runs only after that metadata check passes.
pub fn locate(installation: &GitInstallation, worktree: &Path) -> Result<RepositoryIdentity, GitError> {
    installation.require_supported()?;
    let metadata = fs::symlink_metadata(worktree.join(".git")).map_err(|_| GitError::NotARepository)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(GitError::NotARepository);
    }

    let output = runner::run(
        installation.executable(),
        Some(worktree),
        &["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-common-dir"],
        Limits::METADATA,
    )?;
    let text = std::str::from_utf8(&output.stdout).map_err(|_| GitError::UnrepresentableName)?;
    let mut lines = text.lines();
    let (Some(toplevel), Some(common), None) = (lines.next(), lines.next(), lines.next()) else {
        return Err(GitError::MalformedOutput { record: 0 });
    };

    let common_dir = fs::canonicalize(common).map_err(GitError::Io)?;
    let worktree_root = fs::canonicalize(toplevel).map_err(GitError::Io)?;
    // The identity is a coordination key, so a name that cannot be spelled
    // exactly is refused instead of lossily merged with another repository.
    let key = common_dir.to_str().ok_or(GitError::UnrepresentableName)?;
    let id = RepositoryId::new(BackendKind::Git, key);
    Ok(RepositoryIdentity {
        id,
        worktree_root,
        common_dir,
    })
}

/// Reads the complete working-copy status of an identified repository.
pub fn read_status(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    generation: Generation,
) -> Result<StatusSnapshot, GitError> {
    installation.require_supported()?;
    let output = runner::run(
        installation.executable(),
        Some(&identity.worktree_root),
        &["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=normal"],
        Limits::STATUS,
    )?;
    parse_status(&output.stdout, identity.id.clone(), generation)
}

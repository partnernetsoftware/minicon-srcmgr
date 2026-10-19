use std::ffi::{OsStr, OsString};
use srcmgr_domain::RevisionToken;
use std::fs;
use std::path::{Path, PathBuf};

use srcmgr_domain::{BackendKind, Generation, RepositoryId, StatusSnapshot};

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::runner::{self, Limits};
use crate::status::{parse_status, revision};

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
        None,
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
        None,
        Limits::STATUS,
    )?;
    parse_status(&output.stdout, identity.id.clone(), generation)
}

/// Stages the whole working-tree state of each path, including deletions.
/// Paths are literal pathspecs, so glob characters are never expanded.
pub fn stage(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    paths: &[OsString],
) -> Result<(), GitError> {
    run_mutation(installation, identity, &pathspec_command("add", paths), None)?;
    Ok(())
}

/// Removes each path from the index, leaving the working tree unchanged.
pub fn unstage(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    paths: &[OsString],
) -> Result<(), GitError> {
    run_mutation(installation, identity, &pathspec_command("reset", paths), None)?;
    Ok(())
}

/// Commits the index with `message`, supplied on standard input so it never
/// appears in an argument vector. Returns the new HEAD revision.
///
/// Repository hooks run as Git's own configuration dictates; the trust grant
/// that allowed this command is what authorizes them (PRD 11).
pub fn commit(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    message: &str,
) -> Result<RevisionToken, GitError> {
    let arguments = [OsString::from("commit"), "--cleanup=strip".into(), "-F".into(), "-".into()];
    run_mutation(installation, identity, &arguments, Some(message))?;
    let head = run_mutation(
        installation,
        identity,
        &[OsString::from("rev-parse"), "--verify".into(), "HEAD".into()],
        None,
    )?;
    let text = std::str::from_utf8(&head.stdout).map_err(|_| GitError::MalformedOutput { record: 0 })?;
    revision(text.trim().as_bytes())
}

fn pathspec_command(verb: &str, paths: &[OsString]) -> Vec<OsString> {
    let mut arguments = vec![
        OsString::from("--literal-pathspecs"),
        OsString::from(verb),
        OsString::from("--"),
    ];
    arguments.extend(paths.iter().cloned());
    arguments
}

fn run_mutation(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    arguments: &[OsString],
    input: Option<&str>,
) -> Result<runner::Output, GitError> {
    installation.require_supported()?;
    runner::run(
        installation.executable(),
        Some(&identity.worktree_root),
        arguments,
        input,
        Limits::MUTATION,
    )
}

/// Returns the textual diff of one path, either index-against-HEAD (`staged`)
/// or working-tree-against-index. External diff and textconv drivers are
/// disabled, so repository-configured programs do not run for display.
/// Output is decoded lossily because it is shown, never applied.
pub fn diff(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    path: &OsStr,
    staged: bool,
) -> Result<String, GitError> {
    let mut arguments = vec![
        OsString::from("--literal-pathspecs"),
        OsString::from("diff"),
        OsString::from("--no-color"),
        OsString::from("--no-ext-diff"),
        OsString::from("--no-textconv"),
    ];
    if staged {
        arguments.push(OsString::from("--cached"));
    }
    arguments.push(OsString::from("--"));
    arguments.push(path.to_os_string());
    let output = run_mutation(installation, identity, &arguments, None)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Creates a branch at HEAD and switches to it. The name is validated by Git's
/// own rules first, so an invalid or option-like name never reaches `switch`.
pub fn create_branch(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    name: &OsStr,
) -> Result<(), GitError> {
    let name = checked_branch_name(installation, identity, name)?;
    run_mutation(
        installation,
        identity,
        &[OsString::from("switch"), "-c".into(), name],
        None,
    )?;
    Ok(())
}

/// Switches the worktree to an existing local branch. Git itself refuses a
/// switch that would overwrite local changes, and that refusal is returned.
pub fn switch_branch(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    name: &OsStr,
) -> Result<(), GitError> {
    let name = checked_branch_name(installation, identity, name)?;
    run_mutation(installation, identity, &[OsString::from("switch"), name], None)?;
    Ok(())
}

/// Asks Git whether `name` is a valid branch name. Returns it as an owned
/// `OsString` ready for an argument vector.
fn checked_branch_name(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    name: &OsStr,
) -> Result<OsString, GitError> {
    let text = name.to_str().ok_or(GitError::UnrepresentableName)?;
    if text.is_empty() || text.starts_with('-') {
        return Err(GitError::InvalidName);
    }
    run_mutation(
        installation,
        identity,
        &[OsString::from("check-ref-format"), "--branch".into(), name.to_os_string()],
        None,
    )
    .map_err(|_| GitError::InvalidName)?;
    Ok(name.to_os_string())
}

/// The text hunks of one path's diff, each starting at its `@@` line. Index
/// `0` is the first hunk. The same diff options as [`diff`] apply.
pub fn hunks(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    path: &OsStr,
    staged: bool,
) -> Result<Vec<String>, GitError> {
    let patch = diff(installation, identity, path, staged)?;
    crate::hunks::split(&patch)
        .map(|parts| parts.hunks)
        .ok_or(GitError::NoHunks)
}

/// Stages (`reverse == false`, working tree against index) or unstages
/// (`reverse == true`, index against HEAD) one hunk by zero-based index.
///
/// The hunk is applied to the index only (`git apply --cached`); the working
/// tree is never written. `--recount` lets the hunk apply after its neighbours
/// were changed.
pub fn apply_hunk(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    path: &OsStr,
    index: usize,
    reverse: bool,
) -> Result<(), GitError> {
    let patch = diff(installation, identity, path, reverse)?;
    let parts = crate::hunks::split(&patch).ok_or(GitError::NoHunks)?;
    let hunk = parts.hunks.get(index).ok_or(GitError::HunkOutOfRange)?;
    let mut text = format!("{}{hunk}", parts.header);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let mut arguments = vec![
        OsString::from("apply"),
        OsString::from("--cached"),
        OsString::from("--recount"),
    ];
    if reverse {
        arguments.push(OsString::from("-R"));
    }
    run_mutation(installation, identity, &arguments, Some(&text))?;
    Ok(())
}

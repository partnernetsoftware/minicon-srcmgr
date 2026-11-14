//! Materializing a working tree for a repository cloned with `--no-checkout`.
//!
//! Clone deliberately leaves the index populated and the worktree empty (PRD
//! 07): a user who has not yet reviewed the repository's filters should not have
//! files written from a remote tree. This module is the second half of that
//! decision — a reviewed checkout after trust, never an automatic one.
//!
//! The index is the only source. No network is contacted, no revision is moved,
//! and Git itself decides whether an existing file would be overwritten.

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::repository::RepositoryIdentity;
use crate::runner::{self, Limits};

/// What a checkout would write, shown before anything runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckoutPreview {
    pub lines: Vec<String>,
    pub files: usize,
}

/// The number of index entries waiting for a worktree, or `None` when the
/// checkout already happened.
///
/// A worktree holding any visible entry is not treated as pending: the check
/// costs one directory read for the common case, and only an empty worktree pays
/// for `git ls-files`. `.git` itself is the one entry that does not count.
pub fn pending_checkout(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
) -> Result<Option<usize>, GitError> {
    match std::fs::read_dir(&repo.worktree_root) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(GitError::Io)?;
                if entry.file_name() != ".git" {
                    return Ok(None);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(GitError::Io(error)),
    }
    // A clone with --no-checkout leaves HEAD set and the index empty, so the
    // tree that will be written is HEAD's, not the index's.
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["ls-tree", "-r", "-z", "--name-only", "HEAD"],
        None,
        Limits::METADATA,
    );
    let Ok(output) = output else {
        // No HEAD at all: an unborn repository has nothing to check out.
        return Ok(None);
    };
    let files = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .count();
    Ok((files > 0).then_some(files))
}

/// Describes the exact effect of checking out. It refuses when nothing is
/// pending, so the caller never reviews an operation that would write nothing.
pub fn preview(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
) -> Result<CheckoutPreview, GitError> {
    let files = pending_checkout(git, repo)?.ok_or(GitError::CheckoutTarget(
        "this worktree already has files; there is nothing pending to check out",
    ))?;
    let target = target_label(git, repo)?;
    let mut lines = vec![
        format!("check out {target}"),
        format!("  files written: {files}"),
        "  every path comes from the index; no network is contacted".to_owned(),
        "  no revision changes and no local modification is discarded".to_owned(),
        "  Git refuses if an existing file would be overwritten; untracked files stay".to_owned(),
    ];
    lines.push("  the repository must already be trusted, because this writes files".to_owned());
    Ok(CheckoutPreview { lines, files })
}

/// Fills the worktree from HEAD without moving HEAD, and refuses rather than
/// overwriting an existing or untracked file.
pub fn apply(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<usize, GitError> {
    let files = pending_checkout(git, repo)?.ok_or(GitError::CheckoutTarget(
        "this worktree already has files; there is nothing pending to check out",
    ))?;
    // read-tree fills the index from HEAD's tree and writes no file; then
    // checkout-index -a writes those entries. Without -f it refuses any path
    // that already exists, so an untracked file is never replaced.
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["read-tree", "HEAD"],
        None,
        Limits::METADATA,
    )?;
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["checkout-index", "-a"],
        None,
        Limits::METADATA,
    )?;
    Ok(files)
}

/// The branch name, or the short object id when HEAD is detached.
fn target_label(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<String, GitError> {
    let branch = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["symbolic-ref", "--short", "-q", "HEAD"],
        None,
        Limits::METADATA,
    );
    if let Ok(output) = &branch {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !name.is_empty() && !name.chars().any(char::is_control) {
            return Ok(format!("branch {name}"));
        }
    }
    let head = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["rev-parse", "--short", "HEAD"],
        None,
        Limits::METADATA,
    )?;
    let oid = String::from_utf8_lossy(&head.stdout).trim().to_owned();
    if oid.is_empty() || oid.chars().any(char::is_control) {
        return Err(GitError::MalformedOutput { record: 0 });
    }
    Ok(format!("detached HEAD at {oid}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn git() -> PathBuf {
        std::env::var_os("SRCMGR_TEST_GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from)
    }

    fn fixture(name: &str) -> PathBuf {
        let base = {
            #[cfg(unix)]
            {
                std::fs::canonicalize(std::env::temp_dir()).unwrap()
            }
            #[cfg(windows)]
            {
                std::env::temp_dir()
            }
        };
        let root = base.join(format!("srcmgr-checkout-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn run(dir: &Path, args: &[&str]) {
        let output = Command::new(git()).current_dir(dir).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    /// An origin repository with one commit and one file.
    fn origin(root: &Path) -> PathBuf {
        let repo = root.join("origin");
        std::fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "-q", "-b", "main"]);
        run(&repo, &["config", "user.name", "Fixture"]);
        run(&repo, &["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(repo.join("a.txt"), "one
").unwrap();
        run(&repo, &["add", "a.txt"]);
        run(&repo, &["commit", "-q", "-m", "one"]);
        repo
    }

    fn installation() -> GitInstallation {
        GitInstallation::probe(git()).unwrap()
    }

    #[test]
    fn a_clone_without_checkout_is_pending_until_it_is_checked_out() {
        let root = fixture("pending");
        let source = origin(&root);
        run(&root, &["clone", "-q", "--no-checkout", source.to_str().unwrap(), "target"]);
        let target = root.join("target");
        let installation = installation();
        let repo = crate::locate(&installation, &target).unwrap();

        assert_eq!(pending_checkout(&installation, &repo).unwrap(), Some(1));
        let preview = preview(&installation, &repo).unwrap();
        assert_eq!(preview.files, 1);
        assert!(preview.lines.iter().any(|line| line.contains("branch main")), "{preview:?}");
        assert!(preview.lines.iter().any(|line| line.contains("no network")), "{preview:?}");

        assert_eq!(apply(&installation, &repo).unwrap(), 1);
        assert_eq!(std::fs::read_to_string(target.join("a.txt")).unwrap(), "one
");
        assert_eq!(pending_checkout(&installation, &repo).unwrap(), None);
    }

    #[test]
    fn a_normal_worktree_is_never_pending_and_cannot_be_reviewed() {
        let root = fixture("normal");
        let repo_path = origin(&root);
        let installation = installation();
        let repo = crate::locate(&installation, &repo_path).unwrap();
        assert_eq!(pending_checkout(&installation, &repo).unwrap(), None);
        matches!(preview(&installation, &repo), Err(GitError::CheckoutTarget(_)));
    }

    #[test]
    fn an_existing_file_is_refused_by_git_and_survives_the_attempt() {
        let root = fixture("collision");
        let source = origin(&root);
        run(&root, &["clone", "-q", "--no-checkout", source.to_str().unwrap(), "target"]);
        let target = root.join("target");
        let installation = installation();
        let repo = crate::locate(&installation, &target).unwrap();
        // The worktree is no longer empty, so the reviewed path refuses first.
        std::fs::write(target.join("notes.txt"), "keep me
").unwrap();
        assert_eq!(pending_checkout(&installation, &repo).unwrap(), None);
        assert!(matches!(apply(&installation, &repo), Err(GitError::CheckoutTarget(_))));
        assert_eq!(std::fs::read_to_string(target.join("notes.txt")).unwrap(), "keep me
");
        assert!(!target.join("a.txt").exists(), "nothing may be written without a review");
    }
}

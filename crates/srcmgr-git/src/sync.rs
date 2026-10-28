//! Upstream configuration and fast-forward-only pull (PRD 07, PRD 11).
//!
//! The upstream is a single ref name, never a free-form revision, so an
//! option-like value cannot change what Git runs. A pull only fast-forwards:
//! divergence is Git's own refusal, reported with its exit detail rather than
//! repaired with a merge commit the user did not review.

use std::sync::atomic::AtomicBool;

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::repository::RepositoryIdentity;
use crate::runner::{self, Limits, ProgressSink};

/// The result of a fast-forward pull: the commit before and after, and whether
/// the branch actually moved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PullReport {
    pub before: String,
    pub after: String,
    pub updated: bool,
}

/// Points the current branch at `upstream`, a `remote/branch` name.
pub fn set_upstream(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    upstream: &str,
) -> Result<(), GitError> {
    if upstream.is_empty()
        || !upstream.contains('/')
        || upstream.starts_with('-')
        || upstream.chars().any(char::is_control)
    {
        return Err(GitError::InvalidRemote);
    }
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["branch", "--set-upstream-to", upstream],
        None,
        Limits::MUTATION,
    )?;
    Ok(())
}

/// Removes the current branch's upstream configuration.
pub fn clear_upstream(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<(), GitError> {
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["branch", "--unset-upstream"],
        None,
        Limits::MUTATION,
    )?;
    Ok(())
}

/// Fast-forwards the current branch to its already-fetched upstream.
///
/// Git performs no fetch here: the caller fetches first, so the merge can only
/// move the branch to the commit that was reviewed. A missing upstream is
/// `NoUpstream`; a diverged branch is Git's non-zero exit, unchanged.
pub fn pull_ff(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    cancel: &AtomicBool,
    sink: Option<&ProgressSink>,
) -> Result<PullReport, GitError> {
    let upstream = match runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["rev-parse", "--abbrev-ref", "@{upstream}"],
        None,
        Limits::METADATA,
    ) {
        Ok(output) => String::from_utf8(output.stdout)
            .map_err(|_| GitError::UnrepresentableName)?
            .trim()
            .to_owned(),
        Err(GitError::Exit { .. }) => return Err(GitError::NoUpstream),
        Err(error) => return Err(error),
    };
    // Pull is observation followed by integration: fetch the upstream remote
    // first so a new remote commit is visible, then integrate only if the
    // history is linear.
    if let Some((remote, _)) = upstream.split_once('/') && !remote.is_empty() {
        crate::remotes::fetch(git, repo, remote, cancel, sink)?;
    }
    // An unborn branch has no HEAD to compare with; it reports an empty before.
    let before = head(git, repo);
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["merge", "--ff-only", "--no-edit", "@{upstream}"],
        None,
        Limits::MUTATION,
    )?;
    let after = head(git, repo);
    Ok(PullReport {
        updated: before != after,
        before,
        after,
    })
}

fn head(git: &GitInstallation, repo: &RepositoryIdentity) -> String {
    match runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["rev-parse", "HEAD"],
        None,
        Limits::METADATA,
    ) {
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        Err(_) => String::new(),
    }
}

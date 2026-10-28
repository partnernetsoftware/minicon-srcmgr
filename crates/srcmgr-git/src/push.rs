//! Publishing the current branch (PRD 07, PRD 11).
//!
//! The refspec names both sides explicitly, so the destination branch is the
//! same name as the reviewed local branch and nothing else is pushed. Git's
//! porcelain report is the only source for the state; when the transfer is
//! interrupted, the remote is observed once with `ls-remote` to tell the user
//! what landed, and the push is never retried automatically.

use std::sync::atomic::AtomicBool;

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::remotes;
use crate::repository::RepositoryIdentity;
use crate::runner::{self, Limits, ProgressSink};

/// What the remote did with the pushed branch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PushState {
    /// The remote ref moved to the pushed commit.
    Updated,
    /// The remote already had the pushed commit.
    UpToDate,
    /// The remote refused the update, for example a non-fast-forward.
    Rejected,
    /// The transfer ended without a report; `reconciled` is true only when the
    /// remote ref was observed to equal the local HEAD afterwards.
    Unknown { reconciled: bool },
}

/// One push attempt, named by remote, branch and the exact refspec sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushReport {
    pub remote: String,
    pub branch: String,
    pub reference: String,
    pub state: PushState,
    pub observed: Option<String>,
}

/// Pushes the current branch to `remote` under its own name.
pub fn push_current(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    remote: &str,
    cancel: &AtomicBool,
    sink: Option<&ProgressSink>,
) -> Result<PushReport, GitError> {
    if remote.is_empty() || remote.starts_with('-') || remote.chars().any(char::is_control) {
        return Err(GitError::InvalidRemote);
    }
    let branch = match runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["symbolic-ref", "--short", "HEAD"],
        None,
        Limits::METADATA,
    ) {
        Ok(output) => String::from_utf8(output.stdout)
            .map_err(|_| GitError::UnrepresentableName)?
            .trim()
            .to_owned(),
        Err(GitError::Exit { .. }) => return Err(GitError::DetachedHead),
        Err(error) => return Err(error),
    };
    let known = remotes::list(git, repo)?;
    if !known.iter().any(|entry| entry.name == remote) {
        return Err(GitError::UnknownRemote);
    }
    let reference = format!("refs/heads/{branch}:refs/heads/{branch}");
    let report = |state: PushState, observed: Option<String>| PushReport {
        remote: remote.to_owned(),
        branch: branch.clone(),
        reference: reference.clone(),
        state,
        observed,
    };
    let output = match runner::run_network(
        git.executable(),
        Some(&repo.worktree_root),
        &["push", "--porcelain", "--no-follow-tags", remote, reference.as_str()],
        cancel,
        sink,
    ) {
        Ok(output) => output,
        // Cancellation and the deadline leave the remote in an unknown state,
        // so the landing ref is observed once, without a new transfer.
        Err(GitError::Cancelled | GitError::Timeout) => {
            let observed = observe(git, repo, remote, &branch);
            let reconciled = observed.is_some() && observed == head(git, repo);
            return Ok(report(PushState::Unknown { reconciled }, observed));
        }
        // Git exits 1 both for a refused ref and for transport or auth
        // failures. Only a report that names the ref update is a rejection;
        // anything else keeps its own error so it is not shown as a policy
        // refusal. stdout is not kept by the runner, so the captured first
        // error line is the discriminator.
        Err(GitError::Exit { code: 1, detail })
            if detail.contains("failed to push some refs")
                || detail.contains("non-fast-forward")
                || detail.contains("fetch first")
                || detail.contains("rejected") =>
        {
            return Ok(report(PushState::Rejected, observe(git, repo, remote, &branch)));
        }
        Err(error) => return Err(error),
    };
    let text = String::from_utf8(output.stdout).map_err(|_| GitError::MalformedOutput { record: 0 })?;
    let state = classify(&text)?;
    // A successful update of this exact ref means the remote now points at the
    // pushed local commit, so the local HEAD is the observed destination.
    let observed = match state {
        PushState::Updated => head(git, repo),
        PushState::Rejected => observe(git, repo, remote, &branch),
        _ => None,
    };
    Ok(report(state, observed))
}

/// Reads the remote's current commit for `branch`, if it can be observed
/// without a transfer. A failed read reports nothing rather than an error,
/// because it only refines a state that is already unknown or rejected.
fn observe(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    remote: &str,
    branch: &str,
) -> Option<String> {
    let reference = format!("refs/heads/{branch}");
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["ls-remote", "--heads", remote, reference.as_str()],
        None,
        Limits::METADATA,
    )
    .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let oid = fields.next()?;
        let name = fields.next()?;
        (name == reference && is_object_id(oid)).then(|| oid.to_owned())
    })
}

fn head(git: &GitInstallation, repo: &RepositoryIdentity) -> Option<String> {
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["rev-parse", "HEAD"],
        None,
        Limits::METADATA,
    )
    .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Reads `git push --porcelain` output. Rejection wins over an update reported
/// for another ref, and an unrecognized report is malformed rather than a
/// claimed success.
fn classify(text: &str) -> Result<PushState, GitError> {
    let mut updated = false;
    let mut up_to_date = false;
    for line in text.lines() {
        let Some((flag, rest)) = line.split_once('\t') else {
            continue;
        };
        let summary = rest.rsplit('\t').next().unwrap_or(rest);
        if summary.contains("[rejected]") || summary.contains("[remote rejected]") {
            return Ok(PushState::Rejected);
        }
        if flag.trim() == "=" || summary.contains("[up to date]") {
            up_to_date = true;
            continue;
        }
        if summary.contains("[new branch]") || summary.contains("..") || summary.contains(" -> ") {
            updated = true;
        }
    }
    if updated {
        return Ok(PushState::Updated);
    }
    if up_to_date {
        return Ok(PushState::UpToDate);
    }
    Err(GitError::MalformedOutput { record: 0 })
}

fn is_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

use std::ffi::OsString;

use crate::{GitError, GitInstallation, RepositoryIdentity, runner};
use srcmgr_domain::NativePath;

const LIMIT: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StashEntry {
    pub id: String,
    pub subject: String,
}

fn run(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    args: &[OsString],
) -> Result<runner::Output, GitError> {
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        args,
        None,
        runner::Limits::MUTATION,
    )
}

pub fn stash_list(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
) -> Result<Vec<StashEntry>, GitError> {
    let output = run(
        git,
        repo,
        &[
            "stash".into(),
            "list".into(),
            format!("--max-count={LIMIT}").into(),
            "--format=%H%x00%gs".into(),
            "-z".into(),
        ],
    )?;
    if output.stdout.is_empty() {
        return Ok(Vec::new());
    }
    if !output.stdout.ends_with(&[0]) {
        return Err(GitError::MalformedOutput { record: 0 });
    }
    let fields: Vec<_> = output.stdout[..output.stdout.len() - 1]
        .split(|b| *b == 0)
        .collect();
    if !fields.len().is_multiple_of(2) {
        return Err(GitError::MalformedOutput { record: 0 });
    }
    fields
        .chunks(2)
        .enumerate()
        .map(|(record, pair)| {
            let id = std::str::from_utf8(pair[0]).map_err(|_| GitError::InvalidRevision)?;
            crate::status::revision(id.as_bytes())?;
            let subject = NativePath::from_tool_bytes(pair[1])
                .map_err(|_| GitError::MalformedOutput { record })?
                .display_label();
            Ok(StashEntry {
                id: id.to_owned(),
                subject,
            })
        })
        .collect()
}

/// Rejects stale lists/duplicate OIDs instead of resolving a moving row number.
pub fn check_stash_selection(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    oid: &str,
    expected: &[String],
) -> Result<usize, GitError> {
    crate::status::revision(oid.as_bytes())?;
    let list = stash_list(git, repo)?;
    let ids: Vec<_> = list.iter().map(|s| s.id.clone()).collect();
    if ids != expected || list.len() >= LIMIT || ids.iter().filter(|id| *id == oid).count() != 1 {
        return Err(GitError::StaleSelection);
    }
    ids.iter()
        .position(|id| id == oid)
        .ok_or(GitError::StaleSelection)
}

pub fn stash_selected(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    oid: &str,
    expected: &[String],
    operation: &str,
    transaction: &std::path::Path,
) -> Result<String, GitError> {
    check_stash_selection(git, repo, oid, expected)?;
    if operation != "drop" {
        if !matches!(operation, "apply" | "pop") {
            return Err(GitError::InvalidName);
        }
        // Always apply immutable content, never a potentially shifted selector.
        run(git, repo, &["stash".into(), "apply".into(), oid.into()])?;
        if operation == "apply" {
            return Ok("Applied selected stash; source retained".to_owned());
        }
        if check_stash_selection(git, repo, oid, expected).is_err() {
            return Ok(
                "Applied selected stash; list changed, source retained (no pop retry)".to_owned(),
            );
        }
    }
    let dropped = crate::stash_removal::remove(git, repo, oid, expected, transaction);
    match dropped {
        Ok(_) => Ok(if operation == "pop" {
            "Applied selected stash and removed its source entry"
        } else {
            "Dropped selected stash entry"
        }
        .to_owned()),
        Err(error) if operation == "pop" => Ok(format!(
            "Applied selected stash; source removal failed or is uncertain; inspect refreshed list and recovery transaction, do not replay apply: {error}"
        )),
        Err(error) => Err(error),
    }
}

pub fn stash_diff(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    oid: &str,
) -> Result<String, GitError> {
    crate::status::revision(oid.as_bytes())?;
    let parent = run(git, repo, &["rev-parse".into(), format!("{oid}^1").into()])?;
    let parent = std::str::from_utf8(&parent.stdout)
        .map_err(|_| GitError::InvalidRevision)?
        .trim();
    crate::status::revision(parent.as_bytes())?;
    let output = run(
        git,
        repo,
        &[
            "stash".into(),
            "show".into(),
            "--include-untracked".into(),
            "--patch".into(),
            "--no-color".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            oid.into(),
        ],
    )?;
    Ok(format!(
        "Stash {oid}\nBase: {parent}\nWorktree and captured untracked payload vs base\n{}",
        String::from_utf8_lossy(&output.stdout)
    ))
}

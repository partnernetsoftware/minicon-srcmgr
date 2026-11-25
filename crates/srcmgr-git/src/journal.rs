//! A one-line record of the last mutation this window started (PRD 08, PRD 13).
//!
//! The record exists for one question: did a job that touched this repository
//! finish? It is written before the Git command and removed when the command
//! returns, so a record found at startup means the process died mid-flight or
//! lost the repository. Git itself may then hold a half-applied state, and the
//! only honest report is "needs reconciliation" until the user looks.
//!
//! It lives next to the repository's common directory, never in the worktree,
//! and holds an action name and a timestamp — no paths, no content, no remote.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::repository::RepositoryIdentity;

/// A job that started and never reported its end.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingJob {
    /// The action name, such as merge or conflict-resolution.
    pub action: String,
    /// Seconds since the Unix epoch when it started.
    pub started: u64,
}

const FILE: &str = "srcmgr-job";

fn path(repo: &RepositoryIdentity) -> std::path::PathBuf {
    repo.common_dir.join(FILE)
}

fn checked_action(action: &str) -> Result<&str, GitError> {
    if action.is_empty()
        || action.len() > 64
        || !action
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(GitError::Journal("the action name is not a short lowercase label"));
    }
    Ok(action)
}

/// Records that a mutation is starting. Nothing is written when this fails, so
/// a caller can treat the error as a refusal to act.
pub fn begin(
    _git: &GitInstallation,
    repo: &RepositoryIdentity,
    action: &str,
) -> Result<(), GitError> {
    let action = checked_action(action)?;
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| GitError::Journal("the system clock is before the Unix epoch"))?
        .as_secs();
    let target = path(repo);
    let temporary = repo.common_dir.join(format!("{FILE}.tmp"));
    std::fs::write(&temporary, format!("{action} {started}\n"))
        .map_err(|_| GitError::Journal("the job record could not be written"))?;
    std::fs::rename(&temporary, &target)
        .map_err(|_| GitError::Journal("the job record could not be published"))?;
    Ok(())
}

/// Removes the record after the command returned, whatever it returned.
pub fn finish(_git: &GitInstallation, repo: &RepositoryIdentity) -> Result<(), GitError> {
    match std::fs::remove_file(path(repo)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(GitError::Journal("the job record could not be cleared")),
    }
}

/// Reads a record left by a process that did not finish its job.
pub fn pending(
    _git: &GitInstallation,
    repo: &RepositoryIdentity,
) -> Result<Option<PendingJob>, GitError> {
    let text = match std::fs::read_to_string(path(repo)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(GitError::Journal("the job record could not be read")),
    };
    let mut fields = text.split_whitespace();
    let (Some(action), Some(started)) = (fields.next(), fields.next()) else {
        return Err(GitError::Journal("the job record is not in the expected form"));
    };
    let started = started
        .parse::<u64>()
        .map_err(|_| GitError::Journal("the job record has no usable timestamp"))?;
    Ok(Some(PendingJob { action: action.to_owned(), started }))
}

/// Accepts the current repository state after the user inspected it.
///
/// Returns whether a record was there; the caller reports that, so an
/// acknowledgement of nothing is never shown as a recovery.
pub fn acknowledge(_git: &GitInstallation, repo: &RepositoryIdentity) -> Result<bool, GitError> {
    let existed = path(repo).exists();
    finish(_git, repo)?;
    Ok(existed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    fn git() -> PathBuf {
        std::env::var_os("SRCMGR_TEST_GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from)
    }

    fn repository(name: &str) -> (PathBuf, GitInstallation, RepositoryIdentity) {
        let base = std::env::temp_dir();
        let root = base.join(format!("srcmgr-journal-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for arguments in [vec!["init", "-q", "-b", "main"], vec!["config", "user.name", "F"], vec!["config", "user.email", "f@e.invalid"]] {
            let output = Command::new(git()).current_dir(&root).args(arguments).output().unwrap();
            assert!(output.status.success());
        }
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        for arguments in [vec!["add", "a.txt"], vec!["commit", "-q", "-m", "one"]] {
            let output = Command::new(git()).current_dir(&root).args(arguments).output().unwrap();
            assert!(output.status.success());
        }
        let installation = GitInstallation::probe(git()).unwrap();
        let identity = crate::repository::locate(&installation, &root).unwrap();
        (root, installation, identity)
    }

    #[test]
    fn a_record_left_behind_is_reported_until_it_is_acknowledged() {
        let (root, installation, identity) = repository("pending");
        assert_eq!(pending(&installation, &identity).unwrap(), None);
        begin(&installation, &identity, "merge").unwrap();
        let record = pending(&installation, &identity).unwrap().expect("a record");
        assert_eq!(record.action, "merge");
        assert!(record.started > 0);
        assert!(acknowledge(&installation, &identity).unwrap());
        assert_eq!(pending(&installation, &identity).unwrap(), None);
        assert!(!acknowledge(&installation, &identity).unwrap());
        let _ = root;
    }

    #[test]
    fn a_finished_job_leaves_nothing_behind() {
        let (_root, installation, identity) = repository("finished");
        begin(&installation, &identity, "rebase").unwrap();
        finish(&installation, &identity).unwrap();
        assert_eq!(pending(&installation, &identity).unwrap(), None);
        finish(&installation, &identity).unwrap();
    }

    #[test]
    fn an_unusable_action_name_is_refused_before_anything_is_written() {
        let (root, installation, identity) = repository("action");
        for action in ["", "Merge", "merge now", "merge\n"] {
            assert!(matches!(begin(&installation, &identity, action), Err(GitError::Journal(_))));
        }
        assert!(!root.join(".git/srcmgr-job").exists());
    }
}

//! What Git says is already in progress, and which paths are unresolved.
//!
//! This reads only Git's own metadata and index stages. It never infers an
//! operation from a filename and never guesses that a broken record means
//! "nothing in progress": a record that exists but cannot be read is reported as
//! unreadable so the caller can tell the user to inspect rather than act.

use std::path::Path;

use crate::runner::Limits;
use crate::{GitError, GitInstallation, RepositoryIdentity, runner};

/// An integration Git has started and not finished.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    ApplySeries,
}

impl Operation {
    /// The name the window shows. It is a Git state name, not prose.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
            Self::ApplySeries => "am",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationState {
    /// The operation Git is in the middle of, if any.
    pub operation: Option<Operation>,
    /// Paths the index reports as unmerged, in Git's order.
    pub unmerged: Vec<String>,
    /// A marker exists but could not be read, so the state is unknown rather
    /// than clean. The caller must not treat this as "no operation".
    pub unreadable: Option<String>,
}

impl OperationState {
    /// True when a write would produce a partial or misleading result.
    pub fn blocks_writes(&self) -> bool {
        self.operation.is_some() || !self.unmerged.is_empty() || self.unreadable.is_some()
    }

    /// The refusal reason, or `None` when writes are allowed.
    pub fn refusal(&self) -> Option<String> {
        if let Some(detail) = &self.unreadable {
            return Some(format!(
                "The repository's operation state cannot be read ({detail}); inspect it before any write"
            ));
        }
        let mut parts = Vec::new();
        if let Some(operation) = self.operation {
            parts.push(format!("a {} is in progress", operation.name()));
        }
        if !self.unmerged.is_empty() {
            parts.push(format!("{} path(s) are unmerged", self.unmerged.len()));
        }
        if parts.is_empty() {
            None
        } else {
            Some(format!(
                "Refused: {}. Resolve it in a terminal, then refresh. Press t in the window to open a configured terminal at this repository.",
                parts.join(" and ")
            ))
        }
    }
}

/// Reads the operation markers and the index's unmerged stages.
pub fn detect(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<OperationState, GitError> {
    let marker = |name: &str| -> Result<bool, GitError> {
        match std::fs::symlink_metadata(repo.common_dir.join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(GitError::Io(error)),
            Ok(_) => Ok(true),
        }
    };
    // A rebase leaves either a directory or, for `git am`, a different one.
    let rebase = marker("rebase-merge")? || marker("rebase-apply")?;
    let operation = if marker("MERGE_HEAD")? {
        Some(Operation::Merge)
    } else if rebase {
        Some(Operation::Rebase)
    } else if marker("CHERRY_PICK_HEAD")? {
        Some(Operation::CherryPick)
    } else if marker("REVERT_HEAD")? {
        Some(Operation::Revert)
    } else if marker("rebase-apply/applying")? {
        Some(Operation::ApplySeries)
    } else {
        None
    };

    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["diff", "--name-only", "--diff-filter=U", "-z"],
        None,
        Limits::METADATA,
    )?;
    let mut unmerged = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path = std::str::from_utf8(raw).map_err(|_| GitError::UnrepresentableName)?;
        if Path::new(path).components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            return Err(GitError::UnrepresentableName);
        }
        unmerged.push(path.to_owned());
    }
    Ok(OperationState { operation, unmerged, unreadable: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
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
        let root = base.join(format!("srcmgr-opstate-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn run(dir: &Path, args: &[&str]) {
        let output = Command::new(git()).current_dir(dir).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    fn init(dir: &Path) {
        run(dir, &["init", "-q", "-b", "main"]);
        run(dir, &["config", "user.name", "Fixture"]);
        run(dir, &["config", "user.email", "fixture@example.invalid"]);
    }

    fn state(dir: &Path) -> OperationState {
        let installation = GitInstallation::probe(git()).unwrap();
        let repo = crate::locate(&installation, dir).unwrap();
        detect(&installation, &repo).unwrap()
    }

    #[test]
    fn a_clean_repository_blocks_nothing() {
        let root = fixture("clean");
        init(&root);
        std::fs::write(root.join("a.txt"), "one
").unwrap();
        run(&root, &["add", "a.txt"]);
        run(&root, &["commit", "-qm", "one"]);
        let state = state(&root);
        assert_eq!(state.operation, None);
        assert!(state.unmerged.is_empty());
        assert!(!state.blocks_writes());
        assert_eq!(state.refusal(), None);
    }

    #[test]
    fn a_merge_conflict_is_named_with_its_unmerged_path() {
        let root = fixture("merge");
        init(&root);
        std::fs::write(root.join("a.txt"), "base
").unwrap();
        run(&root, &["add", "a.txt"]);
        run(&root, &["commit", "-qm", "base"]);
        run(&root, &["checkout", "-q", "-b", "other"]);
        std::fs::write(root.join("a.txt"), "other
").unwrap();
        run(&root, &["commit", "-qam", "other"]);
        run(&root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("a.txt"), "main
").unwrap();
        run(&root, &["commit", "-qam", "main"]);
        let output = Command::new(git()).current_dir(&root).args(["merge", "other"]).output().unwrap();
        assert!(!output.status.success(), "the fixture must actually conflict");
        let state = state(&root);
        assert_eq!(state.operation, Some(Operation::Merge));
        assert_eq!(state.unmerged, vec!["a.txt".to_owned()]);
        assert!(state.blocks_writes());
        let refusal = state.refusal().unwrap();
        assert!(refusal.contains("merge"), "{refusal}");
        assert!(refusal.contains("1 path(s) are unmerged"), "{refusal}");
    }

    #[test]
    fn a_cherry_pick_conflict_is_named_as_a_cherry_pick() {
        let root = fixture("pick");
        init(&root);
        std::fs::write(root.join("a.txt"), "base
").unwrap();
        run(&root, &["add", "a.txt"]);
        run(&root, &["commit", "-qm", "base"]);
        run(&root, &["checkout", "-q", "-b", "other"]);
        std::fs::write(root.join("a.txt"), "other
").unwrap();
        run(&root, &["commit", "-qam", "other"]);
        let pick = String::from_utf8(
            Command::new(git()).current_dir(&root).args(["rev-parse", "HEAD"]).output().unwrap().stdout,
        )
        .unwrap()
        .trim()
        .to_owned();
        run(&root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("a.txt"), "main
").unwrap();
        run(&root, &["commit", "-qam", "main"]);
        let output = Command::new(git()).current_dir(&root).args(["cherry-pick", &pick]).output().unwrap();
        assert!(!output.status.success(), "the fixture must actually conflict");
        let state = state(&root);
        assert_eq!(state.operation, Some(Operation::CherryPick));
        assert!(state.refusal().unwrap().contains("cherry-pick"));
    }
}

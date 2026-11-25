//! Reviewed merge previews (PRD 08 conflict center, PRD 06 references).
//!
//! A preview answers one question without changing what the user can see:
//! what would merging this target into the current branch do right now? It
//! resolves the target to a commit, compares it with HEAD and asks Git's own
//! merge machinery which paths would conflict.
//!
//! Predicting a conflict with \u0060merge-tree --write-tree\u0060 writes unreferenced
//! objects into the object database. That is the one side effect a preview has,
//! so it is stated here and in the preview's own documentation instead of being
//! hidden: refs, the index and the worktree are untouched, and no merge state
//! file is created.

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::repository::RepositoryIdentity;
use crate::runner::{self, Limits};

/// What merging one target into the current branch would do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergeOutcome {
    /// The target is already reachable from HEAD; nothing would move.
    UpToDate,
    /// HEAD is an ancestor of the target: the branch moves without a commit.
    FastForward,
    /// Both sides advanced and a merge commit would be created.
    RealMerge,
    /// Both sides advanced and at least one path would conflict.
    Conflicts,
}

/// The reviewed effect of merging a target into the current branch.
///
/// The preview never merges, never writes the index and never creates merge
/// state; \u0060git merge\u0060 runs only after the caller approves these values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergePreview {
    pub outcome: MergeOutcome,
    /// The branch the merge would move.
    pub branch: String,
    /// HEAD as it is now.
    pub current: String,
    /// The commit the target resolves to.
    pub target: String,
    /// The best common ancestor the two commits share.
    pub base: String,
    /// Commits the target adds on top of the base.
    pub commits: usize,
    /// Paths Git reports as conflicting; empty unless \u0060outcome\u0060 is \u0060Conflicts\u0060.
    pub conflicted: Vec<String>,
    /// Whether the worktree or index already holds changes beside the merge.
    pub worktree_dirty: bool,
}

/// Rejects a target that could change what Git runs.
fn checked_target(target: &str) -> Result<&str, GitError> {
    if target.is_empty()
        || target.starts_with('-')
        || target.chars().any(char::is_control)
        || target.trim() != target
    {
        return Err(GitError::IntegrationTarget(
            "the merge target is empty, option-like, padded or contains a control character",
        ));
    }
    Ok(target)
}

fn read_line(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    arguments: &[&str],
) -> Result<String, GitError> {
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        arguments,
        None,
        Limits::METADATA,
    )?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Reads the working tree's dirtiness as a yes/no, without reading its content.
fn worktree_dirty(git: &GitInstallation, repo: &RepositoryIdentity) -> Result<bool, GitError> {
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["status", "--porcelain=v1", "-z"],
        None,
        Limits::STATUS,
    )?;
    Ok(!output.stdout.is_empty())
}

/// Splits \u0060merge-tree -z\u0060 output into its tree id and the conflicted paths.
fn conflicted_paths(stdout: &[u8]) -> Vec<String> {
    let mut fields: Vec<&[u8]> = stdout.split(|byte| *byte == 0).collect();
    if fields.first().is_some_and(|field| field.is_empty()) {
        fields.remove(0);
    }
    // The first field is the written tree OID, not a path.
    fields
        .into_iter()
        .skip(1)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect()
}

/// Previews merging \u0060target\u0060 into the current branch.
///
/// A detached HEAD is refused because there is no branch for Git to move; the
/// caller merges only a branch. Unrelated histories are refused by name rather
/// than reported as Git's exit code.
pub fn preview_merge(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    target: &str,
) -> Result<MergePreview, GitError> {
    let target = checked_target(target)?;

    let branch = read_line(git, repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch == "HEAD" {
        return Err(GitError::DetachedHead);
    }
    let current = read_line(git, repo, &["rev-parse", "HEAD"])?;
    let target_oid = read_line(
        git,
        repo,
        &["rev-parse", "--verify", &format!("{target}^{{commit}}")],
    )?;

    let base = match runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["merge-base", "HEAD", &target_oid],
        None,
        Limits::METADATA,
    ) {
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        Err(GitError::Exit { .. }) => {
            return Err(GitError::IntegrationTarget(
                "the two histories share no commit, so there is no merge base",
            ))
        }
        Err(other) => return Err(other),
    };

    let commits = if base == target_oid {
        0
    } else {
        read_line(
            git,
            repo,
            &["rev-list", "--count", &format!("{base}..{target_oid}")],
        )?
        .parse::<usize>()
        .map_err(|_| GitError::MalformedOutput { record: 0 })?
    };

    let (outcome, conflicted) = if base == target_oid {
        (MergeOutcome::UpToDate, Vec::new())
    } else if base == current {
        (MergeOutcome::FastForward, Vec::new())
    } else {
        let output = runner::run_allowing_failure(
            git.executable(),
            Some(&repo.worktree_root),
            &[
                "merge-tree",
                "--write-tree",
                "--name-only",
                "--no-messages",
                "-z",
                "HEAD",
                &target_oid,
            ],
            Limits::STATUS,
        )?;
        let paths = conflicted_paths(&output.stdout);
        if output.code == 0 {
            (MergeOutcome::RealMerge, Vec::new())
        } else {
            (MergeOutcome::Conflicts, paths)
        }
    };

    Ok(MergePreview {
        outcome,
        branch,
        current,
        target: target_oid,
        base,
        commits,
        conflicted,
        worktree_dirty: worktree_dirty(git, repo)?,
    })
}
/// The actual result of an approved merge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeReport {
    /// HEAD before the merge.
    pub before: String,
    /// HEAD after the merge; equal to \`before\` when the merge stopped.
    pub after: String,
    /// Paths Git left unmerged; empty unless the merge stopped on conflicts.
    pub conflicted: Vec<String>,
    /// True when the branch moved. A conflict stop leaves it where it was.
    pub moved: bool,
}

/// Merges the previewed target into the current branch.
///
/// The preview is the permission: the caller approves its values and this
/// function revalidates them immediately before writing. A moved HEAD, a target
/// that no longer resolves to the reviewed commit, or an operation Git already
/// has unfinished refuses instead of merging something the user did not see.
///
/// A non-zero exit from \`git merge\` is a conflict stop only when Git left merge
/// state behind; anything else is reported with Git's own first error line.
pub fn apply_merge(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    preview: &MergePreview,
) -> Result<MergeReport, GitError> {
    let state = crate::operation_state::detect(git, repo)?;
    if state.blocks_writes() {
        return Err(GitError::OperationInProgress);
    }

    let current = read_line(git, repo, &["rev-parse", "HEAD"])?;
    if current != preview.current {
        return Err(GitError::StaleMergePreview);
    }
    let target = read_line(
        git,
        repo,
        &["rev-parse", "--verify", &format!("{}^{{commit}}", preview.target)],
    )?;
    if target != preview.target {
        return Err(GitError::StaleMergePreview);
    }

    crate::journal::begin(git, repo, "merge")?;
    let output = runner::run_allowing_failure(
        git.executable(),
        Some(&repo.worktree_root),
        &["merge", "--no-edit", &preview.target],
        Limits::MUTATION,
    )?;

    if output.code == 0 {
        let after = read_line(git, repo, &["rev-parse", "HEAD"])?;
        crate::journal::finish(git, repo)?;
        return Ok(MergeReport {
            moved: after != preview.current,
            before: preview.current.clone(),
            after,
            conflicted: Vec::new(),
        });
    }

    let after = crate::operation_state::detect(git, repo)?;
    if after.operation.is_some() || !after.unmerged.is_empty() {
        crate::journal::finish(git, repo)?;
        return Ok(MergeReport {
            before: preview.current.clone(),
            after: current,
            conflicted: after.unmerged,
            moved: false,
        });
    }
    let _ = crate::journal::finish(git, repo);
    Err(GitError::Exit { code: output.code, detail: output.detail })
}

/// The reviewed effect of moving the current branch onto another commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebasePreview {
    /// The branch that would be rewound and replayed.
    pub branch: String,
    /// HEAD as it is now.
    pub current: String,
    /// The commit the branch would be replayed onto.
    pub onto: String,
    /// The common ancestor the replay starts from.
    pub base: String,
    /// Commits that would be replayed onto \u0060onto\u0060.
    pub commits: usize,
    /// Whether the worktree or index already holds changes beside the rebase.
    pub worktree_dirty: bool,
}

/// The actual result of an approved rebase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RebaseReport {
    /// HEAD before the rebase.
    pub before: String,
    /// HEAD after the rebase; equal to \u0060before\u0060 when it stopped.
    pub after: String,
    /// Paths Git left unmerged.
    pub conflicted: Vec<String>,
    /// True when the branch was rewritten. A conflict stop leaves it part-way.
    pub moved: bool,
}

/// Previews replaying the current branch onto \u0060onto\u0060.
pub fn preview_rebase(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    onto: &str,
) -> Result<RebasePreview, GitError> {
    let onto = checked_target(onto)?;
    let branch = read_line(git, repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch == "HEAD" {
        return Err(GitError::DetachedHead);
    }
    let current = read_line(git, repo, &["rev-parse", "HEAD"])?;
    let onto_oid = read_line(
        git,
        repo,
        &["rev-parse", "--verify", &format!("{onto}^{{commit}}")],
    )?;
    let base = match runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["merge-base", "HEAD", &onto_oid],
        None,
        Limits::METADATA,
    ) {
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        Err(GitError::Exit { .. }) => {
            return Err(GitError::IntegrationTarget(
                "the two histories share no commit, so there is nothing to replay onto it",
            ))
        }
        Err(other) => return Err(other),
    };
    let commits = if base == current {
        0
    } else {
        read_line(
            git,
            repo,
            &["rev-list", "--count", &format!("{base}..{current}")],
        )?
        .parse::<usize>()
        .map_err(|_| GitError::MalformedOutput { record: 0 })?
    };
    Ok(RebasePreview {
        branch,
        current,
        onto: onto_oid,
        base,
        commits,
        worktree_dirty: worktree_dirty(git, repo)?,
    })
}

/// Replays the reviewed branch onto the reviewed commit.
///
/// \u0060git rebase\u0060 exits non-zero when it stops on a conflict and when it refuses;
/// the difference is whether a rebase directory exists afterwards, so the state
/// is re-read before the exit is reported as an error.
pub fn apply_rebase(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    preview: &RebasePreview,
) -> Result<RebaseReport, GitError> {
    let state = crate::operation_state::detect(git, repo)?;
    if state.blocks_writes() {
        return Err(GitError::OperationInProgress);
    }
    let current = read_line(git, repo, &["rev-parse", "HEAD"])?;
    if current != preview.current {
        return Err(GitError::StaleMergePreview);
    }
    let onto = read_line(
        git,
        repo,
        &["rev-parse", "--verify", &format!("{}^{{commit}}", preview.onto)],
    )?;
    if onto != preview.onto {
        return Err(GitError::StaleMergePreview);
    }

    crate::journal::begin(git, repo, "rebase")?;
    let output = runner::run_allowing_failure(
        git.executable(),
        Some(&repo.worktree_root),
        &["rebase", &preview.onto],
        Limits::MUTATION,
    )?;

    let after_state = crate::operation_state::detect(git, repo)?;
    let stopped = after_state.operation.is_some() || !after_state.unmerged.is_empty();
    if output.code == 0 && !stopped {
        let after = read_line(git, repo, &["rev-parse", "HEAD"])?;
        crate::journal::finish(git, repo)?;
        return Ok(RebaseReport {
            moved: after != preview.current,
            before: preview.current.clone(),
            after,
            conflicted: Vec::new(),
        });
    }
    if stopped {
        crate::journal::finish(git, repo)?;
        return Ok(RebaseReport {
            before: preview.current.clone(),
            after: read_line(git, repo, &["rev-parse", "HEAD"])?,
            conflicted: after_state.unmerged,
            moved: false,
        });
    }
    Err(GitError::Exit { code: output.code, detail: output.detail })
}

/// What an abort would restore, read before anything is undone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AbortPreview {
    /// The operation Git reports, which is the one an abort would undo.
    pub operation: crate::operation_state::Operation,
    /// The branch the repository is on, when it has one.
    pub branch: String,
    /// HEAD while the operation is unfinished.
    pub head: String,
    /// The commit the branch is restored to; \u0060ORIG_HEAD\u0060 for both operations.
    pub restore_to: String,
    /// Paths still unmerged.
    pub unmerged: Vec<String>,
}

/// The actual result of an abort.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AbortReport {
    pub operation: crate::operation_state::Operation,
    pub before: String,
    pub after: String,
    /// True only when Git no longer reports an unfinished operation afterwards.
    pub restored: bool,
}

/// The result of continuing an operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContinueReport {
    pub operation: crate::operation_state::Operation,
    pub before: String,
    pub after: String,
}

fn operation_verb(operation: crate::operation_state::Operation) -> Result<&'static str, GitError> {
    match operation {
        crate::operation_state::Operation::Merge => Ok("merge"),
        crate::operation_state::Operation::Rebase => Ok("rebase"),
        _ => Err(GitError::IntegrationTarget(
            "only a merge or a rebase can be continued or aborted from the window",
        )),
    }
}

/// Describes what aborting the unfinished operation would restore.
pub fn preview_abort(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
) -> Result<AbortPreview, GitError> {
    let state = crate::operation_state::detect(git, repo)?;
    let Some(operation) = state.operation else {
        return Err(GitError::NoOperationInProgress);
    };
    operation_verb(operation)?;
    let branch = read_line(git, repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = read_line(git, repo, &["rev-parse", "HEAD"])?;
    let restore_to = read_line(git, repo, &["rev-parse", "ORIG_HEAD"]).map_err(|_| {
        GitError::IntegrationTarget("Git recorded no ORIG_HEAD, so the pre-operation commit is unknown")
    })?;
    Ok(AbortPreview {
        operation,
        branch,
        head,
        restore_to,
        unmerged: state.unmerged,
    })
}

/// Aborts the reviewed operation and re-reads the state afterwards.
///
/// A failed abort is reported as not restored; it is never called a rollback.
pub fn apply_abort(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    preview: &AbortPreview,
) -> Result<AbortReport, GitError> {
    let state = crate::operation_state::detect(git, repo)?;
    match state.operation {
        None => return Err(GitError::NoOperationInProgress),
        Some(operation) if operation == preview.operation => {}
        Some(_) => return Err(GitError::StaleOperationPreview),
    }
    let verb = operation_verb(preview.operation)?;
    let before = read_line(git, repo, &["rev-parse", "HEAD"])?;
    crate::journal::begin(git, repo, "abort")?;
    runner::run_allowing_failure(
        git.executable(),
        Some(&repo.worktree_root),
        &[verb, "--abort"],
        Limits::MUTATION,
    )?;
    let after_state = crate::operation_state::detect(git, repo)?;
    crate::journal::finish(git, repo)?;
    Ok(AbortReport {
        operation: preview.operation,
        before: before.clone(),
        after: read_line(git, repo, &["rev-parse", "HEAD"])?,
        restored: !after_state.blocks_writes(),
    })
}

/// Continues the unfinished operation after its conflicts were resolved.
///
/// Git refuses while an unmerged entry remains, and that refusal is returned as
/// Git's own first error line rather than replaced with a summary.
pub fn continue_operation(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
) -> Result<ContinueReport, GitError> {
    let state = crate::operation_state::detect(git, repo)?;
    let Some(operation) = state.operation else {
        return Err(GitError::NoOperationInProgress);
    };
    let verb = operation_verb(operation)?;
    let before = read_line(git, repo, &["rev-parse", "HEAD"])?;
    crate::journal::begin(git, repo, "continue")?;
    let output = runner::run_allowing_failure(
        git.executable(),
        Some(&repo.worktree_root),
        &["-c", "core.editor=true", verb, "--continue"],
        Limits::MUTATION,
    )?;
    if output.code != 0 {
        return Err(GitError::Exit { code: output.code, detail: output.detail });
    }
    crate::journal::finish(git, repo)?;
    Ok(ContinueReport {
        operation,
        before,
        after: read_line(git, repo, &["rev-parse", "HEAD"])?,
    })
}

/// What skipping the current step would omit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkipPreview {
    pub operation: crate::operation_state::Operation,
    /// The commit the sequencer is stopped on, which the skip discards.
    pub omitting: String,
    /// Its subject, when Git can read it.
    pub subject: String,
}

/// The result of skipping one step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkipReport {
    pub omitting: String,
    pub after: String,
    /// True when Git still has a sequencer afterwards (more commits to replay).
    pub still_in_progress: bool,
}

/// Describes the step a skip would discard. Nothing is skipped.
pub fn preview_skip(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
) -> Result<SkipPreview, GitError> {
    let state = crate::operation_state::detect(git, repo)?;
    match state.operation {
        Some(crate::operation_state::Operation::Rebase) => {}
        Some(_) => {
            return Err(GitError::ConflictResolution(
                "only a rebase has a step to skip; a merge is continued or aborted",
            ))
        }
        None => return Err(GitError::NoOperationInProgress),
    }
    // REBASE_HEAD is the commit the sequencer stopped on.
    let omitting = read_line(git, repo, &["rev-parse", "REBASE_HEAD"]).map_err(|_| {
        GitError::ConflictResolution("Git does not report the commit this step is applying")
    })?;
    let subject = read_line(git, repo, &["log", "-1", "--format=%s", &omitting]).unwrap_or_default();
    Ok(SkipPreview {
        operation: crate::operation_state::Operation::Rebase,
        omitting,
        subject,
    })
}

/// Skips the reviewed step and re-reads the sequencer afterwards.
pub fn apply_skip(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    preview: &SkipPreview,
) -> Result<SkipReport, GitError> {
    let current = preview_skip(git, repo)?;
    if current.omitting != preview.omitting {
        return Err(GitError::StaleOperationPreview);
    }
    crate::journal::begin(git, repo, "skip")?;
    let output = runner::run_allowing_failure(
        git.executable(),
        Some(&repo.worktree_root),
        &["-c", "core.editor=true", "rebase", "--skip"],
        Limits::MUTATION,
    )?;
    if output.code != 0 {
        return Err(GitError::Exit { code: output.code, detail: output.detail });
    }
    let after_state = crate::operation_state::detect(git, repo)?;
    crate::journal::finish(git, repo)?;
    Ok(SkipReport {
        omitting: preview.omitting.clone(),
        after: read_line(git, repo, &["rev-parse", "HEAD"])?,
        still_in_progress: after_state.operation.is_some(),
    })
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
        let root = base.join(format!("srcmgr-integrate-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn run(dir: &Path, args: &[&str]) {
        let output = Command::new(git()).current_dir(dir).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    fn commit(dir: &Path, message: &str) {
        run(dir, &["add", "-A"]);
        run(dir, &["commit", "-q", "-m", message]);
    }

    /// A repository with one commit on `main` holding `a.txt`.
    fn base(name: &str) -> (PathBuf, GitInstallation) {
        let root = fixture(name);
        run(&root, &["init", "-q", "-b", "main"]);
        run(&root, &["config", "user.name", "Fixture"]);
        run(&root, &["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        commit(&root, "one");
        (root, GitInstallation::probe(git()).unwrap())
    }

    fn identity(root: &Path, installation: &GitInstallation) -> RepositoryIdentity {
        crate::repository::locate(installation, root).unwrap()
    }

    fn head(root: &Path) -> String {
        let output = Command::new(git()).current_dir(root).args(["rev-parse", "HEAD"]).output().unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// A feature branch with one extra commit the base does not have.
    fn feature(root: &Path) -> GitInstallation {
        run(root, &["switch", "-q", "-c", "feature"]);
        std::fs::write(root.join("b.txt"), "two\n").unwrap();
        commit(root, "two");
        run(root, &["switch", "-q", "main"]);
        GitInstallation::probe(git()).unwrap()
    }

    #[test]
    fn a_target_already_in_head_moves_nothing() {
        let (root, installation) = base("uptodate");
        let repo = identity(&root, &installation);
        let preview = preview_merge(&installation, &repo, "main").unwrap();
        assert_eq!(preview.outcome, MergeOutcome::UpToDate);
        assert_eq!(preview.commits, 0);
        assert_eq!(preview.current, preview.target);
        assert!(!preview.worktree_dirty);
    }

    #[test]
    fn a_feature_ahead_of_main_is_a_fast_forward_and_head_does_not_move() {
        let (root, _) = base("ff");
        let installation = feature(&root);
        let repo = identity(&root, &installation);
        let before = head(&root);
        let preview = preview_merge(&installation, &repo, "feature").unwrap();
        assert_eq!(preview.outcome, MergeOutcome::FastForward);
        assert_eq!(preview.branch, "main");
        assert_eq!(preview.commits, 1);
        assert!(preview.conflicted.is_empty());
        assert_eq!(preview.current, before);
        assert_eq!(head(&root), before, "a preview must not move the branch");
        assert!(!root.join(".git/MERGE_HEAD").exists(), "a preview must not start a merge");
    }

    #[test]
    fn both_sides_moving_on_different_paths_is_a_real_merge() {
        let (root, _) = base("merge");
        let installation = feature(&root);
        std::fs::write(root.join("a.txt"), "one\nmain\n").unwrap();
        commit(&root, "main edit");
        let repo = identity(&root, &installation);
        let before = head(&root);
        let preview = preview_merge(&installation, &repo, "feature").unwrap();
        assert_eq!(preview.outcome, MergeOutcome::RealMerge);
        assert_eq!(preview.commits, 1);
        assert!(preview.conflicted.is_empty());
        assert!(!preview.worktree_dirty);
        assert_eq!(head(&root), before);
    }

    #[test]
    fn both_sides_editing_one_path_names_the_path_without_starting_a_merge() {
        let (root, _) = base("conflict");
        let installation = feature(&root);
        std::fs::write(root.join("a.txt"), "one\nmain\n").unwrap();
        commit(&root, "main edit");
        run(&root, &["switch", "-q", "feature"]);
        std::fs::write(root.join("a.txt"), "one\nfeature\n").unwrap();
        commit(&root, "feature edit");
        run(&root, &["switch", "-q", "main"]);
        let repo = identity(&root, &installation);
        let preview = preview_merge(&installation, &repo, "feature").unwrap();
        assert_eq!(preview.outcome, MergeOutcome::Conflicts);
        assert_eq!(preview.conflicted, vec!["a.txt".to_owned()]);
        assert!(!root.join(".git/MERGE_HEAD").exists());
        let status = Command::new(git())
            .current_dir(&root)
            .args(["status", "--porcelain=v1", "-z"])
            .output()
            .unwrap();
        assert!(status.stdout.is_empty(), "a preview must not dirty the worktree");
    }

    #[test]
    fn an_option_like_or_padded_target_is_refused_before_git_runs() {
        let (root, installation) = base("target");
        let repo = identity(&root, &installation);
        for target in ["--force", "", " main", "main\t"] {
            assert!(
                matches!(
                    preview_merge(&installation, &repo, target),
                    Err(GitError::IntegrationTarget(_))
                ),
                "{target:?}"
            );
        }
    }

    #[test]
    fn an_approved_fast_forward_moves_the_branch_to_the_reviewed_commit() {
        let (root, _) = base("apply-ff");
        let installation = feature(&root);
        let repo = identity(&root, &installation);
        let preview = preview_merge(&installation, &repo, "feature").unwrap();
        let report = apply_merge(&installation, &repo, &preview).unwrap();
        assert!(report.moved);
        assert_eq!(report.before, preview.current);
        assert_eq!(report.after, preview.target);
        assert_eq!(head(&root), preview.target);
        assert!(report.conflicted.is_empty());
    }

    #[test]
    fn a_conflicted_merge_stops_with_the_paths_and_moves_nothing() {
        let (root, _) = base("apply-conflict");
        let installation = feature(&root);
        std::fs::write(root.join("a.txt"), "one\nmain\n").unwrap();
        commit(&root, "main edit");
        run(&root, &["switch", "-q", "feature"]);
        std::fs::write(root.join("a.txt"), "one\nfeature\n").unwrap();
        commit(&root, "feature edit");
        run(&root, &["switch", "-q", "main"]);
        let repo = identity(&root, &installation);
        let preview = preview_merge(&installation, &repo, "feature").unwrap();
        let before = head(&root);
        let report = apply_merge(&installation, &repo, &preview).unwrap();
        assert!(!report.moved);
        assert_eq!(report.after, before);
        assert_eq!(report.conflicted, vec!["a.txt".to_owned()]);
        assert!(root.join(".git/MERGE_HEAD").exists(), "a conflict stop leaves merge state");
        assert_eq!(head(&root), before);
        assert!(matches!(
            apply_merge(&installation, &repo, &preview),
            Err(GitError::OperationInProgress)
        ));
    }

    #[test]
    fn a_repository_that_moved_after_the_preview_is_not_merged() {
        let (root, _) = base("stale");
        let installation = feature(&root);
        let repo = identity(&root, &installation);
        let preview = preview_merge(&installation, &repo, "feature").unwrap();
        std::fs::write(root.join("c.txt"), "three\n").unwrap();
        commit(&root, "moved on");
        assert!(matches!(
            apply_merge(&installation, &repo, &preview),
            Err(GitError::StaleMergePreview)
        ));
        assert_ne!(head(&root), preview.target);
    }

    #[test]
    fn a_rebase_preview_names_the_commits_it_would_replay() {
        let (root, installation) = base("rebase-preview");
        let _ = feature(&root);
        run(&root, &["switch", "-q", "feature"]);
        let repo = identity(&root, &installation);
        let preview = preview_rebase(&installation, &repo, "main").unwrap();
        assert_eq!(preview.branch, "feature");
        assert_eq!(preview.commits, 1);
        assert_eq!(preview.onto.len(), 40, "the review carries a full object id");
        assert!(!preview.worktree_dirty);
        assert!(!root.join(".git/rebase-merge").exists());
    }

    #[test]
    fn an_approved_rebase_replays_the_branch_and_leaves_no_sequencer() {
        let (root, _) = base("rebase-apply");
        let installation = feature(&root);
        std::fs::write(root.join("c.txt"), "three\n").unwrap();
        commit(&root, "main moves on");
        run(&root, &["switch", "-q", "feature"]);
        let repo = identity(&root, &installation);
        let preview = preview_rebase(&installation, &repo, "main").unwrap();
        let before = head(&root);
        let report = apply_rebase(&installation, &repo, &preview).unwrap();
        assert!(report.moved);
        assert_eq!(report.before, before);
        assert_ne!(report.after, before);
        assert_eq!(head(&root), report.after);
        assert!(report.conflicted.is_empty());
        assert!(!root.join(".git/rebase-merge").exists());
        assert!(!root.join(".git/rebase-apply").exists());
        assert_eq!(std::fs::read_to_string(root.join("c.txt")).unwrap(), "three\n");
    }

    #[test]
    fn a_conflicted_rebase_stops_and_reports_the_path() {
        let (root, _) = base("rebase-conflict");
        let _ = feature(&root);
        std::fs::write(root.join("a.txt"), "one\nmain\n").unwrap();
        commit(&root, "main edit");
        run(&root, &["switch", "-q", "feature"]);
        std::fs::write(root.join("a.txt"), "one\nfeature\n").unwrap();
        commit(&root, "feature edit");
        let installation = GitInstallation::probe(git()).unwrap();
        let repo = identity(&root, &installation);
        let preview = preview_rebase(&installation, &repo, "main").unwrap();
        let report = apply_rebase(&installation, &repo, &preview).unwrap();
        assert!(!report.moved);
        assert_eq!(report.conflicted, vec!["a.txt".to_owned()]);
        assert!(root.join(".git/rebase-merge").exists() || root.join(".git/rebase-apply").exists());
        assert!(matches!(
            apply_rebase(&installation, &repo, &preview),
            Err(GitError::OperationInProgress)
        ));
        run(&root, &["rebase", "--abort"]);
    }

    #[test]
    fn a_branch_that_moved_after_the_rebase_preview_is_not_replayed() {
        let (root, installation) = base("rebase-stale");
        let _ = feature(&root);
        run(&root, &["switch", "-q", "feature"]);
        let repo = identity(&root, &installation);
        let preview = preview_rebase(&installation, &repo, "main").unwrap();
        std::fs::write(root.join("d.txt"), "four\n").unwrap();
        commit(&root, "moved on");
        assert!(matches!(
            apply_rebase(&installation, &repo, &preview),
            Err(GitError::StaleMergePreview)
        ));
    }

    /// A repository stopped in a conflicted merge, with its pre-merge HEAD.
    fn stopped_merge(name: &str) -> (PathBuf, GitInstallation, String) {
        let (root, _) = base(name);
        let installation = feature(&root);
        std::fs::write(root.join("a.txt"), "one\nmain\n").unwrap();
        commit(&root, "main edit");
        run(&root, &["switch", "-q", "feature"]);
        std::fs::write(root.join("a.txt"), "one\nfeature\n").unwrap();
        commit(&root, "feature edit");
        run(&root, &["switch", "-q", "main"]);
        let repo = identity(&root, &installation);
        let preview = preview_merge(&installation, &repo, "feature").unwrap();
        let before = head(&root);
        let report = apply_merge(&installation, &repo, &preview).unwrap();
        assert_eq!(report.conflicted, vec!["a.txt".to_owned()]);
        (root, installation, before)
    }

    #[test]
    fn aborting_a_reviewed_merge_restores_the_pre_merge_head() {
        let (root, installation, before) = stopped_merge("abort-merge");
        let repo = identity(&root, &installation);
        let preview = preview_abort(&installation, &repo).unwrap();
        assert_eq!(preview.operation, crate::operation_state::Operation::Merge);
        assert_eq!(preview.restore_to, before);
        assert_eq!(preview.unmerged, vec!["a.txt".to_owned()]);
        let report = apply_abort(&installation, &repo, &preview).unwrap();
        assert!(report.restored);
        assert_eq!(report.after, before);
        assert_eq!(head(&root), before);
        assert!(!root.join(".git/MERGE_HEAD").exists());
    }

    #[test]
    fn aborting_a_stopped_rebase_restores_the_original_branch() {
        let (root, _) = base("abort-rebase");
        let _ = feature(&root);
        std::fs::write(root.join("a.txt"), "one\nmain\n").unwrap();
        commit(&root, "main edit");
        run(&root, &["switch", "-q", "feature"]);
        std::fs::write(root.join("a.txt"), "one\nfeature\n").unwrap();
        commit(&root, "feature edit");
        let installation = GitInstallation::probe(git()).unwrap();
        let repo = identity(&root, &installation);
        let preview = preview_rebase(&installation, &repo, "main").unwrap();
        let before = head(&root);
        let report = apply_rebase(&installation, &repo, &preview).unwrap();
        assert!(!report.moved);

        let abort = preview_abort(&installation, &repo).unwrap();
        assert_eq!(abort.operation, crate::operation_state::Operation::Rebase);
        let done = apply_abort(&installation, &repo, &abort).unwrap();
        assert!(done.restored);
        assert_eq!(head(&root), before, "the branch is back where it started");
        assert!(!root.join(".git/rebase-merge").exists());
        assert!(!root.join(".git/rebase-apply").exists());
    }

    #[test]
    fn continuing_while_a_path_is_unmerged_is_refused_with_gits_own_line() {
        let (root, installation, before) = stopped_merge("continue-refused");
        let repo = identity(&root, &installation);
        let error = continue_operation(&installation, &repo).expect_err("unmerged paths must refuse");
        assert!(matches!(error, GitError::Exit { .. }), "{error}");
        assert_eq!(head(&root), before);
        assert!(root.join(".git/MERGE_HEAD").exists());
    }

    #[test]
    fn abort_without_an_unfinished_operation_is_refused() {
        let (root, installation) = base("abort-clean");
        let repo = identity(&root, &installation);
        assert!(matches!(
            preview_abort(&installation, &repo),
            Err(GitError::NoOperationInProgress)
        ));
    }

    #[test]
    fn skipping_the_reviewed_step_ends_a_one_commit_rebase() {
        let (root, _) = base("skip-rebase");
        let _ = feature(&root);
        std::fs::write(root.join("a.txt"), "one\nmain\n").unwrap();
        commit(&root, "main edit");
        run(&root, &["switch", "-q", "feature"]);
        std::fs::write(root.join("a.txt"), "one\nfeature\n").unwrap();
        commit(&root, "feature edit");
        let installation = GitInstallation::probe(git()).unwrap();
        let repo = identity(&root, &installation);
        let preview = preview_rebase(&installation, &repo, "main").unwrap();
        let stopped = apply_rebase(&installation, &repo, &preview).unwrap();
        assert!(!stopped.moved);

        let skip = preview_skip(&installation, &repo).unwrap();
        assert_eq!(skip.operation, crate::operation_state::Operation::Rebase);
        assert_eq!(skip.subject, "feature edit");
        let report = apply_skip(&installation, &repo, &skip).unwrap();
        assert!(!report.still_in_progress);
        assert_eq!(report.omitting, skip.omitting);
        let top = read_line(&installation, &repo, &["log", "-1", "--format=%s", "HEAD"]).unwrap();
        assert_eq!(top, "two", "the skipped commit is gone and the earlier one stays");
        assert!(!root.join(".git/rebase-merge").exists() && !root.join(".git/rebase-apply").exists());
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\nmain\n");
    }

    #[test]
    fn a_merge_has_no_step_to_skip() {
        let (root, installation, _before) = stopped_merge("skip-merge");
        let repo = identity(&root, &installation);
        let error = preview_skip(&installation, &repo).expect_err("a merge is not skipped");
        assert!(matches!(error, GitError::ConflictResolution(_)), "{error}");
        run(&root, &["merge", "--abort"]);
    }
}

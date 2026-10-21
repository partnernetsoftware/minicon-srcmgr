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

/// Resolves an exact ref, not an arbitrary revision expression.
pub fn reference_target(git: &GitInstallation, repo: &RepositoryIdentity, reference: &str) -> Result<String, GitError> {
    let output = run_mutation(git, repo, &["show-ref".into(), "--verify".into(), "--hash".into(), reference.into()], None)?;
    let oid = std::str::from_utf8(&output.stdout).map_err(|_| GitError::InvalidRevision)?.trim();
    revision(oid.as_bytes())?;
    Ok(oid.to_owned())
}

/// Creates a new recovery ref; an existing record is never overwritten.
pub fn retain_recovery_ref(git: &GitInstallation, repo: &RepositoryIdentity, ticket: &str, oid: &str) -> Result<(), GitError> {
    if ticket.len() != 32 || !ticket.bytes().all(|b| b.is_ascii_hexdigit()) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(GitError::InvalidRevision);
    }
    run_mutation(git, repo, &["update-ref".into(), format!("refs/srcmgr/recovery/{ticket}").into(), oid.into(), "".into()], None)?;
    Ok(())
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
    apply_patch(installation, identity, &format!("{}{hunk}", parts.header), true, reverse)
}

/// Stages (or, with `reverse`, unstages) only the changed lines `selected` of one
/// hunk. Indexes count from 0 at the hunk's `@@` line; see
/// [`crate::hunks::select_lines`] for how the rest of the hunk is kept. Applies
/// to the index only, like [`apply_hunk`].
pub fn apply_lines(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    path: &OsStr,
    index: usize,
    selected: &[usize],
    reverse: bool,
) -> Result<(), GitError> {
    let patch = diff(installation, identity, path, reverse)?;
    let parts = crate::hunks::split(&patch).ok_or(GitError::NoHunks)?;
    let hunk = parts.hunks.get(index).ok_or(GitError::HunkOutOfRange)?;
    let chosen = crate::hunks::select_lines(hunk, selected).ok_or(GitError::NoChangedLines)?;
    apply_patch(installation, identity, &format!("{}{chosen}", parts.header), true, reverse)
}

/// Feeds one patch to `git apply`, to the index when `cached` and to the working
/// tree otherwise. `--recount` lets it apply after its neighbours were changed.
fn apply_patch(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    patch: &str,
    cached: bool,
    reverse: bool,
) -> Result<(), GitError> {
    let mut text = patch.to_owned();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let mut arguments = vec![OsString::from("apply"), OsString::from("--recount")];
    if cached {
        arguments.push(OsString::from("--cached"));
    }
    if reverse {
        arguments.push(OsString::from("-R"));
    }
    run_mutation(installation, identity, &arguments, Some(&text))?;
    Ok(())
}

/// Stages every changed path, untracked files included (`git add -A`).
pub fn stage_all(installation: &GitInstallation, identity: &RepositoryIdentity) -> Result<(), GitError> {
    run_mutation(installation, identity, &[OsString::from("add"), OsString::from("-A")], None)?;
    Ok(())
}

/// Removes every path from the index, leaving the working tree unchanged.
pub fn unstage_all(installation: &GitInstallation, identity: &RepositoryIdentity) -> Result<(), GitError> {
    run_mutation(installation, identity, &[OsString::from("reset"), OsString::from("-q")], None)?;
    Ok(())
}

/// Deletes a local branch with `git branch -d`, which refuses a branch whose
/// commits are not merged. Git also refuses the checked-out branch.
pub fn delete_branch(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    name: &OsStr,
) -> Result<(), GitError> {
    let name = checked_branch_name(installation, identity, name)?;
    run_mutation(installation, identity, &[OsString::from("branch"), OsString::from("-d"), name], None)?;
    Ok(())
}

/// Throws away the working-tree changes of one path, so the work is lost. A tracked
/// path is restored from the index (`git restore --worktree`); an untracked path is
/// removed with `git clean`, which only removes untracked files. The index keeps its
/// staged content. The caller must confirm first (PRD 08).
pub fn discard_file(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    path: &OsStr,
    untracked: bool,
) -> Result<(), GitError> {
    let arguments = if untracked {
        vec![OsString::from("clean"), OsString::from("-f"), OsString::from("--"), path.to_os_string()]
    } else {
        vec![
            OsString::from("restore"),
            OsString::from("--worktree"),
            OsString::from("--"),
            path.to_os_string(),
        ]
    };
    run_mutation(installation, identity, &arguments, None)?;
    Ok(())
}

/// Throws away one unstaged hunk of a tracked path from the working tree. Like
/// [`apply_hunk`] it counts hunks from 0, but it reverses the hunk in the working
/// tree rather than the index. The caller must confirm first (PRD 08).
pub fn discard_hunk(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    path: &OsStr,
    index: usize,
) -> Result<(), GitError> {
    let patch = diff(installation, identity, path, false)?;
    let parts = crate::hunks::split(&patch).ok_or(GitError::NoHunks)?;
    let hunk = parts.hunks.get(index).ok_or(GitError::HunkOutOfRange)?;
    apply_patch(installation, identity, &format!("{}{hunk}", parts.header), false, true)
}

/// Saves the tracked staged and unstaged changes as a new stash and cleans them
/// from the working tree (`git stash push`). Untracked files stay where they are.
pub fn stash_push(installation: &GitInstallation, identity: &RepositoryIdentity) -> Result<(), GitError> {
    run_mutation(
        installation,
        identity,
        &[OsString::from("stash"), OsString::from("push"), OsString::from("-m"), OsString::from("minicon-srcmgr")],
        None,
    )?;
    Ok(())
}

/// Applies the newest stash and keeps it (`git stash apply`), the preferred form
/// because the source survives a conflict (PRD 09).
pub fn stash_apply(installation: &GitInstallation, identity: &RepositoryIdentity) -> Result<(), GitError> {
    run_mutation(installation, identity, &[OsString::from("stash"), OsString::from("apply")], None)?;
    Ok(())
}

/// Checks out a commit by its full or abbreviated hexadecimal id, leaving HEAD
/// detached (`git switch --detach`). Git refuses it when local edits would be
/// overwritten. The id is checked before it reaches the argument vector.
pub fn checkout_detached(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    revision: &str,
) -> Result<(), GitError> {
    if revision.len() < 4 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GitError::InvalidRevision);
    }
    run_mutation(
        installation,
        identity,
        &[OsString::from("switch"), OsString::from("--detach"), OsString::from(revision)],
        None,
    )?;
    Ok(())
}

/// The pattern part of a `source:line:pattern` rule. The source may contain a
/// colon (a Windows drive letter), so the split is at the first colon that is
/// followed by a line number and another colon.
fn rule_pattern(rule: &str) -> Option<&str> {
    rule.match_indices(':').find_map(|(index, _)| {
        let after = &rule[index + 1..];
        let (number, rest) = after.split_once(':')?;
        (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())).then_some(rest)
    })
}

/// Reports which ignore rule matches `path`, as Git's `check-ignore -v` names
/// it (`source:line:pattern`). `None` means no rule matches. Matching uses the
/// repository's own `.gitignore`, `info/exclude` and core settings.
pub fn ignore_match(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    path: &OsStr,
) -> Result<Option<String>, GitError> {
    match run_mutation(
        installation,
        identity,
        &[OsString::from("check-ignore"), "-v".into(), "--".into(), path.to_os_string()],
        None,
    ) {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            let rule = text.lines().next().unwrap_or_default().split('\t').next().unwrap_or_default();
            // With -v, a negated pattern (`!name`) is reported with exit 0 but means
            // the path is NOT ignored; it names the rule that re-included the path.
            if rule_pattern(rule).is_some_and(|pattern| pattern.starts_with('!')) {
                return Ok(None);
            }
            Ok(Some(rule.to_owned()))
        }
        // check-ignore exits 1 when nothing matches; that is an answer, not a failure.
        Err(GitError::Exit { code: 1, .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod ignore_rule_tests {
    use super::rule_pattern;

    #[test]
    fn pattern_follows_the_line_number() {
        assert_eq!(rule_pattern(".gitignore:1:*.log"), Some("*.log"));
        assert_eq!(rule_pattern(".gitignore:2:!keep.log"), Some("!keep.log"));
    }

    #[test]
    fn a_drive_letter_in_the_source_does_not_confuse_the_split() {
        assert_eq!(rule_pattern("C:/cfg/ignore:4:build/"), Some("build/"));
    }

    #[test]
    fn a_pattern_containing_colons_is_kept_whole() {
        assert_eq!(rule_pattern(".gitignore:3:a:b:c"), Some("a:b:c"));
    }
}

/// One local branch. `name` is the exact ref name, safe to pass back to
/// `switch_branch`; `label` is the same name with controls escaped for display.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchSummary {
    pub name: String,
    pub label: String,
    pub current: bool,
}

/// Lists the local branches in name order. Git marks the checked-out branch.
/// A branch name that is not UTF-8 is reported as malformed output rather than
/// shown lossily, because the name could not be passed back to `switch`.
pub fn local_branches(installation: &GitInstallation, identity: &RepositoryIdentity) -> Result<Vec<BranchSummary>, GitError> {
    let output = run_mutation(
        installation,
        identity,
        &[
            OsString::from("for-each-ref"),
            "--format=%(HEAD) %(refname:lstrip=2)".into(),
            "--sort=refname".into(),
            "refs/heads/".into(),
        ],
        None,
    )?;
    // `%(HEAD)` is always one character ('*' or a space), followed by a space
    // and the name. A ref name cannot contain a space or a control character, so
    // one line per branch is unambiguous.
    let text = String::from_utf8(output.stdout).map_err(|_| GitError::MalformedOutput { record: 0 })?;
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let marker = line.get(..1).ok_or(GitError::MalformedOutput { record: 0 })?;
            let name = line.get(2..).ok_or(GitError::MalformedOutput { record: 0 })?;
            Ok(BranchSummary {
                name: name.to_owned(),
                label: srcmgr_domain::NativePath::new(name).display_label(),
                current: marker == "*",
            })
        })
        .collect()
}

/// The message and changed-file summary of one commit, as text for display.
/// The id must come from [`log`]; it is passed as a single argument and the
/// `--` ends the revision list, so it cannot be read as an option or a path.
pub fn commit_detail(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    id: &str,
) -> Result<String, GitError> {
    let output = run_mutation(
        installation,
        identity,
        &[
            OsString::from("show"),
            "--no-color".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            "--stat".into(),
            "--format=commit %H%nAuthor: %an%nDate:   %ad%n%n%B".into(),
            id.into(),
            "--".into(),
        ],
        None,
    )?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// One commit in the history list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitSummary {
    pub id: RevisionToken,
    pub parents: Vec<RevisionToken>,
    /// The subject with control and bidirectional characters escaped for display.
    pub subject: String,
}

/// Lists up to `limit` commits reachable from HEAD, newest first, after the
/// first `skip` of them. An unborn branch has no history and yields an empty
/// list. Records are split on the ASCII record separator so a subject cannot
/// forge a record boundary.
pub fn log(
    installation: &GitInstallation,
    identity: &RepositoryIdentity,
    skip: usize,
    limit: usize,
) -> Result<Vec<CommitSummary>, GitError> {
    let head = run_mutation(
        installation,
        identity,
        &[OsString::from("rev-parse"), "--verify".into(), "-q".into(), "HEAD".into()],
        None,
    );
    if matches!(head, Err(GitError::Exit { .. })) {
        return Ok(Vec::new());
    }
    head?;
    let output = run_mutation(
        installation,
        identity,
        &[
            OsString::from("log"),
            format!("--skip={skip}").into(),
            format!("--max-count={limit}").into(),
            "--format=%H%x1f%P%x1f%s%x1e".into(),
        ],
        None,
    )?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split('\x1e')
        .map(str::trim)
        .filter(|record| !record.is_empty())
        .map(|record| {
            let mut fields = record.split('\x1f');
            let (Some(id), Some(parents), Some(subject)) = (fields.next(), fields.next(), fields.next()) else {
                return Err(GitError::MalformedOutput { record: 0 });
            };
            Ok(CommitSummary {
                id: crate::status::revision(id.as_bytes())?,
                parents: parents
                    .split_whitespace()
                    .map(|parent| crate::status::revision(parent.as_bytes()))
                    .collect::<Result<_, _>>()?,
                subject: srcmgr_domain::NativePath::new(subject).display_label(),
            })
        })
        .collect()
}

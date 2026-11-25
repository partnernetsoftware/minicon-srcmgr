//! Unmerged index entries with their stages and side content (PRD 08).
//!
//! Git records a conflict as up to three index entries per path: stage 1 is the
//! base, stage 2 the current side, stage 3 the incoming side. This module reads
//! exactly those entries and never infers a conflict from a filename or from
//! conflict markers in the worktree, because a marker-like line is text the user
//! may have written deliberately.

use crate::error::GitError;
use crate::installation::GitInstallation;
use crate::repository::RepositoryIdentity;
use crate::runner::{self, Limits};

/// What kind of conflict the index records for one path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictClass {
    /// Base, current and incoming all exist: both sides edited one file.
    Content,
    /// No base entry: both sides added the same path.
    AddAdd,
    /// One side deleted the path while the other kept or changed it.
    DeleteModify,
    /// The path is a symbolic link.
    Symlink,
    /// The path is a submodule entry.
    Submodule,
    /// The stages do not match a class this slice can name.
    Unclassified,
}

/// One index entry: the mode and object id Git recorded at one stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictSide {
    pub mode: u32,
    pub oid: String,
}

/// One unmerged path with whatever stages the index holds for it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictEntry {
    pub path: String,
    pub base: Option<ConflictSide>,
    pub current: Option<ConflictSide>,
    pub incoming: Option<ConflictSide>,
    pub class: ConflictClass,
}

/// Reads the unmerged entries of the index.
///
/// \u0060ls-files -u\u0060 lists exactly the conflicted stages; a path with no entry is
/// not conflicted, and a conflict is never guessed from the worktree.
pub fn read_conflicts(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
) -> Result<Vec<ConflictEntry>, GitError> {
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["ls-files", "-u", "-z"],
        None,
        Limits::STATUS,
    )?;
    let mut entries: Vec<ConflictEntry> = Vec::new();
    let mut index: Vec<(String, u8, ConflictSide)> = Vec::new();

    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        // "<mode> <oid> <stage>\t<path>"
        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            return Err(GitError::MalformedOutput { record: index.len() });
        };
        let header = String::from_utf8_lossy(&record[..tab]);
        let mut fields = header.split(' ');
        let (Some(mode), Some(oid), Some(stage)) = (fields.next(), fields.next(), fields.next()) else {
            return Err(GitError::MalformedOutput { record: index.len() });
        };
        let mode = u32::from_str_radix(mode, 8).map_err(|_| GitError::MalformedOutput { record: index.len() })?;
        let stage = stage.parse::<u8>().map_err(|_| GitError::MalformedOutput { record: index.len() })?;
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| GitError::UnrepresentableName)?
            .to_owned();
        index.push((path, stage, ConflictSide { mode, oid: oid.to_owned() }));
    }

    for (path, stage, side) in index {
        let is_new = entries.last().is_none_or(|entry| entry.path != path);
        if is_new {
            entries.push(ConflictEntry {
                path,
                base: None,
                current: None,
                incoming: None,
                class: ConflictClass::Unclassified,
            });
        }
        let entry = entries.last_mut().expect("pushed above");
        match stage {
            1 => entry.base = Some(side),
            2 => entry.current = Some(side),
            3 => entry.incoming = Some(side),
            _ => return Err(GitError::MalformedOutput { record: 0 }),
        }
    }

    for entry in &mut entries {
        entry.class = classify(entry);
    }
    Ok(entries)
}

fn classify(entry: &ConflictEntry) -> ConflictClass {
    let modes = [
        entry.base.as_ref().map(|side| side.mode),
        entry.current.as_ref().map(|side| side.mode),
        entry.incoming.as_ref().map(|side| side.mode),
    ];
    if modes.iter().flatten().any(|mode| mode & 0o170000 == 0o160000) {
        return ConflictClass::Submodule;
    }
    if modes.iter().flatten().any(|mode| mode & 0o170000 == 0o120000) {
        return ConflictClass::Symlink;
    }
    match (entry.base.is_some(), entry.current.is_some(), entry.incoming.is_some()) {
        (true, true, true) => ConflictClass::Content,
        (false, true, true) => ConflictClass::AddAdd,
        (true, false, _) | (true, _, false) => ConflictClass::DeleteModify,
        _ => ConflictClass::Unclassified,
    }
}

/// One side's text, when it is text this slice can show.
///
/// The bytes are read from the object database, bounded, and treated as binary
/// when a NUL appears early; a binary or oversized side returns \u0060None\u0060 rather
/// than a truncated string that would look like the whole file.
pub fn side_text(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    oid: &str,
) -> Result<Option<String>, GitError> {
    if oid.len() != 40 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GitError::InvalidRevision);
    }
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["cat-file", "blob", oid],
        None,
        Limits::STATUS,
    )?;
    let bytes = output.stdout;
    if bytes.iter().take(8000).any(|byte| *byte == 0) {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Which recorded side a resolution takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionSide {
    /// Stage 2: the current side.
    Current,
    /// Stage 3: the incoming side.
    Incoming,
}

impl ResolutionSide {
    pub const fn stage(self) -> u8 {
        match self {
            Self::Current => 2,
            Self::Incoming => 3,
        }
    }
}

/// What taking one side would write, shown before anything is staged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionPreview {
    pub path: String,
    pub side: ResolutionSide,
    /// The exact object the resolution would put in the index and worktree.
    pub oid: String,
    pub mode: u32,
    pub bytes: usize,
    /// True when the side is binary; the preview then carries no text.
    pub binary: bool,
    /// The side's text, bounded, when it is text.
    pub text: Option<String>,
}

/// The same bound the conflict center uses for one side.
const RESOLUTION_TEXT_LIMIT: usize = 4000;

fn conflicting_entry(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    path: &str,
    side: ResolutionSide,
) -> Result<ConflictEntry, GitError> {
    let entries = read_conflicts(git, repo)?;
    let Some(entry) = entries.into_iter().find(|entry| entry.path == path) else {
        return Err(GitError::ConflictResolution(
            "the path is not unmerged any more; refresh the conflict center",
        ));
    };
    match entry.class {
        ConflictClass::Content | ConflictClass::AddAdd => {}
        ConflictClass::DeleteModify => {
            return Err(GitError::ConflictResolution(
                "one side deleted this path; taking a side here also decides whether the file exists, which this version does not stage",
            ))
        }
        ConflictClass::Symlink | ConflictClass::Submodule | ConflictClass::Unclassified => {
            return Err(GitError::ConflictResolution(
                "this conflict type is not resolvable from the window; resolve it in a terminal",
            ))
        }
    }
    let present = match side {
        ResolutionSide::Current => entry.current.is_some(),
        ResolutionSide::Incoming => entry.incoming.is_some(),
    };
    if !present {
        return Err(GitError::ConflictResolution(
            "the index has no entry for that side of this path",
        ));
    }
    Ok(entry)
}

/// Describes taking one side of one conflicted path. Nothing is written.
pub fn preview_resolution(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    path: &str,
    side: ResolutionSide,
) -> Result<ResolutionPreview, GitError> {
    let entry = conflicting_entry(git, repo, path, side)?;
    let chosen = match side {
        ResolutionSide::Current => entry.current.clone().expect("checked above"),
        ResolutionSide::Incoming => entry.incoming.clone().expect("checked above"),
    };
    let output = runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["cat-file", "-s", &chosen.oid],
        None,
        Limits::METADATA,
    )?;
    let bytes = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>()
        .map_err(|_| GitError::MalformedOutput { record: 0 })?;
    let text = side_text(git, repo, &chosen.oid)?;
    Ok(ResolutionPreview {
        path: entry.path,
        side,
        oid: chosen.oid,
        mode: chosen.mode,
        bytes,
        binary: text.is_none(),
        text: text.map(|text| text.chars().take(RESOLUTION_TEXT_LIMIT).collect()),
    })
}

/// Writes the reviewed side into the index and the worktree.
///
/// The index entry is replaced with the exact object the review showed, and
/// then the worktree is filled from that entry. The \`--ours\`/\`--theirs\` names are
/// never used: during a rebase their meanings swap, while a stage number does
/// not.
pub fn apply_resolution(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    preview: &ResolutionPreview,
) -> Result<(), GitError> {
    let entry = conflicting_entry(git, repo, &preview.path, preview.side)?;
    let chosen = match preview.side {
        ResolutionSide::Current => entry.current.as_ref().expect("checked by conflicting_entry"),
        ResolutionSide::Incoming => entry.incoming.as_ref().expect("checked by conflicting_entry"),
    };
    if chosen.oid != preview.oid {
        return Err(GitError::StaleOperationPreview);
    }
    crate::journal::begin(git, repo, "resolve")?;
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &[
            "update-index",
            "--cacheinfo",
            &format!("{:o},{},{}", preview.mode, preview.oid, preview.path),
        ],
        None,
        Limits::MUTATION,
    )?;
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["checkout-index", "-f", "--", &preview.path],
        None,
        Limits::MUTATION,
    )?;
    crate::journal::finish(git, repo)?;
    let remaining = read_conflicts(git, repo)?;
    if remaining.iter().any(|entry| entry.path == preview.path) {
        return Err(GitError::ConflictResolution(
            "the index still reports this path as unmerged after the resolution",
        ));
    }
    Ok(())
}

/// What staging the worktree copy of a conflicted path found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedResolution {
    /// True when the staged file still holds conflict-marker-like lines.
    pub markers: bool,
}

/// Stages whatever the worktree now holds for an unmerged path.
///
/// This is the editor path: the user resolved the file in a trusted editor and
/// the window stages the result. Marker-like lines are reported, not refused,
/// because the user may have written them deliberately; Git's index decides
/// whether the path is still unmerged afterwards, and that decision is checked.
pub fn stage_resolution(
    git: &GitInstallation,
    repo: &RepositoryIdentity,
    path: &str,
) -> Result<StagedResolution, GitError> {
    let entries = read_conflicts(git, repo)?;
    if !entries.iter().any(|entry| entry.path == path) {
        return Err(GitError::ConflictResolution(
            "the path is not unmerged any more; refresh the conflict center",
        ));
    }
    crate::journal::begin(git, repo, "stage-resolution")?;
    runner::run(
        git.executable(),
        Some(&repo.worktree_root),
        &["add", "--", path],
        None,
        Limits::MUTATION,
    )?;
    crate::journal::finish(git, repo)?;
    let remaining = read_conflicts(git, repo)?;
    if remaining.iter().any(|entry| entry.path == path) {
        return Err(GitError::ConflictResolution(
            "the index still reports this path as unmerged after staging it",
        ));
    }
    let markers = conflict_markers(&repo.worktree_root.join(path));
    Ok(StagedResolution { markers })
}

/// Reads the staged file for marker-like lines. Absence of the file is a
/// deletion, not an error.
fn conflict_markers(path: &std::path::Path) -> bool {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let text = String::from_utf8_lossy(&bytes);
    text.lines().any(|line| {
        line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line == "======="
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::locate;
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
        let root = base.join(format!("srcmgr-conflicts-{}-{name}", std::process::id()));
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

    fn start(name: &str) -> (PathBuf, GitInstallation, RepositoryIdentity) {
        let root = fixture(name);
        run(&root, &["init", "-q", "-b", "main"]);
        run(&root, &["config", "user.name", "Fixture"]);
        run(&root, &["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        commit(&root, "one");
        let installation = GitInstallation::probe(git()).unwrap();
        let identity = locate(&installation, &root).unwrap();
        (root, installation, identity)
    }

    /// Leaves the repository stopped in a merge conflict.
    fn stopped(root: &Path, ours: &[u8], theirs: &[u8]) {
        run(root, &["switch", "-q", "-c", "feature"]);
        std::fs::write(root.join("a.txt"), theirs).unwrap();
        commit(root, "feature edit");
        run(root, &["switch", "-q", "main"]);
        std::fs::write(root.join("a.txt"), ours).unwrap();
        commit(root, "main edit");
        let output = Command::new(git()).current_dir(root).args(["merge", "feature"]).output().unwrap();
        assert!(!output.status.success(), "the fixture must conflict");
    }

    #[test]
    fn a_clean_index_has_no_conflicts() {
        let (root, installation, identity) = start("clean");
        assert!(read_conflicts(&installation, &identity).unwrap().is_empty());
        assert!(!root.join(".git/MERGE_HEAD").exists());
    }

    #[test]
    fn a_content_conflict_carries_three_stages_and_readable_sides() {
        let (root, installation, identity) = start("content");
        stopped(&root, b"one\nmain\n", b"one\nfeature\n");
        let entries = read_conflicts(&installation, &identity).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.path, "a.txt");
        assert_eq!(entry.class, ConflictClass::Content);
        assert!(entry.base.is_some() && entry.current.is_some() && entry.incoming.is_some());
        let current = side_text(&installation, &identity, &entry.current.as_ref().unwrap().oid).unwrap();
        assert_eq!(current.as_deref(), Some("one\nmain\n"));
        let incoming = side_text(&installation, &identity, &entry.incoming.as_ref().unwrap().oid).unwrap();
        assert_eq!(incoming.as_deref(), Some("one\nfeature\n"));
        run(&root, &["merge", "--abort"]);
    }

    #[test]
    fn both_sides_adding_one_path_is_add_add_without_a_base() {
        let (root, installation, identity) = start("addadd");
        run(&root, &["switch", "-q", "-c", "feature"]);
        std::fs::write(root.join("b.txt"), "feature\n").unwrap();
        commit(&root, "feature adds b");
        run(&root, &["switch", "-q", "main"]);
        std::fs::write(root.join("b.txt"), "main\n").unwrap();
        commit(&root, "main adds b");
        let output = Command::new(git()).current_dir(&root).args(["merge", "feature"]).output().unwrap();
        assert!(!output.status.success());
        let entries = read_conflicts(&installation, &identity).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "b.txt");
        assert_eq!(entries[0].class, ConflictClass::AddAdd);
        assert!(entries[0].base.is_none());
        run(&root, &["merge", "--abort"]);
    }

    #[test]
    fn one_side_deleting_a_path_is_delete_modify() {
        let (root, installation, identity) = start("deletemodify");
        run(&root, &["switch", "-q", "-c", "feature"]);
        run(&root, &["rm", "-q", "a.txt"]);
        commit(&root, "feature deletes a");
        run(&root, &["switch", "-q", "main"]);
        std::fs::write(root.join("a.txt"), "one\nmain\n").unwrap();
        commit(&root, "main edits a");
        let output = Command::new(git()).current_dir(&root).args(["merge", "feature"]).output().unwrap();
        assert!(!output.status.success());
        let entries = read_conflicts(&installation, &identity).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].class, ConflictClass::DeleteModify);
        assert!(entries[0].current.is_none() || entries[0].incoming.is_none());
        run(&root, &["merge", "--abort"]);
    }

    #[test]
    fn a_binary_side_is_reported_as_unreadable_text() {
        let (root, installation, identity) = start("binary");
        stopped(&root, b"one\n\x00main\n", b"one\n\x00feature\n");
        let entries = read_conflicts(&installation, &identity).unwrap();
        let oid = entries[0].current.as_ref().unwrap().oid.clone();
        assert_eq!(side_text(&installation, &identity, &oid).unwrap(), None);
        run(&root, &["merge", "--abort"]);
    }

    #[test]
    fn a_side_text_call_refuses_a_non_object_id() {
        let (_root, installation, identity) = start("badoid");
        for oid in ["", "HEAD", "zzzz"] {
            assert!(matches!(
                side_text(&installation, &identity, oid),
                Err(GitError::InvalidRevision)
            ));
        }
    }

    #[test]
    fn taking_the_current_side_writes_that_blob_and_clears_the_path() {
        let (root, installation, identity) = start("resolve-current");
        stopped(&root, b"one\nmain\n", b"one\nfeature\n");
        let preview = preview_resolution(&installation, &identity, "a.txt", ResolutionSide::Current).unwrap();
        assert_eq!(preview.side, ResolutionSide::Current);
        assert!(!preview.binary);
        assert_eq!(preview.text.as_deref(), Some("one\nmain\n"));
        assert!(std::fs::read_to_string(root.join("a.txt")).unwrap().contains("<<<<<<<"), "the conflict markers are still there before the resolution");
        apply_resolution(&installation, &identity, &preview).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\nmain\n");
        assert!(read_conflicts(&installation, &identity).unwrap().is_empty());
    }

    #[test]
    fn taking_the_incoming_side_writes_the_other_blob() {
        let (root, installation, identity) = start("resolve-incoming");
        stopped(&root, b"one\nmain\n", b"one\nfeature\n");
        let preview = preview_resolution(&installation, &identity, "a.txt", ResolutionSide::Incoming).unwrap();
        assert_eq!(preview.text.as_deref(), Some("one\nfeature\n"));
        apply_resolution(&installation, &identity, &preview).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\nfeature\n");
        assert!(read_conflicts(&installation, &identity).unwrap().is_empty());
    }

    #[test]
    fn a_delete_modify_conflict_is_refused_by_name() {
        let (root, installation, identity) = start("resolve-delete");
        run(&root, &["switch", "-q", "-c", "feature"]);
        run(&root, &["rm", "-q", "a.txt"]);
        commit(&root, "feature deletes a");
        run(&root, &["switch", "-q", "main"]);
        std::fs::write(root.join("a.txt"), "one\nmain\n").unwrap();
        commit(&root, "main edits a");
        let output = Command::new(git()).current_dir(&root).args(["merge", "feature"]).output().unwrap();
        assert!(!output.status.success());
        let error = preview_resolution(&installation, &identity, "a.txt", ResolutionSide::Current).expect_err("refused");
        assert!(matches!(error, GitError::ConflictResolution(_)), "{error}");
        run(&root, &["merge", "--abort"]);
    }

    #[test]
    fn an_unmerged_path_that_no_longer_exists_is_refused_after_a_refresh() {
        let (root, installation, identity) = start("resolve-gone");
        let error = preview_resolution(&installation, &identity, "a.txt", ResolutionSide::Current).expect_err("clean index");
        assert!(matches!(error, GitError::ConflictResolution(_)), "{error}");
        assert!(root.join("a.txt").exists());
    }

    #[test]
    fn staging_the_edited_copy_clears_the_path_and_reports_kept_markers() {
        let (root, installation, identity) = start("stage-markers");
        stopped(&root, b"one\nmain\n", b"one\nfeature\n");
        let staged = stage_resolution(&installation, &identity, "a.txt").unwrap();
        assert!(staged.markers, "the untouched file still holds markers");
        assert!(read_conflicts(&installation, &identity).unwrap().is_empty());
    }

    #[test]
    fn staging_a_cleanly_edited_copy_reports_no_markers() {
        let (root, installation, identity) = start("stage-clean");
        stopped(&root, b"one\nmain\n", b"one\nfeature\n");
        std::fs::write(root.join("a.txt"), "one\nmain and feature\n").unwrap();
        let staged = stage_resolution(&installation, &identity, "a.txt").unwrap();
        assert!(!staged.markers);
        assert!(read_conflicts(&installation, &identity).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\nmain and feature\n");
    }

    #[test]
    fn staging_a_path_that_is_not_unmerged_is_refused() {
        let (_root, installation, identity) = start("stage-clean-index");
        let error = stage_resolution(&installation, &identity, "a.txt").expect_err("refused");
        assert!(matches!(error, GitError::ConflictResolution(_)), "{error}");
    }
}

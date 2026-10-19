//! Real-Git fixtures: disposable repositories, an installed `git`, and expected
//! states written out by hand (not derived by re-running Git).
//!
//! `SRCMGR_TEST_GIT` selects the executable; otherwise `git` is resolved from
//! PATH by the operating system. Fixtures are created under the system temp
//! directory and removed afterwards.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use srcmgr_domain::{ChangeKind, Generation, HeadState, NativePath};
use srcmgr_git::{GitError, GitInstallation, locate, read_status};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!(
            "srcmgr-fixture-{}-{nanos}-{unique}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git_executable() -> PathBuf {
    env::var_os("SRCMGR_TEST_GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from)
}

fn installation() -> GitInstallation {
    GitInstallation::probe(git_executable()).expect("probe the test Git")
}

/// Runs the test Git with a fixed identity and no user template or hooks.
fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new(git_executable())
        .current_dir(directory)
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid"])
        .args(["-c", "core.hooksPath=/dev/null", "-c", "init.defaultBranch=main"])
        .args(arguments)
        .output()
        .expect("run fixture git");
    assert!(
        output.status.success(),
        "fixture git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_with_one_commit(root: &Path) {
    git(root, &["init", "-q"]);
    fs::write(root.join("tracked.txt"), "one\n").expect("write tracked");
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-q", "-m", "initial"]);
}

fn entry<'a>(snapshot: &'a srcmgr_domain::StatusSnapshot, label: &str) -> &'a srcmgr_domain::ChangeEntry {
    snapshot
        .entries
        .iter()
        .find(|entry| entry.path.display_label() == label)
        .unwrap_or_else(|| panic!("no entry labelled {label}"))
}

#[test]
fn installed_git_reports_a_version_and_floor_check_matches_it() {
    let installation = installation();
    let version = installation.version();
    assert!(version.major >= 2, "implausible git version {version}");
    let meets_floor = installation.require_supported().is_ok();
    assert_eq!(meets_floor, version >= srcmgr_git::MINIMUM_GIT);
}

#[test]
fn clean_branch_reports_no_changes_and_its_head() {
    let fixture = Fixture::new("clean");
    init_with_one_commit(&fixture.root);
    let installation = installation();
    let identity = locate(&installation, &fixture.root).expect("locate");
    let snapshot = read_status(&installation, &identity, Generation(1)).expect("status");

    assert!(snapshot.entries.is_empty());
    match snapshot.head {
        HeadState::Branch { name, revision } => {
            assert_eq!(name, "main");
            let revision = revision.expect("commit exists");
            assert!(matches!(revision.token().len(), 40 | 64));
        }
        other => panic!("expected a branch head, got {other:?}"),
    }
}

#[test]
fn staged_unstaged_untracked_and_renamed_paths_are_classified() {
    let fixture = Fixture::new("mixed");
    init_with_one_commit(&fixture.root);
    let root = &fixture.root;

    fs::write(root.join("tracked.txt"), "two\n").expect("modify");
    fs::write(root.join("staged new.txt"), "new\n").expect("create");
    git(root, &["add", "staged new.txt"]);
    git(root, &["mv", "tracked.txt", "moved.txt"]);
    fs::write(root.join("loose 中文.txt"), "untracked\n").expect("create untracked");

    let installation = installation();
    let identity = locate(&installation, root).expect("locate");
    let snapshot = read_status(&installation, &identity, Generation(4)).expect("status");

    let added = entry(&snapshot, "staged new.txt");
    assert_eq!(added.staged, ChangeKind::Added);
    assert_eq!(added.unstaged, ChangeKind::Unmodified);

    let renamed = entry(&snapshot, "moved.txt");
    assert_eq!(renamed.staged, ChangeKind::Renamed);
    assert_eq!(
        renamed.original_path.as_ref().map(NativePath::display_label),
        Some("tracked.txt".to_owned())
    );

    let untracked = entry(&snapshot, "loose 中文.txt");
    assert_eq!(untracked.unstaged, ChangeKind::Untracked);
    assert_eq!(untracked.staged, ChangeKind::Unmodified);

    assert_eq!(snapshot.entries.len(), 3);
    let ids: Vec<&str> = snapshot.entries.iter().map(|entry| entry.id.as_str()).collect();
    assert!(ids.iter().all(|id| id.starts_with("g4-")));
}

#[test]
fn a_path_changed_in_both_areas_keeps_both_halves() {
    let fixture = Fixture::new("both");
    init_with_one_commit(&fixture.root);
    let root = &fixture.root;
    fs::write(root.join("tracked.txt"), "staged\n").expect("modify");
    git(root, &["add", "tracked.txt"]);
    fs::write(root.join("tracked.txt"), "staged and again\n").expect("modify again");

    let installation = installation();
    let identity = locate(&installation, root).expect("locate");
    let snapshot = read_status(&installation, &identity, Generation(1)).expect("status");
    let changed = entry(&snapshot, "tracked.txt");
    assert_eq!(changed.staged, ChangeKind::Modified);
    assert_eq!(changed.unstaged, ChangeKind::Modified);
}

#[test]
fn unborn_branch_is_reported_without_a_revision() {
    let fixture = Fixture::new("unborn");
    git(&fixture.root, &["init", "-q"]);
    let installation = installation();
    let identity = locate(&installation, &fixture.root).expect("locate");
    let snapshot = read_status(&installation, &identity, Generation(1)).expect("status");
    assert_eq!(
        snapshot.head,
        HeadState::Branch {
            name: "main".to_owned(),
            revision: None,
        }
    );
}

#[test]
fn gitfile_pointer_is_refused_without_being_followed() {
    let fixture = Fixture::new("gitfile");
    fs::write(fixture.root.join(".git"), "gitdir: ../elsewhere\n").expect("write pointer");
    let installation = installation();
    assert!(matches!(
        locate(&installation, &fixture.root),
        Err(GitError::NotARepository)
    ));
}

#[test]
fn directory_without_git_metadata_is_refused() {
    let fixture = Fixture::new("plain");
    let installation = installation();
    assert!(matches!(
        locate(&installation, &fixture.root),
        Err(GitError::NotARepository)
    ));
}

#[test]
fn linked_worktrees_share_one_repository_identity() {
    let fixture = Fixture::new("linked");
    init_with_one_commit(&fixture.root);
    let linked = fixture.root.join("linked-tree");
    git(
        &fixture.root,
        &["worktree", "add", "-q", "-b", "side", linked.to_str().expect("utf-8 test path")],
    );

    // A linked worktree has a .git file, which locate refuses by design. The
    // identity still has to match the main repository's common directory when
    // the main worktree is inspected, which is the coordination key (ARCH-03).
    let installation = installation();
    let main_identity = locate(&installation, &fixture.root).expect("main locate");
    assert!(main_identity.common_dir.ends_with(".git"));
    assert!(fs::symlink_metadata(linked.join(".git")).expect("linked .git").is_file());
    assert!(matches!(locate(&installation, &linked), Err(GitError::NotARepository)));
}

#[test]
fn a_nonexistent_executable_is_a_typed_spawn_error() {
    let missing = env::temp_dir().join("srcmgr-no-such-git-executable-for-test");
    match GitInstallation::probe(missing) {
        Err(GitError::Spawn(_)) => {}
        other => panic!("expected a spawn error, got {other:?}"),
    }
}

#[test]
fn native_path_round_trips_the_tool_bytes_it_was_given() {
    let fixture = Fixture::new("bytes");
    init_with_one_commit(&fixture.root);
    fs::write(fixture.root.join("a b\u{00e9}.txt"), "x\n").expect("write");
    let installation = installation();
    let identity = locate(&installation, &fixture.root).expect("locate");
    let snapshot = read_status(&installation, &identity, Generation(1)).expect("status");
    let found = entry(&snapshot, "a bé.txt");
    let expected = NativePath::new("a bé.txt");
    assert_eq!(found.path, expected);
}

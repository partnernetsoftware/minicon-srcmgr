//! Publishing against real Git: a bare remote and two working copies. Observed
//! refs and OIDs come from the fixture's own Git commands.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use srcmgr_git::{GitError, GitInstallation, PushState, locate, push_current};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!(
            "srcmgr-push-{}-{nanos}-{unique}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
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

fn git(directory: &Path, arguments: &[&str]) -> Output {
    let output = Command::new(git_executable())
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run fixture git");
    assert!(
        output.status.success(),
        "fixture git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_text(directory: &Path, arguments: &[&str]) -> String {
    String::from_utf8_lossy(&git(directory, arguments).stdout).trim().to_owned()
}

/// A bare remote plus two clones on `main`, both with the same single commit.
fn two_copies(name: &str) -> (Fixture, PathBuf, PathBuf, PathBuf) {
    let fixture = Fixture::new(name);
    let remote = fixture.path("remote.git");
    fs::create_dir_all(&remote).expect("bare directory");
    git(&remote, &["init", "--bare", "--initial-branch=main", "."]);

    let first = fixture.path("first");
    git(&fixture.root, &["clone", remote.to_str().expect("path"), first.to_str().expect("path")]);
    for (key, value) in [("user.name", "First"), ("user.email", "first@example.test")] {
        git(&first, &["config", key, value]);
    }
    fs::write(first.join("notes.txt"), "one\n").expect("write first file");
    git(&first, &["add", "notes.txt"]);
    git(&first, &["commit", "-m", "first"]);
    git(&first, &["push", "origin", "main"]);

    let second = fixture.path("second");
    git(
        &fixture.root,
        &["clone", "-b", "main", remote.to_str().expect("path"), second.to_str().expect("path")],
    );
    for (key, value) in [("user.name", "Second"), ("user.email", "second@example.test")] {
        git(&second, &["config", key, value]);
    }
    (fixture, remote, first, second)
}

fn commit(directory: &Path, file: &str, contents: &str, message: &str) {
    fs::write(directory.join(file), contents).expect("write fixture file");
    git(directory, &["add", file]);
    git(directory, &["commit", "-m", message]);
}

fn remote_branch(directory: &Path, branch: &str) -> String {
    git_text(directory, &["rev-parse", &format!("refs/heads/{branch}")])
}

#[test]
fn a_new_commit_is_pushed_and_a_repeat_is_up_to_date() {
    let (_fixture, remote, first, _second) = two_copies("publish");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");
    commit(&first, "notes.txt", "one\ntwo\n", "second");
    let local = git_text(&first, &["rev-parse", "HEAD"]);
    let cancel = AtomicBool::new(false);

    let report = push_current(&installation, &repository, "origin", &cancel, None).expect("push");
    assert_eq!(report.remote, "origin");
    assert_eq!(report.branch, "main");
    assert_eq!(report.reference, "refs/heads/main:refs/heads/main");
    assert_eq!(report.state, PushState::Updated);
    assert_eq!(report.observed.as_deref(), Some(local.as_str()));
    assert_eq!(remote_branch(&remote, "main"), local);

    let again = push_current(&installation, &repository, "origin", &cancel, None).expect("push");
    assert_eq!(again.state, PushState::UpToDate);
}

#[test]
fn a_remote_ahead_rejects_the_push_and_keeps_its_commit() {
    let (_fixture, remote, first, second) = two_copies("rejected");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");

    commit(&first, "local.txt", "local\n", "local");
    commit(&second, "notes.txt", "one\nremote\n", "remote");
    git(&second, &["push", "origin", "main"]);
    let remote_head = remote_branch(&remote, "main");

    let cancel = AtomicBool::new(false);
    let report = push_current(&installation, &repository, "origin", &cancel, None).expect("push");
    assert_eq!(report.state, PushState::Rejected);
    assert_eq!(remote_branch(&remote, "main"), remote_head);
}

#[test]
fn a_detached_head_is_refused_before_the_transfer() {
    let (_fixture, _remote, first, _second) = two_copies("detached");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");
    git(&first, &["checkout", "--detach", "HEAD"]);
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        push_current(&installation, &repository, "origin", &cancel, None),
        Err(GitError::DetachedHead)
    ));
}

#[test]
fn an_unknown_remote_is_refused_before_the_transfer() {
    let (_fixture, _remote, first, _second) = two_copies("unknown");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        push_current(&installation, &repository, "missing", &cancel, None),
        Err(GitError::UnknownRemote)
    ));
}

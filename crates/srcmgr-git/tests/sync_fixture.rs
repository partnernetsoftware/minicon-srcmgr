//! Upstream and fast-forward pull against real Git: a bare remote and two
//! working copies. Expected commits come from the fixture's own Git commands.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use srcmgr_git::{
    GitError, GitInstallation, clear_upstream, fetch, locate, pull_ff, set_upstream, tracking,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!(
            "srcmgr-sync-{}-{nanos}-{unique}-{name}",
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

#[test]
fn upstream_is_set_and_cleared() {
    let (_fixture, _remote, first, _second) = two_copies("upstream");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");

    git(&first, &["branch", "--unset-upstream"]);
    assert_eq!(tracking(&installation, &repository).expect("tracking").upstream, None);

    set_upstream(&installation, &repository, "origin/main").expect("set upstream");
    assert_eq!(
        tracking(&installation, &repository).expect("tracking").upstream.as_deref(),
        Some("origin/main")
    );

    clear_upstream(&installation, &repository).expect("clear upstream");
    assert_eq!(tracking(&installation, &repository).expect("tracking").upstream, None);

    for invalid in ["origin", "-origin/main", "origin/mai\nn"] {
        assert!(
            matches!(
                set_upstream(&installation, &repository, invalid),
                Err(GitError::InvalidRemote)
            ),
            "{invalid:?} is not an acceptable upstream"
        );
    }
}

#[test]
fn pull_fast_forwards_to_the_fetched_upstream() {
    let (_fixture, _remote, first, second) = two_copies("fast-forward");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");
    let before = git_text(&first, &["rev-parse", "HEAD"]);

    commit(&second, "notes.txt", "one\ntwo\n", "second");
    git(&second, &["push", "origin", "main"]);
    let upstream = git_text(&second, &["rev-parse", "HEAD"]);

    let cancel = AtomicBool::new(false);
    fetch(&installation, &repository, "origin", &cancel, None).expect("fetch");
    let report = pull_ff(&installation, &repository, &cancel, None).expect("pull");

    assert!(report.updated);
    assert_eq!(report.before, before);
    assert_eq!(report.after, upstream);
    assert_eq!(git_text(&first, &["rev-parse", "HEAD"]), upstream);
    assert_eq!(fs::read_to_string(first.join("notes.txt")).expect("read"), "one\ntwo\n");
}

#[test]
fn a_diverged_branch_is_refused_without_moving_head() {
    let (_fixture, _remote, first, second) = two_copies("diverged");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");

    commit(&first, "local.txt", "local\n", "local");
    let before = git_text(&first, &["rev-parse", "HEAD"]);
    commit(&second, "notes.txt", "one\nremote\n", "remote");
    git(&second, &["push", "origin", "main"]);

    let cancel = AtomicBool::new(false);
    fetch(&installation, &repository, "origin", &cancel, None).expect("fetch");
    let error = pull_ff(&installation, &repository, &cancel, None)
        .expect_err("a diverged branch cannot fast-forward");
    assert!(matches!(error, GitError::Exit { .. }), "unexpected {error:?}");
    assert_eq!(git_text(&first, &["rev-parse", "HEAD"]), before);
}

#[test]
fn a_missing_upstream_is_reported_before_any_merge() {
    let (_fixture, _remote, first, _second) = two_copies("no-upstream");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");
    git(&first, &["branch", "--unset-upstream"]);
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        pull_ff(&installation, &repository, &cancel, None),
        Err(GitError::NoUpstream)
    ));
}

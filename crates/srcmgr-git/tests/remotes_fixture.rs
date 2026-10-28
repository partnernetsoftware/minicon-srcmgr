//! Remote observation against real Git: a bare remote and two working copies.
//! Expected refs and OIDs come from the fixture's own Git commands, not from
//! the product's report.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use srcmgr_git::{GitError, GitInstallation, fetch, list_remotes, locate, tracking};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!("srcmgr-remotes-{}-{nanos}-{unique}-{name}", std::process::id()));
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

/// A bare remote plus two clones, each with one commit, and the second copy one
/// commit ahead of the first.
fn two_copies(name: &str) -> (Fixture, PathBuf, PathBuf) {
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
    git(&fixture.root, &["clone", "-b", "main", remote.to_str().expect("path"), second.to_str().expect("path")]);
    for (key, value) in [("user.name", "Second"), ("user.email", "second@example.test")] {
        git(&second, &["config", key, value]);
    }
    fs::write(second.join("notes.txt"), "one\ntwo\n").expect("append second file");
    git(&second, &["commit", "-am", "second"]);
    git(&second, &["push", "origin", "main"]);
    (fixture, first, second)
}

#[test]
fn remotes_carry_redacted_urls_and_refspecs() {
    let fixture = Fixture::new("list");
    let worktree = fixture.path("work");
    fs::create_dir_all(&worktree).expect("worktree");
    git(&worktree, &["init", "--initial-branch=main", "."]);
    git(
        &worktree,
        &["remote", "add", "secure", "https://alice:s3cret@example.test/repo.git"],
    );
    git(
        &worktree,
        &["remote", "set-url", "--push", "secure", "https://oauth2:token@example.test/repo.git"],
    );

    let installation = installation();
    let repository = locate(&installation, &worktree).expect("locate");
    let remotes = list_remotes(&installation, &repository).expect("list");

    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].name, "secure");
    assert_eq!(remotes[0].fetch_url, "https://alice@example.test/repo.git");
    assert_eq!(remotes[0].push_url.as_deref(), Some("https://oauth2@example.test/repo.git"));
    assert_eq!(remotes[0].fetch_refspecs, vec!["+refs/heads/*:refs/remotes/secure/*"]);
    let raw = fs::read_to_string(worktree.join(".git/config")).expect("config");
    assert!(raw.contains("s3cret") && !remotes[0].fetch_url.contains("s3cret"));
}

#[test]
fn tracking_and_fetch_report_the_actual_remote_commit() {
    let (fixture, first, second) = two_copies("fetch");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");
    let remote_head = git_text(&second, &["rev-parse", "HEAD"]);

    let before = tracking(&installation, &repository).expect("tracking before fetch");
    assert_eq!(before.upstream.as_deref(), Some("origin/main"));
    assert_eq!((before.ahead, before.behind), (0, 0));

    let cancel = AtomicBool::new(false);
    let report = fetch(&installation, &repository, "origin", &cancel, None).expect("fetch");
    assert_eq!(report.remote, "origin");
    assert!(report.updates.iter().any(|update| {
        update.reference == "refs/remotes/origin/main" && update.new == remote_head
    }));

    let after = tracking(&installation, &repository).expect("tracking after fetch");
    assert_eq!((after.ahead, after.behind), (0, 1));

    let again = fetch(&installation, &repository, "origin", &cancel, None).expect("second fetch");
    assert!(again.updates.is_empty(), "an unchanged remote reports no update");

    let _ = fixture;
    let _ = second;
}

#[test]
fn a_mirror_remote_is_refused_before_any_transfer() {
    let (fixture, first, _second) = two_copies("mirror");
    git(&first, &["config", "remote.origin.mirror", "true"]);
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");
    let cancel = AtomicBool::new(false);
    let result = fetch(&installation, &repository, "origin", &cancel, None);
    assert!(matches!(result, Err(GitError::RemotePolicy(_))), "got {result:?}");
    let _ = fixture;
}

#[test]
fn an_unknown_remote_is_refused_before_any_transfer() {
    let (fixture, first, _second) = two_copies("unknown");
    let installation = installation();
    let repository = locate(&installation, &first).expect("locate");
    let cancel = AtomicBool::new(false);
    let result = fetch(&installation, &repository, "missing", &cancel, None);
    assert!(matches!(result, Err(GitError::UnknownRemote)), "got {result:?}");
    let _ = fixture;
}

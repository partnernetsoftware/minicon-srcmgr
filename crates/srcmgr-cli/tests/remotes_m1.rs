//! Remote observation through the product CLI. The oracle is Git's own ref
//! state: the fetch must update exactly the ref Git reported, and the credential
//! in the configured URL must not appear in product output.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!("srcmgr-remote-cli-{}-{nanos}-{unique}-{name}", std::process::id()));
        fs::create_dir_all(&root).expect("create fixture");
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

fn git(directory: &Path, arguments: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn cli(profile: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minicon-srcmgr"))
        .args(["--profile", profile.to_str().expect("profile")])
        .args(["--git", git_executable().to_str().expect("git")])
        .args(arguments)
        .output()
        .expect("run product")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn fetch_updates_the_reported_ref_and_credentials_stay_out_of_output() {
    let fixture = Fixture::new("fetch");
    let remote = fixture.root.join("remote.git");
    fs::create_dir_all(&remote).expect("bare");
    git(&remote, &["init", "--bare", "--initial-branch=main", "."]);

    let first = fixture.root.join("first");
    git(&fixture.root, &["clone", remote.to_str().expect("remote"), first.to_str().expect("first")]);
    for (key, value) in [("user.name", "First"), ("user.email", "first@example.test")] {
        git(&first, &["config", key, value]);
    }
    fs::write(first.join("notes.txt"), "one\n").expect("write");
    git(&first, &["add", "notes.txt"]);
    git(&first, &["commit", "-m", "first"]);
    git(&first, &["push", "origin", "main"]);

    let second = fixture.root.join("second");
    git(&fixture.root, &["clone", "-b", "main", remote.to_str().expect("remote"), second.to_str().expect("second")]);
    for (key, value) in [("user.name", "Second"), ("user.email", "second@example.test")] {
        git(&second, &["config", key, value]);
    }
    fs::write(second.join("notes.txt"), "one\ntwo\n").expect("append");
    git(&second, &["commit", "-am", "second"]);
    git(&second, &["push", "origin", "main"]);
    let remote_head = git(&second, &["rev-parse", "HEAD"]);

    // A credential in a configured URL must be invisible in product output.
    // The remote is added before the trust grant because repository config is
    // part of the reviewed fingerprint.
    git(&first, &["remote", "add", "secure", "https://alice:s3cret@example.test/repo.git"]);
    git(&first, &["remote", "set-url", "--push", "secure", "https://oauth2:token@example.test/repo.git"]);

    let profile = fixture.root.join("profile");
    let granted = cli(&profile, &["trust", "grant", first.to_str().expect("first")]);
    assert!(granted.status.success(), "trust grant failed: {}", stderr(&granted));

    let listed = cli(&profile, &["remote", first.to_str().expect("first"), "list", "--json"]);
    assert!(listed.status.success(), "list failed: {}", stderr(&listed));
    let listing = stdout(&listed);
    assert!(listing.contains("\"origin\""), "listing: {listing}");
    assert!(listing.contains("https://alice@example.test/repo.git"), "listing: {listing}");
    assert!(!listing.contains("s3cret") && !listing.contains("token@"), "credential leaked: {listing}");
    assert!(listing.contains("refs/remotes/origin/*"), "refspec missing: {listing}");
    let fetched = cli(&profile, &["fetch", first.to_str().expect("first"), "origin"]);
    assert!(fetched.status.success(), "fetch failed: {}", stderr(&fetched));
    let report = stdout(&fetched);
    assert!(report.contains("refs/remotes/origin/main"), "report: {report}");
    assert!(report.contains(&remote_head[..7]), "report: {report}");

    let actual = git(&first, &["rev-parse", "refs/remotes/origin/main"]);
    assert_eq!(actual, remote_head);

    let tracking = cli(&profile, &["remote", first.to_str().expect("first"), "tracking", "--json"]);
    assert!(tracking.status.success(), "tracking failed: {}", stderr(&tracking));
    let state = stdout(&tracking);
    assert!(state.contains("origin/main"), "tracking: {state}");
    assert!(state.contains("\"behind\": 1"), "tracking: {state}");
    assert!(state.contains("\"ahead\": 0"), "tracking: {state}");
}

#[test]
fn fetch_reports_an_unknown_remote_without_changing_refs() {
    let fixture = Fixture::new("unknown");
    let worktree = fixture.root.join("work");
    fs::create_dir_all(&worktree).expect("work");
    git(&worktree, &["init", "--initial-branch=main", "."]);
    for (key, value) in [("user.name", "Solo"), ("user.email", "solo@example.test")] {
        git(&worktree, &["config", key, value]);
    }
    fs::write(worktree.join("a.txt"), "a\n").expect("write");
    git(&worktree, &["add", "a.txt"]);
    git(&worktree, &["commit", "-m", "a"]);
    let head = git(&worktree, &["rev-parse", "HEAD"]);

    let profile = fixture.root.join("profile");
    let granted = cli(&profile, &["trust", "grant", worktree.to_str().expect("worktree")]);
    assert!(granted.status.success(), "trust grant failed: {}", stderr(&granted));

    let fetched = cli(&profile, &["fetch", worktree.to_str().expect("worktree"), "missing"]);
    assert!(!fetched.status.success());
    assert!(stderr(&fetched).contains("not configured"), "stderr: {}", stderr(&fetched));
    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), head);
}

//! Pull and push through the CLI, checked against the remote's own refs.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture { root: PathBuf }

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!("srcmgr-sync-{}-{nanos}-{unique}-{name}", std::process::id()));
        fs::create_dir_all(&root).expect("fixture");
        Self { root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
}

fn git_executable() -> PathBuf {
    env::var_os("SRCMGR_TEST_GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from)
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new(git_executable()).current_dir(directory).args(arguments).output().expect("git");
    assert!(output.status.success(), "git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn cli(profile: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minicon-srcmgr"))
        .args(["--profile", profile.to_str().expect("profile")])
        .args(["--git", git_executable().to_str().expect("git")])
        .args(arguments)
        .output()
        .expect("product")
}

fn text(output: &Output) -> String { String::from_utf8_lossy(&output.stdout).into_owned() }
fn err(output: &Output) -> String { String::from_utf8_lossy(&output.stderr).into_owned() }

#[test]
fn push_then_pull_moves_exactly_the_pushed_commit() {
    let fixture = Fixture::new("exchange");
    let bare = fixture.root.join("remote.git");
    fs::create_dir_all(&bare).expect("bare");
    git(&bare, &["init", "--bare", "--initial-branch=main", "."]);
    let source = fixture.root.join("source");
    git(&fixture.root, &["init", "--initial-branch=main", "source"]);
    for (key, value) in [("user.name", "Source"), ("user.email", "source@example.test")] {
        git(&source, &["config", key, value]);
    }
    fs::write(source.join("a.txt"), "one\n").expect("write");
    git(&source, &["add", "a.txt"]);
    git(&source, &["commit", "-m", "one"]);
    git(&source, &["push", bare.to_str().expect("bare"), "main"]);

    let work = fixture.root.join("work");
    let second = fixture.root.join("second");
    let profile = fixture.root.join("profile");
    for (destination, name) in [(&work, "work"), (&second, "second")] {
        let cloned = cli(&profile, &["clone", bare.to_str().expect("bare"), destination.to_str().expect(name)]);
        assert!(cloned.status.success(), "clone: {}", err(&cloned));
        // Check out so the product has a normal worktree; clone itself defers it.
        git(destination, &["checkout", "main"]);
        for (key, value) in [("user.name", name), ("user.email", "user@example.test")] {
            git(destination, &["config", key, value]);
        }
        let granted = cli(&profile, &["trust", "grant", destination.to_str().expect("path")]);
        assert!(granted.status.success(), "trust: {}", err(&granted));
    }

    fs::write(work.join("a.txt"), "one\ntwo\n").expect("edit");
    git(&work, &["commit", "-am", "two"]);
    let local = git(&work, &["rev-parse", "HEAD"]);
    let pushed = cli(&profile, &["push", work.to_str().expect("work"), "origin"]);
    assert!(pushed.status.success(), "push: {}", err(&pushed));
    assert!(text(&pushed).contains("updated"), "{}", text(&pushed));
    assert_eq!(git(&bare, &["rev-parse", "refs/heads/main"]), local);

    let pulled = cli(&profile, &["pull", second.to_str().expect("second")]);
    assert!(pulled.status.success(), "pull: {}", err(&pulled));
    assert!(text(&pulled).contains("fast-forwarded"), "{}", text(&pulled));
    assert_eq!(git(&second, &["rev-parse", "HEAD"]), local);
    assert_eq!(fs::read_to_string(second.join("a.txt")).expect("content"), "one\ntwo\n");

    let again = cli(&profile, &["pull", second.to_str().expect("second")]);
    assert!(again.status.success(), "{}", err(&again));
    assert!(text(&again).contains("already up to date"), "{}", text(&again));
}

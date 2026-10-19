//! App preferences through the CLI. A saved `history.limit` must survive
//! separate process runs and apply when `history` is given no `--limit`; an
//! invalid or unknown setting must be refused without changing the stored value.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!("srcmgr-settings-{}-{nanos}-{unique}-{name}", std::process::id()));
        fs::create_dir_all(&root).expect("create scratch");
        Self { root }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git_executable() -> PathBuf {
    env::var_os("SRCMGR_TEST_GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from)
}

fn oracle(directory: &Path, arguments: &[&str]) {
    let output = Command::new(git_executable())
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run oracle git");
    assert!(output.status.success(), "oracle git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn cli(profile: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minicon-srcmgr"))
        .args(["--profile", profile.to_str().expect("utf-8 profile")])
        .args(["--git", git_executable().to_str().expect("utf-8 git")])
        .args(arguments)
        .output()
        .expect("run minicon-srcmgr")
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn p(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

/// A repository with five commits and a trust grant, plus a profile directory.
fn repository(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    oracle(&repo, &["init", "-q", "-b", "main"]);
    oracle(&repo, &["config", "user.name", "Fixture"]);
    oracle(&repo, &["config", "user.email", "fixture@example.invalid"]);
    for index in 1..=5 {
        fs::write(repo.join("file.txt"), format!("{index}\n")).expect("write");
        oracle(&repo, &["add", "file.txt"]);
        oracle(&repo, &["commit", "-q", "-m", &format!("commit {index}")]);
    }
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    (repo, profile)
}

#[test]
fn a_saved_history_limit_applies_in_a_later_process() {
    let scratch = Scratch::new("saved");
    let (repo, profile) = repository(&scratch);
    assert!(cli(&profile, &["setting", "set", "history.limit", "2"]).status.success());
    assert_eq!(text(&cli(&profile, &["setting", "get", "history.limit"])).trim(), "2");

    let shown = text(&cli(&profile, &["history", p(&repo)]));
    assert_eq!(shown.lines().count(), 2, "{shown}");
}

#[test]
fn an_explicit_limit_overrides_the_saved_value() {
    let scratch = Scratch::new("override");
    let (repo, profile) = repository(&scratch);
    assert!(cli(&profile, &["setting", "set", "history.limit", "2"]).status.success());

    let shown = text(&cli(&profile, &["history", p(&repo), "--limit", "4"]));
    assert_eq!(shown.lines().count(), 4, "{shown}");
}

#[test]
fn without_a_saved_value_the_default_is_used() {
    let scratch = Scratch::new("default");
    let (repo, profile) = repository(&scratch);
    assert_eq!(text(&cli(&profile, &["setting", "get", "history.limit"])).trim(), "unset");

    let shown = text(&cli(&profile, &["history", p(&repo)]));
    assert_eq!(shown.lines().count(), 5, "{shown}");
}

#[test]
fn an_invalid_value_is_refused_and_the_stored_value_is_kept() {
    let scratch = Scratch::new("invalid");
    let (_repo, profile) = repository(&scratch);
    assert!(cli(&profile, &["setting", "set", "history.limit", "3"]).status.success());

    for bad in ["0", "201", "-1", "many", ""] {
        let refused = cli(&profile, &["setting", "set", "history.limit", bad]);
        assert!(!refused.status.success(), "accepted {bad:?}");
    }
    assert_eq!(text(&cli(&profile, &["setting", "get", "history.limit"])).trim(), "3");
}

#[test]
fn an_unknown_key_is_refused() {
    let scratch = Scratch::new("unknown");
    let (_repo, profile) = repository(&scratch);
    let refused = cli(&profile, &["setting", "set", "history.limt", "3"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("unknown setting"));
}

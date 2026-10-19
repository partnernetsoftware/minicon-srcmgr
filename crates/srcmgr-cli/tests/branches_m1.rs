//! Branch listing through the CLI, checked against `git for-each-ref`. The list
//! must match Git's names and order, mark only the checked-out branch, pass
//! every listed name back to `switch` unchanged, and escape bidirectional
//! controls in the display label.

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
        let root = env::temp_dir().join(format!("srcmgr-branches-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn oracle(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new(git_executable())
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run oracle git");
    assert!(output.status.success(), "oracle git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
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

/// main (checked out), `feature/a` and `ünïcode`, with one commit and a grant.
fn repository(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    oracle(&repo, &["init", "-q", "-b", "main"]);
    oracle(&repo, &["config", "user.name", "Fixture"]);
    oracle(&repo, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(repo.join("file.txt"), "one\n").expect("write");
    oracle(&repo, &["add", "file.txt"]);
    oracle(&repo, &["commit", "-q", "-m", "one"]);
    oracle(&repo, &["branch", "feature/a"]);
    oracle(&repo, &["branch", "ünïcode"]);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    (repo, profile)
}

/// The names Git itself lists, in Git's order.
fn oracle_names(repo: &Path) -> Vec<String> {
    oracle(repo, &["for-each-ref", "--format=%(refname:lstrip=2)", "--sort=refname", "refs/heads/"])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_list_matches_git_and_marks_only_the_checked_out_branch() {
    let scratch = Scratch::new("list");
    let (repo, profile) = repository(&scratch);
    let shown = text(&cli(&profile, &["branch", p(&repo), "list"]));
    let lines: Vec<&str> = shown.lines().collect();
    let names = oracle_names(&repo);
    assert_eq!(lines.len(), names.len(), "{shown}");
    for (line, name) in lines.iter().zip(&names) {
        assert!(&line[2..] == name.as_str(), "{line:?} does not name {name:?}");
    }
    let current: Vec<&&str> = lines.iter().filter(|line| line.starts_with("* ")).collect();
    assert_eq!(current.len(), 1, "{shown}");
    assert_eq!(&current[0][2..], "main");
}

#[test]
fn the_marker_follows_a_switch_and_listed_names_switch_back() {
    let scratch = Scratch::new("switch");
    let (repo, profile) = repository(&scratch);
    for name in oracle_names(&repo) {
        let switched = cli(&profile, &["branch", p(&repo), "switch", &name]);
        assert!(switched.status.success(), "switch to {name}: {}", String::from_utf8_lossy(&switched.stderr));
        let shown = text(&cli(&profile, &["branch", p(&repo), "list"]));
        let current: Vec<&str> = shown.lines().filter(|line| line.starts_with("* ")).collect();
        assert_eq!(current, vec![format!("* {name}").as_str()], "{shown}");
    }
}

#[test]
fn an_unborn_branch_lists_no_branches() {
    let scratch = Scratch::new("unborn");
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    oracle(&repo, &["init", "-q", "-b", "main"]);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    assert_eq!(text(&cli(&profile, &["branch", p(&repo), "list"])), "");
}

#[test]
fn bidi_controls_in_a_branch_name_are_escaped_in_the_list() {
    let scratch = Scratch::new("bidi");
    let (repo, profile) = repository(&scratch);
    oracle(&repo, &["branch", "left\u{202E}right"]);
    let shown = text(&cli(&profile, &["branch", p(&repo), "list"]));
    assert!(shown.contains("left\\u{202E}right"), "{shown}");
    assert!(!shown.contains('\u{202E}'), "raw bidi control reached the list");
}

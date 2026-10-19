//! Commit list through the CLI, checked against `git log`. The list must match
//! Git's order and ids, mark merges, respect the limit, handle an unborn
//! branch, and escape bidirectional controls in subjects.

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
        let root = env::temp_dir().join(format!("srcmgr-history-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn commit_file(repo: &Path, file: &str, message: &str) {
    fs::write(repo.join(file), format!("{message}\n")).expect("write");
    oracle(repo, &["add", file]);
    oracle(repo, &["commit", "-q", "-m", message]);
}

/// main: base, main-2; side: side-1; then a non-fast-forward merge of side.
fn history_repository(scratch: &Scratch) -> PathBuf {
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    oracle(&repo, &["init", "-q", "-b", "main"]);
    oracle(&repo, &["config", "user.name", "Fixture"]);
    oracle(&repo, &["config", "user.email", "fixture@example.invalid"]);
    commit_file(&repo, "base.txt", "base");
    oracle(&repo, &["switch", "-q", "-c", "side"]);
    commit_file(&repo, "side.txt", "side one");
    oracle(&repo, &["switch", "-q", "main"]);
    commit_file(&repo, "main.txt", "main two");
    oracle(&repo, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    repo
}

#[test]
fn history_matches_git_log_order_and_marks_the_merge() {
    let scratch = Scratch::new("order");
    let repo = history_repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());

    let shown = text(&cli(&profile, &["history", p(&repo), "--limit", "10"]));
    let expected_ids: Vec<String> = oracle(&repo, &["log", "--format=%H", "--max-count=10"])
        .lines()
        .map(str::to_owned)
        .collect();
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines.len(), expected_ids.len(), "{shown}");
    for (line, id) in lines.iter().zip(&expected_ids) {
        // The first two columns are the merge marker and a space; the id follows.
        let short = line[2..].split_whitespace().next().expect("short id");
        assert!(id.starts_with(short), "{line} is not a prefix of {id}");
    }
    let merge_lines = lines.iter().filter(|line| line.starts_with("M ")).count();
    assert_eq!(merge_lines, 1, "{shown}");
    assert!(lines[0].ends_with("merge side"), "{shown}");
}

#[test]
fn the_limit_is_respected() {
    let scratch = Scratch::new("limit");
    let repo = history_repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    let shown = text(&cli(&profile, &["history", p(&repo), "--limit", "2"]));
    assert_eq!(shown.lines().count(), 2, "{shown}");
}

#[test]
fn paging_with_skip_matches_git_log_skip() {
    let scratch = Scratch::new("paging");
    let repo = history_repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());

    let shown = text(&cli(&profile, &["history", p(&repo), "--skip", "1", "--limit", "2"]));
    let expected: Vec<String> = oracle(&repo, &["log", "--format=%H", "--skip=1", "--max-count=2"])
        .lines()
        .map(str::to_owned)
        .collect();
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines.len(), expected.len(), "{shown}");
    for (line, id) in lines.iter().zip(&expected) {
        let short = line[2..].split_whitespace().next().expect("short id");
        assert!(id.starts_with(short), "{line} is not a prefix of {id}");
    }
}

#[test]
fn an_unborn_branch_has_no_history() {
    let scratch = Scratch::new("unborn");
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    oracle(&repo, &["init", "-q", "-b", "main"]);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    assert_eq!(text(&cli(&profile, &["history", p(&repo)])).trim(), "no commits yet");
}

#[test]
fn bidi_controls_in_a_subject_are_escaped() {
    let scratch = Scratch::new("bidi");
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    oracle(&repo, &["init", "-q", "-b", "main"]);
    oracle(&repo, &["config", "user.name", "Fixture"]);
    oracle(&repo, &["config", "user.email", "fixture@example.invalid"]);
    commit_file(&repo, "a.txt", "left\u{202E}right");
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    let shown = text(&cli(&profile, &["history", p(&repo)]));
    assert!(shown.contains("left\\u{202E}right"), "{shown}");
    assert!(!shown.contains('\u{202E}'), "raw bidi control reached the list");
}

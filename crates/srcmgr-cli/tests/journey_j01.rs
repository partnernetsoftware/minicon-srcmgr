//! QA-J01 subset for the M1 local closure: open, mixed states, first commit,
//! dirty switch, and escaped paths. Expected results are checked against Git's
//! own plumbing (index entries, blob hashes, refs), not against CLI output.

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
        let root = env::temp_dir().join(format!("srcmgr-j01-{}-{nanos}-{unique}-{name}", std::process::id()));
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

/// Runs the oracle Git and returns its trimmed stdout. Panics on failure.
fn oracle(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new(git_executable())
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run oracle git");
    assert!(output.status.success(), "oracle git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn fixture_git(directory: &Path, arguments: &[&str]) {
    oracle(directory, arguments);
}

/// An empty repository on `main` with a local identity; no commits yet.
fn unborn(scratch: &Scratch) -> PathBuf {
    let root = scratch.root.join("repo");
    fs::create_dir_all(&root).expect("create repo");
    fixture_git(&root, &["init", "-q", "-b", "main"]);
    fixture_git(&root, &["config", "user.name", "Fixture"]);
    fixture_git(&root, &["config", "user.email", "fixture@example.invalid"]);
    root
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

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn p(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

/// Blob id Git would store for these exact bytes.
fn blob_of(directory: &Path, file: &str) -> String {
    oracle(directory, &["hash-object", "--", file])
}

/// `ls-files --stage` line for `file`, reduced to `mode blob` (Git's index truth).
fn index_entry(directory: &Path, file: &str) -> String {
    let line = oracle(directory, &["ls-files", "--stage", "--", file]);
    let mut fields = line.split_whitespace();
    let mode = fields.next().unwrap_or_default().to_owned();
    let blob = fields.next().unwrap_or_default().to_owned();
    format!("{mode} {blob}")
}

#[test]
fn open_unborn_repository_and_make_the_first_commit() {
    let scratch = Scratch::new("first-commit");
    let repo = unborn(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());

    let before = cli(&profile, &["status", p(&repo)]);
    assert!(text(&before).starts_with("On branch main (no commits yet)"), "{}", text(&before));

    fs::write(repo.join("first.txt"), "hello\n").expect("write");
    assert!(cli(&profile, &["stage", p(&repo), "first.txt"]).status.success());
    let committed = cli(&profile, &["commit", p(&repo), "-m", "first"]);
    assert!(committed.status.success(), "{}", err(&committed));

    // The index and HEAD must match what Git itself reports.
    assert_eq!(index_entry(&repo, "first.txt"), format!("100644 {}", blob_of(&repo, "first.txt")));
    let head = oracle(&repo, &["rev-parse", "HEAD"]);
    assert!(text(&committed).contains(&head[..12]), "{}", text(&committed));
    assert_eq!(oracle(&repo, &["log", "-1", "--format=%s"]), "first");
}

#[test]
fn mixed_states_are_classified_and_stage_matches_the_index_oracle() {
    let scratch = Scratch::new("mixed");
    let repo = unborn(&scratch);
    fs::write(repo.join("base.txt"), "base\n").expect("write");
    fixture_git(&repo, &["add", "base.txt"]);
    fixture_git(&repo, &["commit", "-q", "-m", "base"]);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());

    fs::write(repo.join("base.txt"), "changed\n").expect("modify");
    fs::write(repo.join("new file.txt"), "fresh\n").expect("create");
    fs::write(repo.join("loose.txt"), "untracked\n").expect("create untracked");
    assert!(cli(&profile, &["stage", p(&repo), "base.txt", "new file.txt"]).status.success());

    let status = text(&cli(&profile, &["status", p(&repo)]));
    assert!(status.contains("M. base.txt"), "{status}");
    assert!(status.contains("A. new file.txt"), "{status}");
    assert!(status.contains(".? loose.txt"), "{status}");

    // Oracle: the index holds the staged bytes, and the untracked file is not in it.
    assert_eq!(index_entry(&repo, "base.txt"), format!("100644 {}", blob_of(&repo, "base.txt")));
    assert_eq!(index_entry(&repo, "new file.txt"), format!("100644 {}", blob_of(&repo, "new file.txt")));
    assert!(oracle(&repo, &["ls-files", "--", "loose.txt"]).is_empty());
}

#[test]
fn unstage_restores_the_head_blob_in_the_index() {
    let scratch = Scratch::new("unstage");
    let repo = unborn(&scratch);
    fs::write(repo.join("base.txt"), "base\n").expect("write");
    fixture_git(&repo, &["add", "base.txt"]);
    fixture_git(&repo, &["commit", "-q", "-m", "base"]);
    let head_blob = index_entry(&repo, "base.txt");
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());

    fs::write(repo.join("base.txt"), "changed\n").expect("modify");
    assert!(cli(&profile, &["stage", p(&repo), "base.txt"]).status.success());
    assert_ne!(index_entry(&repo, "base.txt"), head_blob);
    assert!(cli(&profile, &["unstage", p(&repo), "base.txt"]).status.success());
    assert_eq!(index_entry(&repo, "base.txt"), head_blob);
    // The working-tree bytes survive the unstage.
    assert_eq!(fs::read_to_string(repo.join("base.txt")).expect("read"), "changed\n");
}

#[test]
fn dirty_switch_that_would_overwrite_local_edits_is_refused_and_changes_nothing() {
    let scratch = Scratch::new("dirty-switch");
    let repo = unborn(&scratch);
    fs::write(repo.join("shared.txt"), "main\n").expect("write");
    fixture_git(&repo, &["add", "shared.txt"]);
    fixture_git(&repo, &["commit", "-q", "-m", "base"]);
    fixture_git(&repo, &["switch", "-q", "-c", "other"]);
    fs::write(repo.join("shared.txt"), "other\n").expect("write");
    fixture_git(&repo, &["commit", "-q", "-am", "other side"]);
    fixture_git(&repo, &["switch", "-q", "main"]);

    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    fs::write(repo.join("shared.txt"), "local edit\n").expect("local edit");

    let switched = cli(&profile, &["branch", p(&repo), "switch", "other"]);
    assert!(!switched.status.success(), "switch should be refused");
    assert_eq!(oracle(&repo, &["symbolic-ref", "--short", "HEAD"]), "main");
    assert_eq!(fs::read_to_string(repo.join("shared.txt")).expect("read"), "local edit\n");
}

#[test]
fn escaped_and_dash_prefixed_paths_round_trip_through_stage_and_commit() {
    let scratch = Scratch::new("escapes");
    let repo = unborn(&scratch);
    fs::write(repo.join("base.txt"), "base\n").expect("write");
    fixture_git(&repo, &["add", "base.txt"]);
    fixture_git(&repo, &["commit", "-q", "-m", "base"]);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());

    // Spaces, non-ASCII and a leading dash: all must be taken literally.
    let names = ["sp ace.txt", "中文.txt", "-dash.txt"];
    for name in names {
        fs::write(repo.join(name), format!("{name}\n")).expect("write");
    }
    let staged = cli(&profile, &["stage", p(&repo), "sp ace.txt", "中文.txt", "./-dash.txt"]);
    assert!(!staged.status.success(), "a non-changed literal ./path must be refused, not expanded");

    assert!(cli(&profile, &["stage", p(&repo), "sp ace.txt", "中文.txt", "-dash.txt"]).status.success());
    assert!(cli(&profile, &["commit", p(&repo), "-m", "escaped"]).status.success());
    for name in names {
        assert_eq!(index_entry(&repo, name), format!("100644 {}", blob_of(&repo, name)), "{name}");
    }
    assert_eq!(oracle(&repo, &["status", "--porcelain"]), "");
}

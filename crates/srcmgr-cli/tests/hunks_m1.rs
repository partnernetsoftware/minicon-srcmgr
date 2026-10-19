//! M1 hunk staging through the CLI. A file with two distant edits has two
//! hunks; staging hunk 1 must put exactly that change into the index, leave the
//! working tree untouched, and be reversible. Expected index bytes are blobs
//! Git computes from the expected file contents.

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
        let root = env::temp_dir().join(format!("srcmgr-hunks-{}-{nanos}-{unique}-{name}", std::process::id()));
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
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn cli(profile: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minicon-srcmgr"))
        .args(["--profile", profile.to_str().expect("utf-8 profile")])
        .args(["--git", git_executable().to_str().expect("utf-8 git")])
        .args(arguments)
        .output()
        .expect("run minicon-srcmgr")
}

fn p(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

fn numbered(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn blob_of_text(scratch: &Scratch, name: &str, text: &str) -> String {
    let file = scratch.root.join(name);
    fs::write(&file, text).expect("write expected");
    oracle(&scratch.root, &["hash-object", "--", p(&file)])
}

/// A committed 20-line file, then edits at lines 1 and 20 (two hunks).
/// Returns the repository, the profile, and the original and edited lines.
fn two_hunk_repository(scratch: &Scratch) -> (PathBuf, PathBuf, Vec<String>, Vec<String>) {
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    oracle(&repo, &["init", "-q", "-b", "main"]);
    oracle(&repo, &["config", "user.name", "Fixture"]);
    oracle(&repo, &["config", "user.email", "fixture@example.invalid"]);
    let original: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
    fs::write(repo.join("a.txt"), numbered(&original)).expect("write base");
    oracle(&repo, &["add", "a.txt"]);
    oracle(&repo, &["commit", "-q", "-m", "base"]);

    let mut edited = original.clone();
    edited[0] = "LINE ONE".to_owned();
    edited[19] = "LINE TWENTY".to_owned();
    fs::write(repo.join("a.txt"), numbered(&edited)).expect("write edits");

    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    (repo, profile, original, edited)
}

#[test]
fn list_shows_both_hunks_of_an_unstaged_change() {
    let scratch = Scratch::new("list");
    let (repo, profile, _, _) = two_hunk_repository(&scratch);
    let listed = cli(&profile, &["hunk", p(&repo), "list", "a.txt"]);
    assert!(listed.status.success());
    let text = String::from_utf8_lossy(&listed.stdout).into_owned();
    let labels = text.lines().filter(|line| line.starts_with("hunk ")).count();
    let headers = text.lines().filter(|line| line.starts_with("@@")).count();
    assert_eq!((labels, headers), (2, 2), "{text}");
}

#[test]
fn staging_hunk_one_puts_only_that_change_in_the_index() {
    let scratch = Scratch::new("stage");
    let (repo, profile, original, edited) = two_hunk_repository(&scratch);
    let worktree_before = fs::read(repo.join("a.txt")).expect("read");

    let staged = cli(&profile, &["hunk", p(&repo), "stage", "a.txt", "1"]);
    assert!(staged.status.success(), "{}", String::from_utf8_lossy(&staged.stderr));

    // Expected index: the first edit applied, the second still at its original text.
    let mut expected = original.clone();
    expected[0] = edited[0].clone();
    let index_blob = oracle(&repo, &["ls-files", "--stage", "--", "a.txt"])
        .split_whitespace()
        .nth(1)
        .expect("index blob")
        .to_owned();
    assert_eq!(index_blob, blob_of_text(&scratch, "expected-staged.txt", &numbered(&expected)));

    // The working tree is untouched by staging.
    assert_eq!(fs::read(repo.join("a.txt")).expect("read"), worktree_before);
    // The remaining change is still unstaged.
    let remaining = oracle(&repo, &["diff", "--", "a.txt"]);
    assert!(remaining.contains("+LINE TWENTY") && !remaining.contains("+LINE ONE"), "{remaining}");
}

#[test]
fn unstaging_a_staged_hunk_restores_the_head_blob() {
    let scratch = Scratch::new("unstage");
    let (repo, profile, _, _) = two_hunk_repository(&scratch);
    let head_blob = oracle(&repo, &["ls-files", "--stage", "--", "a.txt"])
        .split_whitespace()
        .nth(1)
        .expect("blob")
        .to_owned();

    assert!(cli(&profile, &["hunk", p(&repo), "stage", "a.txt", "1"]).status.success());
    assert!(cli(&profile, &["hunk", p(&repo), "unstage", "a.txt", "1"]).status.success());

    let index_blob = oracle(&repo, &["ls-files", "--stage", "--", "a.txt"])
        .split_whitespace()
        .nth(1)
        .expect("blob")
        .to_owned();
    assert_eq!(index_blob, head_blob);
}

#[test]
fn an_out_of_range_hunk_number_changes_nothing() {
    let scratch = Scratch::new("range");
    let (repo, profile, _, _) = two_hunk_repository(&scratch);
    let before = oracle(&repo, &["ls-files", "--stage", "--", "a.txt"]);

    let refused = cli(&profile, &["hunk", p(&repo), "stage", "a.txt", "9"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("no hunk has that number"));
    assert_eq!(oracle(&repo, &["ls-files", "--stage", "--", "a.txt"]), before);
}

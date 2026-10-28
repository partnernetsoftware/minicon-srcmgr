//! Draft recovery through the CLI: a saved draft is committed without `-m`, is
//! cleared after a successful commit, and survives a refused commit. A draft
//! never bypasses the trust gate.

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
        let root = env::temp_dir().join(format!("srcmgr-draft-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn fixture_git(directory: &Path, arguments: &[&str]) {
    let output = Command::new(git_executable())
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run fixture git");
    assert!(output.status.success(), "fixture git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
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

/// A repository with one commit and a staged change ready to commit.
fn staged_repository(scratch: &Scratch) -> PathBuf {
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    fixture_git(&repo, &["init", "-q", "-b", "main"]);
    fixture_git(&repo, &["config", "user.name", "Fixture"]);
    fixture_git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(repo.join("a.txt"), "one\n").expect("write");
    fixture_git(&repo, &["add", "a.txt"]);
    fixture_git(&repo, &["commit", "-q", "-m", "initial"]);
    fs::write(repo.join("a.txt"), "two\n").expect("modify");
    repo
}

#[test]
fn a_saved_draft_is_committed_without_a_message_and_then_cleared() {
    let scratch = Scratch::new("commit-draft");
    let repo = staged_repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    assert!(cli(&profile, &["stage", p(&repo), "a.txt"]).status.success());

    assert!(cli(&profile, &["draft", p(&repo), "save", "-m", "from the draft"]).status.success());
    assert_eq!(text(&cli(&profile, &["draft", p(&repo), "show"])), "from the draft");

    let committed = cli(&profile, &["commit", p(&repo)]);
    assert!(committed.status.success(), "{}", err(&committed));
    let subject = Command::new(git_executable())
        .current_dir(&repo)
        .args(["log", "-1", "--format=%s"])
        .output()
        .expect("log");
    assert_eq!(String::from_utf8_lossy(&subject.stdout).trim(), "from the draft");
    assert_eq!(text(&cli(&profile, &["draft", p(&repo), "show"])).trim(), "no draft");
}

#[test]
fn a_refused_commit_keeps_the_draft_for_the_next_attempt() {
    let scratch = Scratch::new("keep-draft");
    let repo = staged_repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    assert!(cli(&profile, &["draft", p(&repo), "save", "-m", "keep me"]).status.success());

    // Nothing is staged, so the commit is refused and the draft must remain.
    let refused = cli(&profile, &["commit", p(&repo)]);
    assert!(!refused.status.success());
    assert!(err(&refused).to_ascii_lowercase().contains("nothing is staged"), "{}", err(&refused));
    assert_eq!(text(&cli(&profile, &["draft", p(&repo), "show"])), "keep me");
}

#[test]
fn a_draft_does_not_bypass_the_trust_gate() {
    let scratch = Scratch::new("draft-trust");
    let repo = staged_repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["draft", p(&repo), "save", "-m", "sneaky"]).status.success());
    assert_eq!(text(&cli(&profile, &["draft", p(&repo), "show"])), "sneaky");

    let output = cli(&profile, &["commit", p(&repo)]);
    assert!(!output.status.success());
    assert!(err(&output).contains("not trusted yet"), "{}", err(&output));
}

#[test]
fn draft_clear_removes_the_saved_message() {
    let scratch = Scratch::new("clear");
    let repo = staged_repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["draft", p(&repo), "save", "-m", "gone soon"]).status.success());
    assert!(cli(&profile, &["draft", p(&repo), "clear"]).status.success());
    assert_eq!(text(&cli(&profile, &["draft", p(&repo), "show"])).trim(), "no draft");
}

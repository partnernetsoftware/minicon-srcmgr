//! QA-J11 subset that this Windows host can exercise: supplementary-plane names,
//! bidirectional controls in display labels, and bracket names that a glob would
//! expand. Expected index state comes from Git plumbing, not from the CLI.
//! Unix byte-level names (invalid UTF-8, newlines) need a Unix host and are not
//! run here.

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
        let root = env::temp_dir().join(format!("srcmgr-j11-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn p(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

/// Repository with one commit, a local identity and a trust grant.
fn granted_repository(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    oracle(&repo, &["init", "-q", "-b", "main"]);
    oracle(&repo, &["config", "user.name", "Fixture"]);
    oracle(&repo, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(repo.join("base.txt"), "base\n").expect("write");
    oracle(&repo, &["add", "base.txt"]);
    oracle(&repo, &["commit", "-q", "-m", "base"]);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    (repo, profile)
}

fn index_blob(repo: &Path, file: &str) -> String {
    let line = oracle(repo, &["ls-files", "--stage", "--", file]);
    line.split_whitespace().nth(1).unwrap_or_default().to_owned()
}

fn blob(repo: &Path, file: &str) -> String {
    oracle(repo, &["hash-object", "--", file])
}

#[test]
fn supplementary_plane_name_stages_and_commits_with_exact_index_bytes() {
    let scratch = Scratch::new("emoji");
    let (repo, profile) = granted_repository(&scratch);
    let name = "🙂 note.txt";
    fs::write(repo.join(name), "smile\n").expect("write emoji name");

    let staged = cli(&profile, &["stage", p(&repo), name]);
    assert!(staged.status.success(), "{}", err(&staged));
    assert_eq!(index_blob(&repo, name), blob(&repo, name));
    assert!(cli(&profile, &["commit", p(&repo), "-m", "emoji"]).status.success());
    assert_eq!(oracle(&repo, &["status", "--porcelain"]), "");
}

#[test]
fn bidi_control_in_a_name_is_escaped_in_every_display_label() {
    let scratch = Scratch::new("bidi");
    let (repo, profile) = granted_repository(&scratch);
    // U+202E (right-to-left override) is legal in a Windows file name.
    let name = "report\u{202E}gnp.txt";
    fs::write(repo.join(name), "x\n").expect("this host must accept a bidi control in a name for the check to run");

    let status = cli(&profile, &["status", p(&repo)]);
    let shown = text(&status);
    assert!(shown.contains("\\u{202E}"), "{shown}");
    assert!(!shown.contains('\u{202E}'), "raw bidi control reached the label");

    let json = text(&cli(&profile, &["status", p(&repo), "--json"]));
    assert!(json.contains("\\u{202E}"), "{json}");
    assert!(!json.contains('\u{202E}'), "raw bidi control reached JSON");
}

#[test]
fn bracket_name_is_staged_literally_not_as_a_glob() {
    let scratch = Scratch::new("brackets");
    let (repo, profile) = granted_repository(&scratch);
    fs::write(repo.join("lit[1].txt"), "bracket\n").expect("write bracket name");
    fs::write(repo.join("lit1.txt"), "digit\n").expect("write lookalike");

    // As a glob, lit[1].txt would match lit1.txt; the literal pathspec must not.
    assert!(cli(&profile, &["stage", p(&repo), "lit[1].txt"]).status.success());
    assert_eq!(index_blob(&repo, "lit[1].txt"), blob(&repo, "lit[1].txt"));
    assert!(oracle(&repo, &["ls-files", "--", "lit1.txt"]).is_empty(), "the glob matched a lookalike");
}

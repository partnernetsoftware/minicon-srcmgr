//! Ignore rules through the CLI, checked against `git check-ignore -v`. An
//! ignored untracked file must stay out of status and must not be stageable;
//! negation and `info/exclude` must behave as Git does.

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
        let root = env::temp_dir().join(format!("srcmgr-ignore-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn oracle(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(git_executable())
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run oracle git")
}

fn oracle_ok(directory: &Path, arguments: &[&str]) {
    let output = oracle(directory, arguments);
    assert!(output.status.success(), "oracle git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
}

/// Git's own rule for `path`: the `source:line:pattern` part, or `None` when the
/// path is not ignored. Exit 1 means no match; a negated pattern (`!name`) is
/// reported with exit 0 but re-includes the path, so it also means not ignored.
fn git_rule(directory: &Path, path: &str) -> Option<String> {
    let output = oracle(directory, &["check-ignore", "-v", "--", path]);
    match output.status.code() {
        Some(0) => {
            let rule = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or_default()
                .split('\t')
                .next()
                .unwrap_or_default()
                .to_owned();
            let pattern = rule.rsplit_once(':').map_or("", |(_, pattern)| pattern);
            if rule.contains(":!") || pattern.starts_with('!') { None } else { Some(rule) }
        }
        Some(1) => None,
        other => panic!("oracle check-ignore failed with {other:?}"),
    }
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

/// A repository with `.gitignore` (`*.log`, with `!keep.log`) and an
/// `info/exclude` rule (`secret.tmp`), one commit, and the trust grant.
fn scoped_repository(scratch: &Scratch) -> (PathBuf, PathBuf) {
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    oracle_ok(&repo, &["init", "-q", "-b", "main"]);
    oracle_ok(&repo, &["config", "user.name", "Fixture"]);
    oracle_ok(&repo, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(repo.join(".gitignore"), "*.log\n!keep.log\n").expect("gitignore");
    fs::write(repo.join("base.txt"), "base\n").expect("base");
    oracle_ok(&repo, &["add", "."]);
    oracle_ok(&repo, &["commit", "-q", "-m", "base"]);
    fs::write(repo.join(".git").join("info").join("exclude"), "secret.tmp\n").expect("exclude");
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    (repo, profile)
}

#[test]
fn an_ignored_file_is_named_with_the_same_rule_as_git() {
    let scratch = Scratch::new("rule");
    let (repo, profile) = scoped_repository(&scratch);
    fs::write(repo.join("build.log"), "noise\n").expect("write");

    let shown = text(&cli(&profile, &["ignore", p(&repo), "build.log"]));
    let oracle_rule = git_rule(&repo, "build.log").expect("git ignores build.log");
    assert_eq!(shown.trim(), format!("ignored by {oracle_rule}"));
    assert!(shown.contains(".gitignore:1:*.log"), "{shown}");
}

#[test]
fn negation_keeps_a_file_that_the_pattern_would_ignore() {
    let scratch = Scratch::new("negation");
    let (repo, profile) = scoped_repository(&scratch);
    fs::write(repo.join("keep.log"), "kept\n").expect("write");

    assert!(git_rule(&repo, "keep.log").is_none(), "git does not ignore keep.log");
    assert_eq!(text(&cli(&profile, &["ignore", p(&repo), "keep.log"])).trim(), "not ignored");
}

#[test]
fn info_exclude_rules_are_honoured_and_named() {
    let scratch = Scratch::new("exclude");
    let (repo, profile) = scoped_repository(&scratch);
    fs::write(repo.join("secret.tmp"), "x\n").expect("write");

    let shown = text(&cli(&profile, &["ignore", p(&repo), "secret.tmp"]));
    assert!(shown.contains("info/exclude"), "{shown}");
    assert_eq!(
        shown.trim(),
        format!("ignored by {}", git_rule(&repo, "secret.tmp").expect("git ignores secret.tmp"))
    );
}

#[test]
fn ignored_files_stay_out_of_status_and_cannot_be_staged() {
    let scratch = Scratch::new("status");
    let (repo, profile) = scoped_repository(&scratch);
    fs::write(repo.join("build.log"), "noise\n").expect("write");

    let status = text(&cli(&profile, &["status", p(&repo)]));
    assert!(!status.contains("build.log"), "{status}");
    assert!(status.contains("Working tree clean"), "{status}");

    let refused = cli(&profile, &["stage", p(&repo), "build.log"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("not a changed path"));
}

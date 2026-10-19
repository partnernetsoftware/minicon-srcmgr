//! QA-J10 subset: a hostile repository configuration must not execute before
//! trust is granted, and any change after a grant must stop execution again.
//! A positive control proves the marker program does run once trust is granted,
//! so an absent marker is evidence of no execution rather than a broken probe.

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
        let root = env::temp_dir().join(format!("srcmgr-j10-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn p(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

/// A repository with one commit, and a marker program wired in as the
/// `core.fsmonitor` command. Git runs that command during status refreshes.
struct Hostile {
    repo: PathBuf,
    marker: PathBuf,
}

fn hostile_repository(scratch: &Scratch) -> Hostile {
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    fixture_git(&repo, &["init", "-q", "-b", "main"]);
    fixture_git(&repo, &["config", "user.name", "Fixture"]);
    fixture_git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(repo.join("a.txt"), "one\n").expect("write");
    fixture_git(&repo, &["add", "a.txt"]);
    fixture_git(&repo, &["commit", "-q", "-m", "initial"]);

    let marker = scratch.root.join("ran.marker");
    let script = scratch.root.join("fsmonitor.sh");
    let script_text = format!("#!/bin/sh\necho ran >> \"{}\"\n", p(&marker).replace('\\', "/"));
    fs::write(&script, script_text).expect("write script");
    let command = format!("sh \"{}\"", p(&script).replace('\\', "/"));
    fixture_git(&repo, &["config", "core.fsmonitor", &command]);
    Hostile { repo, marker }
}

#[test]
fn untrusted_status_does_not_run_the_repository_fsmonitor_command() {
    let scratch = Scratch::new("untrusted");
    let hostile = hostile_repository(&scratch);
    let profile = scratch.root.join("profile");

    let output = cli(&profile, &["status", p(&hostile.repo)]);
    assert!(!output.status.success());
    assert!(err(&output).contains("not trusted yet"), "{}", err(&output));
    assert!(!hostile.marker.exists(), "the repository command ran before trust was granted");
}

#[test]
fn granted_status_runs_the_command_which_proves_the_probe_works() {
    let scratch = Scratch::new("positive-control");
    let hostile = hostile_repository(&scratch);
    let profile = scratch.root.join("profile");
    let granted = cli(&profile, &["trust", "grant", p(&hostile.repo)]);
    assert!(granted.status.success(), "{}", err(&granted));

    // The exit status of status is not the subject here; only whether the
    // configured command ran under the grant.
    let _ = cli(&profile, &["status", p(&hostile.repo)]);
    assert!(hostile.marker.exists(), "the positive control did not run; the probe is not trustworthy");
}

#[test]
fn a_config_change_after_the_grant_stops_execution_again() {
    let scratch = Scratch::new("changed");
    let hostile = hostile_repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&hostile.repo)]).status.success());

    // Attacker-style edit after the user granted trust: a new hook-like key.
    fixture_git(&hostile.repo, &["config", "core.quotePath", "false"]);
    let _ = fs::remove_file(&hostile.marker);

    let output = cli(&profile, &["status", p(&hostile.repo)]);
    assert!(!output.status.success());
    assert!(err(&output).contains("changed since trust was granted"), "{}", err(&output));
    assert!(!hostile.marker.exists(), "a changed, untrusted configuration still executed");
}

#[test]
fn a_revoked_grant_stops_execution() {
    let scratch = Scratch::new("revoked");
    let hostile = hostile_repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&hostile.repo)]).status.success());
    assert!(cli(&profile, &["trust", "revoke", p(&hostile.repo)]).status.success());
    let _ = fs::remove_file(&hostile.marker);

    let output = cli(&profile, &["status", p(&hostile.repo)]);
    assert!(!output.status.success());
    assert!(!hostile.marker.exists(), "a revoked repository still executed its command");
}

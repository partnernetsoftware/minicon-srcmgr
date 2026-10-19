//! QA-J10 subset for hooks and clean filters. Before trust, neither a commit
//! hook nor a clean filter may run; under a grant each runs (positive
//! control); and a change after the grant stops them again. Each program
//! records its execution in a marker file.

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
        let root = env::temp_dir().join(format!("srcmgr-j10b-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn script(scratch: &Scratch, name: &str, marker: &Path, body: &str) -> PathBuf {
    let path = scratch.root.join(name);
    let text = format!("#!/bin/sh\necho ran >> \"{}\"\n{body}\n", p(marker).replace('\\', "/"));
    fs::write(&path, text).expect("write script");
    path
}

/// A repository with one commit and an unstaged change ready to commit.
fn repository(scratch: &Scratch) -> PathBuf {
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

/// Installs a commit hook that records its run and succeeds.
fn install_pre_commit(scratch: &Scratch, repo: &Path, marker: &Path) {
    let hook = script(scratch, "pre-commit.sh", marker, "exit 0");
    let hooks = repo.join(".git").join("hooks");
    fs::create_dir_all(&hooks).expect("hooks dir");
    let body = format!("#!/bin/sh\nsh \"{}\"\n", p(&hook).replace('\\', "/"));
    fs::write(hooks.join("pre-commit"), body).expect("write hook");
}

#[test]
fn untrusted_commit_does_not_run_the_pre_commit_hook() {
    let scratch = Scratch::new("hook-untrusted");
    let repo = repository(&scratch);
    let marker = scratch.root.join("hook.marker");
    install_pre_commit(&scratch, &repo, &marker);
    let profile = scratch.root.join("profile");

    let output = cli(&profile, &["commit", p(&repo), "-m", "x"]);
    assert!(!output.status.success());
    assert!(err(&output).contains("not trusted yet"), "{}", err(&output));
    assert!(!marker.exists(), "the hook ran before trust was granted");
}

#[test]
fn granted_commit_runs_the_pre_commit_hook_as_a_positive_control() {
    let scratch = Scratch::new("hook-granted");
    let repo = repository(&scratch);
    let marker = scratch.root.join("hook.marker");
    install_pre_commit(&scratch, &repo, &marker);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    assert!(cli(&profile, &["stage", p(&repo), "a.txt"]).status.success());

    let output = cli(&profile, &["commit", p(&repo), "-m", "x"]);
    assert!(output.status.success(), "{}", err(&output));
    assert!(marker.exists(), "the positive control did not run the hook");
}

#[test]
fn a_hook_changed_after_the_grant_does_not_run_on_commit() {
    let scratch = Scratch::new("hook-changed");
    let repo = repository(&scratch);
    let marker = scratch.root.join("hook.marker");
    install_pre_commit(&scratch, &repo, &marker);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());
    assert!(cli(&profile, &["stage", p(&repo), "a.txt"]).status.success());

    // Edit the hook after the grant: the fingerprint must catch it.
    install_pre_commit(&scratch, &repo, &marker);
    fs::write(repo.join(".git").join("hooks").join("pre-commit"), "#!/bin/sh\nexit 0\n").expect("edit hook");
    let _ = fs::remove_file(&marker);

    let output = cli(&profile, &["commit", p(&repo), "-m", "x"]);
    assert!(!output.status.success());
    assert!(err(&output).contains("changed since trust was granted"), "{}", err(&output));
    assert!(!marker.exists(), "a changed hook still ran");
}

#[test]
fn untrusted_stage_does_not_run_the_clean_filter() {
    let scratch = Scratch::new("filter-untrusted");
    let repo = repository(&scratch);
    let marker = scratch.root.join("filter.marker");
    let filter = script(&scratch, "clean.sh", &marker, "cat");
    fs::write(repo.join(".gitattributes"), "*.txt filter=hostile\n").expect("attributes");
    fixture_git(&repo, &["config", "filter.hostile.clean", &format!("sh \"{}\"", p(&filter).replace('\\', "/"))]);
    let profile = scratch.root.join("profile");

    let output = cli(&profile, &["stage", p(&repo), "a.txt"]);
    assert!(!output.status.success());
    assert!(!marker.exists(), "the clean filter ran before trust was granted");
}

#[test]
fn granted_stage_runs_the_clean_filter_as_a_positive_control() {
    let scratch = Scratch::new("filter-granted");
    let repo = repository(&scratch);
    let marker = scratch.root.join("filter.marker");
    let filter = script(&scratch, "clean.sh", &marker, "cat");
    fs::write(repo.join(".gitattributes"), "*.txt filter=hostile\n").expect("attributes");
    fixture_git(&repo, &["config", "filter.hostile.clean", &format!("sh \"{}\"", p(&filter).replace('\\', "/"))]);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());

    let output = cli(&profile, &["stage", p(&repo), "a.txt"]);
    assert!(output.status.success(), "{}", err(&output));
    assert!(marker.exists(), "the positive control did not run the clean filter");
}

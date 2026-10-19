//! End-to-end checks of the M1-A local slice through the real binary, against
//! disposable repositories and an installed Git (`SRCMGR_TEST_GIT`, else PATH).
//! The profile is a temporary directory, so no user state is touched.

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
        let root = env::temp_dir().join(format!("srcmgr-cli-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new(git_executable())
        .current_dir(directory)
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(arguments)
        .output()
        .expect("run fixture git");
    assert!(output.status.success(), "fixture git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
}

/// A repository with one commit, a local identity, and a clean tree.
fn repository(scratch: &Scratch) -> PathBuf {
    let root = scratch.root.join("repo");
    fs::create_dir_all(&root).expect("create repo");
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "Fixture"]);
    git(&root, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(root.join("a.txt"), "one\n").expect("write");
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "initial"]);
    root
}

fn cli(profile: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minicon-srcmgr"))
        .args(["--profile", profile.to_str().expect("utf-8 profile path")])
        .args(["--git", git_executable().to_str().expect("utf-8 git path")])
        .args(arguments)
        .output()
        .expect("run minicon-srcmgr")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("utf-8 test path")
}

#[test]
fn untrusted_repository_is_refused_before_git_runs() {
    let scratch = Scratch::new("untrusted");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    let output = cli(&profile, &["status", path_str(&repo)]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("not trusted yet"), "{}", stderr(&output));
}

#[test]
fn grant_then_status_shows_the_branch() {
    let scratch = Scratch::new("grant");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    let granted = cli(&profile, &["trust", "grant", path_str(&repo)]);
    assert!(granted.status.success(), "{}", stderr(&granted));
    let status = cli(&profile, &["status", path_str(&repo)]);
    assert!(status.status.success(), "{}", stderr(&status));
    assert!(stdout(&status).starts_with("On branch main ("), "{}", stdout(&status));
    assert!(stdout(&status).contains("Working tree clean"));
}

#[test]
fn stage_unstage_and_commit_round_trip() {
    let scratch = Scratch::new("round-trip");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", path_str(&repo)]).status.success());

    fs::write(repo.join("a.txt"), "two\n").expect("modify");
    let staged = cli(&profile, &["stage", path_str(&repo), "a.txt"]);
    assert!(staged.status.success(), "{}", stderr(&staged));
    assert!(stdout(&cli(&profile, &["status", path_str(&repo)])).contains("M. a.txt"));

    let unstaged = cli(&profile, &["unstage", path_str(&repo), "a.txt"]);
    assert!(unstaged.status.success(), "{}", stderr(&unstaged));
    assert!(stdout(&cli(&profile, &["status", path_str(&repo)])).contains(".M a.txt"));

    assert!(cli(&profile, &["stage", path_str(&repo), "a.txt"]).status.success());
    let committed = cli(&profile, &["commit", path_str(&repo), "-m", "second"]);
    assert!(committed.status.success(), "{}", stderr(&committed));
    assert!(stdout(&committed).starts_with("committed "));

    let after = stdout(&cli(&profile, &["status", path_str(&repo)]));
    assert!(after.contains("Working tree clean"), "{after}");
    let log = Command::new(git_executable())
        .current_dir(&repo)
        .args(["log", "-1", "--format=%s"])
        .output()
        .expect("log");
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "second");
}

#[test]
fn commit_without_anything_staged_is_refused() {
    let scratch = Scratch::new("nothing");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", path_str(&repo)]).status.success());
    let output = cli(&profile, &["commit", path_str(&repo), "-m", "empty"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("nothing is staged"));
}

#[test]
fn pathspec_patterns_are_not_expanded() {
    let scratch = Scratch::new("literal");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", path_str(&repo)]).status.success());
    fs::write(repo.join("b.txt"), "new\n").expect("write");
    let output = cli(&profile, &["stage", path_str(&repo), "*.txt"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("not a changed path"));
}

#[test]
fn changed_config_invalidates_the_grant() {
    let scratch = Scratch::new("config");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", path_str(&repo)]).status.success());
    let config = repo.join(".git").join("config");
    let mut text = fs::read_to_string(&config).expect("read config");
    text.push_str("[alias]\n\tx = status\n");
    fs::write(&config, text).expect("write config");
    let output = cli(&profile, &["status", path_str(&repo)]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("changed since trust was granted"), "{}", stderr(&output));
}

#[test]
fn a_hook_edit_invalidates_the_grant() {
    let scratch = Scratch::new("hook");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", path_str(&repo)]).status.success());
    let hooks = repo.join(".git").join("hooks");
    fs::create_dir_all(&hooks).expect("hooks dir");
    fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 0\n").expect("hook");
    let output = cli(&profile, &["status", path_str(&repo)]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("changed since trust was granted"));
}

#[test]
fn config_with_include_cannot_be_granted() {
    let scratch = Scratch::new("include");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    let config = repo.join(".git").join("config");
    let mut text = fs::read_to_string(&config).expect("read config");
    text.push_str("[include]\n\tpath = ../other.cfg\n");
    fs::write(&config, text).expect("write config");
    let output = cli(&profile, &["trust", "grant", path_str(&repo)]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("includes other files"));
}

#[test]
fn revoke_blocks_later_commands() {
    let scratch = Scratch::new("revoke");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", path_str(&repo)]).status.success());
    assert!(cli(&profile, &["trust", "revoke", path_str(&repo)]).status.success());
    let output = cli(&profile, &["status", path_str(&repo)]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("not trusted yet"));
}

#[test]
fn diff_shows_the_unstaged_and_staged_change_of_a_path() {
    let scratch = Scratch::new("diff");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", path_str(&repo)]).status.success());

    fs::write(repo.join("a.txt"), "two\n").expect("modify");
    let unstaged = cli(&profile, &["diff", path_str(&repo), "a.txt"]);
    assert!(unstaged.status.success(), "{}", stderr(&unstaged));
    let text = stdout(&unstaged);
    assert!(text.contains("-one") && text.contains("+two"), "{text}");

    assert!(cli(&profile, &["stage", path_str(&repo), "a.txt"]).status.success());
    let staged = cli(&profile, &["diff", path_str(&repo), "a.txt", "--staged"]);
    assert!(stdout(&staged).contains("+two"), "{}", stdout(&staged));
    assert!(stdout(&cli(&profile, &["diff", path_str(&repo), "a.txt"])).is_empty());
}

#[test]
fn branch_create_and_switch_update_the_status_heading() {
    let scratch = Scratch::new("branches");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", path_str(&repo)]).status.success());

    let created = cli(&profile, &["branch", path_str(&repo), "create", "feature/x"]);
    assert!(created.status.success(), "{}", stderr(&created));
    assert!(stdout(&cli(&profile, &["status", path_str(&repo)])).starts_with("On branch feature/x ("));

    let switched = cli(&profile, &["branch", path_str(&repo), "switch", "main"]);
    assert!(switched.status.success(), "{}", stderr(&switched));
    assert!(stdout(&cli(&profile, &["status", path_str(&repo)])).starts_with("On branch main ("));
}

#[test]
fn invalid_and_option_like_branch_names_are_refused_before_git_changes_anything() {
    let scratch = Scratch::new("bad-names");
    let repo = repository(&scratch);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", path_str(&repo)]).status.success());

    let dotted = cli(&profile, &["branch", path_str(&repo), "create", "bad..name"]);
    assert!(!dotted.status.success());
    assert!(stderr(&dotted).contains("not valid"), "{}", stderr(&dotted));

    let option_like = cli(&profile, &["branch", path_str(&repo), "create", "--orphan=x"]);
    assert!(!option_like.status.success());
    assert!(stdout(&cli(&profile, &["status", path_str(&repo)])).starts_with("On branch main ("));
}

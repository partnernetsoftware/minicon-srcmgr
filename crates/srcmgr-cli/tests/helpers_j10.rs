//! QA-J10 external-helper subset: the product's diff must not execute a
//! repository-configured external diff driver or textconv filter, even under a
//! grant. Raw `git diff` with the same configuration runs both, which is the
//! positive control that proves the markers can detect execution.

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
        let root = env::temp_dir().join(format!("srcmgr-j10h-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn fixture_git(directory: &Path, arguments: &[&str]) -> Output {
    let output = Command::new(git_executable())
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run fixture git");
    assert!(output.status.success(), "fixture git {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
    output
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

/// A program that records its run, then prints its last argument's content
/// (for textconv, the file path is the last argument).
fn program(scratch: &Scratch, name: &str, marker: &Path) -> PathBuf {
    let path = scratch.root.join(name);
    let body = format!(
        "#!/bin/sh\necho ran >> \"{}\"\nfor last in \"$@\"; do :; done\ncat \"$last\" 2>/dev/null\nexit 0\n",
        p(marker).replace('\\', "/")
    );
    fs::write(&path, body).expect("write program");
    path
}

/// A repository with a `.txt` file that has a textconv and an external diff.
fn configured_repository(scratch: &Scratch, external: &Path, textconv: &Path) -> PathBuf {
    let repo = scratch.root.join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    fixture_git(&repo, &["init", "-q", "-b", "main"]);
    fixture_git(&repo, &["config", "user.name", "Fixture"]);
    fixture_git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(repo.join("a.txt"), "one\n").expect("write");
    fs::write(repo.join(".gitattributes"), "*.txt diff=conv\n").expect("attributes");
    fixture_git(&repo, &["add", "."]);
    fixture_git(&repo, &["commit", "-q", "-m", "initial"]);
    fs::write(repo.join("a.txt"), "two\n").expect("modify");
    fixture_git(&repo, &["config", "diff.external", &format!("sh \"{}\"", p(external).replace('\\', "/"))]);
    fixture_git(&repo, &["config", "diff.conv.textconv", &format!("sh \"{}\"", p(textconv).replace('\\', "/"))]);
    repo
}

#[test]
fn product_diff_does_not_run_external_or_textconv_helpers_even_when_granted() {
    let scratch = Scratch::new("product");
    let external_marker = scratch.root.join("external.marker");
    let textconv_marker = scratch.root.join("textconv.marker");
    let external = program(&scratch, "external.sh", &external_marker);
    let textconv = program(&scratch, "textconv.sh", &textconv_marker);
    let repo = configured_repository(&scratch, &external, &textconv);
    let profile = scratch.root.join("profile");
    assert!(cli(&profile, &["trust", "grant", p(&repo)]).status.success());

    let output = cli(&profile, &["diff", p(&repo), "a.txt"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("+two"));
    assert!(!external_marker.exists(), "the external diff driver ran");
    assert!(!textconv_marker.exists(), "the textconv filter ran");
}

#[test]
fn raw_git_diff_runs_the_same_helpers_as_a_positive_control() {
    let scratch = Scratch::new("control");
    let external_marker = scratch.root.join("external.marker");
    let textconv_marker = scratch.root.join("textconv.marker");
    let external = program(&scratch, "external.sh", &external_marker);
    let textconv = program(&scratch, "textconv.sh", &textconv_marker);
    let repo = configured_repository(&scratch, &external, &textconv);

    // Plain `git diff` honours diff.external, which then takes precedence over
    // textconv, so this run proves the external driver can execute. The product
    // must not run it.
    fixture_git(&repo, &["diff", "--", "a.txt"]);
    assert!(external_marker.exists(), "the positive control did not run the external driver");
}

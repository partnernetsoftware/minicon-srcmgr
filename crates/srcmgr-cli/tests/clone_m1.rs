//! Clone and remote configuration through the product CLI, checked against the
//! destination's own metadata and configuration bytes.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!("srcmgr-clone-cli-{}-{nanos}-{unique}-{name}", std::process::id()));
        fs::create_dir_all(&root).expect("fixture");
        Self { root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git_executable() -> PathBuf {
    env::var_os("SRCMGR_TEST_GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from)
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new(git_executable())
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("fixture git");
    assert!(
        output.status.success(),
        "fixture git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn cli(profile: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minicon-srcmgr"))
        .args(["--profile", profile.to_str().expect("profile")])
        .args(["--git", git_executable().to_str().expect("git")])
        .args(arguments)
        .output()
        .expect("run product")
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn error(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn bare_with_commit(fixture: &Fixture) -> PathBuf {
    let bare = fixture.root.join("remote.git");
    fs::create_dir_all(&bare).expect("bare");
    git(&bare, &["init", "--bare", "--initial-branch=main", "."]);
    let source = fixture.root.join("source");
    git(&fixture.root, &["init", "--initial-branch=main", "source"]);
    for (key, value) in [("user.name", "Source"), ("user.email", "source@example.test")] {
        git(&source, &["config", key, value]);
    }
    fs::write(source.join("notes.txt"), "content\n").expect("write");
    git(&source, &["add", "notes.txt"]);
    git(&source, &["commit", "-m", "one"]);
    git(&source, &["push", bare.to_str().expect("bare"), "main"]);
    bare
}

#[test]
fn clone_defers_checkout_and_a_nonempty_target_is_preserved() {
    let fixture = Fixture::new("clone");
    let bare = bare_with_commit(&fixture);
    let profile = fixture.root.join("profile");
    let destination = fixture.root.join("dest");

    let cloned = cli(&profile, &["clone", bare.to_str().expect("bare"), destination.to_str().expect("dest")]);
    assert!(cloned.status.success(), "clone failed: {}", error(&cloned));
    assert!(destination.join(".git").is_dir());
    assert!(!destination.join("notes.txt").exists(), "checkout must be deferred");
    assert!(text(&cloned).contains("checkout is deferred"), "{}", text(&cloned));

    let occupied = fixture.root.join("occupied");
    fs::create_dir_all(&occupied).expect("occupied");
    fs::write(occupied.join("mine.txt"), "mine\n").expect("mine");
    let refused = cli(&profile, &["clone", bare.to_str().expect("bare"), occupied.to_str().expect("occupied")]);
    assert!(!refused.status.success());
    assert!(error(&refused).contains("not empty"), "{}", error(&refused));
    assert_eq!(fs::read_to_string(occupied.join("mine.txt")).expect("mine"), "mine\n");
}

#[test]
fn remote_changes_preview_first_and_apply_only_with_yes() {
    let fixture = Fixture::new("remote-edit");
    let bare = bare_with_commit(&fixture);
    let profile = fixture.root.join("profile");
    let worktree = fixture.root.join("work");
    let cloned = cli(&profile, &["clone", bare.to_str().expect("bare"), worktree.to_str().expect("work")]);
    assert!(cloned.status.success(), "clone failed: {}", error(&cloned));

    let granted = cli(&profile, &["trust", "grant", worktree.to_str().expect("work")]);
    assert!(granted.status.success(), "{}", error(&granted));
    let config = worktree.join(".git/config");

    let preview = cli(
        &profile,
        &["remote", worktree.to_str().expect("work"), "add", "backup", "https://example.test/backup.git"],
    );
    assert!(preview.status.success(), "{}", error(&preview));
    assert!(text(&preview).contains("preview only"), "{}", text(&preview));
    assert!(!fs::read_to_string(&config).expect("config").contains("backup"));

    let applied = cli(
        &profile,
        &["remote", worktree.to_str().expect("work"), "add", "backup", "https://example.test/backup.git", "--yes"],
    );
    assert!(applied.status.success(), "{}", error(&applied));
    assert!(fs::read_to_string(&config).expect("config").contains("backup"));
    assert!(text(&applied).contains("trust fingerprint changed"), "{}", text(&applied));
}

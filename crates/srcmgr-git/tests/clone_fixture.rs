//! Clone against a real bare remote: destination preflight, deferred checkout
//! and refusal without deletion.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use srcmgr_git::{CloneRequest, GitError, GitInstallation, clone_repository, preview_clone};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!("srcmgr-clone-{}-{nanos}-{unique}-{name}", std::process::id()));
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

fn git(directory: &Path, arguments: &[&str]) {
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
}

fn installation() -> GitInstallation {
    GitInstallation::probe(git_executable()).expect("probe")
}

/// A bare remote with one commit on `main` and one line of content.
fn remote(fixture: &Fixture) -> PathBuf {
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

fn request(bare: &Path, destination: PathBuf) -> CloneRequest {
    CloneRequest {
        url: bare.to_str().expect("path").to_owned(),
        destination,
        branch: None,
        depth: None,
    }
}

#[test]
fn clone_leaves_the_worktree_unchecked_out() {
    let fixture = Fixture::new("ok");
    let bare = remote(&fixture);
    let destination = fixture.root.join("dest");
    let change = request(&bare, destination.clone());
    let installation = installation();

    let preview = preview_clone(&installation, &change).expect("preview");
    assert!(preview.lines.iter().any(|line| line.contains("checkout is disabled")));

    clone_repository(&installation, &change, &AtomicBool::new(false), None).expect("clone");
    assert!(destination.join(".git").is_dir());
    assert!(!destination.join("notes.txt").exists(), "checkout must wait for review");

    let head = Command::new(git_executable())
        .current_dir(&destination)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("head");
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "main");
}

#[test]
fn a_nonempty_destination_is_refused_and_preserved() {
    let fixture = Fixture::new("nonempty");
    let bare = remote(&fixture);
    let destination = fixture.root.join("dest");
    fs::create_dir_all(&destination).expect("dest");
    fs::write(destination.join("keep.txt"), "mine\n").expect("keep");
    let change = request(&bare, destination.clone());
    let installation = installation();

    assert!(matches!(preview_clone(&installation, &change), Err(GitError::CloneTarget(_))));
    assert!(matches!(
        clone_repository(&installation, &change, &AtomicBool::new(false), None),
        Err(GitError::CloneTarget(_))
    ));
    assert_eq!(fs::read_to_string(destination.join("keep.txt")).expect("keep"), "mine\n");
    assert!(!destination.join(".git").exists(), "no metadata is written into a refused target");
}

#[test]
fn unsupported_transport_and_bad_options_create_nothing() {
    let fixture = Fixture::new("refused");
    let destination = fixture.root.join("dest");
    let installation = installation();

    let http = CloneRequest {
        url: "http://example.test/repo.git".into(),
        destination: destination.clone(),
        branch: None,
        depth: None,
    };
    assert!(matches!(preview_clone(&installation, &http), Err(GitError::InvalidRemoteUrl)));
    assert!(!destination.exists());

    let relative = CloneRequest {
        url: "https://example.test/repo.git".into(),
        destination: PathBuf::from("relative"),
        branch: None,
        depth: None,
    };
    assert!(matches!(preview_clone(&installation, &relative), Err(GitError::CloneTarget(_))));

    let branch = CloneRequest {
        url: "https://example.test/repo.git".into(),
        destination,
        branch: Some("-evil".into()),
        depth: Some(0),
    };
    assert!(matches!(preview_clone(&installation, &branch), Err(GitError::CloneTarget(_))));
}

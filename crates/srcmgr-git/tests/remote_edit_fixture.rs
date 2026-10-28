//! Reviewed remote configuration writes against real Git. The oracle is the
//! repository's own `.git/config` bytes: a preview and its applied change must
//! leave exactly the reviewed values and preserve unrelated keys.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use srcmgr_git::{
    GitError, GitInstallation, RemoteChange, apply_remote_change, locate, preview_remote_change,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!("srcmgr-remote-edit-{}-{nanos}-{unique}-{name}", std::process::id()));
        fs::create_dir_all(&root).expect("fixture root");
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

fn repository(name: &str) -> (Fixture, GitInstallation, srcmgr_git::RepositoryIdentity) {
    let fixture = Fixture::new(name);
    let worktree = fixture.root.join("work");
    fs::create_dir_all(&worktree).expect("worktree");
    git(&worktree, &["init", "--initial-branch=main", "."]);
    git(&worktree, &["remote", "add", "origin", "https://example.test/team/repo.git"]);
    git(&worktree, &["config", "remote.origin.keepme", "untouched"]);
    let installation = GitInstallation::probe(git_executable()).expect("probe");
    let identity = locate(&installation, &worktree).expect("locate");
    (fixture, installation, identity)
}

fn config(identity: &srcmgr_git::RepositoryIdentity) -> String {
    fs::read_to_string(identity.common_dir.join("config")).expect("config")
}

#[test]
fn add_set_rename_and_remove_match_the_preview_and_preserve_other_keys() {
    let (_fixture, installation, identity) = repository("crud");

    let add = RemoteChange::Add {
        name: "backup".into(),
        url: "https://alice:s3cret@example.test/backup.git".into(),
        push_url: Some("ssh://git@example.test/backup.git".into()),
    };
    let preview = preview_remote_change(&installation, &identity, &add).expect("preview add");
    assert!(preview.lines.iter().any(|line| line.contains("https://alice@example.test/backup.git")));
    assert!(!preview.lines.iter().any(|line| line.contains("s3cret")));
    apply_remote_change(&installation, &identity, &add).expect("apply add");
    let text = config(&identity);
    assert!(text.contains("remote \"backup\"") || text.contains("[remote \"backup\"]"), "{text}");
    assert!(text.contains("https://alice:s3cret@example.test/backup.git"), "raw URL stays in config");
    assert!(text.contains("ssh://git@example.test/backup.git"));

    let set = RemoteChange::SetUrl {
        name: "origin".into(),
        url: "https://example.test/team/renamed.git".into(),
        push: false,
    };
    apply_remote_change(&installation, &identity, &set).expect("apply set-url");
    assert!(config(&identity).contains("https://example.test/team/renamed.git"));

    let rename = RemoteChange::Rename { from: "backup".into(), to: "mirror".into() };
    apply_remote_change(&installation, &identity, &rename).expect("apply rename");
    let text = config(&identity);
    assert!(text.contains("[remote \"mirror\"]"), "{text}");
    assert!(!text.contains("[remote \"backup\"]"), "{text}");
    assert!(text.contains("keepme"), "unrelated keys are preserved");

    let remove = RemoteChange::Remove { name: "mirror".into() };
    apply_remote_change(&installation, &identity, &remove).expect("apply remove");
    let text = config(&identity);
    assert!(!text.contains("[remote \"mirror\"]"), "{text}");
    assert!(text.contains("[remote \"origin\"]"));
    assert!(text.contains("keepme"), "removing one remote keeps other keys");
}

#[test]
fn invalid_input_is_refused_without_changing_the_configuration() {
    let (_fixture, installation, identity) = repository("invalid");
    let before = config(&identity);

    for change in [
        RemoteChange::Add { name: "-evil".into(), url: "https://example.test/x.git".into(), push_url: None },
        RemoteChange::Add { name: "http".into(), url: "http://example.test/x.git".into(), push_url: None },
        RemoteChange::Add { name: "ext".into(), url: "ext::sh -c id".into(), push_url: None },
        RemoteChange::SetUrl { name: "missing".into(), url: "https://example.test/x.git".into(), push: false },
        RemoteChange::Remove { name: "missing".into() },
    ] {
        assert!(
            preview_remote_change(&installation, &identity, &change).is_err(),
            "preview accepted {change:?}"
        );
        assert!(apply_remote_change(&installation, &identity, &change).is_err(), "apply accepted {change:?}");
    }
    assert_eq!(config(&identity), before, "a refused change must not write config");
}

#[test]
fn a_duplicate_name_is_refused() {
    let (_fixture, installation, identity) = repository("duplicate");
    let change = RemoteChange::Add {
        name: "origin".into(),
        url: "https://example.test/other.git".into(),
        push_url: None,
    };
    assert!(matches!(
        preview_remote_change(&installation, &identity, &change),
        Err(GitError::RemotePolicy(_))
    ));
}

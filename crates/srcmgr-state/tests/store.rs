//! Durable profile store: drafts survive a restart, a second writer is refused,
//! drafts are backend-scoped, and a newer schema is refused without changes.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use srcmgr_state::{ProfileStore, SCHEMA_VERSION, StoreError};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Profile {
    root: PathBuf,
}

impl Profile {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = env::temp_dir().join(format!("srcmgr-store-{}-{nanos}-{unique}-{name}", std::process::id()));
        Self { root }
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_draft_survives_a_restart_and_the_lock_is_released() {
    let profile = Profile::new("restart");
    {
        let mut store = ProfileStore::open(&profile.root).expect("first open");
        store.put_draft("git", "/repo/.git", "work in progress").expect("save draft");
    }
    let store = ProfileStore::open(&profile.root).expect("reopen after the first handle dropped");
    assert_eq!(
        store.get_draft("git", "/repo/.git").expect("read draft").as_deref(),
        Some("work in progress")
    );
}

#[test]
fn a_second_writer_is_refused_while_the_first_is_open() {
    let profile = Profile::new("lock");
    let _first = ProfileStore::open(&profile.root).expect("first open");
    match ProfileStore::open(&profile.root) {
        Err(StoreError::Locked) => {}
        other => panic!("expected Locked, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn drafts_are_scoped_to_their_backend() {
    let profile = Profile::new("scope");
    let mut store = ProfileStore::open(&profile.root).expect("open");
    store.put_draft("git", "/shared/path", "git text").expect("save");
    assert_eq!(store.get_draft("contract-fixture", "/shared/path").expect("read"), None);
    assert_eq!(
        store.get_draft("git", "/shared/path").expect("read").as_deref(),
        Some("git text")
    );
}

#[test]
fn saving_again_replaces_and_deleting_removes_the_draft() {
    let profile = Profile::new("replace");
    let mut store = ProfileStore::open(&profile.root).expect("open");
    store.put_draft("git", "/r", "one").expect("save");
    store.put_draft("git", "/r", "two").expect("replace");
    assert_eq!(store.get_draft("git", "/r").expect("read").as_deref(), Some("two"));
    store.delete_draft("git", "/r").expect("delete");
    assert_eq!(store.get_draft("git", "/r").expect("read"), None);
}

#[test]
fn settings_round_trip_across_a_restart() {
    let profile = Profile::new("settings");
    {
        let mut store = ProfileStore::open(&profile.root).expect("open");
        store.put_setting("theme", "dark").expect("save");
        store.put_setting("theme", "light").expect("replace");
    }
    let store = ProfileStore::open(&profile.root).expect("reopen");
    assert_eq!(store.get_setting("theme").expect("read").as_deref(), Some("light"));
    assert_eq!(store.get_setting("missing").expect("read"), None);
}

#[test]
fn a_newer_schema_is_refused_and_left_unchanged() {
    let profile = Profile::new("newer");
    fs::create_dir_all(&profile.root).expect("dir");
    {
        let connection = rusqlite::Connection::open(profile.root.join("profile.sqlite3")).expect("raw open");
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("set newer version");
    }
    match ProfileStore::open(&profile.root) {
        Err(StoreError::NewerSchema { found }) => assert_eq!(found, SCHEMA_VERSION + 1),
        other => panic!("expected NewerSchema, got {:?}", other.map(|_| ())),
    }
    let connection = rusqlite::Connection::open(profile.root.join("profile.sqlite3")).expect("raw reopen");
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read version");
    assert_eq!(version, SCHEMA_VERSION + 1);
}

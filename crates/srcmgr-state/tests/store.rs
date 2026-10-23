//! Durable profile store: drafts survive a restart, a second writer is refused,
//! drafts are backend-scoped, and a newer schema is refused without changes.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use srcmgr_state::{ProfileStore, SCHEMA_VERSION, StoreError};

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn git_preference_survives_restart_in_native_units_and_clear_preserves_other_state() {
    use srcmgr_domain::NativePath;
    let profile = Profile::new("git-preference-native");
    #[cfg(windows)]
    let path = {
        use std::os::windows::ffi::OsStringExt;
        NativePath::new(std::ffi::OsString::from_wide(&[67, 58, 92, 0xd800, 92, 103, 105, 116]))
    };
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        NativePath::new(std::ffi::OsString::from_vec(b"/missing/\xff/git".to_vec()))
    };
    {
        let mut store = ProfileStore::open(&profile.root).unwrap();
        store.set_git_executable(Some(&path)).unwrap();
        store.put_draft("git", "repo", "keep draft").unwrap();
        store.remember_repository("git", &profile.root.join("missing repo")).unwrap();
    }
    let mut store = ProfileStore::open(&profile.root).unwrap();
    assert_eq!(store.git_executable().unwrap(), Some(path));
    store.set_git_executable(None).unwrap();
    assert!(store.git_executable().unwrap().is_none());
    assert_eq!(store.get_draft("git", "repo").unwrap().as_deref(), Some("keep draft"));
    assert_eq!(store.recent_repositories().unwrap().len(), 1);
    assert!(!profile.root.join("missing repo").exists());
}

#[test]
fn git_preference_refuses_foreign_corrupt_and_oversized_records_without_reset() {
    let profile = Profile::new("git-preference-invalid");
    let mut store = ProfileStore::open(&profile.root).unwrap();
    #[cfg(windows)]
    let foreign = "unix-bytes\t2f676974";
    #[cfg(unix)]
    let foreign = "windows-utf16\t670069007400";
    for value in [foreign.to_owned(), "windows-utf16\tff".into(), "unknown\t00".into(), "unix-bytes\tzz".into(), "x".repeat(132_001)] {
        store.put_setting("git.executable.v1", &value).unwrap();
        assert!(matches!(store.git_executable(), Err(StoreError::InvalidMetadata)));
        assert_eq!(store.get_setting("git.executable.v1").unwrap().as_deref(), Some(value.as_str()));
    }
}

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
fn recents_survive_restart_reorder_and_forget_without_touching_files_or_drafts() {
    let profile = Profile::new("recents");
    let repo = profile.root.join("repo with space 中文");
    fs::create_dir_all(&repo).expect("repository fixture");
    fs::write(repo.join("keep.txt"), b"work in progress").expect("content");
    let absent = profile.root.join("removed-worktree");
    {
        let mut store = ProfileStore::open(&profile.root).expect("open");
        store.put_draft("git", "repo-id", "preserve draft").expect("draft");
        store.remember_repository("git", &repo).expect("first");
        store.remember_repository("git", &absent).expect("passive missing entry");
        store.remember_repository("git", &repo).expect("reopen reorders");
    }
    let mut store = ProfileStore::open(&profile.root).expect("reopen");
    let entries = store.recent_repositories().expect("recents");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].native_path().unwrap().as_os_str(), repo.as_os_str());
    assert_eq!(entries[1].native_path().unwrap().as_os_str(), absent.as_os_str());
    store.forget_repository(&entries[0]).expect("forget");
    assert_eq!(fs::read(repo.join("keep.txt")).unwrap(), b"work in progress");
    assert_eq!(store.get_draft("git", "repo-id").unwrap().as_deref(), Some("preserve draft"));
    assert_eq!(store.recent_repositories().unwrap(), vec![entries[1].clone()]);
}

#[test]
fn recents_are_bounded_backend_scoped_and_corrupt_records_are_not_reset() {
    let profile = Profile::new("recent-bounds");
    let mut store = ProfileStore::open(&profile.root).expect("open");
    for i in 0..srcmgr_state::RECENT_REPOSITORY_LIMIT + 2 {
        store.remember_repository("git", &profile.root.join(format!("repo-{i}"))).unwrap();
    }
    let entries = store.recent_repositories().unwrap();
    assert_eq!(entries.len(), srcmgr_state::RECENT_REPOSITORY_LIMIT);
    let latest = entries[0].native_path().unwrap();
    store.remember_repository("contract-fixture", std::path::Path::new(latest.as_os_str())).unwrap();
    let entries = store.recent_repositories().unwrap();
    assert_eq!(entries[0].backend, "contract-fixture");
    assert_eq!(entries[1].backend, "git");
    assert_eq!(entries[0].path, entries[1].path);
    let corrupt = "git\twindows-utf16\tfff\n";
    store.put_setting("workspace.recents.v1", corrupt).unwrap();
    assert!(matches!(store.remember_repository("git", &profile.root), Err(StoreError::InvalidMetadata)));
    assert_eq!(store.get_setting("workspace.recents.v1").unwrap().as_deref(), Some(corrupt));
    let oversized = "x".repeat(2 * 1024 * 1024 + 1);
    store.put_setting("workspace.recents.v1", &oversized).unwrap();
    assert!(matches!(store.recent_repositories(), Err(StoreError::InvalidMetadata)));
    assert_eq!(store.get_setting("workspace.recents.v1").unwrap().as_deref(), Some(oversized.as_str()));
}

#[cfg(windows)]
#[test]
fn recent_native_paths_keep_unpaired_utf16_and_foreign_paths_remain_unavailable() {
    use std::os::windows::ffi::OsStringExt;
    let profile = Profile::new("recent-native-path");
    let mut store = ProfileStore::open(&profile.root).unwrap();
    let path = PathBuf::from(std::ffi::OsString::from_wide(&[67, 58, 92, 0xD800, 46, 103, 105, 116]));
    store.remember_repository("git", &path).unwrap();
    let entries = store.recent_repositories().unwrap();
    assert_eq!(entries[0].native_path().unwrap().as_os_str(), path.as_os_str());
    store.put_setting("workspace.recents.v1", "git\tunix-bytes\t2fff\n").unwrap();
    assert!(store.recent_repositories().unwrap()[0].native_path().is_err());
}

#[cfg(unix)]
#[test]
fn recent_native_paths_keep_invalid_utf8_and_foreign_paths_remain_unavailable() {
    use std::os::unix::ffi::OsStringExt;
    let profile = Profile::new("recent-native-path");
    let mut store = ProfileStore::open(&profile.root).unwrap();
    let path = PathBuf::from(std::ffi::OsString::from_vec(b"/repo/\xff".to_vec()));
    store.remember_repository("git", &path).unwrap();
    assert_eq!(store.recent_repositories().unwrap()[0].native_path().unwrap().as_os_str(), path.as_os_str());
    store.put_setting("workspace.recents.v1", "git\twindows-utf16\t43003a005c00\n").unwrap();
    assert!(store.recent_repositories().unwrap()[0].native_path().is_err());
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

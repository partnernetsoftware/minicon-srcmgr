//! Durable profile state in SQLite (PRD 13): drafts and settings, with a
//! single-writer lock per profile directory. Every draft row is keyed by
//! backend and repository, so a record written by one backend cannot be read
//! through another (PRD 15, VCS-02).

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Transaction};

/// The schema this build reads and writes. A newer schema is refused rather
/// than guessed at (PRD 13: incompatible majors fail safely).
pub const SCHEMA_VERSION: i64 = 1;

const LOCK_FILE: &str = "profile.lock";
const DATABASE_FILE: &str = "profile.sqlite3";

#[derive(Debug)]
pub enum StoreError {
    /// Another process or handle holds this profile. A stale lock left by a
    /// crash is not removed automatically; the user removes it after checking.
    Locked,
    /// The database was written by a newer build.
    NewerSchema { found: i64 },
    Sql(rusqlite::Error),
    Io(io::Error),
    /// A bounded product metadata record is malformed or unsupported. Keep it
    /// intact for inspection instead of replacing it with an empty default.
    InvalidMetadata,
    CommitPending,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked => formatter.write_str(
                "this profile is in use by another instance; if none is running, remove profile.lock after checking",
            ),
            Self::NewerSchema { found } => write!(
                formatter,
                "the profile was written by a newer build (schema {found}, this build reads {SCHEMA_VERSION})"
            ),
            Self::Sql(_) => formatter.write_str("the profile database could not be read or written"),
            Self::Io(_) => formatter.write_str("the profile directory could not be prepared"),
            Self::InvalidMetadata => formatter.write_str("stored navigation or preference metadata is invalid, foreign to this platform, or exceeds supported limits; existing records were preserved"),
            Self::CommitPending => formatter.write_str("a previous commit attempt needs reconciliation; inspect repository state with Refresh before retrying"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

/// An exclusive writer for one profile directory. Creation is atomic
/// (`create_new`), and the file is removed when the holder drops it.
struct WriterLock {
    path: PathBuf,
}

impl WriterLock {
    fn acquire(path: PathBuf) -> Result<Self, StoreError> {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                writeln!(file, "{}", std::process::id()).map_err(StoreError::Io)?;
                file.sync_all().map_err(StoreError::Io)?;
                Ok(Self { path })
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(StoreError::Locked),
            Err(error) => Err(StoreError::Io(error)),
        }
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let mut bytes = Vec::new();
        if fs::File::open(&self.path).and_then(|file| file.take(64).read_to_end(&mut bytes)).is_ok()
            && bytes == format!("{}\n", std::process::id()).as_bytes() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The opened profile: one writer, one connection.
pub struct ProfileStore {
    connection: Connection,
    _lock: WriterLock,
}

impl ProfileStore {
    pub fn put_navigation(&mut self, repository: &srcmgr_domain::NativePath, value: &str) -> Result<(), StoreError> {
        if value.len() > 256 * 1024 { return Err(StoreError::InvalidMetadata); }
        self.put_setting(&navigation_key(repository)?, value)
    }

    pub fn navigation(&self, repository: &srcmgr_domain::NativePath) -> Result<Option<String>, StoreError> {
        self.get_setting_bounded(&navigation_key(repository)?, 256 * 1024)
    }

    /// Recovery review is separately bounded; normal navigation reads remain 256 KiB.
    pub fn navigation_for_repair(&self, repository: &srcmgr_domain::NativePath) -> Result<Option<String>, StoreError> {
        self.get_setting_bounded(&navigation_key(repository)?, 1024 * 1024)
    }

    /// Move exactly the reviewed record to an immutable recovery key. A stale
    /// review or archive collision rolls back; drafts and other settings stay put.
    pub fn archive_navigation(&mut self, repository: &srcmgr_domain::NativePath, expected: &str, nonce: &str) -> Result<String, StoreError> {
        if expected.len() > 1024 * 1024 || nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(StoreError::InvalidMetadata); }
        let key = navigation_key(repository)?;
        let archive = format!("git.navigation.archive.v1.{}.{}", key.strip_prefix("git.navigation.v1.").ok_or(StoreError::InvalidMetadata)?, nonce);
        let transaction = self.connection.transaction()?;
        let changed = transaction.execute("UPDATE settings SET key = ?1 WHERE key = ?2 AND value = ?3 COLLATE BINARY", (&archive, &key, expected))?;
        if changed != 1 { return Err(StoreError::InvalidMetadata); }
        transaction.commit()?;
        Ok(archive)
    }

    /// Persist the exact submitted draft and pending operation atomically,
    /// before launching Git. The profile writer remains held by the caller.
    pub fn begin_git_commit(&mut self, repository: &str, message: &str, before: Option<&str>) -> Result<(), StoreError> {
        if self.git_commit_intent(repository)?.is_some() { return Err(StoreError::CommitPending); }
        if before.is_some_and(|oid| !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit())) { return Err(StoreError::InvalidMetadata); }
        let key = commit_intent_key(repository);
        let value = format!("pending:{}", before.unwrap_or("unborn"));
        let transaction = self.connection.transaction()?;
        transaction.execute("INSERT INTO drafts (backend, repository, body) VALUES ('git', ?1, ?2) ON CONFLICT (backend, repository) DO UPDATE SET body = excluded.body", (repository, message))?;
        transaction.execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value", (&key, &value))?;
        transaction.commit()?;
        Ok(())
    }

    /// Some(None) means an interrupted attempt on an unborn HEAD.
    pub fn git_commit_intent(&self, repository: &str) -> Result<Option<Option<String>>, StoreError> {
        match self.get_setting_bounded(&commit_intent_key(repository), 80)? {
            None => Ok(None),
            Some(value) if value == "clear" => Ok(None),
            Some(value) => match value.strip_prefix("pending:") {
                Some("unborn") => Ok(Some(None)),
                Some(oid) if matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) => Ok(Some(Some(oid.into()))),
                _ => Err(StoreError::InvalidMetadata),
            },
        }
    }

    pub fn finish_git_commit(&mut self, repository: &str) -> Result<(), StoreError> {
        self.git_commit_intent(repository)?;
        self.put_setting(&commit_intent_key(repository), "clear")
    }
    /// Takes the profile's writer lock, opens the database in WAL mode with
    /// full synchronous commits, and migrates an empty or older schema.
    pub fn open(directory: &Path) -> Result<Self, StoreError> {
        fs::create_dir_all(directory).map_err(StoreError::Io)?;
        let lock = WriterLock::acquire(directory.join(LOCK_FILE))?;
        let mut connection = Connection::open(directory.join(DATABASE_FILE))?;
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;")?;
        migrate(&mut connection)?;
        Ok(Self {
            connection,
            _lock: lock,
        })
    }

    /// Stores (or replaces) the draft for one backend and repository.
    pub fn put_draft(&mut self, backend: &str, repository: &str, body: &str) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO drafts (backend, repository, body) VALUES (?1, ?2, ?3)
             ON CONFLICT (backend, repository) DO UPDATE SET body = excluded.body",
            (backend, repository, body),
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Reads the draft for one backend and repository, if one was saved.
    pub fn get_draft(&self, backend: &str, repository: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT body FROM drafts WHERE backend = ?1 AND repository = ?2",
                (backend, repository),
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Removes the draft after its change was committed or discarded.
    pub fn delete_draft(&mut self, backend: &str, repository: &str) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM drafts WHERE backend = ?1 AND repository = ?2",
            (backend, repository),
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Stores (or replaces) a per-profile setting.
    pub fn put_setting(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            (key, value),
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0))
            .optional()?)
    }

    pub(crate) fn delete_setting(&mut self, key: &str) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        transaction.commit()?;
        Ok(())
    }

    /// Check byte length inside SQLite before materializing bounded metadata.
    /// An oversized record is refused, never treated as a missing setting.
    pub(crate) fn get_setting_bounded(&self, key: &str, limit: usize) -> Result<Option<String>, StoreError> {
        let value: Option<Option<String>> = self.connection.query_row(
            "SELECT CASE WHEN length(CAST(value AS BLOB)) <= ?2 THEN value ELSE NULL END
             FROM settings WHERE key = ?1",
            (key, limit as u64),
            |row| row.get(0),
        ).optional()?;
        match value {
            Some(None) => Err(StoreError::InvalidMetadata),
            Some(Some(value)) => Ok(Some(value)),
            None => Ok(None),
        }
    }
}

fn commit_intent_key(repository: &str) -> String {
    repository_record_key("git.commit.intent.v1", repository)
}

fn navigation_key(repository: &srcmgr_domain::NativePath) -> Result<String, StoreError> {
    let encoded = repository.encoded();
    Ok(repository_record_key("git.navigation.v1", &format!("{}:{}", encoded.tag(), crate::recent::encode_path(&encoded)?)))
}

fn repository_record_key(prefix: &str, repository: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest: String = Sha256::digest(repository.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{prefix}.{digest}")
}

/// Creates the schema on an empty database and refuses a newer one. Each
/// step runs in one transaction, so an interrupted migration leaves the old
/// schema intact.
fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let found: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(StoreError::NewerSchema { found });
    }
    if found == SCHEMA_VERSION {
        return Ok(());
    }
    let transaction: Transaction<'_> = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS drafts (
             backend TEXT NOT NULL,
             repository TEXT NOT NULL,
             body TEXT NOT NULL,
             PRIMARY KEY (backend, repository)
         );
         CREATE TABLE IF NOT EXISTS settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    )?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

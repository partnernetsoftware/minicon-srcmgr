//! Durable profile state in SQLite (PRD 13): drafts and settings, with a
//! single-writer lock per profile directory. Every draft row is keyed by
//! backend and repository, so a record written by one backend cannot be read
//! through another (PRD 15, VCS-02).

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
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
        let _ = fs::remove_file(&self.path);
    }
}

/// The opened profile: one writer, one connection.
pub struct ProfileStore {
    connection: Connection,
    _lock: WriterLock,
}

impl ProfileStore {
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

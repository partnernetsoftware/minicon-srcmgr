//! Product-owned profile storage and trust grants for MiniCon SrcMgr
//! (PRD 11 trust, PRD 13 durable state).
//!
//! The SQLite store durably keeps drafts/settings and bounded recent entries;
//! trust grants use a separate atomic file. Only the profile directory is
//! written; the repository is never modified by this crate.

mod profile;
mod store;
mod trust;
mod recent;
mod preferences;

pub use profile::{default_root, write_atomic};
pub use store::{ProfileStore, SCHEMA_VERSION, StoreError};
pub use trust::{Grant, TrustError, TrustStore, fingerprint};
pub use recent::{RECENT_REPOSITORY_LIMIT, RecentRepository};

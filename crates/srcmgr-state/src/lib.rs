//! Product-owned profile storage and trust grants for MiniCon SrcMgr
//! (PRD 11 trust, PRD 13 durable state).
//!
//! The store is a small line-oriented file written atomically with owner-only
//! permissions on Unix. Only the profile directory is written; the repository
//! is never modified by this crate.

mod profile;
mod store;
mod trust;

pub use profile::{default_root, write_atomic};
pub use store::{ProfileStore, SCHEMA_VERSION, StoreError};
pub use trust::{Grant, TrustError, TrustStore, fingerprint};

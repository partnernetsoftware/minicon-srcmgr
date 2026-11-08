//! Directory durability, where the platform can express it.
//!
//! Linux and Windows accept a flush on a directory handle, so a failure there is
//! a real I/O error and the caller must stop. macOS rejects fsync on a directory
//! with EINVAL on APFS and HFS+, so the call cannot express directory durability
//! at all; treating that as fatal would make every stash removal fail on that
//! cell for a reason the platform, not the transaction, owns.
//!
//! The rename that published the entry has already succeeded at every call site
//! here. This module only decides whether the durability flush is binding.

use std::path::Path;

use crate::GitError;

/// Flushes a directory entry where the platform supports it.
///
/// Returns `Err` only when the platform can express directory durability and
/// the flush failed. On macOS the call is advisory by construction.
pub(crate) fn sync_directory(path: &Path) -> Result<(), GitError> {
    #[cfg(target_os = "macos")]
    {
        let _ = minicon_platform::filesystem::sync_parent(path);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        minicon_platform::filesystem::sync_parent(path).map_err(GitError::Io)
    }
}

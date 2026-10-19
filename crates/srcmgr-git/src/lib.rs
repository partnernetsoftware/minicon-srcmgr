//! Git adapter for MiniCon SrcMgr (PRD 02 Git adapter, PRD 15 Git backend).
//!
//! Git is the semantic engine: this crate runs the installed `git` as an
//! argument vector inside MiniCon's owned containment, reads machine formats
//! only, and reports what the product can do with it.
//!
//! Callers must reach [`repository::locate`] only after the repository's trust
//! decision (PRD 11). That decision is not implemented yet (M0-C), so the CLI
//! currently requires an explicit per-invocation approval flag.

mod error;
mod hunks;
mod installation;
mod repository;
mod runner;
mod status;

use srcmgr_domain::{BackendKind, Capability, CapabilityState, VcsBackend};

pub use error::GitError;
pub use installation::{GitInstallation, MINIMUM_GIT, parse_version};
pub use repository::{
    RepositoryIdentity, apply_hunk, commit, create_branch, diff, hunks, locate, read_status, stage,
    switch_branch, unstage,
};
pub use status::parse_status;

/// Capability routing for an installed Git.
#[derive(Clone, Debug)]
pub struct GitBackend {
    installation: GitInstallation,
}

impl GitBackend {
    #[must_use]
    pub const fn new(installation: GitInstallation) -> Self {
        Self { installation }
    }

    #[must_use]
    pub const fn installation(&self) -> &GitInstallation {
        &self.installation
    }
}

impl VcsBackend for GitBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Git
    }

    fn capability(&self, capability: Capability) -> CapabilityState {
        match capability {
            Capability::ReadStatus | Capability::ReadDiff | Capability::Stage | Capability::Commit => {
                match self.installation.require_supported() {
                    Ok(()) => CapabilityState::Available,
                    Err(_) => CapabilityState::Unavailable {
                        reason: "installed Git is older than the 2.43 product floor",
                    },
                }
            }
            Capability::Push => CapabilityState::Unsupported {
                reason: "remote publication is not implemented in the current slice",
            },
        }
    }
}

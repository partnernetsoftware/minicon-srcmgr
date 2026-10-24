//! Git adapter for MiniCon SrcMgr (PRD 02 Git adapter, PRD 15 Git backend).
//!
//! Git is the semantic engine: this crate runs the installed `git` as an
//! argument vector inside MiniCon's owned containment, reads machine formats
//! only, and reports what the product can do with it.
//!
//! Callers must reach [`repository::locate`] only after the repository's trust
//! decision (PRD 11), enforced by the application's durable trust boundary.
//! Initialization separately consumes an explicitly reviewed target, without
//! discovering or reinitializing an existing repository.

mod error;
mod hunks;
mod installation;
mod initialization;
mod identity_config;
mod ignore_edit;
pub use ignore_edit::{IgnoreEdit, ignore_effect};
mod commit_review;
mod index_summary;
mod repository;
mod runner;
mod status;
mod stash;
mod stash_removal;
mod commit_files;

use srcmgr_domain::{BackendKind, Capability, CapabilityState, VcsBackend};

pub use error::GitError;
pub use installation::{GitInstallation, MINIMUM_GIT, parse_version};
pub use initialization::InitializationTarget;
pub use identity_config::{IdentityConfig, IdentityValue};
pub use commit_review::{IndexBinding, StagedFile, StagedReview, ReviewedCommit, index_binding};
pub use index_summary::{IndexSummary, index_summary};
pub use repository::{reference_target, retain_recovery_ref};
pub use repository::ReviewedDiff;
pub use repository::{
    BranchSummary, CommitSummary, RepositoryIdentity, apply_hunk, apply_lines, checkout_detached, commit,
    commit_detail, create_branch, delete_branch, diff, discard_file, discard_hunk, hunks, ignore_match, local_branches,
    locate, log, read_status, stage, stage_all, stash_apply, stash_push, switch_branch, unstage, unstage_all,
};
pub use status::parse_status;
pub use stash::{StashEntry, check_stash_selection, stash_list, stash_selected, stash_diff};
pub use commit_files::{CommitFile, CommitFiles, commit_files, commit_file_diff};

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

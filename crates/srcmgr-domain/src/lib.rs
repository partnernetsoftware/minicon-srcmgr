//! Product-owned source-control contracts for MiniCon SrcMgr.
//!
//! This crate knows no VCS. Identities carry their backend tag, paths keep
//! their native units, capabilities say what a backend can actually do, and
//! plans bind to the backend, repository and generation they were made from.
//! Backend meaning lives in adapters such as `srcmgr-git` (PRD 02, PRD 15).

mod backend;
mod capability;
mod path;
mod plan;
mod snapshot;
mod tool;
pub use tool::{ExternalTool, ToolKind};

pub use backend::{BackendKind, RepositoryId, RevisionToken};
pub use capability::{Capability, CapabilityState, VcsBackend};
pub use path::{EncodedPath, NativePath, PathError};
pub use plan::{PlanBinding, PlanRejection};
pub use snapshot::{ChangeEntry, ChangeKind, EntryId, Generation, HeadState, StatusSnapshot};

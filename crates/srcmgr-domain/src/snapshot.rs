use crate::backend::{BackendKind, RepositoryId, RevisionToken};
use crate::path::NativePath;

/// Monotonic read generation. Results from an older generation are stale.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct Generation(pub u64);

impl Generation {
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// An opaque per-snapshot entry identifier. It names a row for the UI and
/// control output, never a path; mutations still bind the native path.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct EntryId(String);

impl EntryId {
    #[must_use]
    pub fn new(generation: Generation, index: usize) -> Self {
        Self(format!("g{}-{index}", generation.0))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where HEAD points.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HeadState {
    /// On a branch. `revision` is absent when the branch has no commits yet.
    Branch {
        name: String,
        revision: Option<RevisionToken>,
    },
    Detached {
        revision: RevisionToken,
    },
}

/// What happened to a path in one area (index or working tree).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Unmodified,
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    Ignored,
}

/// One changed path. Staged and unstaged halves are separate because the same
/// path can be changed in both areas at once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeEntry {
    pub id: EntryId,
    pub path: NativePath,
    pub original_path: Option<NativePath>,
    pub staged: ChangeKind,
    pub unstaged: ChangeKind,
}

/// A complete, generation-tagged working-copy status read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusSnapshot {
    pub backend: BackendKind,
    pub repository: RepositoryId,
    pub generation: Generation,
    pub head: HeadState,
    pub entries: Vec<ChangeEntry>,
}

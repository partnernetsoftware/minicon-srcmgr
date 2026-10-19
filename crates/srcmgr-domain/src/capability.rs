use crate::backend::BackendKind;

/// A product-level action a backend may or may not provide.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Capability {
    ReadStatus,
    ReadDiff,
    Stage,
    Commit,
    Push,
}

/// The actual state of one capability, with the reason a user can act on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityState {
    /// Available for use now.
    Available,
    /// The backend has no such operation, or it is not implemented yet.
    Unsupported { reason: &'static str },
    /// The backend supports it, but a prerequisite is missing here.
    Unavailable { reason: &'static str },
    /// Trust or policy forbids it for this repository until the user decides.
    Restricted { reason: &'static str },
}

impl CapabilityState {
    #[must_use]
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Capability dispatch surface every backend adapter provides.
///
/// Views ask for a capability instead of matching on the backend kind, so
/// Git-only controls never render for another backend (PRD 15).
pub trait VcsBackend {
    fn kind(&self) -> BackendKind;

    fn capability(&self, capability: Capability) -> CapabilityState;
}

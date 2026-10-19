/// The VCS adapter that produced an identity or snapshot.
///
/// Persisted records carry this tag so a record written by one backend can
/// never be interpreted by another (PRD 15, VCS-02).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum BackendKind {
    Git,
    /// Inert contract fixture for PRD 15. It has no operations.
    ContractFixture,
}

impl BackendKind {
    /// Stable identifier written to durable state and control output.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::ContractFixture => "contract-fixture",
        }
    }
}

/// An opaque, backend-scoped repository identity.
///
/// For Git this is the canonical common-directory path. Callers compare it for
/// equality only; it is never parsed or shown as a mutation target.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RepositoryId {
    backend: BackendKind,
    value: String,
}

impl RepositoryId {
    #[must_use]
    pub fn new(backend: BackendKind, value: impl Into<String>) -> Self {
        Self {
            backend,
            value: value.into(),
        }
    }

    #[must_use]
    pub const fn backend(&self) -> BackendKind {
        self.backend
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// A backend-tagged revision token. Its shape belongs to the backend: Git
/// object IDs are variable-length hex, so no fixed width or integer form is
/// assumed here.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RevisionToken {
    backend: BackendKind,
    token: String,
}

impl RevisionToken {
    #[must_use]
    pub fn new(backend: BackendKind, token: impl Into<String>) -> Self {
        Self {
            backend,
            token: token.into(),
        }
    }

    #[must_use]
    pub const fn backend(&self) -> BackendKind {
        self.backend
    }

    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }
}

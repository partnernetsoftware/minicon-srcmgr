//! PRD 15 contract fixture: an inert, non-Git backend exercises the shared
//! contracts without any Git semantics, proving that capability routing,
//! opaque revisions and plan rejection do not require a Git index or OID.

use srcmgr_domain::{
    BackendKind, Capability, CapabilityState, Generation, HeadState, PlanBinding, PlanRejection,
    RepositoryId, RevisionToken, StatusSnapshot, VcsBackend,
};

/// Describes a hypothetical backend that offers reads only. No staging, no
/// push, and a revision token whose shape is not an object hash.
struct ContractFixture;

impl VcsBackend for ContractFixture {
    fn kind(&self) -> BackendKind {
        BackendKind::ContractFixture
    }

    fn capability(&self, capability: Capability) -> CapabilityState {
        match capability {
            Capability::ReadStatus | Capability::ReadDiff => CapabilityState::Available,
            Capability::Stage | Capability::Commit => CapabilityState::Unsupported {
                reason: "backend has no staging area",
            },
            Capability::Push => CapabilityState::Unavailable {
                reason: "remote state is not reachable in this fixture",
            },
        }
    }
}

#[test]
fn capability_routing_reports_absent_operations_with_reasons() {
    let backend = ContractFixture;
    assert_eq!(backend.kind(), BackendKind::ContractFixture);
    assert!(backend.capability(Capability::ReadStatus).is_available());
    assert!(matches!(
        backend.capability(Capability::Stage),
        CapabilityState::Unsupported { .. }
    ));
    assert!(matches!(
        backend.capability(Capability::Push),
        CapabilityState::Unavailable { .. }
    ));
}

#[test]
fn snapshot_accepts_non_hex_revision_tokens_without_conversion() {
    let token = RevisionToken::new(BackendKind::ContractFixture, "rev:42/alpha");
    let snapshot = StatusSnapshot {
        backend: BackendKind::ContractFixture,
        repository: RepositoryId::new(BackendKind::ContractFixture, "fixture://one"),
        generation: Generation(9),
        head: HeadState::Detached { revision: token.clone() },
        entries: Vec::new(),
    };
    assert_eq!(snapshot.head, HeadState::Detached { revision: token });
    assert_eq!(snapshot.backend.id(), "contract-fixture");
}

#[test]
fn plan_made_for_git_is_rejected_against_a_fixture_with_the_same_path() {
    let planned = PlanBinding {
        backend: BackendKind::Git,
        repository: RepositoryId::new(BackendKind::Git, "/shared/path"),
        generation: Generation(3),
    };
    let current = PlanBinding {
        backend: BackendKind::ContractFixture,
        repository: RepositoryId::new(BackendKind::ContractFixture, "/shared/path"),
        generation: Generation(3),
    };
    assert_eq!(
        planned.verify(&current),
        Err(PlanRejection::BackendMismatch {
            planned: BackendKind::Git,
            current: BackendKind::ContractFixture,
        })
    );
}

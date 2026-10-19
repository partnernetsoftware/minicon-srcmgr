use crate::backend::{BackendKind, RepositoryId};
use crate::snapshot::Generation;

/// What a previewed operation was made against. Execution must present the
/// same binding again, or the plan is rejected without dispatch (PRD 15).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanBinding {
    pub backend: BackendKind,
    pub repository: RepositoryId,
    pub generation: Generation,
}

/// Why a plan no longer matches the current state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanRejection {
    BackendMismatch {
        planned: BackendKind,
        current: BackendKind,
    },
    RepositoryMismatch,
    StaleGeneration {
        planned: Generation,
        current: Generation,
    },
}

impl PlanBinding {
    /// Accepts only an exact match. A changed backend, repository or read
    /// generation invalidates the plan; nothing is reinterpreted.
    pub fn verify(&self, current: &Self) -> Result<(), PlanRejection> {
        if self.backend != current.backend {
            return Err(PlanRejection::BackendMismatch {
                planned: self.backend,
                current: current.backend,
            });
        }
        if self.repository != current.repository {
            return Err(PlanRejection::RepositoryMismatch);
        }
        if self.generation != current.generation {
            return Err(PlanRejection::StaleGeneration {
                planned: self.generation,
                current: current.generation,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(backend: BackendKind, repository: &str, generation: u64) -> PlanBinding {
        PlanBinding {
            backend,
            repository: RepositoryId::new(backend, repository),
            generation: Generation(generation),
        }
    }

    #[test]
    fn exact_binding_is_accepted() {
        let planned = binding(BackendKind::Git, "/r/.git", 3);
        assert_eq!(planned.verify(&planned.clone()), Ok(()));
    }

    #[test]
    fn stale_generation_is_rejected() {
        let planned = binding(BackendKind::Git, "/r/.git", 3);
        let current = binding(BackendKind::Git, "/r/.git", 4);
        assert_eq!(
            planned.verify(&current),
            Err(PlanRejection::StaleGeneration {
                planned: Generation(3),
                current: Generation(4),
            })
        );
    }

    #[test]
    fn same_path_under_another_backend_is_rejected() {
        let planned = binding(BackendKind::Git, "/r/.git", 3);
        let current = binding(BackendKind::ContractFixture, "/r/.git", 3);
        assert!(matches!(
            planned.verify(&current),
            Err(PlanRejection::BackendMismatch { .. })
        ));
    }
}

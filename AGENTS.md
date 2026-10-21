# Repository working agreement

Discuss work with the owner in Chinese. Keep code, identifiers, commits and this documentation series in English unless instructed otherwise.

Start at [PRD.md](PRD.md). Each material capability has exactly one owner in `prd/PRD_*.md`; update that owner when changing behavior. Keep README and this guide as pointers rather than duplicate specifications. Each PRD module carries a Markdown outcome tree, a Mermaid relationship diagram and delivery contracts naming invariants, evidence, safe failure, dependencies and non-goals.

Status: `[v]` completed with named evidence; `[-]` in progress; `[_]` not started. Missing required evidence is `BLOCKED`, never a skipped pass. A written requirement is not an implemented feature. Plans sequence work; they do not override requirements.

Reuse MiniCon platform mechanisms and keep native extensions behind the typed boundary defined in [architecture/reuse](prd/PRD_02_architecture_reuse.md); VCS meaning stays in this product. Follow that owner before adding dependencies or copying code. Never modify the sibling MiniCon checkout as a side effect of this project. Preserve existing user changes and stage explicit reviewed paths. Never publish private paths, credentials or machine identities.

The repository holds the PRD baseline, Rust foundation and the delivered Windows local GUI slice (`crates/`). Current version scope, qualification and blockers live in the linked plan under [PRD 14](prd/PRD_14_verification_roadmap.md#implementation-work-order-and-evidence). Do not claim build, runtime, signing or release success beyond recorded evidence. Future checks and evidence follow [verification](prd/PRD_14_verification_roadmap.md) and [delivery](prd/PRD_12_platform_delivery.md).

For implementation, follow [the work order](prd/PRD_14_verification_roadmap.md#implementation-work-order-and-evidence) and reconcile status with named evidence. Product behavior remains owned by the PRD series.

Choose verification by changed behavior, risk and dependency impact under [PRD 14](prd/PRD_14_verification_roadmap.md#risk-based-test-selection-and-stopping). State the minimum check and stopping condition first; reuse unaffected evidence and stop when the selected checks pass without unresolved issues. Do not default to full suites, six-cell matrices or long runs for every change.

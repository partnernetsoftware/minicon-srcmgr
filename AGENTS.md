# AGENTS.md

- 除源代码与注释外,其它场景都用中文
- 拒绝无聊的聊天，强制要求输出只能是：1, 决策型，2，工具使用型（可带简单注释）, 3, 结论型。不能有第4种形式（如自由文体不允许）, 如果需要思考CoT可以用bash的echo


# Repository working agreement

Discuss work with the owner in Chinese. Keep code, identifiers, commits and this documentation series in English unless instructed otherwise.

Start at [PRD.md](PRD.md). Each material capability has exactly one owner in `prd/PRD_*.md`; update that owner when changing behavior. Keep README and this guide as pointers rather than duplicate specifications. Each PRD module carries a Markdown outcome tree, a Mermaid relationship diagram and delivery contracts naming invariants, evidence, safe failure, dependencies and non-goals.

Status: `[v]` completed with named evidence; `[-]` in progress; `[_]` not started. Missing required evidence is `BLOCKED`, never a skipped pass. A written requirement is not an implemented feature. Plans sequence work; they do not override requirements.

Reuse MiniCon platform mechanisms and keep native extensions behind the typed boundary defined in [architecture/reuse](prd/PRD_02_architecture_reuse.md); VCS meaning stays in this product. Follow that owner before adding dependencies or copying code. Never modify the sibling MiniCon checkout as a side effect of this project. Preserve existing user changes and stage explicit reviewed paths. Never publish private paths, credentials or machine identities.

The repository holds the PRD baseline, Rust foundation and the delivered Windows local GUI slice (`crates/`). Current version scope, qualification and blockers live in the linked plan under [PRD 14](prd/PRD_14_verification_roadmap.md#implementation-work-order-and-evidence). Do not claim build, runtime, signing or release success beyond recorded evidence. Future checks and evidence follow [verification](prd/PRD_14_verification_roadmap.md) and [delivery](prd/PRD_12_platform_delivery.md).

For implementation, follow [the work order](prd/PRD_14_verification_roadmap.md#implementation-work-order-and-evidence) and reconcile status with named evidence. Product behavior remains owned by the PRD series.

Choose verification by changed behavior, risk and dependency impact under [PRD 14](prd/PRD_14_verification_roadmap.md#risk-based-test-selection-and-stopping). State the minimum check and stopping condition first; reuse unaffected evidence and stop when the selected checks pass without unresolved issues. Do not default to full suites, six-cell matrices or long runs for every change.

## Parallel work

Decompose a slice into independent paths before starting. Git adapter, UI/model, CLI wiring, fixtures, documentation and review usually touch different files and should proceed at the same time; default to parallel search, implementation and verification instead of one serial chain.

- Partition by file ownership, not by task name. Two workers editing the same file, or editing different files for one unresolved contract, is a conflict; a Git-layer worker and a UI worker are not.
- One integrator owns the final tree. Parallel workers return patches, findings or evidence; they do not merge into each other's files.
- Keep a short dependency graph in the active plan: what must land first, what runs beside it and the single merge point.
- Use subagents for independent review, failure classification, fixture construction and documentation consistency while implementation continues. Do not sit idle waiting for them.
- Serialize only what the machine forces: one cargo build/test at a time because they share the target directory; keep writing, searching and review parallel.
- Do not parallelize a public contract, a requirement-owner change or a version-scope decision. Those get one decision and one writer.

Use [continuous suite feedback](prd/PRD_14_verification_roadmap.md#continuous-suite-feedback) to drive the next task. Preserve and classify failures, retain useful defect regressions in the appropriate existing lane, and record actionable coverage or suite-cost gaps in the active plan. A later green run does not erase an unexplained failure or replace milestone evidence.

# Repository working agreement

Discuss work with the owner in Chinese. Keep code, identifiers, commits and this documentation series in English unless instructed otherwise.

Start at [PRD.md](PRD.md). Each material capability has exactly one owner in `prd/PRD_*.md`; update that owner when changing behavior. Keep README and this guide as pointers rather than duplicate specifications. Each PRD module carries a Markdown outcome tree, a Mermaid relationship diagram and delivery contracts naming invariants, evidence, safe failure, dependencies and non-goals.

Status: `[v]` completed with named evidence; `[-]` in progress; `[_]` not started. Missing required evidence is `BLOCKED`, never a skipped pass. A written requirement is not an implemented feature. Plans sequence work; they do not override requirements.

Reuse MiniCon platform mechanisms and keep native extensions behind the typed boundary defined in [architecture/reuse](prd/PRD_02_architecture_reuse.md); VCS meaning stays in this product. Follow that owner before adding dependencies or copying code. Never modify the sibling MiniCon checkout as a side effect of this project. Preserve existing user changes and stage explicit reviewed paths. Never publish private paths, credentials or machine identities.

This repository currently contains documentation only. Do not claim build, runtime, signing or release success. Future checks and evidence follow [verification](prd/PRD_14_verification_roadmap.md) and [delivery](prd/PRD_12_platform_delivery.md). Do not fabricate build commands before the implementation supplies them.

For implementation, follow [the handoff sequence](plan/implementation-handoff.md) and update this documentation-only status when evidence exists. Product behavior remains owned by the PRD series.

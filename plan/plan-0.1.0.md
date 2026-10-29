# 0.1.0 first-user beta plan

Status: `[-]` in progress. Workspace version remains **0.0.9**; no 0.1.0 tag or release exists. Product admission is owned by [01](../prd/PRD_01_product.md#010-first-user-feature-tree), packaging and signing by [12](../prd/PRD_12_platform_delivery.md#010-first-user-beta-delivery-target), interaction by [10](../prd/PRD_10_desktop_ux.md), and sequencing by [the roadmap](roadmap-0.1.0.md).

## Product outcome

**A developer obtains a Windows package, clones or opens a repository, commits, pulls and pushes without the Rust toolchain.**

## Delivered toward this target

| Area | State | Evidence |
|---|---|---|
| Clone, remote configuration, observation, fetch, upstream, FF pull, current-branch push | `[v]` | 0.0.7–0.0.9 public source checkpoints with Git and CLI fixtures |
| Window fetch/pull/push actions and status distance | `[v]` | 96 `srcmgr-ui` cases; scope in the 0.0.7/0.0.9 plans |
| Unsigned Windows x64 candidate | `[v]` private | `010-candidate-v1`: source `b609290`, artifact `ec245f2f174bd2701865aaca288a610c0c2ef16828e5fa980c6744b9c57c800b`, zero known build-root occurrences, fresh-profile startup exits 0 and reads `unset` |
| Window Clone and Remotes panels | `[_]` | Not implemented; CLI only |
| zh-CN primary strings and locale selection | `[_]` | English fallback keys exist |
| Signed package | `BLOCKED` | No signing authority or identity is available to this project |
| Clean-image launch, reference host, CJK/clipboard, accessibility | `BLOCKED` | No supplied environment |
| Five-participant study | `BLOCKED` | No participants or facilitated sessions |

## Remaining work packages

| Package | Deliverable | Minimum evidence and stop condition |
|---|---|---|
| P1 · Clone surface | Welcome Clone form bound to the existing `preview_clone`/`clone_repository` contracts, with destination review, progress and cancel | Workspace cases plus one native clone journey; a nonempty target keeps its contents |
| P2 · Remotes surface | Panel listing redacted remotes, with add/set-URL/rename/remove through the reviewed preview and an explicit confirmation | Model cases plus native captures; a refused edit changes no configuration |
| P3 · Language | zh-CN for primary flows using the typed key catalogue and a visible locale choice | Key coverage check; no untranslated primary control; English fallback intact |
| P4 · Package | Signed Windows x86_64 ZIP with notices, source/artifact identity, prerequisites and first-run instructions | Signature verification, scan, clean-image launch; otherwise `BLOCKED` |
| P5 · Study and quality | Five-participant task study plus changed-path resource evidence | Four of five complete open/clone → stage/commit → pull → push; failures change UX before release |

## Release gate

0.1.0 is publishable only when P4's signed package and P5's study pass under [12](../prd/PRD_12_platform_delivery.md). Until then the 0.0.9 source checkpoint remains the latest public release, and the private candidate is not published. An unsigned artifact is never described as the beta.

## Execution record

None yet beyond the private candidate receipt above.

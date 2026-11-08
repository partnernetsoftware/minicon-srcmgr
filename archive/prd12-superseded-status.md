# Superseded delivery status, 0.0.1 to 0.0.9

Archived 2026-11-08. These are the per-version status records that
[PRD 12](../prd/PRD_12_platform_delivery.md) carried while each version was
current. They are kept verbatim as release evidence: the tags, run ids,
digests and known limits below are the record of what those checkpoints
actually shipped, including what they did not ship.

Do not update this file. Current delivery status belongs to
[PRD 12](../prd/PRD_12_platform_delivery.md); execution evidence for a
version belongs to its plan under [plan/](../plan/), and the public notes
belong to [releases/](../releases/).

| Version | What it was | Where its evidence lives |
|---|---|---|
| 0.0.1 – 0.0.3 | Source-level releases with no binary | sections below; [PRD 14](../prd/PRD_14_verification_roadmap.md) |
| 0.0.4 | First discard/stash/detached-checkout shell | section below |
| 0.0.5 | Unpublished local-workflow checkpoint | [plan-0.0.5](../plan/plan-0.0.5.md) |
| 0.0.6 | Entry, tabs and full-index commit review; public source checkpoint | [plan-0.0.6](../plan/plan-0.0.6.md), [0.0.6 notes](../releases/0.0.6.md) |
| 0.0.7 | Remote observation and explicit fetch | [plan-0.0.7](../plan/plan-0.0.7.md), [0.0.7 notes](../releases/0.0.7.md) |
| 0.0.8 | Clone and reviewed remote configuration | [plan-0.0.8](../plan/plan-0.0.8.md), [0.0.8 notes](../releases/0.0.8.md) |
| 0.0.9 | Upstream, fast-forward pull and current-branch push | [plan-0.0.9](../plan/plan-0.0.9.md), [0.0.9 notes](../releases/0.0.9.md) |

## Release 0.0.1 status

Source-level first version, tagged locally as `v0.0.1` and not published. No binary is built for distribution, no signing or notarization is performed, and nothing is pushed to the public repository. The work order excludes binary publication, and the outcome-tree gates above require signed, cross-platform evidence before any binary release.

### What works on the Windows host

Verified with MinGit 2.56.0 through `SRCMGR_TEST_GIT`:

- Repository status with a trust gate: a metadata-only check runs before any Git command; grants cover the `.git` directory and a SHA-256 fingerprint of the config, attributes and hooks.
- Stage and unstage of literal paths, whole-hunk staging through `git apply --cached --recount`, and commit with the message passed on stdin.
- Ignore reporting through `git check-ignore -v`, with negated patterns treated as not ignored.
- Commit history from HEAD, paged with `--skip` and `--limit`, with a merge marker.
- Local branch list, create, and switch; the status window cycles branches with `B`.
- Commit-message drafts and an app setting `history.limit`, both in the SQLite profile store.
- A native status window built on MiniCon's platform host, font masks and pixel kernels.

### Known limits

- Git 2.43 is the release floor; the host's own Git 2.42 is refused by design.
- No graph lanes, per-parent diffs, scrollable diff view, branch picker, or window branch creation.
- Window interaction, layout, accessibility, IME, DPI and resource measurements are not evidenced.
- Linux and macOS cells are not run; invalid UTF-8 and newline file names need a Unix host.
- Crash injection and power-loss durability are not evidenced.
- Remote helpers, the full deny/grant matrix, and clean-machine or MSRV verification are open.

See [the implementation work order and evidence](../prd/PRD_14_verification_roadmap.md#implementation-work-order-and-evidence) for the evidence behind each item.

## Release 0.0.2 status

Source-level release, tagged locally as `v0.0.2` and not published: nothing is pushed, no GitHub Release is created, and no binary is built for distribution. The same limits as 0.0.1 apply: no signing or notarization, and no six-cell platform evidence.

### What changed since 0.0.1

- The status window is now a workspace with three list panels (Files, Branches, Commits) and a main panel for the diff, the commit detail or the commit composer. The key map follows lazygit's core flow, with the differences listed in [10](../prd/PRD_10_desktop_ux.md#release-002-shell-target-lazygit-level-minimum).
- From the window: stage and unstage files and hunks, check out and create branches, commit from a composer that keeps the saved draft, browse history and commit details, and scroll long diffs.
- Every Git call runs on a worker thread, so the window keeps handling keys while Git works.
- The `B` key from 0.0.1 is removed; the Branches panel does the switch.

### Verified on the Windows host

- `cargo test --workspace` with MinGit 2.56.0 through `SRCMGR_TEST_GIT`: 139 tests pass, including the backend job tests that drive the same trust gate, Git calls and profile store the window uses.
- `cargo clippy --workspace --all-targets`: zero warnings.
- `python scripts/check_docs.py`: PASS.
- A desktop capture of the window contents shows the three list panels, the diff and the footer hints.

### Known limits

- Key-to-redraw latency and resource use are not measured against the budgets in [14](../prd/PRD_14_verification_roadmap.md#performance-and-resource-budgets). The worker thread removes Git from the UI thread, but that is not yet evidence of responsiveness.
- The window's key handling is covered by unit tests of the state machine, not by sending keys to the live window: during verification, focus stayed with another application, so synthetic keys were not delivered.
- No IME composition beyond committed text, no screen-reader bridge, no line-level staging, no checkout of a commit, no graph lanes.
- Linux and macOS cells are not run.

See [the implementation work order and evidence](../prd/PRD_14_verification_roadmap.md#release-002-plan-lazygit-level-minimum) for the package-by-package record.

## Release 0.0.3 status

Source-level release, tagged `v0.0.3` and published to `origin` as a GitHub Release with source only. No binary is built or attached: the signing and six-cell evidence in the release gates above are not available, so a binary would not qualify. The limits of 0.0.1 and 0.0.2 still apply.

### What changed since 0.0.2

- The window follows lazygit's panel numbers (1 Status, 2 Files, 3 Branches, 4 Commits) and has a cursor in the main panel.
- Line-level and hunk staging from the diff, stage-all and unstage-all on Files, branch delete with a confirmation, and list motion keys (`<`/`>`, PgUp/PgDn, Ctrl+U/Ctrl+D).
- The window redraws when it is resized, and it responds to the mouse: clicks select rows and lines, and the wheel moves lists and the diff.

### Verified on the Windows host

- `cargo test --workspace` with MinGit 2.56.0 through `SRCMGR_TEST_GIT`: 154 tests pass.
- `cargo clippy --workspace --all-targets`: zero warnings. `python scripts/check_docs.py`: PASS.
- A live window run, driven by posted keys and clicks and by real keys in the foreground window: a click selects a branch; a line stage put only the added line in the index; `a` unstaged everything; the wheel scrolls the diff and End/Home move it; Ctrl+D and Ctrl+U move the view; `d` asks, `n` keeps the branch and `y` deletes it; a click on the Status row changes focus; a resize redraws the panels.

### Known limits

- Key-to-redraw latency and resource use are not measured against the budgets in [14](../prd/PRD_14_verification_roadmap.md#performance-and-resource-budgets).
- No IME composition beyond committed text, no screen-reader bridge, no checkout of a commit, no commit files, no search or filter, no discard, stash or rebase.
- Linux and macOS cells are not run.

## Release 0.0.4 status

Source-level release, tagged `v0.0.4` and published to `origin` as a GitHub Release with source only. No binary is built or attached: the signing and six-cell evidence in the release gates above are not available. The limits of 0.0.1 to 0.0.3 still apply.

### What changed since 0.0.3

- Discard: `d` in Files discards a file's unstaged changes, and `d` in the main panel discards the hunk under the cursor. Both ask first.
- Stash: `s` in Files saves tracked changes, and `S` applies the newest stash and keeps it.
- Detached checkout: `Space` in Commits checks a commit out detached, after a question.
- A refused Git command shows Git's first error line, for example why a checkout would overwrite local edits.

### Verified on the Windows host

- `cargo test --workspace` with MinGit 2.56.0 through `SRCMGR_TEST_GIT`: 164 tests pass.
- `cargo clippy --workspace --all-targets`: zero warnings. `python scripts/check_docs.py`: PASS.
- A live window run in the foreground: a hunk discard reverted only the first of two hunks; `s` stashed the change and `S` restored it; `d` then `n` kept the change and `d` then `y` restored the file; a commit checkout asked, and `y` left HEAD detached. A dirty-tree checkout was refused with Git's reason in the notice.

### Known limits

- Line discard, the stash list with pop, drop and messages, commit files, search and filter, reset and revert are not in 0.0.4. The plan and the benchmark against lazygit, reference GUI and Git SCM are in [10](../prd/PRD_10_desktop_ux.md#release-004-shell-target-discard-stash-and-detached-checkout).
- Key-to-redraw latency and resource use are not measured against the budgets in [14](../prd/PRD_14_verification_roadmap.md#performance-and-resource-budgets).
- No IME composition beyond committed text, no screen-reader bridge. Linux and macOS cells are not run.

## 0.0.5 source checkpoint

Historical workspace target: **0.0.5**, an unpublished source checkpoint with scoped qualification evidence; no `v0.0.5` public tag or GitHub Release has been published. Local optimized binaries exist solely for validation and are not distribution artifacts. At that checkpoint the latest public release was 0.0.4; current release status is recorded above. Its scope, scoped host evidence and remaining gates live in [the version plan](../plan/plan-0.0.5.md). Reference-environment response/resource certification remains open; implementation and a passing diagnostic do not waive that gate.

## Development 0.0.6 status

Workspace version: **0.0.6**, publicly released as a [development source checkpoint](https://github.com/partnernetsoftware/minicon-srcmgr/releases/tag/v0.0.6) on 2026-10-24 at 14:58:21 UTC. Annotated tag `v0.0.6` resolves to source `fc57042651a35add655141c0f5e04f1a2ca2a066`; the public release is neither draft nor prerelease and has no uploaded binary assets. The [execution plan](../plan/plan-0.0.6.md) records implemented entry/setup, coherent Changes/full-index composition, tabs/context, quick filters and external handoff, together with actual native/Git/UIA and available-host measurements. The measured refresh-navigation defect is repaired and its regression/negative control and native follow-up pass. No supported-platform, presentation-budget or signed-binary certification is claimed; qualified runnable delivery remains the separate 0.1.0 target.

The public tag's source ZIP was downloaded without authentication and all 124 files compared byte-for-byte against that exact source commit, including version 0.0.6 and release notes. SHA-256 is `8f63138b34820c44f74a3d43155940dac571f8cdbc84e155eac782480290f6d5` (587,352 bytes). No target outputs or native binaries occur in the archive. Final-source local-client CI `40104639266`, Windows/Linux ignore/runner CI `40104639293` and documentation CI `40104639260` pass; unchanged profile/accessibility/Windows build contracts retain their successful `31971ba` integration evidence. The final private candidate's build/source identity and native CLI startup pass, but it is not distributed. Earlier failed or incomplete qualification remains recorded rather than waived.

## Development 0.0.7 status

Status: source checkpoint delivered for remote observation and explicit fetch. It adds redacted remote listing with configured refspecs, upstream distance, one-remote fetch with per-ref results, cancellation, filtered progress, the `f` window action and `remote`/`fetch` CLI commands. Clone and remote configuration writes are 0.0.8; pull, push and authentication qualification are 0.0.9. No executable is distributed: authenticated transports are BLOCKED without a controlled fixture host, the window has no dedicated Remotes surface or live progress, and no native remote journey, reference-host, signing or platform certification is claimed. Scoped evidence is in [the 0.0.7 plan](../plan/plan-0.0.7.md).

## Development 0.0.8 status

Status: source checkpoint delivered for clone and reviewed remote configuration. Clone validates transport, destination, branch and depth before transfer, disables checkout until review, refuses a nonempty target without touching it and retains a partial target. Remote add/set-URL/rename/remove preview the local effect, apply only with an explicit `--yes`, preserve unrelated keys and report the trust-fingerprint change. Both are delivered through the Git layer and CLI; the window has no Clone form or Remotes surface yet, so window surfaces, pull and push remain 0.0.9. No executable is distributed, authenticated transports are BLOCKED without a fixture host, and no native, reference-host or signing certification is claimed. Scoped evidence is in [the 0.0.8 plan](../plan/plan-0.0.8.md).

## Development 0.0.9 status

Status: source checkpoint delivered for explicit upstream, fetch-then-fast-forward pull and current-branch push. Pull refuses divergence instead of merging, push sends one explicit refspec and reports per-ref Updated/UpToDate/Rejected, and a cancelled or timed-out push is reconciled once with `ls-remote` and never retried. Window `p`/`P` report the final outcome. Clone/Remotes panels, authenticated HTTPS/SSH qualification, a killed-transfer receipt, JSON for the new CLI verbs, native journeys and signing remain open. No executable is distributed and no platform certification is claimed. Scoped evidence is in [the 0.0.9 plan](../plan/plan-0.0.9.md).

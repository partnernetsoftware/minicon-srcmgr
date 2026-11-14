# Requirements memory palace

A distilled map of the requirements baseline for a reader who needs to act
correctly without re-reading fifteen modules. It answers four questions fast:
what the product is, how this baseline talks, who owns a fact, and which
decisions have already been made and must not be re-litigated.

**This file carries no authority and no evidence.** Every statement here is a
compression of an owning module. When this file and an owner disagree, the
owner wins; when this file says "delivered", only the plan or release record can
prove it. Nothing here qualifies a milestone, a platform cell or a release.

## The product in one sentence

A small native GUI that lets a developer understand and change Git repositories
confidently, offline, without an account, on Windows, macOS and Linux with the
same operation meanings.

The promise has three pillars that must hold **together**: small memory
footprint, immediate interaction, dependable source control. Functionality that
passes tests but regresses a pillar is not releasable. The admitted wedge is the
low-specification/restricted-environment native client; hosting workflows,
feature parity and multi-repository cockpits are not the wedge. Owner:
[01](PRD_01_product.md).

## How this baseline talks

| Notation | Meaning | Trap |
|---|---|---|
| `0.1.0`, `0.1.2`, `0.1.3` | Application versions and their published artifacts | A plan may describe a version long before it exists |
| `M0`–`M3` | Cumulative delivery milestones, defined only in 01 | They are **not** version numbers; do not convert one into the other |
| `[v]` / `[-]` / `[_]` | Delivered and verified / work exists but incomplete / future allocation | `[-]` never means "nearly done"; it means the named checks do not all pass |
| `PASS` / `FAIL` / `BLOCKED` / `NOT-RUN` | Verdicts on one named, scoped piece of evidence | Passing one slice never qualifies a journey, milestone or platform matrix |
| `PROD-`, `ARCH-`, `REPO-`, `CHANGE-`, `HIST-`, `REF-`, `REMOTE-`, `REC-`, `ADV-`, `UX-`, `SEC-`, `SHIP-`, `STATE-`, `QA-`, `VCS-` | Requirement namespaces, one owner each | An identifier appearing in two modules is a defect |
| `EARS`-style "shall" clauses in owner tables | Intended behavior, not completion | Documentation completeness is not runtime evidence |

Every requirement row has the same contract shape: **invariant · observable
evidence · safe failure · dependencies · explicit non-goal**. A row without a
safe failure or a non-goal is incomplete, not permissive. Owner:
[14](PRD_14_verification_roadmap.md).

## The ten governing invariants

1. The repository, index and refs are authoritative; cached UI state never
   overrides Git state.
2. Every mutation binds a repository identity, explicit targets and **fresh**
   preconditions; a stale preview cannot authorize a changed operation.
3. No automatic destructive cleanup, stash, history rewrite, push or hook
   bypass. Recovery states exactly what can and cannot be restored.
4. Shared MiniCon mechanisms are consumed at immutable revisions; product state
   and Git semantics live in this repository.
5. Cross-architecture means matching native artifacts and independently
   recorded build/runtime evidence. Compilation proves no GUI behavior.
6. No mandatory service account, telemetry or network for local work.
   Repository-controlled code needs explicit trust.
7. One fact has one owning PRD. Links express dependency; acceptance evidence
   cannot be inferred from documentation.
8. Git is the first backend; identities, storage and actions preserve a
   capability boundary for SVN and others without inventing Git semantics.
9. Simple, practical, intuitive workflows and low resource use are release
   criteria.
10. The three pillars qualify together under 14 before an affected milestone can
    pass.

## Who owns what

| Branch | The question it answers | IDs |
|---|---|---|
| [01 Product](PRD_01_product.md) | Who is this for, what is admitted, when is it done | PROD |
| [02 Architecture](PRD_02_architecture_reuse.md) | Which MiniCon mechanism supplies which capability, at what revision | ARCH |
| [03 Workspace](PRD_03_workspace.md) | How repositories are discovered, cloned, organized and observed | REPO |
| [04 Changes](PRD_04_changes_commits.md) | How content is inspected, staged and committed | CHANGE |
| [05 History](PRD_05_history.md) | How graphs, ranges, file history and blame are read | HIST |
| [06 References](PRD_06_references.md) | How branches, tags, upstream and integration behave | REF |
| [07 Remotes](PRD_07_remotes.md) | Which remote receives which ref, and what actually happened | REMOTE |
| [08 Recovery](PRD_08_recovery.md) | What an unfinished or destructive operation can restore | REC |
| [09 Advanced](PRD_09_advanced_git.md) | Stash, worktrees, submodules, LFS, patches | ADV |
| [10 Desktop UX](PRD_10_desktop_ux.md) | Surfaces, keys, focus, accessibility, wording | UX |
| [11 Security](PRD_11_security.md) | Credentials, executable content, privacy | SEC |
| [12 Delivery](PRD_12_platform_delivery.md) | Cells, packages, signing, release status | SHIP |
| [13 State](PRD_13_state_control.md) | Persistence, command model, automation | STATE |
| [14 Verification](PRD_14_verification_roadmap.md) | Traceability, budgets, gates, implementation order | QA |
| [15 VCS](PRD_15_vcs_backends.md) | Where Git ends and another backend would begin | VCS |

## Decisions already made — do not re-propose them

These are the proposals that keep coming back in disguise. Each is a recorded
decision, not an omission.

| Proposal | Disposition | Why |
|---|---|---|
| Bypass commit hooks / add `--no-verify` | Not offered | Hooks run under trust; a bypass needs its own trust policy first |
| Custom actions or user scripts | Excluded | Arbitrary plugin or script execution is outside the baseline |
| Auto-install Git during onboarding | Excluded | A missing Git shows setup guidance instead |
| Hosted accounts, PRs, hosted project browsing | Excluded | An account is not the workspace identity |
| Built-in secret store / saved passwords | Not stored | No vault in this baseline |
| Ignore by extension as the default | Exact path is the default | A broad wildcard hides files the user did not choose |
| Stage-all or commit-all in one step | Not copied | A commit submits the live index only |
| One-step sync (pull then push) | Not copied | Every remote write has its own preview |
| Ambient defaults for pull mode, push refspec, auto-stash | Not copied | A silent default can rewrite history or stash work |
| Universal undo | Not promised | Only recovery refs, safety copies and reflog-reachable commits restore |
| Silent commit-message rewrite | Not copied | The draft is user-owned |
| Periodic remote polling / background auto-fetch | Not copied | Auto-fetch is opt-in per repository |
| Discard without a safety copy or recovery label | Not copied | Every destructive action states what survives |
| Git Flow, subtree, graft, PuTTY as a second transport | Not admitted | Each needs an owner and its own write semantics first |

Owner: [01](PRD_01_product.md) for product boundary, [08](PRD_08_recovery.md) for
recovery, [11](PRD_11_security.md) for secrets, [07](PRD_07_remotes.md) for
transport.

## Where the versions stand

Use the linked record as truth; this table only tells you where to look.

| Version | State | Look at |
|---|---|---|
| 0.1.0 | Published 2026-11-07: signed, runnable Windows x86_64 beta; its notes name six gaps | [notes](../releases/0.1.0.md) |
| 0.1.2 | Published 2026-11-09: all six cells build/run/package; both Windows cells Authenticode-signed, `release_eligible=true`; macOS and Linux unsigned | [notes](../releases/0.1.2.md), [PRD 12](PRD_12_platform_delivery.md#support-matrix) |
| 0.1.3 | Code complete against its five delivered criteria: Clone, Remotes, language, divergence and conflict handoff are `[v]`. The clean-image court and the five-participant study are `[_]` and BLOCKED on a clean machine and real users, so the version is not published | [plan](../plan/plan-0.1.3.md) |

Two items are **BLOCKED by missing external conditions**, not by code: the
clean-image court needs a machine that never had the product or the Rust
toolchain, and the five-participant study needs real people. Both are release
gates; neither can be satisfied by writing code, and neither may be reported as
passed.

## Facts that had to be re-learned

Operational knowledge that cost a debugging cycle at least once. Each is
enforced in code or a fixture today.

- **Pull never integrates.** It is fast-forward only, and refuses with both tips
  named when the branch diverged. It never merges, rebases or stashes on its own.
- **Clone is always `--no-checkout`.** The working tree appears only after the
  repository is trusted and a separate checkout review approves it, so a rejected
  filter leaves no files behind.
- **`--no-checkout` leaves the index empty, not populated.** A checkout must read
  HEAD's tree (`read-tree HEAD`, then `checkout-index -a` without `-f`); writing
  from the index would write nothing. "Worktree empty except `.git`" is the cheap
  signal that a checkout is pending.
- **The write guard reads Git's own metadata**, never a filename: `MERGE_HEAD`,
  `rebase-merge`/`rebase-apply`, `CHERRY_PICK_HEAD`, `REVERT_HEAD` plus
  `diff --diff-filter=U`. A marker that exists but cannot be read is *unknown*,
  not clean.
- **A preview is not a permission.** Approval sends a one-use ticket bound to the
  values the review showed, so a field edited after review cannot execute.
- **A failed clone keeps its target, and only the backend names it.** Inspection
  and removal take no path from the interface: the backend holds the recorded
  destination, and removal is offered only when the destination did not exist
  before the attempt. Anything else is refused, because deleting a directory the
  product did not create is not a recovery.
- **`FETCH_HEAD`'s modification time is the only evidence of when the counts were
  taken.** Show it, or say the time is unavailable; never present ahead/behind as
  live server truth.
- **A surface that is drawn but not routed is inert.** `key_inner` must hand keys
  to every open surface; the clone form was rendered and titled for a whole round
  while no keystroke reached it, and only a workspace-level key test caught it.
  Test keys through `Workspace::key`, not only through the form's own methods.
- **Editing repository configuration invalidates trust.** The trust fingerprint
  covers `.git/config`, so any remote or identity write makes the loaded session
  stale; the window must ask for a fresh trust review instead of refreshing.
- **Fetch is not a write; every other remote action is.** Fetch contacts the
  server by definition and skips the review. Add, Set URL, Rename and Remove must
  state that no server is touched, and the preview carries that sentence for
  every one of them, not only for the destructive one.
- **Locale changes display text only.** Paths, object ids, refspecs, ref names and
  Git's own output stay literal in every language.
- **Hints are translated too, but key names stay literal.** A footer line is a
  template of Latin key names plus translated verbs; translating "Space" or "Esc"
  would hide the key the user must press.
- **The catalogue is one table, so a key without a translation cannot compile.**
  Templates keep their placeholders and substitute at the call site; do not build
  translated sentences by concatenation.
- **Backend sentences come from the catalogue too, through one helper.** Every
  refusal the product composes itself is a template plus data; a caller without
  a self parameter uses the free helper, and both fall back to English rather
  than losing the message. Git's own error detail stays literal in both
  languages, because Git printed it.
- **Refusal text belongs to the window, not to `srcmgr-git`.** A blocked write and
  a diverged pull are composed in the catalogue from Git's own state fields, so
  the Git crate keeps no locale. Older backend messages that still travel as
  strings are named in the plan rather than assumed translated.
- **Windows canonical paths break Git.** `fs::canonicalize` returns a verbatim
  `\\?\` path that Git rejects with status 128; resolve a symlinked temp root on
  unix only.
- **`fsync` on a directory is EINVAL on macOS.** The flush binding stays on Linux
  and Windows and is advisory on macOS; the rename has already published.
- **Fixtures need a real Git.** Point `SRCMGR_TEST_GIT` at an installed
  executable; the product never bundles or installs one.
- **Six cells prove three things only**: the archive was built on that cell, the
  executable starts there, and it performs real Git reads on a fixture. No window
  was opened on five of them, and no cell is signed except Windows.
- **Release assets come from two runs.** The six-cell job finishes before signing
  exists, so the Windows archives are repackaged from the signed executables and
  the digests are recomputed on the bytes that ship.
- **Run identifiers exceed Int32.** Anything converting `GITHUB_RUN_ID` must read
  it as Int64 or it throws before the first file is signed.

## Four reading paths

1. **Implement a mutation** — this file, then the owning module, then
   [08](PRD_08_recovery.md)/[11](PRD_11_security.md)/[13](PRD_13_state_control.md),
   then the plan's acceptance ledger.
2. **Answer "can the product do X"** — find X in the ownership table, read that
   module's requirement rows and non-goals. Do not answer from this file.
3. **Check delivery status** — [PRD 12](PRD_12_platform_delivery.md) for release
   status, the active plan for the ledger, the release notes for what a published
   artifact does and does not prove.
4. **Start a new increment** — [01](PRD_01_product.md) for admission,
   [14](PRD_14_verification_roadmap.md#implementation-work-order-and-evidence)
   for sequence and evidence, then write the plan before the code.

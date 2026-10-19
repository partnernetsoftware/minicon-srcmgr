# Implementation handoff · requirements baseline 1.2

Status: `[-]` M0-A foundation and the M0-B Git runner/status slice are implemented; M0-C, M0-D and M1 are not started (see evidence below). Start with [AGENTS.md](../AGENTS.md) and [PRD.md](../PRD.md). This plan sequences work; the linked PRD owners define behavior, scope and acceptance. The completed review disposition lives in [14](../prd/PRD_14_verification_roadmap.md).

## First assignment

Implement the M0 foundation, then demonstrate the smallest host-native M1 local Git slice. Use disposable fixture repositories and an isolated profile. Preserve the sibling MiniCon checkout and user repositories. Keep work reviewable in sequential commits; do not begin the entire M3 feature set at once. No binary publication is part of this assignment.

Read 01 → 02 → 15 → 11 → 08 → 13 → 10 → 14 before introducing production interfaces. For the local slice also read 03/04/05/06. Use these owners for decisions rather than deriving behavior from this plan.

## Ordered work packages

| Work package | Concrete deliverable | Exit evidence / dependency |
|---|---|---|
| M0-A · reproducible foundation | Rust workspace, pinned toolchain/dependencies/lockfile, consumer-root fork policy, actual-feature manifest and real build/test drivers | ARCH-01/04; isolated clean consumer and host build; configure six-cell build lanes without claiming unavailable runtimes |
| M0-B · domain and Git runner | Backend-tagged IDs/lossless paths/capabilities, snapshot/plan/error contracts; typed supervised Git execution with bounded pipes and environment | ARCH-02/03, VCS-01/02/03; domain non-Git fixture, actual Git parser fixtures, cancellation/reap and no shell effects |
| M0-C · trust and durable state | Metadata-only pre-trust entry, grant/revoke/config checks, minimal SQLite schema, durable draft/journal acknowledgements and plan rejection | SEC-01/02/03, STATE-01/02; hostile pre-trust no-execution, I/O failure/crash and consumed/stale plan fixtures |
| M0-D · native shell and access | Real MiniCon-backed window, compact layout/theme/input/IME/clipboard, semantic editable field and action, retained drawing and resource instrumentation | UX-02/03/04, QA-04; host native interaction and resource receipt, per-OS-family accessibility prototype receipts as courts become available |
| M1-A · first visible local slice | Open/trust one repository → status list → selected diff → whole-file stage/unstage → message draft → ordinary commit → refreshed history/HEAD | Relevant QA-J01/J05/J08/J09 subcases; real before/after Git trees/index/worktree, persisted draft and no hidden mutation |
| M1-B · complete local alpha | Extend to hunk staging, paged graph, branch create/switch, scoped settings/ignore and all remaining M1 clauses | Cumulative M1 scope from 01 plus applicable resource/safety/accessibility subcases from 14 |

M0 packages establish contracts together; the order is a dependency sequence rather than permission to skip safety while building the UI. A visual scaffold may precede completed mechanisms, but it must not mutate repositories or be called an implemented workflow. Host-native M1 work can progress when its prerequisites are proven even if other target receipts remain BLOCKED; milestone completion still follows 01/12/14.

## Initial environment observation

Read-only review on 2026-10-18 observed Rust 1.97.0 and a default Git 2.42.0.windows.2. Recheck at implementation start. The latter is below the product floor owned by [02](../prd/PRD_02_architecture_reuse.md): select an independently verified supported Git for product fixtures before claiming adapter qualification. Existing Git can still maintain this documentation repository; do not globally replace it or lower the product floor just to obtain a green run. Record executable identity/version privately in local receipts and redact exported paths.

The initial MiniCon revision is owned by 02, regardless of newer upstream/sibling changes. If it cannot satisfy an essential consumer capability, record a concrete decision with a reviewed published replacement revision and actual feature/fork checks; never use an uncommitted sibling path. The pinned accessibility publication API has known non-Linux gaps, so inspect/resolve this early rather than treating a successful constructor as support.

## Evidence recorded so far

- Build and test (host Windows, rustc 1.97.0 pinned by `rust-toolchain.toml`): `cargo build --workspace`, `cargo clippy --workspace --all-targets` clean, `cargo test --workspace` 31 tests passing against MinGit 2.56.0.windows.2 through `SRCMGR_TEST_GIT`. The test fixtures are disposable repositories; the release-blocking floor remains Git 2.43.
- MiniCon dependency: `minicon-core` and `minicon-platform` (feature `contained-process-spawn` only) at commit `5aa3a8a8893b311d07b503f4b0a45e896727f3b9`. The sibling checkout was not modified.
- Host Git on this machine is 2.42.0.windows.2, below the 2.43 floor. `minicon-srcmgr status` refuses it with the floor message; the fixture suite needs an explicitly supplied supported Git.
- M0-C (partial): `srcmgr-state` stores trust grants in an atomically written, owner-only profile file. Each grant covers the canonical `.git` directory plus a SHA-256 fingerprint of the config, `info/attributes`, the worktree `.gitattributes` and `hooks/`. The fingerprint is computed from metadata before any Git command runs. A config with `[include]` cannot be granted. Dependency added for this: `sha2` 0.11 (MiniCon's checksums are not collision-resistant). Still missing: the QA-J10 hostile-fixture suite, persisted SQLite state, and the single-writer lock.
- M1-A (CLI slice, not GUI): `minicon-srcmgr trust grant|revoke`, `status`, `stage`, `unstage` and `commit -m`. Stage and unstage accept only literal paths that the status read reports as changed. The commit message is passed on stdin, never in argv. Covered by 9 end-to-end tests in `crates/srcmgr-cli/tests/slice.rs`, including the invalidation cases: a changed config, a hook edit, an `include` directive and revocation.
- Test totals with MinGit 2.56.0 (`SRCMGR_TEST_GIT`): `cargo test --workspace` 43 passing, `cargo clippy --workspace --all-targets` zero warnings.
- Not yet evidenced: the QA-J11 injection and encoding corpus (non-UTF-8 Unix paths were not run on this Windows host), a clean-clone consumer build (ARCH-01/QA-J12), persisted SQLite state and draft recovery (STATE-*), hunk staging, branches, the GUI (M0-D), and the M1 graph and diff views.
- The `--approve-git-execution` flag from the first slice has been removed. The durable grant replaces it.
- M0-D (first window, Windows only): `minicon-srcmgr gui <worktree>` opens a native window through MiniCon's `window_host::run_pixel_window`, using `minicon-platform` font masks and `minicon-ui-core` blend kernels. It passes the same trust gate as `status`. Visual evidence: desktop capture of the window over the demo repository showing the branch line, a modified row and an untracked row in the expected tones. The capture is kept in the local scratchpad, not the repository. `srcmgr-ui` has 3 view-model unit tests.
- Still open for M0-D: keyboard and pointer interaction, layout and scrolling, accessibility on each OS family, IME, DPI beyond one scale check, resource measurements, and the Linux and macOS cells.
- M1 selected-file diff (first slice): `diff <worktree> <path> [--staged]` runs `git diff` with external diff and textconv disabled. The status window selects rows with the Up and Down keys and shows the selected entry's diff below the list, with hunk, added and removed lines toned differently. Evidence: a desktop capture after Down and Up, showing an untracked row and a modified row with its hunk. Covered by an end-to-end CLI test and 5 view-model tests; the diff view shows at most 200 entries and does not scroll yet.
- M1 branch create and switch (CLI only): `branch <worktree> create|switch <name>`. Names go through `git check-ref-format --branch` first, and a leading `-` is refused, so an option-like name never reaches `switch`. Git's own refusal of a switch over local changes is returned unchanged. Covered by two end-to-end tests; the window does not yet offer these actions.
- QA-J01 subset for the narrowed M1 local closure (`crates/srcmgr-cli/tests/journey_j01.rs`, 5 tests passing against MinGit 2.56.0): first commit on an unborn branch with the index blob checked by `hash-object`; mixed staged, unstaged and untracked states checked against `ls-files --stage`; unstage restoring the HEAD blob in the index while keeping the worktree bytes; a dirty switch that would overwrite local edits is refused, leaving HEAD and the file unchanged; spaces, non-ASCII names and a leading dash round-trip through stage and commit, with `./`-prefixed input refused rather than expanded.
- QA-J01 subcases NOT covered by this closure: hunk and line staging, scoped config and escaped ignore rules, and the graph view. These remain open.
- QA-J10 subset (`crates/srcmgr-cli/tests/hostile_j10.rs`, 4 tests): a repository `core.fsmonitor` command does not run before trust (marker file absent); under a grant the same command does run, which proves the probe can detect execution; a config change after the grant stops execution again; a revoked grant stops execution. Not covered: external helpers, `include` interaction at runtime, and the full deny/grant matrix. QA-J10 remains open.
- QA-J10 hooks and filters subset (`crates/srcmgr-cli/tests/hostile_hooks_filters.rs`, 5 tests): an untrusted commit does not run a pre-commit hook; under a grant the hook runs (positive control); a hook edited after the grant is refused and does not run; an untrusted stage does not run a `filter.<name>.clean` command; under a grant the clean filter runs (positive control).
- Still open for M1: a scrolling diff view, graph view, hunk staging, window controls for branches, and persisted settings.

## Evidence and blocker handling

- Introduce executable build/test commands with the implementation and link their actual owners; do not advertise commands that do not exist. Keep `python scripts/check_docs.py` passing throughout.
- Record requirement and journey subcase IDs, source/artifact/feature identity, environment and expected versus observed Git state. Update owner/frontier status only with named evidence.
- Missing target court/toolchain/signing capability is BLOCKED for that gate. Continue independent work, preserve the target and list the precise missing prerequisite. Never substitute emulation or a skipped test as native PASS.
- An upstream mechanism gap uses the local typed extension route defined in 02 after recording the contract and failing consumer evidence, or is explicitly BLOCKED if infeasible. This task does not authorize modifying sibling repositories or weakening trust, recovery or accessibility requirements.
- The first handback contains runnable host commands, the demonstrated local journey, scoped receipt links, resource measurements and unresolved blockers. It must distinguish implemented functionality, prototypes and specification-only later scope.

## Suggested assignment text

> Implement MiniCon SrcMgr starting at AGENTS.md, PRD.md and plan/implementation-handoff.md. Complete the M0 foundation work packages and then the smallest safe host-native M1 local Git slice. Reuse pinned MiniCon crates, preserve the future VCS capability boundary, and keep low resource use measurable from the first native shell. Use disposable fixtures and actual Git-state oracles. Keep later features behind their milestone boundaries, record blocked native/platform gates honestly, and do not modify the sibling MiniCon checkout or publish binaries. Follow the handoff's completion/evidence rules and update owning PRDs as implementation progresses.

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
| 0.1.3 | Published 2026-11-14 as `0.1.3-beta` (pre-release, `releases/tag/v0.1.3-beta`): Clone, Remotes, language, divergence and conflict handoff are `[v]`; the clean-image and five-participant courts are `NOT-RUN`; it closed under exit path 2 with both courts named in the notes | [notes](../releases/0.1.3.md), [plan](../plan/plan-0.1.3.md#qualification-path-and-exit-criteria) |
| 0.2.0 | Published 2026-11-26 as `0.2.0-beta` (pre-release under exit path 2, `releases/tag/v0.2.0-beta`): reviewed merge and rebase, conflict center with stages, whole-side and external-tool resolution, continue/skip/abort, restart truth. C7 is NOT-RUN with its runbook; the in-app result editor is 0.2.1, and no note may call the center complete without it | [plan](../plan/plan-0.2.0.md) |
| 0.2.1 | Planned, not started: the in-app result editor (four panes, search, IME, encoding and newline preservation), reusing the 0.2.0 stage check | [plan](../plan/plan-0.2.1.md) |

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
- **One bound, owned by the buffer.** `MAX_BYTES` and `MAX_LINES` live in the result
  buffer and the file module re-exports them; duplicating the numbers lets the
  edited text and the saved file disagree about what is too large.
- **Text belongs to the user: BOM or UTF-8, nothing else.** Guessing UTF-16 from
  NUL bytes will eventually rewrite a file nobody asked to change. Detect the
  BOM, refuse what does not decode, keep the newline style, and write through a
  temporary file plus rename so a failed save leaves the original bytes.
- **An editor resolution is staged, not assumed.** After the tool returns, the
  window stages the worktree copy and asks Git whether the path is still
  unmerged. Marker-like lines are reported as a warning because the user may
  have kept them on purpose; a zero exit from the tool proves nothing.
- **A job record is written before the command, never after.** If the record
  cannot be written, the mutation must not run: the window would otherwise make
  a change it cannot later account for. Clear it on every returning path; leave
  it only when the command itself failed, which is the case worth inspecting.
- **`repository()` leaves the fixture dirty on purpose, and tests must absorb it.**
  It ends with a modified `a.txt` and an untracked `u.txt`; a test that branches
  without committing them puts someone else's dirt in its own commit and can
  stop a rebase on the wrong commit.
- **Resolve by stage and object id, not by `--ours`/`--theirs`.** The two names swap
  meaning during a rebase, so a whole-side choice written with them can take the
  wrong side without any error. `update-index --cacheinfo mode,oid,path` plus
  `checkout-index -f` writes exactly the object the review named.
- **An attribute belongs to the item directly below it.** Inserting a struct
  between `#[derive(...)]` and its enum silently strips the enum's derives and
  breaks every test that formats it. Insert above the attribute, never between.
- **A conflict is what the index says, never what the file looks like.** Stage 1
  is the base, 2 current, 3 incoming; `ls-files -u` is the record, and a
  marker-like line may be the user's own text. Classify from the recorded modes
  (160000 submodule, 120000 symlink) and refuse classes the version cannot settle.
- **An abort restores from `ORIG_HEAD`, and only Git can say it worked.** Read
  `ORIG_HEAD` before offering the review, refuse when it is absent, and after
  `--abort` re-read the operation state: restored means no sequencer remains, not a
  zero exit.
- **A new footer hint must not preempt a stronger one.** The blocked-repository
  line carries the terminal handoff; an operation hint inserted above it hid the
  handoff and failed its test. Narrow the new condition instead of reordering by
  taste.
- **A letter bound as a cancel key eats the same letter in a field.** `n` closed
  the merge form whenever it was idle, so a target containing `n` could not be
  typed; the test that typed `feature` missed it. Cancel keys are scoped to the
  review state, and a typing test uses a value that contains every cancel letter.
- **Route a new surface in `key_inner` in the same commit that draws it.** The
  clone form proved the cost of the other order: it was painted and titled for a
  whole round while no keystroke reached its fields, and every test drove the
  form object directly. The merge form shipped with both the routing branch and
  the `Workspace::key` test.
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
- **A gate that cannot run here becomes NOT-RUN, not an open-ended blank.** Give
  it a runbook and a receipt shape, and write the three endings in advance: pass,
  ship as a beta naming it, or fail and lead the next plan. 0.1.0 set the
  precedent by shipping as a beta with six named gaps.
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

- **`merge-tree` mixes paths with prose unless told not to.** With `-z
  --name-only` it still appends its informational records to the same
  NUL-separated stream; `--no-messages` is what makes the output parsable, and the
  first field is the written tree OID, not a path.
- **A refused ticket must not consume the review.** Read the pending entry,
  compare the ticket, refuse without taking it, and take it only on a match.
  `take()` before the comparison burns a valid review on one mistyped approval.
- **A non-zero merge is not always a conflict.** `git merge` exits 1 both when it
  stops on conflicts and when it refuses; the only reliable difference is whether
  Git left merge state (`MERGE_HEAD` or unmerged entries). Read the state after
  the exit, never the code alone.
- **A non-zero exit can be data.** `runner::run` turns any non-zero code into an
  error and drops stdout, so a preview that reads Git's own verdict (`merge-tree`
  exits 1 when paths conflict) needs `run_allowing_failure`; do not widen `run`.
- **The host Git can be below the product floor.** This machine ships 2.42 against
  a 2.43 floor, so every fixture test fails with `UnsupportedVersion` and looks
  like a code defect. Point `SRCMGR_TEST_GIT` at a newer binary for fixtures; never
  lower the floor to get a green run.
- **A new capability class moves the minor number.** 0.1.1-0.1.3 closed promised gaps and kept the minor; merge and rebase write an integration commit the product never wrote before, so they opened 0.2.0. Gap closure and new reach are different claims and get different numbers.
- **A beta is declared, not inferred.** The tag and the release page say
  `0.1.3-beta` and name the unrun courts; a green suite cannot promote a build to
  a normal release. Precedent: 0.1.0 shipped as a beta naming six gaps rather than
  letting "we tested it" stand in for a court.
- **A published version's ledger rows keep `NOT-RUN`, never `[_]`.** `[_]` reads
  as future work on something already shipped; the honest mark is the one the
  release record uses.

- **A download table is a measurement, not a transcription.** Before the release
  notes are committed, measure each archive, check every line of `SHA256SUMS.txt`,
  run the packaged executable for its version and read the signature status back.
  A table copied from a CI log is a claim; only the shipped bytes are evidence.
- **An overlay pane must see keys before the page underneath.** The result editor
  sits on top of the conflict centre, which stays open so Escape can go back. The
  centre's arm was checked first, so every key went to the list and the buffer was
  drawn but deaf. Route the topmost surface first.
- **A multi-line buffer needs Enter.** `ResultEditor::edit` silently ignored
  `Key::Enter`, so a drawn newline could never be typed. The unit that owns the
  text owns every key that changes it.
- **Saving a conflict result re-derives the path from the index.** The pane names
  a path a moment ago; before writing, the backend re-reads the unmerged entries
  and refuses anything else, so a stale pane cannot write a file Git no longer
  considers conflicted. The same check rejects parent-dir components.
- **Newline normalisation belongs at save, not in the buffer.** Read keeps CRLF in
  the text; the buffer never rewrites it. Matching the file's recorded style just
  before the write keeps "CRLF stays CRLF" true without a second editor rule.
- **A requirement left out keeps its ledger row open.** 0.2.1's panes arrived
  without the cross-pane search, so D1 stays `[-]` even though the four panes
  draw and take keys; a delivered half is not a closed case.
- **A field over a buffer is another layer of the same key chain.** The find
  field inside the result editor takes characters, text and paste before the
  buffer does; Escape closes the field before it closes the editor. Any new input
  surface joins the top of that chain, never a parallel one.
- **IME here is one committed text event.** The window has no composition stream,
  so a composition arrives as a single `Key::Text` that inserts once and costs one
  undo step. A `preedit` field nothing can set is decoration; it was removed
  rather than described as a feature.
- **"Empty" is a seed like any other.** The result buffer can start from the
  current side, the incoming side or nothing, and the third case needs its own key
  and its own test; `None` falling back to an empty string is not a choice.
- **A search count states its rule.** Matches are counted as plain byte
  occurrences, case-sensitive, across the result and three sides; the panel says
  "across four panes" and the plan does not call it case-insensitive search.
- **Download release artifacts one at a time.** `gh run download <run> -D dir`
  can stop partway through a run with an EOF from blob storage; per-artifact
  downloads are resumable and their failures are nameable. `gh release download`
  needs `-R owner/repo` when it runs outside the checkout.
- **The signed artifact already holds the signed bytes.** The signing job's
  `signed-candidates` upload carries `signing-input/cells/{cell}/minicon-srcmgr.exe`
  (the signed files, not the inputs they started from) plus one inspection per
  cell and the receipt; hand that directory to `assemble_release.py --signed` and
  it finds each cell's bytes and inspection exactly once.
- **A release page is checked from the outside.** After `gh release create`,
  re-download at least one archive through the release URL and compare its digest
  with `SHA256SUMS.txt`; the local assembly proves the pipeline, not the upload.
- **The owner decides which minor a move belongs to.** An implementer split the
  ref and commit moves into 0.2.2 (rename, tags, amend) and 0.3.0 (pick, revert,
  reset) by reasoning about which ones change the graph. The owner overrode both
  steps: keep the 0.2 minor, and move the whole of the planned 0.3.0 scope into
  0.2.2. Version numbering is the owner's call, not a consequence derived from
  reach alone, and neither split is to be re-derived.
- **A move is reported only when the after-value differs.** Read the ref before
  and after with `rev-parse`; a zero exit is not a move, and a ticket is stale
  when the value it reviewed is no longer the value on disk.
- **A destructive mode names what it destroys first.** Reset lists the commits
  that become unreachable, read from the graph; a hard reset refuses over tracked
  worktree changes instead of offering a force flag.
- **Direction is the owner's sentence, written into the owner module first.**  "Match and then exceed SourceTree and lazygit; the CLI matches standard git and  adds its own control surface" arrived as one message. It went into PRD 01  (targets plus a CLI contract table) and PRD 13 (the `control` surface) before  any code, and the versions were re-sequenced from those owners: 0.2.2 CLI,  0.2.3 control, 0.2.4 history moves.- **The owner chose removal over a compatibility mode.** The private CLI spelling  (`branch <worktree> create`, `history`, `setting`, worktree-first) is removed,  not kept beside the Git spelling. "Exceed" for UI/UX is bound to resources and  response by measurement; do not re-derive it as feature count.- **When a file's line ending is unknown, match per line.** Two patch scripts  aborted on a `
` assumption in one file and a `
` assumption in the next,  and both aborted before writing. Assert single-line anchors that contain no  line break, or detect the ending from the bytes.- **Measure `git` before writing "compatible".** Four rules were run on a real
  `git` before the parser was written, and three corrected what was about to be
  coded: `-C` is valid before the subcommand only, `-C ""` is valid and changes
  nothing, and `--porcelain=v2 -z` has no `# branch` header without `--branch`.
  A compatibility rule remembered from impression is a guess; the oracle is the
  binary.
- **A smoke script's own mistakes imitate product defects.** Three rounds failed
  on the script (`-C` placed after the verb, `> -- -dash.txt` which is not a
  redirect in bash, a `cd` that moved the working directory) while the product
  was right each time. Before changing the product for a failing smoke step,
  run the same command by hand and compare with real `git`.
- **Do not assemble Markdown patches from `"..." + nl` chains in Python.** Three
  scripts failed on that shape, and a heredoc holding `''` or `})` broke bash
  quoting twice. Write each insertion as one triple-quoted block, convert its
  line endings with one `.replace`, and keep the Python to locate-and-insert.
- **Porcelain compatibility is a pass-through, not a rebuild.** `StatusSnapshot`
  has two coarse `ChangeKind`s and no mode, oid or score, so it cannot produce a
  rename record. The product already reads `status --porcelain=v2 -z --branch`,
  so `--porcelain` returns that command's bytes after the trust gate.
- **A mechanical migration is reviewed in the diff before a test runs.** The
  first pass of the CLI test migration inserted `--` into `diff` and `hunk`
  calls and turned `--staged` and an action word into paths; the diff showed it.
  Migrate call shapes only, leave assertions alone, and read the diff per shape.

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

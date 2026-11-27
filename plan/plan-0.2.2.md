# 0.2.2 refs and amend

Status: `[_]` planned, not started. The workspace version stays **0.2.1** until
this version's work begins. The owner kept the 0.2 minor open rather than opening
0.3.0, so this version holds the moves that do not produce a new commit graph.

## Why this is 0.2.2 and not 0.3.0

Cherry-pick, revert and reset change what the repository commits to; they earn a
new minor. Renaming a branch, creating or deleting a local tag and amending the
tip all edit a name or the last commit in place. They are the ref half of the same
sentence 0.2.0 started — "move something on purpose, after naming the loss" — and
they close inside the current minor.

## Product outcome

**A developer renames a branch, creates or deletes a local tag, and amends the tip
from a reviewed decision that names the target commit and the old id.**

```text
0.2.2 · names and the tip, moved only after the loss is named
├── A. Branch name
│   ├── Rename the current or a selected local branch
│   ├── The upstream setting follows the new name
│   └── An existing target refuses before anything moves
├── B. Tags
│   ├── Create a lightweight or annotated tag on a named commit
│   ├── Annotated needs an identity; without one the create is refused
│   └── Delete a local tag after naming the commit it points at
└── C. Amend
    ├── Amend the tip message, the staged content, or both
    ├── The review names the old id and says it stops being reachable
    └── A review whose staged index changed is stale and is refused
```

## Boundaries

**Must.** One review per move naming the before and after value; one-use tickets;
every mutation in a job record; the after-value read from Git; a rename that keeps
the upstream; a tag create that shows the object Git would write and the identity
it used; a delete that names the commit; an amend that re-reads the index at
approval and refuses a stale review.

**Not in 0.2.2.** Cherry-pick, revert, reset of any mode, remote tag deletion,
force-push, interactive rebase, reflog browsing, patch exchange, submodule and LFS
administration, and worktrees. Those are deferred whole to 0.3.0 rather than
half-built here.

**Excluded.** A move without a ticket; a rename over an existing name; an
annotated tag written with an invented identity; an amend that silently keeps a
review whose index changed; treating a zero exit as proof the ref moved.

## Review of the starting point

| Area | Actual state at 0.2.1 | Consequence for 0.2.2 |
|---|---|---|
| Preview/apply | Merge, rebase, resolution and skip each review one outcome with a one-use ticket | Refs reuse the ticket shape; a new ticket is still one use |
| Job record | Every mutation writes `action timestamp` first and clears it after | Rename, tag and amend join it |
| Write guard | An unfinished operation or unmerged path blocks writes | Ref moves refuse under it rather than half-applying |
| Branch reads | Branches are listed with upstream and ahead/behind | Rename writes through the same read afterwards |
| Tags | Nothing reads or writes them | New: list, create, delete; annotated needs an identity |
| Composer | Multiline message with refresh review and a pending guard | Amend extends it with the old id and the staged index check |

## Dependency graph

```text
T1 ref read/write helpers (srcmgr-git, no UI)
      │
T2 branch rename ──┐
      │            │
T3 tag list/create/delete ─┤  need T1
      │            │
T4 amend in the composer ──┘  needs T1, reuses the 0.2.1 save habits
      ↓
   release 0.2.2 (courts inherited)
```

Merge points: each preview lands with the key that opens it; a drawn surface that
cannot take keys is the defect 0.2.0 paid for twice.

## What must be true before the first line of code

- A move is read before and after with `rev-parse`; the report carries both
  values and only claims a move when they differ.
- A branch rename keeps `branch.<name>.remote` and `.merge` by renaming through
  Git, not by writing config beside it.
- An annotated tag uses the identity Git already resolves; when none exists the
  create is refused with the same reason a commit would give.
- A tag delete review names the commit the tag points at, not just the tag name.
- Amend re-reads the staged tree at approval; a review whose tree changed is
  refused as stale rather than amending the wrong content.

## Risks this plan accepts, and why

| Risk | Why it is accepted | What bounds it |
|---|---|---|
| A rename can break scripts and upstream tracking | Users ask for it and Git keeps the config consistent | Upstream follows the name; an existing target refuses first |
| Amend rewrites a commit that may be pushed | It is the ordinary way to fix the tip | The review names the old id and that it stops being reachable |
| Deleting a tag can lose a release marker | Local tags are cheap to recreate and the review names the commit | Only local tags; a tag that exists on the remote is not touched |

## Work packages

| Package | Deliverable | Minimum evidence |
|---|---|---|
| T1 · refs | Read a ref value, write a branch or tag, refuse a missing source | Fixture: values before and after for branch and tag; a missing ref refuses |
| T2 · rename | Rename a local branch from the branch list, with upstream preserved | Fixture: rename keeps `branch.<name>.remote/merge`; existing target refuses and writes nothing |
| T3 · tags | List tags, create lightweight or annotated, delete with the commit named | Fixture: annotated create shows identity; delete names the target; local-only |
| T4 · amend | Amend from the composer with the old id in the review | Fixture: message-only and staged-content amend; stale index refused |

## Invariants this increment must not weaken

- A name or the tip moves only after a review of that exact move and a one-use
  ticket; a changed repository makes the ticket stale, not the review optional.
- Every mutation writes a job record first and clears it when the command returns.
- The reported after-value is read from Git, never assumed from a zero exit.
- An amend names the commit it replaces before it replaces it.
- Nothing here pushes; a moved ref stays local until the existing push path runs.
- Binary, symlink, submodule and delete/modify conflicts stay refused by name.

## Acceptance ledger

| Case | Required outcome | State |
|---|---|---|
| P1 · rename | A local branch is renamed with its upstream; a taken name refuses | `[_]` |
| P2 · tags | Lightweight and annotated tags are created; an absent identity refuses | `[_]` |
| P3 · delete | Deleting a local tag names the commit and is read back | `[_]` |
| P4 · amend | Message-only and staged-content amend replace the tip; the old id is shown | `[_]` |
| P5 · stale | A ticket from a changed repository is refused without a write | `[_]` |

## Selected verification and stopping

Cheapest oracle first: fixture repositories with a known graph, ref values through
`git rev-parse`, and CLI tests through the window backend. Run the affected
`cargo test` selection with `SRCMGR_TEST_GIT`, `cargo clippy --workspace
--all-targets` and `python scripts/check_docs.py`.

## Round protocol

Same three artefacts as 0.2.0 and 0.2.1: the ledger row rewritten with the evidence
it has, an execution record entry, and [the memory palace](../prd/prd_mempalace.md)
updated with any fact that would otherwise be re-learned.

## Qualification path and exit criteria

C6 and C7 are inherited with their runbooks unchanged. 0.2.2 closes in one of the
same three ways; if the courts still cannot run, it publishes as `0.2.2-beta`
naming them.

## Deferred to 0.3.0

Cherry-pick, revert, soft/mixed/hard reset, remote tag deletion, force-push and
interactive rebase. Their product requirements stay in [PRD 01](../prd/PRD_01_product.md)
and [PRD 06](../prd/PRD_06_references.md); this plan does not implement a part of
them, and no partial surface is exposed.

## Execution record

Nothing built yet. The first round starts with T1 and writes down the fixture
graph it will use before writing the helper: one branch with an upstream, one
annotated tag, and a tip that can be amended twice.

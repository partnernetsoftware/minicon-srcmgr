# 0.2.4 history moves

Status: `[_]` planned, not started. The workspace version stays **0.2.1** until this version's work begins. The owner kept the 0.2 minor open and moved the whole of the planned 0.3.0 scope into it, so this version is every ref and commit move the product has not built yet: rename, tags, cherry-pick, revert, reset and amend. It is sequenced after the Git-compatible command line ([0.2.2](plan-0.2.2.md)) and the `control` surface ([0.2.3](plan-0.2.3.md)), because each new verb there is written once, in both spellings.

## Why this is 0.2.4 and not 0.3.0

Picking, reverting and resetting do change what the repository commits to, and
renaming a branch, managing tags and amending the tip edit a name or the last
commit in place. An earlier split — graph-changing moves to 0.3.0, the rest to a
patch — was an implementer's derivation. The owner's decision is that all of it
finishes the sentence 0.2.0 started, "move something on purpose, after naming the
loss", inside the current minor. Version numbering is the owner's call.

## Product outcome

**A developer moves a branch, a tag or a commit from a reviewed decision, sees
exactly what is lost, and is never told a move happened when Git refused.**

```text
0.2.4 · move refs and commits only after the loss is named
├── A. Refs
│   ├── Rename a local branch, with its upstream and the refs that point at it
│   └── Create or delete a tag, annotated or lightweight, local only
├── B. Commits
│   ├── Cherry-pick one commit onto the current branch
│   ├── Revert one commit with its message
│   └── Amend the last commit's message, author or staged content
└── C. Reset
    ├── soft, mixed and hard, each naming the commits that stop being reachable
    └── a hard reset refuses while the worktree holds changes it would overwrite
```

## Boundaries

**Must.** One review per move that names the current and resulting ref value;
one-use tickets bound to that review; every mutation in a job record; a reset
that lists the commits it makes unreachable; amend that keeps the old commit id
in the review; conflict stops that name paths and leave the branch where Git left
it; refusal while another operation is in progress.

**Not in 0.2.4.** Interactive rebase, multi-commit cherry-pick ranges, remote tag
deletion, force-push, patch exchange, submodule and LFS administration, worktrees,
and reflog browsing. Force-push stays a separate earlier decision and is not
widened here.

**Excluded.** A move without a ticket, a reset whose lost commits are not named,
deleting a tag that exists only on the remote, and treating a clean exit code as
proof that a ref moved — the resulting value is read back.

## Review of the starting point

| Area | Actual state at 0.2.1 | Consequence for 0.2.4 |
|---|---|---|
| Preview/apply | Merge, rebase, resolution and skip each review one outcome with a one-use ticket | Every move reuses the ticket shape; a new ticket is still one use |
| Job record | Every mutation writes `action timestamp` first and clears it after | Rename, tags, pick, revert, reset and amend join it |
| Write guard | An unfinished operation or unmerged path blocks writes | Every move refuses under it rather than half-applying |
| Conflict stop | 0.2.0 detects an operation from Git's own metadata and 0.2.1 resolves it in the window | A pick or revert that stops on conflicts lands in the existing conflict centre |
| Branch reads | Branches are listed with upstream and ahead/behind | Rename and reset write through the same read afterwards |
| Tags | Nothing reads or writes them | New: list, create, delete; annotated needs an identity |
| Composer | Multiline message with refresh review and a pending guard | Amend extends it with the old id and the staged index check |
| History | Commits are listed with subject and id | Pick, revert and reset take their target from that list |

## Dependency graph

```text
T1 ref read/write helpers (srcmgr-git, no UI)
      │
T2 branch rename + tag list/create/delete ──┐
      │                                    │
T3 cherry-pick / revert (single commit) ────┤  need T1
      │                                    │
T4 reset soft/mixed/hard ───────────────────┤  needs T1
      │                                    │
T5 amend in the composer ──────────────────┘  needs T1
      ↓
   release 0.2.4 (courts inherited)
```

Merge points: each preview lands with the key that opens it; a drawn surface that
cannot take keys is the defect 0.2.0 paid for twice. T2-T5 touch different files
in the Git layer and different keys in the window, so they may proceed side by
side once T1 has landed; `workspace.rs` key routing stays with one integrator.

## What must be true before the first line of code

- "Move" means the ref value is read before and after; the report carries both
  ids, and a move is only reported when the after-value differs.
- A branch rename keeps `branch.<name>.remote` and `.merge` by renaming through
  Git, not by writing config beside it.
- An annotated tag uses the identity Git already resolves; when none exists the
  create is refused with the same reason a commit would give.
- A tag delete review names the commit the tag points at, not just the tag name.
- A reset review lists every commit that becomes unreachable from the target ref,
  read from Git, not inferred from a count.
- A hard reset refuses when the worktree has tracked changes it would overwrite,
  and the refusal names the paths rather than offering a force flag.
- Amend re-reads the staged index at approval time; a review whose index changed
  is stale and is refused.
- A cherry-pick or revert that stops on conflicts is reported by the operation
  state Git leaves, never by the exit code alone.

## Risks this plan accepts, and why

| Risk | Why it is accepted | What bounds it |
|---|---|---|
| Rewriting history can lose commits | The product's whole premise is explicit previews | Lost commits are listed; reset is three named modes; tickets are one use |
| Refs can be moved while another operation runs | Git itself allows some of it | The write guard refuses before the first mutation |
| Annotated tags vary by Git configuration | Asking Git to write the tag is the only honest path | The review shows the object Git would create and the identity it used |
| Amend changes what a push would publish | Users ask for it; hiding it is worse | The review names the old id and the fact that the old commit stops being reachable |
| A pick or revert stops mid-way | Git leaves an operation state for exactly this case | The conflict centre opens on the paths; the branch is read back, not assumed |
| Scope is four surfaces instead of one | The owner chose to finish the minor | Each surface is one work package with its own fixture; the ledger closes per row |

## Work packages

| Package | Deliverable | Minimum evidence |
|---|---|---|
| T1 · refs | Read a ref value, write a branch or tag, refuse a missing source | Fixture: values before and after for branch and tag; a missing ref refuses |
| T2 · rename and tags | `r` renames a branch with its upstream; `T` opens tags for create and delete, each with a review | Fixture: rename keeps `branch.<name>.remote/merge`; annotated create shows identity; delete names the target; an existing target refuses and writes nothing |
| T3 · pick and revert | One commit picked or reverted from the history list, conflict stop named | Fixture: applied and conflicted outcomes, branch id read back |
| T4 · reset | soft, mixed and hard with the unreachable-commit list and a dirty-worktree refusal | Fixture: each mode against a known graph; refusal writes nothing |
| T5 · amend | Amend from the composer with the old id in the review | Fixture: staged-content and message-only amend; stale index refused |

## Invariants this increment must not weaken

- A ref or commit moves only after a review of that exact move and a one-use
  ticket; a changed repository makes the ticket stale, not the review optional.
- Every mutation writes a job record first and clears it when the command returns.
- The reported after-value is read from Git, never assumed from a zero exit.
- A destructive mode names what it destroys before it runs.
- An amend names the commit it replaces before it replaces it.
- Nothing in this version pushes; a moved ref stays local until the existing push
  path publishes it.
- Binary, symlink, submodule and delete/modify conflicts stay refused by name.

## Acceptance ledger

| Case | Required outcome | State |
|---|---|---|
| R1 · refs | Branch rename and tag create/delete are reviewed, ticketed and read back | `[_]` |
| R2 · pick/revert | One commit is picked or reverted; conflicts name the paths | `[_]` |
| R3 · reset | soft, mixed and hard all list the commits they make unreachable | `[_]` |
| R4 · reset refusal | A hard reset over tracked changes refuses and writes nothing | `[_]` |
| R5 · amend | Amend shows the old id, accepts staged content and refuses a stale index | `[_]` |
| R6 · guard | Every move refuses while an operation or unmerged path is present | `[_]` |

## Selected verification and stopping

Cheapest oracle first: fixture repositories with a known graph for each move,
byte-level assertions on refs through `git rev-parse`, and CLI tests through the
window backend. Run the affected `cargo test` selection with `SRCMGR_TEST_GIT`,
`cargo clippy --workspace --all-targets` and `python scripts/check_docs.py`.

## Round protocol

Same three artefacts as 0.2.0 and 0.2.1: the ledger row rewritten with the evidence
it has, an execution record entry, and [the memory palace](../prd/prd_mempalace.md)
updated with any fact that would otherwise be re-learned.

## Qualification path and exit criteria

C6 and C7 are inherited with their runbooks unchanged. 0.2.4 closes in one of the
same three ways; if the courts still cannot run, it publishes as `0.2.4-beta`
naming them.

## Still outside the 0.2 minor

Interactive rebase, multi-commit cherry-pick ranges, remote tag deletion,
force-push, patch exchange, submodule and LFS administration, worktrees and reflog
browsing. Their product requirements stay in [PRD 01](../prd/PRD_01_product.md)
and [PRD 06](../prd/PRD_06_references.md); this plan does not implement a part of
them, and no partial surface is exposed.

## Execution record

Nothing built yet. The first round starts with T1 and writes down the fixture
graph it will use before writing the helper: one branch with an upstream, one
annotated tag, a second branch carrying one commit to pick, and a tip that can be
amended twice.

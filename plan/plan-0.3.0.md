# 0.3.0 history moves

Status: `[_]` planned, not started. The workspace version stays **0.2.1** until
this version's work begins. 0.2.0 and 0.2.1 taught the product to integrate and
to resolve; 0.3.0 teaches it to move what is already committed.

## Why this is 0.3.0 and not 0.2.2

0.2.x closed one capability class: an integration that stops on conflicts and a
result written in the window. 0.3.0 adds a class the product has never had —
history and refs that are moved on purpose: a branch renamed, a tag created or
deleted, a commit picked or reverted, a branch reset, a commit amended. Every one
of them can lose work or publish a wrong ref, which is why they need their own
review contract rather than another form in the 0.2.0 mould.

## Product outcome

**A developer moves a branch, a tag or a commit from a reviewed decision, sees
exactly what is lost, and is never told a move happened when Git refused.**

```text
0.3.0 · move refs and commits only after the loss is named
├── A. Refs
│   ├── Rename a local branch, with its upstream and the refs that point at it
│   └── Create or delete a tag, annotated or lightweight, local or pushed
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

**Not in 0.3.0.** Interactive rebase, multi-commit cherry-pick ranges, patch
exchange, submodule and LFS administration, worktrees, reflog browsing, and any
rewrite of pushed history beyond what the review shows. Force-push stays a
separate 0.2-era decision and is not widened here.

**Excluded.** A move without a ticket, a reset whose lost commits are not named,
deleting a tag that exists only on the remote, and treating a clean exit code as
proof that a ref moved — the resulting value is read back.

## Review of the starting point

| Area | Actual state at 0.2.1 | Consequence for 0.3.0 |
|---|---|---|
| Preview/apply | `preview_merge`/`apply_merge`, conflict previews and skip/abort follow one ticket per reviewed outcome | Refs and commits reuse the same ticket shape; a new ticket is still one use |
| Job record | Every mutation writes `action timestamp` first and clears it after | Reset and amend join it; a leftover record still means reconciliation |
| Write guard | Unfinished operations and unmerged paths block writes | A move refuses to start under an unfinished operation rather than half-applying |
| Composer | Multiline message with refresh review and pending guard | Amend extends it with the old commit id; no new message surface |
| Refs list | Branches and remotes are read and displayed | Rename and tags write to the same reads afterwards |
| Tags | Nothing reads or writes them | New: list, create, delete; annotated tags need an identity |

## Dependency graph

```text
H1 refs read/write helpers (srcmgr-git)
      │
H2 branch rename + tag create/delete (preview/apply)
      │
H3 cherry-pick / revert (single commit, conflict stop)
      │
H4 reset soft/mixed/hard (lost-commit list)
      │
H5 amend in the composer
      ↓
   release 0.3.0 (courts inherited)
```

Merge points: every preview and its apply land together with the key that shows
it; a drawn form that cannot take keys is the defect 0.2.0 paid for twice.

## What must be true before the first line of code

- "Move" means the ref value is read before and after; the report carries both
  ids, and a move is only reported when the after-value differs.
- A reset review lists every commit that becomes unreachable from the target ref,
  read from Git, not inferred from a count.
- A hard reset refuses when the worktree has tracked changes it would overwrite,
  and the refusal names the paths rather than offering a force flag.
- Amend re-reads the staged index at approval time; a review whose index changed
  is stale and is refused.
- Annotated tags need an author identity; when Git has none, the create is
  refused with the same reason a commit would give.

## Risks this plan accepts, and why

| Risk | Why it is accepted | What bounds it |
|---|---|---|
| Rewriting history can lose commits | The product's whole premise is explicit previews | Lost commits are listed; reset is three named modes; tickets are one use |
| Refs can be moved while another operation runs | Git itself allows some of it | The write guard refuses before the first mutation |
| Annotated tags vary by Git configuration | Asking Git to write the tag is the only honest path | The review shows the object Git would create and the identity it used |
| Amend changes what a push would publish | Users ask for it; hiding it is worse | The review names the old id and the fact that the old commit stops being reachable |

## Work packages

| Package | Deliverable | Minimum evidence |
|---|---|---|
| H1 · refs | Read and write branch and tag refs with before/after values | Fixture: rename keeps upstream; tag create/delete round-trips |
| H2 · refs surface | `r` renames a branch, `T` opens tags for create/delete, each with a review | Key routing test plus a real repository assertion |
| H3 · pick and revert | One commit picked or reverted, conflict stop named | Fixture: applied and conflicted outcomes, branch id read back |
| H4 · reset | soft/mixed/hard with a lost-commit list and a dirty-worktree refusal | Fixture: each mode against a known graph; refusal writes nothing |
| H5 · amend | Amend from the composer with the old id in the review | Fixture: staged change and message-only amend; stale index refused |

## Invariants this increment must not weaken

- A ref or commit moves only after a review of that exact move and a one-use
  ticket; a changed repository makes the ticket stale, not the review optional.
- Every mutation writes a job record first and clears it when the command returns.
- The reported after-value is read from Git, never assumed from a zero exit.
- A destructive mode names what it destroys before it runs.
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

C6 and C7 are inherited with their runbooks unchanged. 0.3.0 closes in one of the
same three ways; if the courts still cannot run, it publishes as `0.3.0-beta`
naming them.

## Execution record

Nothing built yet. The first round starts with H1 and writes down the fixture
graph it will use before writing the helper.

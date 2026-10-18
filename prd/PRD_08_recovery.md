# 08 · Conflicts, operation safety and recovery

Parent: [PRD index](../PRD.md). Owns the mutation gate, interrupted operation state, destructive action policy and undo limits. This is the common prerequisite for all writers.

## Outcome tree

```text
Outcome: every risky transition is deliberate and every interruption is understandable
├── Gate — identity, freshness, trust, locks and explicit intent
├── Operate — one owned writer, progress and truthful cancellation
├── Resolve — actual Git sequencer state and conflict entries
└── Recover — bounded snapshots, refs, reflog and explicit limitations
```

```mermaid
stateDiagram-v2
    [*] --> Preview
    Preview --> Rejected: stale / untrusted / busy
    Preview --> Queued: authorized intent
    Queued --> Running: revalidated
    Running --> Waiting: helper / hook / signer
    Waiting --> Running: response
    Running --> Conflicted: Git conflict
    Running --> Reconciling: exit / cancel / crash / timeout
    Conflicted --> Running: continue / skip / abort
    Reconciling --> Completed: actual post-state known
    Reconciling --> Unknown: unresolved result
    Unknown --> Reconciling: observe again
    Rejected --> [*]
    Completed --> [*]
```

## Mutation gate and risks

An operation plan contains worktree/common-dir identities, action, literal paths/resolved OIDs, observed HEAD/index/content fingerprints, trust generation, risk class, expected effects and recovery limits. Preflight occurs before preview and immediately before execution. A changed target, trust decision or material state invalidates confirmation. A queued job never treats an old confirmation as permission for a different effect.

| Risk class | Examples | Interaction |
|---|---|---|
| Read | graph, raw diff, status | No risk dialog; security policy still applies |
| Reversible local intent | stage/unstage, create branch, normal commit | Direct named action and visible result; preflight required |
| Destructive/rewrite | discard, hard/mixed reset, amend, rebase/drop, force-delete, replace tag | Preview exact scope and recovery limits; explicit confirm bound to current state |
| Remote publication/deletion | push, lease rewrite, delete remote ref | Destination/ref preview; lease rewrite and delete require a distinct confirmation |
| Irrecoverable deletion | remove untracked content, delete worktree files, delete local safety copy | Typed target confirmation and explicit no-guaranteed-recovery warning |

A confirmation is part of the GUI workflow, not a request to the repository owner during this documentation task. No permanent “never warn again” switch for destructive classes. Keyboard Enter cannot confirm while a modal is still receiving its opening keypress; destructive default focus is Cancel.

## Conflict center

Detect merge, rebase, cherry-pick, revert, commit-series `am` and sequencer progress from Git-reported state and metadata, including operations started outside the app. Show operation type, original/current HEAD, current item and remaining sequence when available. Externally created/broken metadata yields Inspect/Refresh guidance; no inferred abort based on one filename.

List unmerged paths and index stages. Provide base/current/incoming and result panes for text, with exact stage labels; during rebase explain the stage roles instead of relabeling them as the user's branch ambiguously. Handle add/add, delete/modify, rename/rename, directory/file, binary, symlink and submodule conflicts explicitly. Whole-side choice previews the resulting path/content/deletion and stages only after the user marks resolved. External merge tool launch is separately trusted and path-safe. A zero tool exit code does not prove resolution; inspect unmerged entries afterward.

The text result editor is limited to resolving a conflict, supports search and IME, and saves exact user-approved encoding/newlines. Check marker-like lines as a warning rather than assuming every marker is a conflict. Mark resolved requires an actual chosen result or explicit deletion. Continue is enabled only when the required Git state permits it. Skip previews the commit being omitted. Abort invokes the matching Git operation; abort failure or leftover edits remains visible, never called a successful rollback.

## Cancellation, crash and locks

Cancel means request interruption of the owned process tree, allow bounded graceful exit, then terminate only owned descendants under the platform policy. Do not kill arbitrary processes named Git. After cancellation, re-read repository and remote state. Hooks can have external side effects and Git can leave a resumable operation; cancellation is not undo. Local read default timeout is thirty seconds with explicit extension; mutation/network jobs prompt after 120 seconds of no observable progress, without silently terminating a possibly successful write. Bounded pipe sizes are owned by [14](PRD_14_verification_roadmap.md).

On startup, journal jobs left running become Needs reconciliation, never success/failed by assumption. A second app instance observes shared common-dir writer ownership. Git `index.lock`/ref locks are reported as Busy with path-safe diagnostics. No automatic lock removal; manual removal is outside the baseline UI.

## Recovery and destructive content

Before destructive tracked-content actions, offer a durable safety copy of affected worktree bytes and index intent, including deletions/modes where supported. Recovery storage is outside the repository and must pass free-space/permission/write/read-back checks. If the action depends on a safety copy and storage fails, block it; a separate clearly labeled proceed-without-copy path requires the irreversible confirmation class. Untracked cleanup is previewed file by file and defaults to OS trash where reliably supported, with explicit capability error otherwise; permanent deletion is never a fallback selected silently. Submodules, nested repositories and links are never recursively cleaned by a generic action.

Before ref rewrites, retain an application recovery ref to the old OID and record the operation. Reflog/recovery refs can restore reachable commits, not never-committed bytes or external hook effects. Restore previews current versus recovered state and uses the same gate; do not overwrite new work because an old job has an Undo button. Delete safety records only through storage management under [13](PRD_13_state_control.md). No unlimited durable backup guarantee.

## Delivery contracts

| ID / leaf | Invariant | Observable evidence | Safe failure | Dependencies | Non-goal |
|---|---|---|---|---|---|
| REC-01 · gate | Authorizations bind fresh target/effects | QA-J05 stale diff/ref/identity and queue race | Reject without widening intent | 02, 11, 13 | Global transaction across Git |
| REC-02 · resolve | Continue/skip/abort follow actual Git state | QA-J03 all conflict types and external sequencers | Preserve state; show actionable error | 04, 06 | Automatic semantic resolution |
| REC-03 · interruption | Cancel/crash reports observed result truthfully | QA-J04/J10 crash at each state; helper lifecycle | Needs reconciliation / Unknown | 02, 07, 13 | Cancel guarantees rollback |
| REC-04 · recover | Recovery promises match stored content/refs | QA-J04 destructive preview, full disk and undo-after-new-edit | Block unsafe overwrite; expose limits | 11, 13 | Full backup of every file |

References: Git defines [index stages](https://git-scm.com/docs/git-update-index); the product's risk, recovery and UX requirements above are design decisions, not claims that Git implements a transactional GUI undo layer.

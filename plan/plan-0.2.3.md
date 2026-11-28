# 0.2.3 the control surface

Status: `[_]` planned, not started. The workspace version stays **0.2.1** until
this version's work begins.

## Why this version exists

Decision 2026-11-28: beside the Git-compatible command line, the product adds
its own management interface. [PRD 13](../prd/PRD_13_state_control.md#control-surface)
planned `minicon-srcmgr control <verb>` for M1/M2 and nothing implemented it.
This version delivers it as a local command-line surface: one JSON schema, the
same operation gate as the window, and no listener. It follows
[0.2.2](plan-0.2.2.md) so that its option parsing and trust lookup are written
once.

## Product outcome

**An automation inspects repository state and performs the same reviewed writes
as the window, and can never reach Git around the gate.**

```text
0.2.3 · observe and act through one gate
├── A. Observe (no grant beyond trust)
│   ├── version, capabilities
│   ├── list-repositories, repo-snapshot, jobs
│   └── one JSON document per verb, schema minicon-srcmgr.control/1
├── B. Act (the window's own writes)
│   ├── plan-action: id, repo, trust fingerprint, generation, effects, risk, 60 s
│   ├── execute-plan: consumes the plan, starts a journaled job
│   └── wait-job: final state and observed outcome, or timeout
└── C. Authority
    ├── stale plan (fingerprint or generation changed) refused without a write
    ├── replayed id fails; another instance's plans are invalid
    └── destructive risk needs control.destructive=allow AND --confirm-destructive
```

## Boundaries

**Must.** Every verb in the PRD 13 table; the schema string on every document;
plans bound to trust fingerprint and generation; one-use plans; jobs in the
profile store with a bounded ring; a refused plan leaves the repository
byte-identical; unknown verbs and invalid enums refused before Git runs.

**Not in 0.2.3.** The named-pipe/socket transport, event streams, semantic UI
actions and `cancel-job`; these stay in PRD 13's transport section for a later
milestone. A generic API token. Any raw Git argument list.

**Excluded.** A plan that survives a repository change; success reported for an
unknown result; a destructive write enabled by default.

## Review of the starting point

| Area | Actual state at 0.2.2 | Consequence for 0.2.3 |
|---|---|---|
| Writes | Each is a `srcmgr-git` function behind a preview in the window backend | `plan-action` calls the same preview functions; `execute-plan` the same apply |
| Job record | A one-line file in `.git/srcmgr-job`, written before and cleared after | Reused as-is; `control` adds the profile-side job document |
| Tickets | Random 128-bit, one use, expiring, bound to what was shown | Plans use the same generator and the same binding discipline |
| Store | SQLite profile with a settings namespace | Plans and jobs go under `control.*.v1.<id>` |
| JSON | `minicon_core::json` with `to_vec_pretty` | The control documents use it; schema version is a field, not a path |

## Dependency graph

```text
S1 observe: version, capabilities, list-repositories, repo-snapshot, jobs
      │
S2 plan-action for the existing write verbs (preview functions, no new write path)
      │
S3 execute-plan + wait-job (one-use, stale, instance-bound, journaled)
      │
S4 destructive class and control.destructive
      ↓
   release 0.2.3 (courts inherited)
```

Merge points: S1 is one commit and is useful alone. S2 and S3 land together for
each verb group, because a plan nobody can execute is a drawn form that cannot
take keys.

## What must be true before the first line of code

- `plan-action` never writes and `execute-plan` never previews: the effects a
  plan lists are the ones its apply performs.
- The plan id is 128 bits from the platform entropy source, and the stored plan
  holds the repo, the trust fingerprint and the generation it was made under.
- `execute-plan` reads the repository again and refuses on any difference before
  the job record is written.
- A job's document records the observed outcome read from Git, never an
  assumption from an exit code.
- `control.destructive` is absent or `refuse` by default; setting it to `allow`
  is a deliberate `config set`.

## Risks this plan accepts, and why

| Risk | Why it is accepted | What bounds it |
|---|---|---|
| Automation multiplies a mistake | The window already refuses the same mistake | The same gate, one-use plans, a stale check before the job record |
| A plan is swapped between creation and execution | Tickets solved this for the window | The id is unpredictable and the stored effects are compared, not trusted from argv |
| The JSON schema drifts | It is new | Schema string in every document; a fixture per verb pins the field set |
| Jobs accumulate | They are small | A bounded ring of 256 per profile |

## Work packages

| Package | Deliverable | Minimum evidence |
|---|---|---|
| S1 · observe | Five read verbs with the schema string | Fixture per verb asserting the field set; `repo-snapshot` refuses an untrusted repository |
| S2 · plan | `plan-action` for stage, unstage, commit, create-branch, switch-branch | Fixture: plan matches what the window preview shows for the same request |
| S3 · execute | `execute-plan`, `wait-job`; stale, replayed and foreign-instance refusals | Fixture: each refusal leaves HEAD and index byte-identical |
| S4 · destructive | Risk class and the `control.destructive` switch | Fixture: destructive plan refused by default, allowed only with both the setting and the flag |

## Invariants this increment must not weaken

- No path from `control` to Git that does not pass trust, the operation check
  and the job record.
- A plan is consumed on launch; an unknown result is reported as unknown.
- No credential, patch body or raw helper output in a plan or job document.
- No network listener, no background service, no world-readable token.

## Acceptance ledger

| Case | Required outcome | State |
|---|---|---|
| K1 · observe | Each read verb emits one document with the schema string and the pinned field set | `[_]` |
| K2 · plan | A plan lists exactly the effects its apply performs; nothing is written at plan time | `[_]` |
| K3 · execute | Stale, replayed and foreign-instance plans are refused without a write; a job records the observed outcome | `[_]` |
| K4 · destructive | A destructive plan needs the setting and the flag; default refuses | `[_]` |
| K5 · gate | No `control` verb reaches repository Git on an untrusted repository | `[_]` |

## Selected verification and stopping

Cheapest oracle first: JSON field-set fixtures, then real-repository refusals
with byte comparison of HEAD and index, then one end-to-end plan/execute/wait
per verb group. Run with `SRCMGR_TEST_GIT`,
`cargo clippy --workspace --all-targets` and `python scripts/check_docs.py`.

## Round protocol

Same three artefacts as 0.2.0 and 0.2.1: the ledger row rewritten with the
evidence it has, an execution record entry, and
[the memory palace](../prd/prd_mempalace.md) updated with any fact that would
otherwise be re-learned.

## Qualification path and exit criteria

C6 and C7 are inherited with their runbooks unchanged. 0.2.3 closes in one of the
same three ways; if the courts still cannot run, it publishes as `0.2.3-beta`
naming them.

## Execution record

Nothing built yet. The first round starts with S1, because the observe verbs
prove the schema and the trust lookup before any plan exists.

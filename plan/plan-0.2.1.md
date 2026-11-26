# 0.2.1 the result editor, inside the window

Status: `[_]` planned, not started. Workspace version stays **0.2.0** until that
version publishes. This plan finishes the sentence 0.2.0 deliberately left open:
whole-side and edited-copy resolution shipped, but the file itself was resolved
in an external editor. 0.2.1 puts the result pane in the window.

## Why this is 0.2.1

0.2.0 proved the conflict model, the tickets and the continuation. What it did
not do is let a user write the merged text where they read the three sides. That
is not a new capability class — it is the missing half of one already admitted —
so it takes the same minor number and the next patch.

## Product outcome

**A developer reads base, current and incoming in the window, writes the result
in the fourth pane, and the file is saved with the encoding and newlines it
already had — then staged by the same contract that staged it in 0.2.0.**

```text
0.2.1 · write the result where you read the sides
├── A. Read all four panes
│   ├── Base, current and incoming, read-only, labelled by stage role
│   ├── Result, editable, starting empty or from a chosen side
│   └── Search across the path's four panes with the match count
├── B. Edit without surprises
│   ├── Caret, selection, undo, IME composition, paste
│   ├── Encoding and newline style preserved from the conflicted file
│   └── Bounded: a 2 MiB or 200k-line file is refused by name, not truncated
└── C. Save, warn, stage
    ├── Save writes the worktree file atomically under a job record
    ├── Marker-like lines are a warning, never a refusal
    └── Stage uses the same index check 0.2.0 already proved
```

## Boundaries

**Must.** Four panes with the stage role named; a result pane that starts from
the current side, the incoming side or empty; search with a visible match count;
IME composition and paste; UTF-8, UTF-8 with BOM and UTF-16LE read and written
back unchanged; the file's newline style preserved (LF stays LF, CRLF stays
CRLF); a save that either replaces the file completely or leaves it untouched;
marker warning on save and on stage; staging gated by the existing index check.

**Not in 0.2.1.** Syntax colouring, word wrap, multiple cursors, diff3 markers
as an editing feature, conflict resolution for binary, symlink, submodule or
delete/modify paths (0.2.0 refuses those by name and this version keeps that),
and any change to Git's own merge or rerere behaviour.

**Excluded.** Writing a file whose encoding could not be determined — it is
refused with the reason instead of being re-encoded silently; saving without a
job record; staging without the index re-read; treating a marker-like line as
proof of an unresolved conflict.

## Review of the starting point

| Area | Actual state at 0.2.0 | Consequence for 0.2.1 |
|---|---|---|
| Editor widgets | `ComposerState` in `minicon_core` handles caret, selection, IME preedit and paste for single-line fields | The result pane needs a multi-line view with scrolling; the composer is the input contract, not the renderer |
| Conflict read model | `read_conflicts` and `side_text` return stages and bounded text | Panes read from these; nothing new is parsed from the worktree |
| Staging | `stage_resolution` stages the worktree copy and re-reads the index | Reused unchanged after Save |
| Encoding | Nothing in the product reads or preserves a file's encoding | New: detect, remember and write back; refuse what cannot be determined |
| Bounds | Sides are cut at 4000 characters for display | A file being edited cannot be cut; it is refused above the bound instead |
| Job record | Every mutation begins and finishes one | Save joins it like any other mutation |

## Dependency graph

```text
E1 encoding (new module, no UI)
      │
E2 editor state (new module, no UI) ──┐
      │                               │
E3 panes (window + workspace) ────────┤  needs E1+E2
      │                               │
E4 save and stage (backend) ──────────┘  needs E1, reuses 0.2.0 stage check
      ↓
   release 0.2.1 (courts inherited)
```

Merge points: `crates/srcmgr-ui/src/result_editor.rs` is one writer; `window.rs`
takes E3 in one commit with its `Workspace::key` test, because a pane that is
drawn and not routed is the defect 0.2.0 already paid for twice.

## What must be true before the first line of code

- The bound is chosen once and written down here: 2 MiB of bytes or 200 000
  lines, whichever comes first. A larger file is refused by name, and the
  conflict center keeps showing its bounded sides.
- Encoding detection is specification-first: a BOM decides UTF-8 or UTF-16LE;
  no BOM means UTF-8 if the bytes decode, otherwise the file is refused. No
  heuristic re-encoding, no locale guessing.
- Newline style is taken from the file as it exists in the worktree, and a file
  with mixed endings keeps them as written; the editor does not normalise.
- Undo is a bounded stack of editor snapshots (32 steps), not an operation log.

## Risks this plan accepts, and why

| Risk | Why it is accepted | What bounds it |
|---|---|---|
| A full text editor is a large surface | The four-pane reading already exists and the composer contract already handles caret/selection/IME | Bound, no syntax colouring, no word wrap, no multi-cursor |
| Encoding round trips can corrupt a file | Refusing an undeterminable encoding is safer than guessing | Byte-compared fixtures for every supported combination |
| A failed save can truncate work | Save is atomic: write beside the file, then rename | A test that a failed save leaves the file byte-identical |

## Work packages

| Package | Deliverable | Minimum evidence |
|---|---|---|
| E1 · encoding | Detect UTF-8, UTF-8 with BOM and UTF-16LE, and the newline style, from the conflicted file; write back exactly | Fixtures for each combination, byte-compared after a round trip |
| E2 · editor state | A multi-line buffer with caret, selection, undo, IME preedit, paste and vertical scrolling inside the bound | Unit cases for each edit operation; a frame test that a composed character lands once |
| E3 · panes | Base, current, incoming read-only and the result editable, with the stage role named and search across all four | Frame tests in both languages; a search case with the match count |
| E4 · save and stage | Atomic save under a job record, marker warning, then the existing stage check | A failed save leaves the file byte-identical; staging reuses the 0.2.0 path |

## Invariants this increment must not weaken

- The four panes show what Git recorded; the result pane is the only editable one.
- Save replaces the file atomically and preserves encoding and newlines; it never re-encodes silently.
- Marker-like lines are a warning on save and stage, and the index decides whether the path is resolved.
- A file above the bound is refused by name; it is never truncated into a result.
- Every mutation writes a job record first and clears it when the command returns.
- A binary, symlink, submodule or delete/modify conflict stays refused by name.

## Acceptance ledger

| Case | Required outcome | State |
|---|---|---|
| D1 · panes | Four panes, roles named, read-only except the result | `[_]` |
| D2 · encoding | UTF-8, BOM and UTF-16LE round-trip byte-identically; newline style preserved | `[_]` |
| D3 · editing | Caret, selection, undo, IME and paste behave inside the bound | `[_]` |
| D4 · save | Atomic under a job record; a failed save leaves the file untouched | `[_]` |
| D5 · stage | Marker warning, then the index check; an unresolved path still blocks commit | `[_]` |

## Selected verification and stopping

Cheapest oracle first: byte comparisons for encoding, unit cases for the editor,
frame tests for the panes, and a real conflicted repository for save and stage.
Run the affected `cargo test` selection with `SRCMGR_TEST_GIT`,
`cargo clippy --workspace --all-targets` and `python scripts/check_docs.py`.

## Round protocol

Same three artefacts as 0.2.0: the ledger row rewritten with the evidence it
has, an execution record entry, and [the memory palace](../prd/prd_mempalace.md)
updated with any fact that would otherwise be re-learned.

## Qualification path and exit criteria

C6 and C7 are inherited with their runbooks unchanged. 0.2.1 closes in one of
the same three ways; if the courts still cannot run, it publishes as
`0.2.1-beta` naming them.

## Execution record

No round has run.

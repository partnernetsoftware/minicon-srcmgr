# 0.2.1 the result editor, inside the window

Status: `[-]` in progress. Every ledger row D1-D5 is `[v]`; what remains is the
version bump, the release notes and the beta publication. Workspace version stays
**0.2.0** until that happens. This plan finishes the sentence 0.2.0 deliberately left open:
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
| D1 · panes | Four panes, roles named, read-only except the result | `[v]` `window.rs::result_editor_panel` draws base/current/incoming/result with caret bar and `L{line}:{col}`; `Ctrl+F` opens a find field that counts matches live across all four panes, and Enter/i/0 open the result from the current side, the incoming side or empty. The frame test passes in both languages and asserts `4 match(es) across four panes` / `四个窗格共 4 处匹配` |
| D2 · encoding | UTF-8, BOM and UTF-16LE round-trip byte-identically; newline style preserved | `[v]` `text_file` fixtures cover the three encodings plus LF/CRLF/mixed normalisation; `a_saved_result_keeps_the_file_encoding_and_newlines_and_clears_its_record` saves into a BOM+CRLF conflicted file through the real backend and compares the bytes |
| D3 · editing | Caret, selection, undo, IME and paste behave inside the bound | `[v]` `ResultEditor` holds text, caret, anchor and a 32-step undo stack, refuses any insertion that would cross either bound, and takes Enter, Ctrl+Z, Ctrl+Y and Ctrl+F from the window; ten fixtures pass. IME arrives as committed `Key::Text` (one insertion, one undo step) because the window has no composition stream, so no preedit is drawn or claimed; paste arrives as `Key::Pasted` through the same insert |
| D4 · save | Atomic under a job record; a failed save leaves the file untouched | `[v]` `Job::SaveResolution` re-reads the unmerged entries, refuses a path the index does not hold (and any parent-dir component), refuses a result over the bound, records `save-result` before `text_file::write` (temp file + rename) and clears it after; the fixture asserts a refusal writes nothing and the record is gone |
| D5 · stage | Marker warning, then the index check; an unresolved path still blocks commit | `[v]` The same fixture warns on a marker-like save, stages the saved copy, sees the stage warning again and reloads an empty conflict list. `conflict_markers` now drops a UTF-8 BOM before scanning, so a first-line marker in a BOM file is seen |

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

### 2026-11-26 · encoding first, UI later

- **Built.** `crates/srcmgr-cli/src/text_file.rs`: `read` detects UTF-8, UTF-8 with
  a BOM and UTF-16LE with a BOM, records the newline style (LF, CRLF or mixed)
  and refuses anything else with a reason that says it will not re-encode.
  `encode` and `write` put the bytes back exactly, and the write goes through a
  temporary file and a rename so a failure leaves the original untouched.
  `within_bound` holds the 2 MiB / 200 000 line limit in one place.
- **Why a BOM decides.** A heuristic that guesses UTF-16 from NUL bytes will
  eventually rewrite a file the user did not ask to change. The rule is written
  in the plan and implemented literally: a BOM or UTF-8, nothing else.
- **Evidence.** Six fixtures: plain UTF-8 with LF, a BOM kept out of the text and
  put back, UTF-16LE through a full round trip, undecodable bytes refused, a
  missing file refused without writing, and the bound checked in both bytes and
  lines.
- **Open.** Nothing calls it yet; E3/E4 will. Until the save path uses it, D2
  stays `[-]` even though the codec itself is done.

### 2026-11-26 · the buffer behind the result pane

- **Built.** `crates/srcmgr-ui/src/result_editor.rs`: `ResultEditor` holds the text,
  a byte caret and a selection anchor, with insertion that refuses to cross the
  byte or line bound, backspace that joins lines, vertical movement that keeps
  the column and clamps at a short line, select-all, and a 32-step undo/redo
  stack of text snapshots.
- **Why snapshots and not an operation log.** A conflict result is at most a few
  thousand lines in practice; 32 whole-text snapshots bound memory to something
  predictable, and a log would need its own inverse for every operation.
- **One bound, two crates.** `MAX_BYTES` and `MAX_LINES` live in the buffer and
  the CLI file module re-exports them, so the saved file and the edited buffer
  cannot disagree about what "too large" means.
- **Evidence.** Seven unit cases: typing at the caret, whole-selection replace,
  backspace joining two lines, vertical movement clamping at a short line and
  extending with an anchor, an insertion refused whole at the byte bound, undo
  and redo walking the same steps, and the line bound counted separately.
- **Open.** No pane, no save. D3 stays `[-]` until the window routes keys into
  this buffer.

### 2026-11-26 · four panes, and the save behind Ctrl+S

- **Built.** `window.rs::result_editor_panel` draws base, current and incoming
  read-only and the result editable, with the stage role named, a caret bar
  inserted between characters and an `L{line}:{col}` readout; each side is cut at
  eight lines with its remaining count and any display-bound truncation. Enter in
  the conflict centre opens it, seeded from the current side.
- **Routing came first, not last.** The editor sits above the conflict centre,
  which stays open so Escape can return. The centre's key arm was checked first,
  so every key reached the list and the buffer was drawn but deaf; the plane that
  is on top now gets the keys first, and a test types into the buffer and asserts
  the list selection did not move.
- **The buffer learned Enter, Ctrl+Z and Ctrl+Y.** `edit` silently ignored
  `Key::Enter`, so a newline could never be typed; undo and redo had no key at all
  even though the stack existed. Both now go through the buffer, and the footer
  names them.
- **Save is a job, not a file handle in the pane.** `Ctrl+S` sends
  `Job::SaveResolution { path, text }`. The backend re-reads the unmerged entries,
  refuses a path the index no longer holds and any parent-dir component, decodes
  the file, refuses a result over the 2 MiB / 200 000 line bound, matches the
  recorded newline style, records `save-result`, writes through a temporary file
  and a rename, clears the record, and reports marker-like lines as a warning.
- **A BOM hid a marker.** The fixture saved `<<<<<<< HEAD` as the first line of a
  BOM file and the stage refused to warn: `conflict_markers` compared raw bytes,
  so line one began with `EF BB BF`. The BOM is now dropped before the scan, and
  the same fixture asserts the warning on save and again on stage.
- **Evidence.** `cargo test -p srcmgr-ui` — 149 passed, including the bilingual
  four-pane frame and the Ctrl+S routing case. With `SRCMGR_TEST_GIT` pointing at
  MinGit 2.47.1, `a_saved_result_keeps_the_file_encoding_and_newlines_and_clears_its_record`
  passes: bytes `EF BB BF` + CRLF survive, the record is gone, a refusal writes
  nothing, staging the saved copy empties the conflict list.
- **Open.** Cross-pane search and the incoming/empty seed keep D1 `[-]`; IME
  composition keeps D3 `[-]`. Neither is claimed by the release note until it
  lands.

### 2026-11-26 · find, the seed choice, and what IME really is here

- **Built.** `Ctrl+F` opens a find field inside the result editor; typing counts
  matches live across the result and the three sides (plain byte search, no case
  folding), Enter closes the field and keeps the count, Escape clears it. The
  count line is drawn in both languages. In the conflict centre, Enter, `i` and
  `0` open the result from the current side, the incoming side or empty.
- **The find field is above the buffer.** While it is open it takes characters,
  text and paste itself, so a query can hold anything a filename can; the buffer
  never sees those keys. Escape closes the field before it closes the editor.
- **IME is committed text, and that is all this window has.** Earlier text claimed
  "IME composition" sat in `preedit`, but nothing could ever set that field and
  nothing drew it. The field is gone; a composition arrives as one `Key::Text`
  commit, inserts once, and costs one undo step. No preedit display is claimed.
- **Evidence.** `cargo test -p srcmgr-ui` — 152 passed, including the live-count
  case (4) and both seeds; the bilingual frame test asserts the count line. The
  whole-workspace run and the backend save fixture are unchanged from the previous
  entry.
- **Open.** Release preparation: version bump, notes, tag, and the inherited C6/C7
  courts.


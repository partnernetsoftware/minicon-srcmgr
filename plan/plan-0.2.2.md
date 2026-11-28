# 0.2.2 the Git-compatible command line

Status: `[-]` in progress. C1-C4 are built: the parser, repository discovery,
the Git names and formats, and the migrated tests. The workspace version stays
**0.2.1** until this version publishes.

## Why this version exists

Decision 2026-11-28 ([PRD 01](../prd/PRD_01_product.md#direction-2026-11-28-match-then-exceed-the-reference-clients)):
the command line matches the standard `git` spelling, and the private spelling
is removed rather than kept beside it. Today `minicon-srcmgr branch <worktree>
create <name>` cannot be reached by a script written for `git`, the worktree is
the first positional instead of `-C <path>` or the current directory, `history`
and `setting` are names `git` does not use, and `status` prints a private text
format. This version moves every verb to the Git spelling and removes the old
one. It comes before the `control` surface ([0.2.3](plan-0.2.3.md)) because
`control` reuses the option parsing and the trust lookup built here.

## Product outcome

**A script written for `git status --porcelain`, `git log` or `git branch -m`
runs against `minicon-srcmgr` unchanged apart from the binary name, and every
write still passes trust, the preview and the job record.**

```text
0.2.2 · one spelling, Git's
├── A. Invocation
│   ├── subcommand first; -C <path> selects the repository
│   ├── without -C, discovery from the current directory, as git does
│   └── -- ends options; a path that begins with - is still a path
├── B. Git names and formats
│   ├── status --porcelain[=v2] [-z], matching git's bytes for the fields it emits
│   ├── log replaces history; check-ignore replaces ignore; branch -m renames
│   └── exit codes: 0 success, 1 "nothing matched" where git uses 1, non-zero refusal
└── C. SrcMgr verbs, same rules
    ├── stage, unstage, hunk, draft, trust, config, gui, remote tracking
    └── trust refused → the message names the exact grant command
```

## Boundaries

**Must.** `-C <path>` and current-directory discovery for every repository verb;
`--porcelain=v2 -z` output byte-compatible with `git status --porcelain=v2 -z`
for ordinary, renamed, unmerged and untracked entries; `log` with `--skip` and
`--limit`; the old spelling rejected with a message that names the replacement;
every integration test moved, none deleted; a write that is refused leaves the
repository byte-identical.

**Not in 0.2.2.** Any new Git verb the product does not already perform
(`rebase`, `tag`, `cherry-pick`, `reset` are [0.2.4](plan-0.2.4.md)); full
`git log` option parity (`--author`, `--grep`, `--graph`); `status` long format;
`add -p`/`reset -p` interactive modes; aliases; a config file. The `control`
verbs are [0.2.3](plan-0.2.3.md).

**Excluded.** Keeping the private spelling as a hidden compatibility mode;
parsing `git`'s human-readable output as the compatibility oracle (the oracle is
real `git` on the same repository); changing what a verb does to the repository
in order to make its spelling match.

## Review of the starting point

| Area | Actual state at 0.2.1 | Consequence for 0.2.2 |
|---|---|---|
| Parser | One pass over `argv`; `--profile`, `--git`, `--json`, `-m` are lifted out anywhere; everything else is positional | Replaced by a per-verb parser that knows `-C`, `--` and its own options |
| Worktree | First positional on every repository verb | Becomes `-C <path>`, else current directory |
| Tests | 15 integration files, each with its own `cli()` helper; roughly 150 call sites use the private order | One shared helper; call sites rewritten mechanically, assertions kept |
| Scripts and CI | `candidate.yml` and `windows-candidate.yml` call `setting get history.limit` to prove the binary starts | Rewritten to `config get history.limit` |
| Docs | `first-run.txt` shows `clone <url> <destination>` (already Git's spelling); PRD 14 evidence lines quote the private spelling | `first-run.txt` unchanged; PRD 14 lines are historical evidence and stay as written |
| GUI backend | Talks to `srcmgr-git` directly; never shells out to the CLI | Untouched |

## Dependency graph

```text
C1 shared option parser (-C, --, per-verb options)  ── no behaviour change yet
      │
C2 discovery + trust lookup from -C / cwd
      │
C3 Git names and formats (status --porcelain=v2 -z, log, check-ignore, branch -m)
      │
C4 tests + scripts + usage moved to the new spelling; old spelling rejected
      ↓
   release 0.2.2 (courts inherited)
```

Merge points: C1 and C2 land together, because a parser without lookup is
untestable end to end. C3 is one commit per verb with its byte-comparison
fixture. C4 is one mechanical commit that touches no assertion.

## What must be true before the first line of code

- The compatibility oracle is real `git` on the same repository, run through
  `SRCMGR_TEST_GIT`, never a hand-written expected string.
- `--porcelain=v2 -z` is compared for the fields `git` emits; a field SrcMgr
  does not know yet is compared as "SrcMgr emits the same subset", and the test
  names which fields are not emitted.
- A path beginning with `-` is a path after `--`, and an option before it.
- An untrusted repository refuses with the exact grant command, including `-C`.
- Current-directory discovery does not run repository Git before trust is
  checked: metadata only, then the gate, then Git.

## Risks this plan accepts, and why

| Risk | Why it is accepted | What bounds it |
|---|---|---|
| Removing the old spelling breaks an existing caller | The owner chose removal over a compatibility mode | The old spelling names its replacement; release notes list every renamed verb |
| Byte compatibility with `git` is subtle (renames, unmerged, `-z`) | It is the point of the version | One fixture per entry kind, compared against real `git` |
| Current-directory discovery differs from `git` in edge cases (`.git` file, bare) | Today the CLI refuses both already | The existing refusal is kept and stated; no new claim |
| Mechanical test rewrite hides a behaviour change | Rewrites are error-prone | C4 changes call shapes only; assertions and fixtures are untouched and reviewed in the diff |

## Work packages

| Package | Deliverable | Minimum evidence |
|---|---|---|
| C1 · parser | `-C`, `--`, per-verb options; unknown option is a usage error naming the verb | Unit cases: `-C` before and after the verb, `--` then `-dash.txt`, unknown option |
| C2 · discovery | Repository from `-C` or the current directory; metadata then trust then Git | Fixture: run from inside the worktree with no path; untrusted from cwd refuses before Git runs |
| C3 · Git formats | `status --porcelain=v2 [-z]`, `log`, `check-ignore`, `branch -m`, `config` | Byte comparison against real `git` for each `status` entry kind; `log` order against `git log --format` |
| C4 · migration | Tests, two workflows, usage text; old spelling rejected | Workspace tests green with `SRCMGR_TEST_GIT`; CI workflows parse; old spelling returns usage with the new name |

## Invariants this increment must not weaken

- Trust is checked before any repository Git runs, whichever way the repository
  was found.
- A refused write leaves the repository byte-identical.
- Paths are literal, never patterns, whatever their spelling.
- The GUI backend keeps calling `srcmgr-git` directly.
- `--porcelain` output is derived from the same status read the window uses.

## Acceptance ledger

| Case | Required outcome | State |
|---|---|---|
| G1 · invocation | `-C <path>` and current-directory discovery work for every repository verb | `[v]` `the_repository_comes_from_c_or_the_current_directory` runs from the repository, from a subdirectory, from `-C`, from the attached `-C<path>` and from `-C ""`; `args` unit cases cover the global options |
| G2 · porcelain | `status --porcelain=v2 -z` matches real `git` byte for byte on the emitted fields | `[v]` `porcelain_v2_and_v1_are_byte_identical_to_git` and `porcelain_matches_git_through_a_rename_and_a_conflict` compare the product's bytes with `git`'s on the same repository, through a rename record; the product passes the command through rather than rebuilding it |
| G3 · names | `log`, `check-ignore`, `branch -m`, `config` replace the private names; the old spelling names its replacement | `[v]` `the_git_names_replace_the_private_ones_and_the_old_ones_say_so` and `branch_m_renames_through_git_and_reads_the_result_back`; `history`, `setting` and `ignore` exit 129 naming `log`, `config` and `check-ignore` |
| G4 · gate | An untrusted repository refuses before Git runs, from `-C` and from the current directory | `[v]` `an_untrusted_repository_refuses_before_git_runs_from_either_path` uses a Git that leaves a marker when executed, from the directory, a subdirectory and `-C`; the marker never appears |
| G5 · migration | Every integration test runs in the new spelling with its assertions intact; CI workflows pass | `[-]` 15 integration files moved by `scripts/migrate_cli_tests_022.py` plus three hand moves; all green with `SRCMGR_TEST_GIT`. The two CI workflows now call `config get`; their next run is the evidence |

## Selected verification and stopping

Cheapest oracle first: real `git` on the same fixture for output comparison, then
the moved integration tests, then the two CI workflows. Run the affected
`cargo test -p srcmgr-cli` selection with `SRCMGR_TEST_GIT`,
`cargo clippy --workspace --all-targets` and `python scripts/check_docs.py`.
Stop when the moved tests and the byte comparisons pass without an unexplained
difference.

## Round protocol

Same three artefacts as 0.2.0 and 0.2.1: the ledger row rewritten with the
evidence it has, an execution record entry, and
[the memory palace](../prd/prd_mempalace.md) updated with any fact that would
otherwise be re-learned.

## Qualification path and exit criteria

C6 and C7 are inherited with their runbooks unchanged. 0.2.2 closes in one of the
same three ways; if the courts still cannot run, it publishes as `0.2.2-beta`
naming them.

## Execution record

The first round built C1-C4 together; the record follows.

### 2026-11-28 · the spelling is measured, not remembered

- **Built.** `crates/srcmgr-cli/src/args.rs` parses one invocation into global
  options (`--profile`, `--git`, `-C`, before the verb only) and a per-verb
  `Command` with its own options; an option a verb does not know is refused by
  name. `main.rs` dispatches on it, every repository verb takes the repository
  from `-C` or the current directory, and `history`, `setting` and `ignore` are
  `log`, `config` and `check-ignore`. A usage error exits 129, as `git` does.
- **The oracle ran before the code.** Four rules were measured on `git` 2.47
  first and three corrected what was about to be written: `-C` is accepted
  before the subcommand only (`git status -C .` is an unknown switch), `-C ""`
  is accepted and changes nothing, and `--porcelain=v2 -z` carries no
  `# branch` header unless `--branch` is asked for. The smoke script then
  failed three times on its own `-C` placement, a bash redirect and the working
  directory; the product was right each time.
- **Porcelain is Git's bytes.** The product already reads
  `status --porcelain=v2 -z --branch --untracked-files=normal` for its own
  snapshot, so `--porcelain` passes that command's stdout through
  (`srcmgr_git::run_raw`) after the trust gate. Rebuilding the records from the
  coarser `StatusSnapshot` could not produce a rename's score and second
  operand; passing the bytes through makes compatibility constructive.
- **Discovery reads metadata, then the gate runs.** Without `-C` the repository
  is the first ancestor whose `.git` is a plain directory; nothing runs Git to
  find it, and what is found is a candidate that still goes through
  `open_trusted`.
- **`branch -m` is new behaviour.** `srcmgr_git::rename_branch` validates the
  new name, reads the old value, writes a `branch-rename` job record, renames
  through Git so the upstream follows, and reads the new value back; a taken
  destination is refused by Git before anything moves and the record is
  cleared.
- **Migration changed shapes only.** `scripts/migrate_cli_tests_022.py`
  rewrote the single-line calls; three were moved by hand: two multi-line
  calls, and one whose path literal holds a `]`. Its first run inserted `--`
  into `diff` and `hunk` calls and turned a trailing `--staged` and the `hunk`
  action word into paths; the diff review caught it before a test ran.
  `branch list` was accepted beside `--list` because the tests and the PRD
  contract both use it.
- **Evidence.** `cargo test -p srcmgr-cli --tests` with `SRCMGR_TEST_GIT`: 17
  suites green including the new `git_compat.rs` (8 cases).
  `cargo clippy --workspace --all-targets` clean. A 26-step smoke against real
  `git` passes.
- **Open.** The full-workspace run and the two CI workflows are the remaining
  evidence for G5; the release bump, notes and beta publication follow them.

# Changelog

## 0.0.1 - source-level first version (not published)

This is the first source version of MiniCon SrcMgr. It is tagged locally as `v0.0.1`. No binary is built for distribution, no signing or notarization is performed, and nothing is pushed to the public repository. The handoff excludes binary publication from this assignment, and PRD 12 requires signed, cross-platform evidence before any binary release.

### What works (Windows host, MinGit 2.56.0 through `SRCMGR_TEST_GIT`)

- Repository status with a trust gate: a metadata-only check runs before any Git command; grants cover the `.git` directory and a SHA-256 fingerprint of the config, attributes and hooks.
- Stage and unstage of literal paths, whole-hunk staging through `git apply --cached --recount`, and commit with the message passed on stdin.
- Ignore reporting through `git check-ignore -v`, with negated patterns treated as not ignored.
- Commit history from HEAD, paged with `--skip` and `--limit`, with a merge marker.
- Local branch list, create, and switch; the status window cycles branches with `B`.
- Commit-message drafts and an app setting `history.limit`, both in the SQLite profile store.
- A native status window built on MiniCon's platform host, font masks and pixel kernels.

### Known limits

- Git 2.43 is the release floor; the host's own Git 2.42 is refused by design.
- No graph lanes, per-parent diffs, scrollable diff view, branch picker, or window branch creation.
- Window interaction, layout, accessibility, IME, DPI and resource measurements are not evidenced.
- Linux and macOS cells are not run; invalid UTF-8 and newline file names need a Unix host.
- Crash injection and power-loss durability are not evidenced.
- Remote helpers, the full deny/grant matrix, and clean-machine or MSRV verification are open.

See [the implementation work order and evidence](prd/PRD_14_verification_roadmap.md#implementation-work-order-and-evidence) for the evidence behind each item.

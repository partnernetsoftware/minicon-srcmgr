# MiniCon SrcMgr

A lightweight, native source-control GUI for Windows, macOS and Linux on x86_64 and ARM64. Git comes first, with an explicit extension boundary for future SVN and other VCS backends. Built around reusable MiniCon mechanisms; scope and resource targets live in the [PRD](PRD.md).

**Current state: source version 0.0.1 (see [CHANGELOG](CHANGELOG.md)), not published as a binary. It contains the M0 foundation and the M1 local slice on Windows: status, stage/unstage, hunk staging, commit, ignore, history, branch switching, and a status window. Several milestone items remain open.** Build and test commands and their evidence are listed in the [work order](prd/PRD_14_verification_roadmap.md#implementation-work-order-and-evidence).

Start with [PRD.md](PRD.md) for product scope, capability ownership and delivery gates. The [PRD series](prd/) defines behavior and acceptance criteria. [AGENTS.md](AGENTS.md) defines repository working rules.

License: [MIT](LICENSE). MiniCon dependencies retain their original licenses and notices; see [reuse ownership](prd/PRD_02_architecture_reuse.md).

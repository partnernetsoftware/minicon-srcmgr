# MiniCon SrcMgr

A lightweight, native source-control GUI for Windows, macOS and Linux on x86_64 and ARM64. Git comes first, with an explicit extension boundary for future SVN and other VCS backends. Built around reusable MiniCon mechanisms; scope and resource targets live in the [PRD](PRD.md).

**Current state: requirements baseline plus an M0 foundation (Git runner, domain contracts, read-only `status` CLI). No GUI, trust store, mutation or release exists yet.** Build and test commands and their evidence are listed in the [handoff](plan/implementation-handoff.md).

Start with [PRD.md](PRD.md) for product scope, capability ownership and delivery gates. The [PRD series](prd/) defines behavior and acceptance criteria. [AGENTS.md](AGENTS.md) defines repository working rules.

License: [MIT](LICENSE). MiniCon dependencies retain their original licenses and notices; see [reuse ownership](prd/PRD_02_architecture_reuse.md).

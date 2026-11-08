# MiniCon SrcMgr

A lightweight, native source-control GUI for Windows, macOS and Linux on x86_64 and ARM64. Git comes first, with an explicit extension boundary for future SVN and other VCS backends. Built around reusable MiniCon mechanisms; scope and resource targets live in the [PRD](PRD.md).

**Latest published release: [0.1.0](https://github.com/partnernetsoftware/minicon-srcmgr/releases/tag/v0.1.0) — the first signed Windows x86_64 beta, with a runnable executable.** Download `minicon-srcmgr-0.1.0-windows-x86_64.zip`: the executable is Authenticode-signed by PARTNERNET SOFTWARE PTY LTD with a Microsoft RFC 3161 timestamp, and the archive carries a checksum list and a public signing receipt. It adds remote observation, clone with deferred checkout, reviewed remote configuration, upstream, fast-forward pull and current-branch push. Known limits are stated in the release notes: no in-window Clone/Remotes panel, no Chinese localisation, no merge/rebase or in-app conflict resolution, and no clean-image or five-participant study. See [0.1.0 notes](releases/0.1.0.md), [GitHub releases](https://github.com/partnernetsoftware/minicon-srcmgr/releases) and [delivery status](prd/PRD_12_platform_delivery.md#010-first-user-beta-delivery-target).

The [implementation plan](plan/plan-0.0.6.md) records actual local-workflow and resource evidence, including the repaired refresh/navigation bug. Windows Server developer-host results do not certify Windows 11, other platforms or the reference presentation target. Native CJK/clipboard/screen-reader/DPI, long-session and signed-package qualification remain open for first-user delivery.

The [0.0.x → 0.1.0 roadmap](plan/roadmap-0.1.0.md) allocates subsequent collaboration/recovery/history work. The qualified runnable Windows beta remains the 0.1.0 target; a source checkpoint does not complete that outcome.

## Try the Windows developer build

Use installed Git 2.43 or later, the pinned Rust toolchain in `rust-toolchain.toml` and the MSVC build tools. From this source checkout:

```powershell
cargo build --locked --release -p srcmgr-cli
.\target\release\minicon-srcmgr.exe gui
```

Welcome provides Open/Create and explicit repository trust. Git settings let you select and test an installed Git executable. Review the complete staged index before committing. These are developer build instructions, not a signed portable package or a cross-platform runtime-support claim.

Start with [PRD.md](PRD.md) for product scope, capability ownership and delivery gates. The [PRD series](prd/) defines behavior and acceptance criteria. [AGENTS.md](AGENTS.md) defines repository working rules.

License: [MIT](LICENSE). MiniCon dependencies retain their original licenses and notices; see [reuse ownership](prd/PRD_02_architecture_reuse.md).

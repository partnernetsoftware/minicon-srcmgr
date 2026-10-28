# MiniCon SrcMgr

A lightweight, native source-control GUI for Windows, macOS and Linux on x86_64 and ARM64. Git comes first, with an explicit extension boundary for future SVN and other VCS backends. Built around reusable MiniCon mechanisms; scope and resource targets live in the [PRD](PRD.md).

**Latest published release: [0.0.8](https://github.com/partnernetsoftware/minicon-srcmgr/releases/tag/v0.0.8) — a development source release, with no distributed executable.** It adds clone with deferred checkout and destination preflight, plus reviewed remote add/set-url/rename/remove with credential-free previews, through the Git layer and CLI. Remote observation from 0.0.7 remains available. See [0.0.8 notes](releases/0.0.8.md), [GitHub releases](https://github.com/partnernetsoftware/minicon-srcmgr/releases) and [delivery status](prd/PRD_12_platform_delivery.md#development-008-status).

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

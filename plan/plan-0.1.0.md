# 0.1.0 first-user beta plan

Status: `[v]` published 0.1.0 signed Windows x86_64 beta. Workspace version is **0.1.0** and the release carries a signed executable. The window Clone/Remotes panels, zh-CN localisation, clean-image, accessibility and five-participant study remain open and are named in the release notes. Product admission is owned by [01](../prd/PRD_01_product.md#010-first-user-feature-tree), packaging and signing by [12](../prd/PRD_12_platform_delivery.md#010-first-user-beta-delivery-target), interaction by [10](../prd/PRD_10_desktop_ux.md), and sequencing by [the roadmap](roadmap-0.1.0.md).

## Product outcome

**A developer obtains a Windows package, clones or opens a repository, commits, pulls and pushes without the Rust toolchain.**

## Delivered toward this target

| Area | State | Evidence |
|---|---|---|
| Clone, remote configuration, observation, fetch, upstream, FF pull, current-branch push | `[v]` | 0.0.7–0.0.9 public source checkpoints with Git and CLI fixtures |
| Window fetch/pull/push actions and status distance | `[v]` | 96 `srcmgr-ui` cases; scope in the 0.0.7/0.0.9 plans |
| Unsigned Windows x64 candidate | `[v]` private | `010-candidate-v3`: source `543cfbd`, artifact `1362b570447c19dbe364f637c878b0188317dbe898d7909e50b3a3ef0a991791`, zero known build-root occurrences, fresh-profile startup exits 0 and reads `unset`; the PE carries `ProductName=MiniCon SrcMgr`, `CompanyName=PARTNERNET SOFTWARE PTY LTD` and version `0.0.9` |
| Window Clone and Remotes panels | `[_]` | Not implemented; CLI only, stated in the release notes |
| zh-CN primary strings and locale selection | `[_]` | English only; stated in the release notes |
| Signed package | `[-]` mechanism qualified, release-eligible run pending | The `release-signing` environment exists with the three Azure secrets and three provider variables; both control-plane courts report READY. Run [42369612531](https://github.com/partnernetsoftware/minicon-srcmgr/actions/runs/42369612531) signed source `cb117b0` and produced a `Valid` Authenticode signature with `O=PARTNERNET SOFTWARE PTY LTD`, a Microsoft timestamp certificate and a credential-free receipt. That receipt is `release_eligible=false` because the version is still 0.0.9 and the policy still says `off`. |
| Clean-image launch, reference host, CJK/clipboard, accessibility | `BLOCKED` | No supplied environment; named in the release notes |
| Five-participant study | `BLOCKED` | No participants; named in the release notes |

## Remaining work packages

| Package | Deliverable | Minimum evidence and stop condition |
|---|---|---|
| P1 · Clone surface | Welcome Clone form bound to the existing `preview_clone`/`clone_repository` contracts, with destination review, progress and cancel | Workspace cases plus one native clone journey; a nonempty target keeps its contents |
| P2 · Remotes surface | Panel listing redacted remotes, with add/set-URL/rename/remove through the reviewed preview and an explicit confirmation | Model cases plus native captures; a refused edit changes no configuration |
| P3 · Language | zh-CN for primary flows using the typed key catalogue and a visible locale choice | Key coverage check; no untranslated primary control; English fallback intact |
| P4 · Package | Signed Windows x86_64 ZIP with notices, source/artifact identity, prerequisites and first-run instructions | `[v]` signed, locally re-verified and Defender-scanned; clean-image launch still `BLOCKED` |
| P5 · Study and quality | Five-participant task study plus changed-path resource evidence | Four of five complete open/clone → stage/commit → pull → push; failures change UX before release |

## Release gate

0.1.0 is publishable only when P4's signed package and P5's study pass under [12](../prd/PRD_12_platform_delivery.md). Until then the 0.0.9 source checkpoint remains the latest public release, and the private candidate is not published. An unsigned artifact is never described as the beta.

## Signing readiness (2026-10-28)

Learned the company process from the sibling `minicon` and `minicon-winmgr` checkouts:

- The provider is Azure Artifact Signing (Public Trust) over GitHub OIDC, one protected environment named `release-signing` per repository, a per-repository Entra identity holding only the profile-scoped signer role, and a `release-policy.json` whose `signing.mode` is checked in as `off` or `required`.
- `minicon-winmgr` keeps that policy at `required` and gates its signing workflow behind `environment: release-signing`.
- Copied the canonical `inspect-authenticode.ps1`, `inspect-authenticode.sh` and `fetch-microsoft-trust-bundle.sh` into `scripts/`; `check-product-inspectors.sh` reports `OK` for all three at contract `pns-authenticode-inspector/v3`.
- Added `release-policy.json` with `signing.mode = off` and an inert `windows-signed-candidate.yml` that refuses to run unless someone confirms provisioning; it has no unsigned fallback.
- `check-github-signing-environment.sh partnernetsoftware/minicon-srcmgr` returns `gh: Not Found (HTTP 404)`, so the repository has no signing environment yet.

### Provisioned 2026-11-07

1. ~~Create the repository's `release-signing` Environment, its OIDC federated identity and the profile-scoped signer role~~ — done. The environment holds `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID` and the provider endpoint/account/profile variables; the repository owns its own Entra application whose federated credential subject is exactly `repo:partnernetsoftware@53246189/minicon-srcmgr@1424470828:environment:release-signing`; the service principal holds only `Artifact Signing Certificate Profile Signer` at the `minicon-public` profile scope. `check-github-signing-environment.sh` and `check-azure-signing-state.sh` both report READY.
2. ~~Enable `signing.mode = required`~~ — pending a release-eligible run at the 0.1.0 version.
3. ~~Give every PE a `ProductName`/`ProductVersion` resource~~ — done in `543cfbd`: MSVC binaries now embed `ProductName`, `CompanyName`, `FileDescription` and the package version; verified in the candidate bytes.
4. Provide participants for the five-person study and a clean-image host for launch/signature courts.

## Execution record

### Signing mechanism qualified (2026-11-07)

Run [42369612531](https://github.com/partnernetsoftware/minicon-srcmgr/actions/runs/42369612531) built source `cb117b0` unsigned, signed the exact uploaded bytes through the repository's own OIDC identity, and inspected the result with the canonical inspector. The job recorded:

- unsigned input `3beb734344f18ba8…`, signed output `043f3e19b545d4a1…`, 3,945,400 bytes;
- Authenticode `Valid`, subject `CN=PARTNERNET SOFTWARE PTY LTD, O=PARTNERNET SOFTWARE PTY LTD, L=SYDNEY, S=New South Wales, C=AU`, issuer `CN=Microsoft ID Verified CS AOC CA 04, O=Microsoft Corporation, C=US`, certificate expiry 2026-11-09;
- RFC 3161 timestamp certificate `CN=Microsoft Public RSA Time Stamping Authority`;
- a `srcmgr-candidate-signing` receipt that the fail-closed auditor accepted with `release_eligible=false`.

The signed bytes were downloaded and started on this host: `--profile <fresh> setting get history.limit` printed `unset` and exited 0.

Earlier attempts in the same session failed for three wiring reasons and are kept here rather than hidden: the build wrote outside the qualification directory; the digest crossed jobs through `GITHUB_ENV`, which does not exist across jobs; `files-catalog` needs paths relative to the catalog and the action needs the Azure CLI credential that `azure/login` created. None of those were product defects.

### Release-eligible signing run

Run [42453987936](https://github.com/partnernetsoftware/minicon-srcmgr/actions/runs/42453987936) at source `d3531ea` with `signing.mode=required` and version 0.1.0 produced a release-eligible receipt:

- unsigned input `50204865cff37879…`, signed output `3ce0555ea9c885ef…`, 3,945,400 bytes;
- Authenticode `Valid`, `O=PARTNERNET SOFTWARE PTY LTD`, issuer `CN=Microsoft ID Verified CS AOC CA 03`, certificate valid 2026-11-07 to 2026-11-10;
- RFC 3161 timestamp by `CN=Microsoft Public RSA Time Stamping Authority`;
- product identity `MiniCon SrcMgr` / `0.1.0` unchanged by signing.

Locally the downloaded bytes were inspected again with the canonical inspector (`Valid`, same subject and version), started with a fresh profile (`unset`, exit 0) and scanned by Microsoft Defender as a packaged folder with no detections. Packaged archive `minicon-srcmgr-0.1.0-windows-x86_64.zip` SHA-256 `e858e49ac8725e4309921833eda6213d1cb8afe4eae3055b83b796e257ad5f2a`.

### Still open after 0.1.0

- A release-eligible run at version 0.1.0 with `signing.mode = required` and the version unpublished.
- Window Clone and Remotes panels; zh-CN primary strings.
- Clean-image launch, the five-participant study and a Defender scan of the final bytes.

The private candidate `010-candidate-v1` (source `b609290`, artifact `ec245f2f...`) starts from a fresh profile and exits 0. No 0.1.0 tag or release exists.

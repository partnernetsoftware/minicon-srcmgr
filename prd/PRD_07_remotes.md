# 07 · Remotes and collaboration

Parent: [PRD index](../PRD.md). Owns remote configuration and network Git workflows. [11](PRD_11_security.md) owns secret storage, trust and executable-content policy.

## Outcome tree

```text
Outcome: exchange exactly the intended refs with an explicit remote
├── Configure — remote names, fetch/push URLs and refspecs
├── Fetch — remote observations with freshness and prune preview
├── Integrate — explicit pull merge/rebase/fast-forward choice
└── Publish — branch/tag selection, upstream and lease-aware rewrite
```

```mermaid
flowchart LR
    C["Remote + refspec selection"] --> A["Trust / credential interaction"]
    A --> F["Fetch"] --> I["Inspect divergence"]
    I --> M["Explicit pull integration"] --> R["08 Recovery"]
    I --> P["Push preview"] --> L["Expected refs / lease"] --> V["Per-ref result"]
```

## Remote model

Support multiple named remotes, distinct fetch/push URLs and visible refspecs. Ordinary forms cover HTTPS, SSH including scp-like syntax, and deliberate local-path remotes. Git/ext helpers and unknown transports are quarantined by [11](PRD_11_security.md). URL labels redact credentials. Edit, rename or remove remote configuration previews the local configuration change and never removes a server repository. Custom refspecs are an advanced explicit form, not silently replaced by a friendly branch selector.

Clone uses this network/authentication model; destination handling belongs to [03](PRD_03_workspace.md). Show selected Git executable and transport diagnostics without exposing secrets. Reuse the user's configured credential helper/SSH agent only after trust. Askpass is a private per-job interaction with masked entry, cancel and exact endpoint identity. SSH host-key or HTTPS certificate failure is an actionable error; do not auto-accept host keys or disable TLS verification. Supporting an external SSH trust flow must keep that decision visible and user initiated.

Fetch defaults to the selected remote; Fetch all is a separate action. Show transfer progress when available, indeterminate otherwise, and cancellation. Prune defaults off; when enabled show local remote-tracking refs eligible for deletion, then reconcile actual deletions. Background auto-fetch defaults off, can be enabled per repository with interval and metered/offline suspension, and never opens surprise credential prompts; mark authentication required for the next foreground action. Ahead/behind counts are relative to fetched tracking refs, with timestamp.

Pull is a named two-phase flow: fetch, inspect divergence, then fast-forward-only (default), merge or rebase selected explicitly. Do not inherit a hidden integration choice from ambient `pull.*` settings without showing the effective mode. Dirty changes, unborn branch and missing upstream are handled before integration. If fetch succeeds and integration fails, report both outcomes; do not describe it as all-or-nothing.

Push preview shows destination URL label, local source OID/ref, remote destination ref, creation/deletion, upstream intent and expected non-fast-forward behavior. Default is current branch, not every ref. Missing upstream offers an explicit mapping. Multi-ref push displays each result and partial success; atomic push is optional only when supported. Remote branch deletion is a separate risk action.

Construct explicit reviewed source/destination refspecs; do not let ambient `push.default`, `remote.*.push`, mirror or follow-tags settings add unpreviewed refs. Recovery refs from 08 are never selected automatically. If effective remote settings cannot be overridden safely to match the preview, block and explain rather than execute a broader push. Use noninteractive machine-readable per-ref results and reconcile uncertain remote state; Git/helper progress is not itself proof a ref was updated.

Ordinary force push is excluded. A deliberate rewrite uses `--force-with-lease` with an explicit expected destination OID recorded at the reviewed fetch; background fetch cannot silently update that expectation. On lease mismatch, fetch/review/reconfirm; never retry with bare force. Lease protects against that ref moving, not branch policy or all server-side effects. Server protections/hooks/permissions surface as remote rejection. Cancellation or lost connection may leave remote completion unknown: mark Unknown, query the destination and reconcile before retry. A timed-out push is never automatically replayed.

## Candidate object-storage remotes

Status: future candidate, not implemented, not selected for 0.0.6 and assigned no release. The initial product role is a **Git remote transport/storage option**, not a new VCS backend. Reuse the installed Git and its reviewed remote helper through the existing supervised worker; no embedded S3 SDK, always-running service or automatic helper download is admitted by this candidate. [Git's remote-helper protocol](https://git-scm.com/docs/gitremote-helpers) defines the extension mechanism; actual compatibility still requires a pinned helper and execution evidence.

Candidate selection must identify the helper implementation/version and remote storage format as well as bucket/prefix, endpoint, region and profile. Sharing an `s3://` URL shape does not establish interchangeable object layouts. Do not probe a prefix with an unrelated helper or initialize/overwrite an unrecognized remote format. A setup/connectivity check must state its network effects and remain separate from first publication. UI capability states must distinguish unsupported operations, missing prerequisites, access denial and unknown outcome. Helper capabilities are an input, not independent proof of atomic pushes or safe rewrites.

Credentials remain with a trusted external provider/profile; the workspace stores nonsecret configuration only. Review actual credential sources, including any network-based discovery, under [11](PRD_11_security.md). A remote URL alone cannot authorize execution of a discovered `git-remote-s3` binary; identify and approve the executable and relevant configuration, scope protocol permission to the approved job, and do not weaken global Git settings. A missing/disabled integration leaves all local Git workflows usable.

Before admitting a provider, qualify ordinary clone/fetch/explicit-ref push; competing writers to one ref; timeout/cancel after possible publication; actual-result reconciliation; access/expired-credential failure; and bounded memory/CPU/temp-storage behavior. Use a disposable object-store prefix and two clients; object-store compatibility or helper documentation alone is not a PASS. Report the complete owned helper process tree. Rewrites, multi-ref atomicity, deletion, submodules and LFS are individually qualified capabilities, not inferred from basic clone/push. Server-side protection/review/backup guarantees must not be inferred from bucket storage or client locks. Do not automatically replay an uncertain push or run remote maintenance/garbage collection.

This section owns provider admission behavior. [15](PRD_15_vcs_backends.md#vcs-versus-remote-transport) owns the VCS distinction; product scheduling belongs to [01](PRD_01_product.md). Provider selection and implementation require a later scoped plan; neither current version plan expands as a side effect.

## Version split 0.0.7–0.1.0

0.0.7 delivers clone (checkout deferred until filter approval), remote configuration, explicit fetch with progress and cancel, ahead/behind with fetch time, and remote-tracking listing. Pull, push, upstream writes and authentication qualification follow in 0.0.8. Pull is one named action that fetches first and integrates fast-forward only; divergence stops with a review. Merge, rebase, conflict resolution and lease-protected rewrites remain unallocated after 0.1.0. Authenticated HTTPS/SSH paths stay BLOCKED until a controlled fixture host exists; a local-path remote is never counted as authentication evidence.

## Delivery contracts

| ID / leaf | Invariant | Observable evidence | Safe failure | Dependencies | Non-goal |
|---|---|---|---|---|---|
| REMOTE-01 · auth | Auth prompts bind current job and endpoint without leaking secrets | QA-J02/J10 helper/agent/askpass and cancellation | Auth required/cancelled; no credential fallback | 02, 11 | Built-in hosting accounts |
| REMOTE-02 · fetch/pull | Observation and integration outcomes remain distinct | QA-J02/J03 diverged refs and failed merge after fetch | Preserve fetched facts; offer explicit continuation | 06, 08 | Silent pull strategy |
| REMOTE-03 · push | Only confirmed destination refs are attempted | QA-J02 per-ref partial success and policy rejection | Report per-ref actual/unknown outcome | 08, 13 | Push every ref by default |
| REMOTE-04 · rewrite | Explicit lease uses the reviewed expected OID | QA-J02/J05 two-client stale lease race | Reject and require new review | 06, 11 | Bare force push |

## Acceptance details

Use two independent clones and a local bare remote for deterministic collaboration tests; controlled HTTPS/SSH fixtures cover transport/auth separately. Exercise successful push, competing update, server rejection, lost response after server acceptance, remote deletion and fetch prune. Diagnostics must distinguish DNS/connectivity, authentication, host trust, ref conflict and server policy without printing the credential-bearing input.

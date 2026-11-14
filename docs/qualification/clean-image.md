# Clean-image court (C6)

The gate: **the published archive launches and completes first-run on an image
that never had the product or the Rust toolchain.** This file is the runbook for
producing that evidence, and the receipt shape that proves it. The plan's exit
criteria decide what happens if the court does not run.

## What "clean" means

An image qualifies when all three hold before the archive is extracted:

1. SrcMgr was never installed: no `minicon-srcmgr` on `PATH`, no profile
   directory under the user's data directory, no start-menu or desktop entry.
2. No Rust toolchain: `rustc --version` and `cargo --version` both fail with
   "command not found". `~/.cargo` and `~/.rustup` do not exist.
3. No build outputs from this repository: no `target/` directory and no copied
   `minicon-srcmgr` binary anywhere on `PATH`.

Git is **not** part of "clean". The product does not bundle Git and its
documented requirement is an installed Git, so the operator installs Git and
nothing else, and records its version.

## Machine

- A fresh virtual machine, or a machine that can be re-imaged afterwards.
- Windows 11 24H2 x86_64, or Ubuntu 24.04 x86_64 on a physical machine or VM.
- Take a snapshot before step 1 so the image can be reused for a later version.
- One cell per receipt. A clean image proves nothing about another cell.

## Steps

1. Download the archive and `SHA256SUMS.txt` from the release page.
2. Verify the archive digest. Windows: `certutil -hashfile <archive> SHA256`.
   Linux and macOS: `sha256sum -c SHA256SUMS.txt`. A mismatch stops here.
3. Install Git and record `git --version`. Install nothing else.
4. Run the three cleanliness checks above and record their output.
5. Extract the archive into an empty writable folder.
6. Launch the binary from that folder.
7. Complete first-run, in this order, capturing a screenshot at each step:
   Welcome → Create (or Open) → trust review → stage a file → commit → confirm
   the commit appears in the history list.
8. On macOS only, run the documented quarantine step first and record it.
9. Fill the receipt below and store it with the release evidence.

## Receipt

```json
{
  "schema": 1,
  "kind": "srcmgr-clean-image-court",
  "status": "passed | failed",
  "cell": "win-x86_64",
  "product_version": "0.1.3",
  "archive": { "name": "minicon-srcmgr-0.1.3-win-x86_64.zip", "sha256": "<as verified>" },
  "image": { "os": "Windows 11 24H2", "source": "fresh VM snapshot", "git": "2.47.1" },
  "cleanliness": {
    "product_absent": true,
    "rustc_absent": true,
    "cargo_absent": true,
    "build_output_absent": true
  },
  "first_run": {
    "launched": true,
    "welcome_seen": true,
    "repository_opened_or_created": true,
    "trust_review_completed": true,
    "commit_created": true,
    "notes": "any refusal or surprise, in the operator's words"
  },
  "operator": "<role, not a personal account>",
  "recorded_at": "2026-11-14T00:00:00Z",
  "artifact_dir": "plan/evidence/clean-image/<cell>/"
}
```

A receipt with any `false` in `cleanliness` is not a clean-image court; label it
`image-not-clean` and run it on another machine. A receipt with any `false` in
`first_run` is a **failed** court, and the failing step becomes the first item of
the next plan.

## What this court does not prove

- Any cell other than the one in the receipt.
- Signing or notarization (that is 12's signature gate).
- Accessibility, CJK input, or long-session stability.
- A machine that differs from the recorded OS build.

"""Run one cell's runtime court against its own built executable.

The court proves the artifact starts and performs real Git reads on the cell it
was built for. It never claims a GUI journey: opening a window needs a display
and, on Linux, a compositor, so that is a separate court.

Everything the court asserts comes from an independent oracle: the fixture's
own Git output and the expected product version. The product's own report is
never the only evidence.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


def run(command: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(command, cwd=cwd, capture_output=True, text=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    binary = args.binary.resolve()
    checks: list[dict] = []
    failed = False

    def record(name: str, ok: bool, detail: str) -> None:
        nonlocal failed
        checks.append({"check": name, "ok": ok, "detail": detail[:400]})
        if not ok:
            failed = True

    version = run([str(binary), "--version"])
    record(
        "version",
        version.returncode == 0 and version.stdout.strip() == f"minicon-srcmgr {args.expected_version}",
        f"exit={version.returncode} out={version.stdout.strip()} err={version.stderr.strip()}",
    )

    git = os.environ.get("GIT_FOR_COURT", "git")
    git_version = run([git, "--version"])
    record("git_present", git_version.returncode == 0, git_version.stdout.strip() or git_version.stderr.strip())

    with tempfile.TemporaryDirectory(prefix="srcmgr-court-") as scratch:
        root = Path(scratch)
        worktree = root / "repo"
        worktree.mkdir()
        for command in (
            [git, "init", "-q", "-b", "main", "."],
            [git, "config", "user.email", "court@example.test"],
            [git, "config", "user.name", "Court"],
        ):
            done = run(command, cwd=worktree)
            if done.returncode != 0:
                record("fixture", False, f"{command}: {done.stderr.strip()}")
                break
        (worktree / "court.txt").write_text("one\n", encoding="utf-8")
        for command in ([git, "add", "court.txt"], [git, "commit", "-qm", "court"]):
            done = run(command, cwd=worktree)
            if done.returncode != 0:
                record("fixture", False, f"{command}: {done.stderr.strip()}")
                break
        head = run([git, "rev-parse", "HEAD"], cwd=worktree).stdout.strip()

        profile = root / "profile"
        granted = run([str(binary), "--profile", str(profile), "--git", git, "trust", "grant", str(worktree)])
        record("trust_grant", granted.returncode == 0, f"exit={granted.returncode} err={granted.stderr.strip()}")

        status = run([str(binary), "--profile", str(profile), "--git", git, "status", str(worktree), "--json"])
        body = status.stdout
        record(
            "status_json",
            status.returncode == 0 and head[:12] in body and '"main"' in body,
            f"exit={status.returncode} err={status.stderr.strip()} out={body[:200]}",
        )

        remotes = run([str(binary), "--profile", str(profile), "--git", git, "remote", str(worktree), "list", "--json"])
        record(
            "remote_list",
            remotes.returncode == 0 and remotes.stdout.strip().startswith("["),
            f"exit={remotes.returncode} out={remotes.stdout.strip()[:120]} err={remotes.stderr.strip()}",
        )

        # A repository without an upstream must report that, not invent one.
        tracking = run([str(binary), "--profile", str(profile), "--git", git, "remote", str(worktree), "tracking", "--json"])
        record(
            "tracking_honest",
            tracking.returncode == 0 and '"upstream": null' in tracking.stdout,
            f"exit={tracking.returncode} out={tracking.stdout.strip()[:160]} err={tracking.stderr.strip()}",
        )

    receipt = {
        "schema": 1,
        "kind": "srcmgr-cell-runtime",
        "status": "failed" if failed else "passed",
        "platform": sys.platform,
        "machine": os.uname().machine if hasattr(os, "uname") else os.environ.get("PROCESSOR_ARCHITECTURE", ""),
        "checks": checks,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    for check in checks:
        print(("PASS " if check["ok"] else "FAIL ") + check["check"] + " :: " + check["detail"])
    return 0 if not failed else 1


if __name__ == "__main__":
    raise SystemExit(main())

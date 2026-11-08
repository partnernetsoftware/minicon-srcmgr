"""Require the exact six-cell set and write the aggregate receipt.

A missing cell, a failed runtime court, or a receipt that names another source
is a failure. A partial set never becomes a release candidate.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

EXPECTED = {
    "lnx-x86_64": "linux",
    "lnx-aarch64": "linux",
    "win-x86_64": "windows",
    "win-aarch64": "windows",
    "osx-x86_64": "macos",
    "osx-aarch64": "macos",
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cells", required=True, type=Path)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    problems: list[str] = []
    receipts: dict[str, dict] = {}
    for cell in sorted(EXPECTED):
        # Each artifact downloads into a directory named after itself. The
        # archiver decides how deep the files sit inside it, so look them up
        # rather than assuming a depth: a layout change must not turn a
        # passing set into a silent refusal, or the reverse.
        root = args.cells / f"cell-{cell}"
        found_archive = sorted(root.rglob("cell-receipt.json")) if root.is_dir() else []
        found_runtime = sorted(root.rglob("runtime-receipt.json")) if root.is_dir() else []
        if len(found_archive) != 1:
            problems.append(f"{cell}: expected one cell receipt, found {len(found_archive)}")
            continue
        if len(found_runtime) != 1:
            problems.append(f"{cell}: expected one runtime receipt, found {len(found_runtime)}")
            continue
        archive, runtime = found_archive[0], found_runtime[0]
        cell_receipt = json.loads(archive.read_text(encoding="utf-8"))
        ran = json.loads(runtime.read_text(encoding="utf-8"))
        if ran.get("status") != "passed":
            problems.append(f"{cell}: runtime court {ran.get('status')}")
        if cell_receipt.get("cell") != cell:
            problems.append(f"{cell}: receipt names {cell_receipt.get('cell')}")
        if cell_receipt.get("os") != EXPECTED[cell]:
            problems.append(f"{cell}: receipt os {cell_receipt.get('os')}")
        if cell_receipt.get("source_sha") != args.source_sha:
            problems.append(f"{cell}: source {cell_receipt.get('source_sha')}")
        for name, asset in (cell_receipt.get("assets") or {}).items():
            if not isinstance(asset.get("sha256"), str) or len(asset["sha256"]) != 64:
                problems.append(f"{cell}: {name} lacks a sha256")
        receipts[cell] = cell_receipt

    if problems:
        for line in problems:
            print(line, file=sys.stderr)
        return 1

    summary = {
        "schema": 1,
        "kind": "srcmgr-six-cell",
        "source_sha": args.source_sha,
        "cell_count": len(receipts),
        "windows_signing": "blocked" if all(
            receipts[c]["signing"] == "blocked" for c in ("win-x86_64", "win-aarch64")
        ) else "present",
        "cells": {
            cell: {
                "os": receipt["os"],
                "arch": receipt["arch"],
                "runtime_court": receipt["runtime_court"],
                "signing": receipt["signing"],
                "assets": receipt["assets"],
            }
            for cell, receipt in receipts.items()
        },
    }
    args.out.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"{len(receipts)} cells passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

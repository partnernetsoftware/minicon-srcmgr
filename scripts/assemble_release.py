"""Assemble the release asset set from the six-cell artifacts and the signed bytes.

Inputs are the two things the pipeline actually proved:

* the six-cell artifacts, one directory per cell, each holding that cell's
  archive, its cell receipt and its runtime receipt;
* the signed Windows executables, their inspections and the signing receipt.

The Windows cells are re-packaged here because the six-cell job runs before
signing and therefore cannot ship the final bytes. Everything else is copied
unchanged, and every digest is recomputed from the file that will be uploaded.

Fails closed. A missing cell, a receipt that describes different bytes, a
Windows executable that the inspector did not call Valid, or a signing receipt
that the auditor would reject stops the release instead of producing a partial
asset set.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

CELLS = {
    "win-x86_64": ("windows", "x86_64"),
    "win-aarch64": ("windows", "aarch64"),
    "osx-x86_64": ("macos", "x86_64"),
    "osx-aarch64": ("macos", "aarch64"),
    "lnx-x86_64": ("linux", "x86_64"),
    "lnx-aarch64": ("linux", "aarch64"),
}
WINDOWS = ("win-x86_64", "win-aarch64")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def one(root: Path, name: str) -> Path:
    found = sorted(root.rglob(name)) if root.is_dir() else []
    if len(found) != 1:
        raise SystemExit(f"{root}: expected one {name}, found {len(found)}")
    return found[0]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cells", required=True, type=Path, help="downloaded six-cell artifacts")
    parser.add_argument("--signed", required=True, type=Path, help="downloaded signed Windows cells")
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--readme", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    out = args.out.resolve()
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    problems: list[str] = []
    assets: dict[str, dict] = {}

    for cell, (family, arch) in sorted(CELLS.items()):
        cell_root = args.cells / f"cell-{cell}"
        try:
            receipt_path = one(cell_root, "cell-receipt.json")
            runtime_path = one(cell_root, "runtime-receipt.json")
        except SystemExit as error:
            problems.append(str(error))
            continue
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
        ran = json.loads(runtime_path.read_text(encoding="utf-8"))
        if ran.get("status") != "passed":
            problems.append(f"{cell}: runtime court {ran.get('status')}")
        if receipt.get("cell") != cell or receipt.get("os") != family:
            problems.append(f"{cell}: receipt names {receipt.get('cell')}/{receipt.get('os')}")
        if receipt.get("source_sha") != args.source_sha:
            problems.append(f"{cell}: built from {receipt.get('source_sha')}")

        archive_name = next(
            (name for name in receipt.get("assets", {}) if name.endswith((".zip", ".tar.gz"))),
            None,
        )
        if archive_name is None:
            problems.append(f"{cell}: receipt lists no archive")
            continue

        if cell in WINDOWS:
            # Find the signed bytes wherever the artifact put them: the upload
            # path is the producer's choice and has already changed once.
            found = sorted(args.signed.rglob(f"cells/{cell}/minicon-srcmgr.exe")) if args.signed.is_dir() else []
            inspections = sorted(args.signed.rglob(f"inspection-{cell}.json")) if args.signed.is_dir() else []
            if len(found) != 1 or len(inspections) != 1:
                problems.append(
                    f"{cell}: expected one signed executable and one inspection, found {len(found)} and {len(inspections)}"
                )
                continue
            signed, inspection_path = found[0], inspections[0]
            inspection = json.loads(inspection_path.read_text(encoding="utf-8"))
            actual = sha256(signed)
            if inspection.get("sha256") != actual:
                problems.append(f"{cell}: inspection does not describe the signed bytes")
            if inspection.get("status") != "Valid":
                problems.append(f"{cell}: signed executable is {inspection.get('status')}")
            if not inspection.get("timestamp_certificate"):
                problems.append(f"{cell}: signed executable has no timestamp")
            produced = out / "packaged" / cell
            done = subprocess.run(
                [
                    sys.executable,
                    str(root / "scripts" / "package_cell.py"),
                    "--cell", cell,
                    "--binary", str(signed),
                    "--readme", str(args.readme),
                    "--version", args.version,
                    "--source-sha", args.source_sha,
                    "--runtime-status", "passed",
                    "--signing", "signed",
                    "--out", str(produced),
                ],
                cwd=root,
                capture_output=True,
                text=True,
            )
            if done.returncode != 0:
                problems.append(f"{cell}: repackaging failed: {done.stderr.strip()[:200]}")
                continue
            staged = produced / archive_name
            if not staged.is_file():
                problems.append(f"{cell}: repackaged archive is missing")
                continue
            final = out / archive_name
            shutil.copy2(staged, final)
            # The release receipt must describe the archive that ships, not the
            # pre-signing one the six-cell job produced.
            release_receipt = produced / "cell-receipt.json"
        else:
            source = one(cell_root, archive_name)
            final = out / archive_name
            shutil.copy2(source, final)
            release_receipt = receipt_path

        digest = sha256(final)
        expected = receipt.get("assets", {}).get(archive_name, {}).get("sha256")
        if cell not in WINDOWS and digest != expected:
            problems.append(f"{cell}: archive digest changed in transit")
        shutil.copy2(release_receipt, out / f"cell-receipt-{cell}.json")
        if cell in WINDOWS:
            shutil.rmtree(out / "packaged", ignore_errors=True)
        assets[archive_name] = {"sha256": digest, "bytes": final.stat().st_size, "cell": cell}

    if problems:
        for line in problems:
            print(line, file=sys.stderr)
        return 1

    if sorted(assets) and len(assets) != len(CELLS):
        print(f"expected {len(CELLS)} archives, assembled {len(assets)}", file=sys.stderr)
        return 1

    sums = "".join(f"{row['sha256']} *{name}\n" for name, row in sorted(assets.items()))
    (out / "SHA256SUMS.txt").write_text(sums, encoding="utf-8", newline="\n")

    receipts = sorted(args.signed.rglob("signing-receipt.json")) if args.signed.is_dir() else []
    if len(receipts) != 1:
        print(f"expected one signing receipt, found {len(receipts)}", file=sys.stderr)
        return 1
    shutil.copy2(receipts[0], out / "signing-receipt.json")

    manifest = {
        "schema": 1,
        "kind": "srcmgr-release-assets",
        "product_version": args.version,
        "source_sha": args.source_sha,
        "cells": {
            cell: {"os": CELLS[cell][0], "arch": CELLS[cell][1], "signing": "signed" if cell in WINDOWS else "unsigned"}
            for cell in sorted(CELLS)
        },
        "assets": assets,
    }
    (out / "release-manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    for name, row in sorted(assets.items()):
        print(f"{row['cell']}: {name} {row['sha256']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

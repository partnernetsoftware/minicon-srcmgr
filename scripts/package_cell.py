"""Assemble one cell's archive and a per-cell receipt.

The archive is the only thing a user downloads, so the receipt records exactly
what went into it and what was verified on the cell's own machine. No provider
coordinate, no local path and no build-root string enters either file.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import shutil
import tarfile
import zipfile
from pathlib import Path

CELLS = {
    "win-x86_64": "windows",
    "win-aarch64": "windows",
    "osx-x86_64": "macos",
    "osx-aarch64": "macos",
    "lnx-x86_64": "linux",
    "lnx-aarch64": "linux",
}

NOTICES = {
    "windows": "minicon-srcmgr.exe",
    "macos": "minicon-srcmgr",
    "linux": "minicon-srcmgr",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cell", required=True, choices=sorted(CELLS))
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--readme", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--runtime-status", required=True, choices=("passed", "failed"))
    parser.add_argument("--signing", required=True, choices=("signed", "unsigned", "blocked"))
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    family = CELLS[args.cell]
    if not args.binary.is_file():
        raise SystemExit(f"binary is missing: {args.binary}")
    after = sha256(args.binary)

    stage = args.out / "stage"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    name = NOTICES[family]
    shutil.copy2(args.binary, stage / name)
    shutil.copy2(args.readme, stage / "README.txt")
    (stage / "SHA256SUMS.txt").write_text(
        f"{after} *{name}\n", encoding="utf-8", newline="\n"
    )

    stem = f"minicon-srcmgr-{args.version}-{args.cell}"
    if family == "windows":
        archive = args.out / f"{stem}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
            for item in sorted(stage.iterdir()):
                bundle.write(item, item.name)
    else:
        archive = args.out / f"{stem}.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            for item in sorted(stage.iterdir()):
                info = bundle.gettarinfo(str(item), item.name)
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mode = 0o755 if item.name == name else 0o644
                with item.open("rb") as handle:
                    bundle.addfile(info, handle)

    receipt = {
        "schema": 1,
        "kind": "srcmgr-cell-archive",
        "cell": args.cell,
        "os": family,
        "arch": "x86_64" if "x86_64" in args.cell else "aarch64",
        "product_version": args.version,
        "source_sha": args.source_sha,
        "runner": {"os": platform.system(), "machine": platform.machine()},
        "runtime_court": args.runtime_status,
        "signing": args.signing,
        "assets": {
            name: {"sha256": after, "bytes": args.binary.stat().st_size},
            archive.name: {"sha256": sha256(archive), "bytes": archive.stat().st_size},
        },
    }
    (args.out / "cell-receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(f"{args.cell}: {archive.name} {receipt['assets'][archive.name]['sha256']}")


if __name__ == "__main__":
    main()

"""Compose the public signing receipt from canonical inspector output.

The receipt is the only public record of a signed asset set. Provider
coordinates (endpoint, account, profile, tenant, subscription, application or
principal identifiers) never enter it, and
scripts/audit-public-signing-receipt.py fails closed when any of them appear.

Input is a manifest so one signing run can cover several files without the
command line becoming a positional puzzle:

    {
      "version": "0.1.0",
      "run_id": 123, "run_attempt": 1,
      "assets": [
        {"cell": "win-x86_64", "path": "...", "relative_path": "cells/win-x86_64/minicon-srcmgr.exe",
         "inspection": "inspection-win-x86_64.json", "before_sha256": "..."}
      ]
    }
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

PROVIDER = "azure-artifact-signing"
PUBLISHER = "PARTNERNET SOFTWARE PTY LTD"
TIMESTAMP_URL = "http://timestamp.acs.microsoft.com"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_text(source: dict, key: str, where: str) -> str:
    value = source.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{where}: inspection is missing {key}")
    return value


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--release-eligible", required=True, choices=("true", "false"))
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    manifest = json.loads(args.manifest.read_text(encoding="utf-8-sig"))
    entries = manifest.get("assets") or []
    if not entries:
        raise ValueError("manifest lists no assets")
    eligible = args.release_eligible == "true"
    assets: dict[str, dict] = {}

    for entry in entries:
        cell = entry["cell"]
        asset = Path(entry["path"])
        inspection = json.loads(Path(entry["inspection"]).read_text(encoding="utf-8-sig"))
        after = sha256(asset)
        if inspection.get("sha256") != after:
            raise ValueError(f"{cell}: inspection does not describe the signed bytes")
        signer = inspection.get("signer") or {}
        timestamp = inspection.get("timestamp_certificate") or {}
        subject = signer.get("subject") or ""
        if f"O={PUBLISHER}" not in subject:
            raise ValueError(f"{cell}: signer subject does not carry the company organization")
        if not timestamp:
            raise ValueError(f"{cell}: a timestamp certificate is required")
        status = require_text(inspection, "status", cell)
        if eligible and status != "Valid":
            raise ValueError(f"{cell}: a release-eligible receipt requires a Valid signature")
        assets[cell] = {
            "path": entry["relative_path"],
            "before_sha256": entry["before_sha256"],
            "after_sha256": after,
            "after_bytes": int(inspection.get("size_bytes") or 0),
            "authenticode_status": status,
            "signer_subject": subject,
            "signer_issuer": require_text(signer, "issuer", cell),
            "signer_thumbprint": require_text(signer, "thumbprint", cell),
            "signer_not_before": require_text(signer, "not_before", cell),
            "signer_not_after": require_text(signer, "not_after", cell),
            "timestamp_subject": require_text(timestamp, "subject", cell),
            "timestamp_issuer": require_text(timestamp, "issuer", cell),
        }

    receipt = {
        "schema": 1,
        "kind": "srcmgr-candidate-signing",
        "source_sha": args.source_sha,
        "product_version": manifest["version"],
        "signing_provider": PROVIDER,
        "publisher_organization": PUBLISHER,
        "file_digest": "SHA256",
        "timestamp_rfc3161": TIMESTAMP_URL,
        "timestamp_digest": "SHA256",
        "release_eligible": eligible,
        "signing_run": {"id": int(manifest["run_id"]), "attempt": int(manifest["run_attempt"])},
        "assets": assets,
    }
    args.out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for cell, row in sorted(assets.items()):
        print(f"{cell}: {row['path']} {row['after_sha256']}")


if __name__ == "__main__":
    main()

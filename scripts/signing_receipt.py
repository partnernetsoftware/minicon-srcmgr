"""Compose the public signing receipt from inspector output.

The receipt is the only public record of a signed candidate. Provider
coordinates (endpoint, account, profile, tenant, subscription, application and
principal identifiers) never enter it; scripts/audit-public-signing-receipt.py
fails closed when they do.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

PROVIDER = "azure-artifact-signing"
EXPECTED_PUBLISHER = "PARTNERNET SOFTWARE PTY LTD"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--asset", required=True, type=Path)
    parser.add_argument("--before-sha256", required=True)
    parser.add_argument("--inspection", required=True, type=Path)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--release-eligible", required=True, choices=("true", "false"))
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    inspection = json.loads(args.inspection.read_text(encoding="utf-8-sig"))
    after = sha256(args.asset)
    if inspection.get("sha256") != after:
        raise ValueError("inspection does not describe the signed bytes")
    if args.release_eligible == "true" and inspection.get("status") != "Valid":
        raise ValueError("a release-eligible receipt requires a Valid signature")
    signer = inspection.get("signer") or {}
    if f"O={EXPECTED_PUBLISHER}" not in (signer.get("subject") or ""):
        raise ValueError("signer subject does not carry the company organization")
    if not inspection.get("timestamp_certificate"):
        raise ValueError("a timestamp certificate is required")

    receipt = {
        "schema": "srcmgr-signing-receipt/1",
        "signing_provider": PROVIDER,
        "release_eligible": args.release_eligible == "true",
        "source_sha": args.source_sha,
        "version": args.version,
        "assets": [
            {
                "path": inspection.get("file_name"),
                "before_sha256": args.before_sha256,
                "after_sha256": after,
                "size_bytes": inspection.get("size_bytes"),
                "signature_status": inspection.get("status"),
                "product_name": inspection.get("product_name"),
                "product_version": inspection.get("product_version"),
                "signer_subject": signer.get("subject"),
                "signer_issuer": signer.get("issuer"),
                "signer_thumbprint": signer.get("thumbprint"),
                "signer_not_after": signer.get("not_after"),
                "timestamp_subject": (inspection.get("timestamp_certificate") or {}).get("subject"),
                "timestamp_issuer": (inspection.get("timestamp_certificate") or {}).get("issuer"),
            }
        ],
    }
    args.out.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(f"receipt written for {receipt['assets'][0]['path']} ({after})")


if __name__ == "__main__":
    main()

"""Compose the public signing receipt from canonical inspector output.

The receipt is the only public record of a signed file. Provider coordinates
(endpoint, account, profile, tenant, subscription, application or principal
identifiers) never enter it, and scripts/audit-public-signing-receipt.py fails
closed when any of them appear.
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
    return hashlib.sha256(path.read_bytes()).hexdigest()


def text(inspection: dict, key: str) -> str:
    value = inspection.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"inspection is missing {key}")
    return value


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--asset", required=True, type=Path)
    parser.add_argument("--name", required=True)
    parser.add_argument("--relative-path", required=True)
    parser.add_argument("--before-sha256", required=True)
    parser.add_argument("--inspection", required=True, type=Path)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--run-id", required=True, type=int)
    parser.add_argument("--run-attempt", required=True, type=int)
    parser.add_argument("--release-eligible", required=True, choices=("true", "false"))
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    inspection = json.loads(args.inspection.read_text(encoding="utf-8-sig"))
    after = sha256(args.asset)
    if inspection.get("sha256") != after:
        raise ValueError("inspection does not describe the signed bytes")
    signer = inspection.get("signer") or {}
    timestamp = inspection.get("timestamp_certificate") or {}
    subject = signer.get("subject") or ""
    if f"O={PUBLISHER}" not in subject:
        raise ValueError("signer subject does not carry the company organization")
    eligible = args.release_eligible == "true"
    if eligible and inspection.get("status") != "Valid":
        raise ValueError("a release-eligible receipt requires a Valid signature")

    receipt = {
        "schema": 1,
        "kind": "srcmgr-candidate-signing",
        "source_sha": args.source_sha,
        "product_version": args.version,
        "signing_provider": PROVIDER,
        "publisher_organization": PUBLISHER,
        "file_digest": "SHA256",
        "timestamp_rfc3161": TIMESTAMP_URL,
        "timestamp_digest": "SHA256",
        "release_eligible": eligible,
        "signing_run": {"id": args.run_id, "attempt": args.run_attempt},
        "assets": {
            args.name: {
                "path": args.relative_path,
                "before_sha256": args.before_sha256,
                "after_sha256": after,
                "after_bytes": int(inspection.get("size_bytes") or 0),
                "authenticode_status": text(inspection, "status"),
                "signer_subject": subject,
                "signer_issuer": text(signer, "issuer"),
                "signer_thumbprint": text(signer, "thumbprint"),
                "signer_not_before": text(signer, "not_before"),
                "signer_not_after": text(signer, "not_after"),
                "timestamp_subject": text(timestamp, "subject"),
                "timestamp_issuer": text(timestamp, "issuer"),
            }
        },
    }
    args.out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"receipt written for {args.name} ({after})")


if __name__ == "__main__":
    main()

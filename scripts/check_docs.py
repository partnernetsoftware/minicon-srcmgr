"""Check the requirements baseline's links, owner structure and case coverage."""

from pathlib import Path
import re
import sys
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[1]
PREFIXES = {
    "01": "PROD", "02": "ARCH", "03": "REPO", "04": "CHANGE",
    "05": "HIST", "06": "REF", "07": "REMOTE", "08": "REC",
    "09": "ADV", "10": "UX", "11": "SEC", "12": "SHIP",
    "13": "STATE", "14": "QA",
}


def main() -> int:
    errors = []
    modules = sorted((ROOT / "prd").glob("PRD_*.md"))
    if len(modules) != len(PREFIXES):
        errors.append(f"Expected 14 owner modules, found {len(modules)}")
    index = (ROOT / "PRD.md").read_text(encoding="utf-8")
    definitions = {}
    numbers = set()
    for module in modules:
        content = module.read_text(encoding="utf-8")
        number = module.name.split("_")[1]
        if number in numbers or number not in PREFIXES:
            errors.append(f"Duplicate or unknown owner number: {module.name}")
        numbers.add(number)
        relative = module.relative_to(ROOT).as_posix()
        if f"]({relative})" not in index:
            errors.append(f"Unindexed module: {relative}")
        for marker in ("```text", "```mermaid", "| Invariant |", "Safe failure", "Dependencies", "Non-goal"):
            if marker not in content:
                errors.append(f"{relative}: missing owner contract marker {marker!r}")
        for identifier in re.findall(r"^\| ([A-Z]+-\d{2}) ·", content, re.MULTILINE):
            if identifier in definitions:
                errors.append(f"Multiple owners for {identifier}")
            if not identifier.startswith(PREFIXES.get(number, "INVALID") + "-"):
                errors.append(f"{relative}: wrong requirement namespace {identifier}")
            definitions[identifier] = relative
        if content.count("```") % 2:
            errors.append(f"{relative}: unbalanced Markdown fences")

    for document in ROOT.rglob("*.md"):
        if ".git" in document.relative_to(ROOT).parts:
            continue
        content = document.read_text(encoding="utf-8")
        for target in re.findall(r"\[[^\]]+\]\(([^)]+)\)", content):
            if re.match(r"[a-zA-Z][a-zA-Z0-9+.-]*:", target) or target.startswith("#"):
                continue
            path = unquote(target.split("#", 1)[0].strip("<>"))
            resolved = (document.parent / path).resolve()
            if not resolved.is_relative_to(ROOT) or not resolved.exists():
                errors.append(f"{document.relative_to(ROOT)}: invalid local link {target}")

    registry = (ROOT / "prd/PRD_14_verification_roadmap.md").read_text(encoding="utf-8")
    registry_rows = re.findall(r"^\| (QA-J\d{2}) ·.*$", registry, re.MULTILINE)
    if set(registry_rows) != {f"QA-J{i:02d}" for i in range(1, 14)} or len(registry_rows) != 13:
        errors.append("Journey registry must define QA-J01 through QA-J13 exactly once")
    coverage = set()
    for line in registry.splitlines():
        if not line.startswith("| QA-J"):
            continue
        for prefix, suffixes in re.findall(r"([A-Z]+)-((?:\d{2})(?:/\d{2})*)", line.split("|")[2]):
            coverage.update(f"{prefix}-{suffix}" for suffix in suffixes.split("/"))
    behavior = {identifier for identifier in definitions if not identifier.startswith("QA-")}
    for identifier in sorted(behavior - coverage):
        errors.append(f"Requirement has no journey coverage: {identifier}")
    for identifier in sorted(coverage - behavior):
        errors.append(f"Journey cites undefined requirement: {identifier}")
    for document in modules:
        for journey in set(re.findall(r"QA-J\d{2}", document.read_text(encoding="utf-8"))):
            if journey not in registry_rows:
                errors.append(f"{document.name}: undefined journey {journey}")

    if errors:
        print("Documentation check FAILED:")
        print("\n".join(f"- {error}" for error in errors))
        return 1
    print(f"PASS: {len(modules)} owner modules, {len(definitions)} requirement contracts, "
          f"{len(registry_rows)} journey families; local links and behavior coverage valid.")
    print("This checks documentation structure, not application implementation or runtime evidence.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

# -*- coding: utf-8 -*-
"""Move the CLI integration tests onto the Git spelling.

Only call *shapes* change. Assertions, fixtures and oracles are untouched: the
rewrite is a transformation of `cli(&profile, &[...])` argument lists.

  ["verb", W, rest...]          -> ["-C", W, "verb", rest...]      (repository verbs)
  ["trust", "grant", W]         -> ["-C", W, "trust", "grant"]
  ["history", W, ...]           -> ["-C", W, "log", ...]
  ["setting", "get", K]         -> ["config", "get", K]
  ["ignore", W, P]              -> ["-C", W, "check-ignore", "--", P]
"""
import re
import sys
from pathlib import Path

REPOSITORY_VERBS = {"status", "stage", "unstage", "commit", "draft", "diff", "branch", "remote", "fetch",
                    "pull", "push", "hunk", "history", "ignore", "gui"}
RENAMED = {"history": "log"}

# A single-line call whose argument list is one slice literal. The two
# multi-line calls in clone_m1.rs and the one call whose path literal holds a
# `]` (corpus_j11.rs) were moved by hand.
CALL = re.compile(r'cli\(\s*&profile,\s*&\[(?P<body>[^\]]*)\]\)', re.S)


def split_items(body: str) -> list[str]:
    """Split a Rust slice body on top-level commas."""
    items, depth, quote, cur, i = [], 0, None, "", 0
    while i < len(body):
        c = body[i]
        if quote:
            cur += c
            if c == "\\":
                cur += body[i + 1]
                i += 2
                continue
            if c == quote:
                quote = None
        elif c in "\"'":
            quote = c
            cur += c
        elif c in "([{":
            depth += 1
            cur += c
        elif c in ")]}":
            depth -= 1
            cur += c
        elif c == "," and depth == 0:
            items.append(cur.strip())
            cur = ""
        else:
            cur += c
        i += 1
    if cur.strip():
        items.append(cur.strip())
    return items


def is_literal(item: str) -> bool:
    return len(item) >= 2 and item[0] == '"' and item[-1] == '"' and "\\" not in item


def rewrite_call(match: re.Match) -> str:
    items = split_items(match.group("body"))
    if not items:
        return match.group(0)
    verb = items[0]
    if not is_literal(verb):
        return match.group(0)
    name = verb[1:-1]

    if name == "trust" and len(items) >= 3 and is_literal(items[1]) and items[1] in ('"grant"', '"revoke"') and len(items) == 3:
        return f'cli(&profile, &["-C", {items[2]}, "trust", {items[1]}])'
    if name == "setting" and is_literal(items[1]):
        return f'cli(&profile, &["config", {items[1]}, {", ".join(items[2:])}])'
    if name == "ignore" and len(items) == 3:
        return f'cli(&profile, &["-C", {items[1]}, "check-ignore", "--", {items[2]}])'
    if name in REPOSITORY_VERBS and len(items) >= 2:
        rest = items[2:]
        target = RENAMED.get(name, name)
        # `diff` and `hunk` keep their option/operand order as written; inserting
        # `--` there would turn a trailing `--staged`, or the `hunk` action word,
        # into a path. Only the verbs that take bare paths get the separator.
        sep = ['"--"'] if target in ("stage", "unstage") and rest and not rest[0].startswith('"--"') else []
        joined = ", ".join([f'"{target}"'] + sep + rest)
        return f'cli(&profile, &["-C", {items[1]}, {joined}])'
    return match.group(0)


def main() -> int:
    root = Path("crates/srcmgr-cli/tests")
    changed = 0
    for path in sorted(root.glob("*.rs")):
        text = path.read_text(encoding="utf-8")
        new = CALL.sub(rewrite_call, text)
        if new != text:
            path.write_text(new, encoding="utf-8", newline="")
            changed += 1
    print(f"rewrote call sites in {changed} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())

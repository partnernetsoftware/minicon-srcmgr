from pathlib import Path

p = Path("plan/plan-0.1.3.md")
s = p.read_text(encoding="utf-8")
i = s.index("| C3 · language |")
j = s.index(chr(10), i)
s = s[:i] + "| C3 · language | Every primary control, status and refusal has a zh-CN string; the override survives restart; paths and OIDs are untouched | `[-]` 166 keys. UI text, status suffixes, pane titles, the blocked-write and diverged-pull refusals, and now the backend's own policy and result strings (review expiry, commit policy, fetch results, action outcome prefixes) all come from the catalogue, with English and Chinese asserted for a block, a divergence and a commit refusal. Still English: footer hints, Git's own error detail (it prints it), and 25 remaining literals in the entry and window backends, mostly internal guards and the clone/trust hand-off |" + s[j:]

old_tail = "Not built yet: the footer hint lines that name keys, raw Git error text (still English because Git printed it), the remaining backend policy strings, and the clean-image and five-participant courts (C6, C7)."
new_tail = """### 2026-11-14 · backend policy strings

- **Language (C3, partial).** The backend's own sentences now come from the same table: review expiry for checkout and remotes, the commit policy refusals (needs reconciliation, unavailable, changed, empty message, load the review first), the fetch result, the three action-result prefixes and the observed-commit line. Two helpers decide the wording — one on the backend, one for associated functions that have options but no self — and both fall back to English when the profile cannot be read, so a broken store never costs the user the explanation.
- **What stays English, deliberately.** Git's own error detail is printed by Git, and translating it would report something the tool did not say. Internal guards ("nested confirmation is not allowed") are unreachable through the window and stay as code text.
- **Evidence.** A backend test refuses an empty commit message in English and then in Chinese from one profile, asserting the exact English string and the Chinese one, so the fallback and the switch are both covered.

Not built yet: the footer hint lines that name keys, Git's own error text, the 25 remaining literals in the entry and window backends, and the clean-image and five-participant courts (C6, C7)."""
assert s.count(old_tail) == 1, s.count(old_tail)
s = s.replace(old_tail, new_tail)
p.write_text(s, encoding="utf-8", newline="")

q = Path("prd/prd_mempalace.md")
t = q.read_text(encoding="utf-8")
old_fact = """- **Refusal text belongs to the window, not to `srcmgr-git`.**"""
new_fact = """- **Backend sentences come from the catalogue too, through one helper.** Every
  refusal the product composes itself is a template plus data; a caller without
  a self parameter uses the free helper, and both fall back to English rather
  than losing the message. Git's own error detail stays literal in both
  languages, because Git printed it.
- **Refusal text belongs to the window, not to `srcmgr-git`.**"""
assert t.count(old_fact) == 1, t.count(old_fact)
t = t.replace(old_fact, new_fact)
q.write_text(t, encoding="utf-8", newline="")
print("ok")

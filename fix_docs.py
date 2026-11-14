from pathlib import Path

p = Path("scripts/check_docs.py")
s = p.read_text(encoding="utf-8")
old = 'modules = sorted((ROOT / "prd").glob("PRD_*.md"))'
new = '# Owner modules are exactly PRD_<two digits>_<name>.md. The narrower glob keeps
new += chr(10) + '    # a differently named companion file (for example the memory palace) from
new += chr(10) + '    # being counted as a sixteenth owner module on case-insensitive filesystems.'
new += chr(10) + '    modules = sorted((ROOT / "prd").glob("PRD_[0-9][0-9]_*.md"))'
assert s.count(old) == 1, s.count(old)
p.write_text(s.replace(old, new), encoding="utf-8", newline="")

prd = Path("PRD.md")
t = prd.read_text(encoding="utf-8")
old_par = "[prd/prd_mempalace.md](prd/prd_mempalace.md) is the distilled map of this baseline: notation, the ten invariants, requirement ownership, the decisions that must not be re-proposed, where each version stands, and the operational facts that had to be re-learned. Read it first when returning to the project."
new_par = "[prd/prd_mempalace.md](prd/prd_mempalace.md) is the distilled map of this" + chr(10)
new_par += "baseline: notation, the ten invariants, requirement ownership, the decisions" + chr(10)
new_par += "that must not be re-proposed, where each version stands, and the operational" + chr(10)
new_par += "facts that had to be re-learned. Read it first when returning to the project."
assert t.count(old_par) == 1, t.count(old_par)
t = t.replace(old_par, new_par)
old_par2 = "It is a reading aid only. It carries no requirement text and no acceptance evidence; the owning module below always wins, and only a plan or release record can prove delivery. When this index and the memory palace disagree, this index and the owner modules are correct."
new_par2 = "It is a reading aid only. It carries no requirement text and no acceptance" + chr(10)
new_par2 += "evidence; the owning module below always wins, and only a plan or release" + chr(10)
new_par2 += "record can prove delivery. When this index and the memory palace disagree," + chr(10)
new_par2 += "this index and the owner modules are correct."
assert t.count(old_par2) == 1, t.count(old_par2)
t = t.replace(old_par2, new_par2)
prd.write_text(t, encoding="utf-8", newline="")
print("patched")
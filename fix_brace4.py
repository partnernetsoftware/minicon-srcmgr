from pathlib import Path
p = Path("crates/srcmgr-git/src/checkout.rs")
lines = p.read_text(encoding="utf-8").split(chr(10))
assert lines[139] == "" and lines[140] == "}" and lines[141] == "", (lines[139:142])
del lines[140:142]
p.write_text(chr(10).join(lines), encoding="utf-8", newline="")
print("ok")

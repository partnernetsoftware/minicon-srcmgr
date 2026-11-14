from pathlib import Path
p = Path("crates/srcmgr-git/src/checkout.rs")
s = p.read_text(encoding="utf-8")
old = '    Ok(format!("detached HEAD at {oid}"))' + chr(10) + '}}'
new = '    Ok(format!("detached HEAD at {oid}"))' + chr(10) + '}' + chr(10)
assert s.count(old) == 1, s.count(old)
s = s.replace(old, new)
p.write_text(s, encoding="utf-8", newline="")
print("ok")

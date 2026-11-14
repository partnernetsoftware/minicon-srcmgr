from pathlib import Path
p = Path("crates/srcmgr-cli/src/window_backend.rs")
s = p.read_text(encoding="utf-8")
old = '"two' + chr(10) + '");'
new = '"one" + chr(92) + 'n");'
if s.count(old) != 1:
    print("count", s.count(old))
    print(repr(s[s.index('"two')-20:s.index('"two')+40]))
else:
    s = s.replace(old, new)
    p.write_text(s, encoding="utf-8", newline="")
    print("fixed")

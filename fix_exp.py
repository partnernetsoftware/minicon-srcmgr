from pathlib import Path
p = Path("crates/srcmgr-cli/src/window_backend.rs")
s = p.read_text(encoding="utf-8")
old = 'assert_eq!(std::fs::read_to_string(target.join("a.txt")).unwrap(), "two\\n");'
new = '// The clone carries the committed content, not the uncommitted edit in the source.\n        assert_eq!(std::fs::read_to_string(target.join("a.txt")).unwrap(), "one\\n");'
assert s.count(old) == 1, s.count(old)
p.write_text(s.replace(old, new), encoding="utf-8", newline="")
print("ok")

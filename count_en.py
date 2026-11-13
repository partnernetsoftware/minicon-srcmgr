from pathlib import Path
import re
for path in ("crates/srcmgr-ui/src/window.rs", "crates/srcmgr-ui/src/workspace.rs"):
    s = Path(path).read_text(encoding="utf-8")
    # user-facing literals: MainView::text, notice = Some("..."), hints
    counts = {}
    for pat, name in [
        (r"MainView::text\("", "MainView::text"),
        (r"notice = Some\("", "notice"),
        (r"hints\(", "hints"),
        (r"Draw::Text \{[^}]*text: "", "Draw::Text literal"),
        (r"format!\("", "format!"),
    ]:
        counts[name] = len(re.findall(pat, s))
    print(path, counts)
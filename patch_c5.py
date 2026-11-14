from pathlib import Path

w = Path("crates/srcmgr-ui/src/window.rs")
s = w.read_text(encoding="utf-8")
old = "    let focus = match workspace.focus() {"
new = '''    // An unfinished integration makes every write refuse. Point at the one
    // action that can move it forward instead of leaving the user to guess.
    if !workspace.is_welcome() {
        let snapshot = workspace.snapshot();
        if snapshot.operation.is_some() || !snapshot.unmerged.is_empty() {
            return "Writes are blocked until this operation is resolved   T Terminal handoff   R Remotes   ? help".to_owned();
        }
    }
    let focus = match workspace.focus() {'''
assert s.count(old) == 1, s.count(old)
s = s.replace(old, new)

anchor = "    pub(super) fn snapshot(files: usize) -> Snapshot {"
test = '''    #[test]
    fn a_blocked_repository_points_at_the_terminal_handoff() {
        let mut blocked = super::tests::snapshot(1);
        blocked.operation = Some("merge".into());
        blocked.unmerged = vec!["a.txt".into()];
        let hint = hints(&Workspace::new(blocked));
        assert!(hint.contains("blocked"), "{hint}");
        assert!(hint.contains("T Terminal handoff"), "{hint}");
        let clean = hints(&Workspace::new(super::tests::snapshot(1)));
        assert!(!clean.contains("blocked"), "{clean}");
    }

'''
assert s.count(anchor) == 1
s = s.replace(anchor, test + anchor)
w.write_text(s, encoding="utf-8", newline="")

b = Path("crates/srcmgr-cli/src/window_backend.rs")
t = b.read_text(encoding="utf-8")
old_msg = '''                "Refused: {}. Resolve it in a terminal, then refresh.",'''
new_msg = '''                "Refused: {}. Resolve it in a terminal, then refresh. Press T in the window to open a configured terminal at this repository.",'''
assert t.count(old_msg) == 1, t.count(old_msg)
b.write_text(t.replace(old_msg, new_msg), encoding="utf-8", newline="")
print("ok")

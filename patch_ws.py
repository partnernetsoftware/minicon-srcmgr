from pathlib import Path

p = Path("crates/srcmgr-ui/src/workspace.rs")
s = p.read_text(encoding="utf-8")

def sub(old, new, count=1):
    global s
    assert s.count(old) == count, (s.count(old), old[:60])
    s = s.replace(old, new)

# 1. field
sub("    composer: Option<CommitComposer>,\n}",
    "    composer: Option<CommitComposer>,\n    remotes: Option<crate::RemotesPanel>,\n}")

# 2. constructor
sub("            locate: None,\n            clone: None,\n            tools: None,",
    "            locate: None,\n            clone: None,\n            remotes: None,\n            tools: None,")

# 3. modal
sub("self.navigation_issue.is_some()\n    }",
    "self.navigation_issue.is_some() || self.remotes.is_some()\n    }")

# 4. key dispatch: remotes surface owns keys before the panes do
sub("    fn key_inner(&mut self, key: Key) -> Effect {\n",
    "    fn key_inner(&mut self, key: Key) -> Effect {\n        if self.remotes.is_some() { return self.remotes_key(key); }\n")

# 5. R opens the panel
sub("            Key::Char('r') => Effect::Send(Job::Reload),",
    "            Key::Char('R') => self.request_remotes(),\n            Key::Char('r') => Effect::Send(Job::Reload),")

# 6. reply handlers, before LocateReview
sub("            Reply::LocateReview { ticket, lines } => {",
    '''            Reply::RemoteReview { ticket, lines } => {
                if let Some(panel) = self.remotes.as_mut() {
                    panel.working = false;
                    panel.review = Some((ticket, lines));
                    panel.detail = "Review the local effect above. y applies exactly this; n cancels. No server repository is touched.".into();
                }
                self.trust_scroll = 0;
                Effect::Nothing
            }
            Reply::RemoteApplied { lines } => {
                self.remotes = None;
                let detail = if lines.is_empty() { String::new() } else { format!(" {}", lines.join(" ")) };
                self.notice = Some(format!(
                    "Applied.{detail} The repository configuration changed, so its trust fingerprint changed with it. Reopen the repository from Recent to review the new fingerprint; until then no Git action will run."
                ));
                Effect::Nothing
            }
            Reply::LocateReview { ticket, lines } => {''')

# 7. accessors and key routing, after clone accessors
sub('''    pub fn close_clone(&mut self) -> Effect {
        self.clone = None;
        Effect::Send(Job::CancelClone)
    }''',
'''    pub fn close_clone(&mut self) -> Effect {
        self.clone = None;
        Effect::Send(Job::CancelClone)
    }

    pub fn remotes_panel(&self) -> Option<&crate::RemotesPanel> { self.remotes.as_ref() }

    /// Opens the Remotes panel over an open repository. It is refused on the
    /// Welcome surface, where there is no configuration to edit.
    pub fn request_remotes(&mut self) -> Effect {
        if self.is_welcome() || self.modal() {
            return Effect::Nothing;
        }
        self.remotes = Some(crate::RemotesPanel::new());
        Effect::Nothing
    }

    fn selected_remote(&self) -> Option<crate::RemoteRow> {
        let panel = self.remotes.as_ref()?;
        self.snapshot.remotes.get(panel.selected).cloned()
    }

    /// Keys for the Remotes surface. A review owns y/n and scrolling; an open
    /// form owns text editing; otherwise the letters pick an action.
    pub fn remotes_key(&mut self, key: Key) -> Effect {
        if self.remotes.as_ref().is_none_or(|panel| panel.working) {
            return Effect::Nothing;
        }
        if self.remotes.as_ref().is_some_and(|panel| panel.review.is_some()) {
            return match key {
                Key::Char('y') => self.remotes.as_mut().map_or(Effect::Nothing, crate::RemotesPanel::approve),
                Key::Char('n') => {
                    if let Some(panel) = self.remotes.as_mut() { panel.close(); }
                    Effect::Send(Job::CancelRemoteEdit)
                }
                Key::Up => self.scroll_trust(-1),
                Key::Down => self.scroll_trust(1),
                Key::PageUp => self.scroll_trust(-(PAGE_LINES as isize)),
                Key::PageDown => self.scroll_trust(PAGE_LINES as isize),
                _ => Effect::Nothing,
            };
        }
        if self.remotes.as_ref().is_some_and(|panel| panel.action.is_some()) {
            if key == Key::Escape || key == Key::CtrlC {
                if let Some(panel) = self.remotes.as_mut() { panel.close(); }
                return Effect::Send(Job::CancelRemoteEdit);
            }
            return self.remotes.as_mut().map_or(Effect::Nothing, |panel| panel.edit(key));
        }
        if key == Key::Escape || key == Key::CtrlC {
            self.remotes = None;
            return Effect::Nothing;
        }
        let rows = self.snapshot.remotes.len();
        let selected = self.selected_remote();
        let Some(panel) = self.remotes.as_mut() else { return Effect::Nothing };
        match key {
            Key::Down | Key::Char('j') => panel.move_selection(1, rows),
            Key::Up | Key::Char('k') => panel.move_selection(-1, rows),
            Key::Char('a') => panel.begin_add(),
            Key::Char('s') => panel.begin_on_selected(crate::RemotesAction::SetUrl, selected.as_ref()),
            Key::Char('r') => panel.begin_on_selected(crate::RemotesAction::Rename, selected.as_ref()),
            Key::Char('d') => panel.begin_on_selected(crate::RemotesAction::Remove, selected.as_ref()),
            Key::Char('f') => match selected {
                Some(row) => Effect::Send(Job::Fetch { remote: row.name }),
                None => {
                    panel.detail = "Select a remote first; there is nothing to fetch.".into();
                    Effect::Nothing
                }
            },
            _ => Effect::Nothing,
        }
    }''')

# 8. test fixture row
sub('''        state.remotes = vec![crate::RemoteRow {''',
    '''        state.remotes = vec![crate::RemoteRow {
            refspecs: Vec::new(),''')

p.write_text(s, encoding="utf-8", newline="")

q = Path("crates/srcmgr-ui/src/lib.rs")
t = q.read_text(encoding="utf-8")
assert t.count("pub use model::{") == 1
t = t.replace("pub use model::{", "pub use model::{RemoteEdit, RemoteRow,")
q.write_text(t, encoding="utf-8", newline="")
print("workspace patched")

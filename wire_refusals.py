from pathlib import Path

m = Path("crates/srcmgr-ui/src/messages.rs")
s = m.read_text(encoding="utf-8")
s = s.replace("pub(crate) enum Key {", "pub enum Key {")
s = s.replace("pub(crate) const fn text(key: Key, locale: Locale)", "pub const fn text(key: Key, locale: Locale)")
s = s.replace("pub(crate) const fn english(key: Key)", "pub const fn english(key: Key)")
s = s.replace("pub(crate) const fn chinese(key: Key)", "pub const fn chinese(key: Key)")
m.write_text(s, encoding="utf-8", newline="")

lib = Path("crates/srcmgr-ui/src/lib.rs")
t = lib.read_text(encoding="utf-8")
old = "pub use messages::Locale;"
new = "pub use messages::{Key as TextKey, Locale, text as text_of};"
assert t.count(old) == 1, t.count(old)
t = t.replace(old, new)
lib.write_text(t, encoding="utf-8", newline="")

b = Path("crates/srcmgr-cli/src/window_backend.rs")
u = b.read_text(encoding="utf-8")
old_fn = '''    fn refuse_while_unresolved(&self, session: &crate::Session) -> Result<(), String> {
        let state = srcmgr_git::detect_operation(&session.installation, &session.identity)
            .map_err(|error| format!("Cannot read the repository's operation state: {error}"))?;
        match state.refusal() {
            Some(reason) => Err(format!(
                "{reason}\nRepository: {}",
                crate::NativePath::new(session.identity.worktree_root.as_os_str()).display_label()
            )),
            None => Ok(()),
        }
    }'''
new_fn = '''    fn refuse_while_unresolved(&self, session: &crate::Session) -> Result<(), String> {
        let state = srcmgr_git::detect_operation(&session.installation, &session.identity)
            .map_err(|error| format!("Cannot read the repository's operation state: {error}"))?;
        if state.operation.is_none() && state.unmerged.is_empty() && state.unreadable.is_none() {
            return Ok(());
        }
        // Composed here rather than in srcmgr-git: the text is interface language,
        // and the Git crate has no locale. The state itself is Git's own.
        let locale = self.locale()?;
        let text = srcmgr_ui::text_of;
        let mut message = if let Some(detail) = &state.unreadable {
            text(srcmgr_ui::TextKey::RefuseOpUnreadable, locale).replace("{detail}", detail)
        } else {
            let count = state.unmerged.len();
            let operation = state.operation.map(|operation| operation.name());
            match (operation, count) {
                (Some(operation), 0) => text(srcmgr_ui::TextKey::RefuseOpInProgress, locale).replace("{operation}", operation),
                (None, count) => text(srcmgr_ui::TextKey::RefuseOpUnmerged, locale).replace("{count}", &count.to_string()),
                (Some(operation), count) => text(srcmgr_ui::TextKey::RefuseOpBoth, locale)
                    .replace("{operation}", operation)
                    .replace("{count}", &count.to_string()),
            }
        };
        message.push('\\n');
        message.push_str(
            &text(srcmgr_ui::TextKey::RefusePath, locale).replace(
                "{path}",
                &crate::NativePath::new(session.identity.worktree_root.as_os_str()).display_label(),
            ),
        );
        Err(message)
    }

    /// The interface language this backend answers in. It changes wording only.
    fn locale(&self) -> Result<srcmgr_ui::Locale, String> {
        let store = open_profile_store(&self.options)?;
        crate::interface_locale(&store)
    }'''
assert u.count(old_fn) == 1, u.count(old_fn)
u = u.replace(old_fn, new_fn)

old_div = '''                    return Err(format!(
                        "Refused: the local branch and its fetched upstream have both advanced.\nLocal: {local}\nFetched upstream: {upstream}\nPull never merges, rebases or stashes on its own; resolve the divergence in a terminal, then refresh."
                    ));'''
new_div = '''                    return Err(srcmgr_ui::text_of(srcmgr_ui::TextKey::RefuseDiverged, self.locale()?)
                        .replace("{local}", local)
                        .replace("{upstream}", upstream));'''
assert u.count(old_div) == 1, u.count(old_div)
u = u.replace(old_div, new_div)
b.write_text(u, encoding="utf-8", newline="")
print("ok")

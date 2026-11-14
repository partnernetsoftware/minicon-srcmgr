//! The Remotes panel: one row per configured remote, and five reviewed actions.
//!
//! Every write goes through the same two steps the CLI uses. The first shows the
//! exact local values before and after and states that no server is touched; the
//! second applies that reviewed effect through a one-use ticket. A refusal is
//! returned with its reason and leaves the configuration untouched, because
//! validation happens before Git runs.
//!
//! Fetch is not a write. It contacts the server by definition, so it skips the
//! review and reports the refs Git actually updated.

use minicon_core::composer::{self, ComposerState, Move};

use crate::{Effect, Job, RemoteEdit, RemoteRow};

/// The edit an open form will request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemotesAction {
    Add,
    SetUrl,
    Rename,
    Remove,
}

/// Which text field the next typed character lands in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemotesField {
    Name,
    Url,
    To,
}

impl RemotesField {
    const fn row(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Url => 1,
            Self::To => 2,
        }
    }
}

pub struct RemotesPanel {
    /// Index into the snapshot's remote rows.
    pub selected: usize,
    pub action: Option<RemotesAction>,
    editors: [ComposerState; 3],
    pub focus: RemotesField,
    /// Set URL: edit the push URL instead of the fetch URL.
    pub push: bool,
    /// Ticket and lines the backend produced. Present means the form is showing
    /// a review and refuses edits until it is approved or cancelled.
    pub review: Option<(String, Vec<String>)>,
    pub working: bool,
    pub detail: String,
}

impl Default for RemotesPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl RemotesPanel {
    pub fn new() -> Self {
        Self {
            selected: 0,
            action: None,
            editors: std::array::from_fn(|_| ComposerState::default()),
            focus: RemotesField::Name,
            push: false,
            review: None,
            working: false,
            detail: "a Add   s Set URL   r Rename   d Remove   f Fetch this remote   Esc Close".into(),
        }
    }

    pub fn text(&self, field: RemotesField) -> &str {
        &self.editors[field.row()].text
    }

    /// The fields Tab walks for the open action, in order.
    fn order(&self) -> &'static [RemotesField] {
        match self.action {
            Some(RemotesAction::Add) => &[RemotesField::Name, RemotesField::Url],
            Some(RemotesAction::SetUrl) => &[RemotesField::Url],
            Some(RemotesAction::Rename) => &[RemotesField::To],
            Some(RemotesAction::Remove) | None => &[],
        }
    }

    pub fn move_selection(&mut self, delta: isize, rows: usize) -> Effect {
        if self.action.is_some() || rows == 0 {
            return Effect::Nothing;
        }
        let next = self.selected.saturating_add_signed(delta);
        self.selected = next.min(rows - 1);
        Effect::Nothing
    }

    fn reset(&mut self, action: RemotesAction, name: &str, url: &str) {
        self.editors = std::array::from_fn(|_| ComposerState::default());
        self.action = Some(action);
        self.review = None;
        self.working = false;
        self.push = false;
        self.editors[0].text = name.to_owned();
        self.editors[1].text = url.to_owned();
        // Prefilled values put the caret at the end, so typing extends the URL
        // the user is editing instead of preceding it.
        for index in 0..2 {
            self.editors[index].caret = self.editors[index].text.len();
        }
        self.focus = match action {
            RemotesAction::Add => RemotesField::Name,
            RemotesAction::SetUrl => RemotesField::Url,
            RemotesAction::Rename => RemotesField::To,
            RemotesAction::Remove => RemotesField::Name,
        };
        self.detail = match action {
            RemotesAction::Add => "Enter Review  ·  Tab Field  ·  Esc Cancel. The review names the values written; no server is touched.".into(),
            RemotesAction::SetUrl => "Enter Review  ·  Space fetch/push URL  ·  Esc Cancel".into(),
            RemotesAction::Rename => "Enter Review  ·  Esc Cancel. The remote's stored configuration is renamed; nothing is fetched.".into(),
            RemotesAction::Remove => "Enter Review  ·  Esc Cancel. Removing a remote deletes its stored URL and refspecs; no server is touched.".into(),
        };
    }

    pub fn begin_add(&mut self) -> Effect {
        if self.review.is_some() || self.working {
            return Effect::Nothing;
        }
        self.reset(RemotesAction::Add, "", "");
        Effect::Nothing
    }

    /// Set URL, Rename and Remove need a selected row; without one they refuse
    /// with the reason instead of silently doing nothing.
    pub fn begin_on_selected(&mut self, action: RemotesAction, selected: Option<&RemoteRow>) -> Effect {
        if self.review.is_some() || self.working {
            return Effect::Nothing;
        }
        let Some(row) = selected else {
            self.detail = "Select a remote first; there is no row to change.".into();
            return Effect::Nothing;
        };
        match action {
            RemotesAction::Add => self.begin_add(),
            RemotesAction::SetUrl => {
                self.reset(action, &row.name, &row.fetch_url);
                Effect::Nothing
            }
            RemotesAction::Rename => {
                self.reset(action, &row.name, "");
                Effect::Nothing
            }
            RemotesAction::Remove => {
                self.reset(action, &row.name, "");
                Effect::Nothing
            }
        }
    }

    /// Builds the request the backend validates. Blank required values are
    /// refused here so a request that cannot be described never leaves the UI.
    pub fn submit(&mut self) -> Effect {
        if self.working || self.review.is_some() {
            return Effect::Nothing;
        }
        let Some(action) = self.action else { return Effect::Nothing };
        let name = self.editors[0].text.trim().to_owned();
        let url = self.editors[1].text.trim().to_owned();
        let to = self.editors[2].text.trim().to_owned();
        let change = match action {
            RemotesAction::Add if name.is_empty() || url.is_empty() => {
                self.detail = "A remote needs both a name and a URL; nothing was requested.".into();
                return Effect::Nothing;
            }
            RemotesAction::Add => RemoteEdit::Add { name, url, push_url: None },
            RemotesAction::SetUrl if url.is_empty() => {
                self.detail = "A URL is required; nothing was requested.".into();
                return Effect::Nothing;
            }
            RemotesAction::SetUrl => RemoteEdit::SetUrl { name, url, push: self.push },
            RemotesAction::Rename if to.is_empty() => {
                self.detail = "A new name is required; nothing was requested.".into();
                return Effect::Nothing;
            }
            RemotesAction::Rename => RemoteEdit::Rename { from: name, to },
            RemotesAction::Remove => RemoteEdit::Remove { name },
        };
        self.working = true;
        self.detail = "Reviewing the local effect...".into();
        Effect::Send(Job::PreviewRemoteEdit(change))
    }

    /// Approves the reviewed effect. Only the ticket is sent, so the values the
    /// review showed are the values that run.
    pub fn approve(&mut self) -> Effect {
        if self.working {
            return Effect::Nothing;
        }
        let Some((ticket, _)) = &self.review else {
            return Effect::Nothing;
        };
        self.working = true;
        Effect::Send(Job::ApplyRemoteEdit(ticket.clone()))
    }

    pub fn close(&mut self) {
        self.action = None;
        self.review = None;
        self.working = false;
        self.detail = "a Add   s Set URL   r Rename   d Remove   f Fetch this remote   Esc Close".into();
    }

    pub fn edit(&mut self, key: crate::Key) -> Effect {
        if self.working || self.review.is_some() {
            return Effect::Nothing;
        }
        if self.action.is_none() {
            return Effect::Nothing;
        }
        match key {
            crate::Key::Tab => {
                let order = self.order();
                if !order.is_empty() {
                    let index = order.iter().position(|field| *field == self.focus).unwrap_or(0);
                    self.focus = order[(index + 1) % order.len()];
                }
                return Effect::Nothing;
            }
            crate::Key::BackTab => {
                let order = self.order();
                if !order.is_empty() {
                    let index = order.iter().position(|field| *field == self.focus).unwrap_or(0);
                    self.focus = order[(index + order.len() - 1) % order.len()];
                }
                return Effect::Nothing;
            }
            crate::Key::Space if self.action == Some(RemotesAction::SetUrl) => {
                self.push = !self.push;
                self.detail = if self.push {
                    "Editing the push URL. Enter reviews the change.".into()
                } else {
                    "Editing the fetch URL. Enter reviews the change.".into()
                };
                return Effect::Nothing;
            }
            _ => {}
        }
        if self.action == Some(RemotesAction::Remove) {
            return if key == crate::Key::Enter { self.submit() } else { Effect::Nothing };
        }
        if !self.editor().preedit.is_empty() {
            if key == crate::Key::Escape {
                self.editor().preedit.clear();
            }
            return Effect::Nothing;
        }
        match key {
            crate::Key::Enter => return self.submit(),
            crate::Key::SelectAll => composer::select_all(self.editor()),
            crate::Key::CtrlC | crate::Key::Cut => {
                return composer::selection_text(self.editor()).map_or(Effect::Nothing, |text| {
                    Effect::Copy { text: text.into(), cut: key == crate::Key::Cut }
                });
            }
            crate::Key::Paste => return Effect::Paste,
            crate::Key::Backspace => composer::backspace(self.editor()),
            crate::Key::Delete => composer::delete_forward(self.editor()),
            crate::Key::Left => composer::move_caret(self.editor(), Move::Left),
            crate::Key::Right => composer::move_caret(self.editor(), Move::Right),
            crate::Key::Home => composer::move_caret(self.editor(), Move::LineStart),
            crate::Key::End => composer::move_caret(self.editor(), Move::LineEnd),
            crate::Key::EditMove { movement, extend } => {
                if extend {
                    composer::extend_selection(self.editor(), movement);
                } else {
                    composer::move_caret(self.editor(), movement);
                }
            }
            crate::Key::Char(ch) => return self.insert(&ch.to_string()),
            crate::Key::Text(text) | crate::Key::Pasted(text) => return self.insert(&text),
            _ => {}
        }
        Effect::Nothing
    }

    fn editor(&mut self) -> &mut ComposerState {
        &mut self.editors[self.focus.row()]
    }

    fn insert(&mut self, text: &str) -> Effect {
        let limit = match self.focus {
            RemotesField::Name | RemotesField::To => 255,
            RemotesField::Url => 4 * 1024,
        };
        let removed = composer::selection_bounds(self.editor()).map_or(0, |(a, b)| b - a);
        if text.contains(['\n', '\r', '\0'])
            || self.editor().text.len() - removed + text.len() > limit
        {
            self.detail = format!("This field takes one line up to {limit} bytes; nothing was inserted.");
            return Effect::Nothing;
        }
        composer::insert(self.editor(), text);
        Effect::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, url: &str) -> RemoteRow {
        RemoteRow {
            name: name.to_owned(),
            fetch_url: url.to_owned(),
            push_url: None,
            refspecs: vec![format!("+refs/heads/*:refs/remotes/{name}/*")],
        }
    }

    fn type_text(panel: &mut RemotesPanel, text: &str) {
        for ch in text.chars() {
            panel.edit(crate::Key::Char(ch));
        }
    }

    #[test]
    fn selection_clamps_and_stays_put_while_a_form_is_open() {
        let mut panel = RemotesPanel::new();
        panel.move_selection(1, 3);
        assert_eq!(panel.selected, 1);
        panel.move_selection(10, 3);
        assert_eq!(panel.selected, 2);
        panel.move_selection(-10, 3);
        assert_eq!(panel.selected, 0);
        panel.begin_add();
        panel.move_selection(1, 3);
        assert_eq!(panel.selected, 0, "a form owns the keys while it is open");
    }

    #[test]
    fn a_row_action_without_a_row_refuses_with_the_reason() {
        let mut panel = RemotesPanel::new();
        assert_eq!(panel.begin_on_selected(RemotesAction::Remove, None), Effect::Nothing);
        assert!(panel.action.is_none());
        assert!(panel.detail.contains("Select a remote first"));
    }

    #[test]
    fn each_action_requests_the_exact_edit_the_review_described() {
        let mut panel = RemotesPanel::new();
        panel.begin_add();
        type_text(&mut panel, "origin");
        panel.edit(crate::Key::Tab);
        type_text(&mut panel, "https://example.invalid/r.git");
        assert_eq!(
            panel.submit(),
            Effect::Send(Job::PreviewRemoteEdit(RemoteEdit::Add {
                name: "origin".into(),
                url: "https://example.invalid/r.git".into(),
                push_url: None,
            }))
        );

        let mut panel = RemotesPanel::new();
        panel.begin_on_selected(RemotesAction::SetUrl, Some(&row("origin", "https://a.invalid/x.git")));
        panel.edit(crate::Key::Tab);
        type_text(&mut panel, "/new");
        assert_eq!(
            panel.submit(),
            Effect::Send(Job::PreviewRemoteEdit(RemoteEdit::SetUrl {
                name: "origin".into(),
                url: "https://a.invalid/x.git/new".into(),
                push: false,
            }))
        );

        let mut panel = RemotesPanel::new();
        panel.begin_on_selected(RemotesAction::Rename, Some(&row("origin", "u")));
        type_text(&mut panel, "upstream");
        assert_eq!(
            panel.submit(),
            Effect::Send(Job::PreviewRemoteEdit(RemoteEdit::Rename { from: "origin".into(), to: "upstream".into() }))
        );

        let mut panel = RemotesPanel::new();
        panel.begin_on_selected(RemotesAction::Remove, Some(&row("origin", "u")));
        assert_eq!(
            panel.submit(),
            Effect::Send(Job::PreviewRemoteEdit(RemoteEdit::Remove { name: "origin".into() }))
        );
    }

    #[test]
    fn a_blank_required_value_never_reaches_the_backend() {
        let mut panel = RemotesPanel::new();
        panel.begin_add();
        assert_eq!(panel.submit(), Effect::Nothing);
        assert!(!panel.working);
        assert!(panel.detail.contains("needs both a name and a URL"));

        let mut panel = RemotesPanel::new();
        panel.begin_on_selected(RemotesAction::Rename, Some(&row("origin", "u")));
        assert_eq!(panel.submit(), Effect::Nothing);
        assert!(panel.detail.contains("new name is required"));
    }

    #[test]
    fn space_switches_set_url_between_the_fetch_and_push_url() {
        let mut panel = RemotesPanel::new();
        panel.begin_on_selected(RemotesAction::SetUrl, Some(&row("origin", "u")));
        assert!(!panel.push);
        panel.edit(crate::Key::Space);
        assert!(panel.push);
        panel.edit(crate::Key::Space);
        assert!(!panel.push);
    }

    #[test]
    fn a_reviewed_form_refuses_edits_and_sends_only_its_ticket() {
        let mut panel = RemotesPanel::new();
        panel.begin_on_selected(RemotesAction::Remove, Some(&row("origin", "u")));
        panel.submit();
        panel.working = false;
        panel.review = Some(("ticket-9".into(), vec!["remove origin".into()]));
        assert_eq!(panel.edit(crate::Key::Char('x')), Effect::Nothing);
        assert_eq!(panel.approve(), Effect::Send(Job::ApplyRemoteEdit("ticket-9".into())));
        assert!(panel.working);
    }

    #[test]
    fn a_form_without_a_review_cannot_approve() {
        let mut panel = RemotesPanel::new();
        assert_eq!(panel.approve(), Effect::Nothing);
        assert!(!panel.working);
    }

    #[test]
    fn a_typed_field_refuses_line_breaks_and_overflow() {
        let mut panel = RemotesPanel::new();
        panel.begin_add();
        assert_eq!(panel.insert("two\nlines"), Effect::Nothing);
        assert!(panel.text(RemotesField::Name).is_empty());
        let long = "x".repeat(300);
        panel.insert(&long);
        assert_eq!(panel.text(RemotesField::Name).len(), 0);
        assert!(panel.detail.contains("255 bytes"));
    }
}

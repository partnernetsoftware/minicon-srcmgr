//! The Welcome clone form: URL, destination, branch and depth, then a review
//! that names the exact effect before anything touches the network.
//!
//! Clone keeps `--no-checkout` (PRD 07). Approving starts one transfer; the
//! working tree stays absent until the repository is trusted and a separate
//! review approves the checkout, so a rejected filter never leaves files behind.

use minicon_core::composer::{self, ComposerState, Move};
use srcmgr_domain::NativePath;

use crate::{Effect, Job, Key};

/// Which field the next typed character lands in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloneField {
    Url,
    Destination,
    Branch,
    Depth,
}

impl CloneField {
    const ORDER: [Self; 4] = [Self::Url, Self::Destination, Self::Branch, Self::Depth];

    fn next(self) -> Self {
        let index = Self::ORDER.iter().position(|field| *field == self).unwrap_or(0);
        Self::ORDER[(index + 1) % Self::ORDER.len()]
    }

    fn previous(self) -> Self {
        let index = Self::ORDER.iter().position(|field| *field == self).unwrap_or(0);
        Self::ORDER[(index + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    /// One row of the form, for the renderer and the accessibility tree.
    pub fn row(self) -> usize {
        Self::ORDER.iter().position(|field| *field == self).unwrap_or(0)
    }
}

pub struct CloneForm {
    editors: [ComposerState; 4],
    pub focus: CloneField,
    pub working: bool,
    /// The ticket and the lines the backend produced. Present means the form is
    /// showing a review and refuses edits until it is approved or cancelled.
    pub review: Option<(String, Vec<String>)>,
    pub detail: String,
    pub cancel: bool,
}

impl Default for CloneForm {
    fn default() -> Self {
        Self::new()
    }
}

impl CloneForm {
    pub fn new() -> Self {
        Self {
            editors: std::array::from_fn(|_| ComposerState::default()),
            focus: CloneField::Url,
            working: false,
            review: None,
            detail: "Clone keeps the working tree absent until the repository is trusted and a checkout review approves it. Nothing is transferred until you approve the review.".into(),
            cancel: false,
        }
    }

    pub fn text(&self, field: CloneField) -> &str {
        &self.editors[field.row()].text
    }

    fn editor(&mut self) -> &mut ComposerState {
        &mut self.editors[self.focus.row()]
    }

    /// Sends the preview. Empty URL or destination is refused here so the
    /// backend never sees a request that cannot be described.
    pub fn submit(&mut self) -> Effect {
        if self.working || self.review.is_some() {
            return Effect::Nothing;
        }
        if !self.editors[0].preedit.is_empty() || !self.editors[1].preedit.is_empty() {
            return Effect::Nothing;
        }
        let url = self.editors[0].text.trim().to_owned();
        let destination = self.editors[1].text.trim().to_owned();
        if url.is_empty() || destination.is_empty() {
            self.detail = "A URL and an absolute destination are required; nothing was cloned.".into();
            return Effect::Nothing;
        }
        self.working = true;
        self.detail = "Reviewing the clone without contacting the server...".into();
        Effect::Send(Job::PreviewClone {
            url,
            destination: NativePath::new(destination),
            branch: non_empty(&self.editors[2].text),
            depth: parse_depth(&self.editors[3].text).unwrap_or(None),
        })
    }

    /// Approves the reviewed effect. A ticket is the only thing sent, so the
    /// values the review showed are the values that run.
    pub fn approve(&mut self) -> Effect {
        if self.working {
            return Effect::Nothing;
        }
        let Some((ticket, _)) = &self.review else {
            return Effect::Nothing;
        };
        self.working = true;
        Effect::Send(Job::ExecuteClone(ticket.clone()))
    }

    pub fn cancel_request(&mut self) -> Effect {
        if self.working || self.review.is_some() {
            self.cancel = true;
            return Effect::Nothing;
        }
        self.cancel = true;
        Effect::Send(Job::CancelClone)
    }

    pub fn edit(&mut self, key: Key) -> Effect {
        if self.working || self.review.is_some() {
            return Effect::Nothing;
        }
        match key {
            Key::Tab => {
                self.focus = self.focus.next();
                return Effect::Nothing;
            }
            Key::BackTab => {
                self.focus = self.focus.previous();
                return Effect::Nothing;
            }
            _ => {}
        }
        if !self.editor().preedit.is_empty() {
            if key == Key::Escape {
                self.editor().preedit.clear();
            }
            return Effect::Nothing;
        }
        match key {
            Key::Enter => return self.submit(),
            Key::SelectAll => composer::select_all(self.editor()),
            Key::CtrlC | Key::Cut => {
                return composer::selection_text(self.editor()).map_or(Effect::Nothing, |text| {
                    Effect::Copy { text: text.into(), cut: key == Key::Cut }
                });
            }
            Key::Paste => return Effect::Paste,
            Key::Backspace => composer::backspace(self.editor()),
            Key::Delete => composer::delete_forward(self.editor()),
            Key::Left => composer::move_caret(self.editor(), Move::Left),
            Key::Right => composer::move_caret(self.editor(), Move::Right),
            Key::Home => composer::move_caret(self.editor(), Move::LineStart),
            Key::End => composer::move_caret(self.editor(), Move::LineEnd),
            Key::EditMove { movement, extend } => {
                if extend {
                    composer::extend_selection(self.editor(), movement);
                } else {
                    composer::move_caret(self.editor(), movement);
                }
            }
            Key::Char(ch) => return self.insert(&ch.to_string()),
            Key::Space => return self.insert(" "),
            Key::Text(text) | Key::Pasted(text) => return self.insert(&text),
            _ => {}
        }
        Effect::Nothing
    }

    fn insert(&mut self, text: &str) -> Effect {
        let limit = match self.focus {
            CloneField::Url => 2 * 1024,
            CloneField::Destination => 4 * 1024,
            CloneField::Branch => 1024,
            CloneField::Depth => 10,
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

fn non_empty(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// `Ok(None)` is a full clone. `Ok(Some(n))` is a shallow one.
fn parse_depth(text: &str) -> Result<Option<u32>, String> {
    match text.trim() {
        "" => Ok(None),
        value => match value.parse::<u32>() {
            Ok(depth) if depth > 0 => Ok(Some(depth)),
            _ => Err("depth must be a positive whole number, or blank for a full clone".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> CloneForm {
        CloneForm::new()
    }

    fn type_text(form: &mut CloneForm, text: &str) {
        for ch in text.chars() {
            form.edit(Key::Char(ch));
        }
    }

    #[test]
    fn an_empty_url_or_destination_never_reaches_the_backend() {
        let mut form = form();
        assert_eq!(form.submit(), Effect::Nothing);
        type_text(&mut form, "https://example.invalid/repo.git");
        assert_eq!(form.submit(), Effect::Nothing, "destination is still empty");
        assert!(!form.working);
    }

    #[test]
    fn tab_walks_the_four_fields_in_order_and_back() {
        let mut form = form();
        assert_eq!(form.focus, CloneField::Url);
        for expected in [CloneField::Destination, CloneField::Branch, CloneField::Depth, CloneField::Url] {
            form.edit(Key::Tab);
            assert_eq!(form.focus, expected);
        }
        form.edit(Key::BackTab);
        assert_eq!(form.focus, CloneField::Depth);
    }

    #[test]
    fn a_review_request_carries_every_typed_value_once() {
        let mut form = form();
        type_text(&mut form, "https://example.invalid/r.git");
        form.edit(Key::Tab);
        type_text(&mut form, "D:/tmp/target");
        form.edit(Key::Tab);
        type_text(&mut form, "main");
        form.edit(Key::Tab);
        type_text(&mut form, "5");
        let Effect::Send(Job::PreviewClone { url, destination, branch, depth }) = form.submit() else {
            panic!("expected a preview request");
        };
        assert_eq!(url, "https://example.invalid/r.git");
        assert_eq!(destination, NativePath::new("D:/tmp/target"));
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(depth, Some(5));
        assert!(form.working);
        assert_eq!(form.submit(), Effect::Nothing, "a second submit while working is refused");
    }

    #[test]
    fn a_blank_branch_and_depth_mean_the_remote_default_and_a_full_clone() {
        let mut form = form();
        type_text(&mut form, "https://example.invalid/r.git");
        form.edit(Key::Tab);
        type_text(&mut form, "/tmp/target");
        let Effect::Send(Job::PreviewClone { branch, depth, .. }) = form.submit() else {
            panic!("expected a preview request");
        };
        assert!(branch.is_none());
        assert!(depth.is_none());
    }

    #[test]
    fn a_zero_or_garbage_depth_is_refused_rather_than_sent() {
        assert_eq!(parse_depth(""), Ok(None));
        assert_eq!(parse_depth("3"), Ok(Some(3)));
        assert!(parse_depth("0").is_err());
        assert!(parse_depth("-1").is_err());
        assert!(parse_depth("deep").is_err());
    }

    #[test]
    fn a_reviewed_form_refuses_edits_and_sends_only_its_ticket() {
        let mut form = form();
        type_text(&mut form, "https://example.invalid/r.git");
        form.edit(Key::Tab);
        type_text(&mut form, "/tmp/target");
        form.submit();
        form.working = false;
        form.review = Some(("ticket-1".into(), vec!["clone into /tmp/target".into()]));
        assert_eq!(form.edit(Key::Char('x')), Effect::Nothing, "review blocks editing");
        assert_eq!(form.approve(), Effect::Send(Job::ExecuteClone("ticket-1".into())));
        assert!(form.working);
    }

    #[test]
    fn a_form_without_a_review_cannot_approve() {
        let mut form = form();
        assert_eq!(form.approve(), Effect::Nothing);
        assert!(!form.working);
    }
}

//! The merge form: one target, then the review the backend produced.
//!
//! The form never merges. It sends \`PreviewMerge\`, shows the catalogued review
//! the backend returned with a one-use ticket, and only \`y\` sends
//! \`ApplyMerge(ticket)\`. A field edited after a review cannot be merged,
//! because the ticket is bound to the preview, not to the text on screen.

use minicon_core::composer::{self, ComposerState, Move};

use crate::{Effect, Job, Key};

/// The bound on the target field, in UTF-8 bytes.
const TARGET_LIMIT: usize = 1024;

pub struct MergeForm {
    /// True when the form replays the branch instead of merging into it. The
    /// two share one surface because they share one shape: a target, a review,
    /// an approval.
    pub rebase: bool,
    /// The target exactly as typed: a branch, a remote-tracking name, a tag or
    /// an object id. The backend decides what it resolves to.
    pub target: ComposerState,
    pub working: bool,
    /// Ticket and review lines. Present means the form is showing a review and
    /// refuses edits until it is approved or cancelled.
    pub review: Option<(String, Vec<String>)>,
}

impl Default for MergeForm {
    fn default() -> Self {
        Self::new()
    }
}

impl MergeForm {
    pub fn new() -> Self {
        Self { rebase: false, target: ComposerState::default(), working: false, review: None }
    }

    /// The same surface, asking Git to replay the branch onto the target.
    pub fn new_rebase() -> Self {
        Self { rebase: true, ..Self::new() }
    }

    /// The exact binding the backend issued, if a review is showing.
    pub fn ticket(&self) -> Option<String> {
        self.review.as_ref().map(|(ticket, _)| ticket.clone())
    }

    /// Asks for a preview. Nothing is merged and nothing is written.
    pub fn submit(&mut self) -> Effect {
        if self.working || self.review.is_some() {
            return Effect::Nothing;
        }
        let target = self.target.text.trim().to_owned();
        if target.is_empty() {
            return Effect::Nothing;
        }
        self.working = true;
        Effect::Send(if self.rebase { Job::PreviewRebase(target) } else { Job::PreviewMerge(target) })
    }

    /// Approves the reviewed merge. The ticket is one-use.
    pub fn approve(&mut self) -> Effect {
        match self.ticket() {
            Some(ticket) => {
                self.working = true;
                Effect::Send(if self.rebase { Job::ApplyRebase(ticket) } else { Job::ApplyMerge(ticket) })
            }
            None => Effect::Nothing,
        }
    }

    pub fn edit(&mut self, key: Key) -> Effect {
        if self.working || self.review.is_some() {
            return Effect::Nothing;
        }
        if !self.target.preedit.is_empty() {
            if key == Key::Escape {
                self.target.preedit.clear();
            }
            return Effect::Nothing;
        }
        match key {
            Key::Enter => return self.submit(),
            Key::SelectAll => composer::select_all(&mut self.target),
            Key::CtrlC | Key::Cut => {
                return composer::selection_text(&self.target).map_or(Effect::Nothing, |text| {
                    Effect::Copy { text: text.into(), cut: key == Key::Cut }
                });
            }
            Key::Paste => return Effect::Paste,
            Key::Backspace => composer::backspace(&mut self.target),
            Key::Delete => composer::delete_forward(&mut self.target),
            Key::Left => composer::move_caret(&mut self.target, Move::Left),
            Key::Right => composer::move_caret(&mut self.target, Move::Right),
            Key::Home => composer::move_caret(&mut self.target, Move::LineStart),
            Key::End => composer::move_caret(&mut self.target, Move::LineEnd),
            Key::EditMove { movement, extend } => {
                if extend {
                    composer::extend_selection(&mut self.target, movement);
                } else {
                    composer::move_caret(&mut self.target, movement);
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
        let removed = composer::selection_bounds(&self.target).map_or(0, |(a, b)| b - a);
        if text.contains(['\n', '\r', '\0']) || self.target.text.len() - removed + text.len() > TARGET_LIMIT {
            return Effect::Nothing;
        }
        composer::insert(&mut self.target, text);
        Effect::Nothing
    }
}

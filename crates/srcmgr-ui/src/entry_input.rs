//! One active single-line entry field, using MiniCon's editor mechanisms.
use crate::{Effect, Key};
use minicon_core::composer::{self, ComposerState, Move};

/// Reserve the caret and both cells of the painter's `..` truncation marker.
/// Paint, pointer hit and IME positioning must share this horizontal window.
pub(crate) fn viewport(editor: &ComposerState, columns: usize) -> composer::VisibleWindow {
    composer::visible_window(&editor.text, &editor.preedit, editor.caret, 3, columns)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Slot {
    Prompt,
    CreatePath,
    CreateBranch,
    IdentityName,
    IdentityEmail,
}

pub(crate) struct EntryInput {
    pub slot: Slot,
    pub context: u64,
    pub id: u64,
    pub editor: ComposerState,
    limit: usize,
}

impl EntryInput {
    pub fn new(slot: Slot, context: u64, text: &str, limit: usize) -> Self {
        let mut editor = ComposerState::default();
        editor.text = text.into();
        editor.caret = text.len();
        editor.focused = true;
        Self {
            slot,
            context,
            id: crate::tools::context_id(),
            limit,
            editor,
        }
    }

    fn validate(&self, text: &str, removed: usize) -> Result<(), String> {
        if text.contains(['\r', '\n', '\0']) {
            return Err("Use one line without CR, LF or NUL; input was not changed".into());
        }
        if self
            .editor
            .text
            .len()
            .saturating_sub(removed)
            .saturating_add(text.len())
            > self.limit
        {
            return Err(format!(
                "Field limit is {} UTF-8 bytes; input was not changed",
                self.limit
            ));
        }
        Ok(())
    }

    pub fn replace(&mut self, text: &str) -> Result<Effect, String> {
        if !self.editor.preedit.is_empty() {
            return Err("Finish or cancel IME composition before replacing the field".into());
        }
        self.validate(text, self.editor.text.len())?;
        self.id = crate::tools::context_id();
        composer::select_all(&mut self.editor);
        composer::insert(&mut self.editor, text);
        Ok(Effect::Nothing)
    }

    pub fn key(&mut self, key: Key) -> Result<Effect, String> {
        if !matches!(key, Key::CtrlC | Key::Cut | Key::Paste) {
            self.id = crate::tools::context_id();
        }
        if !self.editor.preedit.is_empty() {
            if key == Key::Escape {
                self.editor.preedit.clear();
            }
            return Ok(Effect::Nothing);
        }
        match key {
            Key::SelectAll => composer::select_all(&mut self.editor),
            Key::CtrlC | Key::Cut => {
                return Ok(composer::selection_text(&self.editor).map_or(
                    Effect::Nothing,
                    |text| Effect::Copy {
                        text: text.into(),
                        cut: key == Key::Cut,
                    },
                ));
            }
            Key::Paste => return Ok(Effect::Paste),
            Key::Backspace => composer::backspace(&mut self.editor),
            Key::Delete => composer::delete_forward(&mut self.editor),
            Key::Left => composer::move_caret(&mut self.editor, Move::Left),
            Key::Right => composer::move_caret(&mut self.editor, Move::Right),
            Key::Home => composer::move_caret(&mut self.editor, Move::LineStart),
            Key::End => composer::move_caret(&mut self.editor, Move::LineEnd),
            Key::EditMove { movement, extend } => {
                if extend {
                    composer::extend_selection(&mut self.editor, movement);
                } else {
                    composer::move_caret(&mut self.editor, movement);
                }
            }
            Key::Char(_) | Key::Space | Key::Text(_) | Key::Pasted(_) => {
                let text = match key {
                    Key::Char(ch) => ch.to_string(),
                    Key::Space => " ".into(),
                    Key::Text(text) | Key::Pasted(text) => text,
                    _ => unreachable!(),
                };
                let removed = composer::selection_bounds(&self.editor).map_or(0, |(a, b)| b - a);
                self.validate(&text, removed)?;
                composer::insert(&mut self.editor, &text);
            }
            _ => {}
        }
        Ok(Effect::Nothing)
    }
}

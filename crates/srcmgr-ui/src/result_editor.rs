//! The result buffer for the in-app conflict editor (PRD 08, PRD 10).
//!
//! This is state, not paint: it holds the text, the caret, the selection and a
//! bounded undo history, and it refuses to grow past the same limit the conflict
//! center applies. The window draws it and the backend saves it; neither reaches
//! into the fields directly.
use crate::{Effect, Key};

/// The largest buffer this editor holds, in bytes of UTF-8.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// The largest line count this editor holds.
pub const MAX_LINES: usize = 200_000;

/// Undo steps kept. Each step is the text before an edit, so the cost is real
/// and the number is deliberately small.
const UNDO_STEPS: usize = 32;

struct Snapshot {
    text: String,
    caret: usize,
    anchor: Option<usize>,
}

pub struct ResultEditor {
    pub text: String,
    /// Caret position as a byte index on a character boundary.
    pub caret: usize,
    /// The other end of the selection, when one is active.
    anchor: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// First visible line, kept by the renderer.
    pub scroll: usize,
}

impl ResultEditor {
    pub fn new(text: String) -> Self {
        let caret = text.len();
        Self { text, caret, anchor: None, undo: Vec::new(), redo: Vec::new(), scroll: 0 }
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.text.split('\n')
    }

    pub fn line_count(&self) -> usize {
        self.text.matches('\n').count() + 1
    }

    /// The caret as a zero-based line index and a character column, so the
    /// renderer never has to convert byte offsets itself.
    pub fn caret_position(&self) -> (usize, usize) {
        let before = &self.text[..self.caret];
        let line = before.matches('\n').count();
        let column = before.rsplit('\n').next().map_or(0, |tail| tail.chars().count());
        (line, column)
    }

    /// The selected byte range, ordered low to high.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        if anchor == self.caret {
            None
        } else if anchor < self.caret {
            Some((anchor, self.caret))
        } else {
            Some((self.caret, anchor))
        }
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|(start, end)| &self.text[start..end])
    }

    fn push_undo(&mut self) {
        self.undo.push(Snapshot { text: self.text.clone(), caret: self.caret, anchor: self.anchor });
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn delete_selection(&mut self) -> bool {
        if let Some((start, end)) = self.selection() {
            self.push_undo();
            self.text.replace_range(start..end, "");
            self.caret = start;
            self.anchor = None;
            return true;
        }
        false
    }

    /// Inserts committed text. A replace that would cross either bound is
    /// refused whole, so the buffer never holds a partial insertion.
    pub fn insert(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let removed = self.selection().map_or(0, |(start, end)| end - start);
        let bytes_after = self.text.len() - removed + text.len();
        let lines_after = self.line_count() + text.matches('\n').count() - self.text[self.selection().map_or(self.caret..self.caret, |(s, e)| s..e)].matches('\n').count();
        if bytes_after > MAX_BYTES || lines_after > MAX_LINES {
            return false;
        }
        self.push_undo();
        if let Some((start, end)) = self.selection() {
            self.text.replace_range(start..end, text);
            self.caret = start + text.len();
        } else {
            self.text.insert_str(self.caret, text);
            self.caret += text.len();
        }
        self.anchor = None;
        true
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.caret == 0 {
            return;
        }
        let previous = self.text[..self.caret].chars().next_back().map_or(0, |character| character.len_utf8());
        self.push_undo();
        self.text.replace_range(self.caret - previous..self.caret, "");
        self.caret -= previous;
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.caret >= self.text.len() {
            return;
        }
        let next = self.text[self.caret..].chars().next().map_or(0, |character| character.len_utf8());
        self.push_undo();
        self.text.replace_range(self.caret..self.caret + next, "");
    }

    fn line_start(&self, position: usize) -> usize {
        self.text[..position].rfind('\n').map_or(0, |index| index + 1)
    }

    fn line_end(&self, position: usize) -> usize {
        self.text[position..].find('\n').map_or(self.text.len(), |index| position + index)
    }

    /// The caret's column, in characters, from the start of its line.
    fn column(&self) -> usize {
        self.text[self.line_start(self.caret)..self.caret].chars().count()
    }

    pub fn move_left(&mut self, extend: bool) {
        let target = if let Some((start, _)) = self.selection().filter(|_| !extend) {
            start
        } else if self.caret == 0 {
            0
        } else {
            self.caret - self.text[..self.caret].chars().next_back().map_or(0, |character| character.len_utf8())
        };
        self.set_caret(target, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        let target = if let Some((_, end)) = self.selection().filter(|_| !extend) {
            end
        } else if self.caret >= self.text.len() {
            self.text.len()
        } else {
            self.caret + self.text[self.caret..].chars().next().map_or(0, |character| character.len_utf8())
        };
        self.set_caret(target, extend);
    }

    pub fn move_vertical(&mut self, lines: isize, extend: bool) {
        let column = self.column();
        let mut start = self.line_start(self.caret);
        let mut moved = 0;
        while moved != lines {
            if lines < 0 {
                if start == 0 {
                    break;
                }
                start = self.line_start(start - 1);
                moved -= 1;
            } else {
                let end = self.line_end(start);
                if end >= self.text.len() {
                    break;
                }
                start = end + 1;
                moved += 1;
            }
        }
        let end = self.line_end(start);
        let target = self.text[start..end]
            .char_indices()
            .nth(column)
            .map_or(end, |(index, _)| start + index);
        self.set_caret(target, extend);
    }

    pub fn move_home(&mut self, extend: bool) {
        let target = self.line_start(self.caret);
        self.set_caret(target, extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        let target = self.line_end(self.caret);
        self.set_caret(target, extend);
    }

    fn set_caret(&mut self, target: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = target;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.caret = self.text.len();
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else { return false };
        self.redo.push(Snapshot { text: self.text.clone(), caret: self.caret, anchor: self.anchor });
        self.text = previous.text;
        self.caret = previous.caret.min(self.text.len());
        self.anchor = previous.anchor;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(Snapshot { text: self.text.clone(), caret: self.caret, anchor: self.anchor });
        self.text = next.text;
        self.caret = next.caret.min(self.text.len());
        self.anchor = next.anchor;
        true
    }

    /// Handles one editing key. Anything the buffer does not own is ignored, so
    /// the window decides which keys belong here.
    pub fn edit(&mut self, key: Key) -> Effect {
        match key {
            Key::SelectAll => self.select_all(),
            Key::Undo => {
                self.undo();
            }
            Key::Redo => {
                self.redo();
            }
            Key::CtrlC | Key::Cut => {
                return self.selected_text().map_or(Effect::Nothing, |text| Effect::Copy {
                    text: text.to_owned(),
                    cut: key == Key::Cut,
                });
            }
            Key::Paste => return Effect::Paste,
            Key::Backspace => self.backspace(),
            Key::Delete => self.delete_forward(),
            Key::Left => self.move_left(false),
            Key::Right => self.move_right(false),
            Key::Up => self.move_vertical(-1, false),
            Key::Down => self.move_vertical(1, false),
            Key::Home => self.move_home(false),
            Key::End => self.move_end(false),
            Key::EditMove { movement, extend } => match movement {
                minicon_core::composer::Move::Left => self.move_left(extend),
                minicon_core::composer::Move::Right => self.move_right(extend),
                minicon_core::composer::Move::Up => self.move_vertical(-1, extend),
                minicon_core::composer::Move::Down => self.move_vertical(1, extend),
                minicon_core::composer::Move::LineStart => self.move_home(extend),
                minicon_core::composer::Move::LineEnd => self.move_end(extend),
                // Word and draft moves are single-line field movements; a text
                // buffer does not offer them in this slice.
                _ => {}
            },
            Key::Char(character) => {
                self.insert(&character.to_string());
            }
            Key::Enter => {
                self.insert("\n");
            }
            Key::Space => {
                self.insert(" ");
            }
            Key::Text(text) | Key::Pasted(text) => {
                self.insert(&text);
            }
            _ => {}
        }
        Effect::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str) -> ResultEditor {
        let mut editor = ResultEditor::new(text.to_owned());
        editor.caret = 0;
        editor
    }

    #[test]
    fn typing_lands_at_the_caret_and_moves_it() {
        let mut buffer = editor("one\ntwo\n");
        assert!(buffer.insert("zero\n"));
        assert_eq!(buffer.text, "zero\none\ntwo\n");
        assert_eq!(buffer.caret, 5);
        assert_eq!(buffer.line_count(), 4);
    }

    #[test]
    fn enter_inserts_a_newline_and_the_caret_follows_it() {
        let mut buffer = editor("ono");
        buffer.caret = 2;
        buffer.edit(Key::Enter);
        assert_eq!(buffer.text, "on\no");
        assert_eq!(buffer.caret_position(), (1, 0));
    }

    #[test]
    fn one_committed_ime_text_lands_once_at_the_caret() {
        let mut buffer = editor("one\n");
        buffer.caret = 4;
        buffer.edit(Key::Text("中文".into()));
        assert_eq!(buffer.text, "one\n中文");
        assert_eq!(buffer.caret_position(), (1, 2));
        assert!(buffer.undo(), "one commit is one undo step");
        assert_eq!(buffer.text, "one\n");
    }

    #[test]
    fn a_selection_is_replaced_whole() {
        let mut buffer = editor("one\ntwo\n");
        buffer.select_all();
        assert_eq!(buffer.selected_text(), Some("one\ntwo\n"));
        assert!(buffer.insert("both\n"));
        assert_eq!(buffer.text, "both\n");
        assert_eq!(buffer.selection(), None);
    }

    #[test]
    fn backspace_at_a_line_start_joins_the_previous_line() {
        let mut buffer = editor("one\ntwo\n");
        buffer.caret = 4;
        buffer.backspace();
        assert_eq!(buffer.text, "onetwo\n");
        assert_eq!(buffer.caret, 3);
    }

    #[test]
    fn moving_down_keeps_the_column_and_clamps_at_a_short_line() {
        let mut buffer = editor("long line\nx\n");
        buffer.caret = 6;
        buffer.move_vertical(1, false);
        assert_eq!(buffer.caret, buffer.text.find('x').unwrap() + 1, "a short line clamps to its end");
        buffer.caret = 6;
        buffer.move_vertical(1, true);
        assert!(buffer.selection().is_some(), "extending keeps an anchor");
    }

    #[test]
    fn an_insertion_beyond_the_byte_bound_is_refused_whole() {
        let mut buffer = ResultEditor::new("x".repeat(MAX_BYTES - 1));
        assert!(buffer.insert("a"));
        assert!(!buffer.insert("b"), "the bound is enforced before any change");
        assert_eq!(buffer.text.len(), MAX_BYTES);
    }

    #[test]
    fn undo_and_redo_walk_the_same_steps_back_and_forward() {
        let mut buffer = editor("one\n");
        buffer.insert("two\n");
        buffer.insert("three\n");
        assert!(buffer.undo());
        assert_eq!(buffer.text, "two\none\n");
        assert!(buffer.undo());
        assert_eq!(buffer.text, "one\n");
        assert!(buffer.redo());
        assert_eq!(buffer.text, "two\none\n");
    }

    #[test]
    fn the_line_bound_is_counted_separately_from_bytes() {
        let mut buffer = ResultEditor::new("\n".repeat(MAX_LINES - 2));
        assert!(buffer.insert("\n"));
        assert!(!buffer.insert("\n"), "the line bound refuses the next line");
    }
}

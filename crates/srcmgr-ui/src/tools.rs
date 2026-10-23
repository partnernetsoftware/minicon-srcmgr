//! Shared MiniCon text editing for optional tool preferences and launch review.
use minicon_core::composer::{self, ComposerState, Move};
use srcmgr_domain::{ExternalTool, NativePath, ToolKind};
use crate::{Effect, Job, Key};

fn context_id() -> u64 { use std::sync::atomic::{AtomicU64, Ordering}; static NEXT: AtomicU64 = AtomicU64::new(1 << 63); NEXT.fetch_add(1, Ordering::Relaxed) }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolWork { Load, Save, Review, Launch }

pub struct ToolForm {
    pub kind: ToolKind,
    pub executable: ComposerState,
    pub arguments: ComposerState,
    pub argument_focus: bool,
    pub working: Option<ToolWork>,
    pub review: Option<(String, Vec<String>)>,
    pub detail: String,
    pub id: u64,
    pub target: Option<NativePath>,
    original: Option<ExternalTool>,
    native_only: bool,
}


impl ToolForm {
    pub fn new(kind: ToolKind, target: Option<NativePath>) -> Self {
        Self { kind, executable: Default::default(), arguments: Default::default(), argument_focus: false, working: Some(ToolWork::Load), review: None, detail: "Loading application tool preference...".into(), id: context_id(), target, original: None, native_only: false }
    }

    pub fn load(&mut self, definition: Option<ExternalTool>, detail: String) {
        self.id = context_id();
        self.executable = Default::default(); self.arguments = Default::default();
        self.native_only = definition.as_ref().is_some_and(|d| d.executable.as_os_str().to_str().is_none());
        if let Some(d) = &definition {
            self.executable.text = d.executable.as_os_str().to_str().unwrap_or("").into();
            self.arguments.text = d.arguments.join("\n");
        } else if self.kind == ToolKind::Editor { self.arguments.text = "{file}".into(); }
        self.executable.caret = self.executable.text.len(); self.arguments.caret = self.arguments.text.len();
        self.original = definition; self.detail = detail; self.working = None; self.review = None;
        if self.native_only { self.detail.push_str(" The saved native executable path cannot be edited as Unicode; it is retained unless replaced explicitly."); }
    }

    pub fn definition(&self) -> Result<Option<ExternalTool>, String> {
        let executable = if self.executable.text.is_empty() && self.native_only {
            self.original.as_ref().unwrap().executable.clone()
        } else if self.executable.text.is_empty() { return Ok(None); }
        else { NativePath::new(self.executable.text.clone()) };
        let arguments = if self.arguments.text.is_empty() { Vec::new() } else { self.arguments.text.split('\n').map(str::to_owned).collect() };
        let value = ExternalTool { executable, arguments };
        value.validate(self.kind)?;
        Ok(Some(value))
    }

    pub fn editor(&self) -> &ComposerState { if self.argument_focus { &self.arguments } else { &self.executable } }
    pub fn focus(&mut self, arguments: bool) { self.argument_focus = arguments; self.id = context_id(); }
    pub fn original_definition(&self) -> Option<&ExternalTool> { self.original.as_ref() }
    pub fn editor_mut(&mut self) -> &mut ComposerState { if self.argument_focus { &mut self.arguments } else { &mut self.executable } }
    pub fn composed(&self) -> bool { !self.executable.preedit.is_empty() || !self.arguments.preedit.is_empty() }

    pub fn save(&mut self, clear: bool) -> Effect {
        if self.working.is_some() || self.review.is_some() || self.composed() { return Effect::Nothing; }
        let definition = if clear { None } else { match self.definition() { Ok(v) => v, Err(e) => { self.detail = e; return Effect::Nothing; } } };
        self.id = context_id();
        self.working = Some(ToolWork::Save);
        Effect::Send(Job::SaveToolSettings { kind: self.kind, definition })
    }

    pub fn prepare(&mut self) -> Effect {
        if self.working.is_some() || self.review.is_some() || self.composed() { return Effect::Nothing; }
        match self.definition() {
            Ok(value) if value == self.original => {},
            _ => { self.detail = "Save changed settings before reviewing launch. Save does not run the tool.".into(); return Effect::Nothing; }
        }
        self.working = Some(ToolWork::Review);
        self.id = context_id();
        Effect::Send(Job::PrepareTool { kind: self.kind, file: self.target.clone() })
    }

    pub fn approve(&mut self) -> Effect {
        if self.working.is_some() { return Effect::Nothing; }
        let Some((ticket, _)) = &self.review else { return Effect::Nothing; };
        self.working = Some(ToolWork::Launch);
        Effect::Send(Job::LaunchTool(ticket.clone()))
    }

    pub fn edit_key(&mut self, key: Key) -> Effect {
        if self.working.is_some() || self.review.is_some() { return Effect::Nothing; }
        if self.composed() { if key == Key::Escape { self.editor_mut().preedit.clear(); } return Effect::Nothing; }
        if key == Key::Tab || key == Key::BackTab { self.focus(!self.argument_focus); return Effect::Nothing; }
        if key == Key::Save { return self.save(false); }
        if key == Key::CtrlEnter { return self.prepare(); }
        let argument_focus = self.argument_focus;
        let editor = self.editor_mut();
        match key {
            Key::SelectAll => composer::select_all(editor),
            Key::CtrlC | Key::Cut => return composer::selection_text(editor).map_or(Effect::Nothing, |s| Effect::Copy { text: s.into(), cut: key == Key::Cut }),
            Key::Paste => return Effect::Paste,
            Key::Backspace => composer::backspace(editor), Key::Delete => composer::delete_forward(editor),
            Key::Left => composer::move_caret(editor, Move::Left), Key::Right => composer::move_caret(editor, Move::Right),
            Key::Home => composer::move_caret(editor, Move::LineStart), Key::End => composer::move_caret(editor, Move::LineEnd),
            Key::Up => composer::move_caret(editor, Move::Up), Key::Down => composer::move_caret(editor, Move::Down),
            Key::EditMove { movement, extend } => if extend { composer::extend_selection(editor, movement); } else { composer::move_caret(editor, movement); },
            Key::Enter if argument_focus => return self.insert("\n"),
            Key::Char(ch) => return self.insert(&ch.to_string()), Key::Space => return self.insert(" "),
            Key::Text(text) | Key::Pasted(text) => return self.insert(&text),
            _ => {},
        }
        Effect::Nothing
    }

    fn insert(&mut self, text: &str) -> Effect {
        let normalized = self.argument_focus.then(|| text.replace("\r\n", "\n"));
        let text = normalized.as_deref().unwrap_or(text);
        let limit = if self.argument_focus { 16 * 1024 + 31 } else { 32 * 1024 };
        if text.contains('\0') || (!self.argument_focus && text.contains(['\r', '\n'])) || text.contains('\r') { self.detail = "Input contains unsupported line breaks or NUL; nothing was inserted.".into(); return Effect::Nothing; }
        let editor = self.editor_mut();
        let removed = composer::selection_bounds(editor).map_or(0, |(a,b)| b-a);
        if editor.text.len() - removed + text.len() > limit { self.detail = format!("Field limit is {limit} UTF-8 bytes; nothing was inserted."); return Effect::Nothing; }
        composer::insert(editor, text); Effect::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Workspace, Snapshot, Reply, PromptAction};

    #[test]
    fn tool_fields_keep_literal_input_and_fence_clipboard_across_focus_close_and_review() {
        let mut ui = Workspace::new(Snapshot::welcome());
        assert_eq!(ui.key(Key::Char('T')), Effect::Send(Job::LoadToolSettings(ToolKind::Editor)));
        ui.reply(Reply::ToolSettings { kind: ToolKind::Editor, definition: None, detail: "passive".into() });
        ui.key(Key::Text("C:/tools/editor 中文.exe".into()));
        ui.key(Key::SelectAll);
        let id = ui.clipboard_context().unwrap();
        ui.key(Key::Tab);
        ui.complete_clipboard(id, Some("wrong field".into()), None);
        assert_eq!(ui.tool_form().unwrap().arguments.text, "{file}");
        ui.key(Key::Enter); ui.key(Key::Text(" ; & $() \"quoted\" ".into()));
        assert!(ui.tool_form().unwrap().arguments.text.ends_with(" ; & $() \"quoted\" "));
        ui.ime_preedit("未完成".into());
        assert_eq!(ui.key(Key::Save), Effect::Nothing);
        ui.ime_commit("字".into());
        let Effect::Send(Job::SaveToolSettings { kind, definition: Some(definition) }) = ui.key(Key::Save) else { panic!() };
        assert_eq!(kind, ToolKind::Editor);
        assert_eq!(definition.arguments, ["{file}", " ; & $() \"quoted\" 字"]);
        let previous = ui.tool_form().unwrap().arguments.text.clone();
        ui.reply(Reply::PromptFailed { action: PromptAction::ExternalTool, text: "profile locked".into() });
        assert_eq!(ui.tool_form().unwrap().arguments.text, previous);
        ui.key(Key::Save);
        ui.reply(Reply::ToolSettings { kind, definition: Some(definition), detail: "Saved".into() });
        assert!(matches!(ui.prepare_tool(), Effect::Send(Job::PrepareTool { kind: ToolKind::Editor, file: None })));
        ui.reply(Reply::ToolReview { ticket: "one-use".into(), lines: vec!["exact target".into()] });
        assert_eq!(ui.key(Key::Enter), Effect::Nothing, "held Enter cannot launch");
        assert_eq!(ui.key(Key::NextRepository), Effect::Nothing);
        assert_eq!(ui.approve_tool(), Effect::Send(Job::LaunchTool("one-use".into())));
        assert_eq!(ui.close_tool(), Effect::Nothing, "submitted launch cannot pretend to cancel");
        ui.reply(Reply::PromptFailed { action: PromptAction::ExternalTool, text: "target changed".into() });
        assert!(ui.tool_form().unwrap().review.is_none());
        let id = ui.clipboard_context().unwrap();
        ui.close_tool(); ui.request_tools();
        ui.reply(Reply::ToolSettings { kind, definition: None, detail: "new form".into() });
        ui.complete_clipboard(id, Some("late paste".into()), None);
        assert!(ui.tool_form().unwrap().executable.text.is_empty());
    }
}

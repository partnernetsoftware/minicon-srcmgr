//! Recent-location metadata editing with shared MiniCon text mechanisms.
use minicon_core::composer::{self, ComposerState, Move};
use srcmgr_domain::NativePath;
use crate::{Effect, Job, Key, RecentRow};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocateWork { Review, Save }

pub struct LocateForm {
    pub source: RecentRow,
    pub editor: ComposerState,
    pub id: u64,
    pub working: Option<LocateWork>,
    pub review: Option<(String, Vec<String>)>,
    pub detail: String,
}


impl LocateForm {
    pub fn new(source: RecentRow) -> Self {
        let mut editor = ComposerState::default();
        editor.text = NativePath::from_encoded(source.path.clone()).ok().and_then(|p| p.as_os_str().to_str().map(str::to_owned)).unwrap_or_default();
        editor.caret = editor.text.len();
        composer::select_all(&mut editor);
        Self { source, editor, id: crate::tools::context_id(), working: None, review: None, detail: "Enter the absolute local path of the repository. Review changes only this recent record; Open afterward checks trust. Old-path drafts and saved views remain separate.".into() }
    }

    pub fn submit(&mut self) -> Effect {
        if self.working.is_some() || self.review.is_some() || !self.editor.preedit.is_empty() || self.editor.text.is_empty() { return Effect::Nothing; }
        self.working = Some(LocateWork::Review); self.id = crate::tools::context_id();
        Effect::Send(Job::PrepareLocate { backend: self.source.backend.clone(), source: self.source.path.clone(), target: NativePath::new(self.editor.text.clone()) })
    }

    pub fn approve(&mut self) -> Effect {
        if self.working.is_some() { return Effect::Nothing; }
        let Some((ticket, _)) = &self.review else { return Effect::Nothing; };
        self.working = Some(LocateWork::Save);
        Effect::Send(Job::SaveLocated(ticket.clone()))
    }

    pub fn edit(&mut self, key: Key) -> Effect {
        if self.working.is_some() || self.review.is_some() { return Effect::Nothing; }
        if !self.editor.preedit.is_empty() { if key == Key::Escape { self.editor.preedit.clear(); } return Effect::Nothing; }
        match key {
            Key::Enter => return self.submit(),
            Key::SelectAll => composer::select_all(&mut self.editor),
            Key::CtrlC | Key::Cut => return composer::selection_text(&self.editor).map_or(Effect::Nothing, |text| Effect::Copy { text: text.into(), cut: key == Key::Cut }),
            Key::Paste => return Effect::Paste,
            Key::Backspace => composer::backspace(&mut self.editor), Key::Delete => composer::delete_forward(&mut self.editor),
            Key::Left => composer::move_caret(&mut self.editor, Move::Left), Key::Right => composer::move_caret(&mut self.editor, Move::Right),
            Key::Home => composer::move_caret(&mut self.editor, Move::LineStart), Key::End => composer::move_caret(&mut self.editor, Move::LineEnd),
            Key::EditMove { movement, extend } => if extend { composer::extend_selection(&mut self.editor, movement); } else { composer::move_caret(&mut self.editor, movement); },
            Key::Char(ch) => return self.insert(&ch.to_string()), Key::Space => return self.insert(" "),
            Key::Text(text) | Key::Pasted(text) => return self.insert(&text),
            _ => {},
        }
        Effect::Nothing
    }

    fn insert(&mut self, text: &str) -> Effect {
        let removed = composer::selection_bounds(&self.editor).map_or(0, |(a,b)| b-a);
        if text.contains(['\n','\r','\0']) || self.editor.text.len() - removed + text.len() > 32 * 1024 { self.detail = "Use one line up to 32 KiB UTF-8, without NUL; nothing was inserted.".into(); return Effect::Nothing; }
        composer::insert(&mut self.editor, text);
        Effect::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Workspace, Snapshot, Reply, PromptAction};

    #[test]
    fn locate_preserves_raw_source_input_and_explicit_save_without_opening() {
        let old = RecentRow { backend: "git".into(), path: NativePath::new("old 中文").encoded(), label: "escaped display label, not a path".into() };
        let mut ui = Workspace::new(Snapshot::welcome());
        ui.reply(Reply::Welcome(vec![old.clone()]));
        let Effect::Send(Job::EnterRepository { path: Some(path),.. }) = ui.open_recent() else { panic!() };
        assert_eq!(path,NativePath::new("old 中文"));
        ui.reply(Reply::PromptFailed { action: PromptAction::OpenRepository,text:"missing directory".into() });
        ui.request_locate();
        assert_eq!(ui.locate_form().unwrap().source,old);
        let old_id = ui.clipboard_context().unwrap();
        ui.cancel_locate(); ui.request_locate();
        ui.complete_clipboard(old_id,Some("late paste".into()),None);
        assert_eq!(ui.locate_form().unwrap().editor.text,"old 中文");
        ui.ime_preedit("中".into());
        assert_eq!(ui.key(Key::Enter),Effect::Nothing);
        ui.ime_commit("new 中文 & ; $()".into());
        let text = ui.locate_form().unwrap().editor.text.clone();
        let Effect::Send(Job::PrepareLocate { backend,source,target }) = ui.key(Key::Enter) else { panic!() };
        assert_eq!(backend,"git"); assert_eq!(source,old.path); assert_eq!(target,NativePath::new(text.clone()));
        ui.reply(Reply::PromptFailed { action: PromptAction::LocateRepository,text:"profile unavailable".into() });
        assert_eq!(ui.locate_form().unwrap().editor.text,text);
        ui.key(Key::Enter); ui.reply(Reply::LocateReview { ticket:"first".into(),lines:vec!["old -> new".into()] });
        assert_eq!(ui.key(Key::Enter),Effect::Nothing,"held Enter cannot save a location");
        assert_eq!(ui.key(Key::NextRepository),Effect::Nothing);
        assert_eq!(ui.key(Key::Escape),Effect::Send(Job::CancelLocate));
        assert_eq!(ui.snapshot().recents,vec![old.clone()]);
        assert_eq!(ui.locate_form().unwrap().editor.text,text);
        ui.submit_locate(); ui.reply(Reply::LocateReview { ticket:"fresh".into(),lines:vec![] });
        assert_eq!(ui.approve_locate(),Effect::Send(Job::SaveLocated("fresh".into())));
        assert_eq!(ui.cancel_locate(),Effect::Nothing,"submitted save cannot pretend to cancel");
        let other = RecentRow { backend:"git".into(),path:NativePath::new("other").encoded(),label:"other".into() };
        let new = RecentRow { backend:"git".into(),path:NativePath::new(text).encoded(),label:"new".into() };
        assert_eq!(ui.reply(Reply::RecentLocated { entries:vec![other,new.clone()], target:new.path.clone(),detail:"Saved; Open explicitly".into() }),Effect::Nothing);
        assert!(ui.is_welcome() && !ui.modal());
        assert_eq!(ui.selected(crate::Pane::Files),1);
        assert_eq!(ui.forget_recent(),Effect::Send(Job::ForgetRecent { backend:new.backend,path:new.path }));
        assert!(ui.snapshot().repository.is_none(),"saving metadata never opens a repository");
    }
}

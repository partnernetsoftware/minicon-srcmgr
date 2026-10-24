//! The window's state: which pane has focus, what is selected, what the main
//! panel shows and which job each key asks the backend to run. Pure logic, so
//! it is tested without a window.
//!
//! Panels and keys follow lazygit: 1 Status, 2 Files, 3 Branches, 4 Commits.
//! In the main panel a cursor moves over the diff; Space stages the cursor line
//! (or a `v` range of lines) and `a` stages the whole hunk. See PRD 10 for the
//! full list and the deliberate differences.

use srcmgr_domain::NativePath;
use minicon_core::composer::{self, ComposerState, Move};
use std::time::{Duration, Instant};
use crate::model::{Job, Line, MainRequest, MainView, PromptAction, Reply, Snapshot, Tone};

/// Lines moved by PgUp and PgDn.
pub const PAGE_LINES: usize = 12;
/// Lines moved by Ctrl+U and Ctrl+D.
pub const HALF_PAGE_LINES: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pane {
    Status,
    Files,
    Branches,
    Commits,
    Stashes,
    Main,
}

/// A key after the window has normalised the platform event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Key {
    NextRepository,
    PreviousRepository,
    CloseRepository,
    EditMove { movement: Move, extend: bool },
    SelectAll,
    Cut,
    Paste,
    Pasted(String),
    Delete,
    Save,
    RefreshReview,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    Home,
    End,
    Enter,
    CtrlEnter,
    Escape,
    CtrlC,
    CtrlO,
    Tab,
    BackTab,
    Space,
    Backspace,
    Char(char),
    /// Committed text longer than one character, such as an input-method commit.
    Text(String),
}

/// What the window must do after a key or reply.
#[derive(Debug, Eq, PartialEq)]
pub enum Effect {
    Copy { text: String, cut: bool },
    Paste,
    Nothing,
    Send(Job),
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptKind {
    GitSettings,
    OpenRepository,
    Commit,
    NewBranch,
}

/// A single text field taking over the main panel. A submitted prompt stays
/// open, unchanged, until the backend answers, so a refused commit keeps its
/// text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prompt {
    kind: PromptKind,
    text: String,
    submitted: Option<PromptAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateForm {
    pub path: String,
    pub branch: String,
    pub branch_focus: bool,
    pub submitted: bool,
    pub review: Option<(String, Vec<String>)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityForm {
    pub name: String,
    pub email: String,
    pub email_focus: bool,
    pub submitted: bool,
    pub ticket: Option<String>,
    pub detail: String,
    keep_input: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitFocus { Message, Index, Diff }

#[derive(Debug, Eq, PartialEq)]
pub struct CommitComposer {
    pub editor: ComposerState,
    pub review: Option<crate::model::CommitReview>,
    pub selected: usize,
    pub focus: CommitFocus,
    pub diff: MainView,
    pub diff_scroll: usize,
    pub diff_column: usize,
    pub diff_ready: bool,
    pub diff_generation: u64,
    pub review_generation: u64,
    pub id: u64,
    pub save_error: Option<String>,
    saved: String,
    in_flight: Option<String>,
    dirty_since: Option<Instant>,
    closing: bool,
    quit: bool,
}

impl Prompt {
    pub fn pending(&self) -> bool { self.submitted.is_some() }
    #[must_use]
    pub const fn kind(&self) -> PromptKind {
        self.kind
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the effect and whether the prompt stays open.
    fn handle(&mut self, key: Key) -> (Effect, bool) {
        if self.submitted.is_some() {
            return (Effect::Nothing, true);
        }
        match (self.kind, key) {
            (PromptKind::GitSettings, Key::Escape | Key::CtrlC) => (Effect::Nothing, false),
            (PromptKind::GitSettings, Key::Enter) => {
                self.submitted = Some(PromptAction::GitSettings);
                let path = (!self.text.is_empty()).then(|| NativePath::new(self.text.clone()));
                (Effect::Send(Job::ConfigureGit(path)), true)
            }
            (PromptKind::OpenRepository, Key::Escape | Key::CtrlC) => (Effect::Nothing, false),
            (PromptKind::OpenRepository, Key::Enter) => {
                if self.text.is_empty() { return (Effect::Nothing, true); }
                self.submitted = Some(PromptAction::OpenRepository);
                (Effect::Send(Job::OpenRepository(NativePath::new(self.text.clone()))), true)
            }
            (PromptKind::Commit, Key::Escape | Key::CtrlC) => {
                self.submitted = Some(PromptAction::SaveDraft);
                (Effect::Send(Job::SaveDraft(self.text.clone())), true)
            }
            (PromptKind::NewBranch, Key::Escape | Key::CtrlC) => (Effect::Nothing, false),
            (PromptKind::Commit, Key::CtrlEnter) => {
                (Effect::Nothing, true) // requires the workspace's full-index review
            }
            (PromptKind::Commit, Key::Enter) => {
                self.text.push('\n');
                (Effect::Nothing, true)
            }
            (PromptKind::NewBranch, Key::Enter) => {
                let name = self.text.trim().to_owned();
                if name.is_empty() {
                    return (Effect::Nothing, true);
                }
                self.submitted = Some(PromptAction::CreateBranch);
                (Effect::Send(Job::CreateBranch(name)), true)
            }
            (_, Key::Backspace) => {
                self.text.pop();
                (Effect::Nothing, true)
            }
            (_, Key::Space) => {
                self.text.push(' ');
                (Effect::Nothing, true)
            }
            (_, Key::Char(character)) => {
                self.text.push(character);
                (Effect::Nothing, true)
            }
            (_, Key::Text(text)) => {
                self.text.push_str(&text);
                (Effect::Nothing, true)
            }
            _ => (Effect::Nothing, true),
        }
    }
}

/// A yes-or-no question that takes the main panel until it is answered. `y`
/// confirms; `n` or Escape keeps everything as it was.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Confirm {
    Ignore { path: NativePath, local: bool },
    DeleteBranch(String),
    DiscardFile { path: NativePath, untracked: bool },
    DiscardHunk { path: NativePath, index: usize },
    Detach { id: String, short: String },
    Stash { id: String, operation: String },
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum ChangesAction { StageFile, UnstageFile, DiscardFile, Ignore, Unstaged, Staged, Hunk, Lines, Range, DiscardHunk }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryTab { pub path: NativePath, pub head: String }

#[derive(Clone)]
struct FilterAnchor { identity: Option<crate::NavigationSelection>, cursor: usize, scroll: usize }

pub struct FilterEditor {
    pub pane: Pane,
    pub editor: ComposerState,
    pub id: u64,
    previous_query: String,
    previous_anchor: Option<FilterAnchor>,
    origin: FilterAnchor,
    previous_view: Option<MainView>,
    previous_request: Option<MainRequest>,
    previous_back: Vec<crate::NavigationFrame>,
    previous_focus: Pane,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TabAction { Select(NativePath), Close, Quit }

pub struct Workspace {
    locate: Option<crate::LocateForm>,
    tools: Option<crate::ToolForm>,
    navigation_issue: Option<crate::NavigationIssue>,
    filters: [crate::filter::FilterIndex; 3],
    filter_anchors: [Option<FilterAnchor>; 3],
    filter_editor: Option<FilterEditor>,
    next_position: Option<(Pane, Option<crate::NavigationSelection>, usize, usize)>,
    tabs: Vec<RepositoryTab>,
    closing_tab: bool,
    tab_entry: bool,
    tab_action: Option<TabAction>,
    last_request: Option<MainRequest>,
    restore_position: Option<(u64, usize, usize)>,
    snapshot: Snapshot,
    focus: Pane,
    /// The list whose selection the main panel shows. It is set whenever a list
    /// gains focus, and it stays while the main panel has focus.
    source: Pane,
    files: usize,
    branches: usize,
    commits: usize,
    stashes: usize,
    stash_mode: bool,
    main: MainView,
    main_ready: bool,
    /// The main panel's cursor: an index into `main.lines`.
    cursor: usize,
    /// The other end of a `v` line range. `None` when no range is open.
    anchor: Option<usize>,
    /// The first main-panel line drawn.
    scroll: usize,
    /// How many main-panel lines fit; the window sets it before each frame.
    view_rows: usize,
    /// Incremented on every main-panel request, so late replies can be dropped.
    generation: u64,
    notice: Option<String>,
    prompt: Option<Prompt>,
    confirm: Option<Confirm>,
    prepared_ticket: Option<String>,
    confirmation_lines: Vec<String>,
    help: bool,
    history_back: Vec<crate::NavigationFrame>,
    restore_detail_file: Option<NativePath>,
    trust: Option<crate::model::TrustReview>,
    trust_scroll: usize,
    trust_limit: usize,
    create: Option<CreateForm>,
    git_detail: String,
    identity: Option<IdentityForm>,
    composer: Option<CommitComposer>,
}

impl Workspace {
    #[must_use]
    pub fn new(snapshot: Snapshot) -> Self {
        let tabs = snapshot.repository.as_ref().map(|path| RepositoryTab { path: path.clone(), head: snapshot.head.clone() }).into_iter().collect();
        let mut workspace = Self {
            locate: None,
            tools: None,
            navigation_issue: None,
            filters: std::array::from_fn(|_| Default::default()), filter_anchors: [None, None, None], filter_editor: None, next_position: None,
            tabs, closing_tab: false, tab_entry: false, tab_action: None, last_request: None, restore_position: None,
            snapshot,
            focus: Pane::Files,
            source: Pane::Files,
            files: 0,
            branches: 0,
            commits: 0,
            stashes: 0,
            stash_mode: false,
            main: MainView::default(),
            main_ready: false,
            cursor: 0,
            anchor: None,
            scroll: 0,
            view_rows: 0,
            generation: 0,
            notice: None,
            prompt: None,
            confirm: None,
            prepared_ticket: None,
            confirmation_lines: Vec::new(),
            help: false,
            history_back: Vec::new(),
            restore_detail_file: None,
            trust: None,
            trust_scroll: 0,
            trust_limit: 0,
            create: None,
            git_detail: String::new(),
            identity: None,
            composer: None,
        };
        workspace.load_filter_rows();
        workspace
    }

    /// Requests the main-panel content for the first list. Call once after `new`.
    pub fn open(&mut self) -> Effect {
        if self.is_welcome() {
            self.main = MainView::text("Open a repository by path, or select a recent location.\nRecent locations are not trusted automatically.\nNo repository commands run while browsing this screen.\n\nCtrl+O: Open path\nEnter: Open selected recent location\nL: Locate selected recent location\nd: Remove selected recent record only\nr: Reload recent records");
            return Effect::Send(Job::LoadWelcome);
        }
        if let Some(navigation) = self.snapshot.navigation.take() { return self.restore_navigation(navigation); }
        self.show_source()
    }

    pub fn tabs(&self) -> &[RepositoryTab] { &self.tabs }
    pub fn navigation_issue(&self) -> Option<&crate::NavigationIssue> { self.navigation_issue.as_ref() }
    pub fn answer_navigation(&mut self, approve: bool) -> Effect {
        let Some(issue) = self.navigation_issue.as_mut() else { return Effect::Nothing; };
        if issue.submitted { return Effect::Nothing; }
        if approve { issue.submitted = true; return Effect::Send(Job::ResetNavigation(issue.ticket.clone())); }
        self.navigation_issue = None;
        self.closing_tab = false;
        if self.tab_entry { self.prompt = None; self.tab_entry = false; }
        else if let Some(prompt) = self.prompt.as_mut() { prompt.submitted = None; }
        self.notice = Some("Opening cancelled; saved view was retained".into());
        Effect::Send(Job::CancelEntry)
    }

    fn filter_slot(pane: Pane) -> Option<usize> { match pane { Pane::Files => Some(0), Pane::Branches => Some(1), Pane::Commits => Some(2), _ => None } }
    pub fn filter_query(&self, pane: Pane) -> &str { Self::filter_slot(pane).map_or("", |i| self.filters[i].query.as_str()) }
    pub fn visible_rows(&self, pane: Pane) -> &[usize] { Self::filter_slot(pane).map_or(&[], |i| self.filters[i].rows.as_slice()) }
    pub fn filter_editor(&self) -> Option<&FilterEditor> { self.filter_editor.as_ref() }
    pub fn clipboard_context(&self) -> Option<u64> { self.locate.as_ref().filter(|f| f.working.is_none() && f.review.is_none()).map(|f| f.id).or_else(|| self.tools.as_ref().filter(|f| f.working.is_none() && f.review.is_none()).map(|f| f.id)).or_else(|| self.filter_editor.as_ref().map(|e| e.id)).or_else(|| self.composer.as_ref().map(|c| c.id)) }

    pub fn locate_form(&self) -> Option<&crate::LocateForm> { self.locate.as_ref() }
    pub fn request_locate(&mut self) -> Effect {
        let from_open = self.prompt.as_ref().filter(|p| p.kind == PromptKind::OpenRepository && !p.pending()).map(|p| p.text.clone());
        if !self.is_welcome() || (self.modal() && (from_open.is_none() || self.trust.is_some() || self.navigation_issue.is_some())) { return Effect::Nothing; }
        let Some(source) = self.snapshot.recents.get(self.files).cloned() else { return Effect::Nothing; };
        if source.backend != "git" { self.notice = Some("This backend is unavailable; its recent record was preserved".into()); return Effect::Nothing; }
        let mut form = crate::LocateForm::new(source);
        if let Some(text) = from_open { form.editor.text = text; form.editor.caret = form.editor.text.len(); composer::select_all(&mut form.editor); }
        self.prompt = None; self.notice = None; self.trust_scroll = 0; self.locate = Some(form);
        Effect::Nothing
    }
    pub fn submit_locate(&mut self) -> Effect { self.locate.as_mut().map_or(Effect::Nothing, crate::LocateForm::submit) }
    pub fn approve_locate(&mut self) -> Effect { self.locate.as_mut().map_or(Effect::Nothing, crate::LocateForm::approve) }
    pub fn cancel_locate(&mut self) -> Effect {
        let Some(form) = self.locate.as_mut() else { return Effect::Nothing; };
        if form.working == Some(crate::LocateWork::Save) || !form.editor.preedit.is_empty() { return Effect::Nothing; }
        if form.review.take().is_some() { form.detail = "Location review cancelled; the recent record was preserved.".into(); self.trust_scroll = 0; }
        else { self.locate = None; }
        Effect::Send(Job::CancelLocate)
    }
    pub fn click_locate_caret(&mut self, column: usize, columns: usize) -> Effect {
        if let Some(form) = self.locate.as_mut() && form.working.is_none() && form.review.is_none() && form.editor.preedit.is_empty() {
            let start = composer::visible_window(&form.editor.text, "", form.editor.caret, 1, columns).text;
            form.editor.caret = composer::caret_at_cell(&form.editor.text, start, column); form.editor.anchor = None; form.editor.select_all = false;
        }
        Effect::Nothing
    }
    pub fn forget_recent(&mut self) -> Effect {
        if !self.is_welcome() || self.modal() { return Effect::Nothing; }
        self.snapshot.recents.get(self.files).map_or(Effect::Nothing, |entry| Effect::Send(Job::ForgetRecent { backend: entry.backend.clone(), path: entry.path.clone() }))
    }

    pub fn tool_form(&self) -> Option<&crate::ToolForm> { self.tools.as_ref() }
    pub fn request_tools(&mut self) -> Effect {
        if self.modal() { return Effect::Nothing; }
        let file = if !self.stash_mode && (self.focus == Pane::Files || self.source == Pane::Files) { self.snapshot.files.get(self.files).map(|f| f.path.clone()) } else { None };
        self.tools = Some(crate::ToolForm::new(srcmgr_domain::ToolKind::Editor, file));
        self.notice = None;
        Effect::Send(Job::LoadToolSettings(srcmgr_domain::ToolKind::Editor))
    }
    pub fn select_tool_kind(&mut self, kind: srcmgr_domain::ToolKind) -> Effect {
        let Some(form) = self.tools.as_mut() else { return Effect::Nothing; };
        if form.kind == kind || form.working.is_some() || form.review.is_some() || form.composed() { return Effect::Nothing; }
        if form.definition().ok() != Some(form.original_definition().cloned()) { form.detail = "Save or clear changed settings before switching tools; Close discards unsaved edits.".into(); return Effect::Nothing; }
        let target = form.target.clone();
        self.tools = Some(crate::ToolForm::new(kind, target));
        Effect::Send(Job::LoadToolSettings(kind))
    }
    pub fn focus_tool_field(&mut self, arguments: bool) -> Effect {
        if let Some(form) = self.tools.as_mut() && form.working.is_none() && form.review.is_none() && !form.composed() { form.focus(arguments); }
        Effect::Nothing
    }
    pub fn click_tool_caret(&mut self, arguments: bool, row: usize, column: usize, rows: usize, columns: usize) -> Effect {
        self.focus_tool_field(arguments);
        if let Some(form) = self.tools.as_mut() && form.working.is_none() && form.review.is_none() && !form.composed() {
            let editor = form.editor_mut();
            let visible = composer::visible_line_window(&editor.text, editor.caret, rows);
            let range = composer::line_range(&editor.text, visible.first_line + row);
            let line = &editor.text[range.clone()];
            let start = if row == visible.caret_row { composer::visible_window(line, &editor.preedit, editor.caret.saturating_sub(range.start).min(line.len()), 1, columns).text } else { 0 };
            editor.caret = range.start + composer::caret_at_cell(line, start, column);
            editor.anchor = None; editor.select_all = false;
        }
        Effect::Nothing
    }
    pub fn save_tool(&mut self, clear: bool) -> Effect { self.tools.as_mut().map_or(Effect::Nothing, |f| f.save(clear)) }
    pub fn prepare_tool(&mut self) -> Effect { self.tools.as_mut().map_or(Effect::Nothing, crate::ToolForm::prepare) }
    pub fn approve_tool(&mut self) -> Effect { self.tools.as_mut().map_or(Effect::Nothing, crate::ToolForm::approve) }
    pub fn close_tool(&mut self) -> Effect {
        let Some(form) = self.tools.as_mut() else { return Effect::Nothing; };
        if matches!(form.working, Some(crate::ToolWork::Save | crate::ToolWork::Launch)) || form.composed() { return Effect::Nothing; }
        if form.review.take().is_some() { form.detail = "Launch cancelled; no program was started.".into(); } else { self.tools = None; }
        Effect::Send(Job::CancelTool)
    }

    fn load_filter_rows(&mut self) {
        self.filters[0].load(self.snapshot.files.iter().map(|f| f.path.display_label()), std::iter::empty());
        self.filters[1].load(self.snapshot.branches.iter().map(|b| NativePath::new(b.name.clone()).display_label()), std::iter::empty());
        self.filters[2].load(self.snapshot.commits.iter().map(|c| c.subject.clone()), self.snapshot.commits.iter().map(|c| c.id.clone()));
    }

    fn reconcile_filter_selection(&mut self) {
        for (i, current) in [(0, &mut self.files), (1, &mut self.branches), (2, &mut self.commits)] {
            if !self.filters[i].rows.contains(current) { *current = self.filters[i].rows.first().copied().unwrap_or(usize::MAX); }
        }
    }

    fn selected_identity(&self, pane: Pane) -> Option<crate::NavigationSelection> {
        match pane { Pane::Files => self.snapshot.files.get(self.files).map(|f| crate::NavigationSelection::File(f.path.clone())), Pane::Branches => self.snapshot.branches.get(self.branches).map(|b| crate::NavigationSelection::Branch(b.name.clone())), Pane::Commits => self.snapshot.commits.get(self.commits).map(|c| crate::NavigationSelection::Commit(c.id.clone())), _ => None }
    }

    fn restore_filter_identity(&mut self, pane: Pane, identity: &Option<crate::NavigationSelection>) {
        match (pane, identity) {
            (Pane::Files, Some(crate::NavigationSelection::File(path))) => self.files = self.snapshot.files.iter().position(|f| &f.path == path).unwrap_or(usize::MAX),
            (Pane::Branches, Some(crate::NavigationSelection::Branch(name))) => self.branches = self.snapshot.branches.iter().position(|b| &b.name == name).unwrap_or(usize::MAX),
            (Pane::Commits, Some(crate::NavigationSelection::Commit(id))) => self.commits = self.snapshot.commits.iter().position(|c| &c.id == id).unwrap_or(usize::MAX),
            _ => {}
        }
        self.reconcile_filter_selection();
    }

    pub fn request_filter(&mut self, pane: Pane) -> Effect {
        if self.modal() || self.is_welcome() { return Effect::Nothing; }
        let Some(slot) = Self::filter_slot(pane) else { return Effect::Nothing; };
        let same_source = self.source == pane;
        let previous_focus = self.focus;
        self.focus = pane;
        self.source = pane;
        self.generation += 1;
        let origin = FilterAnchor { identity: self.selected_identity(pane), cursor: if same_source { self.cursor } else { 0 }, scroll: if same_source { self.scroll } else { 0 } };
        let mut editor = ComposerState::default();
        editor.text.clone_from(&self.filters[slot].query);
        editor.caret = editor.text.len();
        let previous_view = std::mem::replace(&mut self.main, MainView::note("Filter loaded items; Enter applies, Esc restores the previous query"));
        let previous_request = self.last_request.take();
        let previous_back = std::mem::take(&mut self.history_back);
        self.filter_editor = Some(FilterEditor { pane, editor, id: self.generation, previous_query: self.filters[slot].query.clone(), previous_anchor: self.filter_anchors[slot].clone(), origin, previous_view: (self.main_ready && same_source).then_some(previous_view), previous_request: if self.main_ready && same_source { previous_request } else { None }, previous_back, previous_focus });
        self.main_ready = false;
        self.notice = None;
        Effect::Nothing
    }

    fn update_filter_text(&mut self) {
        let form = self.filter_editor.as_ref().unwrap();
        let slot = Self::filter_slot(form.pane).unwrap();
        if self.filters[slot].query == form.editor.text { return; }
        if self.filters[slot].query.is_empty() && !form.editor.text.is_empty() && self.filter_anchors[slot].is_none() { self.filter_anchors[slot] = Some(form.origin.clone()); }
        self.filters[slot].query.clone_from(&form.editor.text);
        self.filters[slot].project();
        self.reconcile_filter_selection();
        self.generation += 1;
        self.anchor = None;
        self.restore_position = None;
        self.last_request = None;
    }

    pub fn finish_filter(&mut self, apply: bool, inspect: bool) -> Effect {
        let Some(form) = self.filter_editor.take() else { return Effect::Nothing; };
        if !form.editor.preedit.is_empty() { self.notice = Some("Finish IME composition before leaving the filter".into()); self.filter_editor = Some(form); return Effect::Nothing; }
        let slot = Self::filter_slot(form.pane).unwrap();
        self.focus = form.pane;
        self.source = form.pane;
        if !apply {
            self.filters[slot].query = form.previous_query;
            self.filters[slot].project();
            self.filter_anchors[slot] = form.previous_anchor;
            self.restore_filter_identity(form.pane, &form.origin.identity);
            if self.selected_identity(form.pane) == form.origin.identity && let Some(view) = form.previous_view {
                self.main = view;
                self.main_ready = true;
                self.last_request = form.previous_request;
                self.history_back = form.previous_back;
                self.focus = form.previous_focus;
                self.cursor = form.origin.cursor;
                self.scroll = form.origin.scroll;
            }
            return Effect::Nothing;
        }
        if self.filters[slot].query.is_empty() && let Some(anchor) = self.filter_anchors[slot].take() {
            self.restore_filter_identity(form.pane, &anchor.identity);
            self.next_position = Some((form.pane, self.selected_identity(form.pane), anchor.cursor, anchor.scroll));
        }
        if inspect { return self.show_source(); }
        Effect::Nothing
    }

    pub fn clear_filter(&mut self, pane: Pane) -> Effect {
        let Some(slot) = Self::filter_slot(pane) else { return Effect::Nothing; };
        if self.filter_editor.as_ref().is_some_and(|e| !e.editor.preedit.is_empty()) { self.notice = Some("Finish IME composition before clearing the filter".into()); return Effect::Nothing; }
        self.filter_editor = None;
        self.filters[slot].query.clear();
        self.filters[slot].project();
        if let Some(anchor) = self.filter_anchors[slot].take() {
            self.restore_filter_identity(pane, &anchor.identity);
            self.next_position = Some((pane, self.selected_identity(pane), anchor.cursor, anchor.scroll));
        } else { self.reconcile_filter_selection(); }
        self.focus = pane;
        self.source = pane;
        self.generation += 1;
        self.last_request = None;
        self.anchor = None;
        self.history_back.clear();
        self.restore_detail_file = None;
        self.main = MainView::note("Filter cleared. Enter or select a row to inspect current content.");
        Effect::Nothing
    }

    fn filter_key(&mut self, key: Key) -> Effect {
        let form = self.filter_editor.as_mut().unwrap();
        if !form.editor.preedit.is_empty() { if key == Key::Escape { form.editor.preedit.clear(); } return Effect::Nothing; }
        match key {
            Key::Escape => return self.finish_filter(false, false),
            Key::Enter => return self.finish_filter(true, true),
            Key::Tab | Key::BackTab => { self.finish_filter(true, false); return self.cycle(matches!(key, Key::Tab)); }
            Key::Up | Key::Down => {
                let pane = form.pane;
                let length = self.list_length(pane);
                if length > 0 { let current = self.selected(pane); let next = if key == Key::Up { current.saturating_sub(1) } else { current.saturating_add(1).min(length - 1) }; self.set_selected(pane, next); }
                return Effect::Nothing;
            }
            Key::SelectAll => composer::select_all(&mut form.editor),
            Key::CtrlC | Key::Cut => return composer::selection_text(&form.editor).map_or(Effect::Nothing, |s| Effect::Copy { text: s.into(), cut: key == Key::Cut }),
            Key::Paste => return Effect::Paste,
            Key::Backspace => composer::backspace(&mut form.editor), Key::Delete => composer::delete_forward(&mut form.editor),
            Key::Left => composer::move_caret(&mut form.editor, Move::Left), Key::Right => composer::move_caret(&mut form.editor, Move::Right), Key::Home => composer::move_caret(&mut form.editor, Move::LineStart), Key::End => composer::move_caret(&mut form.editor, Move::LineEnd),
            Key::EditMove { movement, extend } => { if extend { composer::extend_selection(&mut form.editor, movement); } else { composer::move_caret(&mut form.editor, movement); } }
            key => {
                let text = match key { Key::Char(c) => c.to_string(), Key::Text(s) | Key::Pasted(s) => s, Key::Space => " ".into(), _ => return Effect::Nothing };
                let removed = composer::selection_bounds(&form.editor).map_or(0, |(a,b)| form.editor.text[a..b].chars().count());
                if text.contains(['\n', '\r']) || form.editor.text.chars().count() - removed + text.chars().count() > 256 { self.notice = Some("Filter limit is 256 Unicode characters on one line; input was not inserted".into()); return Effect::Nothing; }
                composer::insert(&mut form.editor, &text);
            }
        }
        self.update_filter_text();
        Effect::Nothing
    }

    pub fn preview_filter_row(&mut self, pane: Pane, index: usize) -> Effect {
        if self.filter_editor.as_ref().is_some_and(|e| e.pane == pane) { self.set_selected(pane, index); }
        Effect::Nothing
    }

    pub fn click_filter_caret(&mut self, column: usize, columns: usize) -> Effect {
        if let Some(form) = self.filter_editor.as_mut() {
            let start = composer::visible_window(&form.editor.text, &form.editor.preedit, form.editor.caret, 1, columns).text;
            form.editor.caret = composer::caret_at_cell(&form.editor.text, start, column);
            form.editor.anchor = None;
            form.editor.select_all = false;
        }
        Effect::Nothing
    }

    pub fn navigation(&self) -> crate::Navigation {
        crate::Navigation { back: self.history_back.clone(), detail_file: self.selected_detail_file().or_else(|| self.restore_detail_file.clone()), queries: std::array::from_fn(|i| self.filters[i].query.clone()), anchors: std::array::from_fn(|i| self.filter_anchors[i].as_ref().and_then(|a| a.identity.clone())), anchor_positions: std::array::from_fn(|i| self.filter_anchors[i].as_ref().map_or((0, 0), |a| (a.cursor, a.scroll))), focus: self.focus, source: self.source,
            file: self.snapshot.files.get(self.files).map(|f| f.path.clone()), branch: self.snapshot.branches.get(self.branches).map(|b| b.name.clone()),
            commit: self.snapshot.commits.get(self.commits).map(|c| c.id.clone()), stash: self.snapshot.stashes.get(self.stashes).map(|s| s.id.clone()),
            stash_mode: self.stash_mode, cursor: self.cursor, scroll: self.scroll, request: self.last_request.clone() }
    }

    fn restore_navigation(&mut self, navigation: crate::Navigation) -> Effect {
        for i in 0..3 {
            self.filters[i].query.clone_from(&navigation.queries[i]);
            self.filters[i].project();
            let (cursor, scroll) = navigation.anchor_positions[i];
            self.filter_anchors[i] = navigation.anchors[i].clone().map(|identity| FilterAnchor { identity: Some(identity), cursor, scroll });
        }
        self.focus = navigation.focus;
        self.source = navigation.source;
        self.stash_mode = navigation.stash_mode;
        self.files = navigation.file.as_ref().and_then(|p| self.snapshot.files.iter().position(|f| &f.path == p)).unwrap_or(0);
        self.branches = navigation.branch.as_ref().and_then(|n| self.snapshot.branches.iter().position(|b| &b.name == n)).unwrap_or(0);
        self.commits = navigation.commit.as_ref().and_then(|id| self.snapshot.commits.iter().position(|c| &c.id == id)).unwrap_or(0);
        self.stashes = navigation.stash.as_ref().and_then(|id| self.snapshot.stashes.iter().position(|s| &s.id == id)).unwrap_or(0);
        self.reconcile_filter_selection();
        let had_request = navigation.request.is_some();
        let request = navigation.request.filter(|request| match request {
            MainRequest::File { path, staged_view, .. } => self.snapshot.files.get(self.files).is_some_and(|f| &f.path == path && if *staged_view { f.has_staged } else { f.has_unstaged }),
            MainRequest::Stash { id } => self.snapshot.stashes.iter().any(|s| &s.id == id),
            MainRequest::Commit { id } | MainRequest::CommitFiles { id } | MainRequest::CommitFile { id, .. } => self.snapshot.commits.get(self.commits).is_some_and(|c| &c.id == id),
        });
        let Some(request) = request else {
            let effect = self.show_source();
            if had_request { self.notice = Some("Saved detail is outside the current loaded/filter state or changed side; current selection was refreshed".into()); }
            return effect;
        };
        self.history_back = navigation.back;
        self.restore_detail_file = navigation.detail_file;
        self.generation += 1;
        self.restore_position = Some((self.generation, navigation.cursor, navigation.scroll));
        self.last_request = Some(request.clone());
        self.main = MainView::note("Refreshing restored selection...");
        Effect::Send(Job::Main { generation: self.generation, request })
    }

    fn entry(&mut self, path: Option<NativePath>) -> Effect {
        if path.as_ref().is_some_and(|p| self.tabs.len() >= 16 && !self.tabs.iter().any(|t| &t.path == p)) {
            self.notice = Some("16 repositories are open; close a tab before opening another".into());
            if let Some(prompt) = self.prompt.as_mut() { prompt.submitted = None; }
            self.closing_tab = false;
            return Effect::Nothing;
        }
        let current = self.snapshot.repository.clone().map(|p| (p, Box::new(self.navigation())));
        Effect::Send(Job::EnterRepository { path, current })
    }

    pub fn select_repository(&mut self, index: usize) -> Effect {
        let Some(tab) = self.tabs.get(index) else { return Effect::Nothing; };
        if self.snapshot.repository.as_ref() == Some(&tab.path) { return Effect::Nothing; }
        self.repository_action(TabAction::Select(tab.path.clone()))
    }

    pub fn close_repository(&mut self) -> Effect { self.repository_action(TabAction::Close) }

    fn repository_action(&mut self, action: TabAction) -> Effect {
        if self.filter_editor.is_some() {
            self.finish_filter(true, false);
            if self.filter_editor.is_some() { return Effect::Nothing; }
        }
        if self.composer.is_some() {
            if self.composer.as_ref().is_some_and(|form| !form.editor.preedit.is_empty()) { self.notice = Some("Finish IME composition before changing repositories".into()); return Effect::Nothing; }
            if self.prompt.as_ref().is_some_and(Prompt::pending) { self.notice = Some("Wait for the current save or commit before switching repositories".into()); return Effect::Nothing; }
            self.tab_action = Some(action);
            let effect = self.close_commit(false);
            if self.composer.is_none() { return self.finish_repository_action(); }
            return effect;
        }
        if self.modal() { self.notice = Some("Finish or cancel this dialog before changing repositories".into()); return Effect::Nothing; }
        self.tab_action = Some(action);
        self.finish_repository_action()
    }

    fn finish_repository_action(&mut self) -> Effect {
        let Some(action) = self.tab_action.take() else { return Effect::Nothing; };
        if self.is_welcome() && action == TabAction::Quit { return Effect::Quit; }
        let path = match action {
            TabAction::Select(path) => Some(path),
            TabAction::Close => {
                if self.is_welcome() { return Effect::Nothing; }
                self.closing_tab = true;
                let active = self.tabs.iter().position(|t| Some(&t.path) == self.snapshot.repository.as_ref()).unwrap_or(0);
                self.tabs.get(if active > 0 { active - 1 } else { 1 }).map(|t| t.path.clone())
            }
            TabAction::Quit => { self.tab_action = Some(TabAction::Quit); None }
        };
        self.prompt = Some(Prompt { kind: PromptKind::OpenRepository, text: path.as_ref().map(|p| p.display_label()).unwrap_or_default(), submitted: Some(PromptAction::OpenRepository) });
        self.tab_entry = true;
        self.entry(path)
    }

    pub fn request_quit(&mut self) -> Effect { self.repository_action(TabAction::Quit) }

    fn selected_detail_file(&self) -> Option<NativePath> {
        self.main.committed.as_ref().and_then(|list| self.cursor.checked_sub(2).and_then(|i| list.files.get(i))).map(|file| file.path.clone())
    }

    pub fn can_back(&self) -> bool { !self.history_back.is_empty() }
    pub fn history_context(&self) -> Option<(&'static str, &str, bool)> {
        match self.last_request.as_ref()? {
            MainRequest::Commit { id } => Some(("Commit", id, self.main_ready)),
            MainRequest::CommitFiles { id } => Some(("Files", id, self.main_ready)),
            MainRequest::CommitFile { id, .. } => Some(("File Diff", id, self.main_ready)),
            _ => None,
        }
    }

    fn return_history(&mut self) -> Effect {
        let Some(frame) = self.history_back.pop() else { return Effect::Nothing; };
        self.focus = if matches!(frame.request, MainRequest::Commit { .. }) { Pane::Commits } else { Pane::Main };
        self.source = Pane::Commits;
        self.generation += 1;
        self.restore_position = Some((self.generation, frame.cursor, frame.scroll));
        self.restore_detail_file = frame.file;
        self.main = MainView::note("Reading previous history view...");
        self.main_ready = false;
        self.last_request = Some(frame.request.clone());
        self.anchor = None;
        Effect::Send(Job::Main { generation: self.generation, request: frame.request })
    }

    pub fn is_welcome(&self) -> bool { self.snapshot.repository.is_none() }
    pub fn create_form(&self) -> Option<&CreateForm> { self.create.as_ref() }
    pub fn git_detail(&self) -> &str { &self.git_detail }
    pub fn commit_composer(&self) -> Option<&CommitComposer> { self.composer.as_ref() }
    pub fn draft_status(&self) -> &str {
        let Some(form) = &self.composer else { return ""; };
        if form.save_error.is_some() { "Draft save failed: Ctrl+S retries; Ctrl+A/C copies" }
        else if form.in_flight.is_some() { "Saving draft..." }
        else if form.editor.text == form.saved { "Draft saved" }
        else { "Draft unsaved" }
    }

    pub fn ime_preedit(&mut self, text: String) {
        if let Some(form) = self.locate.as_mut() { if form.working.is_none() && form.review.is_none() { form.editor.preedit = text.chars().take(4096).collect(); } return; }
        if let Some(form) = self.tools.as_mut() { if form.working.is_none() && form.review.is_none() { form.editor_mut().preedit = text.chars().take(4096).collect(); } return; }
        if let Some(form) = self.filter_editor.as_mut() { form.editor.preedit = text.chars().take(4096).collect(); return; }
        if let Some(form) = self.composer.as_mut() && (form.focus == CommitFocus::Message || text.is_empty()) {
            form.editor.preedit = text.chars().take(4096).collect();
        }
    }

    pub fn ime_commit(&mut self, text: String) -> Effect {
        if let Some(form) = self.locate.as_mut() { form.editor.preedit.clear(); return form.edit(Key::Text(text)); }
        if let Some(form) = self.tools.as_mut() { form.editor_mut().preedit.clear(); return form.edit_key(Key::Text(text)); }
        if let Some(form) = self.filter_editor.as_mut() { form.editor.preedit.clear(); return self.filter_key(Key::Text(text)); }
        if let Some(form) = self.composer.as_mut() {
            form.editor.preedit.clear();
            let focus = form.focus;
            form.focus = CommitFocus::Message;
            let effect = self.commit_key(Key::Text(text));
            if let Some(form) = self.composer.as_mut() { form.focus = focus; }
            return effect;
        }
        self.key(Key::Text(text))
    }

    pub fn click_commit_caret(&mut self, line: usize, column: usize, rows: usize, columns: usize) -> Effect {
        let Some(form) = self.composer.as_mut() else { return Effect::Nothing; };
        if self.prompt.as_ref().is_some_and(Prompt::pending) { return Effect::Nothing; }
        let window = composer::visible_line_window(&form.editor.text, form.editor.caret, rows);
        let range = composer::line_range(&form.editor.text, window.first_line + line);
        let text = &form.editor.text[range.clone()];
        let horizontal = if line == window.caret_row { composer::visible_window(text, &form.editor.preedit, form.editor.caret.saturating_sub(range.start).min(text.len()), 1, columns).text } else { 0 };
        form.editor.caret = range.start + composer::caret_at_cell(text, horizontal, column);
        form.editor.anchor = None;
        form.editor.select_all = false;
        form.focus = CommitFocus::Message;
        Effect::Nothing
    }

    pub fn request_commit(&mut self) -> Effect {
        if self.modal() || self.is_welcome() { return Effect::Nothing; }
        let text = self.snapshot.draft.clone().unwrap_or_default();
        self.generation += 1;
        let mut editor = ComposerState::default();
        editor.text = text.clone();
        editor.caret = text.len();
        editor.focused = true;
        self.composer = Some(CommitComposer { editor, review: None, selected: 0, focus: CommitFocus::Message, diff: MainView::note("Select a staged path to review its Diff"), diff_scroll: 0, diff_column: 0, diff_ready: false, diff_generation: self.generation, review_generation: self.generation, id: self.generation, saved: text.clone(), in_flight: None, dirty_since: None, save_error: None, closing: false, quit: false });
        self.prompt = Some(Prompt { kind: PromptKind::Commit, text, submitted: None });
        self.notice = Some("Loading complete staged-index review...".into());
        Effect::Send(Job::LoadCommitReview { generation: self.generation })
    }

    fn sync_commit_text(&mut self) {
        let form = self.composer.as_mut().unwrap();
        self.prompt.as_mut().unwrap().text.clone_from(&form.editor.text);
        if form.editor.text != form.saved && form.dirty_since.is_none() { form.dirty_since = Some(Instant::now()); }
        if form.editor.text == form.saved { form.dirty_since = None; }
    }

    pub fn draft_deadline(&self) -> Option<Instant> {
        let form = self.composer.as_ref()?;
        if form.in_flight.is_some() || form.save_error.is_some() || self.prompt.as_ref().is_some_and(Prompt::pending) { return None; }
        form.dirty_since.map(|since| since + Duration::from_millis(500))
    }

    pub fn checkpoint_draft(&mut self) -> Effect {
        let Some(form) = self.composer.as_mut() else { return Effect::Nothing; };
        if form.in_flight.is_some() { return Effect::Nothing; }
        form.save_error = None;
        form.in_flight = Some(form.editor.text.clone());
        Effect::Send(Job::SaveDraft(form.editor.text.clone()))
    }

    pub fn close_commit(&mut self, quit: bool) -> Effect {
        let Some(form) = self.composer.as_mut() else { return if quit { Effect::Quit } else { Effect::Nothing }; };
        if self.prompt.as_ref().is_some_and(|prompt| prompt.submitted == Some(PromptAction::Commit)) {
            self.notice = Some("Commit is running; wait for its observed result before closing".into());
            return Effect::Nothing;
        }
        form.closing = true;
        form.quit |= quit;
        if form.in_flight.is_none() && form.editor.text == form.saved && form.save_error.is_none() {
            let quit = form.quit;
            self.composer = None;
            self.prompt = None;
            return if quit { Effect::Quit } else { Effect::Nothing };
        }
        self.prompt.as_mut().unwrap().submitted = Some(PromptAction::SaveDraft);
        self.checkpoint_draft()
    }

    pub fn refresh_commit_review(&mut self) -> Effect {
        if self.prompt.as_ref().is_some_and(Prompt::pending) { return Effect::Nothing; }
        let Some(form) = self.composer.as_mut() else { return Effect::Nothing; };
        self.generation += 1;
        form.review = None;
        form.diff_ready = false;
        form.review_generation = self.generation;
        form.diff_generation = self.generation;
        form.diff = MainView::note("Loading a fresh complete staged review; message retained");
        self.notice = Some("Loading complete staged-index review...".into());
        Effect::Send(Job::LoadCommitReview { generation: self.generation })
    }

    pub fn select_staged(&mut self, index: usize) -> Effect {
        if self.prompt.as_ref().is_some_and(Prompt::pending) { return Effect::Nothing; }
        let Some(form) = self.composer.as_mut() else { return Effect::Nothing; };
        let Some(review) = &form.review else { return Effect::Nothing; };
        if index >= review.files.len() || review.blocked.is_some() { return Effect::Nothing; }
        self.generation += 1;
        form.selected = index;
        form.diff_generation = self.generation;
        form.diff_scroll = 0;
        form.diff_column = 0;
        form.diff_ready = false;
        form.diff = MainView::note("Loading selected staged Diff...");
        Effect::Send(Job::CommitDiff { ticket: review.ticket.clone(), generation: form.diff_generation, index })
    }

    pub fn focus_commit(&mut self, focus: CommitFocus) -> Effect {
        if let Some(form) = self.composer.as_mut() { form.focus = focus; }
        Effect::Nothing
    }

    pub fn scroll_commit_diff(&mut self, delta: isize) -> Effect {
        if let Some(form) = self.composer.as_mut() { form.diff_scroll = form.diff_scroll.saturating_add_signed(delta).min(form.diff.lines.len().saturating_sub(1)); }
        Effect::Nothing
    }

    pub fn complete_clipboard(&mut self, id: u64, text: Option<String>, cut: Option<String>) -> Effect {
        if let Some(form) = self.locate.as_mut() {
            if form.id != id || form.working.is_some() || form.review.is_some() { return Effect::Nothing; }
            if let Some(text) = text { return form.edit(Key::Pasted(text)); }
            if let Some(cut) = cut && composer::selection_text(&form.editor) == Some(cut.as_str()) { composer::cut(&mut form.editor); }
            return Effect::Nothing;
        }
        if let Some(form) = self.tools.as_mut() {
            if form.id != id || form.working.is_some() || form.review.is_some() { return Effect::Nothing; }
            if let Some(text) = text { return form.edit_key(Key::Pasted(text)); }
            if let Some(cut) = cut && composer::selection_text(form.editor()) == Some(cut.as_str()) { composer::cut(form.editor_mut()); }
            return Effect::Nothing;
        }
        if let Some(form) = self.filter_editor.as_mut() {
            if form.id != id { return Effect::Nothing; }
            if let Some(text) = text { return self.filter_key(Key::Pasted(text)); }
            if let Some(cut) = cut && composer::selection_text(&form.editor) == Some(cut.as_str()) { composer::cut(&mut form.editor); self.update_filter_text(); }
            return Effect::Nothing;
        }
        let Some(form) = self.composer.as_mut() else { return Effect::Nothing; };
        if form.id != id || self.prompt.as_ref().is_some_and(Prompt::pending) { return Effect::Nothing; }
        if let Some(text) = text { return self.commit_key(Key::Pasted(text)); }
        if let Some(cut) = cut && composer::selection_text(&form.editor) == Some(cut.as_str()) {
            composer::cut(&mut form.editor);
            self.sync_commit_text();
        }
        Effect::Nothing
    }

    fn commit_key(&mut self, key: Key) -> Effect {
        if let Some(form) = self.composer.as_mut() && !form.editor.preedit.is_empty() {
            if matches!(key, Key::Escape) { form.editor.preedit.clear(); }
            else { self.notice = Some("Finish IME composition before using editor commands".into()); }
            return Effect::Nothing;
        }
        if matches!(key, Key::Escape) { return self.close_commit(false); }
        if let Some(form) = self.composer.as_mut() && form.save_error.is_some() {
            if matches!(key, Key::Save) { return self.checkpoint_draft(); }
            if matches!(key, Key::SelectAll) { composer::select_all(&mut form.editor); return Effect::Nothing; }
            if matches!(key, Key::CtrlC) { return composer::selection_text(&form.editor).map_or(Effect::Nothing, |text| Effect::Copy { text: text.into(), cut: false }); }
        }
        if self.prompt.as_ref().is_some_and(Prompt::pending) { return Effect::Nothing; }
        if matches!(key, Key::RefreshReview) { return self.refresh_commit_review(); }
        if matches!(key, Key::Save) { return self.checkpoint_draft(); }
        let form = self.composer.as_mut().unwrap();
        if matches!(key, Key::CtrlEnter) {
            let reason = if form.editor.text.trim().is_empty() { Some("Commit message is empty") } else if form.editor.text.len() > composer::PASTE_LIMIT_BYTES { Some("Message exceeds the 64 KiB editor limit; shorten it before submitting") } else if !form.editor.preedit.is_empty() { Some("Finish IME composition before committing") } else if let Some(review) = &form.review { review.blocked.as_deref().or(if form.diff_ready { None } else { Some("Wait for the selected staged Diff") }) } else { Some("Complete staged review is unavailable; Refresh it") };
            if let Some(reason) = reason { self.notice = Some(reason.into()); return Effect::Nothing; }
            let ticket = form.review.as_ref().unwrap().ticket.clone();
            self.prompt.as_mut().unwrap().submitted = Some(PromptAction::Commit);
            return Effect::Send(Job::Commit { ticket, message: form.editor.text.clone() });
        }
        if matches!(key, Key::Tab | Key::BackTab) {
            form.focus = match (form.focus, matches!(key, Key::BackTab)) {
                (CommitFocus::Message, false) | (CommitFocus::Diff, true) => CommitFocus::Index,
                (CommitFocus::Index, false) | (CommitFocus::Message, true) => CommitFocus::Diff,
                _ => CommitFocus::Message,
            };
            return Effect::Nothing;
        }
        if form.focus == CommitFocus::Index {
            let length = form.review.as_ref().map_or(0, |review| review.files.len());
            let selected = form.selected;
            return match key {
                Key::Up => self.select_staged(selected.saturating_sub(1)),
                Key::Down => self.select_staged((selected + 1).min(length.saturating_sub(1))),
                Key::Home => self.select_staged(0),
                Key::End => self.select_staged(length.saturating_sub(1)),
                Key::PageDown => self.select_staged((selected + PAGE_LINES).min(length.saturating_sub(1))),
                Key::PageUp => self.select_staged(selected.saturating_sub(PAGE_LINES)),
                Key::Enter => { form.focus = CommitFocus::Diff; Effect::Nothing }
                _ => Effect::Nothing,
            };
        }
        if form.focus == CommitFocus::Diff {
            return match key {
                Key::Left => { form.diff_column = form.diff_column.saturating_sub(8); Effect::Nothing }
                Key::Right => { let limit = form.diff.lines.iter().map(|line| composer::cells(&line.text)).max().unwrap_or(0); form.diff_column = (form.diff_column + 8).min(limit.saturating_sub(1)); Effect::Nothing }
                Key::Up => self.scroll_commit_diff(-1), Key::Down => self.scroll_commit_diff(1), Key::PageUp => self.scroll_commit_diff(-(PAGE_LINES as isize)), Key::PageDown => self.scroll_commit_diff(PAGE_LINES as isize), _ => Effect::Nothing,
            };
        }
        match key {
            Key::SelectAll => composer::select_all(&mut form.editor),
            Key::CtrlC | Key::Cut => return composer::selection_text(&form.editor).map_or(Effect::Nothing, |text| Effect::Copy { text: text.into(), cut: matches!(key, Key::Cut) }),
            Key::Paste => return Effect::Paste,
            Key::Backspace => composer::backspace(&mut form.editor),
            Key::Delete => composer::delete_forward(&mut form.editor),
            Key::EditMove { movement, extend: true } => composer::extend_selection(&mut form.editor, movement),
            Key::EditMove { movement, extend: false } => composer::move_caret(&mut form.editor, movement),
            Key::Left => composer::move_caret(&mut form.editor, Move::Left), Key::Right => composer::move_caret(&mut form.editor, Move::Right),
            Key::Up => composer::move_caret(&mut form.editor, Move::Up), Key::Down => composer::move_caret(&mut form.editor, Move::Down),
            Key::Home => composer::move_caret(&mut form.editor, Move::LineStart), Key::End => composer::move_caret(&mut form.editor, Move::LineEnd),
            key => {
                let pasted = matches!(&key, Key::Pasted(_));
                let addition = match key { Key::Enter => "\n".into(), Key::Space => " ".into(), Key::Char(ch) => ch.to_string(), Key::Text(text) | Key::Pasted(text) => text, _ => return Effect::Nothing };
                let selected = composer::selection_bounds(&form.editor).map_or(0, |(start, end)| end - start);
                if form.editor.text.len().saturating_sub(selected).saturating_add(addition.len()) > composer::PASTE_LIMIT_BYTES {
                    self.notice = Some("Message limit is 64 KiB UTF-8; input was not inserted".into());
                    return Effect::Nothing;
                }
                if pasted { composer::paste(&mut form.editor, &addition); }
                else { composer::insert(&mut form.editor, &addition.replace("\r\n", "\n")); }
            }
        }
        self.sync_commit_text();
        Effect::Nothing
    }
    pub fn identity_form(&self) -> Option<&IdentityForm> { self.identity.as_ref() }
    pub fn request_identity(&mut self) -> Effect {
        if self.modal() || self.is_welcome() { return Effect::Nothing; }
        self.notice = None;
        self.trust_scroll = 0;
        self.identity = Some(IdentityForm { name: String::new(), email: String::new(), email_focus: false, submitted: true, ticket: None, detail: "Loading trusted repository identity...".into(), keep_input: false });
        Effect::Send(Job::LoadIdentity)
    }

    pub fn focus_identity_field(&mut self, email: bool) -> Effect {
        if let Some(form) = self.identity.as_mut() && !form.submitted { form.email_focus = email; }
        Effect::Nothing
    }

    fn identity_key(&mut self, key: Key) -> Effect {
        let form = self.identity.as_mut().unwrap();
        if form.submitted {
            if form.ticket.is_none() && matches!(key, Key::Escape | Key::CtrlC) {
                self.identity = None;
                return Effect::Send(Job::CancelEntry);
            }
            return Effect::Nothing;
        }
        match key {
            Key::Escape | Key::CtrlC => { self.identity = None; Effect::Send(Job::CancelEntry) }
            Key::Tab | Key::BackTab => { form.email_focus = !form.email_focus; Effect::Nothing }
            Key::Up => self.scroll_trust(-1),
            Key::Down => self.scroll_trust(1),
            Key::PageUp => self.scroll_trust(-(PAGE_LINES as isize)),
            Key::PageDown => self.scroll_trust(PAGE_LINES as isize),
            Key::Enter => {
                form.submitted = true;
                match form.ticket.as_ref() {
                    Some(ticket) => Effect::Send(Job::SaveIdentity { ticket: ticket.clone(), name: form.name.clone(), email: form.email.clone() }),
                    None => Effect::Send(Job::LoadIdentity),
                }
            }
            key => {
                let field = if form.email_focus { &mut form.email } else { &mut form.name };
                let incoming = match key {
                    Key::Backspace => { field.pop(); return Effect::Nothing; }
                    Key::Space => " ".into(), Key::Char(ch) => ch.to_string(), Key::Text(text) => text,
                    _ => return Effect::Nothing,
                };
                if field.len().saturating_add(incoming.len()) > 1024 { self.notice = Some("Identity field limit is 1024 UTF-8 bytes; input was not appended".into()); }
                else { field.push_str(&incoming); self.notice = None; }
                Effect::Nothing
            }
        }
    }
    pub fn request_git_settings(&mut self) -> Effect {
        if self.modal() { return Effect::Nothing; }
        self.notice = None;
        self.git_detail = "Loading application settings; no executable is being run.".into();
        self.prompt = Some(Prompt { kind: PromptKind::GitSettings, text: String::new(), submitted: Some(PromptAction::GitSettings) });
        Effect::Send(Job::LoadGitSettings)
    }
    pub fn focus_create_field(&mut self, branch: bool) -> Effect {
        if let Some(form) = self.create.as_mut() && !form.submitted && form.review.is_none() { form.branch_focus = branch; }
        Effect::Nothing
    }
    pub fn request_create(&mut self) -> Effect {
        if !self.modal() {
            self.notice = None;
            self.create = Some(CreateForm { path: String::new(), branch: "main".into(), branch_focus: false, submitted: false, review: None });
        }
        Effect::Nothing
    }

    pub fn answer_create(&mut self, allow: bool) -> Effect {
        let Some(form) = self.create.as_mut() else { return Effect::Nothing; };
        if form.submitted || form.review.is_none() { return Effect::Nothing; }
        if allow {
            form.submitted = true;
            Effect::Send(Job::ExecuteCreate(form.review.as_ref().unwrap().0.clone()))
        } else {
            form.review = None;
            self.notice = Some("Creation cancelled; no directory or metadata was created".into());
            Effect::Send(Job::CancelEntry)
        }
    }

    fn create_key(&mut self, key: Key) -> Effect {
        let form = self.create.as_mut().unwrap();
        if form.submitted { return Effect::Nothing; }
        if form.review.is_some() {
            return match key {
                Key::Char('y') => self.answer_create(true),
                Key::Char('n') | Key::Escape | Key::CtrlC => self.answer_create(false),
                Key::Up => self.scroll_trust(-1),
                Key::Down => self.scroll_trust(1),
                Key::PageUp => self.scroll_trust(-(PAGE_LINES as isize)),
                Key::PageDown => self.scroll_trust(PAGE_LINES as isize),
                _ => Effect::Nothing,
            };
        }
        match key {
            Key::Escape | Key::CtrlC => { self.create = None; Effect::Send(Job::CancelEntry) }
            Key::Tab | Key::BackTab => { form.branch_focus = !form.branch_focus; Effect::Nothing }
            Key::Enter => {
                if form.path.is_empty() || form.branch.is_empty() { return Effect::Nothing; }
                form.submitted = true;
                Effect::Send(Job::PrepareCreate { path: NativePath::new(form.path.clone()), branch: form.branch.clone() })
            }
            key => {
                let limit = if form.branch_focus { 255 } else { 32 * 1024 };
                let field = if form.branch_focus { &mut form.branch } else { &mut form.path };
                let incoming = match key {
                    Key::Backspace => { field.pop(); return Effect::Nothing; }
                    Key::Space => " ".into(),
                    Key::Char(ch) => ch.to_string(),
                    Key::Text(text) => text,
                    _ => return Effect::Nothing,
                };
                if field.len().saturating_add(incoming.len()) > limit {
                    self.notice = Some(format!("Field limit is {limit} UTF-8 bytes; input was not appended"));
                } else {
                    field.push_str(&incoming);
                    self.notice = None;
                }
                Effect::Nothing
            }
        }
    }
    pub fn trust_review(&self) -> Option<&crate::model::TrustReview> { self.trust.as_ref() }
    pub fn trust_scroll(&self) -> usize { self.trust_scroll }
    pub fn set_trust_scroll_limit(&mut self, limit: usize) {
        self.trust_limit = limit;
        self.trust_scroll = self.trust_scroll.min(limit);
    }
    pub fn scroll_trust(&mut self, delta: isize) -> Effect {
        self.trust_scroll = self.trust_scroll.saturating_add_signed(delta).min(self.trust_limit);
        Effect::Nothing
    }

    pub fn answer_trust(&mut self, allow: bool) -> Effect {
        let Some(review) = self.trust.as_mut() else { return Effect::Nothing; };
        if review.submitted { return Effect::Nothing; }
        if allow {
            review.submitted = true;
            Effect::Send(Job::GrantRepositoryTrust(review.ticket.clone()))
        } else {
            self.trust = None;
            if let Some(prompt) = self.prompt.as_mut() { prompt.submitted = None; }
            self.notice = Some("Trust cancelled; repository opening was not requested".into());
            Effect::Send(Job::CancelEntry)
        }
    }

    pub fn select_recent(&mut self, index: usize) -> Effect {
        if !self.is_welcome() || index >= self.snapshot.recents.len() { return Effect::Nothing; }
        self.trust_scroll = 0;
        self.files = index;
        let entry = &self.snapshot.recents[index];
        self.main = MainView::text(&format!("Recent location ({})\n{}\n\nNot checked or trusted by listing it.\nOpen: inspect this location through the ordinary trust gate.\nLocate: replace this bookmark's path after review.\nRemove: remove this navigation record only; keep files/drafts/trust.", entry.backend, entry.label));
        Effect::Nothing
    }

    pub fn open_recent(&mut self) -> Effect {
        let Some(entry) = self.snapshot.recents.get(self.files) else { return self.request_open(); };
        if entry.backend != "git" {
            self.notice = Some("This backend is not available; the record was preserved".into());
            return Effect::Nothing;
        }
        let Ok(path) = NativePath::from_encoded(entry.path.clone()) else {
            self.notice = Some("This location belongs to another platform; select a local path".into());
            return Effect::Nothing;
        };
        self.open_path(path)
    }

    pub fn open_path(&mut self, path: NativePath) -> Effect {
        let Some(text) = path.as_os_str().to_str() else {
            self.notice = Some("This native path is unavailable with the current Git adapter; no lossy conversion was used".into());
            return Effect::Nothing;
        };
        self.prompt = Some(Prompt { kind: PromptKind::OpenRepository, text: text.into(), submitted: Some(PromptAction::OpenRepository) });
        self.entry(Some(path))
    }

    #[must_use]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    #[must_use]
    pub const fn focus(&self) -> Pane {
        self.focus
    }

    #[must_use]
    pub const fn source(&self) -> Pane {
        self.source
    }

    #[must_use]
    pub fn selected(&self, pane: Pane) -> usize {
        if self.is_welcome() && pane == Pane::Files { return self.files; }
        let raw = match pane {
            Pane::Files => self.files,
            Pane::Branches => self.branches,
            Pane::Commits => self.commits,
            Pane::Stashes => self.stashes,
            Pane::Status | Pane::Main => 0,
        };
        Self::filter_slot(pane).map_or(raw, |i| self.filters[i].rows.iter().position(|r| *r == raw).unwrap_or(usize::MAX))
    }

    #[must_use]
    pub const fn main(&self) -> &MainView {
        &self.main
    }

    #[must_use]
    pub const fn scroll(&self) -> usize {
        self.scroll
    }

    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// The hunk the cursor is in, counted from zero.
    #[must_use]
    pub fn hunk(&self) -> usize {
        self.main.hunks.iter().rposition(|start| *start <= self.cursor).unwrap_or(0)
    }

    /// The first and last line the next Space covers: the cursor line, or the
    /// whole `v` range.
    #[must_use]
    pub fn selection(&self) -> (usize, usize) {
        let anchor = self.anchor.unwrap_or(self.cursor);
        (anchor.min(self.cursor), anchor.max(self.cursor))
    }

    /// Whether a `v` range is open in the main panel.
    #[must_use]
    pub const fn anchor_set(&self) -> bool {
        self.anchor.is_some()
    }

    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    #[must_use]
    pub const fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }

    #[must_use]
    pub fn confirm(&self) -> Option<&Confirm> {
        self.confirm.as_ref()
    }

    pub fn confirmation_lines(&self) -> &[String] {
        &self.confirmation_lines
    }

    pub const fn stash_list_visible(&self) -> bool { self.stash_mode }

    fn selected_change(&self)->Option<&crate::FileRow> {if self.is_welcome()||self.stash_mode||!self.visible_rows(Pane::Files).contains(&self.files){None}else{self.snapshot.files.get(self.files)}}
    pub fn diff_actions_visible(&self)->bool {self.source==Pane::Files&&!self.is_welcome()&&!self.stash_mode}
    pub fn current_file_diff(&self)->bool {
        self.main_ready && self.source==Pane::Files && self.selected_change().is_some_and(|row|self.main.file.as_ref().is_some_and(|target|target.path==row.path))
    }
    pub fn file_view_status(&self)->Option<(&'static str,&'static str)> {
        if !self.diff_actions_visible()||self.selected_change().is_none(){return None;}
        let staged=self.main.file.as_ref().map(|t|t.staged_view).unwrap_or(matches!(&self.last_request,Some(MainRequest::File {staged_view:true,..})));
        Some((if staged {"Staged"}else{"Unstaged"},if self.current_file_diff(){"Ready"}else if self.notice.is_some(){"Unavailable"}else{"Loading"}))
    }
    pub fn change_available(&self,action:ChangesAction)->bool {
        if self.modal(){return false;}
        let Some(row)=self.selected_change() else{return false;};
        let staged=self.main.file.as_ref().is_some_and(|t|t.staged_view);
        match action {
            ChangesAction::StageFile=>row.has_unstaged,ChangesAction::UnstageFile=>row.has_staged,
            ChangesAction::DiscardFile=>row.has_unstaged&&!row.untracked,ChangesAction::Ignore=>true,
            ChangesAction::Unstaged=>row.has_unstaged,ChangesAction::Staged=>row.has_staged,
            ChangesAction::Hunk|ChangesAction::Range=>self.current_file_diff()&&!self.main.hunks.is_empty(),
            ChangesAction::DiscardHunk=>self.current_file_diff()&&!staged&&!row.untracked&&!self.main.hunks.is_empty(),
            ChangesAction::Lines=>{let (a,b)=self.selection();self.current_file_diff()&&self.main.lines.iter().enumerate().any(|(i,l)|i>=a&&i<=b&&matches!(l.tone,Tone::Added|Tone::Removed))},
        }
    }
    pub fn change(&mut self,action:ChangesAction)->Effect {
        if self.modal(){return Effect::Nothing;}
        if action==ChangesAction::DiscardFile {return if self.selected_change().is_some(){self.ask_discard_file()}else{Effect::Nothing};}
        if !self.change_available(action){self.notice=Some(if action==ChangesAction::Lines&&self.current_file_diff(){"select a changed line in the diff"}else{"This action is unavailable for the selected file/view. Select current content or refresh first."}.into());return Effect::Nothing;}
        self.notice=None;
        match action {
            ChangesAction::StageFile|ChangesAction::UnstageFile=>Effect::Send(Job::SetFile {path:self.selected_change().unwrap().path.clone(),stage:action==ChangesAction::StageFile}),
            ChangesAction::Ignore=>self.set_ignore_scope(false),
            ChangesAction::Unstaged|ChangesAction::Staged=>{
                let row=self.selected_change().unwrap().clone();self.source=Pane::Files;self.focus=Pane::Main;
                self.generation+=1;self.cursor=0;self.scroll=0;self.anchor=None;self.main_ready=false;
                self.main=MainView::note("Loading selected file view...");
                let request=MainRequest::File {path:row.path,staged_view:action==ChangesAction::Staged,untracked:row.untracked};
                self.last_request=Some(request.clone());Effect::Send(Job::Main {generation:self.generation,request})
            }
            ChangesAction::Hunk=>{self.focus=Pane::Main;self.stage_hunk()},ChangesAction::Lines=>{self.focus=Pane::Main;self.stage_lines()},
            ChangesAction::Range=>{self.focus=Pane::Main;self.anchor=if self.anchor.is_some(){None}else{Some(self.cursor)};Effect::Nothing},
            ChangesAction::DiscardHunk=>{self.focus=Pane::Main;self.ask_discard_hunk()},ChangesAction::DiscardFile=>unreachable!(),
        }
    }
    pub fn confirmation_ready(&self)->bool {self.prepared_ticket.is_some()}
    pub fn answer_confirmation(&mut self,allow:bool)->Effect {if let Some(confirm)=self.confirm.take(){self.answer(confirm,if allow {Key::Char('y')}else{Key::Char('n')})}else{Effect::Nothing}}
    pub fn set_ignore_scope(&mut self,local:bool)->Effect {
        let path=if let Some(Confirm::Ignore {path,..})=&self.confirm {path.clone()}else{let Some(row)=self.selected_change()else{return Effect::Nothing;};row.path.clone()};
        self.confirm=Some(Confirm::Ignore {path:path.clone(),local});self.prepare_confirmation(Job::Ignore {path,local})
    }

    fn prepare_confirmation(&mut self, action: Job) -> Effect {
        self.trust_scroll=0;
        self.generation += 1;
        self.prepared_ticket = None;
        self.confirmation_lines = vec!["Checking target and recovery capability... y is unavailable until ready.".to_owned()];
        Effect::Send(Job::Prepare { generation: self.generation, action: Box::new(action) })
    }

    #[must_use]
    pub const fn help(&self) -> bool {
        self.help
    }

    /// Whether a prompt, a question or the help panel is open. Pointer input
    /// waits until it closes.
    #[must_use]
    pub const fn modal(&self) -> bool {
        self.locate.is_some() || self.tools.is_some() || self.help || self.prompt.is_some() || self.confirm.is_some() || self.trust.is_some() || self.create.is_some() || self.identity.is_some() || self.filter_editor.is_some() || self.navigation_issue.is_some()
    }

    /// Tells the workspace how many main-panel lines are visible, so the cursor
    /// can be kept on screen.
    pub fn set_view_rows(&mut self, rows: usize) {
        self.view_rows = rows;
        self.follow();
    }

    pub fn key(&mut self, key: Key) -> Effect {
        if let Some(form) = self.locate.as_mut() {
            if !form.editor.preedit.is_empty() { return form.edit(key); }
            if key == Key::Escape { return self.cancel_locate(); }
            if form.review.is_some() { return match key { Key::Char('y') => form.approve(), Key::Char('n') => self.cancel_locate(), Key::Up => self.scroll_trust(-1), Key::Down => self.scroll_trust(1), Key::PageUp => self.scroll_trust(-(PAGE_LINES as isize)), Key::PageDown => self.scroll_trust(PAGE_LINES as isize), _ => Effect::Nothing }; }
            return form.edit(key);
        }
        if let Some(form) = self.tools.as_mut() {
            if form.composed() { return form.edit_key(key); }
            if matches!(key, Key::Escape) { return self.close_tool(); }
            if form.review.is_some() { return match key { Key::Char('y') => form.approve(), Key::Char('n') => self.close_tool(), Key::Up => self.scroll_trust(-1), Key::Down => self.scroll_trust(1), Key::PageUp => self.scroll_trust(-(PAGE_LINES as isize)), Key::PageDown => self.scroll_trust(PAGE_LINES as isize), _ => Effect::Nothing }; }
            return form.edit_key(key);
        }
        match key {
            Key::CloseRepository => return self.close_repository(),
            Key::NextRepository | Key::PreviousRepository => {
                if self.tabs.len() < 2 { return Effect::Nothing; }
                let current = self.tabs.iter().position(|t| Some(&t.path) == self.snapshot.repository.as_ref()).unwrap_or(0);
                return self.select_repository(if matches!(key, Key::NextRepository) { (current + 1) % self.tabs.len() } else { (current + self.tabs.len() - 1) % self.tabs.len() });
            }
            _ => {}
        }
        if self.navigation_issue.is_some() { return match key { Key::Char('y') => self.answer_navigation(true), Key::Char('n') | Key::Escape | Key::CtrlC => self.answer_navigation(false), _ => Effect::Nothing }; }
        if self.filter_editor.is_some() { return self.filter_key(key); }
        if self.composer.is_some() { return self.commit_key(key); }
        if self.identity.is_some() { return self.identity_key(key); }
        if self.create.is_some() { return self.create_key(key); }
        if self.trust.is_some() {
            return match key {
                Key::Down | Key::Char('j') => self.scroll_trust(1),
                Key::Up | Key::Char('k') => self.scroll_trust(-1),
                Key::PageDown => self.scroll_trust(PAGE_LINES as isize),
                Key::PageUp => self.scroll_trust(-(PAGE_LINES as isize)),
                Key::Home => { self.trust_scroll = 0; Effect::Nothing }
                Key::End => { self.trust_scroll = self.trust_limit; Effect::Nothing }
                Key::Char('y') => self.answer_trust(true),
                Key::Char('n') | Key::Escape | Key::CtrlC => self.answer_trust(false),
                _ => Effect::Nothing,
            };
        }
        if let Some(mut prompt) = self.prompt.take() {
            if prompt.kind == PromptKind::GitSettings {
                let extra = match &key { Key::Text(text) => text.len(), Key::Char(ch) => ch.len_utf8(), Key::Space => 1, _ => 0 };
                if extra > 0 && prompt.text.len().saturating_add(extra) > 32 * 1024 {
                    self.notice = Some("Git path limit is 32 KiB UTF-8; input was not appended".into());
                    self.prompt = Some(prompt);
                    return Effect::Nothing;
                }
                if !prompt.pending() && self.git_detail.starts_with("Saved and applied.") && matches!(&key, Key::Text(_) | Key::Char(_) | Key::Space | Key::Backspace) {
                    self.git_detail.insert_str(0, "Unsaved selection. Previously applied choice:\n");
                }
            }
            let (effect, stays_open) = prompt.handle(key);
            if matches!(&effect, Effect::Send(Job::ConfigureGit(_))) {
                self.git_detail = "Testing selection. Previous choice remains active until a successful save.".into();
            }
            if stays_open {
                self.prompt = Some(prompt);
            }
            return if let Effect::Send(Job::OpenRepository(path)) = effect { self.entry(Some(path)) } else { effect };
        }
        if self.confirm.is_some() && matches!(key,Key::PageUp|Key::PageDown|Key::Up|Key::Down) {return self.scroll_trust(match key {Key::Up=>-1,Key::Down=>1,Key::PageUp=>-(PAGE_LINES as isize),_=>PAGE_LINES as isize});}
        if let Some(confirm) = self.confirm.take() {
            return self.answer(confirm, key);
        }
        if self.help {
            if matches!(key, Key::Escape | Key::Char('?')) {
                self.help = false;
            }
            return Effect::Nothing;
        }
        if self.is_welcome() {
            return match key {
                Key::Char('T') => self.request_tools(),
                Key::Char('L') => self.request_locate(),
                Key::Char('G') => self.request_git_settings(),
                Key::Char('N') => self.request_create(),
                Key::Char('O') | Key::CtrlO => self.request_open(),
                Key::Char('q') | Key::CtrlC => Effect::Quit,
                Key::Char('r') => Effect::Send(Job::LoadWelcome),
                Key::PageUp => self.scroll_trust(-(PAGE_LINES as isize)),
                Key::PageDown => self.scroll_trust(PAGE_LINES as isize),
                Key::Enter => self.open_recent(),
                Key::Up | Key::Char('k') => self.select_recent(self.files.saturating_sub(1)),
                Key::Down | Key::Char('j') => self.select_recent((self.files + 1).min(self.snapshot.recents.len().saturating_sub(1))),
                Key::Home => self.select_recent(0),
                Key::End => self.select_recent(self.snapshot.recents.len().saturating_sub(1)),
                Key::Char('d') => self.forget_recent(),
                _ => Effect::Nothing,
            };
        }
        match key {
            Key::Char('T') => self.request_tools(),
            Key::Char('/') => self.request_filter(self.focus),
            Key::Char('I') => self.request_identity(),
            Key::Char('G') => self.request_git_settings(),
            Key::Char('O') | Key::CtrlO => self.request_open(),
            Key::Char('q') | Key::CtrlC => self.request_quit(),
            Key::Char('?') => {
                self.help = true;
                Effect::Nothing
            }
            Key::Char('1') => {
                self.notice = None;
                self.focus = Pane::Status;
                Effect::Nothing
            }
            Key::Char('2') => self.focus_list(Pane::Files),
            Key::Char('3') => self.focus_list(Pane::Branches),
            Key::Char('4') => self.focus_list(Pane::Commits),
            Key::Char('5') => self.focus_list(Pane::Stashes),
            Key::Tab => self.cycle(true),
            Key::BackTab => self.cycle(false),
            Key::Escape => {
                self.notice = None;
                if !self.history_back.is_empty() {
                    return self.return_history();
                } else if self.main.committed.is_some() {
                    self.focus = Pane::Commits;
                    return self.show_source();
                } else if self.anchor.is_some() {
                    self.anchor = None;
                } else if self.focus == Pane::Main {
                    self.focus = self.source;
                } else if !self.filter_query(self.focus).is_empty() {
                    return self.clear_filter(self.focus);
                }
                Effect::Nothing
            }
            Key::Enter => {
                if matches!(self.focus, Pane::Files | Pane::Branches | Pane::Commits) && self.list_length(self.focus) == 0 { return Effect::Nothing; }
                if self.focus == Pane::Files && self.last_request.is_none() { self.focus = Pane::Main; return self.show_source(); }
                if self.focus == Pane::Commits {
                    let Some(commit) = self.snapshot.commits.get(self.commits) else { return Effect::Nothing; };
                    let request = MainRequest::CommitFiles { id: commit.id.clone() };
                    self.history_back = vec![crate::NavigationFrame { request: MainRequest::Commit { id: commit.id.clone() }, cursor: self.cursor, scroll: self.scroll, file: None }];
                    self.restore_detail_file = None;
                    self.focus = Pane::Main;
                    self.generation += 1;
                    self.main = MainView::note("Loading committed files...");
                    self.main_ready = false;
                    self.cursor = 0;
                    self.scroll = 0;
                    self.anchor = None;
                    self.last_request = Some(request.clone());
                    return Effect::Send(Job::Main { generation: self.generation, request });
                }
                if self.focus == Pane::Main && let Some(list) = &self.main.committed {
                        let Some(file) = self.cursor.checked_sub(2).and_then(|i| list.files.get(i)) else { return Effect::Nothing; };
                        let request = MainRequest::CommitFile { id: list.id.clone(), file: file.clone() };
                        self.history_back.push(crate::NavigationFrame { request: MainRequest::CommitFiles { id: list.id.clone() }, cursor: self.cursor, scroll: self.scroll, file: Some(file.path.clone()) });
                        self.generation += 1;
                        self.main = MainView::note("Loading committed file diff...");
                        self.main_ready = false;
                        self.cursor = 0;
                        self.scroll = 0;
                        self.anchor = None;
                        self.last_request = Some(request.clone());
                        return Effect::Send(Job::Main { generation: self.generation, request });
                }
                self.focus = Pane::Main;
                Effect::Nothing
            }
            Key::Up | Key::Char('k') => self.move_by(-1),
            Key::Down | Key::Char('j') => self.move_by(1),
            Key::PageUp | Key::Char('K') => self.move_by(-(PAGE_LINES as isize)),
            Key::PageDown | Key::Char('J') => self.move_by(PAGE_LINES as isize),
            Key::HalfPageUp => self.move_by(-(HALF_PAGE_LINES as isize)),
            Key::HalfPageDown => self.move_by(HALF_PAGE_LINES as isize),
            Key::Home | Key::Char('<') => self.jump(false),
            Key::End | Key::Char('>') => self.jump(true),
            Key::Left | Key::Char('h') => self.step_hunk(false),
            Key::Right | Key::Char('l') => self.step_hunk(true),
            Key::Char('v') if self.focus == Pane::Main => {
                self.anchor = if self.anchor.is_some() { None } else { Some(self.cursor) };
                Effect::Nothing
            }
            Key::Space => self.act(),
            Key::Char('a') => self.act_all(),
            Key::Char('c') => self.request_commit(),
            Key::Char('n') if self.focus == Pane::Branches => {
                self.prompt = Some(Prompt { kind: PromptKind::NewBranch, text: String::new(), submitted: None });
                Effect::Nothing
            }
            Key::Char('d') if self.focus == Pane::Branches => self.ask_delete(),
            Key::Char('d') if self.focus == Pane::Stashes => self.ask_stash_action("drop"),
            Key::Char('p') if self.focus == Pane::Stashes => self.ask_stash_action("pop"),
            Key::Char('d') if self.focus == Pane::Files => self.change(ChangesAction::DiscardFile),
            Key::Char('i') if self.focus == Pane::Files => self.change(ChangesAction::Ignore),
            Key::Char('d') if self.focus == Pane::Main => self.change(ChangesAction::DiscardHunk),
            Key::Char('s') if self.focus == Pane::Files => self.stash(),
            Key::Char('S') if self.focus == Pane::Files => {
                self.focus_list(Pane::Stashes)
            }
            Key::Char('S') if self.focus == Pane::Stashes => self.ask_stash_action("apply"),
            Key::Char('r') => Effect::Send(Job::Reload),
            _ => Effect::Nothing,
        }
    }

    /// Applies a backend answer. Lists are replaced and the main panel is
    /// requested again for the current selection.
    pub fn reply(&mut self, reply: Reply) -> Effect {
        match reply {
            Reply::LocateReview { ticket, lines } => {
                if let Some(form) = self.locate.as_mut() && form.working == Some(crate::LocateWork::Review) { form.working = None; form.review = Some((ticket, lines)); self.trust_scroll = 0; }
                Effect::Nothing
            }
            Reply::RecentLocated { entries, target, detail } => {
                if self.locate.as_ref().is_some_and(|form| form.working == Some(crate::LocateWork::Save)) {
                    self.locate = None;
                    self.files = entries.iter().position(|e| e.backend == "git" && e.path == target).unwrap_or(0);
                    self.snapshot.recents = entries;
                    self.select_recent(self.files); self.notice = Some(detail);
                }
                Effect::Nothing
            }
            Reply::ToolSettings { kind, definition, detail } => {
                if let Some(form) = self.tools.as_mut() && form.kind == kind && matches!(form.working, Some(crate::ToolWork::Load | crate::ToolWork::Save)) { form.load(definition, detail); }
                Effect::Nothing
            }
            Reply::ToolReview { ticket, lines } => {
                if let Some(form) = self.tools.as_mut() && form.working == Some(crate::ToolWork::Review) { form.working = None; form.review = Some((ticket, lines)); self.trust_scroll = 0; }
                Effect::Nothing
            }
            Reply::ToolLaunched(detail) => {
                if self.tools.as_ref().is_some_and(|f| f.working == Some(crate::ToolWork::Launch)) { self.tools = None; self.notice = Some(detail); }
                Effect::Nothing
            }
            Reply::NavigationIssue(issue) => {
                if self.prompt.as_ref().is_some_and(|p| p.submitted == Some(PromptAction::OpenRepository)) { self.navigation_issue = Some(issue); self.notice = None; }
                Effect::Nothing
            }
            Reply::NavigationReset { detail, next, .. } => {
                if !self.navigation_issue.as_ref().is_some_and(|i| i.submitted) { return Effect::Nothing; }
                self.navigation_issue = None;
                let entry_failed = matches!(*next, Reply::PromptFailed { action: PromptAction::OpenRepository, .. });
                let effect = self.reply(*next);
                if !entry_failed { self.notice = Some(detail); }
                effect
            }
            Reply::RepositoryClosed(recents) => {
                if !self.prompt.as_ref().is_some_and(|p| p.submitted == Some(PromptAction::OpenRepository)) { return Effect::Nothing; }
                let quit = self.tab_action.take() == Some(TabAction::Quit);
                let generation = self.generation + 1;
                let mut tabs = std::mem::take(&mut self.tabs);
                if self.closing_tab { tabs.retain(|t| Some(&t.path) != self.snapshot.repository.as_ref()); }
                *self = Self::new(Snapshot::welcome());
                self.generation = generation;
                self.tabs = tabs;
                self.snapshot.recents = recents;
                if quit { return Effect::Quit; }
                self.open()
            }
            Reply::CommitReview { generation, review } => {
                if let Some(form) = self.composer.as_mut() && form.review_generation == generation {
                    form.selected = form.selected.min(review.files.len().saturating_sub(1));
                    self.notice.clone_from(&review.blocked);
                    form.review = Some(review);
                    return self.select_staged(self.composer.as_ref().unwrap().selected);
                }
                Effect::Nothing
            }
            Reply::CommitDiff { generation, view } => {
                if let Some(form) = self.composer.as_mut() && form.diff_generation == generation { form.diff = view; form.diff_scroll = 0; form.diff_ready = true; }
                Effect::Nothing
            }
            Reply::IdentitySettings { ticket, name, email, detail } => {
                if let Some(form) = self.identity.as_mut() && form.submitted {
                    if !form.keep_input { form.name = name; form.email = email; }
                    form.ticket = Some(ticket);
                    form.detail = detail;
                    form.submitted = false;
                    self.trust_scroll = 0;
                }
                Effect::Nothing
            }
            Reply::IdentitySaved { path, review, detail } => {
                if self.identity.is_some() {
                    self.identity = None;
                    self.prompt = Some(Prompt { kind: PromptKind::OpenRepository, text: path.as_os_str().to_str().unwrap_or_default().into(), submitted: review.as_ref().map(|_| PromptAction::OpenRepository) });
                    self.trust = review;
                    self.trust_scroll = 0;
                    self.notice = Some(detail);
                }
                Effect::Nothing
            }
            Reply::GitSettings { path, detail } => {
                if let Some(prompt) = self.prompt.as_mut() && prompt.kind == PromptKind::GitSettings && prompt.submitted == Some(PromptAction::GitSettings) {
                    match path {
                        Some(path) => match path.as_os_str().to_str() {
                            Some(text) => prompt.text = text.into(),
                            None => { self.notice = Some("Saved native Git path cannot be edited as UTF-8; it was preserved. Enter a replacement explicitly or cancel.".into()); }
                        },
                        None => prompt.text.clear(),
                    }
                    prompt.submitted = None;
                    self.git_detail = detail;
                }
                Effect::Nothing
            }
            Reply::CreateReview { ticket, path, branch, new_directory, entries, more_entries } => {
                if let Some(form) = self.create.as_mut() && form.submitted {
                    let mut lines = vec![format!("Location: {}", path.display_label()), format!("Initial branch: {branch}"),
                        if new_directory { "Create a new directory and .git metadata.".into() } else { "Create .git metadata inside the existing directory.".into() },
                        "Existing contents are preserved. Nothing is staged or committed.".into(),
                        "Use an empty template (no copied template hooks). SHA-1 objects, files refs.".into(),
                        "Installed Git runs under inherited user configuration.".into(),
                        "After init, review repository trust separately before opening.".into(),
                        "Failure may leave partial metadata for inspection; no automatic deletion or retry.".into(),
                        "Review expires after 60 seconds.".into(), String::new(), "Existing top-level entries (up to 50; not traversed):".into()];
                    lines.extend(entries);
                    if more_entries { lines.push("More entries exist; preview is incomplete. Existing content will not be staged.".into()); }
                    lines.push("y: Create repository   n/Esc: Back   Enter does not approve".into());
                    form.review = Some((ticket, lines));
                    form.submitted = false;
                    self.trust_scroll = 0;
                }
                Effect::Nothing
            }
            Reply::Welcome(entries) => {
                if self.is_welcome() {
                    self.snapshot.recents = entries;
                    self.files = clamp_index(self.files, self.snapshot.recents.len());
                    if !self.snapshot.recents.is_empty() { return self.select_recent(self.files); }
                    self.main = MainView::text("Welcome to MiniCon SrcMgr\nOpen a local Git repository with Ctrl+O or Open.\nThere are no recent repositories yet.\nAn installed Git is needed only when opening a repository.\nNo account or network is required for local work.");
                }
                Effect::Nothing
            }
            Reply::TrustReview(review) => {
                if self.create.as_ref().is_some_and(|form| form.submitted && form.review.is_some()) {
                    let path = review.worktree.as_os_str().to_str().unwrap_or_default().to_owned();
                    self.create = None;
                    self.prompt = Some(Prompt { kind: PromptKind::OpenRepository, text: path, submitted: Some(PromptAction::OpenRepository) });
                    self.notice = Some("Repository created. Review trust before opening; existing files remain unstaged.".into());
                }
                if self.prompt.as_ref().is_some_and(|prompt| prompt.submitted == Some(PromptAction::OpenRepository)) {
                    self.trust = Some(review);
                    self.trust_scroll = 0;
                }
                Effect::Nothing
            }
            Reply::RepositoryOpened(snapshot) => {
                if !self.prompt.as_ref().is_some_and(|prompt| prompt.submitted == Some(PromptAction::OpenRepository)) {
                    return Effect::Nothing;
                }
                let generation = self.generation + 1;
                let mut tabs = std::mem::take(&mut self.tabs);
                if let Some(active) = tabs.iter_mut().find(|t| Some(&t.path) == self.snapshot.repository.as_ref()) { active.head.clone_from(&self.snapshot.head); }
                if self.closing_tab { tabs.retain(|t| Some(&t.path) != self.snapshot.repository.as_ref()); }
                if let Some(path) = &snapshot.repository && !tabs.iter().any(|t| &t.path == path) { tabs.push(RepositoryTab { path: path.clone(), head: snapshot.head.clone() }); }
                *self = Self::new(snapshot);
                self.tabs = tabs;
                self.generation = generation;
                self.open()
            }
            Reply::NoChange => Effect::Nothing,
            Reply::MainFailed { generation, text } => {
                if let Some(form) = self.composer.as_mut() && form.diff_generation == generation {
                    form.diff_ready = false;
                    form.diff = MainView::note(&text);
                    if let Some(review) = form.review.as_mut() { review.blocked = Some("Staged Diff is unavailable or stale; Refresh review before committing".into()); }
                    self.notice = Some(text);
                    return Effect::Nothing;
                }
                if generation == self.generation {
                    self.main = MainView::text(&text);
                    self.cursor = 0;
                    self.scroll = 0;
                    self.anchor = None;
                    self.notice = Some(text);
                }
                Effect::Nothing
            }
            Reply::ConfirmationFailed { generation, text } => {
                if generation == self.generation && self.confirm.is_some() {
                    self.confirm = None;
                    self.prepared_ticket = None;
                    self.confirmation_lines.clear();
                    self.notice = Some(text);
                }
                Effect::Nothing
            }
            Reply::Prepared { generation, ticket, lines } => {
                if generation == self.generation && self.confirm.is_some() {
                    self.prepared_ticket = Some(ticket);
                    self.confirmation_lines = lines;
                }
                Effect::Nothing
            }
            Reply::Full(snapshot) => {
                let interrupted_confirmation = self.confirm.take().is_some();
                if interrupted_confirmation {
                    self.prepared_ticket = None;
                    self.confirmation_lines.clear();
                }
                let selected_file = self.snapshot.files.get(self.files).map(|file| file.path.clone());
                let selected_branch = self.snapshot.branches.get(self.branches).map(|branch| branch.name.clone());
                let selected_commit = self.snapshot.commits.get(self.commits).map(|commit| commit.id.clone());
                let selected_stash = self.snapshot.stashes.get(self.stashes).map(|s| s.id.clone());
                self.snapshot = snapshot;
                self.files = selected_file.and_then(|path| self.snapshot.files.iter().position(|file| file.path == path))
                    .unwrap_or_else(|| clamp_index(self.files, self.snapshot.files.len()));
                self.branches = selected_branch.and_then(|name| self.snapshot.branches.iter().position(|branch| branch.name == name))
                    .unwrap_or_else(|| clamp_index(self.branches, self.snapshot.branches.len()));
                self.commits = selected_commit.and_then(|id| self.snapshot.commits.iter().position(|commit| commit.id == id))
                    .unwrap_or_else(|| clamp_index(self.commits, self.snapshot.commits.len()));
                self.stashes = clamp_index(self.stashes, self.snapshot.stashes.len());
                let removed_stash = selected_stash.filter(|id| {
                    if let Some(index) = self.snapshot.stashes.iter().position(|s| &s.id == id) {
                        self.stashes = index;
                        false
                    } else { true }
                });
                self.load_filter_rows();
                self.reconcile_filter_selection();
                self.notice = None;
                if let Some(form) = self.filter_editor.as_mut() {
                    form.previous_view = None;
                    form.previous_request = None;
                    self.generation += 1;
                    self.last_request = None;
                    self.main = MainView::note("Loaded items refreshed. Apply the query to inspect a current result.");
                    return Effect::Nothing;
                }
                if self.source == Pane::Stashes && self.snapshot.stage_result.is_none() {
                    self.notice = removed_stash.map(|id| format!("Selected stash {id} was removed; list refreshed"));
                }
                let effect = self.show_source();
                if interrupted_confirmation {
                    self.notice = Some("Repository state refreshed; confirmation cancelled. Preview the action again.".to_owned());
                    self.main = MainView::note("State refreshed. Press the pane number to reload details.");
                    Effect::Send(Job::CancelPrepared)
                } else {
                    effect
                }
            }
            Reply::Main { generation, view } => {
                if generation == self.generation {
                    self.main = view;
                    self.main_ready = true;
                    self.scroll = 0;
                    self.anchor = None;
                    // The cursor starts on the first changed line, or the top.
                    let last = self.main.lines.len().saturating_sub(1);
                    self.cursor = if self.main.committed.as_ref().is_some_and(|list| !list.files.is_empty()) { 2 } else { self.main.hunks.first().map_or(0, |start| (start + 1).min(last)) };
                    self.follow();
                    if let Some((expected, cursor, scroll)) = self.restore_position.take() && expected == generation {
                        self.cursor = cursor.min(self.main.lines.len().saturating_sub(1));
                        self.scroll = scroll.min(self.main.lines.len().saturating_sub(self.view_rows.max(1)));
                        self.follow();
                    }
                    if let Some(path) = self.restore_detail_file.take() && let Some(list) = &self.main.committed {
                        if let Some(index) = list.files.iter().position(|f| f.path == path) { self.cursor = index + 2; self.follow(); }
                        else { self.notice = Some("Previous committed file is unavailable in this comparison; current list is displayed".into()); }
                    }
                }
                Effect::Nothing
            }
            Reply::Notice(text) => {
                self.notice = Some(text);
                Effect::Nothing
            }
            Reply::PromptFailed { action, text } => {
                if action == PromptAction::LocateRepository {
                    if let Some(form) = self.locate.as_mut() { form.working = None; form.review = None; form.detail = text; }
                    return Effect::Nothing;
                }
                if action == PromptAction::ExternalTool {
                    if let Some(form) = self.tools.as_mut() { form.working = None; form.review = None; form.detail = text; }
                    return Effect::Nothing;
                }
                if action == PromptAction::Commit && let Some(form) = self.composer.as_mut() {
                    if let Some(review) = form.review.as_mut() { review.blocked = Some("Submission was refused; Refresh review before retrying".into()); }
                    self.generation += 1;
                    form.diff_generation = self.generation;
                }
                if action == PromptAction::SaveDraft && let Some(form) = self.composer.as_mut() {
                    form.in_flight = None;
                    form.save_error = Some(text.clone());
                }
                if action == PromptAction::GitSettings { self.git_detail = "Selection was not saved or applied. Correct the path and retry, or close to keep the previous choice.".into(); }
                if action == PromptAction::IdentitySettings && let Some(form) = self.identity.as_mut() {
                    form.submitted = false;
                    form.ticket = None;
                    form.keep_input = true;
                }
                if action == PromptAction::CreateRepository && let Some(form) = self.create.as_mut() {
                    form.submitted = false;
                    form.review = None;
                }
                if action == PromptAction::OpenRepository {
                    self.navigation_issue = None;
                    self.trust = None;
                    self.closing_tab = false;
                    self.tab_action = None;
                    if self.tab_entry { self.prompt = None; self.tab_entry = false; }
                }
                self.notice = Some(text);
                if let Some(prompt) = self.prompt.as_mut() && prompt.submitted == Some(action) {
                    prompt.submitted = None;
                }
                Effect::Nothing
            }
            Reply::PromptCompleted { action, text } => {
                if action == PromptAction::Commit && let Some(form) = &self.composer { self.snapshot.draft = Some(form.editor.text.clone()); }
                self.close_prompt(action);
                self.generation += 1;
                self.main = MainView::text(&text);
                self.cursor = 0;
                self.scroll = 0;
                self.anchor = None;
                self.history_back.clear();
        self.restore_detail_file = None;
                self.notice = Some(text.lines().next().unwrap_or(&text).to_owned());
                Effect::Nothing
            }
            Reply::Committed(snapshot) => {
                self.close_prompt(PromptAction::Commit);
                self.reply(Reply::Full(snapshot))
            }
            Reply::BranchCreated(snapshot) => {
                self.close_prompt(PromptAction::CreateBranch);
                self.reply(Reply::Full(snapshot))
            }
            Reply::DraftSaved(draft) => {
                if let Some(form) = self.composer.as_mut() {
                    if let Some(requested) = form.in_flight.take() {
                        let expected = if requested.trim().is_empty() { None } else { Some(requested.as_str()) };
                        if draft.as_deref() != expected { form.save_error = Some("Draft acknowledgement did not match the submitted text; retry saving".into()); return Effect::Nothing; }
                        form.saved = requested;
                        form.save_error = None;
                        self.snapshot.draft = draft;
                        if form.editor.text == form.saved { form.dirty_since = None; }
                        if form.closing {
                            let quit = form.quit;
                            let effect = self.close_commit(quit);
                            if self.composer.is_none() && self.tab_action.is_some() { return self.finish_repository_action(); }
                            return effect;
                        }
                        return Effect::Nothing;
                    }
                    return Effect::Nothing;
                }
                self.close_prompt(PromptAction::SaveDraft);
                self.snapshot.draft = draft;
                self.notice = Some("Draft saved".to_owned());
                Effect::Nothing
            }
        }
    }

    fn close_prompt(&mut self, action: PromptAction) {
        if self.prompt.as_ref().is_some_and(|prompt| prompt.submitted == Some(action)) {
            self.prompt = None;
            if matches!(action, PromptAction::Commit | PromptAction::SaveDraft) { self.composer = None; }
        }
    }

    pub fn request_open(&mut self) -> Effect {
        if !self.modal() {
            self.notice = None;
            self.prompt = Some(Prompt { kind: PromptKind::OpenRepository, text: String::new(), submitted: None });
        }
        Effect::Nothing
    }

    /// A left click on a list. Focuses the list and selects the row; a row past
    /// the end of the list only focuses it.
    pub fn click_row(&mut self, pane: Pane, index: usize) -> Effect {
        if pane == Pane::Status || index >= self.list_length(pane) {
            return self.focus_list(pane);
        }
        let changed = self.selected(pane) != index || self.source != pane || self.last_request.is_none();
        self.notice = None;
        self.focus = pane;
        self.set_selected(pane, index);
        if !changed {
            return Effect::Nothing;
        }
        self.source = pane;
        self.show_source()
    }

    /// A left click in the main panel. Focuses it and, when `line` (an index into
    /// the whole diff) is known, moves the cursor there.
    pub fn click_main(&mut self, line: Option<usize>) -> Effect {
        self.notice = None;
        self.focus = Pane::Main;
        if let Some(line) = line {
            self.anchor = None;
            self.place_cursor(line);
        }
        Effect::Nothing
    }

    /// The wheel over a list moves its selection and shows it in the main panel,
    /// but keeps the focus where it is, as the wheel scrolls a view under the
    /// pointer rather than switching panels. Over the main panel it scrolls the
    /// content and focuses it.
    pub fn wheel(&mut self, pane: Pane, lines: isize) -> Effect {
        if self.is_welcome() && pane == Pane::Files {
            let index = self.files.saturating_add_signed(lines).min(self.snapshot.recents.len().saturating_sub(1));
            return self.select_recent(index);
        }
        if pane == Pane::Main {
            self.focus = Pane::Main;
            return self.scroll_by(lines);
        }
        let length = self.list_length(pane);
        if pane == Pane::Status || length == 0 {
            return Effect::Nothing;
        }
        let index = (self.selected(pane) as isize + lines).clamp(0, length as isize - 1) as usize;
        if self.selected(pane) == index {
            return Effect::Nothing;
        }
        self.notice = None;
        self.set_selected(pane, index);
        self.source = pane;
        self.show_source()
    }

    fn answer(&mut self, confirm: Confirm, key: Key) -> Effect {
        if matches!(key, Key::Char('n') | Key::Escape) {
            self.prepared_ticket = None;
            self.confirmation_lines.clear();
            return Effect::Send(Job::CancelPrepared);
        }
        if key == Key::Char('y') {
            if let Some(ticket) = self.prepared_ticket.take() {
                self.confirmation_lines.clear();
                return Effect::Send(Job::ExecutePrepared(ticket));
            }
            self.confirm = Some(confirm);
            return Effect::Nothing;
        }
        self.confirm = Some(confirm);
        Effect::Nothing
    }

    fn focus_list(&mut self, pane: Pane) -> Effect {
        if pane == Pane::Stashes { self.stash_mode = true; }
        if pane == Pane::Files { self.stash_mode = false; }
        self.notice = None;
        self.focus = pane;
        if pane == Pane::Status || self.source == pane {
            return Effect::Nothing;
        }
        self.source = pane;
        self.show_source()
    }

    fn cycle(&mut self, forward: bool) -> Effect {
        const ORDER: [Pane; 6] = [Pane::Status, Pane::Files, Pane::Stashes, Pane::Branches, Pane::Commits, Pane::Main];
        let index = ORDER.iter().position(|pane| *pane == self.focus).unwrap_or(0);
        let next =
            if forward { ORDER[(index + 1) % ORDER.len()] } else { ORDER[(index + ORDER.len() - 1) % ORDER.len()] };
        if next == Pane::Main {
            self.focus = Pane::Main;
            return Effect::Nothing;
        }
        self.focus_list(next)
    }

    /// Moves the list selection, or the main-panel cursor when the main panel
    /// has focus. A clamped move asks for nothing.
    fn move_by(&mut self, delta: isize) -> Effect {
        match self.focus {
            Pane::Main => {
                let last = self.main.lines.len().saturating_sub(1) as isize;
                let target = (self.cursor as isize + delta).clamp(0, last) as usize;
                self.place_cursor(target);
                Effect::Nothing
            }
            Pane::Status => Effect::Nothing,
            pane => {
                let length = self.list_length(pane);
                if length == 0 {
                    return Effect::Nothing;
                }
                let target = (self.selected(pane) as isize + delta).clamp(0, length as isize - 1) as usize;
                self.click_row(pane, target)
            }
        }
    }

    /// `<` and `>`: the first or last row of a list, or the top or bottom of the
    /// main panel.
    fn jump(&mut self, end: bool) -> Effect {
        match self.focus {
            Pane::Main => {
                let target = if end { self.main.lines.len().saturating_sub(1) } else { 0 };
                self.place_cursor(target);
                Effect::Nothing
            }
            Pane::Status => Effect::Nothing,
            pane => {
                let length = self.list_length(pane);
                if length == 0 {
                    return Effect::Nothing;
                }
                self.click_row(pane, if end { length - 1 } else { 0 })
            }
        }
    }

    fn scroll_by(&mut self, delta: isize) -> Effect {
        let next = (self.scroll as isize + delta).clamp(0, self.max_scroll() as isize);
        self.scroll = next as usize;
        Effect::Nothing
    }

    fn place_cursor(&mut self, line: usize) {
        self.cursor = line.min(self.main.lines.len().saturating_sub(1));
        self.follow();
    }

    /// Keeps the cursor inside the visible rows of the main panel.
    fn follow(&mut self) {
        let rows = self.view_rows.max(1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        }
        if self.cursor >= self.scroll + rows {
            self.scroll = self.cursor + 1 - rows;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn step_hunk(&mut self, forward: bool) -> Effect {
        if self.focus != Pane::Main || self.main.hunks.is_empty() {
            return Effect::Nothing;
        }
        let last = self.main.hunks.len() - 1;
        let current = self.hunk();
        let next = if forward { (current + 1).min(last) } else { current.saturating_sub(1) };
        let line = self.main.hunks[next];
        self.place_cursor(line);
        Effect::Nothing
    }

    /// Space: stage or check out the focused item, or stage the selected lines.
    fn act(&mut self) -> Effect {
        self.notice = None;
        match self.focus {
            Pane::Status => Effect::Nothing,
            Pane::Files => match self.snapshot.files.get(self.files) {
                Some(row) => self.change(if row.has_unstaged {ChangesAction::StageFile}else{ChangesAction::UnstageFile}),
                None => Effect::Nothing,
            },
            Pane::Branches => match self.snapshot.branches.get(self.branches) {
                Some(branch) if branch.current => {
                    self.notice = Some(format!("{} is already checked out", branch.name));
                    Effect::Nothing
                }
                Some(branch) => Effect::Send(Job::Switch(branch.name.clone())),
                None => Effect::Nothing,
            },
            Pane::Commits => self.ask_detach(),
            Pane::Stashes => self.ask_stash_action("apply"),
            Pane::Main => self.change(ChangesAction::Lines),
        }
    }

    /// `a`: stage or unstage every file, or stage the hunk under the cursor.
    fn act_all(&mut self) -> Effect {
        self.notice = None;
        if self.focus == Pane::Files && !self.filter_query(Pane::Files).is_empty() { self.notice = Some("Clear Files filter to review the full operation scope".into()); return Effect::Nothing; }
        match self.focus {
            Pane::Files => {
                let stage = self.snapshot.files.iter().any(|file| file.has_unstaged);
                let any_staged = self.snapshot.files.iter().any(|file| file.has_staged);
                if stage || any_staged {
                    Effect::Send(Job::SetAll { stage })
                } else {
                    self.notice = Some("nothing to stage or unstage".to_owned());
                    Effect::Nothing
                }
            }
            Pane::Main => self.change(ChangesAction::Hunk),
            Pane::Status | Pane::Branches | Pane::Commits | Pane::Stashes => Effect::Nothing,
        }
    }

    /// Sends the changed lines inside the selection, limited to the cursor's
    /// hunk. The backend stages them when the diff is unstaged and unstages them
    /// when it is staged.
    fn stage_lines(&mut self) -> Effect {
        let Some(target) = self.main.file.clone() else {
            self.notice = Some("Space acts on a file diff".to_owned());
            return Effect::Nothing;
        };
        let number = self.hunk();
        let Some(&start) = self.main.hunks.get(number) else {
            self.notice = Some("this diff has no hunks".to_owned());
            return Effect::Nothing;
        };
        let end = self.main.hunks.get(number + 1).copied().unwrap_or(self.main.lines.len());
        let (first, last) = self.selection();
        let lines: Vec<usize> = (first.max(start + 1)..=last.min(end.saturating_sub(1)))
            .filter(|&line| matches!(self.main.lines[line].tone, Tone::Added | Tone::Removed))
            .map(|line| line - start)
            .collect();
        if lines.is_empty() {
            self.notice = Some("select a changed line in the diff".to_owned());
            return Effect::Nothing;
        }
        self.anchor = None;
        Effect::Send(Job::SetLines {
            path: target.path,
            staged_view: target.staged_view,
            number: number + 1,
            lines,
        })
    }

    /// Stages the whole hunk under the cursor, or unstages it in the index view.
    fn stage_hunk(&mut self) -> Effect {
        let Some(target) = self.main.file.clone() else {
            self.notice = Some("a acts on a file diff".to_owned());
            return Effect::Nothing;
        };
        if self.main.hunks.is_empty() {
            self.notice = Some("this diff has no hunks".to_owned());
            return Effect::Nothing;
        }
        Effect::Send(Job::SetHunk { path: target.path, staged_view: target.staged_view, number: self.hunk() + 1 })
    }

    fn ask_delete(&mut self) -> Effect {
        self.notice = None;
        let Some(branch) = self.snapshot.branches.get(self.branches) else {
            return Effect::Nothing;
        };
        if branch.current {
            self.notice = Some(format!("{} is checked out and cannot be deleted", branch.name));
            return Effect::Nothing;
        }
        self.confirm = Some(Confirm::DeleteBranch(branch.name.clone()));
        self.prepare_confirmation(Job::DeleteBranch(branch.name.clone()))
    }

    /// `d` in Files: asks before the working-tree changes of the file are lost.
    /// Staged content is kept, so only the unstaged part is in question.
    fn ask_discard_file(&mut self) -> Effect {
        self.notice = None;
        let Some(row) = self.snapshot.files.get(self.files) else {
            return Effect::Nothing;
        };
        if !row.has_unstaged {
            self.notice = Some(format!("{} has no unstaged changes to discard", row.path.display_label()));
            return Effect::Nothing;
        }
        self.confirm = Some(Confirm::DiscardFile { path: row.path.clone(), untracked: row.untracked });
        self.prepare_confirmation(Job::Discard { path: row.path.clone(), untracked: row.untracked })
    }

    /// `d` in the main panel: asks before the unstaged hunk under the cursor is
    /// thrown away. Only the unstaged view of a tracked file can be discarded.
    fn ask_discard_hunk(&mut self) -> Effect {
        self.notice = None;
        let Some(target) = self.main.file.clone() else {
            self.notice = Some("discard works on a file diff".to_owned());
            return Effect::Nothing;
        };
        if target.staged_view {
            self.notice = Some("discard works on unstaged changes; this view shows staged ones".to_owned());
            return Effect::Nothing;
        }
        if self.snapshot.files.iter().any(|file| file.path == target.path && file.untracked) {
            self.notice = Some("an untracked file is discarded from the Files panel".to_owned());
            return Effect::Nothing;
        }
        if self.main.hunks.is_empty() {
            self.notice = Some("this change has no hunk to discard".to_owned());
            return Effect::Nothing;
        }
        let index = self.hunk();
        self.confirm = Some(Confirm::DiscardHunk { path: target.path.clone(), index });
        self.prepare_confirmation(Job::DiscardHunk { path: target.path.clone(), index })
    }

    /// `s` in Files: saves the tracked changes as a stash. Untracked files are
    /// left alone, so a list of only untracked files has nothing to save.
    fn stash(&mut self) -> Effect {
        self.notice = None;
        if !self.filter_query(Pane::Files).is_empty() { self.notice = Some("Clear Files filter to review the full operation scope".into()); return Effect::Nothing; }
        if self.snapshot.files.iter().all(|file| file.untracked) {
            self.notice = Some("no tracked changes to stash".to_owned());
            return Effect::Nothing;
        }
        Effect::Send(Job::Stash)
    }

    /// Space in Commits: asks before HEAD moves to the commit, detached.
    fn ask_detach(&mut self) -> Effect {
        self.notice = None;
        let Some(commit) = self.snapshot.commits.get(self.commits) else {
            return Effect::Nothing;
        };
        let short: String = commit.id.chars().take(7).collect();
        self.confirm = Some(Confirm::Detach { id: commit.id.clone(), short });
        self.prepare_confirmation(Job::Detach(commit.id.clone()))
    }

    fn ask_stash_action(&mut self, operation: &str) -> Effect {
        let Some(stash) = self.snapshot.stashes.get(self.stashes) else { return Effect::Nothing; };
        let id = stash.id.clone();
        let expected = self.snapshot.stashes.iter().map(|s| s.id.clone()).collect();
        self.confirm = Some(Confirm::Stash { id: id.clone(), operation: operation.to_owned() });
        self.prepare_confirmation(Job::StashAction { id, expected, operation: operation.to_owned() })
    }

    /// Asks for the main-panel content of the source list. The main panel
    /// shows a loading note until the reply arrives.
    fn show_source(&mut self) -> Effect {
        let previous_side=match &self.last_request {Some(MainRequest::File {path,staged_view,..})=>Some((path.clone(),*staged_view)),_=>None};
        self.main_ready = false;
        let position = self.next_position.take().filter(|(pane, identity, _, _)| *pane == self.source && *identity == self.selected_identity(self.source));
        self.last_request = None;
        self.restore_position = None;
        self.history_back.clear();
        self.restore_detail_file = None;
        self.generation += 1;
        self.scroll = 0;
        self.cursor = 0;
        self.anchor = None;
        let request = match self.source {
            Pane::Stashes => match self.snapshot.stashes.get(self.stashes) {
                Some(stash) => MainRequest::Stash { id: stash.id.clone() },
                None => { self.main = MainView::note("No stashes (list bound: 200)"); return Effect::Nothing; }
            },
            Pane::Files => match self.snapshot.files.get(self.files) {
                Some(row) => MainRequest::File {
                    path: row.path.clone(),
                    staged_view: previous_side.as_ref().filter(|(path,staged)|path==&row.path&&if *staged{row.has_staged}else{row.has_unstaged}).map_or(!row.has_unstaged,|(_,staged)|*staged),
                    untracked: row.untracked,
                },
                None => {
                    self.main = MainView::note(if self.filter_query(Pane::Files).is_empty() { "Working tree clean" } else { "No matching loaded files. Clear the Files filter to see all loaded changes." });
                    return Effect::Nothing;
                }
            },
            Pane::Commits => match self.snapshot.commits.get(self.commits) {
                Some(commit) => MainRequest::Commit { id: commit.id.clone() },
                None => {
                    self.main = MainView::note(if self.filter_query(Pane::Commits).is_empty() { "No commits yet" } else { "No matching loaded commits. This filter does not search unloaded history." });
                    return Effect::Nothing;
                }
            },
            Pane::Branches => {
                self.main = MainView::note("Space checks out the selected branch");
                return Effect::Nothing;
            }
            Pane::Status | Pane::Main => return Effect::Nothing,
        };
        self.main = MainView {
            lines: vec![Line { tone: Tone::Muted, text: "Loading...".to_owned() }],
            hunks: Vec::new(),
            file: None,
            committed: None,
        };
        self.last_request = Some(request.clone());
        self.restore_position = position.map(|(_, _, cursor, scroll)| (self.generation, cursor, scroll));
        Effect::Send(Job::Main { generation: self.generation, request })
    }

    /// The Status panel has one row; the others have one row per item.
    fn list_length(&self, pane: Pane) -> usize {
        if let Some(i) = Self::filter_slot(pane) { return self.filters[i].rows.len(); }
        match pane {
            Pane::Status => 1,
            Pane::Files => self.snapshot.files.len(),
            Pane::Branches => self.snapshot.branches.len(),
            Pane::Commits => self.snapshot.commits.len(),
            Pane::Stashes => self.snapshot.stashes.len(),
            Pane::Main => 0,
        }
    }

    fn set_selected(&mut self, pane: Pane, value: usize) {
        let value = Self::filter_slot(pane).map_or(value, |i| self.filters[i].rows.get(value).copied().unwrap_or(usize::MAX));
        match pane {
            Pane::Files => self.files = value,
            Pane::Branches => self.branches = value,
            Pane::Commits => self.commits = value,
            Pane::Stashes => self.stashes = value,
            Pane::Status | Pane::Main => {}
        }
    }

    fn max_scroll(&self) -> usize {
        self.main.lines.len().saturating_sub(1)
    }
}

fn clamp_index(index: usize, length: usize) -> usize {
    if length == 0 { 0 } else { index.min(length - 1) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn history_return_path_roundtrips_without_cached_diffs_and_back_precedes_filter_clear() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.set_view_rows(6);
        workspace.request_filter(Pane::Commits);
        workspace.key(Key::Text("subject 2".into()));
        let Job::Main { generation, .. } = sent(workspace.key(Key::Enter)) else { panic!() };
        let summary = MainView::text(&(0..100).map(|i| format!("summary {i}\n")).collect::<String>());
        workspace.reply(Reply::Main { generation, view: summary.clone() });
        workspace.click_main(Some(42));
        workspace.focus = Pane::Commits;
        let summary_position = (workspace.cursor, workspace.scroll);
        let Job::Main { generation, .. } = sent(workspace.key(Key::Enter)) else { panic!() };
        let files: Vec<_> = (0..30).map(|i| crate::CommittedFile { status: "A".into(), old_mode: "000000".into(), new_mode: "100644".into(), path: NativePath::new(format!("f{i}.txt")), old_path: None }).collect();
        let mut list = MainView::text(&format!("Commit 2\nBase: root\n{}", (0..30).map(|i| format!("f{i}.txt\n")).collect::<String>()));
        list.committed = Some(crate::CommittedFiles { id: "2".into(), base: "root".into(), files });
        workspace.reply(Reply::Main { generation, view: list.clone() });
        workspace.key(Key::End);
        let list_position = (workspace.cursor, workspace.scroll);
        let Job::Main { generation, .. } = sent(workspace.key(Key::Enter)) else { panic!() };
        let diff = MainView::text(&(0..80).map(|i| format!("diff {i}\n")).collect::<String>());
        workspace.reply(Reply::Main { generation, view: diff.clone() });
        workspace.click_main(Some(50));
        let navigation = workspace.navigation();
        assert_eq!(navigation.back.len(), 2);
        let encoded = navigation.encode();
        let mut broken = navigation.clone();
        broken.back[0].request = MainRequest::Commit { id: "wrong commit".into() };
        assert!(crate::Navigation::decode(&broken.encode()).is_err());
        let mut broken = navigation.clone();
        broken.source = Pane::Files;
        assert!(crate::Navigation::decode(&broken.encode()).is_err(), "restored history cannot expose worktree actions from another pane");
        let mut restored = snapshot(None);
        restored.navigation = Some(crate::Navigation::decode(&encoded).unwrap());
        let mut resumed = Workspace::new(restored);
        resumed.set_view_rows(6);
        let Job::Main { generation: first, request: MainRequest::CommitFile { id, file } } = sent(resumed.open()) else { panic!() };
        assert_eq!((id.as_str(), file.path.as_os_str()), ("2", NativePath::new("f29.txt").as_os_str()));
        resumed.reply(Reply::Main { generation: first, view: diff });
        let Job::Main { generation, request: MainRequest::CommitFiles { id } } = sent(resumed.key(Key::Escape)) else { panic!() };
        assert_eq!(id, "2");
        resumed.reply(Reply::Main { generation: first, view: MainView::text("obsolete diff") });
        resumed.reply(Reply::Main { generation, view: list });
        assert_eq!((resumed.cursor, resumed.scroll), list_position);
        assert_eq!(resumed.selected_detail_file(), Some(NativePath::new("f29.txt")));
        assert_eq!(resumed.filter_query(Pane::Commits), "subject 2");
        let Job::Main { generation, request: MainRequest::Commit { id } } = sent(resumed.key(Key::Escape)) else { panic!() };
        assert_eq!(id, "2");
        resumed.reply(Reply::Main { generation, view: summary });
        assert_eq!(resumed.focus, Pane::Commits);
        assert_eq!((resumed.cursor, resumed.scroll), summary_position);
        assert_eq!(resumed.filter_query(Pane::Commits), "subject 2");
        assert_eq!(resumed.key(Key::Escape), Effect::Nothing);
        assert!(resumed.filter_query(Pane::Commits).is_empty());
    }

    #[test]
    fn navigation_recovery_requires_explicit_approval_and_preserves_cancelled_input() {
        let mut workspace = Workspace::new(Snapshot::welcome());
        workspace.request_open();
        workspace.key(Key::Text("owned repo".into()));
        workspace.key(Key::Enter);
        let issue = crate::NavigationIssue { ticket: "reviewed view".into(), path: NativePath::new("owned repo"), reason: "Future version".into(), submitted: false };
        workspace.reply(Reply::NavigationIssue(issue.clone()));
        assert_eq!(workspace.key(Key::Enter), Effect::Nothing);
        assert_eq!(workspace.key(Key::Escape), Effect::Send(Job::CancelEntry));
        assert_eq!(workspace.prompt().unwrap().text(), "owned repo");
        workspace.key(Key::Enter);
        workspace.reply(Reply::NavigationIssue(issue));
        assert_eq!(workspace.key(Key::Char('y')), Effect::Send(Job::ResetNavigation("reviewed view".into())));
        assert_eq!(workspace.key(Key::Escape), Effect::Nothing, "submitted archive is not silently cancelled");
        workspace.reply(Reply::PromptFailed { action: PromptAction::OpenRepository, text: "View changed; reopen for a fresh review".into() });
        assert!(workspace.navigation_issue().is_none());
        assert!(workspace.is_welcome());
        assert_eq!(workspace.prompt().unwrap().text(), "owned repo");
    }

    #[test]
    fn quick_filter_cancel_during_a_pending_detail_keeps_an_explicit_reload_path() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open(); // Detail has been requested but has not completed.
        workspace.request_filter(Pane::Files);
        workspace.key(Key::Text("a".into()));
        workspace.key(Key::Escape);
        assert!(workspace.main.file.is_none());
        assert!(matches!(workspace.key(Key::Enter), Effect::Send(Job::Main { .. })), "cancel must not restore a loading placeholder whose request was invalidated");
    }

    #[test]
    fn quick_filter_projects_loaded_rows_without_work_and_keeps_raw_action_targets() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.click_row(Pane::Files, 2);
        assert_eq!(workspace.key(Key::Char('/')), Effect::Nothing);
        assert_eq!(workspace.key(Key::Text("B.TXT".into())), Effect::Nothing);
        assert_eq!(workspace.visible_rows(Pane::Files), &[1]);
        assert_eq!(workspace.key(Key::Down), Effect::Nothing);
        let Job::Main { request, .. } = sent(workspace.key(Key::Enter)) else { panic!(); };
        assert!(matches!(request, MainRequest::File { path, staged_view: true, .. } if path == NativePath::new("b.txt")));
        assert_eq!(workspace.key(Key::Space), Effect::Send(Job::SetFile { path: NativePath::new("b.txt"), stage: false }));
        assert_eq!(workspace.key(Key::Char('a')), Effect::Nothing);
        assert_eq!(workspace.key(Key::Char('s')), Effect::Nothing);
        workspace.request_filter(Pane::Files);
        workspace.key(Key::SelectAll);
        assert_eq!(workspace.ime_commit("; c q [*.rs] 中文".into()), Effect::Nothing);
        assert!(workspace.visible_rows(Pane::Files).is_empty());
        workspace.key(Key::Enter);
        assert_eq!(workspace.key(Key::Space), Effect::Nothing);
        assert_eq!(workspace.key(Key::Char('d')), Effect::Nothing);
        assert_eq!(workspace.clear_filter(Pane::Files), Effect::Nothing);
        assert_eq!(workspace.files, 2, "Clear returns to the raw pre-filter identity");
        workspace.request_filter(Pane::Branches);
        workspace.key(Key::Text("MAIN".into()));
        assert_eq!(workspace.visible_rows(Pane::Branches), &[1]);
        workspace.key(Key::Enter);
        assert_eq!(workspace.key(Key::Space), Effect::Nothing, "current branch cannot be checked out again");
        workspace.request_filter(Pane::Commits);
        workspace.key(Key::Text("subject 2".into()));
        assert_eq!(workspace.visible_rows(Pane::Commits), &[1]);
        workspace.key(Key::SelectAll);
        workspace.key(Key::Text("1".into()));
        assert_eq!(workspace.visible_rows(Pane::Commits), &[0], "OID prefixes bind full revision identity");
    }

    #[test]
    fn quick_filter_cancel_refresh_limits_and_navigation_preserve_their_own_context() {
        let mut workspace = Workspace::new(snapshot(None));
        let Job::Main { generation, .. } = sent(workspace.open()) else { panic!(); };
        workspace.reply(Reply::Main { generation, view: MainView::text("original view") });
        workspace.request_filter(Pane::Files);
        workspace.key(Key::Text("b".into()));
        assert_eq!(workspace.key(Key::Escape), Effect::Nothing);
        assert_eq!(workspace.filter_query(Pane::Files), "");
        assert_eq!(workspace.main.lines[0].text, "original view");
        workspace.request_filter(Pane::Files);
        workspace.key(Key::Text("b".into()));
        workspace.key(Key::Enter);
        let nav = crate::Navigation::decode(&workspace.navigation().encode()).unwrap();
        let mut restored = snapshot(None);
        restored.navigation = Some(nav);
        let mut second = Workspace::new(restored);
        second.open();
        assert_eq!(second.filter_query(Pane::Files), "b");
        assert_eq!(second.visible_rows(Pane::Files), &[1]);
        second.clear_filter(Pane::Files);
        assert_eq!(second.files, 0, "the pre-filter anchor survives a navigation round trip");
        second.request_filter(Pane::Files);
        second.key(Key::Text("中".repeat(256)));
        second.key(Key::Text("x".into()));
        assert_eq!(second.filter_editor().unwrap().editor.text.chars().count(), 256);
        assert!(second.notice().unwrap().contains("256"));
        second.key(Key::SelectAll);
        second.ime_preedit("a".into());
        assert_eq!(second.key(Key::Enter), Effect::Nothing);
        second.ime_commit("a".into());
        assert_eq!(second.filter_editor().unwrap().editor.text, "a");
        let mut changed = snapshot(None);
        changed.files.remove(0);
        assert_eq!(second.reply(Reply::Full(changed)), Effect::Nothing, "refresh during editing cannot request details");
        assert!(second.visible_rows(Pane::Files).is_empty());
        second.key(Key::Escape);
        assert!(second.main.file.is_none(), "cancel after refresh cannot revive an obsolete actionable Diff");
        let encoded = workspace.navigation().encode();
        let mut legacy: Vec<_> = encoded.split('\n').take(11).collect();
        legacy[0] = "1";
        assert!(crate::Navigation::decode(&legacy.join("\n")).unwrap().queries.iter().all(String::is_empty));
    }

    #[test]
    fn commit_review_is_required_and_old_refresh_or_diff_replies_cannot_retarget_it() {
        let mut workspace = Workspace::new(snapshot(None));
        let Effect::Send(Job::LoadCommitReview { generation: first }) = workspace.request_commit() else { panic!("review must be requested"); };
        workspace.key(Key::Text("message".into()));
        assert_eq!(workspace.key(Key::CtrlEnter), Effect::Nothing);
        let Effect::Send(Job::LoadCommitReview { generation: fresh }) = workspace.refresh_commit_review() else { panic!("refresh must be explicit"); };
        workspace.reply(Reply::CommitReview { generation: first, review: crate::model::CommitReview { ticket: "obsolete".into(), head: "old".into(), files: vec![], blocked: None } });
        assert!(workspace.commit_composer().unwrap().review.is_none());
        assert!(fresh > first);
        ready_commit(&mut workspace);
        assert_eq!(workspace.prompt().unwrap().text(), "message");
        let current = workspace.commit_composer().unwrap().diff_generation;
        workspace.reply(Reply::CommitDiff { generation: current - 1, view: MainView::note("obsolete staged content") });
        assert!(!workspace.commit_composer().unwrap().diff.lines.iter().any(|line| line.text.contains("obsolete staged content")));
        assert!(matches!(workspace.key(Key::CtrlEnter), Effect::Send(Job::Commit { ticket, message }) if ticket == "test-review" && message == "message"));
    }

    #[test]
    fn continuous_message_typing_schedules_a_checkpoint_and_close_waits_for_exact_ack() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.request_commit();
        workspace.key(Key::Text("first".into()));
        let deadline = workspace.draft_deadline().unwrap();
        workspace.key(Key::Text(" text".into()));
        assert_eq!(workspace.draft_deadline(), Some(deadline), "continuous typing must not postpone the checkpoint");
        assert_eq!(workspace.checkpoint_draft(), Effect::Send(Job::SaveDraft("first text".into())));
        workspace.key(Key::Text(" latest".into()));
        assert_eq!(workspace.close_commit(true), Effect::Nothing);
        assert_eq!(workspace.reply(Reply::DraftSaved(Some("first text".into()))), Effect::Send(Job::SaveDraft("first text latest".into())));
        assert!(workspace.commit_composer().is_some(), "an old acknowledgement cannot close a newer edit");
        assert_eq!(workspace.reply(Reply::DraftSaved(Some("first text latest".into()))), Effect::Quit);
        assert!(workspace.commit_composer().is_none());
        assert_eq!(workspace.snapshot.draft.as_deref(), Some("first text latest"));
    }

    #[test]
    fn commit_editor_reuses_unicode_selection_and_keeps_failed_saves_copyable() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.request_commit();
        workspace.key(Key::Text("hello 中文".into()));
        workspace.key(Key::Home);
        workspace.key(Key::Text("prefix ".into()));
        assert_eq!(workspace.prompt().unwrap().text(), "prefix hello 中文");
        workspace.key(Key::SelectAll);
        assert!(matches!(workspace.key(Key::CtrlC), Effect::Copy { cut: false, text } if text == "prefix hello 中文"));
        workspace.key(Key::Text("replacement".into()));
        let id = workspace.commit_composer().unwrap().id;
        workspace.complete_clipboard(id, None, Some("prefix hello 中文".into()));
        assert_eq!(workspace.prompt().unwrap().text(), "replacement", "a late cut cannot delete a changed selection");
        workspace.key(Key::Escape);
        workspace.reply(Reply::PromptFailed { action: PromptAction::SaveDraft, text: "disk unavailable".into() });
        assert!(workspace.draft_deadline().is_none(), "failed I/O is not retried in a timer loop");
        workspace.key(Key::SelectAll);
        assert!(matches!(workspace.key(Key::CtrlC), Effect::Copy { text, .. } if text == "replacement"));
    }
    #[test]
    fn commit_ime_does_not_submit_or_route_literal_text_as_a_command() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.request_commit();
        ready_commit(&mut workspace);
        workspace.ime_preedit("中文".into());
        assert_eq!(workspace.key(Key::CtrlEnter), Effect::Nothing);
        assert_eq!(workspace.key(Key::Enter), Effect::Nothing);
        assert_eq!(workspace.prompt().unwrap().text(), "");
        workspace.ime_commit("c 中文\nsecond line".into());
        assert_eq!(workspace.prompt().unwrap().text(), "c 中文\nsecond line");
        assert!(workspace.commit_composer().unwrap().editor.preedit.is_empty());
        let Effect::Send(Job::Commit { message, .. }) = workspace.key(Key::CtrlEnter) else { panic!("explicit submit after composition must work"); };
        assert_eq!(message, "c 中文\nsecond line");
        assert_eq!(workspace.key(Key::CtrlEnter), Effect::Nothing, "held submit cannot duplicate a write");
    }

    #[test]
    fn identity_fields_preserve_refused_edits_and_require_explicit_save_after_reload() {
        let mut workspace = Workspace::new(snapshot(None));
        assert_eq!(workspace.key(Key::Char('I')), Effect::Send(Job::LoadIdentity));
        assert_eq!(workspace.key(Key::Escape), Effect::Send(Job::CancelEntry), "a pending read can be dismissed");
        workspace.reply(Reply::IdentitySettings { ticket: "late".into(), name: "Late".into(), email: "late@example.invalid".into(), detail: "late source".into() });
        assert!(workspace.identity_form().is_none(), "a late read cannot reopen the dismissed form");
        assert_eq!(workspace.key(Key::Char('I')), Effect::Send(Job::LoadIdentity));
        workspace.reply(Reply::IdentitySettings { ticket: "first".into(), name: "Old".into(), email: "old@example.invalid".into(), detail: "local source".into() });
        for _ in 0..3 { workspace.key(Key::Backspace); }
        workspace.key(Key::Text("New Name 中文".into()));
        assert_eq!(workspace.key(Key::Enter), Effect::Send(Job::SaveIdentity { ticket: "first".into(), name: "New Name 中文".into(), email: "old@example.invalid".into() }));
        assert_eq!(workspace.key(Key::Escape), Effect::Nothing, "cannot claim cancellation after write submission");
        workspace.reply(Reply::PromptFailed { action: PromptAction::IdentitySettings, text: "config changed".into() });
        assert_eq!(workspace.key(Key::Enter), Effect::Send(Job::LoadIdentity));
        workspace.reply(Reply::IdentitySettings { ticket: "fresh".into(), name: "External".into(), email: "external@example.invalid".into(), detail: "new source".into() });
        assert_eq!(workspace.identity_form().unwrap().name, "New Name 中文");
        assert_eq!(workspace.identity_form().unwrap().email, "old@example.invalid");
        assert_eq!(workspace.key(Key::Enter), Effect::Send(Job::SaveIdentity { ticket: "fresh".into(), name: "New Name 中文".into(), email: "old@example.invalid".into() }));
        workspace.reply(Reply::IdentitySaved { path: NativePath::new("repo"), review: Some(crate::model::TrustReview { ticket: "trust".into(), worktree: NativePath::new("repo"), git_directory: NativePath::new("metadata"), fingerprint: "digest".into(), submitted: false }), detail: "Identity saved".into() });
        assert!(workspace.identity_form().is_none());
        assert_eq!(workspace.key(Key::Enter), Effect::Nothing, "identity Save must not approve changed configuration trust");
        assert_eq!(workspace.key(Key::Escape), Effect::Send(Job::CancelEntry));
    }
    #[test]
    fn git_settings_preserve_failed_input_and_do_not_turn_text_into_repo_shortcuts() {
        let mut workspace = Workspace::new(Snapshot::welcome());
        assert_eq!(workspace.key(Key::Char('G')), Effect::Send(Job::LoadGitSettings));
        workspace.reply(Reply::GitSettings { path: Some(NativePath::new("old Git")), detail: "Saved preference".into() });
        for _ in 0..7 { workspace.key(Key::Backspace); }
        assert_eq!(workspace.key(Key::Text("new git 中文".into())), Effect::Nothing);
        assert_eq!(workspace.key(Key::Enter), Effect::Send(Job::ConfigureGit(Some(NativePath::new("new git 中文")))));
        assert_eq!(workspace.key(Key::Enter), Effect::Nothing);
        workspace.reply(Reply::PromptFailed { action: PromptAction::GitSettings, text: "not saved".into() });
        assert_eq!(workspace.prompt().unwrap().text(), "new git 中文");
        assert_eq!(workspace.key(Key::Escape), Effect::Nothing);
        assert!(workspace.prompt().is_none());
        assert!(workspace.is_welcome());
        assert!(workspace.git_detail().contains("not saved or applied"));
    }
    #[test]
    fn create_repository_fields_review_and_failure_preserve_explicit_input() {
        let mut workspace = Workspace::new(Snapshot::welcome());
        workspace.key(Key::Char('N'));
        assert!(workspace.modal());
        workspace.key(Key::Text("new project 中文".into()));
        workspace.key(Key::Tab);
        for _ in 0..4 { workspace.key(Key::Backspace); }
        workspace.key(Key::Text("feature/start".into()));
        assert_eq!(workspace.key(Key::Enter), Effect::Send(Job::PrepareCreate { path: NativePath::new("new project 中文"), branch: "feature/start".into() }));
        workspace.reply(Reply::CreateReview { ticket: "create-once".into(), path: NativePath::new("canonical target"), branch: "feature/start".into(), new_directory: false, entries: vec!["keep.bin".into()], more_entries: false });
        assert_eq!(workspace.key(Key::Enter), Effect::Nothing, "held Enter must not initialize");
        assert_eq!(workspace.key(Key::Escape), Effect::Send(Job::CancelEntry));
        assert_eq!(workspace.create_form().unwrap().path, "new project 中文");
        assert_eq!(workspace.key(Key::Enter), Effect::Send(Job::PrepareCreate { path: NativePath::new("new project 中文"), branch: "feature/start".into() }));
        workspace.reply(Reply::CreateReview { ticket: "new-ticket".into(), path: NativePath::new("canonical target"), branch: "feature/start".into(), new_directory: false, entries: vec![], more_entries: false });
        assert_eq!(workspace.key(Key::Char('y')), Effect::Send(Job::ExecuteCreate("new-ticket".into())));
        assert_eq!(workspace.key(Key::Char('y')), Effect::Nothing);
        assert_eq!(workspace.key(Key::Escape), Effect::Nothing, "cannot claim cancellation after submission");
        workspace.reply(Reply::PromptFailed { action: PromptAction::CreateRepository, text: "Git missing".into() });
        assert_eq!(workspace.create_form().unwrap().branch, "feature/start");
        assert!(workspace.create_form().unwrap().review.is_none());
        assert!(!workspace.create_form().unwrap().submitted);
    }

    #[test]
    fn create_repository_success_stops_at_separate_trust_and_cancel_keeps_new_path() {
        let mut workspace = Workspace::new(Snapshot::welcome());
        workspace.request_create();
        workspace.key(Key::Text("chosen target".into()));
        workspace.key(Key::Enter);
        workspace.reply(Reply::CreateReview { ticket: "create".into(), path: NativePath::new("chosen target"), branch: "main".into(), new_directory: true, entries: vec![], more_entries: false });
        workspace.answer_create(true);
        workspace.reply(Reply::TrustReview(crate::model::TrustReview { ticket: "trust".into(), worktree: NativePath::new("canonical chosen target"), git_directory: NativePath::new("metadata"), fingerprint: "digest".into(), submitted: false }));
        assert!(workspace.create_form().is_none());
        assert_eq!(workspace.key(Key::Enter), Effect::Nothing);
        assert_eq!(workspace.key(Key::Escape), Effect::Send(Job::CancelEntry));
        assert_eq!(workspace.prompt().unwrap().text(), "canonical chosen target");
        assert!(workspace.is_welcome());
    }
    use srcmgr_domain::NativePath;

    use super::*;
    use crate::model::{BranchRow, CommitRow, FileRow, FileTarget, Tone};

    fn file(path: &str, has_staged: bool, has_unstaged: bool) -> FileRow {
        FileRow {
            path: NativePath::new(path),
            label: path.to_owned(),
            tone: Tone::Changed,
            has_staged,
            has_unstaged,
            untracked: false,
        }
    }

    fn branch(name: &str, current: bool) -> BranchRow {
        BranchRow { name: name.to_owned(), label: name.to_owned(), current }
    }

    fn commit(id: &str) -> CommitRow {
        CommitRow { id: id.to_owned(), subject: format!("subject {id}"), merge: false }
    }

    fn snapshot(draft: Option<&str>) -> Snapshot {
        Snapshot {
            history_limit: 200,
            navigation: None,
            repository: Some(NativePath::new("fixture-repository")),
            recents: Vec::new(),
            title: "t".to_owned(),
            head: "On branch main (0123456789ab)".to_owned(),
            files: vec![file("a.txt", false, true), file("b.txt", true, false), file("c.txt", true, true)],
            branches: vec![branch("feature", false), branch("main", true)],
            commits: vec![commit("1"), commit("2")],
            draft: draft.map(str::to_owned),
            stage_result: None,
            stashes: Vec::new(),
        }
    }

    fn sent(effect: Effect) -> Job {
        match effect {
            Effect::Send(job) => job,
            other => panic!("expected a job, got {other:?}"),
        }
    }

    #[test]
    fn repository_tabs_restore_raw_selection_scroll_and_wait_for_latest_draft() {
        let mut workspace = Workspace::new(snapshot(Some("left draft")));
        workspace.view_rows = 6;
        let Job::Main { generation, .. } = sent(workspace.click_row(Pane::Files, 2)) else { panic!() };
        let view = MainView::diff(&(0..80).map(|n| format!("+line {n}\n")).collect::<String>(), crate::FileTarget { path: NativePath::new("c.txt"), staged_view: false });
        workspace.reply(Reply::Main { generation, view: view.clone() });
        workspace.click_main(Some(50));
        let saved = workspace.navigation();
        let saved = crate::Navigation::decode(&saved.encode()).unwrap();
        assert_eq!(saved.file, Some(NativePath::new("c.txt")));
        assert!(saved.scroll > 0);
        workspace.request_open();
        workspace.key(Key::Text("right".into()));
        workspace.key(Key::Enter);
        let mut right = snapshot(None);
        right.repository = Some(NativePath::new("right"));
        workspace.reply(Reply::RepositoryOpened(right.clone()));
        assert_eq!(workspace.tabs.len(), 2);
        workspace.request_commit();
        workspace.key(Key::Text("right 中文 draft".into()));
        assert_eq!(workspace.select_repository(0), Effect::Send(Job::SaveDraft("right 中文 draft".into())));
        assert_eq!(workspace.snapshot.repository, right.repository);
        let Effect::Send(Job::EnterRepository { path: Some(target), current: Some((source, _)) }) = workspace.reply(Reply::DraftSaved(Some("right 中文 draft".into()))) else { panic!("switch must wait for acknowledged draft"); };
        assert_eq!(source, NativePath::new("right"));
        assert_eq!(target, NativePath::new("fixture-repository"));
        let mut left = snapshot(Some("left draft"));
        left.navigation = Some(saved.clone());
        let Job::Main { generation: fresh, .. } = sent(workspace.reply(Reply::RepositoryOpened(left))) else { panic!() };
        workspace.view_rows = 6;
        workspace.reply(Reply::Main { generation, view: MainView::text("late right view") });
        workspace.reply(Reply::Main { generation: fresh, view });
        assert_eq!(workspace.selected(Pane::Files), 2);
        assert_eq!((workspace.cursor, workspace.scroll), (saved.cursor, saved.scroll));
        assert_eq!(workspace.snapshot.draft.as_deref(), Some("left draft"));
        assert!(matches!(workspace.close_repository(), Effect::Send(Job::EnterRepository { path: Some(p), .. }) if p == NativePath::new("right")));
        workspace.reply(Reply::RepositoryOpened(right));
        assert_eq!(workspace.tabs.len(), 1);
        assert!(matches!(workspace.close_repository(), Effect::Send(Job::EnterRepository { path: None, .. })));
        workspace.reply(Reply::RepositoryClosed(Vec::new()));
        assert!(workspace.is_welcome() && workspace.tabs.is_empty());
    }

    #[test]
    fn repository_tab_save_failure_preserves_editor_and_target_until_explicit_retry() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.tabs.push(RepositoryTab { path: NativePath::new("other"), head: "unread".into() });
        workspace.request_commit();
        workspace.key(Key::Text("cannot lose this".into()));
        assert!(matches!(workspace.select_repository(1), Effect::Send(Job::SaveDraft(_))));
        workspace.reply(Reply::PromptFailed { action: PromptAction::SaveDraft, text: "disk unavailable".into() });
        assert_eq!(workspace.snapshot.repository, Some(NativePath::new("fixture-repository")));
        assert_eq!(workspace.prompt().unwrap().text(), "cannot lose this");
        assert!(workspace.draft_deadline().is_none());
        assert!(matches!(workspace.key(Key::Save), Effect::Send(Job::SaveDraft(_))));
        assert!(matches!(workspace.reply(Reply::DraftSaved(Some("cannot lose this".into()))), Effect::Send(Job::EnterRepository { path: Some(p), .. }) if p == NativePath::new("other")));
    }

    #[test]
    fn open_repository_keeps_input_on_refusal_and_rejects_old_detail_after_switch() {
        let mut workspace = Workspace::new(snapshot(Some("original durable draft")));
        let _ = workspace.open();
        let old_generation = workspace.generation;
        workspace.key(Key::CtrlO);
        workspace.key(Key::Text("D:/other repo".to_owned()));
        assert!(matches!(sent(workspace.key(Key::Enter)), Job::EnterRepository { path: Some(path), current: Some((source, _)) } if path == NativePath::new("D:/other repo") && source == NativePath::new("fixture-repository")));
        workspace.reply(Reply::PromptFailed { action: PromptAction::OpenRepository, text: "not trusted".into() });
        assert_eq!(workspace.prompt().unwrap().text(), "D:/other repo");
        assert_eq!(workspace.snapshot().draft.as_deref(), Some("original durable draft"));
        assert!(matches!(sent(workspace.key(Key::Enter)), Job::EnterRepository { path: Some(path), current: Some((source, _)) } if path == NativePath::new("D:/other repo") && source == NativePath::new("fixture-repository")));
        let mut other = snapshot(Some("other durable draft"));
        other.files = vec![file("other.txt", false, true)];
        other.title = "other repository".into();
        assert!(matches!(workspace.reply(Reply::RepositoryOpened(other)), Effect::Send(Job::Main { .. })));
        workspace.reply(Reply::Main { generation: old_generation, view: MainView::text("old repository detail") });
        assert_eq!(workspace.snapshot().title, "other repository");
        assert_eq!(workspace.snapshot().draft.as_deref(), Some("other durable draft"));
        assert!(workspace.main().lines.iter().all(|line| !line.text.contains("old repository detail")));
        assert!(workspace.prompt().is_none());
    }

    #[test]
    fn welcome_never_dispatches_repository_writes_and_recent_identity_is_not_its_label() {
        let mut workspace = Workspace::new(Snapshot::welcome());
        assert_eq!(workspace.open(), Effect::Send(Job::LoadWelcome));
        for key in [Key::Space, Key::Char('a'), Key::Char('c'), Key::Char('s')] {
            assert_eq!(workspace.key(key), Effect::Nothing);
        }
        let raw = NativePath::new("actual repository");
        workspace.reply(Reply::Welcome(vec![crate::model::RecentRow { backend: "git".into(), path: raw.encoded(), label: "unrelated display label".into() }]));
        assert_eq!(workspace.key(Key::Enter), Effect::Send(Job::EnterRepository { path: Some(raw.clone()), current: None }));
        let review = crate::model::TrustReview { ticket: "one-use".into(), worktree: raw, git_directory: NativePath::new("metadata"), fingerprint: "digest".into(), submitted: false };
        workspace.reply(Reply::TrustReview(review));
        assert_eq!(workspace.key(Key::Enter), Effect::Nothing, "opening Enter cannot approve trust");
        assert_eq!(workspace.key(Key::Escape), Effect::Send(Job::CancelEntry));
        assert!(workspace.is_welcome());
        assert_eq!(workspace.prompt().unwrap().text(), "actual repository");
        assert_eq!(workspace.key(Key::Escape), Effect::Nothing);
        assert!(workspace.prompt().is_none());
    }

    #[test]
    fn trust_submission_is_one_action_and_old_welcome_reply_cannot_replace_a_repository() {
        let mut workspace = Workspace::new(Snapshot::welcome());
        workspace.request_open();
        workspace.key(Key::Text("chosen".into()));
        workspace.key(Key::Enter);
        workspace.reply(Reply::TrustReview(crate::model::TrustReview { ticket: "token".into(), worktree: NativePath::new("chosen"), git_directory: NativePath::new("metadata"), fingerprint: "digest".into(), submitted: false }));
        assert_eq!(workspace.answer_trust(true), Effect::Send(Job::GrantRepositoryTrust("token".into())));
        assert_eq!(workspace.answer_trust(true), Effect::Nothing);
        assert_eq!(workspace.answer_trust(false), Effect::Nothing, "cannot pretend to cancel an executing grant");
        workspace.reply(Reply::RepositoryOpened(snapshot(None)));
        workspace.reply(Reply::Welcome(Vec::new()));
        assert!(!workspace.is_welcome());
        assert!(!workspace.snapshot().files.is_empty());
        assert!(workspace.trust_review().is_none());
    }

    fn approve(workspace: &mut Workspace) -> Job {
        workspace.reply(Reply::Prepared { generation: workspace.generation, ticket: "fixture".to_owned(), lines: vec!["Recovery with tested limits".to_owned()] });
        sent(workspace.key(Key::Char('y')))
    }

    fn text(value: &str) -> Key {
        Key::Text(value.to_owned())
    }

    /// Two hunks: lines 0 and 3 are the `@@` headers, 1-2 and 4-5 the changes.
    fn file_diff(path: &str, staged_view: bool) -> MainView {
        MainView::diff(
            "@@ -1 +1 @@\n-old\n+new\n@@ -5 +5 @@\n-x\n+y\n",
            FileTarget { path: NativePath::new(path), staged_view },
        )
    }

    fn lines_of(count: usize) -> Vec<Line> {
        (0..count).map(|index| Line { tone: Tone::Text, text: index.to_string() }).collect()
    }

    #[test]
    fn opening_asks_for_the_first_file_diff_and_loads_it_into_the_main_panel() {
        let mut workspace = Workspace::new(snapshot(None));
        let job = sent(workspace.open());
        let Job::Main { generation, request } = job else { panic!("main job expected") };
        assert_eq!(request, MainRequest::File { path: NativePath::new("a.txt"), staged_view: false, untracked: false });
        assert_eq!(workspace.reply(Reply::Main { generation, view: file_diff("a.txt", false) }), Effect::Nothing);
        assert_eq!(workspace.main().hunks.len(), 2);
        assert_eq!(workspace.cursor(), 1, "the cursor starts on the first changed line");
    }

    #[test]
    fn space_on_a_file_stages_it_when_it_has_unstaged_changes_and_unstages_it_otherwise() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        assert_eq!(sent(workspace.key(Key::Space)), Job::SetFile { path: NativePath::new("a.txt"), stage: true });
        workspace.key(Key::Down);
        assert_eq!(sent(workspace.key(Key::Space)), Job::SetFile { path: NativePath::new("b.txt"), stage: false });
    }

    #[test]
    fn moving_the_selection_clamps_and_requests_the_newly_selected_diff() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Up);
        assert_eq!(workspace.selected(Pane::Files), 0);
        workspace.key(Key::Down);
        workspace.key(Key::Down);
        workspace.key(Key::Down);
        assert_eq!(workspace.selected(Pane::Files), 2);
        assert_eq!(workspace.key(Key::Down), Effect::Nothing, "a clamped move asks for nothing");
        let job = sent(workspace.key(Key::Up));
        assert_eq!(
            job,
            Job::Main {
                generation: workspace.generation,
                request: MainRequest::File { path: NativePath::new("b.txt"), staged_view: true, untracked: false },
            }
        );
    }

    #[test]
    fn a_late_main_reply_from_an_earlier_selection_is_dropped() {
        let mut workspace = Workspace::new(snapshot(None));
        let Job::Main { generation: first, .. } = sent(workspace.open()) else { panic!() };
        workspace.key(Key::Down);
        assert_eq!(
            workspace.reply(Reply::Main { generation: first, view: file_diff("a.txt", false) }),
            Effect::Nothing
        );
        assert!(workspace.main().hunks.is_empty(), "stale diff must not replace the loading note");
    }

    #[test]
    fn number_keys_follow_lazygit_and_tab_moves_focus_and_source() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('3'));
        assert_eq!((workspace.focus(), workspace.source()), (Pane::Branches, Pane::Branches));
        workspace.key(Key::Tab);
        workspace.key(Key::Tab);
        assert_eq!(workspace.focus(), Pane::Main);
        assert_eq!(workspace.source(), Pane::Commits, "the main panel keeps the list it shows");
        workspace.key(Key::BackTab);
        assert_eq!(workspace.focus(), Pane::Commits);
        workspace.key(Key::Char('2'));
        workspace.key(Key::Enter);
        workspace.key(Key::Escape);
        assert_eq!(workspace.focus(), Pane::Files, "escape returns to the list that feeds the main panel");
        workspace.key(Key::Char('1'));
        assert_eq!((workspace.focus(), workspace.source()), (Pane::Status, Pane::Files));
    }

    #[test]
    fn space_stages_the_cursor_line_or_a_range_and_a_stages_the_hunk() {
        let mut workspace = Workspace::new(snapshot(None));
        let Job::Main { generation, .. } = sent(workspace.open()) else { panic!() };
        workspace.reply(Reply::Main { generation, view: file_diff("a.txt", false) });
        workspace.key(Key::Enter);
        assert_eq!(
            sent(workspace.key(Key::Space)),
            Job::SetLines { path: NativePath::new("a.txt"), staged_view: false, number: 1, lines: vec![1] }
        );
        workspace.key(Key::Down);
        workspace.key(Key::Char('v'));
        workspace.key(Key::Up);
        assert_eq!(workspace.selection(), (1, 2));
        assert_eq!(
            sent(workspace.key(Key::Space)),
            Job::SetLines { path: NativePath::new("a.txt"), staged_view: false, number: 1, lines: vec![1, 2] }
        );
        workspace.key(Key::Right);
        assert_eq!(workspace.hunk(), 1);
        assert_eq!(
            sent(workspace.key(Key::Char('a'))),
            Job::SetHunk { path: NativePath::new("a.txt"), staged_view: false, number: 2 }
        );
    }

    #[test]
    fn space_on_a_hunk_header_asks_for_a_changed_line() {
        let mut workspace = Workspace::new(snapshot(None));
        let Job::Main { generation, .. } = sent(workspace.open()) else { panic!() };
        workspace.reply(Reply::Main { generation, view: file_diff("a.txt", false) });
        workspace.key(Key::Enter);
        workspace.key(Key::Right);
        assert_eq!(workspace.key(Key::Space), Effect::Nothing);
        assert_eq!(workspace.notice(), Some("select a changed line in the diff"));
    }

    #[test]
    fn changes_named_actions_keep_raw_filter_target_diff_side_and_fresh_ignore_scope() {
        let mut initial=snapshot(None);initial.files[0].has_staged=true;
        let mut workspace=Workspace::new(initial);
        workspace.open();
        assert!(!workspace.change_available(ChangesAction::Hunk),"loading is not actionable Diff content");
        let Job::Main {generation,..}=sent(workspace.change(ChangesAction::Staged)) else{panic!()};
        workspace.reply(Reply::Main {generation,view:file_diff("a.txt",true)});
        assert_eq!(sent(workspace.change(ChangesAction::Lines)),Job::SetLines {path:NativePath::new("a.txt"),staged_view:true,number:1,lines:vec![1]});
        assert!(!workspace.change_available(ChangesAction::DiscardHunk));
        assert_eq!(sent(workspace.change(ChangesAction::UnstageFile)),Job::SetFile {path:NativePath::new("a.txt"),stage:false});
        workspace.request_filter(Pane::Files);workspace.key(Key::Text("B.TXT".into()));workspace.key(Key::Enter);
        let Job::Prepare {generation:project,action}=sent(workspace.change(ChangesAction::Ignore)) else{panic!()};
        assert_eq!(*action,Job::Ignore {path:NativePath::new("b.txt"),local:false});
        assert_eq!(workspace.answer_confirmation(true),Effect::Nothing,"approval must wait for preparation");
        let Job::Prepare {generation:local,action}=sent(workspace.set_ignore_scope(true)) else{panic!()};
        assert_eq!(*action,Job::Ignore {path:NativePath::new("b.txt"),local:true});
        workspace.reply(Reply::Prepared {generation:project,ticket:"old project".into(),lines:vec!["obsolete".into()]});
        assert!(!workspace.confirmation_ready(),"late project result cannot approve local scope");
        workspace.reply(Reply::Prepared {generation:local,ticket:"local ticket".into(),lines:vec!["reviewed local scope".into()]});
        assert_eq!(workspace.key(Key::Enter),Effect::Nothing,"Enter cannot save a rule");
        assert!(workspace.confirmation_ready());
        assert_eq!(workspace.answer_confirmation(true),Effect::Send(Job::ExecutePrepared("local ticket".into())));
        workspace.request_filter(Pane::Files);workspace.key(Key::SelectAll);workspace.key(Key::Text("no match".into()));workspace.key(Key::Enter);
        assert!(workspace.visible_rows(Pane::Files).is_empty());
        for action in [ChangesAction::StageFile,ChangesAction::UnstageFile,ChangesAction::Ignore,ChangesAction::Hunk,ChangesAction::Lines]{assert!(!workspace.change_available(action));assert_eq!(workspace.change(action),Effect::Nothing);}
    }

    #[test]
    fn a_on_files_stages_everything_or_unstages_everything() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        assert_eq!(sent(workspace.key(Key::Char('a'))), Job::SetAll { stage: true });
        let mut staged = snapshot(None);
        staged.files = vec![file("b.txt", true, false)];
        let mut workspace = Workspace::new(staged);
        workspace.open();
        assert_eq!(sent(workspace.key(Key::Char('a'))), Job::SetAll { stage: false });
        let mut clean = snapshot(None);
        clean.files.clear();
        let mut workspace = Workspace::new(clean);
        workspace.open();
        assert_eq!(workspace.key(Key::Char('a')), Effect::Nothing);
        assert_eq!(workspace.notice(), Some("nothing to stage or unstage"));
    }

    #[test]
    fn checking_out_the_current_branch_is_refused_and_another_branch_switches() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('3'));
        workspace.key(Key::Down);
        assert_eq!(workspace.key(Key::Space), Effect::Nothing);
        assert_eq!(workspace.notice(), Some("main is already checked out"));
        workspace.key(Key::Up);
        assert_eq!(sent(workspace.key(Key::Space)), Job::Switch("feature".to_owned()));
    }

    #[test]
    fn d_in_files_asks_before_discarding_and_only_y_discards() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('d'));
        assert_eq!(
            workspace.confirm(),
            Some(&Confirm::DiscardFile { path: NativePath::new("a.txt"), untracked: false })
        );
        workspace.key(Key::Char('n'));
        assert_eq!(workspace.confirm(), None, "n keeps the changes");
        workspace.key(Key::Char('d'));
        assert_eq!(
            approve(&mut workspace),
            Job::ExecutePrepared("fixture".to_owned())
        );
    }

    #[test]
    fn recovery_preparation_cannot_confirm_early_or_after_cancel() {
        let mut workspace = Workspace::new(snapshot(None));
        let job = sent(workspace.key(Key::Char('d')));
        let Job::Prepare { generation, action } = job else { panic!("preparation required") };
        assert_eq!(*action, Job::Discard { path: NativePath::new("a.txt"), untracked: false });
        assert_eq!(workspace.key(Key::Char('y')), Effect::Nothing);
        assert!(workspace.confirm().is_some());
        workspace.key(Key::Escape);
        workspace.reply(Reply::Prepared { generation, ticket: "late".to_owned(), lines: vec!["stale record".to_owned()] });
        assert!(workspace.confirm().is_none());
        assert!(workspace.prepared_ticket.is_none());
        workspace.key(Key::Char('d'));
        workspace.reply(Reply::ConfirmationFailed { generation, text: "old failure".to_owned() });
        workspace.reply(Reply::MainFailed { generation, text: "old read failure".to_owned() });
        assert!(workspace.confirm().is_some(), "a late failure cannot close a newer preview");
        assert_ne!(workspace.notice(), Some("old failure"));
    }

    #[test]
    fn d_on_a_file_with_only_staged_changes_says_there_is_nothing_to_discard() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Down);
        assert_eq!(workspace.key(Key::Char('d')), Effect::Nothing);
        assert_eq!(workspace.confirm(), None);
        assert_eq!(workspace.notice(), Some("b.txt has no unstaged changes to discard"));
    }

    #[test]
    fn d_in_the_main_panel_asks_about_the_hunk_under_the_cursor() {
        let mut workspace = Workspace::new(snapshot(None));
        let Job::Main { generation, .. } = sent(workspace.open()) else { panic!() };
        workspace.reply(Reply::Main {
            generation,
            view: MainView {
                lines: lines_of(12),
                hunks: vec![0],
                file: Some(FileTarget { path: NativePath::new("a.txt"), staged_view: false }),
                committed: None,
            },
        });
        workspace.key(Key::Enter);
        workspace.key(Key::Char('d'));
        assert_eq!(workspace.confirm(), Some(&Confirm::DiscardHunk { path: NativePath::new("a.txt"), index: 0 }));
        assert_eq!(
            approve(&mut workspace),
            Job::ExecutePrepared("fixture".to_owned())
        );
    }

    #[test]
    fn s_stashes_tracked_changes_and_refuses_a_list_of_only_untracked_files() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        assert_eq!(sent(workspace.key(Key::Char('s'))), Job::Stash);
        workspace.key(Key::Char('S'));
        assert_eq!(workspace.focus(), Pane::Stashes);
        workspace.key(Key::Char('2'));
        let mut only_untracked = snapshot(None);
        only_untracked.files = vec![FileRow { untracked: true, ..file("u.txt", false, true) }];
        workspace.reply(Reply::Full(only_untracked));
        assert_eq!(workspace.key(Key::Char('s')), Effect::Nothing);
        assert_eq!(workspace.notice(), Some("no tracked changes to stash"));
    }

    #[test]
    fn selected_stash_dispatch_cancel_and_external_removal_preserve_identity() {
        let mut initial = snapshot(None);
        initial.stashes = vec![
            crate::StashRow { id: "a".repeat(40), subject: "newer".into() },
            crate::StashRow { id: "b".repeat(40), subject: "older".into() },
        ];
        let mut workspace = Workspace::new(initial.clone());
        workspace.open();
        workspace.key(Key::Char('5'));
        workspace.key(Key::Down);
        let Job::Prepare { action, .. } = sent(workspace.key(Key::Char('d'))) else { panic!() };
        assert_eq!(*action, Job::StashAction { id: "b".repeat(40), expected: vec!["a".repeat(40), "b".repeat(40)], operation: "drop".into() });
        assert_eq!(sent(workspace.key(Key::Escape)), Job::CancelPrepared);
        assert_eq!(workspace.selected(Pane::Stashes), 1);
        initial.stashes.reverse();
        workspace.reply(Reply::Full(initial.clone()));
        assert_eq!(workspace.selected(Pane::Stashes), 0, "reordering preserves the full selected OID");
        initial.stashes.remove(0);
        workspace.reply(Reply::Full(initial));
        assert!(workspace.notice().unwrap().contains("was removed"));
    }

    #[test]
    fn a_full_refresh_cancels_an_outstanding_confirmation_and_releases_its_worker_record() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        let Job::Prepare { generation, .. } = sent(workspace.key(Key::Char('d'))) else { panic!() };
        assert_eq!(sent(workspace.reply(Reply::Full(snapshot(None)))), Job::CancelPrepared);
        assert!(workspace.confirm().is_none());
        assert!(workspace.notice().unwrap().contains("confirmation cancelled"));
        workspace.reply(Reply::Prepared { generation, ticket: "obsolete".into(), lines: vec!["old recovery state".into()] });
        assert_eq!(workspace.key(Key::Char('y')), Effect::Nothing);
        assert!(workspace.confirm().is_none());
        assert!(!workspace.main().lines[0].text.contains("Loading"), "cancelled confirmation cannot leave a permanent loading view");
    }

    #[test]
    fn refresh_preserves_selected_identities_when_rows_are_inserted_before_them() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Down);
        workspace.key(Key::Char('3'));
        workspace.key(Key::Down);
        workspace.key(Key::Char('4'));
        workspace.key(Key::Down);
        let mut updated = snapshot(None);
        updated.files.insert(0, file("0-new.txt", true, false));
        updated.branches.insert(0, branch("aaa", false));
        updated.commits.insert(0, commit("new"));
        let Job::Main { request, .. } = sent(workspace.reply(Reply::Full(updated))) else { panic!() };
        assert_eq!(request, MainRequest::Commit { id: "2".into() });
        assert_eq!(workspace.selected(Pane::Files), 2);
        assert_eq!(workspace.selected(Pane::Branches), 2);
        assert_eq!(workspace.selected(Pane::Commits), 2);
    }

    #[test]
    fn failed_committed_detail_is_visible_and_returns_to_the_scrolled_file_list() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('4'));
        let Job::Main { generation, .. } = sent(workspace.key(Key::Enter)) else { panic!() };
        let files: Vec<_> = (0..30).map(|i| crate::CommittedFile { status: "A".into(), old_mode: "000000".into(), new_mode: "100644".into(), path: NativePath::new(format!("f{i}.txt")), old_path: None }).collect();
        let mut list = MainView::text(&format!("Commit 1\nBase: empty tree\n{}", (0..30).map(|i| format!("A f{i}.txt\n")).collect::<String>()));
        list.committed = Some(crate::CommittedFiles { id: "1".into(), base: "empty tree".into(), files });
        workspace.reply(Reply::Main { generation, view: list.clone() });
        workspace.set_view_rows(5);
        workspace.key(Key::End);
        let before = (workspace.cursor(), workspace.scroll());
        assert!(before.1 > 0);
        let Job::Main { generation, .. } = sent(workspace.key(Key::Enter)) else { panic!() };
        assert_eq!(workspace.scroll(), 0, "loading remains visible after drilling into a low row");
        workspace.reply(Reply::MainFailed { generation, text: "Commit 1\nBase: unavailable\nObject missing".into() });
        assert_eq!((workspace.cursor(), workspace.scroll()), (0, 0));
        assert_eq!(workspace.main().lines.len(), 3);
        let Job::Main { generation, request: MainRequest::CommitFiles { .. } } = sent(workspace.key(Key::Escape)) else { panic!("Back must reread the prior view"); };
        workspace.reply(Reply::Main { generation, view: list.clone() });
        assert_eq!(workspace.main(), &list);
        assert_eq!((workspace.cursor(), workspace.scroll()), before);
    }

    #[test]
    fn committed_detail_returns_to_list_and_cannot_mutate_or_accept_a_late_diff() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('4'));
        let Job::Main { generation, request: MainRequest::CommitFiles { id } } = sent(workspace.key(Key::Enter)) else { panic!() };
        assert_eq!(id, "1");
        let file = crate::CommittedFile { status: "D".into(), old_mode: "100644".into(), new_mode: "000000".into(), path: NativePath::new("gone.txt"), old_path: None };
        let mut list = MainView::text("Commit 1\nBase: empty tree\nD gone.txt\n");
        list.committed = Some(crate::CommittedFiles { id: id.clone(), base: "empty tree".into(), files: vec![file.clone()] });
        workspace.reply(Reply::Main { generation, view: list.clone() });
        assert_eq!(workspace.cursor(), 2);
        let Job::Main { generation: diff_generation, request: MainRequest::CommitFile { id: target, file: selected } } = sent(workspace.key(Key::Enter)) else { panic!() };
        assert_eq!((target, selected), (id, file));
        workspace.reply(Reply::Main { generation: diff_generation, view: MainView::text("@@ -1 +0,0 @@\n-deleted\n") });
        for key in [Key::Space, Key::Char('a'), Key::Char('d')] {
            assert_eq!(workspace.key(key), Effect::Nothing, "history is read-only");
            assert!(workspace.confirm().is_none());
        }
        let Job::Main { generation, request: MainRequest::CommitFiles { .. } } = sent(workspace.key(Key::Escape)) else { panic!("Back must reread the prior view"); };
        workspace.reply(Reply::Main { generation, view: list.clone() });
        assert_eq!(workspace.main(), &list);
        assert_eq!(workspace.cursor(), 2);
        workspace.reply(Reply::Main { generation: diff_generation, view: MainView::note("late diff") });
        assert_eq!(workspace.main(), &list);
        workspace.key(Key::Escape);
        assert_eq!(workspace.focus(), Pane::Commits);
    }

    #[test]
    fn space_in_commits_asks_before_checking_out_the_commit_detached() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('4'));
        workspace.key(Key::Char('j'));
        workspace.key(Key::Space);
        assert_eq!(workspace.confirm(), Some(&Confirm::Detach { id: "2".to_owned(), short: "2".to_owned() }));
        assert_eq!(approve(&mut workspace), Job::ExecutePrepared("fixture".to_owned()));
    }

    #[test]
    fn d_asks_before_deleting_a_branch_and_only_y_deletes_it() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('3'));
        workspace.key(Key::Down);
        assert_eq!(workspace.key(Key::Char('d')), Effect::Nothing);
        assert_eq!(workspace.notice(), Some("main is checked out and cannot be deleted"));
        workspace.key(Key::Up);
        workspace.key(Key::Char('d'));
        assert_eq!(workspace.confirm(), Some(&Confirm::DeleteBranch("feature".to_owned())));
        assert_eq!(workspace.key(Key::Char('n')), Effect::Send(Job::CancelPrepared));
        assert!(workspace.confirm().is_none());
        workspace.key(Key::Char('d'));
        assert_eq!(approve(&mut workspace), Job::ExecutePrepared("fixture".to_owned()));
    }

    #[test]
    fn n_creates_a_branch_only_from_the_branch_panel_and_refuses_an_empty_name() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('n'));
        assert!(workspace.prompt().is_none());
        workspace.key(Key::Char('3'));
        workspace.key(Key::Char('n'));
        assert_eq!(workspace.prompt().map(Prompt::kind), Some(PromptKind::NewBranch));
        assert_eq!(workspace.key(Key::Enter), Effect::Nothing, "an empty name sends nothing");
        workspace.key(text(" review "));
        assert_eq!(sent(workspace.key(Key::Enter)), Job::CreateBranch("review".to_owned()));
        assert!(workspace.prompt().is_some(), "the prompt stays until the backend answers");
        workspace.reply(Reply::BranchCreated(snapshot(None)));
        assert!(workspace.prompt().is_none());
    }

    fn ready_commit(workspace: &mut Workspace) {
        let generation = workspace.composer.as_ref().unwrap().review_generation;
        workspace.reply(Reply::CommitReview { generation, review: crate::model::CommitReview { ticket: "test-review".into(), head: "main | 0123456789ab | ordinary".into(), files: vec![crate::model::StagedRow { path: NativePath::new("a.txt"), change: "M".into(), added: Some(1), deleted: Some(1) }], blocked: None } });
        let generation = workspace.composer.as_ref().unwrap().diff_generation;
        workspace.reply(Reply::CommitDiff { generation, view: MainView::note("reviewed staged content") });
    }

    #[test]
    fn a_refused_commit_keeps_the_message_and_a_successful_one_closes_the_prompt() {
        let mut workspace = Workspace::new(snapshot(Some("draft text")));
        workspace.open();
        workspace.key(Key::Char('c'));
        ready_commit(&mut workspace);
        assert_eq!(workspace.prompt().map(Prompt::text), Some("draft text"));
        workspace.key(Key::Char('!'));
        assert_eq!(sent(workspace.key(Key::CtrlEnter)), Job::Commit { ticket: "test-review".into(), message: "draft text!".into() });
        workspace.reply(Reply::PromptFailed { action: PromptAction::Commit, text: "nothing is staged to commit".to_owned() });
        assert_eq!(workspace.prompt().map(Prompt::text), Some("draft text!"));
        assert_eq!(workspace.key(Key::CtrlEnter), Effect::Nothing);
        workspace.refresh_commit_review();
        ready_commit(&mut workspace);
        assert_eq!(sent(workspace.key(Key::CtrlEnter)), Job::Commit { ticket: "test-review".into(), message: "draft text!".into() });
        workspace.reply(Reply::Committed(snapshot(None)));
        assert!(workspace.prompt().is_none());
    }

    #[test]
    fn completed_write_with_unavailable_refresh_closes_only_its_own_prompt_and_reports_stale_state() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.key(Key::Char('c'));
        ready_commit(&mut workspace);
        workspace.key(Key::Text("already created".into()));
        assert_eq!(workspace.key(Key::CtrlEnter), Effect::Send(Job::Commit { ticket: "test-review".into(), message: "already created".into() }));
        let old_generation = workspace.generation;
        workspace.reply(Reply::PromptCompleted { action: PromptAction::Commit, text: "Commit abc created.\nState refresh unavailable; displayed lists may be stale.\nDo not resubmit.".into() });
        assert!(workspace.prompt().is_none(), "a known completed write is acknowledged");
        assert!(workspace.main().lines.iter().any(|line| line.text.contains("stale")));
        workspace.reply(Reply::Main { generation: old_generation, view: MainView::note("obsolete preview") });
        assert!(!workspace.main().lines.iter().any(|line| line.text == "obsolete preview"), "an older preview must not erase the reconciliation guidance");
        workspace.key(Key::Char('c'));
        ready_commit(&mut workspace);
        workspace.key(Key::Text("keep unrelated draft".into()));
        workspace.key(Key::Escape);
        workspace.reply(Reply::PromptCompleted { action: PromptAction::CreateBranch, text: "Branch created; refresh unavailable".into() });
        assert_eq!(workspace.prompt().unwrap().submitted, Some(PromptAction::SaveDraft), "a different completion cannot acknowledge a pending draft save");
    }

    #[test]
    fn an_unrelated_snapshot_cannot_acknowledge_a_pending_commit() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('c'));
        ready_commit(&mut workspace);
        workspace.key(text("retain this message"));
        sent(workspace.key(Key::CtrlEnter));
        workspace.reply(Reply::Full(snapshot(None)));
        assert_eq!(workspace.prompt().map(Prompt::text), Some("retain this message"));
        workspace.reply(Reply::Notice("earlier stage refused".into()));
        assert_eq!(workspace.key(Key::CtrlEnter), Effect::Nothing, "a different job's failure cannot enable duplicate submit");
    }

    #[test]
    fn a_draft_save_failure_keeps_the_unsaved_message_available() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('c'));
        ready_commit(&mut workspace);
        workspace.key(text("keep after I/O failure"));
        sent(workspace.key(Key::Escape));
        assert_eq!(workspace.prompt().map(Prompt::text), Some("keep after I/O failure"), "closing before persistence is acknowledged loses the message on failure");
        workspace.reply(Reply::PromptFailed { action: PromptAction::SaveDraft, text: "profile write denied".into() });
        assert_eq!(workspace.prompt().map(Prompt::text), Some("keep after I/O failure"));
        assert_eq!(sent(workspace.key(Key::Escape)), Job::SaveDraft("keep after I/O failure".into()));
        workspace.reply(Reply::DraftSaved(Some("keep after I/O failure".into())));
        assert!(workspace.prompt().is_none());
    }

    #[test]
    fn escape_in_the_commit_prompt_saves_the_draft_and_closes_it() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('c'));
        ready_commit(&mut workspace);
        workspace.key(text("subject"));
        workspace.key(Key::Enter);
        workspace.key(text("body"));
        assert_eq!(sent(workspace.key(Key::Escape)), Job::SaveDraft("subject\nbody".to_owned()));
        assert!(workspace.prompt().is_some());
        workspace.reply(Reply::DraftSaved(Some("subject\nbody".to_owned())));
        assert!(workspace.prompt().is_none());
    }

    #[test]
    fn while_a_prompt_is_open_letters_are_text_not_commands() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('c'));
        ready_commit(&mut workspace);
        assert_eq!(workspace.key(Key::Char('q')), Effect::Nothing);
        assert_eq!(workspace.prompt().map(Prompt::text), Some("q"));
        assert_eq!(workspace.key(Key::CtrlC), Effect::Nothing, "Copy without selection never closes the message editor");
        assert_eq!(workspace.key(Key::Escape), Effect::Send(Job::SaveDraft("q".to_owned())));
    }

    #[test]
    fn q_and_ctrl_c_quit_outside_a_prompt_and_help_swallows_keys() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('?'));
        assert!(workspace.help());
        assert_eq!(workspace.key(Key::Char('q')), Effect::Nothing);
        workspace.key(Key::Escape);
        assert!(!workspace.help());
        assert!(matches!(workspace.key(Key::Char('q')), Effect::Send(Job::EnterRepository { path: None, current: Some(_) })));
        assert_eq!(workspace.reply(Reply::RepositoryClosed(Vec::new())), Effect::Quit);
        assert!(matches!(Workspace::new(snapshot(None)).key(Key::CtrlC), Effect::Send(Job::EnterRepository { path: None, current: Some(_) })));
    }

    #[test]
    fn a_click_on_a_row_focuses_the_list_and_selects_the_row() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        let Job::Main { request: MainRequest::File { path, .. }, .. } = sent(workspace.click_row(Pane::Files, 2))
        else {
            panic!("a file diff is requested")
        };
        assert_eq!(path, NativePath::new("c.txt"));
        assert_eq!((workspace.focus(), workspace.selected(Pane::Files)), (Pane::Files, 2));
        assert_eq!(workspace.click_row(Pane::Files, 2), Effect::Nothing, "the same row asks for nothing");
    }

    #[test]
    fn a_click_past_the_last_row_only_focuses_the_list() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('3'));
        sent(workspace.click_row(Pane::Files, 40));
        assert_eq!((workspace.focus(), workspace.selected(Pane::Files)), (Pane::Files, 0));
    }

    #[test]
    fn a_click_in_the_main_panel_moves_the_cursor_and_selects_its_hunk() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.reply(Reply::Main {
            generation: workspace.generation,
            view: MainView { lines: lines_of(12), hunks: vec![0, 5, 9], file: None, committed: None },
        });
        workspace.click_main(Some(6));
        assert_eq!((workspace.focus(), workspace.cursor(), workspace.hunk()), (Pane::Main, 6, 1));
        workspace.click_main(None);
        assert_eq!(workspace.hunk(), 1, "a click on the frame keeps the hunk");
    }

    #[test]
    fn the_wheel_moves_the_list_selection_and_scrolls_the_main_panel() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        let focus = workspace.focus();
        workspace.wheel(Pane::Files, 9);
        assert_eq!(workspace.selected(Pane::Files), 2, "the wheel stops at the last row");
        assert_eq!(workspace.focus(), focus, "the wheel over a list keeps the focus");
        workspace.reply(Reply::Main {
            generation: workspace.generation,
            view: MainView { lines: lines_of(12), hunks: Vec::new(), file: None, committed: None },
        });
        workspace.wheel(Pane::Main, 3);
        assert_eq!((workspace.focus(), workspace.scroll()), (Pane::Main, 3));
    }

    #[test]
    fn paging_moves_the_cursor_and_the_view_follows_it() {
        let mut workspace = Workspace::new(snapshot(None));
        let Job::Main { generation, .. } = sent(workspace.open()) else { panic!() };
        let lines: String = (0..30).map(|index| format!("line {index}\n")).collect();
        workspace.reply(Reply::Main { generation, view: MainView::text(&lines) });
        workspace.set_view_rows(5);
        workspace.key(Key::Enter);
        workspace.key(Key::PageDown);
        assert_eq!((workspace.cursor(), workspace.scroll()), (PAGE_LINES, PAGE_LINES + 1 - 5));
        workspace.key(Key::End);
        assert_eq!((workspace.cursor(), workspace.scroll()), (29, 25));
        assert_eq!(workspace.key(Key::PageDown), Effect::Nothing, "the cursor stops at the last line");
        workspace.key(Key::Home);
        assert_eq!((workspace.cursor(), workspace.scroll()), (0, 0));
        workspace.key(Key::HalfPageDown);
        assert_eq!(workspace.cursor(), HALF_PAGE_LINES);
    }

    #[test]
    fn a_reload_clamps_the_selection_to_the_new_lists() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Down);
        workspace.key(Key::Down);
        let mut smaller = snapshot(None);
        smaller.files.truncate(1);
        workspace.reply(Reply::Full(smaller));
        assert_eq!(workspace.selected(Pane::Files), 0);
    }
}

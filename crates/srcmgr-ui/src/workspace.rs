//! The window's state: which pane has focus, what is selected, what the main
//! panel shows and which job each key asks the backend to run. Pure logic, so
//! it is tested without a window.
//!
//! Panels and keys follow lazygit: 1 Status, 2 Files, 3 Branches, 4 Commits.
//! In the main panel a cursor moves over the diff; Space stages the cursor line
//! (or a `v` range of lines) and `a` stages the whole hunk. See PRD 10 for the
//! full list and the deliberate differences.

use srcmgr_domain::NativePath;
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
    Nothing,
    Send(Job),
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptKind {
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

impl Prompt {
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
            (PromptKind::Commit, Key::Escape | Key::CtrlC) => {
                self.submitted = Some(PromptAction::SaveDraft);
                (Effect::Send(Job::SaveDraft(self.text.clone())), true)
            }
            (PromptKind::NewBranch, Key::Escape | Key::CtrlC) => (Effect::Nothing, false),
            (PromptKind::Commit, Key::CtrlEnter) => {
                self.submitted = Some(PromptAction::Commit);
                (Effect::Send(Job::Commit(self.text.clone())), true)
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
    DeleteBranch(String),
    DiscardFile { path: NativePath, untracked: bool },
    DiscardHunk { path: NativePath, index: usize },
    Detach { id: String, short: String },
    Stash { id: String, operation: String },
}

pub struct Workspace {
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
    detail_back: Option<(MainView, usize, usize)>,
}

impl Workspace {
    #[must_use]
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            snapshot,
            focus: Pane::Files,
            source: Pane::Files,
            files: 0,
            branches: 0,
            commits: 0,
            stashes: 0,
            stash_mode: false,
            main: MainView::default(),
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
            detail_back: None,
        }
    }

    /// Requests the main-panel content for the first list. Call once after `new`.
    pub fn open(&mut self) -> Effect {
        self.show_source()
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
    pub const fn selected(&self, pane: Pane) -> usize {
        match pane {
            Pane::Files => self.files,
            Pane::Branches => self.branches,
            Pane::Commits => self.commits,
            Pane::Stashes => self.stashes,
            Pane::Status | Pane::Main => 0,
        }
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

    fn prepare_confirmation(&mut self, action: Job) -> Effect {
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
        self.help || self.prompt.is_some() || self.confirm.is_some()
    }

    /// Tells the workspace how many main-panel lines are visible, so the cursor
    /// can be kept on screen.
    pub fn set_view_rows(&mut self, rows: usize) {
        self.view_rows = rows;
        self.follow();
    }

    pub fn key(&mut self, key: Key) -> Effect {
        if let Some(mut prompt) = self.prompt.take() {
            let (effect, stays_open) = prompt.handle(key);
            if stays_open {
                self.prompt = Some(prompt);
            }
            return effect;
        }
        if let Some(confirm) = self.confirm.take() {
            return self.answer(confirm, key);
        }
        if self.help {
            if matches!(key, Key::Escape | Key::Char('?')) {
                self.help = false;
            }
            return Effect::Nothing;
        }
        match key {
            Key::Char('q') | Key::CtrlC => Effect::Quit,
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
                if let Some((view, cursor, scroll)) = self.detail_back.take() {
                    self.generation += 1;
                    self.main = view;
                    self.cursor = cursor;
                    self.scroll = scroll;
                } else if self.main.committed.is_some() {
                    self.focus = Pane::Commits;
                    return self.show_source();
                } else if self.anchor.is_some() {
                    self.anchor = None;
                } else if self.focus == Pane::Main {
                    self.focus = self.source;
                }
                Effect::Nothing
            }
            Key::Enter => {
                if self.focus == Pane::Commits {
                    let Some(commit) = self.snapshot.commits.get(self.commits) else { return Effect::Nothing; };
                    let request = MainRequest::CommitFiles { id: commit.id.clone() };
                    self.detail_back = None;
                    self.focus = Pane::Main;
                    self.generation += 1;
                    self.main = MainView::note("Loading committed files...");
                    self.cursor = 0;
                    self.scroll = 0;
                    self.anchor = None;
                    return Effect::Send(Job::Main { generation: self.generation, request });
                }
                if self.focus == Pane::Main && let Some(list) = &self.main.committed {
                        let Some(file) = self.cursor.checked_sub(2).and_then(|i| list.files.get(i)) else { return Effect::Nothing; };
                        let request = MainRequest::CommitFile { id: list.id.clone(), file: file.clone() };
                        self.detail_back = Some((self.main.clone(), self.cursor, self.scroll));
                        self.generation += 1;
                        self.main = MainView::note("Loading committed file diff...");
                        self.cursor = 0;
                        self.scroll = 0;
                        self.anchor = None;
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
            Key::Char('c') => {
                self.notice = None;
                self.prompt = Some(Prompt {
                    kind: PromptKind::Commit,
                    text: self.snapshot.draft.clone().unwrap_or_default(),
                    submitted: None,
                });
                Effect::Nothing
            }
            Key::Char('n') if self.focus == Pane::Branches => {
                self.prompt = Some(Prompt { kind: PromptKind::NewBranch, text: String::new(), submitted: None });
                Effect::Nothing
            }
            Key::Char('d') if self.focus == Pane::Branches => self.ask_delete(),
            Key::Char('d') if self.focus == Pane::Stashes => self.ask_stash_action("drop"),
            Key::Char('p') if self.focus == Pane::Stashes => self.ask_stash_action("pop"),
            Key::Char('d') if self.focus == Pane::Files => self.ask_discard_file(),
            Key::Char('d') if self.focus == Pane::Main => self.ask_discard_hunk(),
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
            Reply::NoChange => Effect::Nothing,
            Reply::MainFailed { generation, text } => {
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
                self.notice = None;
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
                    self.scroll = 0;
                    self.anchor = None;
                    // The cursor starts on the first changed line, or the top.
                    let last = self.main.lines.len().saturating_sub(1);
                    self.cursor = if self.main.committed.as_ref().is_some_and(|list| !list.files.is_empty()) { 2 } else { self.main.hunks.first().map_or(0, |start| (start + 1).min(last)) };
                    self.follow();
                }
                Effect::Nothing
            }
            Reply::Notice(text) => {
                self.notice = Some(text);
                Effect::Nothing
            }
            Reply::PromptFailed { action, text } => {
                self.notice = Some(text);
                if let Some(prompt) = self.prompt.as_mut() && prompt.submitted == Some(action) {
                    prompt.submitted = None;
                }
                Effect::Nothing
            }
            Reply::PromptCompleted { action, text } => {
                self.close_prompt(action);
                self.generation += 1;
                self.main = MainView::text(&text);
                self.cursor = 0;
                self.scroll = 0;
                self.anchor = None;
                self.detail_back = None;
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
        }
    }

    /// A left click on a list. Focuses the list and selects the row; a row past
    /// the end of the list only focuses it.
    pub fn click_row(&mut self, pane: Pane, index: usize) -> Effect {
        if pane == Pane::Status || index >= self.list_length(pane) {
            return self.focus_list(pane);
        }
        let changed = self.selected(pane) != index || self.source != pane;
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
                Some(row) => Effect::Send(Job::SetFile { path: row.path.clone(), stage: row.has_unstaged }),
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
            Pane::Main => self.stage_lines(),
        }
    }

    /// `a`: stage or unstage every file, or stage the hunk under the cursor.
    fn act_all(&mut self) -> Effect {
        self.notice = None;
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
            Pane::Main => self.stage_hunk(),
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
        self.detail_back = None;
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
                    staged_view: !row.has_unstaged,
                    untracked: row.untracked,
                },
                None => {
                    self.main = MainView::note("Working tree clean");
                    return Effect::Nothing;
                }
            },
            Pane::Commits => match self.snapshot.commits.get(self.commits) {
                Some(commit) => MainRequest::Commit { id: commit.id.clone() },
                None => {
                    self.main = MainView::note("No commits yet");
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
        Effect::Send(Job::Main { generation: self.generation, request })
    }

    /// The Status panel has one row; the others have one row per item.
    const fn list_length(&self, pane: Pane) -> usize {
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
        workspace.key(Key::Escape);
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
        workspace.key(Key::Escape);
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

    #[test]
    fn a_refused_commit_keeps_the_message_and_a_successful_one_closes_the_prompt() {
        let mut workspace = Workspace::new(snapshot(Some("draft text")));
        workspace.open();
        workspace.key(Key::Char('c'));
        assert_eq!(workspace.prompt().map(Prompt::text), Some("draft text"));
        workspace.key(Key::Char('!'));
        assert_eq!(sent(workspace.key(Key::CtrlEnter)), Job::Commit("draft text!".to_owned()));
        workspace.reply(Reply::PromptFailed { action: PromptAction::Commit, text: "nothing is staged to commit".to_owned() });
        assert_eq!(workspace.prompt().map(Prompt::text), Some("draft text!"));
        assert_eq!(sent(workspace.key(Key::CtrlEnter)), Job::Commit("draft text!".to_owned()));
        workspace.reply(Reply::Committed(snapshot(None)));
        assert!(workspace.prompt().is_none());
    }

    #[test]
    fn completed_write_with_unavailable_refresh_closes_only_its_own_prompt_and_reports_stale_state() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.key(Key::Char('c'));
        workspace.key(Key::Text("already created".into()));
        assert_eq!(workspace.key(Key::CtrlEnter), Effect::Send(Job::Commit("already created".into())));
        let old_generation = workspace.generation;
        workspace.reply(Reply::PromptCompleted { action: PromptAction::Commit, text: "Commit abc created.\nState refresh unavailable; displayed lists may be stale.\nDo not resubmit.".into() });
        assert!(workspace.prompt().is_none(), "a known completed write is acknowledged");
        assert!(workspace.main().lines.iter().any(|line| line.text.contains("stale")));
        workspace.reply(Reply::Main { generation: old_generation, view: MainView::note("obsolete preview") });
        assert!(!workspace.main().lines.iter().any(|line| line.text == "obsolete preview"), "an older preview must not erase the reconciliation guidance");
        workspace.key(Key::Char('c'));
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
        assert_eq!(workspace.key(Key::Char('q')), Effect::Nothing);
        assert_eq!(workspace.prompt().map(Prompt::text), Some("q"));
        assert_eq!(workspace.key(Key::CtrlC), Effect::Send(Job::SaveDraft("q".to_owned())));
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
        assert_eq!(workspace.key(Key::Char('q')), Effect::Quit);
        assert_eq!(Workspace::new(snapshot(None)).key(Key::CtrlC), Effect::Quit);
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

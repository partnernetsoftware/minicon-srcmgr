//! The window's state: which pane has focus, what is selected, what the main
//! panel shows and which job each key asks the backend to run. Pure logic, so
//! it is tested without a window.
//!
//! Keys follow lazygit's core panel flow where the reference defines one:
//! Space stages or checks out the selection, `c` commits, `n` makes a branch,
//! and Enter moves into the main panel. See PRD 10 for the full list.

use crate::model::{Job, Line, MainRequest, MainView, Reply, Snapshot, Tone};

/// Lines moved by PgUp and PgDn in the main panel.
pub const PAGE_LINES: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pane {
    Files,
    Branches,
    Commits,
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
    submitted: bool,
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
        if self.submitted {
            return (Effect::Nothing, true);
        }
        match (self.kind, key) {
            (PromptKind::Commit, Key::Escape | Key::CtrlC) => (Effect::Send(Job::SaveDraft(self.text.clone())), false),
            (PromptKind::NewBranch, Key::Escape | Key::CtrlC) => (Effect::Nothing, false),
            (PromptKind::Commit, Key::CtrlEnter) => {
                self.submitted = true;
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
                self.submitted = true;
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

pub struct Workspace {
    snapshot: Snapshot,
    focus: Pane,
    /// The list whose selection the main panel shows. It is set whenever a list
    /// gains focus, and it stays while the main panel has focus.
    source: Pane,
    files: usize,
    branches: usize,
    commits: usize,
    main: MainView,
    scroll: usize,
    hunk: usize,
    /// Incremented on every main-panel request, so late replies can be dropped.
    generation: u64,
    notice: Option<String>,
    prompt: Option<Prompt>,
    help: bool,
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
            main: MainView::default(),
            scroll: 0,
            hunk: 0,
            generation: 0,
            notice: None,
            prompt: None,
            help: false,
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
            Pane::Main => 0,
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
    pub const fn hunk(&self) -> usize {
        self.hunk
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
    pub const fn help(&self) -> bool {
        self.help
    }

    pub fn key(&mut self, key: Key) -> Effect {
        if let Some(mut prompt) = self.prompt.take() {
            let (effect, stays_open) = prompt.handle(key);
            if stays_open {
                self.prompt = Some(prompt);
            }
            return effect;
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
            Key::Char('1') => self.focus_list(Pane::Files),
            Key::Char('2') => self.focus_list(Pane::Branches),
            Key::Char('3') => self.focus_list(Pane::Commits),
            Key::Tab => self.cycle(true),
            Key::BackTab => self.cycle(false),
            Key::Escape => {
                self.notice = None;
                if self.focus == Pane::Main {
                    self.focus = self.source;
                }
                Effect::Nothing
            }
            Key::Enter => {
                if self.focus != Pane::Main {
                    self.focus = Pane::Main;
                }
                Effect::Nothing
            }
            Key::Up | Key::Char('k') => self.move_by(-1),
            Key::Down | Key::Char('j') => self.move_by(1),
            Key::PageUp | Key::Char('K') => self.scroll_by(-(PAGE_LINES as isize)),
            Key::PageDown | Key::Char('J') => self.scroll_by(PAGE_LINES as isize),
            Key::Home => {
                self.scroll = 0;
                Effect::Nothing
            }
            Key::End => {
                self.scroll = self.max_scroll();
                Effect::Nothing
            }
            Key::Left | Key::Char('h') => self.step_hunk(false),
            Key::Right | Key::Char('l') => self.step_hunk(true),
            Key::Space => self.act(),
            Key::Char('c') => {
                self.notice = None;
                self.prompt = Some(Prompt {
                    kind: PromptKind::Commit,
                    text: self.snapshot.draft.clone().unwrap_or_default(),
                    submitted: false,
                });
                Effect::Nothing
            }
            Key::Char('n') if self.focus == Pane::Branches => {
                self.prompt = Some(Prompt { kind: PromptKind::NewBranch, text: String::new(), submitted: false });
                Effect::Nothing
            }
            Key::Char('r') => Effect::Send(Job::Reload),
            _ => Effect::Nothing,
        }
    }

    /// Applies a backend answer. Lists are replaced and the main panel is
    /// requested again for the current selection.
    pub fn reply(&mut self, reply: Reply) -> Effect {
        match reply {
            Reply::Full(snapshot) => {
                self.snapshot = snapshot;
                self.files = clamp_index(self.files, self.snapshot.files.len());
                self.branches = clamp_index(self.branches, self.snapshot.branches.len());
                self.commits = clamp_index(self.commits, self.snapshot.commits.len());
                if self.prompt.as_ref().is_some_and(|prompt| prompt.submitted) {
                    self.prompt = None;
                }
                self.notice = None;
                self.show_source()
            }
            Reply::Main { generation, view } => {
                if generation == self.generation {
                    self.main = view;
                    self.scroll = 0;
                    self.hunk = 0;
                }
                Effect::Nothing
            }
            Reply::Notice(text) => {
                self.notice = Some(text);
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.submitted = false;
                }
                Effect::Nothing
            }
        }
    }

    fn focus_list(&mut self, pane: Pane) -> Effect {
        self.notice = None;
        self.focus = pane;
        if self.source == pane {
            return Effect::Nothing;
        }
        self.source = pane;
        self.show_source()
    }

    fn cycle(&mut self, forward: bool) -> Effect {
        const ORDER: [Pane; 4] = [Pane::Files, Pane::Branches, Pane::Commits, Pane::Main];
        let index = ORDER.iter().position(|pane| *pane == self.focus).unwrap_or(0);
        let next =
            if forward { ORDER[(index + 1) % ORDER.len()] } else { ORDER[(index + ORDER.len() - 1) % ORDER.len()] };
        if next == Pane::Main {
            self.focus = Pane::Main;
            return Effect::Nothing;
        }
        self.focus_list(next)
    }

    fn move_by(&mut self, delta: isize) -> Effect {
        if self.focus == Pane::Main {
            return self.scroll_by(delta);
        }
        let length = self.list_length(self.focus);
        if length == 0 {
            return Effect::Nothing;
        }
        let current = self.selected(self.focus);
        let next = (current as isize + delta).clamp(0, length as isize - 1) as usize;
        if next == current {
            return Effect::Nothing;
        }
        self.set_selected(self.focus, next);
        self.show_source()
    }

    fn scroll_by(&mut self, delta: isize) -> Effect {
        let next = (self.scroll as isize + delta).clamp(0, self.max_scroll() as isize);
        self.scroll = next as usize;
        Effect::Nothing
    }

    fn step_hunk(&mut self, forward: bool) -> Effect {
        if self.focus != Pane::Main || self.main.hunks.is_empty() {
            return Effect::Nothing;
        }
        let last = self.main.hunks.len() - 1;
        self.hunk = if forward { (self.hunk + 1).min(last) } else { self.hunk.saturating_sub(1) };
        self.scroll = self.main.hunks[self.hunk].min(self.max_scroll());
        Effect::Nothing
    }

    /// Space: stage or check out the focused item, or stage the selected hunk.
    fn act(&mut self) -> Effect {
        self.notice = None;
        match self.focus {
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
            Pane::Commits => {
                self.notice = Some("Checking out a commit is not available yet".to_owned());
                Effect::Nothing
            }
            Pane::Main => {
                let Some(target) = self.main.file.clone() else {
                    self.notice = Some("Space acts on a file diff".to_owned());
                    return Effect::Nothing;
                };
                if self.main.hunks.is_empty() {
                    self.notice = Some("this diff has no hunks".to_owned());
                    return Effect::Nothing;
                }
                Effect::Send(Job::SetHunk { path: target.path, staged_view: target.staged_view, number: self.hunk + 1 })
            }
        }
    }

    /// Asks for the main-panel content of the source list. The main panel
    /// shows a loading note until the reply arrives.
    fn show_source(&mut self) -> Effect {
        self.generation += 1;
        self.scroll = 0;
        self.hunk = 0;
        let request = match self.source {
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
            Pane::Main => return Effect::Nothing,
        };
        self.main = MainView {
            lines: vec![Line { tone: Tone::Muted, text: "Loading...".to_owned() }],
            hunks: Vec::new(),
            file: None,
        };
        Effect::Send(Job::Main { generation: self.generation, request })
    }

    const fn list_length(&self, pane: Pane) -> usize {
        match pane {
            Pane::Files => self.snapshot.files.len(),
            Pane::Branches => self.snapshot.branches.len(),
            Pane::Commits => self.snapshot.commits.len(),
            Pane::Main => 0,
        }
    }

    fn set_selected(&mut self, pane: Pane, value: usize) {
        match pane {
            Pane::Files => self.files = value,
            Pane::Branches => self.branches = value,
            Pane::Commits => self.commits = value,
            Pane::Main => {}
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
        }
    }

    fn sent(effect: Effect) -> Job {
        match effect {
            Effect::Send(job) => job,
            other => panic!("expected a job, got {other:?}"),
        }
    }

    fn text(value: &str) -> Key {
        Key::Text(value.to_owned())
    }

    fn file_diff(path: &str, staged_view: bool) -> MainView {
        MainView::diff(
            "@@ -1 +1 @@\n-old\n+new\n@@ -5 +5 @@\n-x\n+y\n",
            FileTarget { path: NativePath::new(path), staged_view },
        )
    }

    #[test]
    fn opening_asks_for_the_first_file_diff_and_loads_it_into_the_main_panel() {
        let mut workspace = Workspace::new(snapshot(None));
        let job = sent(workspace.open());
        let Job::Main { generation, request } = job else { panic!("main job expected") };
        assert_eq!(request, MainRequest::File { path: NativePath::new("a.txt"), staged_view: false, untracked: false });
        assert_eq!(workspace.reply(Reply::Main { generation, view: file_diff("a.txt", false) }), Effect::Nothing);
        assert_eq!(workspace.main().hunks.len(), 2);
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
    fn number_keys_and_tab_move_focus_and_source() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('2'));
        assert_eq!(workspace.focus(), Pane::Branches);
        assert_eq!(workspace.source(), Pane::Branches);
        workspace.key(Key::Tab);
        assert_eq!(workspace.focus(), Pane::Commits);
        workspace.key(Key::Tab);
        assert_eq!(workspace.focus(), Pane::Main);
        assert_eq!(workspace.source(), Pane::Commits, "the main panel keeps the list it shows");
        workspace.key(Key::BackTab);
        assert_eq!(workspace.focus(), Pane::Commits);
        workspace.key(Key::Char('1'));
        workspace.key(Key::Enter);
        assert_eq!(workspace.focus(), Pane::Main);
        workspace.key(Key::Escape);
        assert_eq!(workspace.focus(), Pane::Files, "escape returns to the list that feeds the main panel");
    }

    #[test]
    fn space_in_the_main_panel_stages_the_selected_hunk_with_its_own_direction() {
        let mut workspace = Workspace::new(snapshot(None));
        let Job::Main { generation, .. } = sent(workspace.open()) else { panic!() };
        workspace.reply(Reply::Main { generation, view: file_diff("a.txt", false) });
        workspace.key(Key::Enter);
        workspace.key(Key::Right);
        assert_eq!(workspace.hunk(), 1);
        assert_eq!(
            sent(workspace.key(Key::Space)),
            Job::SetHunk { path: NativePath::new("a.txt"), staged_view: false, number: 2 }
        );
        workspace.key(Key::Escape);
        // b.txt has no unstaged change, so its diff is the index view and hunk
        // actions on it unstage.
        let Job::Main { generation, request } = sent(workspace.key(Key::Down)) else { panic!() };
        assert_eq!(request, MainRequest::File { path: NativePath::new("b.txt"), staged_view: true, untracked: false });
        workspace.reply(Reply::Main { generation, view: file_diff("b.txt", true) });
        workspace.key(Key::Enter);
        assert_eq!(
            sent(workspace.key(Key::Space)),
            Job::SetHunk { path: NativePath::new("b.txt"), staged_view: true, number: 1 }
        );
    }

    #[test]
    fn checking_out_the_current_branch_is_refused_and_another_branch_switches() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('2'));
        workspace.key(Key::Down);
        assert_eq!(workspace.key(Key::Space), Effect::Nothing);
        assert_eq!(workspace.notice(), Some("main is already checked out"));
        workspace.key(Key::Up);
        assert_eq!(sent(workspace.key(Key::Space)), Job::Switch("feature".to_owned()));
    }

    #[test]
    fn n_creates_a_branch_only_from_the_branch_panel_and_refuses_an_empty_name() {
        let mut workspace = Workspace::new(snapshot(None));
        workspace.open();
        workspace.key(Key::Char('n'));
        assert!(workspace.prompt().is_none());
        workspace.key(Key::Char('2'));
        workspace.key(Key::Char('n'));
        assert_eq!(workspace.prompt().map(Prompt::kind), Some(PromptKind::NewBranch));
        assert_eq!(workspace.key(Key::Enter), Effect::Nothing, "an empty name sends nothing");
        workspace.key(text(" review "));
        assert_eq!(sent(workspace.key(Key::Enter)), Job::CreateBranch("review".to_owned()));
        assert!(workspace.prompt().is_some(), "the prompt stays until the backend answers");
        workspace.reply(Reply::Full(snapshot(None)));
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
        workspace.reply(Reply::Notice("nothing is staged to commit".to_owned()));
        assert_eq!(workspace.prompt().map(Prompt::text), Some("draft text!"));
        assert_eq!(sent(workspace.key(Key::CtrlEnter)), Job::Commit("draft text!".to_owned()));
        workspace.reply(Reply::Full(snapshot(None)));
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
    fn page_keys_scroll_the_main_panel_within_its_bounds() {
        let mut workspace = Workspace::new(snapshot(None));
        let Job::Main { generation, .. } = sent(workspace.open()) else { panic!() };
        let lines: String = (0..30).map(|index| format!("line {index}\n")).collect();
        workspace.reply(Reply::Main { generation, view: MainView::text(&lines) });
        workspace.key(Key::PageDown);
        assert_eq!(workspace.scroll(), PAGE_LINES);
        workspace.key(Key::End);
        assert_eq!(workspace.scroll(), 29);
        workspace.key(Key::PageDown);
        assert_eq!(workspace.scroll(), 29);
        workspace.key(Key::Home);
        workspace.key(Key::PageUp);
        assert_eq!(workspace.scroll(), 0);
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

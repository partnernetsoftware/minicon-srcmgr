//! Plain data the window shows, and the messages it exchanges with the Git
//! side. The window never runs Git and the backend never draws; everything
//! crossing the boundary is one of these types.

use srcmgr_domain::{ChangeEntry, ChangeKind, EncodedPath, HeadState, NativePath, StatusSnapshot};

/// Semantic colour role. The window maps roles to pixels; the model never does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Heading,
    Text,
    Muted,
    Staged,
    Changed,
    Untracked,
    Added,
    Removed,
    Hunk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    pub tone: Tone,
    pub text: String,
}

/// One changed path. `path` is the raw path Git receives; `label` is what the
/// list shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileRow {
    pub path: NativePath,
    pub label: String,
    pub tone: Tone,
    pub has_staged: bool,
    pub has_unstaged: bool,
    pub untracked: bool,
}

impl FileRow {
    #[must_use]
    pub fn from_entry(entry: &ChangeEntry) -> Self {
        let untracked = entry.unstaged == ChangeKind::Untracked;
        Self {
            path: entry.path.clone(),
            label: format!("{}{} {}", code(entry.staged), code(entry.unstaged), entry.path.display_label()),
            tone: tone_of(entry.staged, entry.unstaged),
            has_staged: entry.staged != ChangeKind::Unmodified,
            has_unstaged: entry.unstaged != ChangeKind::Unmodified,
            untracked,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchRow {
    pub name: String,
    pub label: String,
    pub current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitRow {
    /// The full revision id, passed back to Git for details.
    pub id: String,
    pub subject: String,
    pub merge: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StashRow {
    pub id: String,
    pub subject: String,
}

/// Everything the lists show. Main-panel content is requested separately.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentRow {
    pub backend: String,
    pub path: EncodedPath,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustReview {
    pub ticket: String,
    pub worktree: NativePath,
    pub git_directory: NativePath,
    pub fingerprint: String,
    pub submitted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub history_limit: usize,
    pub navigation: Option<crate::Navigation>,
    /// None is an explicit Welcome state, not a guessed Git repository.
    pub repository: Option<NativePath>,
    pub recents: Vec<RecentRow>,
    pub title: String,
    pub head: String,
    pub files: Vec<FileRow>,
    pub branches: Vec<BranchRow>,
    pub commits: Vec<CommitRow>,
    pub draft: Option<String>,
    /// Last observed stage/unstage result, naming its own path scope.
    pub stage_result: Option<String>,
    pub stashes: Vec<StashRow>,
}

impl Snapshot {
    #[must_use]
    pub fn new(
        repository: NativePath,
        title: impl Into<String>,
        status: &StatusSnapshot,
        branches: Vec<BranchRow>,
        commits: Vec<CommitRow>,
        draft: Option<String>,
    ) -> Self {
        Self {
            navigation: None,
            history_limit: 200,
            repository: Some(repository),
            recents: Vec::new(),
            title: title.into(),
            head: head_text(&status.head),
            files: status.entries.iter().map(FileRow::from_entry).collect(),
            branches,
            commits,
            draft,
            stage_result: None,
            stashes: Vec::new(),
        }
    }

    pub fn welcome() -> Self {
        Self {
            navigation: None,
            history_limit: 200,
            repository: None, recents: Vec::new(), title: "MiniCon SrcMgr".into(),
            head: "Open a local repository. No account required.".into(),
            files: Vec::new(), branches: Vec::new(), commits: Vec::new(),
            draft: None, stage_result: None, stashes: Vec::new(),
        }
    }
}

/// The file a diff belongs to. Hunk actions use it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileTarget {
    pub path: NativePath,
    pub staged_view: bool,
}

/// Main-panel content: a diff, a commit detail or a plain note.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MainView {
    pub lines: Vec<Line>,
    /// Line index of each hunk header, in hunk order.
    pub hunks: Vec<usize>,
    pub file: Option<FileTarget>,
    pub committed: Option<CommittedFiles>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommittedFile {
    pub status: String,
    pub old_mode: String,
    pub new_mode: String,
    pub path: NativePath,
    pub old_path: Option<NativePath>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommittedFiles {
    pub id: String,
    pub base: String,
    pub files: Vec<CommittedFile>,
}

impl MainView {
    #[must_use]
    pub fn note(text: &str) -> Self {
        Self { lines: vec![Line { tone: Tone::Muted, text: text.to_owned() }], hunks: Vec::new(), file: None, committed: None }
    }

    #[must_use]
    pub fn text(text: &str) -> Self {
        Self {
            lines: text.lines().map(|line| Line { tone: Tone::Text, text: line.to_owned() }).collect(),
            hunks: Vec::new(),
            file: None,
            committed: None,
        }
    }

    #[must_use]
    pub fn diff(text: &str, file: FileTarget) -> Self {
        let lines = diff_lines(text);
        let hunks =
            lines.iter().enumerate().filter(|(_, line)| line.tone == Tone::Hunk).map(|(index, _)| index).collect();
        Self { lines, hunks, file: Some(file), committed: None }
    }
}

/// What the main panel should show for the focused list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MainRequest {
    CommitFiles { id: String },
    CommitFile { id: String, file: CommittedFile },
    Stash { id: String },
    File { path: NativePath, staged_view: bool, untracked: bool },
    Commit { id: String },
}

/// A unit of Git work requested by the window. Jobs run in order on one
/// backend thread, so a slow Git call never blocks the window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Job {
    EnterRepository { path: Option<NativePath>, current: Option<(NativePath, Box<crate::Navigation>)> },
    LoadCommitReview { generation: u64 },
    CommitDiff { ticket: String, generation: u64, index: usize },
    LoadIdentity,
    SaveIdentity { ticket: String, name: String, email: String },
    LoadGitSettings,
    ConfigureGit(Option<NativePath>),
    PrepareCreate { path: NativePath, branch: String },
    ExecuteCreate(String),
    LoadWelcome,
    ForgetRecent { backend: String, path: EncodedPath },
    GrantRepositoryTrust(String),
    CancelEntry,
    OpenRepository(NativePath),
    StashAction { id: String, expected: Vec<String>, operation: String },
    /// Prepare a target-bound, expiring confirmation on the worker.
    Prepare { generation: u64, action: Box<Job> },
    ExecutePrepared(String),
    CancelPrepared,
    Reload,
    /// `generation` lets the window drop a main-panel reply that was overtaken
    /// by a newer selection.
    Main {
        generation: u64,
        request: MainRequest,
    },
    /// Stages (`stage == true`) or unstages the whole path.
    SetFile {
        path: NativePath,
        stage: bool,
    },
    /// Stages the hunk when `staged_view` is false and unstages it otherwise.
    /// `number` starts at 1.
    SetHunk {
        path: NativePath,
        staged_view: bool,
        number: usize,
    },
    /// Stages or unstages the changed lines `lines` of one hunk. `number` starts at
    /// 1; a line index counts from 0 at the hunk's `@@` line.
    SetLines {
        path: NativePath,
        staged_view: bool,
        number: usize,
        lines: Vec<usize>,
    },
    /// Stages every changed path, or removes every path from the index.
    SetAll {
        stage: bool,
    },
    Switch(String),
    CreateBranch(String),
    /// Deletes a local branch that Git considers merged.
    DeleteBranch(String),
    /// Throws away the working-tree changes of one path. `untracked` selects
    /// removal of the file instead of a restore from the index.
    Discard {
        path: NativePath,
        untracked: bool,
    },
    /// Throws away one unstaged hunk of a tracked path.
    DiscardHunk {
        path: NativePath,
        index: usize,
    },
    /// Saves tracked changes as a stash.
    Stash,
    /// Checks out a commit by its full id, leaving HEAD detached.
    Detach(String),
    Commit { ticket: String, message: String },
    /// Replaces the saved draft; an empty text clears it.
    SaveDraft(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reply {
    RepositoryClosed(Vec<RecentRow>),
    CommitReview { generation: u64, review: CommitReview },
    CommitDiff { generation: u64, view: MainView },
    IdentitySettings { ticket: String, name: String, email: String, detail: String },
    IdentitySaved { path: NativePath, review: Option<TrustReview>, detail: String },
    GitSettings { path: Option<NativePath>, detail: String },
    CreateReview { ticket: String, path: NativePath, branch: String, new_directory: bool, entries: Vec<String>, more_entries: bool },
    Welcome(Vec<RecentRow>),
    TrustReview(TrustReview),
    RepositoryOpened(Snapshot),
    Committed(Snapshot),
    BranchCreated(Snapshot),
    DraftSaved(Option<String>),
    PromptFailed { action: PromptAction, text: String },
    /// The Git write completed, but the following snapshot is unavailable.
    PromptCompleted { action: PromptAction, text: String },
    NoChange,
    MainFailed { generation: u64, text: String },
    ConfirmationFailed { generation: u64, text: String },
    Prepared { generation: u64, ticket: String, lines: Vec<String> },
    /// The lists changed; the window reloads the main panel for its focus.
    Full(Snapshot),
    Main {
        generation: u64,
        view: MainView,
    },
    /// An action was refused. The text is shown to the user as is.
    Notice(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedRow {
    pub path: NativePath,
    pub change: String,
    pub added: Option<u64>,
    pub deleted: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitReview {
    pub ticket: String,
    pub head: String,
    pub files: Vec<StagedRow>,
    pub blocked: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptAction {
    IdentitySettings,
    GitSettings,
    CreateRepository,
    OpenRepository,
    Commit,
    CreateBranch,
    SaveDraft,
}

/// Runs jobs on behalf of the window. It is moved to a worker thread, so it
/// owns whatever state it needs. Each call re-checks trust and reads fresh data.
pub trait Backend: Send + 'static {
    fn run(&mut self, job: Job) -> Reply;
}

/// Splits diff text into tone-coded lines.
#[must_use]
pub fn diff_lines(text: &str) -> Vec<Line> {
    text.lines()
        .map(|line| {
            let tone = if line.starts_with("+++") || line.starts_with("---") {
                Tone::Muted
            } else if line.starts_with('+') {
                Tone::Added
            } else if line.starts_with('-') {
                Tone::Removed
            } else if line.starts_with("@@") {
                Tone::Hunk
            } else {
                Tone::Text
            };
            Line { tone, text: line.to_owned() }
        })
        .collect()
}

fn head_text(head: &HeadState) -> String {
    match head {
        HeadState::Branch { name, revision: Some(revision) } => {
            format!("On branch {name} ({})", short(revision.token()))
        }
        HeadState::Branch { name, revision: None } => format!("On branch {name} (no commits yet)"),
        HeadState::Detached { revision } => format!("HEAD detached at {}", short(revision.token())),
    }
}

fn tone_of(staged: ChangeKind, unstaged: ChangeKind) -> Tone {
    if unstaged == ChangeKind::Untracked {
        Tone::Untracked
    } else if staged != ChangeKind::Unmodified {
        Tone::Staged
    } else {
        Tone::Changed
    }
}

/// Abbreviated revision id for display. Full ids stay in the model.
#[must_use]
pub fn short(token: &str) -> &str {
    &token[..token.len().min(12)]
}

fn code(kind: ChangeKind) -> char {
    match kind {
        ChangeKind::Unmodified => '.',
        ChangeKind::Modified => 'M',
        ChangeKind::TypeChanged => 'T',
        ChangeKind::Added => 'A',
        ChangeKind::Deleted => 'D',
        ChangeKind::Renamed => 'R',
        ChangeKind::Copied => 'C',
        ChangeKind::Unmerged => 'U',
        ChangeKind::Untracked => '?',
        ChangeKind::Ignored => '!',
    }
}

//! Native workspace window for MiniCon SrcMgr. The window is a thin shell over
//! plain data (`model`) and a pure state machine (`workspace`); Git runs behind
//! the [`Backend`] trait on a worker thread.

mod model;
mod messages;
mod entry_input;
mod tools;
mod clone_form;
mod locate;
mod remotes_panel;
pub use clone_form::{CloneField, CloneForm};
pub use locate::{LocateForm, LocateWork};
pub use remotes_panel::{RemotesAction, RemotesField, RemotesPanel};
pub use tools::{ToolForm, ToolWork};
mod filter;
mod navigation;
pub use navigation::{Navigation, NavigationSelection, NavigationFrame};
pub use model::NavigationIssue;
mod window;
mod workspace;
mod timing_receipt;

pub use messages::{Key as TextKey, Locale, text as text_of};
pub use model::{
    Backend, BranchRow, CommitRow, FileRow, FileTarget, Job, Line, MainRequest, MainView, RemoteEdit, RemoteRow, Reply,
    Snapshot, Tone,
    diff_lines,
};
pub use window::{show, show_entry, show_with_timing};
pub use model::StashRow;
pub use model::{CommittedFile, CommittedFiles};
pub use model::PromptAction;
pub use model::{RecentRow, TrustReview};
pub use model::{CommitReview, StagedRow};
pub use workspace::{Effect, Key, Pane, Prompt, PromptKind, Workspace};
pub use workspace::ChangesAction;

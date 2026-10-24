//! Native workspace window for MiniCon SrcMgr. The window is a thin shell over
//! plain data (`model`) and a pure state machine (`workspace`); Git runs behind
//! the [`Backend`] trait on a worker thread.

mod model;
mod entry_input;
mod tools;
mod locate;
pub use locate::{LocateForm, LocateWork};
pub use tools::{ToolForm, ToolWork};
mod filter;
mod navigation;
pub use navigation::{Navigation, NavigationSelection, NavigationFrame};
pub use model::NavigationIssue;
mod window;
mod workspace;
mod timing_receipt;

pub use model::{
    Backend, BranchRow, CommitRow, FileRow, FileTarget, Job, Line, MainRequest, MainView, Reply, Snapshot, Tone,
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

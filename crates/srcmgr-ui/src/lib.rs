//! Native workspace window for MiniCon SrcMgr. The window is a thin shell over
//! plain data (`model`) and a pure state machine (`workspace`); Git runs behind
//! the [`Backend`] trait on a worker thread.

mod model;
mod window;
mod workspace;

pub use model::{
    Backend, BranchRow, CommitRow, FileRow, FileTarget, Job, Line, MainRequest, MainView, Reply, Snapshot, Tone,
    diff_lines,
};
pub use window::show;
pub use workspace::{Effect, Key, Pane, Prompt, PromptKind, Workspace};

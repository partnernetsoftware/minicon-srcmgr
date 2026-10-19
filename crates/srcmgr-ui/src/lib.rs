//! MiniCon SrcMgr desktop surface (PRD 10, M0-D first slice).
//!
//! The view model is host-neutral and tested directly. The window module runs
//! MiniCon's native pixel host; it is the only part that needs a desktop.

mod view;
mod window;

pub use view::{Line, StatusView, Tone};
pub use window::show;

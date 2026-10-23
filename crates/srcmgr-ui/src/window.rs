//! Native pixel window for the status workspace. The event loop, surface and
//! presentation belong to MiniCon's platform layer; this module supplies the
//! application: a panel layout built from plain drawing commands, glyph masks
//! from the platform font composited with MiniCon's pixel kernels, and a worker
//! thread that runs Git so the window never waits on it.

use std::sync::Arc;
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;
use minicon_core::composer::{self, Move};

use minicon_platform::contract::font::RasterGlyph;
use minicon_platform::contract::input::{KeyPressState, LogicalKey, NamedKey, NormalizedKeyEvent};
use minicon_platform::font;
use minicon_platform::window_host::{
    LogicalSize, PixelWindow, PixelWindowApplication, PixelWindowDirective, PixelWindowError, PixelWindowEvent,
    PixelWindowOptions, PointerButton, PointerButtonState, WheelDelta, XrgbPixelFrame, run_pixel_window,
};
use minicon_ui_core::GlyphCache;
use minicon_ui_core::pixel::{blend_mask_xrgb, fill_xrgb_rect};

use crate::model::{Backend, Job, Line, PromptAction, Reply, Snapshot, Tone, short};
use crate::workspace::{CommitFocus, Confirm, Effect, Key, Pane, PromptKind, Workspace};

enum ClipboardRequest { Read, Write { text: String, cut: bool } }
enum ClipboardTask {
    Read { operation: minicon_platform::clipboard::ClipboardTextRead, id: u64 },
    Write { receiver: Receiver<Result<(), String>>, id: u64, cut: Option<String> },
}
use crate::timing_receipt::{Category, TimingReceipt};

const BASE_SIZE_PX: f64 = 16.0;
const MARGIN_PX: f64 = 12.0;
const GLYPH_CACHE_ENTRIES: usize = 512;
/// Rows scrolled per wheel notch.
const WHEEL_LINES: f64 = 3.0;
const BACKGROUND: u32 = 0x001E_1E1E;
const BORDER: u32 = 0x003C_3C3C;
const FOCUS_BORDER: u32 = 0x0056_9CD6;
const SELECTION: u32 = 0x0026_4F78;
const SELECTION_DIM: u32 = 0x0033_3740;
const MUTED_TEXT: u32 = 0x0080_8080;

const HELP_LINES: &[&str] = &[
    "/            filter loaded Files, Branches or Commits; Esc clears",
    "Ctrl+Tab     next repository; Ctrl+Shift+Tab previous",
    "Ctrl+W       close active repository after saving context",
    "O            open another trusted repository (or click Open)",
    "1 2 3 4      focus Status, Files, Branches or Commits",
    "Tab          next pane (Shift+Tab previous)",
    "Up/Down j/k  move in a list, or the cursor in the main panel",
    "PgUp/PgDn    page (K / J too); Ctrl+U / Ctrl+D half page",
    "< and >      first and last row; Home and End do the same",
    "Enter        move into the main panel; Esc goes back",
    "Space        stage or unstage a file; checkout a branch",
    "             main panel: stage or unstage the cursor line",
    "v            main panel: start or end a range of lines",
    "a            stage or unstage all (Files); stage the hunk",
    "h / l        previous or next hunk in the main panel",
    "c            commit: Ctrl+Enter commits, Esc keeps the draft",
    "n            new branch (Branches panel)",
    "d            delete a branch (Branches); discard a file or hunk (Files, diff); asks first",
    "s / S        stash tracked changes / open stash list (Files)",
    "5            stash list: Space apply/keep, p pop, d drop; asks first",
    "Space        in Commits: check out that commit detached; asks first",
    "r            reload;  ?  this help;  q or Ctrl+C quits",
];

fn colour(tone: Tone) -> u32 {
    match tone {
        Tone::Heading => 0x0056_9CD6,
        Tone::Text => 0x00D4_D4D4,
        Tone::Muted => MUTED_TEXT,
        Tone::Staged => 0x00B5_CEA8,
        Tone::Changed => 0x00DC_DCAA,
        Tone::Untracked => 0x0073_C991,
        Tone::Added => 0x0068_D391,
        Tone::Removed => 0x00F4_8771,
        Tone::Hunk => 0x0056_9CD6,
    }
}

/// Opens the workspace window and blocks until the user quits. Git work runs on
/// a worker thread that owns `backend`; the window wakes when a reply arrives.
pub fn show(snapshot: Snapshot, backend: Box<dyn Backend>) -> Result<(), PixelWindowError> {
    show_with_timing(snapshot, backend, None)
}

pub fn show_with_timing(snapshot: Snapshot, backend: Box<dyn Backend>, timing: Option<std::fs::File>) -> Result<(), PixelWindowError> {
    let options = PixelWindowOptions::new(snapshot.title.clone(), LogicalSize { width: 1040.0, height: 680.0 });
    let mut application = StatusWindow::new(snapshot, backend);
    application.timing = timing.map(TimingReceipt::new);
    run_pixel_window(options, Box::new(application))
}

pub fn show_entry(initial_path: Option<srcmgr_domain::NativePath>, backend: Box<dyn Backend>, timing: Option<std::fs::File>) -> Result<(), PixelWindowError> {
    let snapshot = Snapshot::welcome();
    let options = PixelWindowOptions::new(snapshot.title.clone(), LogicalSize { width: 1040.0, height: 680.0 });
    let mut application = StatusWindow::new(snapshot, backend);
    application.initial_path = initial_path;
    application.timing = timing.map(TimingReceipt::new);
    run_pixel_window(options, Box::new(application))
}

struct StatusWindow {
    workspace: Workspace,
    backend: Option<Box<dyn Backend>>,
    jobs: Option<Sender<Job>>,
    replies: Option<Receiver<Reply>>,
    /// Jobs sent whose replies have not arrived; shown as "working".
    pending: usize,
    /// Only one job reaches the worker at a time. Superseded detail reads
    /// collapse here instead of spawning Git for every intermediate selection.
    queued: VecDeque<Job>,
    glyphs: GlyphCache<(char, u16), Arc<Option<RasterGlyph>>>,
    /// The last pointer position in logical pixels, for events that omit one.
    pointer: Option<(f64, f64)>,
    /// The size and grid of the last frame, so pointer positions can be mapped to panels.
    surface: Option<Surface>,
    timing: Option<TimingReceipt>,
    native_title: String,
    initial_path: Option<srcmgr_domain::NativePath>,
    clipboard_request: Option<ClipboardRequest>,
    clipboard_task: Option<ClipboardTask>,
    ime_allowed: bool,
    ime_position_failed: bool,
}

impl StatusWindow {
    fn new(snapshot: Snapshot, backend: Box<dyn Backend>) -> Self {
        let native_title = snapshot.title.clone();
        Self {
            workspace: Workspace::new(snapshot),
            backend: Some(backend),
            jobs: None,
            replies: None,
            pending: 0,
            queued: VecDeque::new(),
            glyphs: GlyphCache::with_capacity(GLYPH_CACHE_ENTRIES),
            pointer: None,
            surface: None,
            timing: None,
            native_title,
            initial_path: None,
            clipboard_request: None,
            clipboard_task: None,
            ime_allowed: false,
            ime_position_failed: false,
        }
    }

    fn sync_title(&mut self, window: &PixelWindow) {
        if let Some(issue) = self.workspace.navigation_issue() {
            let title = format!("MiniCon SrcMgr - Saved view recovery | {} | {}", if issue.submitted { "Working" } else { "Ready" }, issue.path.display_label());
            if self.native_title != title { window.set_title(&title); self.native_title = title; }
            return;
        }
        if let Some(form) = self.workspace.filter_editor() {
            let title = format!("MiniCon SrcMgr - Filter | {} | {} matches", pane_label(form.pane), self.workspace.visible_rows(form.pane).len());
            if self.native_title != title { window.set_title(&title); self.native_title = title; }
            return;
        }
        if let Some(form) = self.workspace.commit_composer() {
            let title = if self.workspace.prompt().is_some_and(|prompt| prompt.pending()) { "MiniCon SrcMgr - Commit | Working" } else if form.review.as_ref().is_some_and(|review| review.blocked.is_some()) { "MiniCon SrcMgr - Commit | Review blocked" } else if form.review.is_some() && form.diff_ready { "MiniCon SrcMgr - Commit | Review ready" } else { "MiniCon SrcMgr - Commit | Loading review" };
            if self.native_title != title { window.set_title(title); self.native_title = title.into(); }
        } else if self.workspace.identity_form().is_some() {
            let title = if self.workspace.identity_form().is_some_and(|form| form.submitted) { "MiniCon SrcMgr - Identity settings | Working" } else { "MiniCon SrcMgr - Identity settings | Ready" };
            if self.native_title != title { window.set_title(title); self.native_title = title.into(); }
        } else if let Some(form) = self.workspace.create_form() {
            let title = if form.review.is_some() { format!("MiniCon SrcMgr - Create review | {}", srcmgr_domain::NativePath::new(form.path.clone()).display_label()) } else { "MiniCon SrcMgr - Create repository".into() };
            if self.native_title != title { window.set_title(&title); self.native_title = title; }
        } else if self.workspace.prompt().is_some_and(|prompt| prompt.kind() == PromptKind::GitSettings) {
            let title = if self.workspace.prompt().is_some_and(|prompt| prompt.pending()) { "MiniCon SrcMgr - Git settings | Working" } else if self.workspace.git_detail().starts_with("Saved and applied.") { "MiniCon SrcMgr - Git settings | Saved" } else { "MiniCon SrcMgr - Git settings | Ready" };
            if self.native_title != title { window.set_title(title); self.native_title = title.into(); }
        } else if let Some(review) = self.workspace.trust_review() {
            let title = format!("MiniCon SrcMgr - Trust review | {}", review.worktree.display_label());
            if self.native_title != title { window.set_title(&title); self.native_title = title; }
        } else if let Some((kind, id, ready)) = self.workspace.history_context() {
            let path = self.workspace.snapshot().repository.as_ref().map(|p| p.display_label()).unwrap_or_default();
            let title = format!("MiniCon SrcMgr - History | {kind} | {} | {} | {path}", if ready { "Ready" } else { "Loading" }, short(id));
            if self.native_title != title { window.set_title(&title); self.native_title = title; }
        } else if self.native_title != self.workspace.snapshot().title {
            self.native_title.clone_from(&self.workspace.snapshot().title);
            window.set_title(&self.native_title);
        }
    }

    fn service_clipboard(&mut self, window: &PixelWindow) {
        let completion = match self.clipboard_task.as_ref() {
            Some(ClipboardTask::Read { operation, id }) => match operation.try_poll() {
                minicon_platform::clipboard::ClipboardTextReadPoll::Pending => None,
                minicon_platform::clipboard::ClipboardTextReadPoll::Ready(result) => Some((*id, result.map(Some).map_err(|e| e.to_string()), None)),
            },
            Some(ClipboardTask::Write { receiver, id, cut }) => match receiver.try_recv() {
                Ok(result) => Some((*id, result.map(|_| None), cut.clone())),
                Err(mpsc::TryRecvError::Disconnected) => Some((*id, Err("Clipboard write worker disconnected".into()), None)),
                Err(mpsc::TryRecvError::Empty) => None,
            },
            None => None,
        };
        if let Some((id, result, cut)) = completion {
            self.clipboard_task = None;
            match result {
                Ok(text) => { let effect = self.workspace.complete_clipboard(id, text, cut); self.dispatch(effect); }
                Err(error) => { let _ = self.workspace.reply(Reply::Notice(format!("Clipboard unavailable; text was retained: {error}"))); }
            }
            window.request_redraw();
        }
        let Some(request) = self.clipboard_request.take() else { return; };
        if self.clipboard_task.is_some() { let _ = self.workspace.reply(Reply::Notice("Clipboard work is already pending".into())); return; }
        let Some(id) = self.workspace.clipboard_context() else { return; };
        let wake = window.waker();
        match request {
            ClipboardRequest::Read => match minicon_platform::clipboard::read_text_async(composer::PASTE_LIMIT_BYTES, move || { let _ = wake.wake(); }) {
                Ok(operation) => self.clipboard_task = Some(ClipboardTask::Read { operation, id }),
                Err(error) => { let _ = self.workspace.reply(Reply::Notice(format!("Clipboard unavailable: {error}"))); }
            },
            ClipboardRequest::Write { text, cut } => {
                let source = cut.then(|| text.clone());
                let (sender, receiver) = mpsc::sync_channel(1);
                match thread::Builder::new().name("srcmgr-clipboard-write".into()).spawn(move || {
                    let result = std::panic::catch_unwind(|| minicon_platform::clipboard::set_text(&text).map_err(|e| e.to_string())).unwrap_or_else(|_| Err("Clipboard write worker panicked".into()));
                    let _ = sender.send(result);
                    let _ = wake.wake();
                }) {
                    Ok(_) => self.clipboard_task = Some(ClipboardTask::Write { receiver, id, cut: source }),
                    Err(error) => { let _ = self.workspace.reply(Reply::Notice(format!("Clipboard worker unavailable; text retained: {error}"))); }
                }
            }
        }
    }

    /// Applies a pointer gesture at `point` (logical pixels). Ignored while a
    /// prompt or help is open, so a click cannot change the panel under the
    /// user's text.
    fn pointer_gesture(
        &mut self,
        window: &PixelWindow,
        point: Option<(f64, f64)>,
        gesture: Gesture,
    ) -> Result<PixelWindowDirective, PixelWindowError> {
        let Some((x, y)) = point.or(self.pointer) else {
            return Ok(PixelWindowDirective::Continue);
        };
        let Some(surface) = self.surface else {
            return Ok(PixelWindowDirective::Continue);
        };
        let scale = window.scale_factor()?;
        let px = (x * scale).round() as i64;
        let py = (y * scale).round() as i64;
        if matches!(gesture, Gesture::Click) && let Some(hit) = tab_hit(&self.workspace, surface, px, py) {
            let effect = match hit { Hit::Repository(index) => self.workspace.select_repository(index), Hit::CloseRepository => self.workspace.close_repository(), Hit::NextRepository => self.workspace.key(Key::NextRepository), Hit::PreviousRepository => self.workspace.key(Key::PreviousRepository), _ => Effect::Nothing };
            let directive = self.dispatch(effect);
            self.sync_title(window);
            window.request_redraw();
            return Ok(directive);
        }
        if self.workspace.navigation_issue().is_some() {
            let (allow, cancel) = trust_buttons(layout(surface.width, surface.height, surface.grid).main, surface.grid);
            let effect = match gesture { Gesture::Click if contains(allow, px, py) => self.workspace.answer_navigation(true), Gesture::Click if contains(cancel, px, py) => self.workspace.answer_navigation(false), _ => Effect::Nothing };
            let directive = self.dispatch(effect);
            self.sync_title(window);
            window.request_redraw();
            return Ok(directive);
        }
        if self.workspace.commit_composer().is_some() {
            let scale = window.scale_factor()?;
            let geometry = commit_geometry(surface.width, surface.height, surface.grid);
            let px = (x * scale).round() as i64;
            let py = (y * scale).round() as i64;
            let effect = match gesture {
                Gesture::Click if contains(geometry.commit, px, py) => self.workspace.key(Key::CtrlEnter),
                Gesture::Click if contains(geometry.refresh, px, py) => self.workspace.refresh_commit_review(),
                Gesture::Click if contains(geometry.close, px, py) => self.workspace.close_commit(false),
                Gesture::Click if contains(geometry.index, px, py) => {
                    self.workspace.focus_commit(CommitFocus::Index);
                    let inner = inner_of(geometry.index, surface.grid);
                    let form = self.workspace.commit_composer().unwrap();
                    let length = form.review.as_ref().map_or(0, |review| review.files.len());
                    let rows = (inner.rows - COMMIT_CONTEXT_ROWS).max(1);
                    let offset = window_start(form.selected, length, rows);
                    let first = inner.top + COMMIT_CONTEXT_ROWS * surface.grid.row;
                    if py < first { Effect::Nothing } else { self.workspace.select_staged(offset + ((py - first) / surface.grid.row) as usize) }
                }
                Gesture::Click if contains(geometry.message, px, py) => {
                    let inner = inner_of(geometry.message, surface.grid);
                    self.workspace.click_commit_caret(((py - inner.top).max(0) / surface.grid.row) as usize, (((px - inner.x).max(0) as f32) / surface.grid.advance) as usize, inner.rows.max(1) as usize, inner.cols)
                }
                Gesture::Click if contains(geometry.diff, px, py) => self.workspace.focus_commit(CommitFocus::Diff),
                Gesture::Wheel(delta) if contains(geometry.diff, px, py) => self.workspace.scroll_commit_diff(wheel_lines(delta, surface.grid.row, scale)),
                Gesture::Wheel(delta) if contains(geometry.index, px, py) => {
                    let form = self.workspace.commit_composer().unwrap();
                    let index = form.selected.saturating_add_signed(wheel_lines(delta, surface.grid.row, scale));
                    self.workspace.select_staged(index.min(form.review.as_ref().map_or(0, |review| review.files.len().saturating_sub(1))))
                }
                _ => Effect::Nothing,
            };
            let directive = self.dispatch(effect);
            self.sync_title(window);
            window.request_redraw();
            return Ok(directive);
        }
        if self.workspace.identity_form().is_some() {
            let scale = window.scale_factor()?;
            let main = layout(surface.width, surface.height, surface.grid).main;
            let (save, cancel) = trust_buttons(main, surface.grid);
            let (name, email) = create_fields(main, surface.grid);
            let px = (x * scale).round() as i64;
            let py = (y * scale).round() as i64;
            let effect = match gesture {
                Gesture::Click if contains(save, px, py) => self.workspace.key(Key::Enter),
                Gesture::Click if contains(cancel, px, py) => self.workspace.key(Key::Escape),
                Gesture::Click if contains(name, px, py) => self.workspace.focus_identity_field(false),
                Gesture::Click if contains(email, px, py) => self.workspace.focus_identity_field(true),
                Gesture::Wheel(delta) => self.workspace.scroll_trust(wheel_lines(delta, surface.grid.row, scale)),
                _ => Effect::Nothing,
            };
            let directive = self.dispatch(effect);
            self.sync_title(window);
            window.request_redraw();
            return Ok(directive);
        }
        if self.workspace.create_form().is_some() {
            let scale = window.scale_factor()?;
            let main = layout(surface.width, surface.height, surface.grid).main;
            let px = (x * scale).round() as i64;
            let py = (y * scale).round() as i64;
            let (allow, cancel) = trust_buttons(main, surface.grid);
            let reviewing = self.workspace.create_form().is_some_and(|form| form.review.is_some());
            let (path, branch) = create_fields(main, surface.grid);
            let effect = match gesture {
                Gesture::Click if contains(allow, px, py) && reviewing => self.workspace.answer_create(true),
                Gesture::Click if contains(cancel, px, py) && reviewing => self.workspace.answer_create(false),
                Gesture::Click if contains(allow, px, py) => self.workspace.key(Key::Enter),
                Gesture::Click if contains(cancel, px, py) => self.workspace.key(Key::Escape),
                Gesture::Click if contains(path, px, py) => self.workspace.focus_create_field(false),
                Gesture::Click if contains(branch, px, py) => self.workspace.focus_create_field(true),
                Gesture::Wheel(delta) if reviewing => self.workspace.scroll_trust(wheel_lines(delta, surface.grid.row, scale)),
                _ => Effect::Nothing,
            };
            let directive = self.dispatch(effect);
            self.sync_title(window);
            window.request_redraw();
            return Ok(directive);
        }
        if self.workspace.trust_review().is_some() {
            let scale = window.scale_factor()?;
            let main = layout(surface.width, surface.height, surface.grid).main;
            let (allow, cancel) = trust_buttons(main, surface.grid);
            let px = (x * scale).round() as i64;
            let py = (y * scale).round() as i64;
            let effect = match gesture {
                Gesture::Click if contains(allow, px, py) => self.workspace.answer_trust(true),
                Gesture::Click if contains(cancel, px, py) => self.workspace.answer_trust(false),
                Gesture::Wheel(delta) => self.workspace.scroll_trust(wheel_lines(delta, surface.grid.row, scale)),
                _ => Effect::Nothing,
            };
            let directive = self.dispatch(effect);
            self.sync_title(window);
            window.request_redraw();
            return Ok(directive);
        }
        if self.workspace.prompt().is_some_and(|prompt| prompt.kind() == PromptKind::GitSettings) {
            let scale = window.scale_factor()?;
            let main = layout(surface.width, surface.height, surface.grid).main;
            let (save, cancel) = trust_buttons(main, surface.grid);
            let px = (x * scale).round() as i64;
            let py = (y * scale).round() as i64;
            let effect = match gesture {
                Gesture::Click if contains(save, px, py) => self.workspace.key(Key::Enter),
                Gesture::Click if contains(cancel, px, py) => self.workspace.key(Key::Escape),
                _ => Effect::Nothing,
            };
            let directive = self.dispatch(effect);
            self.sync_title(window);
            window.request_redraw();
            return Ok(directive);
        }
        if self.workspace.filter_editor().is_some() {
            let hit = hit_test(&self.workspace, surface, px, py);
            let effect = match (gesture, hit) {
                (Gesture::Click, Hit::ClearFilter(pane)) => self.workspace.clear_filter(pane),
                (Gesture::Click, Hit::Filter(pane)) => { self.workspace.finish_filter(true, false); self.workspace.request_filter(pane) }
                (Gesture::Click, Hit::Row(pane, index)) => { self.workspace.finish_filter(true, false); if self.workspace.filter_editor().is_none() { self.workspace.click_row(pane, index) } else { Effect::Nothing } }
                (Gesture::Click, Hit::Panel(pane)) => { self.workspace.finish_filter(true, false); if self.workspace.filter_editor().is_none() { self.workspace.click_row(pane, usize::MAX) } else { Effect::Nothing } }
                (Gesture::Click, Hit::Main(_)) => {
                    let inner = inner_of(filter_field(layout(surface.width, surface.height, surface.grid).main, surface.grid), surface.grid);
                    self.workspace.click_filter_caret(((px - inner.x).max(0) as f32 / surface.grid.advance) as usize, inner.cols)
                }
                (Gesture::Wheel(delta), Hit::Row(pane, _) | Hit::Panel(pane)) => {
                    let current = self.workspace.selected(pane);
                    let next = current.saturating_add_signed(wheel_lines(delta, surface.grid.row, scale)).min(self.workspace.visible_rows(pane).len().saturating_sub(1));
                    self.workspace.preview_filter_row(pane, next)
                }
                _ => Effect::Nothing,
            };
            let directive = self.dispatch(effect);
            self.sync_title(window);
            window.request_redraw();
            return Ok(directive);
        }
        if self.workspace.modal() {
            return Ok(PixelWindowDirective::Continue);
        }
        let scale = window.scale_factor()?;
        let hit = hit_test(&self.workspace, surface, (x * scale).round() as i64, (y * scale).round() as i64);
        let effect = match (gesture, hit) {
            (Gesture::Click, Hit::Recent(index)) => self.workspace.select_recent(index),
            (Gesture::Click, Hit::RecentOpen) => self.workspace.open_recent(),
            (Gesture::Click, Hit::Open) => self.workspace.request_open(),
            (Gesture::Click, Hit::Create) => self.workspace.request_create(),
            (Gesture::Click, Hit::GitSettings) => self.workspace.request_git_settings(),
            (Gesture::Click, Hit::IdentitySettings) => self.workspace.request_identity(),
            (Gesture::Click, Hit::Commit) => self.workspace.request_commit(),
            (Gesture::Click, Hit::Back) => self.workspace.key(Key::Escape),
            (Gesture::Click, Hit::Filter(pane)) => self.workspace.request_filter(pane),
            (Gesture::Click, Hit::ClearFilter(pane)) => self.workspace.clear_filter(pane),
            (Gesture::Click, Hit::Repository(index)) => self.workspace.select_repository(index),
            (Gesture::Click, Hit::CloseRepository) => self.workspace.close_repository(),
            (Gesture::Click, Hit::NextRepository) => self.workspace.key(Key::NextRepository),
            (Gesture::Click, Hit::PreviousRepository) => self.workspace.key(Key::PreviousRepository),
            (Gesture::Click, Hit::Panel(pane)) => self.workspace.click_row(pane, usize::MAX),
            (Gesture::Click, Hit::Row(pane, index)) => self.workspace.click_row(pane, index),
            (Gesture::Click, Hit::Main(line)) => self.workspace.click_main(line),
            (Gesture::Click, Hit::Nothing) => Effect::Nothing,
            (Gesture::Wheel(delta), hit) => match pane_of(hit) {
                Some(pane) => self.workspace.wheel(pane, wheel_lines(delta, surface.grid.row, scale)),
                None => Effect::Nothing,
            },
        };
        let directive = self.dispatch(effect);
        self.sync_title(window);
        window.request_redraw();
        Ok(directive)
    }

    fn start_worker(&mut self, window: &PixelWindow) -> Result<(), PixelWindowError> {
        let Some(mut backend) = self.backend.take() else {
            return Ok(());
        };
        let (job_sender, job_receiver) = mpsc::channel::<Job>();
        let (reply_sender, reply_receiver) = mpsc::channel::<Reply>();
        let waker = window.waker();
        thread::Builder::new()
            .name("srcmgr-git".to_owned())
            .spawn(move || {
                for job in job_receiver {
                    let reply = backend.run(job);
                    if reply_sender.send(reply).is_err() || waker.wake().is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| PixelWindowError::failed("status_worker_spawn", error))?;
        self.jobs = Some(job_sender);
        self.replies = Some(reply_receiver);
        Ok(())
    }

    /// Runs an effect. Jobs go to the worker; quitting ends the event loop.
    fn dispatch(&mut self, effect: Effect) -> PixelWindowDirective {
        match effect {
            Effect::Copy { text, cut } => { self.clipboard_request = Some(ClipboardRequest::Write { text, cut }); PixelWindowDirective::Continue }
            Effect::Paste => { self.clipboard_request = Some(ClipboardRequest::Read); PixelWindowDirective::Continue }
            Effect::Nothing => PixelWindowDirective::Continue,
            Effect::Quit if self.pending != 0 || !self.queued.is_empty() => { let _ = self.workspace.reply(Reply::Notice("Wait for current repository work before closing".into())); PixelWindowDirective::Continue }
            Effect::Quit => PixelWindowDirective::Exit,
            Effect::Send(job) => {
                if matches!(job, Job::ResetNavigation(_) | Job::EnterRepository { .. } | Job::LoadIdentity | Job::SaveIdentity { .. } | Job::ConfigureGit(_) | Job::OpenRepository(_) | Job::GrantRepositoryTrust(_) | Job::PrepareCreate { .. } | Job::ExecuteCreate(_)) && (self.pending != 0 || !self.queued.is_empty()) {
                    let _ = self.workspace.reply(job_failure(&job, "Wait for current repository work to finish, then retry entry"));
                    return PixelWindowDirective::Continue;
                }
                if (matches!(job, Job::Main { .. }) && self.queued.back().is_some_and(|last| matches!(last, Job::Main { .. })))
                    || (matches!(job, Job::CommitDiff { .. }) && self.queued.back().is_some_and(|last| matches!(last, Job::CommitDiff { .. })))
                    || (matches!(job, Job::CancelPrepared) && self.queued.back() == Some(&Job::CancelPrepared)) {
                    *self.queued.back_mut().unwrap() = job;
                } else if self.queued.len() < 16 || matches!(job, Job::CancelPrepared | Job::CancelEntry) {
                    self.queued.push_back(job);
                } else {
                    let reply = job_failure(&job, "Work queue is full; wait for the current work, then retry");
                    let _ = self.workspace.reply(reply);
                }
                self.pump();
                PixelWindowDirective::Continue
            }
        }
    }

    fn pump(&mut self) {
        if self.pending != 0 { return; }
        if let Some(job) = self.queued.pop_front() {
            let sent = if let Some(jobs) = &self.jobs { jobs.send(job).map_err(|error| error.0) } else { Err(job) };
            if let Err(job) = sent {
                let _ = self.workspace.reply(job_failure(&job, "the repository worker has stopped"));
                for waiting in self.queued.drain(..) {
                    let _ = self.workspace.reply(job_failure(&waiting, "the repository worker has stopped"));
                }
            } else {
                self.pending = 1;
            }
        }
    }

    fn glyph(&mut self, character: char, size: u16) -> Result<Arc<Option<RasterGlyph>>, PixelWindowError> {
        if let Some(cached) = self.glyphs.get_cloned(&(character, size)) {
            return Ok(cached);
        }
        let rasterized = font::rasterize(character, size)
            .map_err(|error| PixelWindowError::failed("status_glyph_rasterize", format!("{error:?}")))?;
        let value = Arc::new(rasterized);
        self.glyphs.insert((character, size), Arc::clone(&value));
        Ok(value)
    }

    /// Draws `text` with its pen starting at `at` on its baseline. Pixels
    /// outside the canvas are clipped rather than wrapped.
    fn draw_text(
        &mut self,
        canvas: &mut Canvas<'_>,
        at: Pen,
        text: &str,
        style: Style,
    ) -> Result<(), PixelWindowError> {
        let mut pen = at.x;
        for character in text.chars() {
            let glyph = self.glyph(character, style.size)?;
            if let Some(glyph) = glyph.as_ref() {
                let left = pen + i64::from(glyph.offset_x);
                // offset_y is measured from the top of the cell, which sits one ascent above the baseline.
                let top = at.baseline - style.ascent + i64::from(glyph.offset_y);
                blit_mask(canvas, glyph, left, top, style.colour);
            }
            pen += style.advance.round() as i64 * composer::character_cells(character) as i64;
        }
        Ok(())
    }
}

fn job_failure(job: &Job, message: &str) -> Reply {
    let text = message.to_owned();
    match job {
        Job::LoadIdentity | Job::SaveIdentity { .. } => Reply::PromptFailed { action: PromptAction::IdentitySettings, text },
        Job::LoadGitSettings | Job::ConfigureGit(_) => Reply::PromptFailed { action: PromptAction::GitSettings, text },
        Job::PrepareCreate { .. } | Job::ExecuteCreate(_) => Reply::PromptFailed { action: PromptAction::CreateRepository, text },
        Job::ResetNavigation(_) | Job::EnterRepository { .. } | Job::OpenRepository(_) | Job::GrantRepositoryTrust(_) => Reply::PromptFailed { action: PromptAction::OpenRepository, text },
        Job::Main { generation, .. } => Reply::MainFailed { generation: *generation, text },
        Job::Prepare { generation, .. } => Reply::ConfirmationFailed { generation: *generation, text },
        Job::Commit { .. } | Job::LoadCommitReview { .. } => Reply::PromptFailed { action: PromptAction::Commit, text },
        Job::CommitDiff { generation, .. } => Reply::MainFailed { generation: *generation, text },
        Job::CreateBranch(_) => Reply::PromptFailed { action: PromptAction::CreateBranch, text },
        Job::SaveDraft(_) => Reply::PromptFailed { action: PromptAction::SaveDraft, text },
        _ => Reply::Notice(text),
    }
}

/// A row-major XRGB target plus its bounds.
struct Canvas<'a> {
    pixels: &'a mut [u32],
    stride: u32,
    height: u32,
}

#[derive(Clone, Copy)]
struct Pen {
    x: i64,
    baseline: i64,
}

#[derive(Clone, Copy)]
struct Style {
    size: u16,
    colour: u32,
    advance: f32,
    ascent: i64,
}

fn fill(canvas: &mut Canvas<'_>, rect: Rect, colour: u32) {
    let left = rect.x.max(0);
    let top = rect.y.max(0);
    let right = (rect.x + rect.width).min(i64::from(canvas.stride));
    let bottom = (rect.y + rect.height).min(i64::from(canvas.height));
    if right <= left || bottom <= top {
        return;
    }
    fill_xrgb_rect(
        canvas.pixels,
        canvas.stride,
        left as u32,
        top as u32,
        (right - left) as u32,
        (bottom - top) as u32,
        colour,
    );
}

fn blit_mask(canvas: &mut Canvas<'_>, glyph: &RasterGlyph, left: i64, top: i64, foreground: u32) {
    let glyph_width = glyph.width as usize;
    for row in 0..glyph.height as i64 {
        let y = top + row;
        if y < 0 || y >= i64::from(canvas.height) {
            continue;
        }
        let first = (-left).max(0);
        let last = (i64::from(canvas.stride) - left).min(i64::from(glyph.width));
        if first >= last {
            continue;
        }
        let destination_start = y as usize * canvas.stride as usize + (left + first) as usize;
        let alpha_start = row as usize * glyph_width + first as usize;
        let count = (last - first) as usize;
        blend_mask_xrgb(
            &mut canvas.pixels[destination_start..destination_start + count],
            &glyph.alpha[alpha_start..alpha_start + count],
            foreground,
        );
    }
}

impl PixelWindowApplication for StatusWindow {
    fn opened(&mut self, window: &PixelWindow) -> Result<PixelWindowDirective, PixelWindowError> {
        self.start_worker(window)?;
        let effect = self.workspace.open();
        let directive = self.dispatch(effect);
        window.request_redraw();
        Ok(directive)
    }

    fn event(
        &mut self,
        window: &PixelWindow,
        event: PixelWindowEvent,
    ) -> Result<PixelWindowDirective, PixelWindowError> {
        let started = self.timing.as_ref().map(|_| Instant::now());
        let category = started.map(|_| match &event {
            PixelWindowEvent::Keyboard(key) if key_of(key).is_some() => if self.workspace.prompt().is_some() || self.workspace.filter_editor().is_some() { Category::Text } else { Category::Keyboard },
            PixelWindowEvent::PointerButton { .. } | PixelWindowEvent::MouseWheel { .. } => Category::Pointer,
            PixelWindowEvent::Wake => Category::Wake,
            PixelWindowEvent::GeometryChanged { .. } | PixelWindowEvent::ResizeInteractionEnded => Category::Layout,
            _ => Category::Other,
        });
        let result = (|| { match event {
            PixelWindowEvent::CloseRequested => { let effect = self.workspace.request_quit(); Ok(self.dispatch(effect)) }
            PixelWindowEvent::Wake => {
                let replies: Vec<Reply> =
                    self.replies.as_ref().map_or_else(Vec::new, |receiver| receiver.try_iter().collect());
                let mut directive = PixelWindowDirective::Continue;
                for reply in replies {
                    self.pending = self.pending.saturating_sub(1);
                    let effect = self.workspace.reply(reply);
                    if matches!(self.dispatch(effect), PixelWindowDirective::Exit) {
                        directive = PixelWindowDirective::Exit;
                    }
                }
                if self.pending == 0 && self.queued.is_empty()
                    && let Some(path) = self.initial_path.take() && !self.workspace.modal() {
                    let effect = self.workspace.open_path(path);
                    self.dispatch(effect);
                }
                self.pump();
                self.sync_title(window);
                window.request_redraw();
                Ok(directive)
            }
            PixelWindowEvent::Keyboard(event) => {
                let Some(key) = key_of(&event) else {
                    return Ok(PixelWindowDirective::Continue);
                };
                let effect = self.workspace.key(key);
                let directive = self.dispatch(effect);
                self.sync_title(window);
                window.request_redraw();
                Ok(directive)
            }
            PixelWindowEvent::Ime(event) => {
                use minicon_platform::contract::ime::ImeEvent;
                let effect = match event {
                    ImeEvent::Preedit { text, .. } => { self.workspace.ime_preedit(text); Effect::Nothing }
                    ImeEvent::Commit(text) => self.workspace.ime_commit(text),
                    ImeEvent::Disabled => { self.workspace.ime_preedit(String::new()); Effect::Nothing }
                    _ => Effect::Nothing,
                };
                let directive = self.dispatch(effect);
                self.sync_title(window);
                window.request_redraw();
                Ok(directive)
            }
            PixelWindowEvent::PointerMoved { position, .. } => {
                self.pointer = Some((position.x, position.y));
                Ok(PixelWindowDirective::Continue)
            }
            PixelWindowEvent::GeometryChanged { .. } | PixelWindowEvent::ResizeInteractionEnded => {
                window.request_redraw();
                Ok(PixelWindowDirective::Continue)
            }
            PixelWindowEvent::PointerLeft => {
                self.pointer = None;
                Ok(PixelWindowDirective::Continue)
            }
            PixelWindowEvent::PointerButton {
                button: PointerButton::Left,
                state: PointerButtonState::Pressed,
                position,
                ..
            } => self.pointer_gesture(window, position.map(|point| (point.x, point.y)), Gesture::Click),
            PixelWindowEvent::MouseWheel { delta, position, .. } => {
                self.pointer_gesture(window, position.map(|point| (point.x, point.y)), Gesture::Wheel(delta))
            }
            _ => Ok(PixelWindowDirective::Continue),
        } })();
        self.service_clipboard(window);
        let text_focus = self.workspace.filter_editor().is_some() || self.workspace.prompt().is_some_and(|prompt| !prompt.pending() && (prompt.kind() != PromptKind::Commit || self.workspace.commit_composer().is_some_and(|form| form.focus == CommitFocus::Message)))
            || self.workspace.create_form().is_some_and(|form| !form.submitted && form.review.is_none())
            || self.workspace.identity_form().is_some_and(|form| !form.submitted);
        if text_focus != self.ime_allowed { window.set_ime_allowed(text_focus); self.ime_allowed = text_focus; }
        if let (Some(started), Some(category), Some(timing)) = (started, category, self.timing.as_mut()) {
            timing.record(category, started.elapsed());
        }
        result
    }

    fn render(
        &mut self,
        window: &PixelWindow,
        frame: &mut XrgbPixelFrame<'_>,
    ) -> Result<PixelWindowDirective, PixelWindowError> {
        let started = self.timing.as_ref().map(|_| Instant::now());
        let result = (|| {
        let scale = window.scale_factor()?;
        let size = (BASE_SIZE_PX * scale).round().max(8.0) as u16;
        let metrics = font::primary_metrics(size)
            .map_err(|error| PixelWindowError::failed("status_font_metrics", format!("{error:?}")))?;
        let row = metrics.cell_height.ceil().max(1.0) as i64;
        let grid = Grid {
            row,
            ascent: metrics.ascent.round() as i64,
            advance: metrics.cell_width.round().max(1.0),
            margin: (MARGIN_PX * scale).round() as i64,
            gap: (row / 3).max(2),
        };
        let width = frame.width();
        let height = frame.height();
        if !self.ime_position_failed && let Some(form) = self.workspace.commit_composer() && form.focus == CommitFocus::Message {
            let geometry = commit_geometry(i64::from(width), i64::from(height), grid);
            let input = inner_of(geometry.message, grid);
            let visible = composer::visible_line_window(&form.editor.text, form.editor.caret, input.rows.max(1) as usize);
            let range = composer::line_range(&form.editor.text, visible.first_line + visible.caret_row);
            let line = &form.editor.text[range.clone()];
            let caret = form.editor.caret.saturating_sub(range.start).min(line.len());
            let horizontal = composer::visible_window(line, &form.editor.preedit, caret, 1, input.cols).text;
            let cell = composer::cells(&line[horizontal..caret]) + composer::cells(&form.editor.preedit);
            let area = minicon_platform::window_host::LogicalRect::new((input.x + cell as i64 * grid.advance.round() as i64) as f64 / scale, (input.top + visible.caret_row as i64 * grid.row) as f64 / scale, f64::from(grid.advance) / scale, grid.row as f64 / scale);
            if let Err(error) = window.set_ime_cursor_area(area) { self.ime_position_failed = true; let _ = self.workspace.reply(Reply::Notice(format!("IME caret positioning is unavailable: {error}"))); }
        }
        if !self.ime_position_failed && let Some(form) = self.workspace.filter_editor() {
            let input = inner_of(filter_field(layout(i64::from(width), i64::from(height), grid).main, grid), grid);
            let visible = composer::visible_window(&form.editor.text, &form.editor.preedit, form.editor.caret, 1, input.cols);
            let cell = composer::cells(&form.editor.text[visible.text..form.editor.caret]) + composer::cells(&form.editor.preedit);
            let area = minicon_platform::window_host::LogicalRect::new((input.x + cell as i64 * grid.advance.round() as i64) as f64 / scale, input.top as f64 / scale, f64::from(grid.advance) / scale, grid.row as f64 / scale);
            if let Err(error) = window.set_ime_cursor_area(area) { self.ime_position_failed = true; let _ = self.workspace.reply(Reply::Notice(format!("IME caret positioning is unavailable: {error}"))); }
        }
        if let Some(review) = self.workspace.trust_review() {
            let main = layout(i64::from(width), i64::from(height), grid).main;
            let (body, _) = trust_content(main, grid);
            let inner = inner_of(body, grid);
            let count = trust_lines(review, inner.cols).len();
            self.workspace.set_trust_scroll_limit(count.saturating_sub(inner.rows.max(0) as usize));
        }
        if let Some(form) = self.workspace.create_form() && let Some((_, lines)) = &form.review {
            let main = layout(i64::from(width), i64::from(height), grid).main;
            let (body, _) = trust_content(main, grid);
            let inner = inner_of(body, grid);
            let count = wrapped_lines(&lines.join("\n"), inner.cols).len();
            self.workspace.set_trust_scroll_limit(count.saturating_sub(inner.rows.max(0) as usize));
        }
        if let Some(form) = self.workspace.identity_form() {
            let main = layout(i64::from(width), i64::from(height), grid).main;
            let info = identity_info(main, grid);
            let inner = inner_of(info, grid);
            let count = wrapped_lines(&form.detail, inner.cols).len();
            self.workspace.set_trust_scroll_limit(count.saturating_sub(inner.rows.max(0) as usize));
        }
        self.surface = Some(Surface { width: i64::from(width), height: i64::from(height), grid });
        // The main panel's visible rows, so the cursor stays on screen after a resize.
        let main_rows = inner_of(layout(i64::from(width), i64::from(height), grid).main, grid).rows;
        self.workspace.set_view_rows(main_rows.max(0) as usize);
        let draws = scene(&self.workspace, i64::from(width), i64::from(height), grid, self.pending + self.queued.len());
        let mut canvas = Canvas { pixels: frame.pixels_mut(), stride: width, height };
        fill_xrgb_rect(canvas.pixels, width, 0, 0, width, height, BACKGROUND);
        for draw in draws {
            match draw {
                Draw::Rect { rect, colour } => fill(&mut canvas, rect, colour),
                Draw::Text { x, baseline, text, colour } => {
                    let style = Style { size, colour, advance: metrics.cell_width, ascent: grid.ascent };
                    self.draw_text(&mut canvas, Pen { x, baseline }, &text, style)?;
                }
            }
        }
        Ok(PixelWindowDirective::Wait)
        })();
        if let (Some(started), Some(timing)) = (started, self.timing.as_mut()) { timing.record(Category::Render, started.elapsed()); }
        result
    }

    fn about_to_wait(
        &mut self,
        window: &PixelWindow,
        now: Instant,
    ) -> Result<PixelWindowDirective, PixelWindowError> {
        if let Some(timing) = &mut self.timing { timing.record(Category::Wait, std::time::Duration::ZERO); }
        if self.workspace.draft_deadline().is_some_and(|deadline| deadline <= now) {
            let effect = self.workspace.checkpoint_draft();
            self.dispatch(effect);
            window.request_redraw();
        }
        Ok(self.workspace.draft_deadline().map_or(PixelWindowDirective::Wait, PixelWindowDirective::WaitUntil))
    }
}

/// Plain drawing commands. The scene is built from the workspace without any
/// platform type, so the layout is testable on any host.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Draw {
    Rect { rect: Rect, colour: u32 },
    Text { x: i64, baseline: i64, text: String, colour: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Rect {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Grid {
    /// Line height in pixels.
    row: i64,
    ascent: i64,
    advance: f32,
    margin: i64,
    gap: i64,
}

impl Grid {
    fn columns(self, width: i64) -> usize {
        ((width as f32 / self.advance).floor() as usize).max(1)
    }
}

/// Where a panel's text starts and how much of it fits.
struct Inner {
    x: i64,
    top: i64,
    width: i64,
    rows: i64,
    cols: usize,
}

/// Builds the whole window: header, three list panels on the left, the main
/// panel on the right and a two-line footer.
fn scene(workspace: &Workspace, width: i64, height: i64, grid: Grid, pending: usize) -> Vec<Draw> {
    let mut draws = Vec::new();
    let Grid { row, ascent, margin, .. } = grid;
    let snapshot = workspace.snapshot();

    let header = snapshot.title.clone();
    draws.push(Draw::Text {
        x: margin,
        baseline: margin + ascent,
        text: fit(&header, grid.columns(width - 2 * margin).saturating_sub(55)),
        colour: colour(Tone::Heading),
    });
    if pending > 0 {
        let size = (row / 2).max(2);
        draws.push(Draw::Text { x: width - margin - (grid.advance * 51.0) as i64, baseline: margin + ascent, text: "Working".to_owned(), colour: colour(Tone::Heading) });
        draws.push(Draw::Rect { rect: Rect { x: width - margin - size, y: margin, width: size, height: size }, colour: FOCUS_BORDER });
    }

    draws.push(Draw::Text {
        x: width - margin - (grid.advance * 27.0) as i64,
        baseline: margin + ascent,
        text: "Open [O]".to_owned(),
        colour: colour(Tone::Heading),
    });
    if workspace.is_welcome() {
        draws.push(Draw::Text { x: width - margin - (grid.advance * 17.0) as i64, baseline: margin + ascent, text: "Create [N]".into(), colour: colour(Tone::Heading) });
    } else {
        draws.push(Draw::Text { x: width - margin - (grid.advance * 39.0) as i64, baseline: margin + ascent, text: "Commit [C]".into(), colour: colour(Tone::Heading) });
        draws.push(Draw::Text { x: width - margin - (grid.advance * 17.0) as i64, baseline: margin + ascent, text: "Identity".into(), colour: colour(Tone::Heading) });
    }
    draws.push(Draw::Text { x: width - margin - (grid.advance * 6.0) as i64, baseline: margin + ascent, text: "Git".into(), colour: colour(Tone::Heading) });
    for (index, rect) in tab_rectangles(workspace, width, grid) {
        let tab = &workspace.tabs()[index];
        let active = workspace.snapshot().repository.as_ref() == Some(&tab.path);
        let label = std::path::Path::new(tab.path.as_os_str()).file_name().map(|n| srcmgr_domain::NativePath::new(n).display_label()).unwrap_or_else(|| tab.path.display_label());
        let head = if active { workspace.snapshot().head.clone() } else { format!("last: {}", tab.head) };
        button_draw(&mut draws, rect, grid, &format!("{} {label} | {head}", index + 1), if active { FOCUS_BORDER } else { BORDER });
    }
    if !workspace.tabs().is_empty() {
        let [previous, next, close] = tab_controls(width, grid);
        button_draw(&mut draws, previous, grid, "<", BORDER);
        button_draw(&mut draws, next, grid, ">", BORDER);
        button_draw(&mut draws, close, grid, "Close", BORDER);
    }

    if workspace.commit_composer().is_some() {
        commit_scene(&mut draws, workspace, width, height, grid);
        return draws;
    }

    if workspace.is_welcome() {
        let panels = layout(width, height, grid);
        let recents = Rect { x: margin, y: panels.main.y, width: panels.files.width, height: panels.main.height };
        let rows: Vec<Line> = snapshot.recents.iter().map(|entry| Line { tone: Tone::Text, text: format!("{}  {}", entry.backend, entry.label) }).collect();
        list_panel(&mut draws, recents, grid, Panel { focused: !workspace.modal(), title: "Recent repositories", empty: "No recent repositories" }, &rows, workspace.selected(Pane::Files));
        main_panel(&mut draws, panels.main, grid, workspace);
        if !workspace.modal() && !snapshot.recents.is_empty() {
            let (button, _) = trust_buttons(panels.main, grid);
            button_draw(&mut draws, button, grid, "Open selected", FOCUS_BORDER);
        }
        draws.push(Draw::Text { x: margin, baseline: panels.footer_top + ascent, text: fit(workspace.notice().unwrap_or(&snapshot.head), grid.columns(width - 2 * margin)), colour: colour(Tone::Changed) });
        draws.push(Draw::Text { x: margin, baseline: panels.footer_top + row + ascent, text: if workspace.create_form().is_some() { "Tab Switch field   Enter Review   y Create after review   Esc Back/cancel".into() } else if workspace.modal() { hints(workspace) } else { "Ctrl+O Open   N Create   G Git settings   Enter Open selected   d Remove   q Quit".into() }, colour: MUTED_TEXT });
        return draws;
    }

    let Layout {
        footer_top,
        status: status_rect,
        files: files_rect,
        branches: branches_rect,
        commits: commits_rect,
        main: main_rect,
    } = layout(width, height, grid);

    let focus = workspace.focus();
    let selected = |pane| workspace.selected(pane);
    list_panel(
        &mut draws,
        status_rect,
        grid,
        Panel { focused: focus == Pane::Status, title: "[1] Status", empty: "" },
        &[Line { tone: Tone::Text, text: snapshot.head.clone() }],
        selected(Pane::Status),
    );
    let file_pane = if workspace.stash_list_visible() { Pane::Stashes } else { Pane::Files };
    if file_pane == Pane::Stashes {
        let rows: Vec<_> = snapshot.stashes.iter().enumerate().map(|(index, item)| Line { tone: Tone::Text, text: format!("stash@{{{index}}} {} {}", short(&item.id), item.subject) }).collect();
        list_panel(&mut draws, files_rect, grid, Panel { focused: focus == Pane::Stashes, title: &format!("[5] Stashes ({}; max 200)", snapshot.stashes.len()), empty: "No stashes" }, &rows, selected(Pane::Stashes));
    } else { filtered_panel(&mut draws, files_rect, grid, workspace, Pane::Files); }
    filtered_panel(&mut draws, branches_rect, grid, workspace, Pane::Branches);
    filtered_panel(&mut draws, commits_rect, grid, workspace, Pane::Commits);

    main_panel(&mut draws, main_rect, grid, workspace);

    let notice = workspace.notice().or(snapshot.stage_result.as_deref()).unwrap_or("");
    draws.push(Draw::Text {
        x: margin,
        baseline: footer_top + ascent,
        text: fit(notice, grid.columns(width - 2 * margin)),
        colour: colour(Tone::Removed),
    });
    let working = if pending > 0 { "   working..." } else { "" };
    draws.push(Draw::Text {
        x: margin,
        baseline: footer_top + row + ascent,
        text: fit(&format!("{}{working}", hints(workspace)), grid.columns(width - 2 * margin)),
        colour: MUTED_TEXT,
    });
    draws
}

struct CommitGeometry { index: Rect, message: Rect, diff: Rect, commit: Rect, refresh: Rect, close: Rect, footer: i64 }
const COMMIT_CONTEXT_ROWS: i64 = 3;

fn commit_geometry(width: i64, height: i64, grid: Grid) -> CommitGeometry {
    let panels = layout(width, height, grid);
    let index = Rect { x: grid.margin, y: panels.main.y, width: panels.files.width, height: panels.main.height };
    let (_, button_height) = trust_content(panels.main, grid);
    let button_width = ((panels.main.width - 2 * grid.gap) / 3).max(0);
    let commit = Rect { x: panels.main.x, y: panels.main.y + panels.main.height - button_height, width: button_width, height: button_height };
    let refresh = Rect { x: commit.x + button_width + grid.gap, ..commit };
    let close = Rect { x: refresh.x + button_width + grid.gap, ..commit };
    let body_height = (commit.y - panels.main.y - grid.gap).max(0);
    let message = Rect { height: (8 * grid.row).min(body_height / 2), ..panels.main };
    let diff = Rect { y: message.y + message.height + grid.gap, height: (body_height - message.height - grid.gap).max(0), ..panels.main };
    CommitGeometry { index, message, diff, commit, refresh, close, footer: panels.footer_top }
}

fn commit_scene(draws: &mut Vec<Draw>, workspace: &Workspace, width: i64, height: i64, grid: Grid) {
    let form = workspace.commit_composer().unwrap();
    let geometry = commit_geometry(width, height, grid);
    let inner = frame(draws, geometry.index, grid, form.focus == CommitFocus::Index, "Complete staged index");
    if let Some(review) = &form.review {
        for (row, line) in wrapped_lines(&review.head, inner.cols).into_iter().take(2).enumerate() {
            draws.push(Draw::Text { x: inner.x, baseline: inner.top + row as i64 * grid.row + grid.ascent, text: line.text, colour: MUTED_TEXT });
        }
        draws.push(Draw::Text { x: inner.x, baseline: inner.top + 2 * grid.row + grid.ascent, text: fit(&format!("{} paths | unfiltered", review.files.len()), inner.cols), colour: colour(Tone::Heading) });
        let rows = (inner.rows - COMMIT_CONTEXT_ROWS).max(1);
        let offset = window_start(form.selected, review.files.len(), rows);
        for (index, file) in review.files.iter().enumerate().skip(offset).take(rows as usize) {
            let y = inner.top + (COMMIT_CONTEXT_ROWS + (index - offset) as i64) * grid.row;
            if index == form.selected { draws.push(Draw::Rect { rect: Rect { x: inner.x, y, width: inner.width, height: grid.row }, colour: if form.focus == CommitFocus::Index { SELECTION } else { SELECTION_DIM } }); }
            let counts = match (file.added, file.deleted) { (Some(added), Some(deleted)) => format!("+{added}/-{deleted}"), _ => "binary/?".into() };
            draws.push(Draw::Text { x: inner.x, baseline: y + grid.ascent, text: fit(&format!("{} {counts} {}", file.change, file.path.display_label()), inner.cols), colour: colour(Tone::Text) });
        }
    } else {
        draws.push(Draw::Text { x: inner.x, baseline: inner.top + grid.ascent, text: "Review unavailable/loading".into(), colour: MUTED_TEXT });
    }
    let input = frame(draws, geometry.message, grid, form.focus == CommitFocus::Message, "Commit message | Ctrl+Enter submits");
    let visible = composer::visible_line_window(&form.editor.text, form.editor.caret, input.rows.max(1) as usize);
    let selection = composer::selection_bounds(&form.editor);
    for row in 0..visible.line_count {
        let range = composer::line_range(&form.editor.text, visible.first_line + row);
        let line = &form.editor.text[range.clone()];
        let caret = form.editor.caret.saturating_sub(range.start).min(line.len());
        let horizontal = if row == visible.caret_row { composer::visible_window(line, &form.editor.preedit, caret, 1, input.cols).text } else { 0 };
        let top = input.top + row as i64 * grid.row;
        if let Some((start, end)) = selection {
            let start = start.max(range.start + horizontal).min(range.end);
            let end = end.min(range.end).max(start);
            if start < end {
                let before = composer::cells(&form.editor.text[range.start + horizontal..start]);
                let cells = composer::cells(&form.editor.text[start..end]).min(input.cols.saturating_sub(before));
                draws.push(Draw::Rect { rect: Rect { x: input.x + before as i64 * grid.advance.round() as i64, y: top, width: cells as i64 * grid.advance.round() as i64, height: grid.row }, colour: SELECTION });
            }
        }
        let mut text = line[horizontal..].to_owned();
        if row == visible.caret_row && form.focus == CommitFocus::Message {
            text.insert_str(caret.saturating_sub(horizontal), &format!("{}|", form.editor.preedit));
        }
        draws.push(Draw::Text { x: input.x, baseline: top + grid.ascent, text: fit(&text, input.cols), colour: colour(Tone::Text) });
    }
    let diff_inner = frame(draws, geometry.diff, grid, form.focus == CommitFocus::Diff, "Staged Diff | arrows/page scroll");
    for (row, line) in form.diff.lines.iter().skip(form.diff_scroll).take(diff_inner.rows.max(0) as usize).enumerate() {
        let offset = composer::caret_at_cell(&line.text, 0, form.diff_column);
        draws.push(Draw::Text { x: diff_inner.x, baseline: diff_inner.top + row as i64 * grid.row + grid.ascent, text: fit(&line.text[offset..], diff_inner.cols), colour: colour(line.tone) });
    }
    let blocked = form.review.as_ref().and_then(|review| review.blocked.as_deref());
    let ready = form.review.is_some() && blocked.is_none() && form.diff_ready && !form.editor.text.trim().is_empty() && form.editor.text.len() <= composer::PASTE_LIMIT_BYTES && form.editor.preedit.is_empty() && !workspace.prompt().is_some_and(|prompt| prompt.pending());
    button_draw(draws, geometry.commit, grid, if ready { "Commit" } else { "Commit disabled" }, if ready { FOCUS_BORDER } else { BORDER });
    button_draw(draws, geometry.refresh, grid, "Refresh review", BORDER);
    button_draw(draws, geometry.close, grid, "Save and close", BORDER);
    let notice = form.save_error.as_deref().or(workspace.notice()).or(blocked).unwrap_or("Review is bound to the full index; Commit rechecks freshness");
    draws.push(Draw::Text { x: grid.margin, baseline: geometry.footer + grid.ascent, text: fit(&format!("{} | {notice}", workspace.draft_status()), grid.columns(width - 2 * grid.margin)), colour: colour(Tone::Changed) });
    draws.push(Draw::Text { x: grid.margin, baseline: geometry.footer + grid.row + grid.ascent, text: fit("Tab Message/Index/Diff   F5 Refresh   Ctrl+S Save   Ctrl+Enter Commit   Esc Save and close", grid.columns(width - 2 * grid.margin)), colour: MUTED_TEXT });
}

/// The panels' rectangles for one frame, in physical pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Layout {
    footer_top: i64,
    status: Rect,
    files: Rect,
    branches: Rect,
    commits: Rect,
    main: Rect,
}

fn layout(width: i64, height: i64, grid: Grid) -> Layout {
    let Grid { row, margin, gap, .. } = grid;
    let footer_top = height - margin - 2 * row;
    let body_top = margin + 2 * row + 2 * gap;
    let body_height = (footer_top - gap - body_top).max(0);
    let left_width = (width * 38 / 100 - margin - gap / 2).max(0);
    let right_x = margin + left_width + gap;
    let right_width = (width - margin - right_x).max(0);

    // The Status panel is two rows including its frame title; the other three
    // share what is left, with a gap between each pair of panels.
    let status_height = 2 * row + 2 * gap;
    let status = Rect { x: margin, y: body_top, width: left_width, height: status_height };
    let lists_top = status.y + status_height + gap;
    let avail = (body_height - status_height - 3 * gap).max(0);
    let files_height = avail * 50 / 100;
    let branches_height = avail * 25 / 100;
    let commits_height = avail - files_height - branches_height;
    let files = Rect { x: margin, y: lists_top, width: left_width, height: files_height };
    let branches = Rect { x: margin, y: files.y + files_height + gap, width: left_width, height: branches_height };
    let commits = Rect { x: margin, y: branches.y + branches_height + gap, width: left_width, height: commits_height };
    let main = Rect { x: right_x, y: body_top, width: right_width, height: body_height };
    Layout { footer_top, status, files, branches, commits, main }
}

/// The size and grid of the last frame drawn.
#[derive(Clone, Copy)]
struct Surface {
    width: i64,
    height: i64,
    grid: Grid,
}

/// What a pointer position is over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Hit {
    Back,
    Filter(Pane),
    ClearFilter(Pane),
    Repository(usize),
    NextRepository,
    PreviousRepository,
    CloseRepository,
    Commit,
    IdentitySettings,
    GitSettings,
    Create,
    Recent(usize),
    RecentOpen,
    Open,
    /// A list's frame, title or the blank part of a list.
    Panel(Pane),
    /// A list row. The index may be past the last item.
    Row(Pane, usize),
    /// The main panel: `None` on its frame or title, otherwise the line of the
    /// whole diff under the pointer.
    Main(Option<usize>),
    Nothing,
}

#[derive(Clone, Copy)]
enum Gesture {
    Click,
    Wheel(WheelDelta),
}

fn tab_controls(width: i64, grid: Grid) -> [Rect; 3] {
    let close_width = (grid.advance * 8.0) as i64;
    let arrow_width = (grid.advance * 4.0) as i64;
    let close = Rect { x: width - grid.margin - close_width, y: grid.margin + grid.row + grid.gap, width: close_width, height: grid.row + grid.gap };
    let next = Rect { x: close.x - grid.gap - arrow_width, width: arrow_width, ..close };
    let previous = Rect { x: next.x - grid.gap - arrow_width, ..next };
    [previous, next, close]
}

fn tab_rectangles(workspace: &Workspace, width: i64, grid: Grid) -> Vec<(usize, Rect)> {
    let available = (tab_controls(width, grid)[0].x - 2 * grid.margin).max(1);
    let count = ((available / (grid.advance * 25.0) as i64).max(1) as usize).min(workspace.tabs().len());
    if count == 0 { return Vec::new(); }
    let active = workspace.tabs().iter().position(|t| Some(&t.path) == workspace.snapshot().repository.as_ref()).unwrap_or(0);
    let start = active.saturating_sub(count / 2).min(workspace.tabs().len() - count);
    let slot = available / count as i64;
    (start..start + count).enumerate().map(|(column, index)| (index, Rect { x: grid.margin + column as i64 * slot, y: grid.margin + grid.row + grid.gap, width: slot - grid.gap, height: grid.row + grid.gap })).collect()
}

fn tab_hit(workspace: &Workspace, surface: Surface, x: i64, y: i64) -> Option<Hit> {
    if workspace.tabs().is_empty() { return None; }
    let [previous, next, close] = tab_controls(surface.width, surface.grid);
    if contains(previous, x, y) { return Some(Hit::PreviousRepository); }
    if contains(next, x, y) { return Some(Hit::NextRepository); }
    if contains(close, x, y) { return Some(Hit::CloseRepository); }
    tab_rectangles(workspace, surface.width, surface.grid).into_iter().find(|(_, rect)| contains(*rect, x, y)).map(|(index, _)| Hit::Repository(index))
}

fn hit_test(workspace: &Workspace, surface: Surface, x: i64, y: i64) -> Hit {
    let grid = surface.grid;
    let commit_x = surface.width - grid.margin - (grid.advance * 39.0) as i64;
    if !workspace.is_welcome() && x >= commit_x && x < commit_x + (grid.advance * 10.0) as i64 && y >= grid.margin && y < grid.margin + grid.row.max(2 * grid.margin) { return Hit::Commit; }
    let git_x = surface.width - grid.margin - (grid.advance * 6.0) as i64;
    if x >= git_x && x < git_x + (grid.advance * 6.0) as i64 && y >= grid.margin && y < grid.margin + grid.row.max(2 * grid.margin) { return Hit::GitSettings; }
    let create_x = surface.width - grid.margin - (grid.advance * 17.0) as i64;
    if x >= create_x && x < create_x + (grid.advance * 10.0) as i64 && y >= grid.margin && y < grid.margin + grid.row.max(2 * grid.margin) { return if workspace.is_welcome() { Hit::Create } else { Hit::IdentitySettings }; }
    let open_x = surface.width - grid.margin - (grid.advance * 27.0) as i64;
    if x >= open_x && x < open_x + (grid.advance * 8.0) as i64 && y >= grid.margin && y < grid.margin + grid.row.max(2 * grid.margin) {
        return Hit::Open;
    }
    let panels = layout(surface.width, surface.height, grid);
    if workspace.is_welcome() {
        let recents = Rect { x: grid.margin, y: panels.main.y, width: panels.files.width, height: panels.main.height };
        let inner = inner_of(recents, grid);
        let (open, _) = trust_buttons(panels.main, grid);
        if contains(open, x, y) && !workspace.snapshot().recents.is_empty() { return Hit::RecentOpen; }
        if contains(recents, x, y) && y >= inner.top && y < inner.top + inner.rows * grid.row {
            let offset = window_start(workspace.selected(Pane::Files), workspace.snapshot().recents.len(), inner.rows);
            return Hit::Recent(offset + ((y - inner.top) / grid.row) as usize);
        }
        return Hit::Nothing;
    }
    let snapshot = workspace.snapshot();
    let lists = [
        (Pane::Status, panels.status, 1),
        (if workspace.stash_list_visible() { Pane::Stashes } else { Pane::Files }, panels.files, if workspace.stash_list_visible() { snapshot.stashes.len() } else { workspace.visible_rows(Pane::Files).len() }),
        (Pane::Branches, panels.branches, workspace.visible_rows(Pane::Branches).len()),
        (Pane::Commits, panels.commits, workspace.visible_rows(Pane::Commits).len()),
    ];
    for (pane, rect, length) in lists {
        if !contains(rect, x, y) {
            continue;
        }
        let inner = if matches!(pane, Pane::Files | Pane::Branches | Pane::Commits) { filtered_inner_of(rect, grid) } else { inner_of(rect, grid) };
        if matches!(pane, Pane::Files | Pane::Branches | Pane::Commits) {
            let (field, clear) = filter_controls(rect, grid);
            if contains(clear, x, y) { return Hit::ClearFilter(pane); }
            if contains(field, x, y) { return Hit::Filter(pane); }
        }
        if y < inner.top || y >= inner.top + inner.rows * grid.row {
            return Hit::Panel(pane);
        }
        let offset = window_start(workspace.selected(pane), length, inner.rows);
        return Hit::Row(pane, offset + ((y - inner.top) / grid.row) as usize);
    }
    if contains(panels.main, x, y) {
        if workspace.can_back() && contains(back_control(panels.main, grid), x, y) { return Hit::Back; }
        let inner = inner_of(panels.main, grid);
        if y < inner.top || y >= inner.top + inner.rows * grid.row {
            return Hit::Main(None);
        }
        return Hit::Main(Some(workspace.scroll() + ((y - inner.top) / grid.row) as usize));
    }
    Hit::Nothing
}

fn contains(rect: Rect, x: i64, y: i64) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

const fn pane_of(hit: Hit) -> Option<Pane> {
    match hit {
        Hit::Panel(pane) | Hit::Row(pane, _) => Some(pane),
        Hit::Main(_) => Some(Pane::Main),
        Hit::Recent(_) => Some(Pane::Files),
        Hit::Nothing | Hit::Open | Hit::Create | Hit::GitSettings | Hit::IdentitySettings | Hit::RecentOpen | Hit::Commit | Hit::Repository(_) | Hit::NextRepository | Hit::PreviousRepository | Hit::CloseRepository | Hit::Filter(_) | Hit::ClearFilter(_) | Hit::Back => None,
    }
}

/// Rows a wheel delta moves. Scrolling up moves towards the top, so the result
/// is negative for a positive wheel delta.
fn wheel_lines(delta: WheelDelta, row: i64, scale: f64) -> isize {
    let rows = match delta {
        WheelDelta::Lines { y, .. } => f64::from(y) * WHEEL_LINES,
        WheelDelta::LogicalPixels { y, .. } => y * scale / row.max(1) as f64,
        _ => 0.0,
    };
    (-rows).round() as isize
}

struct Panel<'a> {
    focused: bool,
    title: &'a str,
    empty: &'a str,
}

fn pane_label(pane: Pane) -> &'static str { match pane { Pane::Files => "Files", Pane::Branches => "Branches", Pane::Commits => "Commits", _ => "List" } }

fn filter_controls(rect: Rect, grid: Grid) -> (Rect, Rect) {
    let inner = inner_of(rect, grid);
    let clear = Rect { x: inner.x + inner.width - (grid.advance * 6.0) as i64, y: inner.top, width: (grid.advance * 6.0) as i64, height: grid.row };
    (Rect { x: inner.x, y: inner.top, width: (clear.x - inner.x - grid.gap).max(0), height: grid.row }, clear)
}

fn filtered_inner_of(rect: Rect, grid: Grid) -> Inner {
    let mut inner = inner_of(rect, grid);
    inner.top += grid.row;
    inner.rows = (inner.rows - 1).max(0);
    inner
}

fn filtered_panel(draws: &mut Vec<Draw>, rect: Rect, grid: Grid, workspace: &Workspace, pane: Pane) {
    let snapshot = workspace.snapshot();
    let rows = workspace.visible_rows(pane);
    let total = match pane { Pane::Files => snapshot.files.len(), Pane::Branches => snapshot.branches.len(), Pane::Commits => snapshot.commits.len(), _ => 0 };
    let number = match pane { Pane::Files => 2, Pane::Branches => 3, _ => 4 };
    let query = workspace.filter_query(pane);
    let counts = if query.is_empty() { total.to_string() } else { format!("{}/{total}", rows.len()) };
    let title = if pane == Pane::Commits { format!("[4] Loaded commits {counts} max{}", snapshot.history_limit) } else { format!("[{number}] {} ({counts})", pane_label(pane)) };
    frame(draws, rect, grid, workspace.focus() == pane, &title);
    let (field, clear) = filter_controls(rect, grid);
    draws.push(Draw::Text { x: field.x, baseline: field.y + grid.ascent, text: fit(&format!("Filter /: {query}"), grid.columns(field.width)), colour: if query.is_empty() { MUTED_TEXT } else { colour(Tone::Heading) } });
    draws.push(Draw::Text { x: clear.x, baseline: clear.y + grid.ascent, text: "Clear".into(), colour: if query.is_empty() { MUTED_TEXT } else { colour(Tone::Heading) } });
    let inner = filtered_inner_of(rect, grid);
    if rows.is_empty() {
        let message = if !query.is_empty() { "No matching loaded items" } else { match pane { Pane::Files => "Working tree clean", Pane::Branches => "No local branches", _ => "No commits yet" } };
        draws.push(Draw::Text { x: inner.x, baseline: inner.top + grid.ascent, text: fit(message, inner.cols), colour: MUTED_TEXT });
        return;
    }
    let selected = workspace.selected(pane);
    let offset = window_start(selected, rows.len(), inner.rows);
    for (position, raw) in rows.iter().enumerate().skip(offset).take(inner.rows.max(0) as usize) {
        let top = inner.top + (position - offset) as i64 * grid.row;
        if position == selected { draws.push(Draw::Rect { rect: Rect { x: inner.x, y: top, width: inner.width, height: grid.row }, colour: if workspace.focus() == pane { SELECTION } else { SELECTION_DIM } }); }
        let (text, tone) = match pane {
            Pane::Files => { let file = &snapshot.files[*raw]; (file.label.clone(), file.tone) }
            Pane::Branches => { let branch = &snapshot.branches[*raw]; (format!("{} {}", if branch.current { '*' } else { ' ' }, branch.label), if branch.current { Tone::Heading } else { Tone::Text }) }
            _ => { let commit = &snapshot.commits[*raw]; (format!("{}{} {}", if commit.merge { 'M' } else { ' ' }, short(&commit.id), commit.subject), Tone::Text) }
        };
        draws.push(Draw::Text { x: inner.x, baseline: top + grid.ascent, text: fit(&text, inner.cols), colour: colour(tone) });
    }
}

fn filter_field(rect: Rect, grid: Grid) -> Rect { Rect { x: rect.x + grid.gap, y: rect.y + grid.row + grid.gap, width: (rect.width - 2 * grid.gap).max(0), height: 3 * grid.row } }

fn filter_scene(draws: &mut Vec<Draw>, rect: Rect, grid: Grid, workspace: &Workspace) {
    let form = workspace.filter_editor().unwrap();
    frame(draws, rect, grid, true, &format!("Filter {} | loaded items only", pane_label(form.pane)));
    let input = frame(draws, filter_field(rect, grid), grid, true, "Literal query | 256 characters");
    let window = composer::visible_window(&form.editor.text, &form.editor.preedit, form.editor.caret, 1, input.cols);
    let mut text = form.editor.text[window.text..].to_owned();
    text.insert_str(form.editor.caret.saturating_sub(window.text), &format!("{}|", form.editor.preedit));
    if let Some((a, b)) = composer::selection_bounds(&form.editor) {
        let start = a.max(window.text).min(form.editor.text.len());
        let end = b.max(start);
        let before = composer::cells(&form.editor.text[window.text..start]);
        let cells = composer::cells(&form.editor.text[start..end]).min(input.cols.saturating_sub(before));
        draws.push(Draw::Rect { rect: Rect { x: input.x + before as i64 * grid.advance.round() as i64, y: input.top, width: cells as i64 * grid.advance.round() as i64, height: grid.row }, colour: SELECTION });
    }
    draws.push(Draw::Text { x: input.x, baseline: input.top + grid.ascent, text: fit(&text, input.cols), colour: colour(Tone::Text) });
    let total = match form.pane { Pane::Files => workspace.snapshot().files.len(), Pane::Branches => workspace.snapshot().branches.len(), _ => workspace.snapshot().commits.len() };
    let scope = if form.pane == Pane::Commits { format!("Latest {total} loaded commits; configured maximum {}. Unloaded history is not searched.", workspace.snapshot().history_limit) } else { "Only loaded items are filtered. No Git, network or profile work occurs while typing.".into() };
    let info = format!("{} / {total} loaded items match.\n{scope}\n\nEnter applies and returns to the list.\nEscape restores the previous query and selection.\nUp/Down previews selection without loading details.\nTab applies and changes pane. Clear restores the unfiltered list.\n\n{}", workspace.visible_rows(form.pane).len(), workspace.notice().unwrap_or("Literal, case-insensitive matching; spaces are meaningful."));
    let body = Rect { y: filter_field(rect, grid).y + 4 * grid.row, height: (rect.height - 6 * grid.row).max(0), ..filter_field(rect, grid) };
    text_panel(draws, body, grid, false, "Filter scope and actions", &wrapped_lines(&info, inner_of(body, grid).cols), 0, None);
}

fn list_panel(draws: &mut Vec<Draw>, rect: Rect, grid: Grid, panel: Panel<'_>, rows: &[Line], selected: usize) {
    let inner = frame(draws, rect, grid, panel.focused, panel.title);
    if rows.is_empty() {
        draws.push(Draw::Text {
            x: inner.x,
            baseline: inner.top + grid.ascent,
            text: fit(panel.empty, inner.cols),
            colour: MUTED_TEXT,
        });
        return;
    }
    let offset = window_start(selected, rows.len(), inner.rows);
    for (index, line) in rows.iter().enumerate().skip(offset).take(inner.rows as usize) {
        let top = inner.top + (index - offset) as i64 * grid.row;
        if index == selected {
            let highlight = if panel.focused { SELECTION } else { SELECTION_DIM };
            draws.push(Draw::Rect {
                rect: Rect { x: inner.x, y: top, width: inner.width, height: grid.row },
                colour: highlight,
            });
        }
        draws.push(Draw::Text {
            x: inner.x,
            baseline: top + grid.ascent,
            text: fit(&line.text, inner.cols),
            colour: colour(line.tone),
        });
    }
}

/// The main panel shows the editor while a prompt is open, the key help while
/// help is open, and otherwise the diff or detail of the source list.
fn back_control(rect: Rect, grid: Grid) -> Rect { Rect { x: rect.x + rect.width - grid.gap - (grid.advance * 10.0) as i64, y: rect.y + edge(grid), width: (grid.advance * 10.0) as i64, height: grid.row + grid.gap / 2 } }

fn main_panel(draws: &mut Vec<Draw>, rect: Rect, grid: Grid, workspace: &Workspace) {
    if let Some(issue) = workspace.navigation_issue() {
        let (body, _) = trust_content(rect, grid);
        let text = format!("Repository: {}\n\n{}\n\nArchive the reviewed saved view and open with the default layout.\nThe archive is retained in this profile; no automatic deletion.\nDrafts, repository trust, source files, index and refs are preserved.\n\nChanged or expired records require a fresh review.\nIf entry later fails, the archive remains available.\nAutomatic recovery review is limited to 1 MiB.\n\ny: Archive view and open   n/Esc: Cancel\nEnter does not approve this change.", issue.path.display_label(), issue.reason);
        text_panel(draws, body, grid, true, "Saved view needs recovery", &wrapped_lines(&text, inner_of(body, grid).cols), 0, None);
        let (allow, cancel) = trust_buttons(rect, grid);
        button_draw(draws, allow, grid, if issue.submitted { "Working..." } else { "Archive view and open" }, BORDER);
        button_draw(draws, cancel, grid, "Cancel", FOCUS_BORDER);
        return;
    }
    if workspace.filter_editor().is_some() { filter_scene(draws, rect, grid, workspace); return; }
    if let Some(form) = workspace.identity_form() {
        let (body, _) = trust_content(rect, grid);
        frame(draws, body, grid, true, "Repository identity (local scope)");
        let (name, email) = create_fields(rect, grid);
        let label = |text: &str| srcmgr_domain::NativePath::new(text).display_label();
        text_panel(draws, name, grid, !form.email_focus && !form.submitted, "Name", &[Line { tone: Tone::Text, text: field_tail(&label(&form.name), inner_of(name, grid).cols, !form.email_focus && !form.submitted) }], 0, None);
        text_panel(draws, email, grid, form.email_focus && !form.submitted, "Email", &[Line { tone: Tone::Text, text: field_tail(&label(&form.email), inner_of(email, grid).cols, form.email_focus && !form.submitted) }], 0, None);
        let info = identity_info(rect, grid);
        text_panel(draws, info, grid, false, "Effective identity and config sources (scroll)", &wrapped_lines(&form.detail, inner_of(info, grid).cols), workspace.trust_scroll(), None);
        let (save, cancel) = trust_buttons(rect, grid);
        button_draw(draws, save, grid, if form.submitted { "Working..." } else if form.ticket.is_none() { "Reload preview" } else { "Save local" }, BORDER);
        button_draw(draws, cancel, grid, "Cancel", FOCUS_BORDER);
        return;
    }
    if let Some(form) = workspace.create_form() {
        let (allow, cancel) = trust_buttons(rect, grid);
        if let Some((_, lines)) = &form.review {
            let (body, _) = trust_content(rect, grid);
            text_panel(draws, body, grid, true, "Review repository creation", &wrapped_lines(&lines.join("\n"), inner_of(body, grid).cols), workspace.trust_scroll(), None);
        } else {
            frame(draws, rect, grid, true, "Create local Git repository");
            let (path, branch) = create_fields(rect, grid);
            let path_lines = vec![Line { tone: Tone::Text, text: field_tail(&form.path, inner_of(path, grid).cols, !form.branch_focus) }];
            let branch_lines = vec![Line { tone: Tone::Text, text: field_tail(&form.branch, inner_of(branch, grid).cols, form.branch_focus) }];
            text_panel(draws, path, grid, !form.branch_focus, "Directory (parent must exist)", &path_lines, 0, None);
            text_panel(draws, branch, grid, form.branch_focus, "Initial branch", &branch_lines, 0, None);
            let info = Rect { y: branch.y + branch.height + grid.gap, height: (cancel.y - branch.y - branch.height - 2 * grid.gap).max(0), ..path };
            text_panel(draws, info, grid, false, "Before creating", &wrapped_lines("Choose a new or existing directory.\nExisting files stay unstaged and unchanged.\nReview the exact target before any write.\nTab or click switches fields; Esc cancels.", inner_of(info, grid).cols), 0, None);
        }
        button_draw(draws, allow, grid, if form.submitted { "Working..." } else if form.review.is_some() { "Create repository" } else { "Review creation" }, BORDER);
        button_draw(draws, cancel, grid, if form.review.is_some() { "Back" } else { "Cancel" }, FOCUS_BORDER);
        return;
    }
    if let Some(review) = workspace.trust_review() {
        let (body, _) = trust_content(rect, grid);
        let lines = trust_lines(review, inner_of(body, grid).cols);
        text_panel(draws, body, grid, true, "Review repository trust", &lines, workspace.trust_scroll(), None);
        let (allow, cancel) = trust_buttons(rect, grid);
        button_draw(draws, allow, grid, if review.submitted { "Opening..." } else { "Trust and open" }, if review.submitted { MUTED_TEXT } else { BORDER });
        button_draw(draws, cancel, grid, "Cancel", if review.submitted { MUTED_TEXT } else { FOCUS_BORDER });
        return;
    }
    let focused = workspace.focus() == Pane::Main || workspace.prompt().is_some();
    if workspace.help() {
        let lines: Vec<Line> =
            HELP_LINES.iter().map(|text| Line { tone: Tone::Text, text: (*text).to_owned() }).collect();
        text_panel(draws, rect, grid, true, "Keys", &lines, 0, None);
        return;
    }
    if let Some(prompt) = workspace.prompt() {
        if prompt.kind() == PromptKind::GitSettings {
            let (body, _) = trust_content(rect, grid);
            frame(draws, body, grid, true, "Application Git settings");
            let (field, _) = create_fields(body, grid);
            text_panel(draws, field, grid, true, "Git executable (blank = PATH)", &[Line { tone: Tone::Text, text: field_tail(prompt.text(), inner_of(field, grid).cols, true) }], 0, None);
            let info = Rect { y: field.y + field.height + grid.gap, height: (body.y + body.height - field.y - field.height - 2 * grid.gap).max(0), ..field };
            let text = format!("Selected path: {}\n\n{}", if prompt.text().is_empty() { "Automatic PATH discovery".into() } else { srcmgr_domain::NativePath::new(prompt.text()).display_label() }, workspace.git_detail());
            text_panel(draws, info, grid, false, "Selection and diagnostics", &wrapped_lines(&text, inner_of(info, grid).cols), 0, None);
            let (save, cancel) = trust_buttons(rect, grid);
            button_draw(draws, save, grid, if prompt.pending() { "Working..." } else { "Test and save" }, BORDER);
            button_draw(draws, cancel, grid, "Close", FOCUS_BORDER);
            return;
        }
        let title = match prompt.kind() {
            PromptKind::GitSettings => unreachable!("rendered above"),
            PromptKind::Commit => "Commit message",
            PromptKind::OpenRepository => "Open repository path (trust is required before Git runs)",
            PromptKind::NewBranch => "New branch name",
        };
        let mut lines: Vec<Line> = format!("{}_", prompt.text())
            .split('\n')
            .map(|text| Line { tone: Tone::Text, text: text.to_owned() })
            .collect();
        if lines.is_empty() {
            lines.push(Line { tone: Tone::Text, text: "_".to_owned() });
        }
        text_panel(draws, rect, grid, true, title, &lines, 0, None);
        return;
    }
    if let Some(confirm) = workspace.confirm() {
        let text = match confirm {
            Confirm::Stash { id, operation } => format!("{operation} stash {}? Full OID: {id}", short(id)),
            Confirm::DeleteBranch(name) => format!("Delete branch {name}?   y deletes it   n keeps it"),
            Confirm::DiscardFile { path, untracked: true } => {
                format!("Delete untracked file {}?   y deletes it   n keeps it", path.display_label())
            }
            Confirm::DiscardFile { path, untracked: false } => {
                format!("Discard unstaged changes to {}?   y discards   n keeps them", path.display_label())
            }
            Confirm::DiscardHunk { path, index } => {
                format!("Discard hunk {} of {}?   y discards   n keeps it", index + 1, path.display_label())
            }
            Confirm::Detach { short, .. } => format!("Check out {short} detached?   y checks out   n keeps HEAD"),
        };
        let mut messages = vec![text];
        messages.extend_from_slice(workspace.confirmation_lines());
        let columns = inner_of(rect, grid).cols.max(1);
        let lines: Vec<Line> = messages.iter().flat_map(|message| {
            let chars: Vec<char> = message.chars().collect();
            chars.chunks(columns).map(|chunk| Line { tone: Tone::Removed, text: chunk.iter().collect() }).collect::<Vec<_>>()
        }).collect();
        text_panel(draws, rect, grid, true, "Confirm and recovery", &lines, 0, None);
        return;
    }
    let main = workspace.main();
    let title = if workspace.is_welcome() { "Welcome" } else { match workspace.source() {
        Pane::Commits => match workspace.history_context() { Some(("Files", _, _)) => "Committed files", Some(("File Diff", _, _)) => "Committed file Diff", _ => "Commit details" },
        Pane::Branches => "Branch",
        Pane::Stashes => "Selected stash",
        Pane::Status | Pane::Files | Pane::Main => "Diff",
    }};
    let highlight = if focused { Some(workspace.selection()) } else { None };
    text_panel(draws, rect, grid, focused, title, &main.lines, workspace.scroll(), highlight);
    if workspace.can_back() { button_draw(draws, back_control(rect, grid), grid, "Back [Esc]", BORDER); }
}

fn trust_buttons(rect: Rect, grid: Grid) -> (Rect, Rect) {
    let height = (grid.row + 2 * grid.gap).max(2 * grid.margin);
    let width = ((grid.advance * 17.0) as i64 + 2 * grid.gap).min((rect.width - 3 * grid.gap).max(0) / 2);
    let first = Rect { x: rect.x + grid.gap, y: rect.y + rect.height - height - grid.gap, width, height };
    (first, Rect { x: first.x + width + grid.gap, ..first })
}

fn create_fields(rect: Rect, grid: Grid) -> (Rect, Rect) {
    let inner = inner_of(rect, grid);
    let path = Rect { x: inner.x, y: inner.top, width: inner.width, height: 3 * grid.row + 2 * grid.gap };
    (path, Rect { y: path.y + path.height + grid.gap, ..path })
}

fn field_tail(text: &str, columns: usize, focused: bool) -> String {
    let room = columns.saturating_sub(usize::from(focused));
    let mut used = 0;
    let reversed: Vec<_> = text.chars().rev().take_while(|ch| { used += composer::character_cells(*ch); used <= room }).collect();
    let mut visible: String = reversed.into_iter().rev().collect();
    if focused { visible.push('_'); }
    visible
}

fn identity_info(rect: Rect, grid: Grid) -> Rect {
    let (_, email) = create_fields(rect, grid);
    let (body, _) = trust_content(rect, grid);
    Rect { y: email.y + email.height + grid.gap, height: (body.y + body.height - email.y - email.height - 2 * grid.gap).max(0), ..email }
}

fn trust_content(rect: Rect, grid: Grid) -> (Rect, i64) {
    let (button, _) = trust_buttons(rect, grid);
    (Rect { height: (button.y - rect.y - grid.gap).max(0), ..rect }, button.height)
}

fn button_draw(draws: &mut Vec<Draw>, rect: Rect, grid: Grid, text: &str, border: u32) {
    draws.push(Draw::Rect { rect, colour: border });
    draws.push(Draw::Rect { rect: Rect { x: rect.x + 1, y: rect.y + 1, width: (rect.width - 2).max(0), height: (rect.height - 2).max(0) }, colour: BACKGROUND });
    draws.push(Draw::Text { x: rect.x + grid.gap, baseline: rect.y + (rect.height - grid.row) / 2 + grid.ascent, text: fit(text, grid.columns(rect.width - 2 * grid.gap)), colour: colour(Tone::Text) });
}

fn trust_lines(review: &crate::model::TrustReview, columns: usize) -> Vec<Line> {
    let text = format!("Location: {}\nGit metadata: {}\n\nRepository config, attributes and hooks can run programs as you.\nInherited Git helper settings remain your authority.\nOnly this location and its reviewed metadata are granted.\nNo repository Git command ran for this review.\n\nFingerprint: {}\nReview expires after 60 seconds; changes require a new review.\n\ny: Trust and open   n/Esc: Cancel\nUp/Down/Page keys scroll this review.", review.worktree.display_label(), review.git_directory.display_label(), review.fingerprint);
    wrapped_lines(&text, columns)
}

fn wrapped_lines(text: &str, columns: usize) -> Vec<Line> {
    let mut lines = Vec::new();
    for line in text.lines() {
        let mut current = String::new();
        let mut cells = 0;
        for ch in line.chars() {
            let width = composer::character_cells(ch);
            if !current.is_empty() && cells + width > columns.max(1) {
                lines.push(Line { tone: Tone::Text, text: std::mem::take(&mut current) });
                cells = 0;
            }
            current.push(ch);
            cells += width;
        }
        lines.push(Line { tone: Tone::Text, text: current });
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn text_panel(
    draws: &mut Vec<Draw>,
    rect: Rect,
    grid: Grid,
    focused: bool,
    title: &str,
    lines: &[Line],
    scroll: usize,
    highlight: Option<(usize, usize)>,
) {
    let inner = frame(draws, rect, grid, focused, title);
    for (index, line) in lines.iter().enumerate().skip(scroll).take(inner.rows as usize) {
        let top = inner.top + (index - scroll) as i64 * grid.row;
        if highlight.is_some_and(|(first, last)| (first..=last).contains(&index)) {
            draws.push(Draw::Rect {
                rect: Rect { x: inner.x, y: top, width: inner.width, height: grid.row },
                colour: SELECTION,
            });
        }
        draws.push(Draw::Text {
            x: inner.x,
            baseline: top + grid.ascent,
            text: fit(&line.text, inner.cols),
            colour: colour(line.tone),
        });
    }
}

/// Draws a bordered panel with its title and returns where its rows go.
fn frame(draws: &mut Vec<Draw>, rect: Rect, grid: Grid, focused: bool, title: &str) -> Inner {
    let edge = edge(grid);
    let border = if focused { FOCUS_BORDER } else { BORDER };
    let bottom = rect.y + rect.height - edge;
    draws.push(Draw::Rect { rect: Rect { x: rect.x, y: rect.y, width: rect.width, height: edge }, colour: border });
    draws.push(Draw::Rect { rect: Rect { x: rect.x, y: bottom, width: rect.width, height: edge }, colour: border });
    draws.push(Draw::Rect { rect: Rect { x: rect.x, y: rect.y, width: edge, height: rect.height }, colour: border });
    draws.push(Draw::Rect {
        rect: Rect { x: rect.x + rect.width - edge, y: rect.y, width: edge, height: rect.height },
        colour: border,
    });

    let inner = inner_of(rect, grid);
    let title_top = rect.y + edge + grid.gap / 2;
    draws.push(Draw::Text {
        x: inner.x,
        baseline: title_top + grid.ascent,
        text: fit(title, inner.cols),
        colour: if focused { FOCUS_BORDER } else { MUTED_TEXT },
    });
    inner
}

/// The border's thickness in pixels.
fn edge(grid: Grid) -> i64 {
    (grid.row / 12).max(1)
}

/// Where a bordered panel's rows go. Pure, so hit tests use the same geometry as drawing.
fn inner_of(rect: Rect, grid: Grid) -> Inner {
    let edge = edge(grid);
    let bottom = rect.y + rect.height - edge;
    let inner_width = (rect.width - 2 * (edge + grid.gap)).max(0);
    let top = rect.y + edge + grid.gap / 2 + grid.row;
    Inner {
        x: rect.x + edge + grid.gap,
        top,
        width: inner_width,
        rows: ((bottom - top) / grid.row).max(0),
        cols: grid.columns(inner_width),
    }
}

/// The first visible index, keeping `selected` inside a window of `rows` rows.
fn window_start(selected: usize, length: usize, rows: i64) -> usize {
    let rows = rows.max(1) as usize;
    if selected >= length || length <= rows || selected < rows { 0 } else { (selected + 1 - rows).min(length - rows) }
}

/// Keys the window shows for the current state, most useful first.
fn hints(workspace: &Workspace) -> String {
    if workspace.navigation_issue().is_some() { return "y Archive view and open   n/Esc Cancel   Enter does not approve".into(); }
    if workspace.filter_editor().is_some() { return "Enter Apply   Esc Restore   Tab Change pane   Up/Down Preview   Ctrl+A/C/V Edit".into(); }
    if let Some(form) = workspace.identity_form() { return if form.submitted { "Working; fields are read-only until completion. Esc cancels a read, not a submitted save." } else { "Tab Switch field   Enter Save local/reload   Page/Up/Down Scroll sources   Esc Cancel" }.into(); }
    if workspace.help() {
        return "Esc close help".to_owned();
    }
    if workspace.confirm().is_some() {
        return "y confirm   n keep".to_owned();
    }
    if let Some(prompt) = workspace.prompt() {
        return match prompt.kind() {
            PromptKind::GitSettings => "Enter test and save   Esc close   Blank uses PATH".to_owned(),
            PromptKind::Commit => "Ctrl+Enter commit   Enter new line   Esc save draft".to_owned(),
            PromptKind::OpenRepository => "Enter open   Esc cancel   Existing repository trust is checked".to_owned(),
            PromptKind::NewBranch => "Enter create   Esc cancel".to_owned(),
        };
    }
    let focus = match workspace.focus() {
        Pane::Status => "2 3 4 focus lists",
        Pane::Files => "Space stage/unstage   a all   d discard   s stash   5 stashes   c commit",
        Pane::Stashes => "Space/S apply and keep   p pop   d drop   2 files   Enter inspect",
        Pane::Branches => "Space checkout   n new   d delete   Enter details",
        Pane::Commits => "Enter files   Space checkout detached",
        Pane::Main if workspace.main().committed.is_some() => "Enter file diff   Esc commits",
        Pane::Main if workspace.main().file.is_none() => "j/k scroll   Esc back",
        Pane::Main if workspace.anchor_set() => "Space stage lines   v end range   a stage hunk   Esc back",
        Pane::Main => "Space stage line   v range   a stage hunk   d discard   Esc back",
    };
    format!("{focus}   Tab panels   ? help   q quit")
}

/// Shortens `text` to `columns` characters, marking the cut with "..". Control
/// characters become spaces so they cannot disturb the layout.
fn fit(text: &str, columns: usize) -> String {
    let clean: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if composer::cells(&clean) <= columns { return clean; }
    let budget = if columns <= 2 { columns } else { columns - 2 };
    let mut used = 0;
    let mut out: String = clean.chars().take_while(|ch| { used += composer::character_cells(*ch); used <= budget }).collect();
    if columns > 2 { out.push_str(".."); }
    out
}

/// Maps a platform key press to the window's key vocabulary. Releases and
/// keys the workspace does not use map to `None`.
fn key_of(event: &NormalizedKeyEvent) -> Option<Key> {
    if event.state != KeyPressState::Pressed {
        return None;
    }
    let control = event.modifiers.control_or_meta();
    let shift = event.modifiers.shift;
    if event.modifiers.control && event.modifiers.alt && !event.modifiers.meta {
        return event.text.as_ref().filter(|text| !text.is_empty()).map(|text| Key::Text(text.clone()));
    }
    match &event.logical {
        LogicalKey::Named(named) => Some(match named {
            NamedKey::ArrowLeft if control || shift => Key::EditMove { movement: if control { Move::WordLeft } else { Move::Left }, extend: shift },
            NamedKey::ArrowRight if control || shift => Key::EditMove { movement: if control { Move::WordRight } else { Move::Right }, extend: shift },
            NamedKey::ArrowUp if shift => Key::EditMove { movement: Move::Up, extend: true },
            NamedKey::ArrowDown if shift => Key::EditMove { movement: Move::Down, extend: true },
            NamedKey::Home if control || shift => Key::EditMove { movement: if control { Move::DraftStart } else { Move::LineStart }, extend: shift },
            NamedKey::End if control || shift => Key::EditMove { movement: if control { Move::DraftEnd } else { Move::LineEnd }, extend: shift },
            NamedKey::F5 => Key::RefreshReview,
            NamedKey::Delete => Key::Delete,
            NamedKey::ArrowUp => Key::Up,
            NamedKey::ArrowDown => Key::Down,
            NamedKey::ArrowLeft => Key::Left,
            NamedKey::ArrowRight => Key::Right,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::Enter if control => Key::CtrlEnter,
            NamedKey::Enter => Key::Enter,
            NamedKey::Escape => Key::Escape,
            NamedKey::Tab if control && shift => Key::PreviousRepository,
            NamedKey::Tab if control => Key::NextRepository,
            NamedKey::Tab if shift => Key::BackTab,
            NamedKey::Tab => Key::Tab,
            NamedKey::Space => Key::Space,
            NamedKey::Backspace => Key::Backspace,
            _ => return None,
        }),
        LogicalKey::Character(text) => {
            if control {
                return match text.as_str() {
                    "a" | "A" => Some(Key::SelectAll),
                    "x" | "X" => Some(Key::Cut),
                    "v" | "V" => Some(Key::Paste),
                    "s" | "S" => Some(Key::Save),
                    "w" | "W" => Some(Key::CloseRepository),
                    "c" | "C" => Some(Key::CtrlC),
                    "o" | "O" => Some(Key::CtrlO),
                    "d" | "D" => Some(Key::HalfPageDown),
                    "u" | "U" => Some(Key::HalfPageUp),
                    _ => None,
                };
            }
            let mut characters = text.chars();
            match (characters.next(), characters.next()) {
                (Some(' '), None) => Some(Key::Space),
                (Some(character), None) => Some(Key::Char(character)),
                (Some(_), Some(_)) => Some(Key::Text(text.clone())),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use srcmgr_domain::NativePath;

    use super::*;
    use crate::model::{BranchRow, CommitRow, FileRow, MainView};

    const GRID: Grid = Grid { row: 20, ascent: 15, advance: 8.0, margin: 10, gap: 6 };

    #[test]
    fn create_repository_queue_guard_preserves_form_and_welcome_has_named_entry() {
        struct UnusedBackend;
        impl Backend for UnusedBackend { fn run(&mut self, _: Job) -> Reply { panic!("must not launch") } }
        let mut window = StatusWindow::new(Snapshot::welcome(), Box::new(UnusedBackend));
        window.workspace.request_create();
        window.workspace.key(Key::Text("chosen target".into()));
        window.pending = 1;
        window.queued.push_back(Job::LoadWelcome);
        let effect = window.workspace.key(Key::Enter);
        window.dispatch(effect);
        assert_eq!(window.queued, VecDeque::from([Job::LoadWelcome]));
        assert_eq!(window.workspace.create_form().unwrap().path, "chosen target");
        assert!(!window.workspace.create_form().unwrap().submitted);
        assert_eq!(window.workspace.key(Key::Enter), Effect::Send(Job::PrepareCreate { path: NativePath::new("chosen target"), branch: "main".into() }));
        let workspace = Workspace::new(Snapshot::welcome());
        let surface = Surface { width: 1040, height: 680, grid: GRID };
        let x = surface.width - GRID.margin - (GRID.advance * 17.0) as i64;
        assert_eq!(hit_test(&workspace, surface, x + 4, GRID.margin + 4), Hit::Create);
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        let labels = texts(&draws);
        assert!(labels.contains(&"Welcome"));
        assert!(!labels.contains(&"Diff"));
    }

    #[test]
    fn ctrl_tab_after_navigation_repair_enters_the_other_raw_repository() {
        use minicon_platform::contract::input::{ModifierState, PhysicalKeyCode};
        let event = NormalizedKeyEvent { logical: LogicalKey::Named(NamedKey::Tab), physical: PhysicalKeyCode::Tab, text: None, state: KeyPressState::Pressed, repeat: false, modifiers: ModifierState { control: true, shift: false, alt: false, meta: false } };
        assert_eq!(key_of(&event), Some(Key::NextRepository));
        let mut workspace = Workspace::new(snapshot(1));
        workspace.request_open(); workspace.key(Key::Text("other".into())); workspace.key(Key::Enter);
        workspace.reply(Reply::NavigationIssue(crate::NavigationIssue { ticket: "repair".into(), path: NativePath::new("other"), reason: "Unknown version".into(), submitted: false }));
        workspace.answer_navigation(true);
        let mut other = snapshot(0); other.repository = Some(NativePath::new("other"));
        workspace.reply(Reply::NavigationReset { archive: "retained".into(), detail: "Archived".into(), next: Box::new(Reply::RepositoryOpened(other)) });
        assert_eq!(workspace.tabs().len(), 2);
        assert!(!workspace.modal());
        assert!(matches!(workspace.key(key_of(&event).unwrap()), Effect::Send(Job::EnterRepository { path: Some(path), .. }) if path == NativePath::new("fixture-repository")));
    }

    #[test]
    fn repository_tab_busy_refusal_keeps_the_tab_and_does_not_leave_an_open_dialog() {
        struct UnusedBackend;
        impl Backend for UnusedBackend { fn run(&mut self, _: Job) -> Reply { panic!("channel fixture only") } }
        let mut window = StatusWindow::new(snapshot(1), Box::new(UnusedBackend));
        window.pending = 1;
        let effect = window.workspace.close_repository();
        window.dispatch(effect);
        assert_eq!(window.workspace.tabs().len(), 1);
        assert_eq!(window.workspace.snapshot().repository, Some(NativePath::new("fixture-repository")));
        assert!(!window.workspace.modal(), "a refused Close is not an editable Open request");
        assert!(window.queued.is_empty());
        assert!(window.workspace.notice().unwrap().contains("Wait"));
        window.pending = 0;
        assert!(matches!(window.workspace.close_repository(), Effect::Send(Job::EnterRepository { path: None, .. })), "explicit retry is available after the work finishes");
    }

    #[test]
    fn open_repository_cannot_retarget_queued_work() {
        struct UnusedBackend;
        impl Backend for UnusedBackend { fn run(&mut self, _: Job) -> Reply { panic!("must not launch") } }
        let mut window = StatusWindow::new(snapshot(1), Box::new(UnusedBackend));
        window.pending = 1;
        window.queued.push_back(Job::Reload);
        window.workspace.request_open();
        window.workspace.key(Key::Text("other repository".into()));
        let effect = window.workspace.key(Key::Enter);
        window.dispatch(effect);
        assert_eq!(window.queued, VecDeque::from([Job::Reload]));
        assert_eq!(window.pending, 1);
        assert_eq!(window.workspace.prompt().unwrap().text(), "other repository");
        assert!(matches!(window.workspace.key(Key::Enter), Effect::Send(Job::EnterRepository { path: Some(path), current: Some(_) }) if path == NativePath::new("other repository")));
        let surface = Surface { width: 1040, height: 680, grid: GRID };
        let x = surface.width - GRID.margin - (GRID.advance * 27.0) as i64;
        assert_eq!(hit_test(&window.workspace, surface, x + 4, GRID.margin + 4), Hit::Open);
    }

    fn snapshot(files: usize) -> Snapshot {
        Snapshot {
            history_limit: 200,
            navigation: None,
            repository: Some(NativePath::new("fixture-repository")),
            recents: Vec::new(),
            title: "MiniCon SrcMgr - demo".to_owned(),
            head: "On branch main (0123456789ab)".to_owned(),
            files: (0..files)
                .map(|index| FileRow {
                    path: NativePath::new(format!("f{index}.txt")),
                    label: format!(".M f{index}.txt"),
                    tone: Tone::Changed,
                    has_staged: false,
                    has_unstaged: true,
                    untracked: false,
                })
                .collect(),
            branches: vec![BranchRow { name: "main".to_owned(), label: "main".to_owned(), current: true }],
            commits: vec![CommitRow { id: "0123456789abcdef".to_owned(), subject: "first".to_owned(), merge: false }],
            draft: None,
            stage_result: None,
            stashes: Vec::new(),
        }
    }

    fn texts(draws: &[Draw]) -> Vec<&str> {
        draws
            .iter()
            .filter_map(|draw| match draw {
                Draw::Text { text, .. } => Some(text.as_str()),
                Draw::Rect { .. } => None,
            })
            .collect()
    }

    #[test]
    fn slow_worker_coalesces_selection_reads_and_preserves_mutation_order() {
        struct UnusedBackend;
        impl Backend for UnusedBackend { fn run(&mut self, _: Job) -> Reply { panic!("channel fixture only") } }
        let mut window = StatusWindow::new(snapshot(1), Box::new(UnusedBackend));
        let (sender, receiver) = mpsc::channel();
        window.jobs = Some(sender);
        let read = |generation| Job::Main { generation, request: crate::MainRequest::Commit { id: format!("{generation:040x}") } };
        window.dispatch(Effect::Send(read(0)));
        assert_eq!(receiver.try_recv().unwrap(), read(0));
        for generation in 1..=100 { window.dispatch(Effect::Send(read(generation))); }
        assert_eq!(window.queued.len(), 1);
        assert!(receiver.try_recv().is_err(), "no queued read spawns Git while one is active");
        let mutation = Job::SetFile { path: NativePath::new("f0.txt"), stage: true };
        window.dispatch(Effect::Send(mutation.clone()));
        window.dispatch(Effect::Send(read(101)));
        window.dispatch(Effect::Send(read(102)));
        for expected in [read(100), mutation, read(102)] {
            window.pending = 0; // observed completion of the sole in-flight job
            window.pump();
            assert_eq!(receiver.try_recv().unwrap(), expected);
        }
        assert!(window.queued.is_empty());
        for _ in 0..100 { window.dispatch(Effect::Send(Job::Reload)); }
        assert_eq!(window.queued.len(), 16, "bounded input while the worker is busy");
    }

    #[test]
    fn a_stopped_worker_keeps_the_unsaved_message_editable() {
        struct UnusedBackend;
        impl Backend for UnusedBackend { fn run(&mut self, _: Job) -> Reply { panic!("channel fixture only") } }
        let mut window = StatusWindow::new(snapshot(1), Box::new(UnusedBackend));
        let (sender, receiver) = mpsc::channel();
        window.jobs = Some(sender);
        drop(receiver);
        window.workspace.open();
        window.workspace.key(Key::Char('c'));
        window.workspace.key(Key::Text("copy this message".into()));
        window.pending = 1;
        window.dispatch(Effect::Send(Job::Reload));
        let save = window.workspace.key(Key::Escape);
        window.dispatch(save);
        window.pending = 0;
        window.pump();
        assert_eq!(window.workspace.prompt().map(|p| p.text()), Some("copy this message"));
        assert_eq!(window.workspace.key(Key::Escape), Effect::Send(Job::SaveDraft("copy this message".into())), "failed save stays available for retry/copy");
    }

    #[test]
    fn fit_truncates_with_a_marker_and_keeps_short_text() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("a long label", 6), "a lo..");
        assert_eq!(fit("tab\there", 20), "tab here");
        assert_eq!(fit("abc", 2), "ab");
    }

    #[test]
    fn the_window_of_rows_follows_the_selection() {
        assert_eq!(window_start(0, 3, 5), 0);
        assert_eq!(window_start(4, 10, 5), 0);
        assert_eq!(window_start(7, 10, 5), 3);
        assert_eq!(window_start(9, 10, 5), 5);
        assert_eq!(window_start(2, 10, 0), 2);
    }

    #[test]
    fn the_scene_shows_the_header_panels_and_the_key_hints() {
        let workspace = Workspace::new(snapshot(2));
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        let all = texts(&draws);
        assert!(all.iter().any(|text| text.contains("MiniCon SrcMgr - demo")));
        assert!(all.iter().any(|text| text.starts_with("[1] Status")));
        assert!(all.iter().any(|text| text.starts_with("[2] Files (2)")));
        assert!(all.iter().any(|text| text.starts_with("[3] Branches (1)")));
        assert!(all.iter().any(|text| text.starts_with("[4] Loaded commits 1 max200")));
        assert!(all.iter().any(|text| text.contains("Space stage/unstage")));
    }

    #[test]
    fn the_status_panel_sits_above_the_lists_and_the_files_hints_show_by_default() {
        let mut workspace = Workspace::new(snapshot(2));
        workspace.open();
        let panels = layout(1040, 680, GRID);
        assert!(panels.status.y < panels.files.y && panels.files.y < panels.branches.y);
        assert!(panels.branches.y < panels.commits.y && panels.commits.y + panels.commits.height <= panels.footer_top);
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        assert!(texts(&draws).iter().any(|text| text.contains("a all")), "the Files hints show while Files has focus");
    }

    #[test]
    fn an_empty_file_list_says_the_tree_is_clean() {
        let workspace = Workspace::new(snapshot(0));
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        assert!(texts(&draws).contains(&"Working tree clean"));
    }

    #[test]
    fn the_selected_row_is_highlighted_only_in_its_own_list() {
        let workspace = Workspace::new(snapshot(2));
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        let highlights =
            draws.iter().filter(|draw| matches!(draw, Draw::Rect { colour, .. } if *colour == SELECTION)).count();
        assert_eq!(highlights, 1, "one focused selection in the files list");
    }

    #[test]
    fn pending_work_is_shown_in_the_footer() {
        let workspace = Workspace::new(snapshot(1));
        let draws = scene(&workspace, 1040, 680, GRID, 2);
        assert!(texts(&draws).iter().any(|text| text.ends_with("working...")));
    }

    #[test]
    fn a_wide_list_row_is_cut_inside_its_panel() {
        let mut snapshot = snapshot(1);
        snapshot.files[0].label = "x".repeat(400);
        let workspace = Workspace::new(snapshot);
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        let row = texts(&draws).into_iter().find(|text| text.starts_with("xxxx")).expect("row");
        assert!(row.len() < 400 && row.ends_with(".."));
    }

    #[test]
    fn a_long_diff_draws_only_the_rows_that_fit_in_the_main_panel() {
        let mut workspace = Workspace::new(snapshot(1));
        let Job::Main { generation, .. } = (match workspace.open() {
            Effect::Send(job) => job,
            other => panic!("expected a job, got {other:?}"),
        }) else {
            panic!("main job expected")
        };
        let text: String = (0..5_000).map(|index| format!("+line {index}\n")).collect();
        workspace.reply(Reply::Main {
            generation,
            view: MainView::text(&text),
        });
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        let drawn = texts(&draws).iter().filter(|text| text.starts_with("+line ")).count();
        assert!(drawn > 0 && drawn < 100, "drew {drawn} diff rows");
    }

    #[test]
    fn a_pointer_over_a_list_row_or_the_main_panel_hits_that_part() {
        let workspace = Workspace::new(snapshot(3));
        let surface = Surface { width: 1040, height: 680, grid: GRID };
        let panels = layout(1040, 680, GRID);
        let row_y = filtered_inner_of(panels.files, GRID).top + GRID.row * 2 + 1;
        assert_eq!(hit_test(&workspace, surface, panels.files.x + 5, row_y), Hit::Row(Pane::Files, 2));
        assert_eq!(hit_test(&workspace, surface, panels.main.x + 5, panels.main.y + 1), Hit::Main(None));
        assert_eq!(hit_test(&workspace, surface, 1, 1), Hit::Nothing, "the header is not a target");
    }

    #[test]
    fn a_wheel_notch_moves_three_rows_up_for_a_positive_delta() {
        assert_eq!(wheel_lines(WheelDelta::Lines { x: 0.0, y: 1.0 }, 20, 1.0), -3);
        assert_eq!(wheel_lines(WheelDelta::Lines { x: 0.0, y: -1.0 }, 20, 1.0), 3);
        assert_eq!(wheel_lines(WheelDelta::LogicalPixels { x: 0.0, y: -40.0 }, 20, 2.0), 4);
    }

    #[test]
    fn the_main_panel_shows_the_help_when_asked() {
        let mut workspace = Workspace::new(snapshot(1));
        workspace.key(Key::Char('?'));
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        assert!(texts(&draws).contains(&"Keys"));
        assert!(texts(&draws).iter().any(|text| text.starts_with("Tab          next pane")));
    }

    #[test]
    fn main_view_text_is_drawn_from_the_scroll_offset() {
        let mut workspace = Workspace::new(snapshot(1));
        let Effect::Send(Job::Main { generation, .. }) = workspace.open() else { panic!(); };
        workspace.reply(Reply::Main { generation, view: MainView::text("alpha\nbeta\ngamma") });
        workspace.key(Key::Enter);
        workspace.key(Key::Down);
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        let all = texts(&draws);
        assert!(!all.contains(&"alpha"), "the first line scrolled away");
        assert!(all.contains(&"beta"));
    }
}

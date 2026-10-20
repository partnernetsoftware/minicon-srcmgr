//! Native pixel window for the status workspace. The event loop, surface and
//! presentation belong to MiniCon's platform layer; this module supplies the
//! application: a panel layout built from plain drawing commands, glyph masks
//! from the platform font composited with MiniCon's pixel kernels, and a worker
//! thread that runs Git so the window never waits on it.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use minicon_platform::contract::font::RasterGlyph;
use minicon_platform::contract::input::{KeyPressState, LogicalKey, NamedKey, NormalizedKeyEvent};
use minicon_platform::font;
use minicon_platform::window_host::{
    LogicalSize, PixelWindow, PixelWindowApplication, PixelWindowDirective, PixelWindowError, PixelWindowEvent,
    PixelWindowOptions, PointerButton, PointerButtonState, WheelDelta, XrgbPixelFrame, run_pixel_window,
};
use minicon_ui_core::GlyphCache;
use minicon_ui_core::pixel::{blend_mask_xrgb, fill_xrgb_rect};

use crate::model::{Backend, Job, Line, Reply, Snapshot, Tone, short};
use crate::workspace::{Confirm, Effect, Key, Pane, PromptKind, Workspace};

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
    "n / d        new branch / delete branch (Branches panel)",
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
    let options = PixelWindowOptions::new(snapshot.title.clone(), LogicalSize { width: 1040.0, height: 680.0 });
    run_pixel_window(options, Box::new(StatusWindow::new(snapshot, backend)))
}

struct StatusWindow {
    workspace: Workspace,
    backend: Option<Box<dyn Backend>>,
    jobs: Option<Sender<Job>>,
    replies: Option<Receiver<Reply>>,
    /// Jobs sent whose replies have not arrived; shown as "working".
    pending: usize,
    glyphs: GlyphCache<(char, u16), Arc<Option<RasterGlyph>>>,
    /// The last pointer position in logical pixels, for events that omit one.
    pointer: Option<(f64, f64)>,
    /// The size and grid of the last frame, so pointer positions can be mapped to panels.
    surface: Option<Surface>,
}

impl StatusWindow {
    fn new(snapshot: Snapshot, backend: Box<dyn Backend>) -> Self {
        Self {
            workspace: Workspace::new(snapshot),
            backend: Some(backend),
            jobs: None,
            replies: None,
            pending: 0,
            glyphs: GlyphCache::with_capacity(GLYPH_CACHE_ENTRIES),
            pointer: None,
            surface: None,
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
        if self.workspace.modal() {
            return Ok(PixelWindowDirective::Continue);
        }
        let scale = window.scale_factor()?;
        let hit = hit_test(&self.workspace, surface, (x * scale).round() as i64, (y * scale).round() as i64);
        let effect = match (gesture, hit) {
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
            Effect::Nothing => PixelWindowDirective::Continue,
            Effect::Quit => PixelWindowDirective::Exit,
            Effect::Send(job) => {
                let sent = self.jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_ok());
                if sent {
                    self.pending += 1;
                } else {
                    let _ = self.workspace.reply(Reply::Notice("the Git worker has stopped".to_owned()));
                }
                PixelWindowDirective::Continue
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
            pen += style.advance.round() as i64;
        }
        Ok(())
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
        match event {
            PixelWindowEvent::CloseRequested => Ok(PixelWindowDirective::Exit),
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
                window.request_redraw();
                Ok(directive)
            }
            PixelWindowEvent::Keyboard(event) => {
                let Some(key) = key_of(&event) else {
                    return Ok(PixelWindowDirective::Continue);
                };
                let effect = self.workspace.key(key);
                let directive = self.dispatch(effect);
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
        }
    }

    fn render(
        &mut self,
        window: &PixelWindow,
        frame: &mut XrgbPixelFrame<'_>,
    ) -> Result<PixelWindowDirective, PixelWindowError> {
        let scale = window.scale_factor()?;
        let size = (BASE_SIZE_PX * scale).round().max(8.0) as u16;
        let metrics = font::primary_metrics(size)
            .map_err(|error| PixelWindowError::failed("status_font_metrics", format!("{error:?}")))?;
        let row = metrics.cell_height.ceil().max(1.0) as i64;
        let grid = Grid {
            row,
            ascent: metrics.ascent.round() as i64,
            advance: metrics.cell_width,
            margin: (MARGIN_PX * scale).round() as i64,
            gap: (row / 3).max(2),
        };
        let width = frame.width();
        let height = frame.height();
        self.surface = Some(Surface { width: i64::from(width), height: i64::from(height), grid });
        // The main panel's visible rows, so the cursor stays on screen after a resize.
        let main_rows = inner_of(layout(i64::from(width), i64::from(height), grid).main, grid).rows;
        self.workspace.set_view_rows(main_rows.max(0) as usize);
        let draws = scene(&self.workspace, i64::from(width), i64::from(height), grid, self.pending);
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
    }

    fn about_to_wait(
        &mut self,
        _window: &PixelWindow,
        _now: Instant,
    ) -> Result<PixelWindowDirective, PixelWindowError> {
        Ok(PixelWindowDirective::Wait)
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
        text: fit(&header, grid.columns(width - 2 * margin)),
        colour: colour(Tone::Heading),
    });

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
    let file_rows: Vec<Line> =
        snapshot.files.iter().map(|file| Line { tone: file.tone, text: file.label.clone() }).collect();
    list_panel(
        &mut draws,
        files_rect,
        grid,
        Panel {
            focused: focus == Pane::Files,
            title: &format!("[2] Files ({})", snapshot.files.len()),
            empty: "Working tree clean",
        },
        &file_rows,
        selected(Pane::Files),
    );
    let branch_rows: Vec<Line> = snapshot
        .branches
        .iter()
        .map(|branch| Line {
            tone: if branch.current { Tone::Heading } else { Tone::Text },
            text: format!("{} {}", if branch.current { '*' } else { ' ' }, branch.label),
        })
        .collect();
    list_panel(
        &mut draws,
        branches_rect,
        grid,
        Panel {
            focused: focus == Pane::Branches,
            title: &format!("[3] Branches ({})", snapshot.branches.len()),
            empty: "No local branches",
        },
        &branch_rows,
        selected(Pane::Branches),
    );
    let commit_rows: Vec<Line> = snapshot
        .commits
        .iter()
        .map(|commit| Line {
            tone: Tone::Text,
            text: format!("{}{} {}", if commit.merge { 'M' } else { ' ' }, short(&commit.id), commit.subject),
        })
        .collect();
    list_panel(
        &mut draws,
        commits_rect,
        grid,
        Panel {
            focused: focus == Pane::Commits,
            title: &format!("[4] Commits ({})", snapshot.commits.len()),
            empty: "No commits yet",
        },
        &commit_rows,
        selected(Pane::Commits),
    );

    main_panel(&mut draws, main_rect, grid, workspace);

    let notice = workspace.notice().unwrap_or("");
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
    let body_top = margin + row + gap;
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

fn hit_test(workspace: &Workspace, surface: Surface, x: i64, y: i64) -> Hit {
    let grid = surface.grid;
    let panels = layout(surface.width, surface.height, grid);
    let snapshot = workspace.snapshot();
    let lists = [
        (Pane::Status, panels.status, 1),
        (Pane::Files, panels.files, snapshot.files.len()),
        (Pane::Branches, panels.branches, snapshot.branches.len()),
        (Pane::Commits, panels.commits, snapshot.commits.len()),
    ];
    for (pane, rect, length) in lists {
        if !contains(rect, x, y) {
            continue;
        }
        let inner = inner_of(rect, grid);
        if y < inner.top || y >= inner.top + inner.rows * grid.row {
            return Hit::Panel(pane);
        }
        let offset = window_start(workspace.selected(pane), length, inner.rows);
        return Hit::Row(pane, offset + ((y - inner.top) / grid.row) as usize);
    }
    if contains(panels.main, x, y) {
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
        Hit::Nothing => None,
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
fn main_panel(draws: &mut Vec<Draw>, rect: Rect, grid: Grid, workspace: &Workspace) {
    let focused = workspace.focus() == Pane::Main || workspace.prompt().is_some();
    if workspace.help() {
        let lines: Vec<Line> =
            HELP_LINES.iter().map(|text| Line { tone: Tone::Text, text: (*text).to_owned() }).collect();
        text_panel(draws, rect, grid, true, "Keys", &lines, 0, None);
        return;
    }
    if let Some(prompt) = workspace.prompt() {
        let title = match prompt.kind() {
            PromptKind::Commit => "Commit message",
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
            Confirm::DeleteBranch(name) => format!("Delete branch {name}?   y deletes it   n keeps it"),
        };
        text_panel(draws, rect, grid, true, "Confirm", &[Line { tone: Tone::Removed, text }], 0, None);
        return;
    }
    let main = workspace.main();
    let title = match workspace.source() {
        Pane::Commits => "Commit",
        Pane::Branches => "Branch",
        Pane::Status | Pane::Files | Pane::Main => "Diff",
    };
    let highlight = if focused { Some(workspace.selection()) } else { None };
    text_panel(draws, rect, grid, focused, title, &main.lines, workspace.scroll(), highlight);
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
    if length <= rows || selected < rows { 0 } else { (selected + 1 - rows).min(length - rows) }
}

/// Keys the window shows for the current state, most useful first.
fn hints(workspace: &Workspace) -> String {
    if workspace.help() {
        return "Esc close help".to_owned();
    }
    if workspace.confirm().is_some() {
        return "y confirm   n keep".to_owned();
    }
    if let Some(prompt) = workspace.prompt() {
        return match prompt.kind() {
            PromptKind::Commit => "Ctrl+Enter commit   Enter new line   Esc save draft".to_owned(),
            PromptKind::NewBranch => "Enter create   Esc cancel".to_owned(),
        };
    }
    let focus = match workspace.focus() {
        Pane::Status => "2 3 4 focus lists",
        Pane::Files => "Space stage/unstage   a all   c commit   Enter diff",
        Pane::Branches => "Space checkout   n new   d delete   Enter details",
        Pane::Commits => "Enter details",
        Pane::Main if workspace.anchor_set() => "Space stage lines   v end range   a stage hunk   Esc back",
        Pane::Main => "Space stage line   v range   a stage hunk   h/l hunk   Esc back",
    };
    format!("{focus}   Tab panels   ? help   q quit")
}

/// Shortens `text` to `columns` characters, marking the cut with "..". Control
/// characters become spaces so they cannot disturb the layout.
fn fit(text: &str, columns: usize) -> String {
    let clean: Vec<char> = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if clean.len() <= columns {
        return clean.into_iter().collect();
    }
    if columns <= 2 {
        return clean.into_iter().take(columns).collect();
    }
    let mut out: String = clean.into_iter().take(columns - 2).collect();
    out.push_str("..");
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
    match &event.logical {
        LogicalKey::Named(named) => Some(match named {
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
            NamedKey::Tab if shift => Key::BackTab,
            NamedKey::Tab => Key::Tab,
            NamedKey::Space => Key::Space,
            NamedKey::Backspace => Key::Backspace,
            _ => return None,
        }),
        LogicalKey::Character(text) => {
            if control {
                return match text.as_str() {
                    "c" | "C" => Some(Key::CtrlC),
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

    fn snapshot(files: usize) -> Snapshot {
        Snapshot {
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
        assert!(all.iter().any(|text| text.starts_with("[4] Commits (1)")));
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
        let row_y = inner_of(panels.files, GRID).top + GRID.row * 2 + 1;
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
        workspace.reply(Reply::Main { generation: 0, view: MainView::text("alpha\nbeta\ngamma") });
        workspace.key(Key::Enter);
        workspace.key(Key::Down);
        let draws = scene(&workspace, 1040, 680, GRID, 0);
        let all = texts(&draws);
        assert!(!all.contains(&"alpha"), "the first line scrolled away");
        assert!(all.contains(&"beta"));
    }
}

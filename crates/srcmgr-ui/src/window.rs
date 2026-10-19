//! Native pixel window for the status view. The event loop, surface and
//! presentation belong to MiniCon's platform layer; this module supplies only
//! the application: glyph masks from the platform font, composited with
//! MiniCon's pixel kernels into the XRGB frame it provides.

use std::sync::Arc;
use std::time::Instant;

use minicon_platform::contract::font::RasterGlyph;
use minicon_platform::contract::input::{KeyPressState, LogicalKey, NamedKey, NormalizedKeyEvent};
use minicon_platform::font;
use minicon_platform::window_host::{
    LogicalSize, PixelWindow, PixelWindowApplication, PixelWindowDirective, PixelWindowError,
    PixelWindowEvent, PixelWindowOptions, XrgbPixelFrame, run_pixel_window,
};
use minicon_ui_core::GlyphCache;
use minicon_ui_core::pixel::{blend_mask_xrgb, fill_xrgb_rect};

use crate::view::{StatusView, Tone};

const BASE_SIZE_PX: f64 = 16.0;
const MARGIN_PX: f64 = 16.0;
const GLYPH_CACHE_ENTRIES: usize = 512;
const BACKGROUND: u32 = 0x001E_1E1E;
const SELECTION: u32 = 0x0026_4F78;

fn colour(tone: Tone) -> u32 {
    match tone {
        Tone::Heading => 0x0056_9CD6,
        Tone::Text => 0x00D4_D4D4,
        Tone::Muted => 0x0080_8080,
        Tone::Staged => 0x00B5_CEA8,
        Tone::Changed => 0x00DC_DCAA,
        Tone::Untracked => 0x0073_C991,
        Tone::Added => 0x0068_D391,
        Tone::Removed => 0x00F4_8771,
        Tone::Hunk => 0x0056_9CD6,
    }
}

/// Switches to the next local branch on behalf of the window and returns the
/// status view that results. The caller owns trust checks and the Git work;
/// an error is shown to the user as a notice and leaves the view unchanged.
pub trait BranchCycler {
    fn next_branch(&mut self) -> Result<StatusView, String>;
}

/// Opens the status window and blocks until the user closes it. `B` asks the
/// cycler for the next branch.
pub fn show(view: StatusView, cycler: Box<dyn BranchCycler>) -> Result<(), PixelWindowError> {
    let options = PixelWindowOptions::new(
        view.title.clone(),
        LogicalSize {
            width: 960.0,
            height: 640.0,
        },
    );
    run_pixel_window(options, Box::new(StatusWindow::new(view, cycler)))
}

struct StatusWindow {
    view: StatusView,
    cycler: Box<dyn BranchCycler>,
    glyphs: GlyphCache<(char, u16), Arc<Option<RasterGlyph>>>,
}

impl StatusWindow {
    fn new(view: StatusView, cycler: Box<dyn BranchCycler>) -> Self {
        Self {
            view,
            cycler,
            glyphs: GlyphCache::with_capacity(GLYPH_CACHE_ENTRIES),
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
    fn draw_text(&mut self, canvas: &mut Canvas<'_>, at: Pen, text: &str, style: Style) -> Result<(), PixelWindowError> {
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
        window.request_redraw();
        Ok(PixelWindowDirective::Continue)
    }

    fn event(
        &mut self,
        window: &PixelWindow,
        event: PixelWindowEvent,
    ) -> Result<PixelWindowDirective, PixelWindowError> {
        match event {
            PixelWindowEvent::CloseRequested => Ok(PixelWindowDirective::Exit),
            PixelWindowEvent::Keyboard(key) => {
                if let Some(forward) = arrow_direction(&key) {
                    let before = self.view.selected();
                    self.view.move_selection(forward);
                    if self.view.selected() != before {
                        window.request_redraw();
                    }
                } else if is_branch_key(&key) {
                    match self.cycler.next_branch() {
                        Ok(next) => self.view = next,
                        Err(message) => self.view.set_notice(&message),
                    }
                    window.request_redraw();
                }
                Ok(PixelWindowDirective::Continue)
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
        let line_height = metrics.cell_height.ceil().max(1.0) as i64;
        let ascent = metrics.ascent.round() as i64;
        let advance = metrics.cell_width;
        let margin = (MARGIN_PX * scale).round() as i64;

        let width = frame.width();
        let height = frame.height();
        let lines = self.view.lines.clone();
        let title = self.view.title.clone();
        let selected_row =
            (self.view.entry_count() > 0).then(|| self.view.first_row_index() + self.view.selected());
        let diff = self.view.selected_diff().to_vec();
        let mut canvas = Canvas {
            pixels: frame.pixels_mut(),
            stride: width,
            height,
        };
        fill_xrgb_rect(canvas.pixels, width, 0, 0, width, height, BACKGROUND);

        let style = |colour: u32| Style {
            size,
            colour,
            advance,
            ascent,
        };
        let title_at = Pen {
            x: margin,
            baseline: margin + ascent,
        };
        self.draw_text(&mut canvas, title_at, &title, style(colour(Tone::Muted)))?;

        let mut baseline = margin + line_height + ascent;
        for (index, line) in lines.iter().enumerate() {
            if Some(index) == selected_row {
                let top = baseline - ascent;
                fill_xrgb_rect(canvas.pixels, width, 0, top.max(0) as u32, width, line_height as u32, SELECTION);
            }
            self.draw_text(&mut canvas, Pen { x: margin, baseline }, &line.text, style(colour(line.tone)))?;
            baseline += line_height;
        }

        // The diff of the selected entry sits below the list, separated by a gap.
        baseline += line_height;
        for line in &diff {
            if baseline > i64::from(height) {
                break;
            }
            self.draw_text(&mut canvas, Pen { x: margin, baseline }, &line.text, style(colour(line.tone)))?;
            baseline += line_height;
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

/// True for a press of the `B` key, with or without Shift.
fn is_branch_key(key: &NormalizedKeyEvent) -> bool {
    if key.state != KeyPressState::Pressed {
        return false;
    }
    matches!(&key.logical, LogicalKey::Character(text) if text == "b" || text == "B")
}

/// `Some(true)` for Down, `Some(false)` for Up, and `None` for any other key
/// or for a release, so only presses move the selection.
fn arrow_direction(key: &NormalizedKeyEvent) -> Option<bool> {
    if key.state != KeyPressState::Pressed {
        return None;
    }
    match &key.logical {
        LogicalKey::Named(NamedKey::ArrowDown) => Some(true),
        LogicalKey::Named(NamedKey::ArrowUp) => Some(false),
        _ => None,
    }
}

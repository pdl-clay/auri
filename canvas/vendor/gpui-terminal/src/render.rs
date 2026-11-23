//! Terminal rendering.
//!
//! The renderer keeps a per-row cache of shaped text runs and background
//! rectangles. Terminal damage tracking from alacritty tells it which rows
//! changed, so a frame only reshapes the rows that actually received output.
//! Moving or zooming a card therefore does not reshape the whole grid.
//!
//! # Rendering Pipeline
//!
//! ```text
//! Terminal damage → rebuild damaged rows → paint cached rows
//!                         │                     │
//!                         ├─ merge backgrounds  ├─ default background
//!                         ├─ batch text runs    ├─ row backgrounds
//!                         └─ shape each run     ├─ shaped text runs
//!                                               └─ cursor
//! ```
//!
//! Text is batched into contiguous runs with identical styling. A row of
//! alternating colors therefore costs one shaping call per run instead of one
//! per character, and unchanged rows cost nothing between frames.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::colors::ColorPalette;
use crate::event::GpuiEventProxy;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Term, TermDamage};
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use gpui::{
    App, Bounds, Edges, Font, FontFeatures, FontStyle, FontWeight, Hsla, Pixels, Point,
    ShapedLine, SharedString, Size, TextRun, UnderlineStyle, Window, px, quad, transparent_black,
};

/// A merged background rectangle for one row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundRect {
    /// First covered column.
    pub start_col: usize,
    /// One past the last covered column.
    pub end_col: usize,
    /// Row the rectangle belongs to.
    pub row: usize,
    /// Resolved background color.
    pub color: Hsla,
}

impl BackgroundRect {
    fn can_merge_with(&self, other: &Self) -> bool {
        self.row == other.row && self.color == other.color && self.end_col == other.start_col
    }
}

/// One contiguous run of equally styled cells.
pub struct PaintedRun {
    /// First column covered by the run.
    pub start_col: usize,
    /// Shaped text, aligned to the cell grid by `force_width`.
    pub shaped: ShapedLine,
}

/// Cached paint data for one terminal row.
#[derive(Default)]
pub struct RowPaint {
    /// Background rectangles, merged while scanning the row.
    pub backgrounds: Vec<BackgroundRect>,
    /// Non-blank text runs.
    pub runs: Vec<PaintedRun>,
}

/// Measured monospace cell metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    /// Advance width of one cell.
    pub cell_width: Pixels,
    /// Height of one cell, including line-height extra space.
    pub cell_height: Pixels,
    /// Vertical offset that centers text in its cell.
    pub vertical_offset: Pixels,
}

#[derive(Clone, Debug, PartialEq)]
struct MetricsKey {
    font_family: String,
    font_size: Pixels,
    line_height_multiplier: f32,
}

struct MetricsState {
    key: MetricsKey,
    metrics: CellMetrics,
}

#[derive(Default)]
struct RenderCache {
    metrics: Option<MetricsState>,
    rows: Vec<RowPaint>,
    columns: usize,
    full_rebuild: bool,
    font_settle_until: Option<Instant>,
}

impl RenderCache {
    fn invalidate(&mut self) {
        self.full_rebuild = true;
    }
}

/// Terminal renderer with cached, damage-tracked row layout.
///
/// The cache is shared between clones so the paint closure can keep a clone
/// without losing the shaped rows.
#[derive(Clone)]
pub struct TerminalRenderer {
    /// Font family name (e.g. `"DejaVu Sans Mono"`).
    pub font_family: String,
    /// Font size in pixels.
    pub font_size: Pixels,
    /// Width of a single character cell.
    pub cell_width: Pixels,
    /// Height of a single character cell (line height).
    pub cell_height: Pixels,
    /// Multiplier applied to the font's ascent + descent.
    pub line_height_multiplier: f32,
    /// Palette used to resolve terminal colors.
    pub palette: ColorPalette,
    cache: Rc<RefCell<RenderCache>>,
}

impl TerminalRenderer {
    /// Create a renderer; cell metrics are measured on the first paint.
    pub fn new(
        font_family: String,
        font_size: Pixels,
        line_height_multiplier: f32,
        palette: ColorPalette,
    ) -> Self {
        let cell_width = font_size * 0.6;
        let cell_height = font_size * 1.4;

        Self {
            font_family,
            font_size,
            cell_width,
            cell_height,
            line_height_multiplier,
            palette,
            cache: Rc::new(RefCell::new(RenderCache::default())),
        }
    }

    /// Force every row to be reshaped on the next paint.
    pub fn mark_full_rebuild(&self) {
        self.cache.borrow_mut().invalidate();
    }

    /// Keep the current grid size while a font change settles.
    pub fn settle_font_for(&self, duration: Duration) {
        self.cache.borrow_mut().font_settle_until = Some(Instant::now() + duration);
    }

    /// Clear the settle window and report whether the grid may resize now.
    pub fn finish_font_settle(&self) {
        self.cache.borrow_mut().font_settle_until = None;
    }

    /// Whether a font change is still settling and the PTY must not resize yet.
    pub fn font_is_settling(&self) -> bool {
        self.cache
            .borrow()
            .font_settle_until
            .is_some_and(|deadline| Instant::now() < deadline)
    }

    /// Measure the font once per size/family change.
    pub fn cell_metrics(&self, window: &mut Window) -> CellMetrics {
        let key = MetricsKey {
            font_family: self.font_family.clone(),
            font_size: self.font_size,
            line_height_multiplier: self.line_height_multiplier,
        };

        let mut cache = self.cache.borrow_mut();
        if let Some(state) = cache.metrics.as_ref()
            && state.key == key
        {
            return state.metrics;
        }

        let (cell_width, cell_height) = measure_cell(
            window,
            &self.font_family,
            self.font_size,
            self.line_height_multiplier,
        );
        let vertical_offset = (cell_height - cell_height / self.line_height_multiplier) / 2.0;
        let metrics = CellMetrics {
            cell_width,
            cell_height,
            vertical_offset,
        };
        cache.metrics = Some(MetricsState { key, metrics });
        cache.invalidate();
        metrics
    }

    /// Measure cell dimensions and store them on the renderer.
    ///
    /// Kept for callers that want the metrics outside of `paint`.
    pub fn measure_cell(&mut self, window: &mut Window) {
        let metrics = self.cell_metrics(window);
        self.cell_width = metrics.cell_width;
        self.cell_height = metrics.cell_height;
    }

    /// Paint the terminal grid, reshaping only rows reported as damaged.
    pub fn paint(
        &self,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        term: &mut Term<GpuiEventProxy>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let metrics = self.cell_metrics(window);
        let rows = term.screen_lines();
        let columns = term.columns();
        let default_bg = self.palette.resolve(
            Color::Named(NamedColor::Background),
            term.colors(),
        );

        // `damage` marks the cursor line on every call, so at least one row is
        // rebuilt while the terminal is open. That keeps the cost proportional
        // to changed lines instead of the whole screen.
        let mut damaged = vec![false; rows];
        let mut rebuild_all = {
            let cache = self.cache.borrow();
            cache.full_rebuild || cache.rows.len() != rows || cache.columns != columns
        };
        match term.damage() {
            TermDamage::Full => rebuild_all = true,
            TermDamage::Partial(lines) => {
                for line in lines {
                    if line.line < rows {
                        damaged[line.line] = true;
                    }
                }
            }
        }

        {
            let mut cache = self.cache.borrow_mut();
            if rebuild_all {
                cache.rows = (0..rows).map(|_| RowPaint::default()).collect();
                cache.columns = columns;
                cache.full_rebuild = false;
            }
            if rebuild_all {
                for (line, row) in cache.rows.iter_mut().enumerate() {
                    self.rebuild_row(term, line, &metrics, row, window);
                }
            } else {
                for (line, row) in cache.rows.iter_mut().enumerate() {
                    if damaged[line] {
                        self.rebuild_row(term, line, &metrics, row, window);
                    }
                }
            }
        }
        term.reset_damage();

        let cursor = term.grid().cursor.point;
        let cursor_color = self
            .palette
            .resolve(Color::Named(NamedColor::Cursor), term.colors());

        // Paint from the cache. The scene is rebuilt every frame, so cached
        // runs are replayed without touching the text system.
        window.paint_quad(quad(
            bounds,
            px(0.0),
            default_bg,
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));

        let origin = Point {
            x: bounds.origin.x + padding.left,
            y: bounds.origin.y + padding.top,
        };

        let cache = self.cache.borrow();
        for (line, row) in cache.rows.iter().enumerate() {
            for background in &row.backgrounds {
                if background.color == default_bg {
                    continue;
                }
                let y = origin.y + metrics.cell_height * (line as f32);
                let x = origin.x + metrics.cell_width * (background.start_col as f32);
                let width = metrics.cell_width * ((background.end_col - background.start_col) as f32);
                window.paint_quad(quad(
                    Bounds {
                        origin: Point { x, y },
                        size: Size {
                            width,
                            height: metrics.cell_height,
                        },
                    },
                    px(0.0),
                    background.color,
                    Edges::<Pixels>::default(),
                    transparent_black(),
                    Default::default(),
                ));
            }

            let y = origin.y
                + metrics.cell_height * (line as f32)
                + metrics.vertical_offset;
            for run in &row.runs {
                let x = origin.x + metrics.cell_width * (run.start_col as f32);
                let _ = run.shaped.paint(
                    Point { x, y },
                    metrics.cell_height,
                    window,
                    cx,
                );
            }
        }

        let cursor_x = origin.x + metrics.cell_width * (cursor.column.0 as f32);
        let cursor_y = origin.y + metrics.cell_height * (cursor.line.0 as f32);
        window.paint_quad(quad(
            Bounds {
                origin: Point {
                    x: cursor_x,
                    y: cursor_y,
                },
                size: Size {
                    width: metrics.cell_width,
                    height: metrics.cell_height,
                },
            },
            px(0.0),
            cursor_color,
            Edges::<Pixels>::default(),
            transparent_black(),
            Default::default(),
        ));
    }

    /// Reshape one row into its cached backgrounds and text runs.
    fn rebuild_row(
        &self,
        term: &Term<GpuiEventProxy>,
        line: usize,
        metrics: &CellMetrics,
        out: &mut RowPaint,
        window: &mut Window,
    ) {
        out.backgrounds.clear();
        out.runs.clear();

        let grid = term.grid();
        let colors = term.colors();
        let columns = grid.columns();

        let mut current_style: Option<(Color, Color, Flags)> = None;
        let mut text = String::new();
        let mut start_col = 0usize;
        let mut bg: Option<BackgroundRect> = None;

        let flush_run = |out: &mut RowPaint,
                         start_col: usize,
                         text: &mut String,
                         style: Option<(Color, Color, Flags)>,
                         window: &mut Window| {
            let Some((fg_color, _bg_color, flags)) = style else {
                return;
            };
            let trimmed_start = text.chars().take_while(|c| *c == ' ').count();
            let trimmed = text.trim_matches(' ').to_owned();
            text.clear();
            if trimmed.is_empty() {
                return;
            }
            let fg = self.palette.resolve(fg_color, colors);
            let font = Font {
                family: self.font_family.clone().into(),
                features: FontFeatures::default(),
                fallbacks: None,
                weight: if flags.contains(Flags::BOLD) {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                },
                style: if flags.contains(Flags::ITALIC) {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
            };
            let text_run = TextRun {
                len: trimmed.len(),
                font,
                color: fg,
                background_color: None,
                underline: if flags.contains(Flags::UNDERLINE) {
                    Some(UnderlineStyle {
                        thickness: px(1.0),
                        color: Some(fg),
                        wavy: false,
                    })
                } else {
                    None
                },
                strikethrough: None,
            };
            let shaped = window.text_system().shape_line(
                SharedString::from(trimmed),
                self.font_size,
                &[text_run],
                Some(metrics.cell_width),
            );
            out.runs.push(PaintedRun {
                start_col: start_col + trimmed_start,
                shaped,
            });
            text.clear();
        };

        for column in 0..columns {
            let cell: &Cell = &grid[AlacPoint::new(Line(line as i32), Column(column))];
            let fg = cell.fg;
            let cell_bg = cell.bg;
            let flags = cell.flags;
            let bg_color = self.palette.resolve(cell_bg, colors);

            // Background rectangles merge independently of text styling.
            let candidate = BackgroundRect {
                start_col: column,
                end_col: column + 1,
                row: line,
                color: bg_color,
            };
            if let Some(rect) = bg.as_mut() {
                if rect.can_merge_with(&candidate) {
                    rect.end_col = candidate.end_col;
                } else {
                    out.backgrounds.push(*rect);
                    bg = Some(candidate);
                }
            } else {
                bg = Some(candidate);
            }

            let style = (fg, cell_bg, flags);
            if current_style != Some(style) {
                flush_run(out, start_col, &mut text, current_style, window);
                current_style = Some(style);
                start_col = column;
            }

            let ch = if cell.c == '\0' { ' ' } else { cell.c };
            text.push(ch);
        }
        flush_run(out, start_col, &mut text, current_style, window);
        if let Some(rect) = bg {
            out.backgrounds.push(rect);
        }
    }
}

/// Shape `M` once to obtain the monospace cell metrics for this font size.
fn measure_cell(
    window: &mut Window,
    font_family: &str,
    font_size: Pixels,
    line_height_multiplier: f32,
) -> (Pixels, Pixels) {
    let font = Font {
        family: font_family.to_string().into(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    let text_run = TextRun {
        len: 1,
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line("M".into(), font_size, &[text_run], None);

    let cell_width = if shaped.width > px(0.0) {
        shaped.width
    } else {
        font_size * 0.6
    };
    let base_height = shaped.ascent + shaped.descent;
    let cell_height = if base_height > px(0.0) {
        base_height * line_height_multiplier
    } else {
        font_size * 1.4
    };
    (cell_width, cell_height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::term::color::Colors;

    #[test]
    fn background_rects_merge_only_when_adjacent_and_equal() {
        let black = gpui::black();
        let rect = BackgroundRect {
            start_col: 0,
            end_col: 5,
            row: 0,
            color: black,
        };
        assert!(rect.can_merge_with(&BackgroundRect {
            start_col: 5,
            end_col: 10,
            row: 0,
            color: black,
        }));
        assert!(!rect.can_merge_with(&BackgroundRect {
            start_col: 6,
            end_col: 10,
            row: 0,
            color: black,
        }));
        assert!(!rect.can_merge_with(&BackgroundRect {
            start_col: 5,
            end_col: 10,
            row: 1,
            color: black,
        }));
    }

    #[test]
    fn renderer_reports_stale_metrics_after_font_change() {
        let renderer =
            TerminalRenderer::new("monospace".into(), px(14.0), 1.2, ColorPalette::default());
        assert!(!renderer.font_is_settling());
        renderer.settle_font_for(Duration::from_millis(150));
        assert!(renderer.font_is_settling());
        renderer.mark_full_rebuild();
        renderer.finish_font_settle();
        assert!(!renderer.font_is_settling());
    }

    #[test]
    fn palette_resolves_named_foreground() {
        let palette = ColorPalette::default();
        let colors = Colors::default();
        let resolved = palette.resolve(Color::Named(NamedColor::Foreground), &colors);
        assert!(resolved.a > 0.0);
    }
}

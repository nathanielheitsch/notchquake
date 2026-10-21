//! CoreText rendering of the terminal grid, inside the silhouette clip.
//!
//! One `CTLine` per maximal same-color run of a row: CoreText shapes the run,
//! so `->`, `=>`, `!=` ligate, while the per-run placement keeps the grid
//! monospaced — every run starts on a cell boundary even when a ligature glyph
//! is wider than the cells it covers.
//!
//! The font is attached as a `kCTFontAttributeName` attribute on a
//! `CFMutableAttributedString`; the text color comes from the CGContext fill
//! color, which is what `CTLineDraw` uses.
//!
//! The view is NOT flipped, so the context is the window's y-up space:
//! `row_rect` puts row 0 at the TOP (`y = PANEL_HEIGHT - cell_height`),
//! decreasing with the row index; each row is drawn upright.

use objc2_core_graphics::{CGContext, CGColor};
use objc2_core_text::{
    CTFont,
    CTLine,
    kCTFontAttributeName,
    kCTForegroundColorFromContextAttributeName,
    kCTLigatureAttributeName,
};
use objc2_core_foundation::{
    CFBoolean,
    CFMutableAttributedString,
    CFNumber,
    CFRange,
    CFString,
    CFRetained,
    CGPoint,
    CGRect,
    CGSize,
    CGFloat,
};
use objc2_core_foundation::kCFBooleanTrue;

use std::ptr::null;

use crate::settings::{CursorShape, Settings};
use crate::consts::{PANEL_HEIGHT};
use crate::terminal::CellSnapshot;

/// Horizontal padding of the grid inside the silhouette (each side).
pub const GRID_PAD: CGFloat = 8.0;

/// Vertical padding at the top of the grid: row 0 must clear the silhouette's
/// rounded top corners and top edge, whose taper reaches ~20pt down each side.
pub const GRID_TOP_PAD: CGFloat = 22.0;
/// Alpha of the filled cursor block.
const CURSOR_ALPHA: CGFloat = 0.85;

/// Thickness of the bar/underline cursor, as a fraction of the cell it sits in.
const CURSOR_STROKE_FRACTION: CGFloat = 0.12;

/// `x0` is the grid's left edge in CANVAS space (the silhouette's left edge
/// plus `GRID_PAD`, supplied by `notch_view.rs::draw`), and `usable` is the
/// width the grid may occupy — the live silhouette's inner width. Column
/// counting uses `usable`, so the grid reflows with the silhouette; the
/// session's own row/col count is baked from `settings.open_width` at spawn
/// (see `grid_dims`). Vertical placement is unchanged: `row_rect` still hangs
/// rows from `PANEL_HEIGHT`.
pub fn draw_grid(
    ctx: &CGContext,
    grid: &[Vec<CellSnapshot>],
    cursor: Option<(usize, usize)>,
    font: &CTFont,
    cell_width: CGFloat,
    cell_height: CGFloat,
    height: CGFloat,
    header: CGFloat,
    settings: &Settings,
    x0: CGFloat,
    usable: CGFloat,
) {
    let ctx_opt = Some(ctx);
    let cols = if grid.is_empty() { 0 } else { grid[0].len() };
    let used_cols = if cell_width > 0.0 { (usable / cell_width).floor() as usize } else { cols };
    let used_cols = if used_cols < cols { used_cols } else { cols };

    // y-up space: the baseline sits `descent` ABOVE the row rect's bottom
    // edge, so glyph ink fills [bottom, bottom + ascent + descent] = the cell.
    // `top + ascent` is the y-DOWN formula and floats the text ~8.6pt above
    // its background block (ascent 12.92 vs descent 4.31 at 14pt Fira Code).
    let descent = unsafe { font.descent() };

    // Background pass: only runs whose bg differs from the panel background —
    // the silhouette is already filled with `settings.background`, so cells
    // painted that same color are skipped (a settings bg change repaints the
    // silhouette in `notch_view.rs::draw`, this only keeps the grid in sync).
    let panel_bg = settings.background.rgb();
    for (row, cells) in grid.iter().enumerate() {
        if let Some((top, h)) = row_rect(row, cell_height, height, header) {
            let mut col = 0;
            while col < used_cols {
                let bg = cells[col].background;
                if bg == panel_bg {
                    col += 1;
                    continue;
                }
                let start = col;
                while col + 1 < used_cols && cells[col + 1].background == bg {
                    col += 1;
                }
                CGContext::set_fill_color_with_color(ctx_opt, Some(&srgb(bg)));
                CGContext::fill_rect(
                    ctx_opt,
                    CGRect::new(
                        CGPoint::new(x0 + start as CGFloat * cell_width, top),
                        CGSize::new((col - start + 1) as CGFloat * cell_width, h),
                    ),
                );
                col += 1;
            }
        }
    }

    // Text pass: one CTLine per same-foreground run, drawn upright.
    for (row, cells) in grid.iter().enumerate() {
        if let Some((top, _)) = row_rect(row, cell_height, height, header) {
            let baseline = top + descent;
            let mut col = 0;
            while col < used_cols {
                if cells[col].character == ' ' {
                    col += 1;
                    continue;
                }
                let fg = cells[col].foreground;
                let start = col;
                while col + 1 < used_cols && cells[col + 1].foreground == fg {
                    col += 1;
                }

                let run_text: String = cells[start..col + 1].iter().map(|cell| cell.character).collect();
                let text = CFString::from_str(&run_text);
                let line = unsafe { run(font, &text, settings.ligatures) };
                CGContext::save_g_state(ctx_opt);
                CGContext::translate_ctm(ctx_opt, x0 + start as CGFloat * cell_width, baseline);
                CGContext::set_fill_color_with_color(ctx_opt, Some(&srgb(fg)));
                CGContext::set_text_position(ctx_opt, 0.0, 0.0);
                unsafe { line.draw(ctx) };
                CGContext::restore_g_state(ctx_opt);

                col += 1;
            }
        }
    }

    // Cursor pass. The COLOR comes from settings; the SHAPE is a pure render
    // read (`Term` never sees it): Block fills the cell, Bar is a vertical
    // stroke at the cell's left edge, Underline a horizontal stroke at its
    // bottom edge (y-up space, so the bottom edge is the rect's `top`).
    if let Some((line, column)) = cursor {
        if column < used_cols && line < grid.len() {
            if let Some((top, h)) = row_rect(line, cell_height, height, header) {
                let (cr, cg, cb) = settings.cursor_color.rgb();
                CGContext::set_fill_color_with_color(
                    ctx_opt,
                    Some(&CGColor::new_srgb(cr, cg, cb, CURSOR_ALPHA)),
                );
                let x = x0 + column as CGFloat * cell_width;
                match settings.cursor_shape {
                  CursorShape::Block =>
                    CGContext::fill_rect(
                        ctx_opt,
                        CGRect::new(
                            CGPoint::new(x, top),
                            CGSize::new(cell_width, h),
                        ),
                    ),
                  CursorShape::Bar =>
                    CGContext::fill_rect(
                        ctx_opt,
                        CGRect::new(
                            CGPoint::new(x, top),
                            CGSize::new(cell_width * CURSOR_STROKE_FRACTION, h),
                        ),
                    ),
                  CursorShape::Underline =>
                    CGContext::fill_rect(
                        ctx_opt,
                        CGRect::new(
                            CGPoint::new(x, top),
                            CGSize::new(cell_width, h * CURSOR_STROKE_FRACTION),
                        ),
                    ),
                }
            }
        }
    }
}

/// The (top, height) of grid row `row` in the flipped silhouette space, or
/// `None` once the row is below the silhouette's current bottom edge.
/// The (bottom, height) of grid row `row` in the y-up layer space: the grid
/// hangs from the panel top, and rows below the silhouette bottom
/// (`PANEL_HEIGHT - height`) are skipped.
fn row_rect(row: usize, cell_height: CGFloat, height: CGFloat, header: CGFloat) -> Option<(CGFloat, CGFloat)> {
    let bottom = PANEL_HEIGHT - GRID_TOP_PAD - header - (row as CGFloat + 1.0) * cell_height;
    if bottom < PANEL_HEIGHT - height {
        return None
    }
    Some((bottom, cell_height))
}

/// An sRGB color from `0..1` components.
fn srgb(rgb: (CGFloat, CGFloat, CGFloat)) -> CFRetained<CGColor> {
    let (r, g, b) = rgb;
    CGColor::new_srgb(r, g, b, 1.0)
}

/// A `CTLine` for `text` shaped by `font`, with the font attribute set over
/// the FULL string range (a zero-length range is the INSERT range for
/// `replace_string`, NOT the range of the inserted text — using it for
/// `set_attribute` is a no-op, so the font never attaches and `CTLineDraw`
/// falls back to the default font/colour). With the font attached,
/// `CTLineDraw` reads the text colour from the CGContext fill colour the
/// caller sets per-run.
///
/// `ligatures` off sets `kCTLigatureAttributeName` to 0 (essential ligatures
/// only) over the same range. This is the text-attribute path that DOES work
/// for ligature shaping — unlike `kCTFontFeatureSettingsAttribute`, which
/// CoreText/HarfBuzz applies from the FONT, not from the attributed string
/// (AGENTS.md lessons 1/2: a `calt` setting put on the string is a dead end).
/// Fira Code draws `->` as two `.seq` half-glyphs that compose the arrow, so
/// with the attribute off the two halves stay visually separate cells; verify
/// with glyph NAMES, not glyph count, which stays 2 either way (lesson 1).
unsafe fn run(font: &CTFont, text: &CFString, ligatures: bool) -> CFRetained<CTLine> {
    let attr = CFMutableAttributedString::new(None, 0).expect("an attributed string");
    let insert = CFRange { location: 0, length: 0 };
    unsafe { CFMutableAttributedString::replace_string(Some(&*attr), insert, Some(text)) };
    let full = CFRange { location: 0, length: text.length() };
    unsafe {
        CFMutableAttributedString::set_attribute(
            Some(&*attr),
            full,
            Some(kCTFontAttributeName),
            Some(&**font),
        );
        // `CTLineDraw` defaults to BLACK unless the string opts in to the
        // CGContext fill colour. Without this the grid paints black-on-black.
        CFMutableAttributedString::set_attribute(
            Some(&*attr),
            full,
            Some(kCTForegroundColorFromContextAttributeName),
            Some(&*kCFBooleanTrue.expect("a boolean")),
        );
        if !ligatures {
            // 0 = only ligatures ESSENTIAL for proper rendering; the standard
            // (font-default) value is 1. A CFNumber, not a CFBoolean.
            let essential = CFNumber::new_i32(0);
            CFMutableAttributedString::set_attribute(
                Some(&*attr),
                full,
                Some(kCTLigatureAttributeName),
                Some(&*essential),
            );
        }
    };
    unsafe { CTLine::with_attributed_string(&**attr) }
}

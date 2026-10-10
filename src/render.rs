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
};
use objc2_core_foundation::{
    CFBoolean,
    CFMutableAttributedString,
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

use crate::consts::{PANEL_HEIGHT, PANEL_WIDTH};
use crate::terminal::CellSnapshot;

/// Horizontal padding of the grid inside the silhouette (each side).
pub const GRID_PAD: CGFloat = 8.0;

/// Vertical padding at the top of the grid: row 0 must clear the silhouette's
/// rounded top corners and top edge, whose taper reaches ~20pt down each side.
pub const GRID_TOP_PAD: CGFloat = 22.0;
/// The default terminal background: cells painted this color are skipped
/// (the silhouette fill is already black).
const DEFAULT_BG: (CGFloat, CGFloat, CGFloat) = (0.0, 0.0, 0.0);

/// Alpha of the filled cursor block.
const CURSOR_ALPHA: CGFloat = 0.85;

/// Paint the grid background rects, then every text run, then the cursor.
///
/// `grid` is top-to-bottom; rows below the silhouette's current bottom edge
/// (`height`) are skipped — the clip already wipes them, this only avoids
/// off-screen work.
pub fn draw_grid(
    ctx: &CGContext,
    grid: &[Vec<CellSnapshot>],
    cursor: Option<(usize, usize)>,
    font: &CTFont,
    cell_width: CGFloat,
    cell_height: CGFloat,
    height: CGFloat,
) {
    let ctx_opt = Some(ctx);
    let cols = if grid.is_empty() { 0 } else { grid[0].len() };
    let x0 = GRID_PAD;
    let usable = PANEL_WIDTH - 2.0 * GRID_PAD;
    let used_cols = if cell_width > 0.0 { (usable / cell_width).floor() as usize } else { cols };
    let used_cols = if used_cols < cols { used_cols } else { cols };

    let ascent = unsafe { font.ascent() };

    // Background pass: only runs whose bg differs from the silhouette black.
    for (row, cells) in grid.iter().enumerate() {
        if let Some((top, h)) = row_rect(row, cell_height, height) {
            let mut col = 0;
            while col < used_cols {
                let bg = cells[col].background;
                if bg == DEFAULT_BG {
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
        if let Some((top, _)) = row_rect(row, cell_height, height) {
            let baseline = top + ascent;
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
                let line = unsafe { run(font, &text) };
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

    // Cursor pass.
    if let Some((line, column)) = cursor {
        if column < used_cols && line < grid.len() {
            if let Some((top, h)) = row_rect(line, cell_height, height) {
                CGContext::set_fill_color_with_color(
                    ctx_opt,
                    Some(&CGColor::new_srgb(0.75, 0.75, 0.75, CURSOR_ALPHA)),
                );
                CGContext::fill_rect(
                    ctx_opt,
                    CGRect::new(
                        CGPoint::new(x0 + column as CGFloat * cell_width, top),
                        CGSize::new(cell_width, h),
                    ),
                );
            }
        }
    }
}

/// The (top, height) of grid row `row` in the flipped silhouette space, or
/// `None` once the row is below the silhouette's current bottom edge.
/// The (bottom, height) of grid row `row` in the y-up layer space: the grid
/// hangs from the panel top, and rows below the silhouette bottom
/// (`PANEL_HEIGHT - height`) are skipped.
fn row_rect(row: usize, cell_height: CGFloat, height: CGFloat) -> Option<(CGFloat, CGFloat)> {
    let bottom = PANEL_HEIGHT - GRID_TOP_PAD - (row as CGFloat + 1.0) * cell_height;
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
unsafe fn run(font: &CTFont, text: &CFString) -> CFRetained<CTLine> {
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
    };
    CTLine::with_attributed_string(&**attr)
}

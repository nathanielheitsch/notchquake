//! The monospace font used for the terminal grid, and the cell metrics that
//! come out of it.
//!
//! `CTFontCreateWithName` (bound as `CTFont::with_name`) resolves a font
//! family name through the system font fallback, so we ask for "Fira Code"
//! first — it is installed on this machine (`fc-list` reports
//! `FiraCode-Regular.ttf` in `~/Library/Fonts`) — and fall back to "Menlo",
//! which macOS always ships.
//!
//! No font registration happens here: `CTFontManagerRegisterFontsForURL`
//! would need a `.ttf` shipped inside the repo, and there is none (Fira Code
//! is already a system font), so registration is skipped entirely.
//!
//! The cell advance is measured with `CTFontGetAdvancesForGlyphs`
//! (`CTFont::advances_for_glyphs`) on the space glyph, which is what keeps
//! the grid monospace.

use objc2_core_graphics::CGGlyph;
use objc2_core_text::{CTFont, CTFontOrientation, CTLine, CTRun, kCTFontAttributeName, kCTFontFeatureSettingsAttribute};
use objc2_core_foundation::{
    CFArrayCallBacks,
    CFComparisonResult,
    CFMutableAttributedString,
    CFRange,
    CFString,
    CFMutableArray,
    CFStringCompareFlags,
    CGSize,
    CFRetained,
    CGAffineTransform,
    CGFloat,
};

use std::ptr::NonNull;
use std::ptr::null;

/// Point size of the terminal font.
pub const FONT_SIZE: CGFloat = 14.0;

/// The space glyph code point, as a UTF-16 code unit (`UniChar`, which is
/// `u16` in `objc2-core-foundation`).
const SPACE_CODE_UNIT: u16 = 32;

/// Creates the monospace font: "Fira Code" when the system resolves it,
/// otherwise "Menlo".
pub fn monospace(size: CGFloat) -> CFRetained<CTFont> {
    let preferred = CFString::from_str("Fira Code");
    let font = unsafe { CTFont::with_name(&preferred, size, null::<CGAffineTransform>()) };

    // `CTFontCreateWithName` never returns nil, it only "best matches" the
    // requested name; so verify the family actually resolved before
    // accepting it, otherwise the fallback would be silently shadowed by
    // whatever CoreText substituted.
    let family = unsafe { font.family_name() };
    let resolved = unsafe { family.compare(Some(&preferred), CFStringCompareFlags(0)) };
    if resolved.0 == CFComparisonResult::CompareEqualTo.0 {
        return font;
    }

    let fallback = CFString::from_str("Menlo");
    unsafe { CTFont::with_name(&fallback, size, null::<CGAffineTransform>()) }
}

/// The system UI font (San Francisco), used for the chrome text — the tab
/// titles and the `⌘<n>` index hint. iTerm2 draws its tab strip in this face,
/// and `.AppleSystemUIFont` is the reserved PostScript name for the current
/// system text face, so it always resolves on macOS: no fallback chain needed
/// (unlike `monospace`, which asks for a third-party family that may be absent).
pub fn ui_font(size: CGFloat) -> CFRetained<CTFont> {
    let name = CFString::from_str(".AppleSystemUIFont");
    unsafe { CTFont::with_name(&name, size, null::<CGAffineTransform>()) }
}

/// The horizontal advance of a space in `font`, in points.
///
/// Measured with `CTFontGetAdvancesForGlyphs` on the space glyph. This is the
/// cell width: the grid stays monospace even when a ligature glyph visually
/// spans more than one cell.
pub fn cell_advance(font: &CTFont) -> CGFloat {
    let code_unit = SPACE_CODE_UNIT;
    let glyph: CGGlyph = 0;

    // `CTFontGetGlyphsForCharacters` maps the code point to a glyph index.
    let mapped = unsafe {
        font.glyphs_for_characters(NonNull::from(&code_unit), NonNull::from(&glyph), 1)
    };
    if !mapped {
        return 0.0;
    }

    // `advances` may be NULL: the summed advance of the single glyph is
    // returned directly, which is exactly the cell width.
    let no_advances: *mut CGSize = std::ptr::null_mut();
    let width = unsafe {
        font.advances_for_glyphs(
            CTFontOrientation::Horizontal,
            NonNull::from(&glyph),
            no_advances,
            1,
        )
    };
    width as CGFloat
}


/// The (cell width, cell height) pair for the terminal grid at `size`.
///
/// Height is the font's ascent + descent + leading, so a line of text never
/// overlaps the next one.
pub fn cell_metrics(size: CGFloat) -> (CGFloat, CGFloat) {
    let font = monospace(size);
    let width = cell_advance(&font);
    let height = unsafe { font.ascent() } + unsafe { font.descent() } + unsafe { font.leading() };
    (width, height)
}

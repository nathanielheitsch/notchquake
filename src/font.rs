//! Strict installed monospace resolution and metrics measured from that exact CTFont.
use std::ptr::{NonNull, null};

use objc2_core_foundation::{CFRetained, CFString, CGAffineTransform, CGFloat, CGSize};
use objc2_core_graphics::CGGlyph;
use objc2_core_text::{
    CTFont, CTFontManagerCopyAvailableFontFamilyNames, CTFontOrientation, CTFontSymbolicTraits,
};

pub const FONT_SIZE: CGFloat = 14.0;

pub struct ResolvedFont {
    pub font: CFRetained<CTFont>,
    pub family: String,
    pub cell: (CGFloat, CGFloat),
}

pub fn resolve_monospace(family: &str, size: CGFloat) -> Result<ResolvedFont, String> {
    if !size.is_finite() || size <= 0.0 {
        return Err("Font size must be finite and positive".into());
    }
    let requested = CFString::from_str(family);
    let font = unsafe { CTFont::with_name(&requested, size, null::<CGAffineTransform>()) };
    let resolved = unsafe { font.family_name() }.to_string();
    if !resolved.eq_ignore_ascii_case(family)
        || !unsafe { font.symbolic_traits() }.contains(CTFontSymbolicTraits::TraitMonoSpace)
    {
        return Err(format!(
            "{family} is not an available monospace family (CoreText resolved {resolved})"
        ));
    }
    let cell = metrics(&font);
    if !(cell.0.is_finite() && cell.1.is_finite() && cell.0 > 0.0 && cell.1 > 0.0) {
        return Err(format!("{family} has invalid cell metrics"));
    }
    Ok(ResolvedFont {
        font,
        family: resolved,
        cell,
    })
}

/// CoreText-only enumeration: no NSFontManager/main-thread AppKit requirements.
pub fn installed_monospace_families() -> Vec<String> {
    let families = unsafe { CTFontManagerCopyAvailableFontFamilyNames() };
    // CoreText documents this array's elements as CFString family names.
    let families = unsafe { families.cast_unchecked::<CFString>() };
    let mut result: Vec<String> = families
        .iter()
        .map(|name| name.to_string())
        .filter(|name| resolve_monospace(name, FONT_SIZE).is_ok())
        .collect();
    result.sort_by_key(|name| name.to_lowercase());
    result.dedup();
    result
}

/// Compatibility renderer fallback. Settings acceptance uses the strict resolver instead.
pub fn monospace_with(size: CGFloat, family: Option<&str>) -> CFRetained<CTFont> {
    resolve_monospace(family.unwrap_or("Fira Code"), size)
        .or_else(|_| resolve_monospace("Menlo", size))
        .expect("macOS includes Menlo")
        .font
}

pub fn monospace(size: CGFloat) -> CFRetained<CTFont> {
    monospace_with(size, None)
}

pub fn ui_font(size: CGFloat) -> CFRetained<CTFont> {
    unsafe {
        CTFont::with_name(
            &CFString::from_str(".AppleSystemUIFont"),
            size,
            null::<CGAffineTransform>(),
        )
    }
}

pub fn cell_advance(font: &CTFont) -> CGFloat {
    let code_unit: u16 = 32;
    let mut glyph: CGGlyph = 0;
    let mapped = unsafe {
        font.glyphs_for_characters(NonNull::from(&code_unit), NonNull::from(&mut glyph), 1)
    };
    if !mapped {
        return 0.0;
    }
    unsafe {
        font.advances_for_glyphs(
            CTFontOrientation::Horizontal,
            NonNull::from(&glyph),
            std::ptr::null_mut::<CGSize>(),
            1,
        )
    }
}

pub fn metrics(font: &CTFont) -> (CGFloat, CGFloat) {
    (cell_advance(font), unsafe {
        font.ascent() + font.descent() + font.leading()
    })
}

pub fn cell_metrics(size: CGFloat) -> (CGFloat, CGFloat) {
    metrics(&monospace(size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_resolution_and_selected_metrics() {
        assert!(resolve_monospace("Definitely Not An Installed Font", 14.0).is_err());
        let resolved = resolve_monospace("Menlo", 14.0).unwrap();
        assert_eq!(resolved.family, "Menlo");
        assert_eq!(resolved.cell, metrics(&resolved.font));
        assert!(resolved.cell.0 > 0.0 && resolved.cell.1 > 0.0);
    }
}

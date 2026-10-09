//! Shared layout constants for the notch panel.
//!
//! The panel window is a FIXED size and is never resized; only the silhouette
//! height drawn inside it animates.

use objc2_core_foundation::CGFloat;

/// Panel width in logical points.
pub const PANEL_WIDTH: CGFloat = 640.0;

/// Maximum terminal height in logical points.
pub const MAX_TERMINAL_HEIGHT: CGFloat = 420.0;

/// Padding below the terminal area (panel height = terminal + padding).
pub const PANEL_PADDING: CGFloat = 200.0;

/// Panel height in logical points (`MAX_TERMINAL_HEIGHT + PANEL_PADDING`).
pub const PANEL_HEIGHT: CGFloat = MAX_TERMINAL_HEIGHT + PANEL_PADDING;

/// Fallback closed height when the screen reports no notch inset.
pub const CLOSED_HEIGHT: CGFloat = 33.0;

/// Fallback closed width (overridden by the measured notch width).
pub const NOTCH_WIDTH: CGFloat = 212.0;

/// Silhouette height when open.
pub const OPEN_HEIGHT: CGFloat = MAX_TERMINAL_HEIGHT;

/// Corner radius of the two corners touching the menu bar.
pub const TOP_CORNER_RADIUS: CGFloat = 19.0;

/// Corner radius of the two hanging "chin" corners, which also inset the
/// bottom edge by this amount on both sides.
pub const BOTTOM_CORNER_RADIUS: CGFloat = 24.0;

/// Spring response, in seconds (Apple `Animation.spring` semantics).
pub const SPRING_RESPONSE: f64 = 0.35;

/// Spring damping fraction (Apple `Animation.spring` semantics).
pub const SPRING_DAMPING_FRACTION: f64 = 0.8;

/// Key code for Space — `NSEvent.keyCode()` returns the macOS virtual key code
/// (Space is 0x31 = 49), NOT an ASCII or X11 code.
pub const SPACE_KEY_CODE: u16 = 49;

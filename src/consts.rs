//! Shared layout constants for the notch panel.
//!
//! The panel WINDOW is a FIXED size, created once at startup and sized to the
//! notch display's logical width; it is never resized. Only the silhouette
//! height/width drawn inside that canvas animates.

use objc2_core_foundation::CGFloat;

/// Default open terminal width in logical points — the default value of
/// `Settings.open_width`. The canvas itself is sized to the notch display's
/// width (see `window::canvas_size`), not to this constant.
pub const PANEL_WIDTH: CGFloat = 640.0;

/// Fallback canvas width used when the process cannot resolve a notch screen
/// (see `window::canvas_size`).
pub const PANEL_WIDTH_FALLBACK: CGFloat = 640.0;

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

/// Scrollback history in lines when `BNT_SCROLLBACK` is unset. 10000 matches
/// alacritty's own default (`Config.scrolling_history`); iTerm2's default is
/// 500, tmux-style terminals commonly use 2000-10000. Override with
/// `BNT_SCROLLBACK=500 …` at launch.
pub const DEFAULT_SCROLLBACK_LINES: usize = 10000;

/// Silhouette height when open.
pub const OPEN_HEIGHT: CGFloat = MAX_TERMINAL_HEIGHT;

/// Corner radius of the two hanging "chin" corners, which also inset the
/// bottom edge by this amount on both sides. The top corners touching the
/// menu bar are square (the panel sits flush under the notch), and 10pt is
/// the standard macOS window radius.
pub const BOTTOM_CORNER_RADIUS: CGFloat = 10.0;

/// Spring response, in seconds (Apple `Animation.spring` semantics).
pub const SPRING_RESPONSE: f64 = 0.35;

/// Spring damping fraction (Apple `Animation.spring` semantics).
pub const SPRING_DAMPING_FRACTION: f64 = 0.8;

/// Key code for Space — `NSEvent.keyCode()` returns the macOS virtual key code
/// (Space is 0x31 = 49), NOT an ASCII or X11 code.
pub const SPACE_KEY_CODE: u16 = 49;

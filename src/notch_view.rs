//! The notch silhouette: an `NSView` subclass that paints the boring.notch
//! shape at the spring's current height, inside a fixed-size panel.
//!
//! The view is NOT flipped (`setIsFlipped:` is unbound in objc2-app-kit,
//! objc2 refuses an unbound selector at runtime); the drawing space is the
//! window's own **y-up** space: the panel top (menu bar) is at `y =
//! PANEL_HEIGHT`, the chin at `y = PANEL_HEIGHT - h`, and the silhouette
//! grows downward from the menu bar as `h` grows.
//!
//! Animation is self-sustaining: `drawRect:` advances the spring by the
//! wall-clock delta since the previous pass and, while the spring has not
//! settled, marks the view dirty so AppKit calls `drawRect:` again next
//! pass (~60Hz).

use crate::animation::Spring;
use crate::consts::{BOTTOM_CORNER_RADIUS, CLOSED_HEIGHT, NOTCH_WIDTH, PANEL_HEIGHT};
use crate::effects::{
    SettingsState, animation_active, displayed, motion_frame, selection_after_removal,
};
use crate::font::{metrics, monospace_with, resolve_monospace, ui_font};
use crate::input::{self, Command, Context, Modifiers, Route};
use crate::render::{GRID_PAD, GRID_TOP_PAD, draw_grid};
use crate::settings::{EditPhase, PersistenceError, Settings, SettingsError, SettingsSnapshot};
use crate::settings_window::SettingsOwner;
use crate::terminal::{OscSnapshot, Session};
use core::ffi::c_double;
use objc2::AnyThread;
use objc2::ClassType;
use objc2::DefinedClass;
use objc2::MainThreadOnly;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventModifierFlags, NSGraphicsContext, NSPasteboard,
    NSPasteboardTypeString, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWorkspace,
};
use objc2_core_foundation::kCFBooleanTrue;
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFRetained, CGFloat, CGPoint, CGRect, CGSize,
};
use objc2_core_foundation::{CFMutableAttributedString, CFRange, CFString};
use objc2_core_graphics::{CGColor, CGColorSpace, CGContext, CGGradient, CGGradientDrawingOptions};
use objc2_core_text::{
    CTFont, CTLine, CTLineTruncationType, kCTFontAttributeName,
    kCTForegroundColorFromContextAttributeName,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize};
use std::cell::OnceCell;
use std::cell::{Cell, RefCell};

/// Tab-bar geometry: an iTerm-style strip hanging from the silhouette's
/// bottom edge — tabs are rounded-top-corner rectangles flush with the chin
/// line, separated by thin dividers, with a `+` button at the right end.
/// Sessions spawned on the first open frame. ONE tab, like a real user's
/// first session; more tabs come from the `+` button or the Cmd+T hotkey.
const INITIAL_TABS: usize = 1;
/// `+` cap so tabs stay clickable.
const MAX_TABS: usize = 8;
/// `tab_strip` hides the strip below this per-tab width.
const MIN_TAB_W: CGFloat = 28.0;
/// `×` close-affordance glyph radius and (generous) hit radius, and its
/// left inset from the tab's left edge (the × is LEFT-aligned in the tab).
const CLOSE_R: CGFloat = 5.0;
const CLOSE_HIT: CGFloat = 9.0;
const CLOSE_INSET: CGFloat = 14.0;
/// The `×` close-affordance footprint from the tab's left edge: its inset
/// plus the BUTTON half-size (glyph radius + backdrop pad), so the title band
/// starts after the drawn button square rather than after the glyph — the
/// backdrop is what a long title would otherwise overpaint.
const CLOSE_BUTTON_PAD: CGFloat = 4.0;
const TAB_CLOSE_W: CGFloat = CLOSE_INSET + CLOSE_R + CLOSE_BUTTON_PAD;
/// Point size of the tab title (measured off iTerm2's tab strip: a 14px cap
/// height at 1.5× window scale ≈ 9.3pt cap, which is ~13pt San Francisco)
/// and of the right-aligned `⌘<n>` index hint.
const TAB_LABEL_SIZE: CGFloat = 13.0;
const TAB_HINT_SIZE: CGFloat = 10.0;
/// Right inset of the `⌘<n>` hint from the tab's right edge.
const TAB_HINT_INSET: CGFloat = 10.0;
/// Left padding between the `×` gutter and the start of the title band, so
/// the title can never touch the `×` even on the tab that draws it.
const TAB_TITLE_PAD: CGFloat = 10.0;
/// Width the right-aligned `⌘<n>` hint reserves from the tab's right edge:
/// its inset plus the widest hint we can draw (`⌘` + up to 2 digits, at the
/// SF average advance of ~0.6em). The title band gives up this much so the
/// title can never paint over the hint.
const TAB_HINT_W: CGFloat = TAB_HINT_INSET + 3.0 * (TAB_HINT_SIZE * 0.6);
const TAB_HEIGHT: CGFloat = 28.0;

/// Tab-strip palette, sRGB — measured off iTerm2's real window at 1.5× scale
/// (inactive tab fill `1A1A1A`, active tab fill `141414` — the ACTIVE tab is
/// DARKER than the inactive ones in iTerm's dark theme — separator/active-tab
/// border `373737`, active label `FFFFFF`, inactive label `969696`, and the
/// `⌘<n>` index hint in iTerm's blue accent `4372E0` on every tab).
const fn rgb(r: u8, g: u8, b: u8) -> (CGFloat, CGFloat, CGFloat) {
    (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
}
const TAB_FILL: (CGFloat, CGFloat, CGFloat) = rgb(0x1A, 0x1A, 0x1A);
/// Selected-tab fill: darker than `TAB_FILL`, used ONLY for the selected tab's
/// rectangle (the header block keeps `TAB_FILL` so it matches the inactive tabs).
const TAB_FILL_ACTIVE: (CGFloat, CGFloat, CGFloat) = rgb(0x14, 0x14, 0x14);
const LINE: (CGFloat, CGFloat, CGFloat) = rgb(0x37, 0x37, 0x37);
const TEXT_ACTIVE: (CGFloat, CGFloat, CGFloat) = rgb(0xFF, 0xFF, 0xFF);
const TEXT_INACTIVE: (CGFloat, CGFloat, CGFloat) = rgb(0x96, 0x96, 0x96);
/// The `⌘<n>` index hint, in iTerm's blue accent, on BOTH active and inactive
/// tabs (it does not follow the label color).
const HINT_ACCENT: (CGFloat, CGFloat, CGFloat) = rgb(0x43, 0x72, 0xE0);
/// The "activity arrived while you were NOT looking at this tab" dot, in
/// iTerm2's blue — measured off the user's iTerm screenshot as RGB (79,113,217)
/// = `4F71D9`.
const DOT_ACTIVE: (CGFloat, CGFloat, CGFloat) = rgb(0x4F, 0x71, 0xD9);
/// Dot diameter: 18px in the 2× screenshot = 9pt.
const DOT_D: CGFloat = 9.0;
/// The `done` dot: OSC 7501 `state=done` on a non-selected tab (T2b).
const DOT_DONE: (CGFloat, CGFloat, CGFloat) = rgb(0x30, 0xB4, 0x5F);
/// The `error` dot: OSC 7501 `state=error` OR OSC 9;4 `st=2` (T2b/T2c). Also
/// the tab-title tint for a nonzero `133;D` exit status (T2a).
const DOT_ERROR: (CGFloat, CGFloat, CGFloat) = rgb(0xE0, 0x43, 0x43);
/// How much of an OSC 6 tab tint is mixed into the tab fill (25%): enough to
/// read as "this tab is coloured" without hiding the selected/inactive
/// distinction (which is a 6/255 fill delta, far smaller than 25%).
const TINT_MIX: CGFloat = 0.25;

/// How far the header's shadow fade reaches down into the grid (pt): the band
/// of terminal-bg color that fades from opaque at the header edge to clear, so
/// the top text rows fade away under the bar instead of meeting a hard line.
const SHADOW_FADE: CGFloat = 32.0;

/// Extra inset beyond the silhouette's own chin radius. 0.0 means the strip
/// spans the FULL silhouette width, and `tab_strip` therefore ignores
/// `chin_radius` entirely — the strip is drawn under the silhouette clip, so
/// the tapered chin corners crop it (intended) rather than insetting it, and
/// the leftmost/rightmost tabs and the `+` button touch the true edges.
/// All visibility guards use `CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT` so they
/// stay correct automatically as this constant changes.
const TAB_INSET: CGFloat = 0.0;
/// Width of the `+` button at the right end of the strip: square with the
/// strip's height so it reads as a button rather than a sliver. `tab_strip`
/// subtracts it from the tab span, so the click rect and the render rect stay
/// in sync automatically.
const TAB_PLUS_W: CGFloat = TAB_HEIGHT;

/// Control-point factor for a quarter-circle cubic (1 - cos(pi/4)/... ).
const KAPPA: CGFloat = 0.4477;

// ── Settings surface geometry ─────────────────────────────────────────────
/// Width (and height) of the gear button in the header band, flush to the
/// open silhouette's right edge and vertically centered in the header. Single
/// source of truth for the render in `draw` and the hit-test in
/// [`click_at_screen_point`], so they cannot drift.
const SETTINGS_BTN_W: CGFloat = 22.0;
/// Gear glyph metrics: an outline circle of this radius with four tick
/// "teeth" sticking out past it.
const SETTINGS_GEAR_R: CGFloat = 5.0;
const SETTINGS_TOOTH_OUT: CGFloat = 2.5;
const SETTINGS_TOOTH_W: CGFloat = 2.0;

/// Instance variables of [`NotchView`].
///
/// Everything is in `Cell`s because ivars are only reachable through an
/// immutable reference (`self.ivars()`).
pub struct NotchViewIvars {
    spring: Spring,
    width_spring: Spring,
    height: Cell<f64>,
    width: Cell<f64>,
    notch: Cell<(CGFloat, CGFloat)>,
    last_time: Cell<f64>,
    /// Cached grid font + cell metrics. `Cell<Option<...>>` (not `OnceCell`) so
    /// `font_invalidate` can reset them when a font knob changes; both payloads
    /// are moved out with `replace`, never `get` (CFRetained is not Copy).
    font: Cell<Option<CFRetained<CTFont>>>,
    cell: Cell<Option<(CGFloat, CGFloat)>>,
    // Live sessions. `Cell<Vec<Session>>` because mutation happens through
    // `&self`: take the vec out with `take_sessions`, mutate, put it back
    // with `put_sessions` (`Cell::update` needs `T: Copy`, which a Vec is
    // not). Keep every take/put pair short and panic-free — a panic between
    // the two `replace`s would drop the sessions and orphan their PTYs.
    sessions: Cell<Vec<Session>>,
    /// True once the initial batch spawned, so "reopen after all tabs
    /// closed" spawns ONE fresh tab instead of resurrecting the initial set.
    spawned: Cell<bool>,
    /// Mirror of `sessions.len()` so the 60Hz mouse/hover path never moves
    /// the whole `Vec<Session>`; written on every session mutation.
    tab_count: Cell<usize>,
    selected: Cell<Option<usize>>,
    hovered: Cell<Option<usize>>,
    label_font: OnceCell<CFRetained<CTFont>>,
    hint_font: OnceCell<CFRetained<CTFont>>,
    /// The hover tracking areas installed over the panel. A `Vec`, not a
    /// `Cell<Option<Retained<…>>>`: `Retained` is not `Copy`, so `get()`/
    /// `is_none()` (and `OnceCell::take` on an immutable ivar borrow) are
    /// unavailable — lesson 17's move-out/move-back pair is the only legal
    /// shape. The vec is always put back before returning, so a dropped
    /// tracking handle (dead hover) cannot happen on a success path.
    tracking: Cell<Vec<Retained<NSTrackingArea>>>,
    /// The canvas width the installed tracking area was sized to; the pair
    /// `(tracking_canvas != canvas)` is the reinstall sentinel. `CGFloat` is
    /// `Copy`, so this one reads with `get()`/`set()`.
    tracking_canvas: Cell<CGFloat>,
    /// The canvas (panel window) width in logical points: the notch display's
    /// logical width, sized once at startup (see `window::canvas_size`) and
    /// re-set by `reposition` on a display change. Every centring expression is
    /// `(canvas ± w) / 2`, so draw and hit-testing share one geometry source.
    canvas: Cell<f64>,
    backing_scale: Cell<f64>,
    // Wheel-scroll state: `scroll` is the whole lines already applied to the
    // selected session's display_offset; `scroll_pts` carries the leftover
    // fractional wheel points between events. Reset on collapse.
    scroll: Cell<i32>,
    scroll_pts: Cell<f64>,
    /// Immutable snapshots; no borrow may cross an AppKit callback.
    settings: RefCell<SettingsState>,
    reduce_motion: Cell<bool>,
    foreground_poll: Cell<f64>,
    foreground_titles: RefCell<Vec<String>>,
    settings_was_visible: Cell<bool>,
    /// The settings WINDOW (a separate normal titled `NSWindow`, not this
    /// panel), created lazily on a gear click and retained here for the
    /// whole process lifetime. AppKit does not retain the window tree for us
    /// — same class of bug as the dropped observer token (lesson 16): if the
    /// `Retained<SettingsOwner>` dies, the window, its controller, its grid
    /// and every control die with it and every later click silently no-ops.
    /// The view itself is retained by `AppDelegate.view`, so this ivar is
    /// the owner slot.
    settings_owner: OnceCell<Retained<SettingsOwner>>,
}

define_class!(
    // SAFETY:
    // - The superclass `NSView` is a normal `NSObject` subclass; we do not
    //   override anything with subclassing requirements.
    // - `NotchViewIvars` does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "NotchView"]
    #[ivars = NotchViewIvars]
    pub struct NotchView;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for NotchView {}

    impl NotchView {
        // SAFETY: `drawRect:` is `NSView`'s draw hook (`- (void)drawRect:(NSRect)`);
        // the signature is correct. AppKit calls it for a non-layer-backed view,
        // and inside it the current graphics context is guaranteed non-nil.
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty_rect: NSRect) {
            let ivars = self.ivars();

            // Integrate frame time; retargeting preserves position and velocity.
            // Zero denotes the first frame after idle, not an animation restart.
            let now = CFAbsoluteTimeGetCurrent();
            let previous = ivars.last_time.get();
            let delta = if previous == 0.0 {
                0.0
            } else {
                now - previous
            };
            ivars.last_time.set(now);

            let (height, width) = motion_frame(
                &ivars.spring, &ivars.width_spring, delta, ivars.reduce_motion.get(),
                ivars.notch.get(), (ivars.canvas.get(), PANEL_HEIGHT),
            );
            ivars.height.set(height);
            ivars.width.set(width);
            if let Some(graphics) = NSGraphicsContext::currentContext() {
                let ctx = graphics.CGContext();
                draw(Some(&ctx), height, ivars.width.get(), ivars);
            } else {
                // A blank panel is otherwise silent: make it diagnosable.
                eprintln!("notch-terminal: no current graphics context in drawRect");
            }

            // Keep the view dirty while the spring is still flying so AppKit
            // re-invokes `drawRect:` next pass.
            if animation_active(ivars.spring.settled(), ivars.width_spring.settled()) {
                self.mark_dirty();
            } else {
                ivars.last_time.set(0.0);
            }

            // The hover tracking area, installed over the whole panel.
            // `ActiveInKeyWindow` ties hover to key focus, which the panel
            // takes on click (see Item 1).
            //
            // shortcut: the rect is reinstalled only when the display canvas
            // changes — the canvas is the only input to its size, so a frame
            // that does not change the canvas cannot leave a stale rect.
            let canvas = ivars.canvas.get();
            if ivars.tracking_canvas.get() != canvas {
                // Move the old handles out, drop them from the view, then move
                // the fresh one back in (lesson 17's idiom).
                let old = ivars.tracking.replace(vec![]);
                for area in &old {
                    self.as_super().removeTrackingArea(area)
                }
                let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(canvas, PANEL_HEIGHT));
                let opts = NSTrackingAreaOptions::MouseEnteredAndExited
                    .union(NSTrackingAreaOptions::MouseMoved)
                    .union(NSTrackingAreaOptions::ActiveInKeyWindow)
                    .union(NSTrackingAreaOptions::InVisibleRect);
                let area = unsafe {
                    NSTrackingArea::initWithRect_options_owner_userInfo(
                        NSTrackingArea::alloc(),
                        rect,
                        opts,
                        Some(self.as_super() as &AnyObject),
                        None,
                    )
                };
                self.as_super().addTrackingArea(&area);
                ivars.tracking.replace(vec![area]);
                ivars.tracking_canvas.set(canvas);
            }
        }

        // A click on the panel's drawn region. The panel becomes the key
        // window on click (Item 1), so its views receive `mouseDown:` for
        // clicks on the opaque silhouette — the reliable path for clicking
        // the closed pill (toggle) and the open tab strip (select).
        // `locationInWindow` is already in this view's y-up panel-local space
        // (the view fills the panel), so convert to a screen point and reuse
        // the shared hit-test.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let panel = match self.as_super().window() {
                Some(w) => w,
                None => return,
            };
            let frame = panel.frame();
            let loc = event.locationInWindow();
            let screen = NSPoint::new(frame.origin.x + loc.x, frame.origin.y + loc.y);
            self.click_at_screen_point(&screen);
        }

        // Hover: the panel becomes the key window on click (see Item 1), and
        // the tracking area installed in `drawRect:` carries
        // `ActiveInKeyWindow`, so these fire while the panel is key. They only
        // record which tab the cursor is over; the repaint is a palette change.
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.update_hover(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.update_hover(event);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            let ivars = self.ivars();
            if ivars.hovered.get().is_some() {
                ivars.hovered.set(None);
                self.mark_dirty();
            }
        }

        #[unsafe(method(tick))]
        fn tick(&self) {
            let ivars = self.ivars();
            // Reap dead sessions (child shell exited) unconditionally: a
            // dead session while closed still needs teardown so its PTY dies.
            self.reap_dead_sessions();
            // Post notifications for bells / finished long commands on tabs
            // the user is NOT looking at. Runs even while closed (every tab is
            // then non-selected), so a background tab can still alert.
            self.drain_notifications();
            self.pump_settings(false).unwrap_or_else(|error| self.record_status(error.to_string()));
            let visible = crate::settings_window::window_frame(&ivars.settings_owner).is_some();
            if ivars.settings_was_visible.replace(visible) && !visible
                && let Err(error) = self.flush_settings() {
                self.record_status(error.to_string());
            }
            let now = CFAbsoluteTimeGetCurrent();
            let sessions = take_sessions(ivars);
            let mut changed = false;
            for session in &sessions { changed |= session.take_dirty(); }
            if now >= ivars.foreground_poll.get() {
                ivars.foreground_poll.set(now + 0.5);
                let titles: Vec<_> = sessions.iter().map(Session::title).collect();
                let mut old = ivars.foreground_titles.borrow_mut();
                changed |= *old != titles;
                *old = titles;
            }
            put_sessions(ivars, sessions);
            let open = self.is_open();
            if animation_active(ivars.spring.settled(), ivars.width_spring.settled()) || (open && changed) {
                self.mark_dirty();
                if let Some(window) = self.as_super().window() {
                    window.update();
                    window.displayIfNeeded();
                }
            }
        }
    }
);
/// The silhouette path, traced in a top-left-origin rect `(0, 0, canvas,
/// h)`: square top corners at the menu bar (a straight top edge, since the
/// panel sits flush under the notch), then the chin corners rounded at
/// `BOTTOM_CORNER_RADIUS`, which also inset the bottom edge on both sides.
fn trace_silhouette(ctx: Option<&CGContext>, h: CGFloat, w: CGFloat, canvas: CGFloat) {
    // AppKit clamps corner radii to the rect; do the same so the closed pill
    // (h < 2 * bottom) stays a proper pill instead of self-intersecting.
    let bottom = chin_radius(h);

    // y-up space: the silhouette hangs from the panel top, so the menu-bar
    // edge is at `max_y` and the chin (bottom) at `min_y`. The shape is
    // horizontally centred in the canvas.
    let (min_x, min_y) = ((canvas - w) / 2.0, PANEL_HEIGHT - h);
    let (max_x, max_y) = ((canvas + w) / 2.0, PANEL_HEIGHT);

    // Square top corners: a straight top edge at the menu bar (`max_y`),
    // quarter-circle arcs only at the chin (`min_y`). Traced counter-clockwise
    // from the left edge, with proper quarter-circle control points (kappa).
    CGContext::begin_path(ctx);
    CGContext::move_to_point(ctx, min_x, max_y);
    // Left side down to the chin corner.
    CGContext::add_line_to_point(ctx, min_x, min_y + bottom);
    // Chin left corner.
    CGContext::add_curve_to_point(
        ctx,
        min_x,
        min_y + bottom * (1.0 - KAPPA),
        min_x + bottom * (1.0 - KAPPA),
        min_y,
        min_x + bottom,
        min_y,
    );
    // Chin bottom edge.
    CGContext::add_line_to_point(ctx, max_x - bottom, min_y);
    // Chin right corner.
    CGContext::add_curve_to_point(
        ctx,
        max_x - bottom * (1.0 - KAPPA),
        min_y,
        max_x,
        min_y + bottom * (1.0 - KAPPA),
        max_x,
        min_y + bottom,
    );
    // Right side up to the square top-right corner.
    CGContext::add_line_to_point(ctx, max_x, max_y);
    // Straight top edge along the menu bar.
    CGContext::add_line_to_point(ctx, min_x, max_y);
    CGContext::close_path(ctx);
}

/// Paints one frame: the black silhouette, then (once open) the terminal
/// grid and the tab bar, all clipped to the silhouette.
fn draw(ctx: Option<&CGContext>, height: CGFloat, width: CGFloat, ivars: &NotchViewIvars) {
    let h = if height < 1.0 { 1.0 } else { height };
    let w = if width < 1.0 { 1.0 } else { width };

    // This non-layer-backed view uses y-up canvas coordinates.
    // Clone the immutable snapshot, then release the state borrow before drawing.
    let settings = ivars.settings.borrow().current.clone();

    // The canvas (panel window) width: the other half of every centring
    // expression `(canvas ± w) / 2`. Read from the ivar so draw, hit-testing
    // and the tracking rect share one source after a display change.
    let canvas = ivars.canvas.get();

    CGContext::save_g_state(ctx);

    // The silhouette. Its fill IS `settings.background` (plan A #7): the grid
    // skips cells painted the panel background, so the panel color has to come
    // from here for a color-well change to repaint anything.
    trace_silhouette(ctx, h, w, canvas);
    CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(settings.background.rgb())));
    CGContext::fill_path(ctx);

    // Everything below stays inside the silhouette. `CGContextFillPath`
    // CONSUMES the current path, so the clip needs its OWN trace: clipping the
    // (already empty) path silently did nothing. That went unnoticed only
    // because the strip used to stop short of the chin corners — once the
    // strip spans the full width it painted straight over them, squaring the
    // silhouette's rounded bottom corners. Re-trace, then clip.
    trace_silhouette(ctx, h, w, canvas);
    CGContext::clip(ctx);

    // Take the session vec once for the whole frame (put it back at the
    // end, before `restore_g_state`): the grid pass, spawn-on-open rule and
    // tab labels all read it, and a second take/put pair would be more
    // panic surface.
    let mut sessions = take_sessions(ivars);

    // The terminal grid, once the panel is open. Sessions spawn on the
    // first open frame at the size `grid_dims` computes from the OPEN height
    // minus the header, so the reveal itself is a pure clip wipe; an ACCEPTED
    // geometry/font change reflows the live sessions separately through
    // `pump_settings` → `Session::resize` (see that seam). After the last tab
    // is closed (`spawned` set), reopening spawns ONE fresh tab.
    //
    // Gate on the spring TARGET, not just the current height: closing the
    // LAST tab empties `sessions` and starts a collapse, and the collapse
    // animation passes through frames with h > CLOSED_HEIGHT. Without the
    // target check, `draw` would respawn a tab on those frames and the
    // panel would never actually collapse (the bug behind "closing the only
    // tab leaves the panel open").
    if h > CLOSED_HEIGHT + 1.0 {
        let (font, cell) = ensure_font(ivars, &settings);
        let header = header_height(ivars);
        if sessions.is_empty() && ivars.spring.target_value() > CLOSED_HEIGHT + 1.0 {
            let count = if ivars.spawned.get() { 1 } else { INITIAL_TABS };
            ivars.spawned.set(true);
            let (rows, columns, scrollback) = grid_dims(cell, header, &settings, canvas);
            let pixels = cell_pixels(cell, ivars.backing_scale.get());
            sessions = (0..count)
                .map(|_| Session::new(rows, columns, pixels.0, pixels.1, scrollback))
                .collect();
            // Select the new tab directly (not via `select_tab`): `tab_count`
            // is still the pre-spawn mirror here because `put_sessions` runs at
            // the end of the frame, so the guard in `select_tab` would reject
            // the index. `selected` starts None and stays None across a collapse,
            // so a fresh spawn must own the selection or `draw_grid` matches
            // `_ => {}` and the terminal renders blank.
            ivars.selected.set(Some(count - 1));
        }
        // The header bar: filled with the TERMINAL background color (settings
        // #7) so it is seamless with the grid below; the shadow band drawn
        // after the grid (see below) is what separates it visually.
        CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(settings.background.rgb())));
        CGContext::fill_rect(
            ctx,
            CGRect::new(
                CGPoint::new((canvas - w) / 2.0, PANEL_HEIGHT - header),
                CGSize::new(w, header),
            ),
        );
        // No hard separator line: the shadow fade band (drawn AFTER the grid
        // so text fades under it) replaces it.
        // The gear button, top-right of the header band, flush to the open
        // silhouette's right edge and vertically centered in the header. Drawn
        // AFTER the header fill so it sits on top of the header block, and
        // BEFORE the grid/settings match so the grid can never overpaint it.
        let (gear_x0, gear_y0, gear_x1, gear_y1) = gear_rect(w, header, canvas);
        draw_gear(ctx, (gear_x0 + gear_x1) / 2.0, (gear_y0 + gear_y1) / 2.0);

        // The grid, unconditional: the settings surface is a separate window
        // now, so the panel only ever draws the terminal.
        match ivars.selected.get().and_then(|i| sessions.get(i)) {
            Some(session) => {
                let grid = session.snapshot(&settings.palette());
                // The grid is laid out inside the LIVE silhouette: `grid_x0` is
                // canvas-relative (the silhouette's left edge plus `GRID_PAD`), and
                // `grid_usable` is the silhouette's inner width — the columns the
                // session baked from `settings.open_width` are centred in it.
                let grid_x0 = (canvas - w) / 2.0 + GRID_PAD;
                let grid_usable = w - 2.0 * GRID_PAD;
                draw_grid(
                    ctx.expect("a current context"),
                    &grid,
                    session.cursor(),
                    &font,
                    cell.0,
                    cell.1,
                    h,
                    header,
                    &settings,
                    grid_x0,
                    grid_usable,
                )
            }
            _ => {}
        }
        // The shadow fade: a band of the TERMINAL background color, opaque
        // at the header's bottom edge and fading to transparent ~32pt down,
        // drawn AFTER the grid so the top row of text fades away under it
        // instead of meeting a hard line. Cohesive: the bar and the terminal
        // share one color, the fade is the only separation.
        draw_header_shadow(ctx, w, header, settings.background.rgb(), canvas)
    }

    // The tab strip hanging from the silhouette's bottom edge — SQUARE
    // rectangles flush with the chin line, all filled TAB_FILL. Only the
    // ACTIVE tab gets 1px side borders (LINE color, left and right edges
    // only — never top or bottom); separators between tabs are LINE too.
    let chin_y = PANEL_HEIGHT - h;
    // `count` is the LIVE session count (the vec was just taken above, so
    // it cannot lag the `tab_count` mirror mid-frame); hover and click use
    // the mirror. All three pass the same count to `tab_strip` — the single
    // source of truth for strip geometry, the `+` rect and the `×` rect.
    let count = sessions.len();
    match tab_strip(h, w, count, canvas) {
        Some((left, tab_w)) => {
            let selected = ivars.selected.get();
            let hovered = ivars.hovered.get();
            for index in 0..count {
                let x = left + index as CGFloat * tab_w;
                // One OSC snapshot per tab, reused for the fill tint AND the dot,
                // so both read a consistent state within the frame.
                let osc = sessions.get(index).map(|s| s.osc()).unwrap_or_default();
                let base = if Some(index) == selected {
                    TAB_FILL_ACTIVE
                } else {
                    TAB_FILL
                };
                let fill = mix_tint(base, osc.tint);
                CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(fill)));
                CGContext::fill_rect(
                    ctx,
                    CGRect::new(CGPoint::new(x, chin_y), CGSize::new(tab_w, TAB_HEIGHT)),
                );
                // The attention dot: this tab wants the user's eyes (iTerm's "go
                // look at that tab" marker). Colour precedence, most urgent first:
                // error (7501 `error` or 9;4 `st=2`) > done (7501 `done`) > blue
                // activity (a command finished / 133;D). ONE dot only, so a tab
                // with several states still shows a single marker. Drawn only when
                // the tab is neither selected nor hovered: the `×` occupies the same
                // gutter on a hovered tab, and the selected tab is the one being
                // looked at. The dot is a FILL (never a stroke: fill and stroke
                // colour are separate CGContext state).
                let looking = Some(index) == selected || hovered == Some(index);
                let active = sessions
                    .get(index)
                    .map(|s| s.has_activity())
                    .unwrap_or(false);
                if !looking {
                    if let Some(color) = attention_dot(&osc, active) {
                        // y-up space: vertically centered in the tab, and centered
                        // in the `×` gutter (`TAB_CLOSE_W` = 23pt) so it sits where
                        // the title band's reservation already leaves room.
                        let cx = x + TAB_CLOSE_W / 2.0;
                        let cy = chin_y + TAB_HEIGHT / 2.0;
                        CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(color)));
                        CGContext::fill_ellipse_in_rect(
                            ctx,
                            CGRect::new(
                                CGPoint::new(cx - DOT_D / 2.0, cy - DOT_D / 2.0),
                                CGSize::new(DOT_D, DOT_D),
                            ),
                        );
                    }
                }
            }
            // Separators between tabs + the active tab's side borders, one
            // stroke pass in LINE color.
            CGContext::begin_path(ctx);
            for index in 1..count {
                let x = left + index as CGFloat * tab_w;
                CGContext::move_to_point(ctx, x, chin_y);
                CGContext::add_line_to_point(ctx, x, chin_y + TAB_HEIGHT);
            }
            if let Some(sel) = selected {
                if sel < count {
                    let x = left + sel as CGFloat * tab_w;
                    for bx in [x, x + tab_w] {
                        CGContext::move_to_point(ctx, bx, chin_y);
                        CGContext::add_line_to_point(ctx, bx, chin_y + TAB_HEIGHT);
                    }
                }
            }
            CGContext::set_stroke_color_with_color(ctx, Some(&rgb_color(LINE)));
            CGContext::set_line_width(ctx, 1.0);
            CGContext::stroke_path(ctx);
            // The `+` button at the right end: a real click target (spawns a
            // session, see `spawn_tab`). Its rect derives from `tab_strip`'s
            // outputs — same math as the hit-test in `click_at_screen_point`.
            let px = left + count as CGFloat * tab_w;
            CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(TAB_FILL)));
            CGContext::fill_rect(
                ctx,
                CGRect::new(
                    CGPoint::new(px, chin_y),
                    CGSize::new(TAB_PLUS_W, TAB_HEIGHT),
                ),
            );
            let cx = px + TAB_PLUS_W / 2.0;
            let cy = chin_y + TAB_HEIGHT * 0.55;
            CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(TEXT_ACTIVE)));
            CGContext::fill_rect(
                ctx,
                CGRect::new(CGPoint::new(cx - 5.0, cy - 0.75), CGSize::new(10.0, 1.5)),
            );
            CGContext::fill_rect(
                ctx,
                CGRect::new(CGPoint::new(cx - 0.75, cy - 5.0), CGSize::new(1.5, 10.0)),
            );

            // The tab labels: the shell's OSC title, truncated so the right-aligned
            // `⌘<n>` hint always stays visible. Active TEXT_ACTIVE, inactive
            // TEXT_INACTIVE; ellipsis-truncated when too wide.
            let label_font = ivars.label_font.get_or_init(|| ui_font(TAB_LABEL_SIZE));
            let hint_font = ivars.hint_font.get_or_init(|| ui_font(TAB_HINT_SIZE));
            for index in 0..count {
                let x = left + index as CGFloat * tab_w;
                let label = sessions
                    .get(index)
                    .map(|s| s.title())
                    .unwrap_or_else(|| format!("Tab {}", index + 1));
                // A nonzero `133;D` exit status tints the title red (T2a): the
                // minimal render that reads at tab size — a red dot lives in the
                // `×` gutter already used by attention, and a red label is
                // unmistakable without a second affordance. Cleared on select,
                // alongside the dot.
                let failed = sessions
                    .get(index)
                    .map(|s| s.osc().exit_status.map(|st| st != 0).unwrap_or(false))
                    .unwrap_or(false);
                let color = if failed {
                    DOT_ERROR
                } else if Some(index) == selected {
                    TEXT_ACTIVE
                } else {
                    TEXT_INACTIVE
                };
                // The title band is `x + TAB_CLOSE_W + TAB_TITLE_PAD .. x +
                // tab_w - TAB_HINT_W`. The `×` footprint and the hint reservation
                // are BOTH reserved on EVERY tab, whether or not that tab currently
                // draws a `×` (hover/selection only): the band is therefore
                // identical for all tabs, so the title never re-centres — and thus
                // never shifts or retruncates — when the pointer enters the strip.
                let left_pad = TAB_CLOSE_W + TAB_TITLE_PAD;
                draw_tab_label(
                    ctx,
                    Some(label_font),
                    &label,
                    x + left_pad,
                    chin_y,
                    tab_w - left_pad - TAB_HINT_W,
                    color,
                );
                let hint = format!("\u{2318}{}", index + 1);
                draw_tab_hint(ctx, Some(hint_font), &hint, x, chin_y, tab_w, HINT_ACCENT);
            }
            // The `×` close affordance: shown ONLY on a HOVERED tab (not the
            // active one) — the pointer has to be on the tab to see/close it. LEFT-
            // aligned in the tab (reference style). Drawn AFTER the labels so a
            // long title cannot overpaint it.
            for index in 0..count {
                if hovered == Some(index) {
                    let x = left + index as CGFloat * tab_w;
                    let (cx, cy) = close_center(x, chin_y);
                    // The reference style puts the `×` inside a rounded dark
                    // square button (gray fill, ~2x glyph size).
                    let b = CLOSE_R + CLOSE_BUTTON_PAD;
                    CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(LINE)));
                    CGContext::fill_rect(
                        ctx,
                        CGRect::new(CGPoint::new(cx - b, cy - b), CGSize::new(b * 2.0, b * 2.0)),
                    );
                    CGContext::begin_path(ctx);
                    CGContext::move_to_point(ctx, cx - CLOSE_R, cy - CLOSE_R);
                    CGContext::add_line_to_point(ctx, cx + CLOSE_R, cy + CLOSE_R);
                    CGContext::move_to_point(ctx, cx - CLOSE_R, cy + CLOSE_R);
                    CGContext::add_line_to_point(ctx, cx + CLOSE_R, cy - CLOSE_R);
                    // Stroke color is separate state from fill — set it BEFORE
                    // `stroke_path` or the default opaque black is invisible.
                    CGContext::set_stroke_color_with_color(ctx, Some(&rgb_color(TEXT_ACTIVE)));
                    CGContext::set_line_width(ctx, 1.5);
                    CGContext::stroke_path(ctx);
                }
            }
        }
        _ => {}
    }

    put_sessions(ivars, sessions);

    CGContext::restore_g_state(ctx);
}

/// The silhouette scale factor that keeps the closed pill a proper pill
/// (AppKit clamps corner radii to the rect; we must match it). The top
/// corners are square, so only the two chin corners clamp: `2 * BOTTOM`.
fn silhouette_scale(h: CGFloat) -> CGFloat {
    if h < 2.0 * BOTTOM_CORNER_RADIUS {
        h / (2.0 * BOTTOM_CORNER_RADIUS)
    } else {
        1.0
    }
}

/// The chin corner radius at height `h`, matching the clamp in
/// [`trace_silhouette`] so the tab strip can sit inside the tapered edge.
fn chin_radius(h: CGFloat) -> CGFloat {
    BOTTOM_CORNER_RADIUS * silhouette_scale(h)
}

/// The tab strip's left edge and per-tab width at silhouette size `(h, w)`;
/// `None` when the strip is not drawn. Single source of truth shared by the
/// renderer and the click hit-test so they cannot drift.
fn tab_strip(h: CGFloat, w: CGFloat, count: usize, canvas: CGFloat) -> Option<(CGFloat, CGFloat)> {
    if h <= CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT || count == 0 {
        return None;
    }
    let inset = TAB_INSET;
    let left = (canvas - w) / 2.0 + inset;
    let right = (canvas + w) / 2.0 - inset;
    let span = right - left - TAB_PLUS_W;
    let tab_w = span / count as CGFloat;
    if span <= 0.0 || tab_w < MIN_TAB_W {
        return None;
    }
    Some((left, tab_w))
}

/// Mix `tint` into `base` at [`TINT_MIX`] (T1b). `None` tint returns `base`
/// unchanged, so an untinted tab is byte-identical to the pre-feature render.
fn mix_tint(
    base: (CGFloat, CGFloat, CGFloat),
    tint: Option<(f64, f64, f64)>,
) -> (CGFloat, CGFloat, CGFloat) {
    match tint {
        None => base,
        Some((r, g, b)) => (
            base.0 * (1.0 - TINT_MIX) + r * TINT_MIX,
            base.1 * (1.0 - TINT_MIX) + g * TINT_MIX,
            base.2 * (1.0 - TINT_MIX) + b * TINT_MIX,
        ),
    }
}

/// The single attention dot color for a non-selected, non-hovered tab, or
/// `None` when the tab wants nothing. Precedence: error > done > activity
/// (T2a/T2b/T2c). `active` is the blue `133;D`/`Title` flag.
fn attention_dot(osc: &OscSnapshot, active: bool) -> Option<(CGFloat, CGFloat, CGFloat)> {
    let program_state = osc.program.as_ref().map(|(s, _, _)| s.as_str());
    let progress_error = matches!(osc.progress, Some((2, _)));
    if program_state == Some("error") || progress_error {
        return Some(DOT_ERROR);
    }
    if program_state == Some("done") {
        return Some(DOT_DONE);
    }
    if active {
        return Some(DOT_ACTIVE);
    }
    None
}

/// The `×` close affordance center — LEFT-aligned in the tab (reference
/// style), vertically centered. Single source of truth for the renderer and
/// the click hit-test.
fn close_center(x: CGFloat, chin_y: CGFloat) -> (CGFloat, CGFloat) {
    (x + CLOSE_INSET, chin_y + TAB_HEIGHT / 2.0)
}

/// The gear button's rect (x0, y0, x1, y1) for an open silhouette of width
/// `w` with a `header`-tall header band: flush to the silhouette's right edge,
/// vertically centered in the header. Single source of truth for the render in
/// `draw` and the hit-test in `click_at_screen_point`.
fn gear_rect(w: CGFloat, header: CGFloat, canvas: CGFloat) -> (CGFloat, CGFloat, CGFloat, CGFloat) {
    let right = (canvas + w) / 2.0;
    let cy = PANEL_HEIGHT - header / 2.0;
    (
        right - SETTINGS_BTN_W,
        cy - SETTINGS_BTN_W / 2.0,
        right,
        cy + SETTINGS_BTN_W / 2.0,
    )
}

/// The gear glyph: an outline circle with four tick teeth — the affordance
/// that opens the settings WINDOW. It no longer repaints on hover (the
/// `settings_hovered` ivar went away with the overlay, and hover is not
/// driver-verifiable anyway, lesson 47), so it is a static `TEXT_INACTIVE`.
/// Strokes must set the STROKE color (fill and stroke are separate
/// CGContext state, lesson 8).
fn draw_gear(ctx: Option<&CGContext>, cx: CGFloat, cy: CGFloat) {
    let color = TEXT_INACTIVE;
    CGContext::begin_path(ctx);
    CGContext::add_ellipse_in_rect(
        ctx,
        CGRect::new(
            CGPoint::new(cx - SETTINGS_GEAR_R, cy - SETTINGS_GEAR_R),
            CGSize::new(SETTINGS_GEAR_R * 2.0, SETTINGS_GEAR_R * 2.0),
        ),
    );
    // The teeth: four rects radiating N/S/E/W, added to the same path so one
    // stroke pass draws circle + teeth.
    let out = SETTINGS_GEAR_R + SETTINGS_TOOTH_OUT;
    let half = SETTINGS_TOOTH_W / 2.0;
    for (x0, y0, w, h) in [
        (
            cx - half,
            cy + SETTINGS_GEAR_R - 0.5,
            SETTINGS_TOOTH_W,
            out - SETTINGS_GEAR_R + 0.5,
        ),
        (
            cx - half,
            cy - out,
            SETTINGS_TOOTH_W,
            out - SETTINGS_GEAR_R + 0.5,
        ),
        (
            cx + SETTINGS_GEAR_R - 0.5,
            cy - half,
            out - SETTINGS_GEAR_R + 0.5,
            SETTINGS_TOOTH_W,
        ),
        (
            cx - out,
            cy - half,
            out - SETTINGS_GEAR_R + 0.5,
            SETTINGS_TOOTH_W,
        ),
    ] {
        CGContext::add_rect(ctx, CGRect::new(CGPoint::new(x0, y0), CGSize::new(w, h)));
    }
    CGContext::set_stroke_color_with_color(ctx, Some(&rgb_color(color)));
    CGContext::set_line_width(ctx, 1.5);
    CGContext::stroke_path(ctx);
    // The hub hole: a small filled dot in the same color reads as a gear.
    CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(color)));
    CGContext::fill_ellipse_in_rect(
        ctx,
        CGRect::new(CGPoint::new(cx - 1.5, cy - 1.5), CGSize::new(3.0, 3.0)),
    );
}

/// The header bar height while open: the measured notch height (the panel
/// top hugs the notch). Refreshed by `reposition` via `set_notch`.
fn header_height(ivars: &NotchViewIvars) -> CGFloat {
    let (_, n) = ivars.notch.get();
    if n < 1.0 { CLOSED_HEIGHT } else { n }
}

/// The shadow fade under the header bar: a band of the TERMINAL background
/// color, opaque at the header's bottom edge and fading to transparent `SHADOW_FADE`
/// pt down, drawn AFTER the grid so the top text rows fade away under it. The
/// header itself is filled with the same background color, so the bar reads as
/// a soft shadow over the terminal rather than a hard-edged block.
fn draw_header_shadow(
    ctx: Option<&CGContext>,
    w: CGFloat,
    header: CGFloat,
    bg: (CGFloat, CGFloat, CGFloat),
    canvas: CGFloat,
) {
    let (r, g, b) = bg;
    let space = CGColorSpace::new_device_rgb();
    // Two stops: opaque terminal-bg at the top, fully clear at the bottom.
    let components = [r, g, b, 1.0, r, g, b, 0.0];
    let stops = [0.0, 1.0];
    let gradient = match space {
        None => None,
        Some(space) => unsafe {
            CGGradient::with_color_components(Some(&*space), components.as_ptr(), stops.as_ptr(), 2)
        },
    };
    // y-up space: the band hangs from the header's bottom edge (y = PANEL_HEIGHT
    // - header) down `SHADOW_FADE` pt. Opaque at the START point (top), clear at
    // the END point (bottom); DrawsAfterEndLocation extends the clear tail so a
    // partial band never shows a hard edge.
    let x0 = (canvas - w) / 2.0;
    CGContext::save_g_state(ctx);
    CGContext::begin_path(ctx);
    CGContext::add_rect(
        ctx,
        CGRect::new(
            CGPoint::new(x0, PANEL_HEIGHT - header - SHADOW_FADE),
            CGSize::new(w, SHADOW_FADE),
        ),
    );
    CGContext::clip(ctx);
    CGContext::draw_linear_gradient(
        ctx,
        gradient.as_ref().map(|g| &**g),
        CGPoint::new(x0, PANEL_HEIGHT - header),
        CGPoint::new(x0, PANEL_HEIGHT - header - SHADOW_FADE),
        CGGradientDrawingOptions::DrawsAfterEndLocation,
    );
    CGContext::restore_g_state(ctx);
}

/// Move the session vec out of its `Cell` (leaves `vec![]` behind); pair
/// with `put_sessions` in a short, panic-free span.
fn take_sessions(ivars: &NotchViewIvars) -> Vec<Session> {
    ivars.sessions.replace(vec![])
}

/// Move the session vec back into its `Cell` and refresh the `tab_count`
/// mirror so every `tab_strip(h, w, count)` call site agrees.
fn put_sessions(ivars: &NotchViewIvars, sessions: Vec<Session>) {
    ivars.tab_count.set(sessions.len());
    ivars.sessions.replace(sessions);
}

/// Draw one tab label centered in the band `(x, y, w)`, ellipsis-truncated
/// when it does not fit. The ellipsis is a real `…` glyph shaped with the
/// SAME font: `CTLineCreateTruncatedLine` with a NULL truncation token just
/// cuts the line off and draws no ellipsis at all, and it returns NULL when
/// the token is wider than `w` — which a mismatched-font token would hit.
/// The line is shaped from a
/// `CFMutableAttributedString` whose font attribute covers the FULL string
/// range (a zero-length range is the INSERT range for
/// `CFMutableAttributedString::replace_string`, not the range of the inserted
/// text — see AGENTS.md lesson 3).
fn draw_tab_label(
    ctx: Option<&CGContext>,
    font: Option<&CTFont>,
    label: &str,
    x: CGFloat,
    y: CGFloat,
    w: CGFloat,
    color: (CGFloat, CGFloat, CGFloat),
) {
    if label.is_empty() || w <= 0.0 {
        return;
    }
    let font = font.expect("a label font");
    let text = CFString::from_str(label);
    let line = unsafe { shaped_line(font, &text) };

    let line_width = unsafe {
        line.typographic_bounds(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } as CGFloat;
    let line = if line_width > w {
        let ellipsis = CFString::from_str("\u{2026}");
        let token = unsafe { shaped_line(font, &ellipsis) };
        match unsafe {
            line.truncated_line(w as c_double, CTLineTruncationType::End, Some(&*token))
        } {
            Some(truncated) => truncated,
            _ => return,
        }
    } else {
        line
    };

    // Measure the line we will actually draw (a truncated line is narrower
    // than the original), so the label centers on its own width.
    let drawn_width = unsafe {
        line.typographic_bounds(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } as CGFloat;

    // Center the visible ink, not the EM box: San Francisco's `ascent`
    // includes accent/diacritic headroom far above the cap height, so
    // centering `(ascent + descent)` pushes visible glyphs ~6pt high. The
    // cap-height band (baseline .. baseline + cap_height) is what the eye
    // sees, so center THAT in the tab: baseline = y + (TAB_HEIGHT - cap) / 2,
    // which puts ink at panel-y ≈209.8..219.2 for a band centred on 214.5 —
    // the same centre as the vector `×` (measured 214.2).
    let cap = unsafe { font.cap_height() };
    let baseline = y + (TAB_HEIGHT - cap) / 2.0;
    let cx = x + (w - drawn_width) / 2.0;

    CGContext::save_g_state(ctx);
    CGContext::translate_ctm(ctx, cx, baseline);
    CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(color)));
    CGContext::set_text_position(ctx, 0.0, 0.0);
    unsafe { line.draw(ctx.expect("a current context")) };
    CGContext::restore_g_state(ctx);
}

/// Draw the right-aligned `⌘<n>` index hint in the tab `(x, y, w)`, inset
/// from the tab's right edge (reference style). Same shaping path as
/// `draw_tab_label`; the only difference is the horizontal anchor.
fn draw_tab_hint(
    ctx: Option<&CGContext>,
    font: Option<&CTFont>,
    hint: &str,
    x: CGFloat,
    y: CGFloat,
    w: CGFloat,
    color: (CGFloat, CGFloat, CGFloat),
) {
    if hint.is_empty() || w <= 0.0 {
        return;
    }
    let font = font.expect("a hint font");
    let text = CFString::from_str(hint);
    let line = unsafe { shaped_line(font, &text) };
    let line_width = unsafe {
        line.typographic_bounds(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    let drawn_width = if line_width as CGFloat > w {
        w
    } else {
        line_width as CGFloat
    };
    // Same ink-centered baseline as `draw_tab_label`: center the cap-height
    // band, so the smaller hint glyphs share the tab's vertical centre.
    let cap = unsafe { font.cap_height() };
    let baseline = y + (TAB_HEIGHT - cap) / 2.0;
    let cx = x + w - drawn_width - TAB_HINT_INSET;
    CGContext::save_g_state(ctx);
    CGContext::translate_ctm(ctx, cx, baseline);
    CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(color)));
    CGContext::set_text_position(ctx, 0.0, 0.0);
    unsafe { line.draw(ctx.expect("a current context")) };
    CGContext::restore_g_state(ctx);
}

/// A `CGColor` from the tab-strip palette (sRGB, opaque).
fn rgb_color(rgb: (CGFloat, CGFloat, CGFloat)) -> CFRetained<CGColor> {
    let (r, g, b) = rgb;
    CGColor::new_srgb(r, g, b, 1.0)
}

/// A `CTLine` for `text` shaped by `font`, with the font attribute set over
/// the whole string range.
unsafe fn shaped_line(font: &CTFont, text: &CFString) -> CFRetained<CTLine> {
    let attr = CFMutableAttributedString::new(None, 0).expect("an attributed string");
    let insert = CFRange {
        location: 0,
        length: 0,
    };
    unsafe { CFMutableAttributedString::replace_string(Some(&*attr), insert, Some(text)) };
    unsafe {
        CFMutableAttributedString::set_attribute(
            Some(&*attr),
            CFRange {
                location: 0,
                length: text.length(),
            },
            Some(kCTFontAttributeName),
            Some(&**font),
        );
        // `CTLineDraw` defaults to BLACK unless the string opts in to the
        // CGContext fill colour — same trap as `render.rs::run`; without it
        // tab titles/hints paint black-on-black and are invisible.
        CFMutableAttributedString::set_attribute(
            Some(&*attr),
            CFRange {
                location: 0,
                length: text.length(),
            },
            Some(kCTForegroundColorFromContextAttributeName),
            Some(&*kCFBooleanTrue.expect("a boolean")),
        );
    };
    unsafe { CTLine::with_attributed_string(&**attr) }
}

/// The (font, (cell width, cell height)) pair, created once and cached.
///
/// The cache is invalidated (`font_invalidate`) when a font knob changes, so
/// the next pass re-creates both the face and the cell metrics at the new
/// size/family. Reads the settings bundle for exactly that reason: a cached
/// `OnceCell` keeps the FIRST size it was created at, which is why the old
/// hardcoded `FONT_SIZE` argument could never become a runtime knob.
fn ensure_font(
    ivars: &NotchViewIvars,
    settings: &Settings,
) -> (CFRetained<CTFont>, (CGFloat, CGFloat)) {
    let font = ivars
        .font
        .replace(None)
        .unwrap_or_else(|| monospace_with(settings.font_size, Some(&settings.font_family)));
    let cell = ivars.cell.get().unwrap_or_else(|| metrics(&font));
    ivars.font.set(Some(font.clone()));
    ivars.cell.set(Some(cell));
    (font, cell)
}

/// Drop the cached grid font + cell metrics so the next `ensure_font` re-creates
/// them from the (possibly changed) settings bundle. Only the font knobs need
/// this; colors and geometry never touch these cells.
fn font_invalidate(ivars: &NotchViewIvars) {
    ivars.font.set(None);
    ivars.cell.set(None);
}

/// Split accumulated wheel points into `(whole lines to scroll, points to keep)`.
///
/// Pure and panic-free: a non-finite or non-positive `cell_height` — which is
/// what a font-size preview leaves behind, because `font_invalidate` drops the
/// cached cell while the grid resize waits for its deadline — yields no motion
/// and preserves the accumulated points for the next event instead of dividing
/// by zero or unwrapping an empty cache.
fn scroll_lines(cell_height: CGFloat, pts: CGFloat) -> (i32, CGFloat) {
    if cell_height.is_finite() && cell_height > 0.0 {
        let lines = (pts / cell_height).trunc() as i32;
        (lines, pts - lines as f64 * cell_height)
    } else {
        (0, pts)
    }
}

/// The (rows, columns, scrollback) every `Session` must be built with at
/// `cell` metrics and `header` reservation — the single formula shared by
/// every spawn path (initial batch in `draw`, `spawn_tab`) so grid sizes
/// cannot drift. Must stay in sync with `row_rect`'s header reservation.
///
/// The grid reflows to the EFFECTIVE open width — the user's stored preference
/// clamped into the live canvas — and NOT to the live spring width: sessions
/// spawn on the FIRST open frame, while the width spring is still flying at the
/// ~212pt notch, so baking columns from live `w` would silently create
/// half-width sessions. `draw_grid` gets the live width (x0/usable) for
/// painting; a session's row/col count is baked at spawn and then kept in step
/// with the knob by `apply_settings`' live `Session::resize` (see that seam).
fn grid_dims(
    cell: (CGFloat, CGFloat),
    header: CGFloat,
    settings: &Settings,
    canvas: CGFloat,
) -> (usize, usize, usize) {
    let (cell_width, cell_height) = cell;
    let usable = settings.effective_open_width(canvas) - 2.0 * GRID_PAD;
    let columns = ((usable / cell_width).floor() as usize).max(1);
    let rows = (((settings.open_height - header - GRID_TOP_PAD - TAB_HEIGHT - 2.0 * TAB_INSET)
        / cell_height)
        .floor() as usize)
        .max(1);
    // Scrollback is a settings knob now (plan A #12): the `BNT_SCROLLBACK`
    // env override it replaces is gone, so the panel is the only control.
    (rows, columns, settings.scrollback_lines)
}

fn cell_pixels(cell: (CGFloat, CGFloat), scale: CGFloat) -> (CGFloat, CGFloat) {
    ((cell.0 * scale).ceil(), (cell.1 * scale).ceil())
}

impl NotchView {
    /// Creates a closed notch view.
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let (mut settings, mut status) = Settings::load_with_status();
        if let Err(warning) = resolve_monospace(&settings.font_family, settings.font_size) {
            status.push(format!("{warning}; using Menlo"));
            settings.font_family = "Menlo".into();
        }
        let this = Self::alloc(mtm).set_ivars(NotchViewIvars {
            spring: Spring::new(CLOSED_HEIGHT),
            width_spring: Spring::new(NOTCH_WIDTH),
            height: Cell::new(CLOSED_HEIGHT),
            width: Cell::new(NOTCH_WIDTH),
            notch: Cell::new((NOTCH_WIDTH, CLOSED_HEIGHT)),
            last_time: Cell::new(0.0),
            font: Cell::new(None),
            cell: Cell::new(None),
            sessions: Cell::new(vec![]),
            spawned: Cell::new(false),
            tab_count: Cell::new(0),
            selected: Cell::new(None),
            hovered: Cell::new(None),
            label_font: OnceCell::new(),
            hint_font: OnceCell::new(),
            tracking: Cell::new(vec![]),
            tracking_canvas: Cell::new(0.0),
            canvas: Cell::new(crate::window::canvas_size(mtm)),
            backing_scale: Cell::new(1.0),
            scroll: Cell::new(0),
            scroll_pts: Cell::new(0.0),
            settings: RefCell::new(SettingsState::new(settings, status)),
            reduce_motion: Cell::new(
                NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion(),
            ),
            foreground_poll: Cell::new(0.0),
            foreground_titles: RefCell::new(vec![]),
            settings_was_visible: Cell::new(false),
            settings_owner: OnceCell::new(),
        });
        // SAFETY: `NSObject`'s `init` signature is correct.
        unsafe { msg_send![super(this), init] }
    }

    /// Sets the measured physical notch size, used as the closed size.
    pub fn set_notch(&self, size: (CGFloat, CGFloat)) {
        let ivars = self.ivars();
        let open = self.is_open();
        ivars.notch.set(size);
        if let Some(window) = self.as_super().window() {
            ivars.backing_scale.set(window.backingScaleFactor());
        }
        let target = if open {
            self.current_settings().open_height
        } else {
            size.1
        };
        self.set_target(target);
    }

    /// Retarget both springs without resetting position or velocity.
    pub fn set_target(&self, target: f64) {
        let ivars = self.ivars();
        let was_open = self.is_open();
        let open = target > ivars.notch.get().1 + 1.0;
        let height = displayed(target, ivars.notch.get().1, PANEL_HEIGHT);
        let width = if open {
            self.current_settings()
                .effective_open_width(ivars.canvas.get())
        } else {
            ivars.notch.get().0
        };
        ivars.spring.retarget(height);
        ivars
            .width_spring
            .retarget(displayed(width, ivars.notch.get().0, ivars.canvas.get()));
        ivars.height.set(displayed(
            ivars.height.get(),
            ivars.notch.get().1,
            PANEL_HEIGHT,
        ));
        ivars.width.set(displayed(
            ivars.width.get(),
            ivars.notch.get().0,
            ivars.canvas.get(),
        ));
        if ivars.reduce_motion.get() {
            self.snap_motion();
        }
        if open {
            ivars
                .settings
                .borrow_mut()
                .require_resize(CFAbsoluteTimeGetCurrent());
            self.pump_settings(false)
                .unwrap_or_else(|error| self.record_status(error.to_string()));
            if !was_open && let Some(window) = self.as_super().window() {
                window.makeKeyWindow();
            }
        }
        self.mark_dirty();
    }

    pub fn is_open(&self) -> bool {
        self.ivars().spring.target_value() > self.ivars().notch.get().1 + 1.0
    }

    fn snap_motion(&self) {
        let ivars = self.ivars();
        let (h, w) = motion_frame(
            &ivars.spring,
            &ivars.width_spring,
            0.0,
            true,
            ivars.notch.get(),
            (ivars.canvas.get(), PANEL_HEIGHT),
        );
        ivars.height.set(h);
        ivars.width.set(w);
    }

    pub fn update_reduce_motion(&self) {
        let enabled = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        self.ivars().reduce_motion.set(enabled);
        if enabled {
            self.snap_motion();
            self.mark_dirty();
        }
    }

    /// Toggles between the closed and open silhouette heights. The open
    /// target is the settings knob (whose default is `OPEN_HEIGHT`), so a
    /// height-% change takes effect on the very next toggle.
    pub fn toggle(&self) {
        if self.is_open() {
            self.collapse();
        } else {
            self.expand();
        }
    }

    /// The height the silhouette was last drawn at.
    /// Dirty the view AND its window: a non-layer-backed view inside a
    /// non-activating panel is only repainted when the window's own
    /// `viewsNeedDisplay` flag is set.
    pub fn mark_dirty(&self) {
        self.as_super().setNeedsDisplay(true);
        if let Some(window) = self.as_super().window() {
            window.setViewsNeedDisplay(true);
        }
    }

    pub fn current_height(&self) -> f64 {
        self.ivars().height.get()
    }

    pub fn current_settings(&self) -> SettingsSnapshot {
        self.ivars().settings.borrow().current.clone()
    }

    pub fn settings_status(&self) -> Vec<String> {
        self.ivars().settings.borrow().status.clone()
    }

    pub fn record_status(&self, message: String) {
        eprintln!("NotchQuake: {message}");
        self.ivars().settings.borrow_mut().status.push(message);
    }

    pub fn show_settings(&self) {
        crate::settings_window::open_or_focus(self.mtm(), &self.ivars().settings_owner, self);
    }

    /// Validate and register before acceptance. Rejection never writes preferences.
    pub fn apply_settings(
        &self,
        mut after: Settings,
        phase: EditPhase,
    ) -> Result<SettingsSnapshot, SettingsError> {
        let ivars = self.ivars();
        ivars
            .settings
            .borrow()
            .validate_proposal(&after, phase, ivars.canvas.get())?;
        let resolved = resolve_monospace(&after.font_family, after.font_size)
            .map_err(SettingsError::Validation)?;
        after.font_family = resolved.family;
        let before = self.current_settings();
        if after.toggle_chord != before.toggle_chord {
            crate::hotkey::change_toggle(self.mtm(), after.toggle_chord)
                .map_err(|e| SettingsError::Hotkey(e.to_string()))?;
        }
        let font_changed =
            after.font_size != before.font_size || after.font_family != before.font_family;
        let geometry_changed =
            after.open_height != before.open_height || after.open_width != before.open_width;
        let changed = ivars
            .settings
            .borrow_mut()
            .accept(after, phase, CFAbsoluteTimeGetCurrent());
        if font_changed {
            font_invalidate(ivars);
        }
        if changed {
            if geometry_changed && self.is_open() {
                // Retarget without stealing focus from native controls. The
                // width is the EFFECTIVE one, so a proposal carrying a stored
                // preference wider than the canvas cannot aim the spring past
                // the display.
                let settings = self.current_settings();
                ivars.spring.retarget(settings.open_height);
                ivars
                    .width_spring
                    .retarget(settings.effective_open_width(ivars.canvas.get()));
                if ivars.reduce_motion.get() {
                    self.snap_motion();
                }
            }
            self.mark_dirty();
        }
        self.pump_settings(phase == EditPhase::Commit)
            .map_err(SettingsError::Persistence)?;
        Ok(self.current_settings())
    }

    pub fn flush_settings(&self) -> Result<(), PersistenceError> {
        self.pump_settings(true)
    }

    fn pump_settings(&self, flush: bool) -> Result<(), PersistenceError> {
        let ivars = self.ivars();
        let now = CFAbsoluteTimeGetCurrent();
        let (resize, save) = {
            let mut state = ivars.settings.borrow_mut();
            if flush {
                state.flush(now);
            }
            (state.resize_due(now, self.is_open()), state.save_due(now))
        };
        if resize {
            let settings = self.current_settings();
            let (_, cell) = ensure_font(ivars, &settings);
            let (rows, columns, _) =
                grid_dims(cell, header_height(ivars), &settings, ivars.canvas.get());
            let mut sessions = take_sessions(ivars);
            let pixels = cell_pixels(cell, ivars.backing_scale.get());
            for session in &mut sessions {
                session.resize(rows, columns, pixels.0, pixels.1);
                session.set_scrollback(settings.scrollback_lines);
            }
            put_sessions(ivars, sessions);
            self.mark_dirty();
        }
        if let Some(snapshot) = save {
            // Submission only; NSUserDefaults exposes no durable-transaction result.
            snapshot.save()?;
            ivars.settings.borrow_mut().saved(snapshot);
        }
        Ok(())
    }

    /// True only when consumed. The monitor returns the original event otherwise.
    pub fn handle_key(&self, event: &NSEvent) -> bool {
        let panel = self.as_super().window();
        let event_window = event.window(self.mtm());
        let app = NSApplication::sharedApplication(self.mtm());
        let key_window = app.keyWindow();
        let same = |other: &Option<Retained<objc2_app_kit::NSWindow>>| {
            panel
                .as_ref()
                .zip(other.as_ref())
                .is_some_and(|(p, o)| std::ptr::eq(&**p, &**o))
        };
        let flags = event.modifierFlags();
        let text = event
            .characters()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let route = input::classify(
            Context {
                terminal_open: self.is_open(),
                panel_is_key: same(&key_window),
                event_is_panel: same(&event_window),
            },
            event.keyCode(),
            &text,
            Modifiers {
                command: flags.contains(NSEventModifierFlags::Command),
                option: flags.contains(NSEventModifierFlags::Option),
                control: flags.contains(NSEventModifierFlags::Control),
                shift: flags.contains(NSEventModifierFlags::Shift),
            },
            // The EFFECTIVE local chord: a stored value that cannot be routed
            // (reserved, or equal to the toggle) is replaced by the terminal-local
            // fallback, so the fallback chord actually works.
            self.current_settings().effective_chords().1,
        );
        match route {
            Route::PassThrough => false,
            Route::Command(Command::NewTab) => {
                self.spawn_tab();
                true
            }
            Route::Command(Command::CloseTab) => {
                if let Some(i) = self.ivars().selected.get() {
                    self.close_tab(i);
                }
                true
            }
            Route::Command(Command::SelectTab(i)) => {
                self.select_tab(i);
                true
            }
            Route::Command(Command::CopyUnavailable) => true,
            Route::Command(Command::Paste) => {
                // Clipboard access only for an explicit runtime paste action.
                let text = NSPasteboard::generalPasteboard()
                    .stringForType(unsafe { NSPasteboardTypeString });
                if let Some(text) = text {
                    let sessions = take_sessions(self.ivars());
                    if let Some(session) = self.ivars().selected.get().and_then(|i| sessions.get(i))
                    {
                        session.send_input(input::paste_bytes(
                            &text.to_string(),
                            session.bracketed_paste(),
                        ));
                    }
                    put_sessions(self.ivars(), sessions);
                }
                true
            }
            Route::Bytes(bytes) => {
                let sessions = take_sessions(self.ivars());
                if let Some(session) = self.ivars().selected.get().and_then(|i| sessions.get(i)) {
                    session.send_input(bytes);
                }
                put_sessions(self.ivars(), sessions);
                true
            }
        }
    }

    /// Post at most one notification per (a) raw bell and (b) finished
    /// command, for every tab that is NOT the selected one. Runs on the 60Hz
    /// `tick`; the `take_*` accessors consume the flags so each event fires
    /// exactly once. A finished command notifies only when it ran >5s
    /// (iTerm's rule), measured from the `133;C`/`133;D` gap (T1a/T2a).
    ///
    /// The flags are drained for EVERY tab, including the selected one — only
    /// the notification is suppressed there. If we left the selected tab's
    /// flags set, a bell or finish that happened while the user was looking
    /// would fire a stale banner the moment they switched away.
    pub fn drain_notifications(&self) {
        let ivars = self.ivars();
        let selected = ivars.selected.get();
        let settings = self.current_settings();
        let sessions = take_sessions(ivars);
        for (index, session) in sessions.iter().enumerate() {
            let looking = self.is_open() && Some(index) == selected;
            let bell = session.take_bell();
            let finished = session.take_finished();
            if looking {
                continue;
            }
            let name = session.base_title();
            if bell && settings.bell_enabled {
                crate::terminal::notify(&name, "Bell");
            }
            if let Some((status, secs)) = finished {
                // iTerm's rule: only a command that ran longer than the
                // settings threshold is worth a banner (default 5s).
                if secs > settings.finished_notify_secs {
                    let message = if status == 0 {
                        "Command completed".to_string()
                    } else {
                        format!("Exited with status {status}")
                    };
                    crate::terminal::notify(&name, &message);
                }
            }
        }
        put_sessions(ivars, sessions);
    }

    /// Spawn a new tab (a fresh login-shell session) at the shared grid
    /// size, capped at `MAX_TABS`. Only while open.
    pub fn spawn_tab(&self) {
        let ivars = self.ivars();
        if !self.is_open() {
            return;
        }
        let mut sessions = take_sessions(ivars);
        if sessions.len() >= MAX_TABS {
            put_sessions(ivars, sessions);
            return;
        }
        let (_, cell) = ensure_font(ivars, &self.current_settings());
        let (rows, columns, scrollback) = grid_dims(
            cell,
            header_height(ivars),
            &self.current_settings(),
            ivars.canvas.get(),
        );
        ivars.hovered.set(None); // session indices are about to shift
        let pixels = cell_pixels(cell, ivars.backing_scale.get());
        sessions.push(Session::new(rows, columns, pixels.0, pixels.1, scrollback));
        let selected = sessions.len() - 1;
        put_sessions(ivars, sessions);
        self.select_tab(selected);
        self.mark_dirty();
    }

    /// Destroy the tab at `index` (kills its PTY via `Msg::Shutdown`),
    /// select the tab that followed it (or the last one), and collapse the
    /// panel when the last tab closes.
    pub fn close_tab(&self, index: usize) {
        let ivars = self.ivars();
        let mut sessions = take_sessions(ivars);
        if index >= sessions.len() {
            put_sessions(ivars, sessions);
            return;
        }
        let mut removed = vec![false; sessions.len()];
        removed[index] = true;
        let selected = selection_after_removal(ivars.selected.get(), &removed);
        let doomed = sessions.remove(index);
        doomed.destroy();
        put_sessions(ivars, sessions);
        ivars.hovered.set(None);
        ivars.selected.set(selected);
        if let Some(index) = selected {
            self.select_tab(index);
        } else {
            self.collapse();
        }
        self.mark_dirty();
    }

    /// Tear down every session whose child shell has exited (one pass for
    /// all of them), with the same selection fix-up as `close_tab`. Called
    /// unconditionally from `tick` — a dead session while closed still
    /// needs teardown so its PTY dies.
    pub fn reap_dead_sessions(&self) {
        let ivars = self.ivars();
        let sessions = take_sessions(ivars);
        if !sessions.iter().any(|s| s.dead()) {
            put_sessions(ivars, sessions);
            return;
        }
        let removed: Vec<_> = sessions.iter().map(Session::dead).collect();
        let selected = selection_after_removal(ivars.selected.get(), &removed);
        let survivors = sessions
            .into_iter()
            .zip(removed)
            .filter_map(|(session, dead)| {
                if dead {
                    session.destroy();
                    None
                } else {
                    Some(session)
                }
            })
            .collect();
        put_sessions(ivars, survivors);
        ivars.hovered.set(None);
        ivars.selected.set(selected);
        if let Some(index) = selected {
            self.select_tab(index);
        } else {
            self.collapse();
        }
        self.mark_dirty();
    }

    /// Handle a click observed by the GLOBAL mouse monitor. A non-activating
    /// panel never owns the click (it passes through to the window below, or
    /// — for synthetic/test clicks — is delivered to the frontmost app), so
    /// the event has no owning window in our app. Use the absolute screen mouse
    /// location and hit-test. (Real clicks ON the panel go to `mouseDown:`
    /// instead; the two paths don't double-fire.)
    /// Handle a click observed by the GLOBAL mouse monitor. A non-activating
    /// panel never owns the click (it passes through to the window below, or
    /// — for synthetic/test clicks — is delivered to the frontmost app), so
    /// the event has no owning window in our app. Use the absolute screen mouse
    /// location and hit-test. (Real clicks ON the panel also reach `mouseDown:`;
    /// the handlers below are idempotent — they set an explicit target rather
    /// than toggling — so a click seen by both paths lands once.)
    pub fn handle_click(&self, _event: &NSEvent) {
        let screen = NSEvent::mouseLocation();
        self.click_at_screen_point(&screen);
    }

    /// Record which tab the cursor is over (a hover palette change, no
    /// selection change). `locationInWindow` is already in this view's y-up
    /// panel-local space (the view fills the panel), so use it directly.
    pub fn update_hover(&self, event: &NSEvent) {
        // The panel must exist for a view-local point to mean anything; the
        // frame itself is not needed (the view fills the panel).
        match self.as_super().window() {
            Some(_) => {}
            None => return,
        }
        let loc = event.locationInWindow();
        let px = loc.x;
        let py = loc.y;
        let ivars = self.ivars();
        let h = ivars.height.get();
        if h <= CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT {
            return;
        }
        let w = ivars.width.get();
        let new = match tab_strip(h, w, ivars.tab_count.get(), ivars.canvas.get()) {
            Some((left, tab_w)) => {
                let chin_y = PANEL_HEIGHT - h;
                if py < chin_y || py > chin_y + TAB_HEIGHT {
                    None
                } else {
                    (0..ivars.tab_count.get()).find(|&i| {
                        let x0 = left + i as CGFloat * tab_w;
                        px >= x0 && px <= x0 + tab_w
                    })
                }
            }
            None => None,
        };
        if ivars.hovered.get() != new {
            ivars.hovered.set(new);
            self.mark_dirty();
        }
    }

    /// Handle a click at an AppKit screen point. Inside the panel: a closed
    /// pill opens, an open panel selects the tab under the cursor. Outside the
    /// panel (or inside its frame but off the silhouette): an OPEN panel
    /// collapses — the user clicked elsewhere, so the terminal is dismissed.
    /// `mouseDown:` only ever delivers points inside the panel, so it can
    /// never collapse; the global monitor delivers the outside clicks.
    pub fn click_at_screen_point(&self, screen: &NSPoint) {
        let panel = self
            .as_super()
            .window()
            .expect("a view always lives in a window");
        let panel_frame = panel.frame();
        let x = screen.x - panel_frame.origin.x;
        let y = screen.y - panel_frame.origin.y;
        let ivars = self.ivars();
        let h = ivars.height.get();
        let open = self.is_open();

        // Native auxiliary windows get their own clicks, including color panels/About.
        for window in NSApplication::sharedApplication(self.mtm()).windows() {
            if std::ptr::eq(&*window, &*panel) || !window.isVisible() || window.isMiniaturized() {
                continue;
            }
            let frame = window.frame();
            if screen.x >= frame.origin.x
                && screen.x <= frame.origin.x + frame.size.width
                && screen.y >= frame.origin.y
                && screen.y <= frame.origin.y + frame.size.height
            {
                return;
            }
        }

        // The settings WINDOW is a separate layer-0 window, so a click on one
        // of its controls is observed by our global `LeftMouseDown` monitor
        // and hit-tests as OUTSIDE the panel — which would COLLAPSE an open
        // notch (the confirmed regression: a click in the window took the
        // panel from 461pt to the 33pt pill). A click inside our own window's
        // frame belongs to that window, so bail before any dismissal branch.
        if let Some(frame) = crate::settings_window::window_frame(&ivars.settings_owner) {
            if screen.x >= frame.origin.x
                && screen.x <= frame.origin.x + frame.size.width
                && screen.y >= frame.origin.y
                && screen.y <= frame.origin.y + frame.size.height
            {
                return;
            }
        }

        // Outside the panel frame entirely: collapse if open, else ignore.
        if x < 0.0 || x > panel_frame.size.width || y < 0.0 || y > panel_frame.size.height {
            if open {
                self.collapse();
            }
            return;
        }

        // Inside the frame but off the silhouette (transparent margin): same
        // dismissal.
        // Inside the frame but OFF the silhouette: same dismissal. Both gates
        // matter now that the canvas is the whole display width — a click
        // beside the terminal at terminal height used to fall outside the
        // 640pt frame (collapse) and must keep collapsing; without the x gate
        // it would fall through to the gear/tab hit-tests and do nothing.
        // Placed BEFORE the closed-pill arm so clicking the pill still opens.
        let top = PANEL_HEIGHT - h;
        let w = ivars.width.get();
        let canvas = ivars.canvas.get();
        let (sx0, sx1) = ((canvas - w) / 2.0, (canvas + w) / 2.0);
        if x < sx0 || x > sx1 || y < top || y > PANEL_HEIGHT {
            if open {
                self.collapse();
            }
            return;
        }

        if !open {
            // Closed pill: a click inside it opens (same as the hotkey).
            self.expand();
            return;
        }

        // `canvas` is the ivar (not the panel frame width): `reposition` keeps
        // them in sync, and the same value drives the renderer above.
        let canvas = ivars.canvas.get();
        // 0) The gear button, top-right of the header band. Routed BEFORE the
        // tab-strip guard because the header sits above the strip and the
        // strip guard early-returns for every point outside it. Gated on
        // `open` (the check above), so a click on the closed pill still just
        // expands. One branch covers `mouseDown:` and the global monitor,
        // which both funnel here.
        let header = header_height(ivars);
        let (gx0, gy0, gx1, gy1) = gear_rect(w, header, canvas);
        if x >= gx0 && x <= gx1 && y >= gy0 && y <= gy1 {
            // The gear opens (or re-focuses) the separate settings window.
            // `open_or_focus` creates the owner at most once and stores it in
            // the ivar above, so a second gear click cannot duplicate the
            // window — it only re-shows and re-claims key focus. It never
            // closes the window: the user closes it with its own ✕.
            let mtm = MainThreadMarker::new().expect("the view is main-thread-only");
            crate::settings_window::open_or_focus(mtm, &ivars.settings_owner, self);
            return;
        }

        // `count` mirrors `sessions.len()` (see invariant comment on
        // `tab_count`); the `+` and `×` rects derive from `tab_strip`'s
        // outputs — same math as the renderer in `draw`.
        let count = ivars.tab_count.get();
        match tab_strip(h, w, count, canvas) {
            Some((left, tab_w)) => {
                let chin_y = PANEL_HEIGHT - h;
                if y < chin_y || y > chin_y + TAB_HEIGHT {
                    return;
                }
                // 1) The `+` button — spawn a new session/tab.
                let px = left + count as CGFloat * tab_w;
                if x >= px && x <= px + TAB_PLUS_W {
                    self.spawn_tab();
                    return;
                }
                // 2) A tab: the `×` first (only on the hovered tab), else select.
                for index in 0..count {
                    let x0 = left + index as CGFloat * tab_w;
                    if x >= x0 && x <= x0 + tab_w {
                        let (cx, cy) = close_center(x0, chin_y);
                        // The `×` exists only on a HOVERED tab (the pointer got
                        // there via `mouseMoved:` before the click), so the hit-
                        // test matches the draw: a click in an un-hovered tab's
                        // gutter just selects the tab.
                        let on_close = ivars.hovered.get() == Some(index)
                            && (x - cx).abs() <= CLOSE_HIT
                            && (y - cy).abs() <= CLOSE_HIT;
                        if on_close {
                            self.close_tab(index);
                        } else {
                            self.select_tab(index);
                        }
                        return;
                    }
                }
            }
            _ => {}
        }
    }

    /// Make the tab at `index` the selected one AND clear its activity flag:
    /// a tab the user is looking at never keeps its blue dot (iTerm clears it
    /// on select). Every selection change goes through here so the clear
    /// cannot be forgotten by one call site.
    pub fn select_tab(&self, index: usize) {
        let ivars = self.ivars();
        if index >= ivars.tab_count.get() {
            return;
        }
        ivars.selected.set(Some(index));
        let sessions = take_sessions(ivars);
        match sessions.get(index) {
            Some(session) => session.take_attention(),
            _ => {}
        }
        put_sessions(ivars, sessions);
        self.mark_dirty();
    }

    /// Re-pin the panel to the notch display's top-centre. A display change
    /// (monitor unplugged, resolution changed) moves the desktop but the
    /// window was placed once at startup, so it ends up off-centre. Called
    /// from the screen-change monitor.
    pub fn reposition(&self) {
        let mtm = MainThreadMarker::new().expect("main thread");
        let panel = self
            .as_super()
            .window()
            .expect("a view always lives in a window");
        let ivars = self.ivars();
        match crate::window::notch_screen(mtm) {
            Some(screen) => {
                let frame = screen.frame();
                let canvas = crate::window::canvas_width(&screen);
                panel.setFrameOrigin(crate::window::panel_origin(frame, canvas));
                // The canvas follows the display width: re-pin the window frame
                // (origin + size in the same call, so the two can never disagree)
                // and hand the same value to the view and its content view, since
                // every centring expression reads `ivars.canvas`.
                panel.setFrame_display(
                    NSRect::new(
                        crate::window::panel_origin(frame, canvas),
                        NSSize::new(canvas, PANEL_HEIGHT),
                    ),
                    true,
                );
                ivars.canvas.set(canvas);
                // The stored open width may now exceed the canvas. Nothing is
                // re-persisted for a geometry change: the preference stays raw
                // and every consumer clamps through `effective_open_width`, so
                // re-target the width spring at the effective value here and
                // schedule an immediate grid reflow when the panel is open. A
                // display change can otherwise leave the PTY grid at its old
                // column count until another settings/open transition.
                let width = self.current_settings().effective_open_width(canvas);
                let open = self.is_open();
                ivars
                    .width_spring
                    .retarget(displayed(width, ivars.notch.get().0, canvas));
                if open {
                    ivars
                        .settings
                        .borrow_mut()
                        .require_resize(CFAbsoluteTimeGetCurrent());
                }
                if width < self.current_settings().open_width {
                    let message = format!(
                        "Open width is limited to {width:.0}pt on this display; your saved preference is kept"
                    );
                    // Screen-parameter notifications can fire repeatedly for one
                    // resize, so the note is recorded once, not per frame.
                    if !self.settings_status().contains(&message) {
                        self.record_status(message);
                    }
                }
                // Re-set the view's own frame (it fills the canvas, parent-relative),
                // mirroring `main.rs`'s initial `setFrame`.
                self.as_super().setFrame(NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(canvas, PANEL_HEIGHT),
                ));
                // The tracking area sized to the OLD canvas is stale at the new
                // edges, but it is NOT removed here: `drawRect:` sees
                // `tracking_canvas != canvas` and does the remove+reinstall pair
                // on the next frame, so only ONE place owns that swap. Hover is
                // not driver-verifiable (lesson 47), so this is correctness-by
                // construction.
                let (w, h) = crate::window::notch_size(&screen);
                self.set_notch((w, h));
            }
            _ => {}
        }
    }

    /// Open the panel (idempotent: already-open stays open) at the settings
    /// open height.
    pub fn expand(&self) {
        self.set_target(self.current_settings().open_height);
    }

    /// Collapse the panel and reset the scroll position so reopening shows
    /// the live viewport (idempotent: already-closed stays closed). The settings
    /// WINDOW is not collapsed along with it: it is a separate window the user
    /// closes with its own ✕, and a closed pill has no room for it anyway.
    pub fn collapse(&self) {
        let ivars = self.ivars();
        ivars.scroll.set(0);
        ivars.scroll_pts.set(0.0);
        let sessions = take_sessions(ivars);
        for session in &sessions {
            session.scroll_display(0);
        }
        put_sessions(ivars, sessions);
        self.set_target(CLOSED_HEIGHT);
    }

    /// Scroll the selected session's history by wheel points accumulated
    /// across events (a single event's `deltaY` is rarely a whole line).
    /// Apple's `deltaY` is positive for a DOWNWARD scroll gesture, which
    /// reveals content BELOW the viewport (newer), so the line delta is
    /// negated against alacritty's `Scroll::Delta(+n)` = show older.
    ///
    /// The line metric comes from the cache when it is warm. A font-size or
    /// family preview invalidates it (`font_invalidate`) and defers the grid
    /// resize to a settings deadline, so a wheel event can land in the window
    /// where nothing has repainted yet; `ensure_font` is then the same call
    /// `draw` makes, so it rebuilds the face and metric from the CURRENT
    /// settings. That restores a usable metric without panicking and keeps the
    /// preview semantics intact — the next draw still resolves the selected font.
    pub fn scroll_wheel(&self, delta_y: CGFloat) {
        let ivars = self.ivars();
        if ivars.height.get() <= CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT {
            return;
        }
        let cell = ivars.cell.get().unwrap_or_else(|| {
            let (_, cell) = ensure_font(ivars, &self.current_settings());
            cell
        });
        ivars.scroll_pts.set(ivars.scroll_pts.get() - delta_y);
        let (lines, carry) = scroll_lines(cell.1, ivars.scroll_pts.get());
        ivars.scroll_pts.set(carry);
        if lines == 0 {
            return;
        }
        ivars.scroll.set(ivars.scroll.get() + lines);
        let sessions = take_sessions(ivars);
        match ivars.selected.get().and_then(|i| sessions.get(i)) {
            Some(session) => session.scroll_display(lines),
            _ => {}
        }
        put_sessions(ivars, sessions);
        self.mark_dirty();
    }
}

#[cfg(test)]
mod core_tests {
    use super::*;

    #[test]
    fn grid_dimensions_use_shared_selected_metrics_and_canvas_bounds() {
        let s = Settings::default();
        let (_, wide, _) = grid_dims((8.0, 16.0), 33.0, &s, 1920.0);
        let (_, narrow, _) = grid_dims((8.0, 16.0), 33.0, &s, 480.0);
        assert_eq!(wide, 78);
        assert_eq!(narrow, 58);
        let (rows, columns, _) = grid_dims((16.0, 32.0), 33.0, &s, 1920.0);
        assert_eq!(columns, 39);
        assert!(rows > 0);
        assert_eq!(cell_pixels((8.1, 16.2), 2.0), (17.0, 33.0));
    }

    #[test]
    fn wheel_scroll_is_safe_when_a_preview_invalidated_the_cell_cache() {
        // A font-size preview clears both cached cells and defers the resize, so
        // the wheel path can run before any draw rebuilt metrics. It derives the
        // metric from the resolved settings rather than unwrapping the cache,
        // and the arithmetic below is that path with the cache absent.
        let settings = Settings::default();
        let resolved = resolve_monospace(&settings.font_family, settings.font_size)
            .unwrap_or_else(|_| resolve_monospace("Menlo", settings.font_size).unwrap());
        assert!(resolved.cell.1.is_finite() && resolved.cell.1 > 0.0);

        let h = resolved.cell.1;
        let (lines, carry) = scroll_lines(h, h * 2.5);
        assert_eq!(lines, 2);
        assert!((carry - h * 0.5).abs() < 1e-9);

        // The invalidated-cache shape: no usable metric means no motion, and the
        // accumulated points survive for the next event instead of panicking.
        assert_eq!(scroll_lines(0.0, 40.0), (0, 40.0));
        assert_eq!(scroll_lines(f64::NAN, 40.0), (0, 40.0));
        assert_eq!(scroll_lines(h, 0.0), (0, 0.0));
        // Sub-line accumulation defers until a whole line exists.
        assert_eq!(scroll_lines(20.0, 15.0), (0, 15.0));
    }
}

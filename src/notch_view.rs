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

use objc2::ClassType;
use objc2::define_class;
use objc2::DefinedClass;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::MainThreadOnly;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_app_kit::{NSGraphicsContext, NSView, NSEvent, NSEventModifierFlags, NSTrackingArea, NSTrackingAreaOptions};
use objc2_core_text::{CTFont, CTLine, CTLineTruncationType, kCTFontAttributeName, kCTForegroundColorFromContextAttributeName};
use objc2_core_foundation::kCFBooleanTrue;
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent,
    CGFloat,
    CFRetained,
    CGPoint,
    CGRect,
    CGSize,
};
use objc2_core_graphics::{CGColor, CGContext};
use objc2_core_foundation::{
    CFMutableAttributedString,
    CFRange,
    CFString,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize};
use objc2_foundation::NSString;
use std::cell::Cell;
use std::cell::OnceCell;
use core::ffi::c_double;
use crate::animation::Spring;
use crate::consts::{BOTTOM_CORNER_RADIUS, CLOSED_HEIGHT, DEFAULT_SCROLLBACK_LINES, NOTCH_WIDTH, OPEN_HEIGHT, PANEL_HEIGHT, PANEL_WIDTH};
use crate::font::{FONT_SIZE, monospace, cell_metrics, ui_font};
use crate::render::{draw_grid, GRID_PAD, GRID_TOP_PAD};
use crate::terminal::Session;

/// Tab-bar geometry: an iTerm-style strip hanging from the silhouette's
/// bottom edge — tabs are rounded-top-corner rectangles flush with the chin
/// line, separated by thin dividers, with a `+` button at the right end.
/// Sessions spawned on the first open frame (after that, tabs are dynamic).
const INITIAL_TABS: usize = 3;
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
    font: OnceCell<CFRetained<CTFont>>,
    cell: OnceCell<(CGFloat, CGFloat)>,
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
    tracking: OnceCell<Retained<NSTrackingArea>>,
    // Wheel-scroll state: `scroll` is the whole lines already applied to the
    // selected session's display_offset; `scroll_pts` carries the leftover
    // fractional wheel points between events. Reset on collapse.
    scroll: Cell<i32>,
    scroll_pts: Cell<f64>,
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

            // Wall-clock delta since the previous draw pass. A zero
            // `last_time` means "the spring was just retargeted": the next
            // pass starts the curve at its beginning.
            let now = CFAbsoluteTimeGetCurrent();
            let previous = ivars.last_time.get();
            let delta = if previous == 0.0 {
                0.0
            } else {
                now - previous
            };
            ivars.last_time.set(now);

            let height = ivars.spring.advance(delta);
            ivars.height.set(height);
            ivars.width.set(ivars.width_spring.advance(delta));
            if let Some(graphics) = NSGraphicsContext::currentContext() {
                let ctx = graphics.CGContext();
                draw(Some(&ctx), height, ivars.width.get(), ivars);
            } else {
                // A blank panel is otherwise silent: make it diagnosable.
                eprintln!("notch-terminal: no current graphics context in drawRect");
            }

            // Keep the view dirty while the spring is still flying so AppKit
            // re-invokes `drawRect:` next pass.
            if !ivars.spring.settled() {
                self.mark_dirty();
            }

            // The hover tracking area, installed once over the whole panel.
            // `ActiveInKeyWindow` ties hover to key focus, which the panel
            // takes on click (see Item 1).
            if ivars.tracking.get().is_none() {
                let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PANEL_WIDTH, PANEL_HEIGHT));
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
                let _ = ivars.tracking.set(area);
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
            // Redraw while the spring is animating OR while the panel is open,
            // so PTY output (processed on the reader thread) is painted. The
            // spring-only redraw left the grid frozen once the open animation
            // settled — the terminal never updated even as the shell wrote its
            // prompt. A terminal is meant to redraw continuously while open.
            let open = ivars.height.get() > CLOSED_HEIGHT + 1.0;
            if !ivars.spring.settled() || open {
                self.mark_dirty();
                if let Some(window) = self.as_super().window() {
                    window.update();
                    window.displayIfNeeded();
                }
            }
        }
    }
);

/// The silhouette path, traced in a top-left-origin rect `(0, 0, PANEL_WIDTH,
/// h)`: square top corners at the menu bar (a straight top edge, since the
/// panel sits flush under the notch), then the chin corners rounded at
/// `BOTTOM_CORNER_RADIUS`, which also inset the bottom edge on both sides.
fn trace_silhouette(ctx: Option<&CGContext>, h: CGFloat, w: CGFloat) {
    // AppKit clamps corner radii to the rect; do the same so the closed pill
    // (h < 2 * bottom) stays a proper pill instead of self-intersecting.
    let bottom = chin_radius(h);

    // y-up space: the silhouette hangs from the panel top, so the menu-bar
    // edge is at `max_y` and the chin (bottom) at `min_y`. The shape is
    // horizontally centred in the fixed panel.
    let (min_x, min_y) = ((PANEL_WIDTH - w) / 2.0, PANEL_HEIGHT - h);
    let (max_x, max_y) = ((PANEL_WIDTH + w) / 2.0, PANEL_HEIGHT);


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
    let h = if height < 1.0 {
        1.0
    } else {
        height
    };
    let w = if width < 1.0 {
        1.0
    } else {
        width
    };

    // The layer-backed view's context is already top-left, y-down (AppKit
    // flips layer-backed views), so drawing uses panel-local coordinates
    // measured down from the panel top.

    CGContext::save_g_state(ctx);

    // The silhouette.
    trace_silhouette(ctx, h, w);
    CGContext::set_fill_color_with_color(ctx, Some(&CGColor::new_generic_gray(0.0, 1.0)));
    CGContext::fill_path(ctx);

    // Everything below stays inside the silhouette. `CGContextFillPath`
    // CONSUMES the current path, so the clip needs its OWN trace: clipping the
    // (already empty) path silently did nothing. That went unnoticed only
    // because the strip used to stop short of the chin corners — once the
    // strip spans the full width it painted straight over them, squaring the
    // silhouette's rounded bottom corners. Re-trace, then clip.
    trace_silhouette(ctx, h, w);
    CGContext::clip(ctx);

    // Take the session vec once for the whole frame (put it back at the
    // end, before `restore_g_state`): the grid pass, spawn-on-open rule and
    // tab labels all read it, and a second take/put pair would be more
    // panic surface.
    let mut sessions = take_sessions(ivars);

    // The terminal grid, once the panel is open. Sessions spawn on the
    // first open frame; the grid keeps its FIXED size (computed from the
    // OPEN height minus the header), so the reveal is a pure clip wipe —
    // no PTY resize. After the last tab is closed (`spawned` set),
    // reopening spawns ONE fresh tab.
    if h > CLOSED_HEIGHT + 1.0 {
        let (font, cell) = ensure_font(ivars);
        let header = header_height(ivars);
        if sessions.is_empty() {
            let count = if ivars.spawned.get() { 1 } else { INITIAL_TABS };
            ivars.spawned.set(true);
            let (rows, columns, scrollback) = grid_dims(cell, header);
            sessions = (0..count).map(|_| Session::new(rows, columns, cell.0, cell.1, scrollback)).collect();
            // Select the new tab — `selected` starts as None and collapse
            // sets it None, so a fresh spawn must always own the selection
            // or `draw_grid` matches `_ => {}` and the terminal renders blank.
            ivars.selected.set(Some(count - 1));
        }
        // The header bar: a filled block hugging the menu-bar edge, height
        // = the measured notch height, so the grid can never slide under
        // the notch. Empty for now (buttons later). Drawn before the grid
        // so the grid cannot overpaint it, inside the silhouette clip.
        CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(TAB_FILL)));
        CGContext::fill_rect(
            ctx,
            CGRect::new(
                CGPoint::new((PANEL_WIDTH - w) / 2.0, PANEL_HEIGHT - header),
                CGSize::new(w, header),
            ),
        );
        // Separator between the header box and the terminal, LINE color.
        CGContext::begin_path(ctx);
        CGContext::move_to_point(ctx, (PANEL_WIDTH - w) / 2.0, PANEL_HEIGHT - header);
        CGContext::add_line_to_point(ctx, (PANEL_WIDTH + w) / 2.0, PANEL_HEIGHT - header);
        CGContext::set_stroke_color_with_color(ctx, Some(&rgb_color(LINE)));
        CGContext::set_line_width(ctx, 1.0);
        CGContext::stroke_path(ctx);
        let selected = ivars.selected.get();
        match selected.and_then(|i| sessions.get(i)) {
          Some(session) => {
            let grid = session.snapshot();
            draw_grid(
                ctx.expect("a current context"),
                &grid,
                session.cursor(),
                &font,
                cell.0,
                cell.1,
                h,
                header,
            )
          }
          _ => {}
        }
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
    match tab_strip(h, w, count) {
      Some((left, tab_w)) => {
        let selected = ivars.selected.get();
        let hovered = ivars.hovered.get();
        for index in 0..count {
            let x = left + index as CGFloat * tab_w;
            let fill = if Some(index) == selected { TAB_FILL_ACTIVE } else { TAB_FILL };
            CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(fill)));
            CGContext::fill_rect(
                ctx,
                CGRect::new(CGPoint::new(x, chin_y), CGSize::new(tab_w, TAB_HEIGHT)),
            );
            // The activity dot: this tab's command produced output while it
            // was NOT the selected one (iTerm's "go look at that tab" marker).
            // Drawn only when the `×` is NOT drawn for this tab — the `×`
            // appears on selected/hovered tabs, so the two are mutually
            // exclusive by construction and can never collide in the shared
            // `TAB_CLOSE_W` gutter. The dot is a FILL (never a stroke: fill
            // and stroke colour are separate CGContext state).
            let looking = Some(index) == selected || hovered == Some(index);
            let active = sessions.get(index).map(|s| s.has_activity()).unwrap_or(false);
            if !looking && active {
                // y-up space: vertically centered in the tab, and centered in
                // the `×` gutter (`TAB_CLOSE_W` = 23pt) so it sits where the
                // title band's reservation already leaves room.
                let cx = x + TAB_CLOSE_W / 2.0;
                let cy = chin_y + TAB_HEIGHT / 2.0;
                CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(DOT_ACTIVE)));
                CGContext::fill_ellipse_in_rect(
                    ctx,
                    CGRect::new(
                        CGPoint::new(cx - DOT_D / 2.0, cy - DOT_D / 2.0),
                        CGSize::new(DOT_D, DOT_D),
                    ),
                );
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
            CGRect::new(CGPoint::new(px, chin_y), CGSize::new(TAB_PLUS_W, TAB_HEIGHT)),
        );
        let cx = px + TAB_PLUS_W / 2.0;
        let cy = chin_y + TAB_HEIGHT * 0.55;
        CGContext::set_fill_color_with_color(ctx, Some(&rgb_color(TEXT_ACTIVE)));
        CGContext::fill_rect(
            ctx,
            CGRect::new(
                CGPoint::new(cx - 5.0, cy - 0.75),
                CGSize::new(10.0, 1.5),
            ),
        );
        CGContext::fill_rect(
            ctx,
            CGRect::new(
                CGPoint::new(cx - 0.75, cy - 5.0),
                CGSize::new(1.5, 10.0),
            ),
        );

        // The tab labels: the shell's OSC title, truncated so the right-aligned
        // `⌘<n>` hint always stays visible. Active TEXT_ACTIVE, inactive
        // TEXT_INACTIVE; ellipsis-truncated when too wide.
        let label_font = ivars.label_font.get_or_init(|| ui_font(TAB_LABEL_SIZE));
        let hint_font = ivars.hint_font.get_or_init(|| ui_font(TAB_HINT_SIZE));
        for index in 0..count {
            let x = left + index as CGFloat * tab_w;
            let label = sessions.get(index)
                .map(|s| s.title())
                .unwrap_or_else(|| format!("Tab {}", index + 1));
            let color = if Some(index) == selected { TEXT_ACTIVE } else { TEXT_INACTIVE };
            // The title band is `x + TAB_CLOSE_W + TAB_TITLE_PAD .. x +
            // tab_w - TAB_HINT_W`. The `×` footprint and the hint reservation
            // are BOTH reserved on EVERY tab, whether or not that tab currently
            // draws a `×` (hover/selection only): the band is therefore
            // identical for all tabs, so the title never re-centres — and thus
            // never shifts or retruncates — when the pointer enters the strip.
            let left_pad = TAB_CLOSE_W + TAB_TITLE_PAD;
            draw_tab_label(ctx, Some(label_font), &label, x + left_pad, chin_y, tab_w - left_pad - TAB_HINT_W, color);
            let hint = format!("\u{2318}{}", index + 1);
            draw_tab_hint(ctx, Some(hint_font), &hint, x, chin_y, tab_w, HINT_ACCENT);
        }
        // The `×` close affordance: shown on the ACTIVE tab always, and on any
        // hovered tab. LEFT-aligned in the tab (reference style). Drawn AFTER
        // the labels so a long title cannot overpaint it.
        for index in 0..count {
            if Some(index) == selected || hovered == Some(index) {
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
fn tab_strip(h: CGFloat, w: CGFloat, count: usize) -> Option<(CGFloat, CGFloat)> {
    if h <= CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT || count == 0 {
        return None
    }
    let inset = TAB_INSET;
    let left = (PANEL_WIDTH - w) / 2.0 + inset;
    let right = (PANEL_WIDTH + w) / 2.0 - inset;
    let span = right - left - TAB_PLUS_W;
    let tab_w = span / count as CGFloat;
    if span <= 0.0 || tab_w < MIN_TAB_W {
        return None
    }
    Some((left, tab_w))
}

/// The `×` close affordance center — LEFT-aligned in the tab (reference
/// style), vertically centered. Single source of truth for the renderer and
/// the click hit-test.
fn close_center(x: CGFloat, chin_y: CGFloat) -> (CGFloat, CGFloat) {
    (x + CLOSE_INSET, chin_y + TAB_HEIGHT / 2.0)
}

/// The header bar height while open: the measured notch height (the panel
/// top hugs the notch). Refreshed by `reposition` via `set_notch`.
fn header_height(ivars: &NotchViewIvars) -> CGFloat {
    let (_, n) = ivars.notch.get();
    if n < 1.0 { CLOSED_HEIGHT } else { n }
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

/// An iTerm tab path: square bottom flush with `y`, rounded top corners at
/// `y + h` (the rounded part faces up into the panel).
fn trace_tab_rect(
    ctx: Option<&CGContext>,
    x: CGFloat,
    y: CGFloat,
    w: CGFloat,
    h: CGFloat,
    r: CGFloat,
) {
    let r = if r < w / 2.0 { r } else { w / 2.0 };
    CGContext::move_to_point(ctx, x, y);
    CGContext::add_line_to_point(ctx, x, y + h - r);
    CGContext::add_curve_to_point(
        ctx,
        x,
        y + h - r * (1.0 - KAPPA),
        x + r * (1.0 - KAPPA),
        y + h,
        x + r,
        y + h,
    );
    CGContext::add_line_to_point(ctx, x + w - r, y + h);
    CGContext::add_curve_to_point(
        ctx,
        x + w - r * (1.0 - KAPPA),
        y + h,
        x + w,
        y + h - r * (1.0 - KAPPA),
        x + w,
        y + h - r,
    );
    CGContext::add_line_to_point(ctx, x + w, y);
    CGContext::close_path(ctx);
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
        return
    }
    let font = font.expect("a label font");
    let text = CFString::from_str(label);
    let line = unsafe { shaped_line(font, &text) };

    let line_width = unsafe { line.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) } as CGFloat;
    let line = if line_width > w {
        let ellipsis = CFString::from_str("\u{2026}");
        let token = unsafe { shaped_line(font, &ellipsis) };
        match unsafe { line.truncated_line(w as c_double, CTLineTruncationType::End, Some(&*token)) } {
          Some(truncated) => truncated,
          _ => return,
        }
    } else {
        line
    };

    // Measure the line we will actually draw (a truncated line is narrower
    // than the original), so the label centers on its own width.
    let drawn_width = unsafe { line.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) } as CGFloat;

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
        return
    }
    let font = font.expect("a hint font");
    let text = CFString::from_str(hint);
    let line = unsafe { shaped_line(font, &text) };
    let line_width = unsafe { line.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
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
    let insert = CFRange { location: 0, length: 0 };
    unsafe { CFMutableAttributedString::replace_string(Some(&*attr), insert, Some(text)) };
    unsafe {
        CFMutableAttributedString::set_attribute(
            Some(&*attr),
            CFRange { location: 0, length: text.length() },
            Some(kCTFontAttributeName),
            Some(&**font),
        );
        // `CTLineDraw` defaults to BLACK unless the string opts in to the
        // CGContext fill colour — same trap as `render.rs::run`; without it
        // tab titles/hints paint black-on-black and are invisible.
        CFMutableAttributedString::set_attribute(
            Some(&*attr),
            CFRange { location: 0, length: text.length() },
            Some(kCTForegroundColorFromContextAttributeName),
            Some(&*kCFBooleanTrue.expect("a boolean")),
        );
    };
    unsafe { CTLine::with_attributed_string(&**attr) }
}

/// The (font, (cell width, cell height)) pair, created once.
fn ensure_font(ivars: &NotchViewIvars) -> (CFRetained<CTFont>, (CGFloat, CGFloat)) {
    let font = ivars.font.get_or_init(|| monospace(FONT_SIZE));
    let cell = ivars.cell.get_or_init(|| cell_metrics(FONT_SIZE));
    (font.clone(), cell.clone())
}

/// The (rows, columns, scrollback) every `Session` must be built with at
/// `cell` metrics and `header` reservation — the single formula shared by
/// every spawn path (initial batch in `draw`, `spawn_tab`) so grid sizes
/// cannot drift. Must stay in sync with `row_rect`'s header reservation.
fn grid_dims(cell: (CGFloat, CGFloat), header: CGFloat) -> (usize, usize, usize) {
    let (cell_width, cell_height) = cell;
    let usable = PANEL_WIDTH - 2.0 * GRID_PAD;
    let columns = ((usable / cell_width).floor() as usize).max(1);
    let rows = (((OPEN_HEIGHT - header - GRID_TOP_PAD - TAB_HEIGHT - 2.0 * TAB_INSET)
                 / cell_height).floor() as usize).max(1);
    // Scrollback cutoff, configurable via BNT_SCROLLBACK (lines of history
    // kept; 0 disables scrolling). Default matches alacritty's 10000.
    let scrollback = std::env::var("BNT_SCROLLBACK").ok()
        .and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(DEFAULT_SCROLLBACK_LINES);
    (rows, columns, scrollback)
}

/// Translate an `NSEvent` keydown into PTY bytes.
fn key_bytes(event: &NSEvent) -> Vec<u8> {
    let keyCode = event.keyCode();
    let flags = event.modifierFlags();
    let alt = flags.contains(NSEventModifierFlags::Option);
    let ctrl = flags.contains(NSEventModifierFlags::Control);

    // Control chords: derive from the CHARACTER, not the keyCode — keyCode
    // numbers are NOT letters (keyCode 4 is H, 2 is D, 13 is W, 10 is Q), so
    // the old `keyCode - 1` mapping sent Ctrl-D for every H. Ctrl+letter is
    // the letter's byte with bit 6 set (h -> 0x08, a -> 0x01).
    if ctrl {
        match event.characters() {
          Some(chars) => {
              let text = chars.to_string();
              match text.as_bytes().first() {
                Some(b) if *b >= 0x40 && *b < 0x7f => return vec![*b & 0x1f],
                _ => {}
              }
          }
          _ => {}
        }
    }

    // Special keys by keyCode (these ARE keyCode-space, not letters).
    // 53=Esc 8=Backspace 13=Return 3=Enter 26=Tab.
    match keyCode {
        53 => return vec![0x1b],
        8 => return vec![0x7f],
        13 => return vec![0x0d],
        3 => return vec![0x0d],
        26 => return vec![0x09],
        _ => {}
    }

    // Printable characters, with the Option-prefixed (M-x) form when Alt is
    // held and the character is ASCII.
    match event.characters() {
      Some(chars) => {
        let text = chars.to_string();
        let bytes = text.as_bytes().to_vec();
        if alt && bytes.len() == 1 && bytes[0] >= 0x20 && bytes[0] < 0x80 {
            return vec![0x1b, bytes[0]]
        }
        return bytes
      }
      _ => {}
    }
    vec![]
}

impl NotchView {
    /// Creates a closed notch view.
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(
            NotchViewIvars {
                spring: Spring::new(CLOSED_HEIGHT),
                width_spring: Spring::new(NOTCH_WIDTH),
                height: Cell::new(CLOSED_HEIGHT),
                width: Cell::new(NOTCH_WIDTH),
                notch: Cell::new((NOTCH_WIDTH, CLOSED_HEIGHT)),
                last_time: Cell::new(0.0),
                font: OnceCell::new(),
                cell: OnceCell::new(),
                sessions: Cell::new(vec![]),
                spawned: Cell::new(false),
                tab_count: Cell::new(0),
                selected: Cell::new(None),
                hovered: Cell::new(None),
                label_font: OnceCell::new(),
                hint_font: OnceCell::new(),
                tracking: OnceCell::new(),
                scroll: Cell::new(0),
                scroll_pts: Cell::new(0.0),
            },
        );
        // SAFETY: `NSObject`'s `init` signature is correct.
        unsafe { msg_send![super(this), init] }
    }

    /// Sets the measured physical notch size, used as the closed size.
    pub fn set_notch(&self, size: (CGFloat, CGFloat)) {
        self.ivars().notch.set(size);
        self.ivars().spring.retarget(size.1);
        self.ivars().width_spring.retarget(size.0);
    }

    /// Retargets the spring to `target` (the current value becomes the new
    /// start, so an interrupted animation reverses from wherever it is).
    pub fn set_target(&self, target: f64) {
        self.ivars().spring.retarget(target);
        self.ivars().width_spring.retarget(if target > CLOSED_HEIGHT + 1.0 { PANEL_WIDTH } else { self.ivars().notch.get().0 });
        self.ivars().last_time.set(0.0);
        self.mark_dirty();
        // Claim key-window focus when opening, so a hotkey-open needs no
        // follow-up click before typing. `canBecomeKeyWindow -> true` makes
        // this legal for a NonactivatingPanel; we never activate the app, so
        // the previous frontmost app stays visually frontmost.
        if target > CLOSED_HEIGHT + 1.0 {
            if let Some(window) = self.as_super().window() {
                window.makeKeyWindow();
            }
        }
    }

    /// Toggles between the closed and open silhouette heights.
    pub fn toggle(&self) {
        let target =
            if self.ivars().spring.target_value() > (CLOSED_HEIGHT + OPEN_HEIGHT) / 2.0 {
                CLOSED_HEIGHT
            } else {
                OPEN_HEIGHT
            };
        self.set_target(target);
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

    /// Route a keystroke event to the selected session's PTY (a no-op when
    /// no tab is selected).
    pub fn handle_key(&self, event: &NSEvent) {
        let bytes = key_bytes(event);
        if bytes.is_empty() {
            return
        }
        let ivars = self.ivars();
        let sessions = take_sessions(ivars);
        let selected = ivars.selected.get();
        match selected.and_then(|i| sessions.get(i)) {
          Some(session) => session.send_input(bytes),
          _ => {}
        }
        put_sessions(ivars, sessions);
    }

    /// Spawn a new tab (a fresh login-shell session) at the shared grid
    /// size, capped at `MAX_TABS`. Only while open.
    pub fn spawn_tab(&self) {
        let ivars = self.ivars();
        if ivars.height.get() <= CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT {
            return
        }
        let mut sessions = take_sessions(ivars);
        if sessions.len() >= MAX_TABS {
            put_sessions(ivars, sessions);
            return
        }
        let cell = ivars.cell.get().expect("a cell metric exists once open");
        let (rows, columns, scrollback) = grid_dims(*cell, header_height(ivars));
        ivars.hovered.set(None); // session indices are about to shift
        sessions.push(Session::new(rows, columns, cell.0, cell.1, scrollback));
        put_sessions(ivars, sessions);
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
            return
        }
        let doomed = sessions.remove(index);
        doomed.destroy();
        let empty = sessions.is_empty();
        let len = sessions.len();
        put_sessions(ivars, sessions);
        ivars.hovered.set(None);
        if empty {
            ivars.selected.set(None);
            self.mark_dirty();
            self.collapse();
        } else {
            // Selection fix-up: closing the SELECTED tab selects the tab
            // that followed it (`min(index, len-1)` after removal); closing
            // one LEFT of the selection shifts the index down so the same
            // tab stays selected; closing one RIGHT leaves it alone.
            match ivars.selected.get() {
              Some(sel) if index == sel => {
                  self.select_tab(if index < len { index } else { len - 1 });
              }
              Some(sel) if index < sel => self.select_tab(sel - 1),
              _ => {}
            }
            self.mark_dirty();
        }
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
            return
        }
        let mut survivors: Vec<Session> = vec![];
        let mut first_removed: Option<usize> = None;
        let mut removed_before_sel = 0;
        let sel_at_entry = ivars.selected.get().unwrap_or(usize::MAX);
        for (index, s) in sessions.into_iter().enumerate() {
            if s.dead() {
                s.destroy();
                if first_removed.is_none() { first_removed = Some(index); }
                if index < sel_at_entry { removed_before_sel += 1; }
            } else {
                survivors.push(s);
            }
        }
        put_sessions(ivars, survivors);
        ivars.hovered.set(None);
        if ivars.tab_count.get() == 0 {
            ivars.selected.set(None);
            self.mark_dirty();
            self.collapse();
        } else {
            // Same remap as `close_tab`: dead tabs LEFT of the selection
            // shift it down by one (the same tab stays selected); a
            // selection at/past a removed tab moves to the tab that
            // followed the first removal, clamped to the end.
            match ivars.selected.get() {
              Some(sel) => {
                  let shifted = sel - removed_before_sel;
                  let target = if shifted >= ivars.tab_count.get() {
                      first_removed.unwrap_or(0).min(ivars.tab_count.get() - 1)
                  } else {
                      shifted
                  };
                  if target != sel { self.select_tab(target); }
              }
              _ => {}
            }
            self.mark_dirty();
        }
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
            return
        }
        let w = ivars.width.get();
        let new = match tab_strip(h, w, ivars.tab_count.get()) {
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
          None => None
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
        let panel = self.as_super().window().expect("a view always lives in a window");
        let panel_frame = panel.frame();
        let x = screen.x - panel_frame.origin.x;
        let y = screen.y - panel_frame.origin.y;
        let ivars = self.ivars();
        let h = ivars.height.get();
        let open = h > CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT;

        // Outside the panel frame entirely: collapse if open, else ignore.
        if x < 0.0 || x > panel_frame.size.width || y < 0.0 || y > panel_frame.size.height {
            if open {
                self.collapse();
            }
            return
        }

        // Inside the frame but off the silhouette (transparent margin): same
        // dismissal.
        let top = PANEL_HEIGHT - h;
        if y < top || y > PANEL_HEIGHT {
            if open {
                self.collapse();
            }
            return
        }

        if !open {
            // Closed pill: a click inside it opens (same as the hotkey).
            self.expand();
            return
        }

        let w = ivars.width.get();
        // `count` mirrors `sessions.len()` (see invariant comment on
        // `tab_count`); the `+` and `×` rects derive from `tab_strip`'s
        // outputs — same math as the renderer in `draw`.
        let count = ivars.tab_count.get();
        match tab_strip(h, w, count) {
          Some((left, tab_w)) => {
            let chin_y = PANEL_HEIGHT - h;
            if y < chin_y || y > chin_y + TAB_HEIGHT {
                return
            }
            // 1) The `+` button — spawn a new session/tab.
            let px = left + count as CGFloat * tab_w;
            if x >= px && x <= px + TAB_PLUS_W {
                self.spawn_tab();
                return
            }
            // 2) A tab: the `×` first (only on the hovered tab), else select.
            for index in 0..count {
                let x0 = left + index as CGFloat * tab_w;
                if x >= x0 && x <= x0 + tab_w {
                    let (cx, cy) = close_center(x0, chin_y);
                    let on_close = (ivars.hovered.get() == Some(index) || ivars.selected.get() == Some(index))
                        && (x - cx).abs() <= CLOSE_HIT && (y - cy).abs() <= CLOSE_HIT;
                    if on_close {
                        self.close_tab(index);
                    } else {
                        self.select_tab(index);
                    }
                    return
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
        ivars.selected.set(Some(index));
        let sessions = take_sessions(ivars);
        match sessions.get(index) {
          Some(session) => { let _ = session.take_activity(); }
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
        let panel = self.as_super().window().expect("a view always lives in a window");
        match crate::window::notch_screen(mtm) {
          Some(screen) => {
            panel.setFrameOrigin(crate::window::panel_origin(screen.frame()));
            let (w, h) = crate::window::notch_size(&screen);
            self.set_notch((w, h));
          }
          _ => {}
        }
    }

    /// Open the panel (idempotent: already-open stays open).
    pub fn expand(&self) {
        self.set_target(OPEN_HEIGHT);
    }

    /// Collapse the panel and reset the scroll position so reopening shows
    /// the live viewport (idempotent: already-closed stays closed).
    pub fn collapse(&self) {
        let ivars = self.ivars();
        ivars.scroll.set(0);
        ivars.scroll_pts.set(0.0);
        ivars.selected.set(None);
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
    pub fn scroll_wheel(&self, delta_y: CGFloat) {
        let ivars = self.ivars();
        if ivars.height.get() <= CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT {
            return
        }
        let cell_height = ivars.cell.get().expect("a cell metric exists once open").1;
        ivars.scroll_pts.set(ivars.scroll_pts.get() - delta_y);
        let lines = (ivars.scroll_pts.get() / cell_height).trunc() as i32;
        if lines == 0 {
            return
        }
        ivars.scroll_pts.set(ivars.scroll_pts.get() - lines as f64 * cell_height);
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


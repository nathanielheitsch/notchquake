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
use objc2_core_text::{CTFont, CTLine, CTLineTruncationType, kCTFontAttributeName};
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
use crate::consts::{BOTTOM_CORNER_RADIUS, CLOSED_HEIGHT, NOTCH_WIDTH, OPEN_HEIGHT, PANEL_HEIGHT, PANEL_WIDTH};
use crate::font::{FONT_SIZE, monospace, cell_metrics};
use crate::render::{draw_grid, GRID_PAD};
use crate::terminal::Session;

/// Tab-bar geometry: an iTerm-style strip hanging from the silhouette's
/// bottom edge — tabs are rounded-top-corner rectangles flush with the chin
/// line, separated by thin dividers, with a `+` button at the right end.
const TAB_COUNT: usize = 3;
const TAB_HEIGHT: CGFloat = 28.0;

/// Point size of the tab labels.
const TAB_LABEL_SIZE: CGFloat = 11.0;
const TAB_INSET: CGFloat = 8.0;
const TAB_RADIUS: CGFloat = 6.0;
const TAB_PLUS_W: CGFloat = 20.0;

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
    sessions: OnceCell<Vec<Session>>,
    selected: Cell<usize>,
    hovered: Cell<Option<usize>>,
    label_font: OnceCell<CFRetained<CTFont>>,
    tracking: OnceCell<Retained<NSTrackingArea>>,
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
            self.select_at_screen_point(&screen);
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

    // Everything below stays inside the silhouette.
    CGContext::clip(ctx);

    // The terminal grid, once the panel is open. Sessions spawn on the
    // first open frame; the grid keeps its FIXED size (computed from the
    // OPEN height), so the reveal is a pure clip wipe — no PTY resize.
    if h > CLOSED_HEIGHT + 1.0 {
        let (font, cell) = ensure_font(ivars);
        let sessions = ivars.sessions.get_or_init(|| spawn_sessions(cell));
        let selected = ivars.selected.get();
        match sessions.get(selected) {
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
            )
          }
          _ => {}
        }
    }

    // The tab strip: iTerm-style tabs hanging from the silhouette's bottom
    // edge (rounded top corners, square bottom flush with the chin line).
    // Every tab is FILLED — selected lighter, unselected dimmed — so no
    // stroke color is ever needed (the old outlined pills painted black on
    // black because only the fill color was set, never the stroke color).
    let chin_y = PANEL_HEIGHT - h;
    match tab_strip(h, w) {
      Some((left, tab_w)) => {
        let selected = ivars.selected.get();
        let hovered = ivars.hovered.get();
        for index in 0..TAB_COUNT {
            let x = left + index as CGFloat * tab_w;
            let gray = if index == selected { 0.18 } else if hovered == Some(index) { 0.14 } else { 0.08 };
            CGContext::set_fill_color_with_color(
                ctx,
                Some(&CGColor::new_generic_gray(gray, 1.0)),
            );
            trace_tab_rect(ctx, x, chin_y, tab_w, TAB_HEIGHT, TAB_RADIUS);
            CGContext::fill_path(ctx);
        }
        // Dividers between tabs.
        CGContext::begin_path(ctx);
        for index in 1..TAB_COUNT {
            let x = left + index as CGFloat * tab_w;
            CGContext::move_to_point(ctx, x, chin_y);
            CGContext::add_line_to_point(ctx, x, chin_y + TAB_HEIGHT - TAB_RADIUS);
        }
        CGContext::set_gray_stroke_color(ctx, 0.35, 1.0);
        CGContext::set_line_width(ctx, 1.0);
        CGContext::stroke_path(ctx);
        // The `+` button at the right end (decorative: sessions are a fixed
        // TAB_COUNT, so it is not a click target).
        let px = left + TAB_COUNT as CGFloat * tab_w;
        CGContext::set_fill_color_with_color(
            ctx,
            Some(&CGColor::new_generic_gray(0.1, 1.0)),
        );
        trace_tab_rect(ctx, px, chin_y, TAB_PLUS_W, TAB_HEIGHT, TAB_RADIUS);
        CGContext::fill_path(ctx);
        let cx = px + TAB_PLUS_W / 2.0;
        let cy = chin_y + TAB_HEIGHT * 0.55;
        CGContext::set_fill_color_with_color(
            ctx,
            Some(&CGColor::new_generic_gray(0.8, 1.0)),
        );
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

        // The tab labels: the OSC title the shell set (`vim`, `git`, …),
        // falling back to the shell basename (`zsh`). Centered in the tab,
        // ellipsis-truncated when wider than the tab.
        let label_font = ivars.label_font.get_or_init(|| monospace(TAB_LABEL_SIZE));
        for index in 0..TAB_COUNT {
            let x = left + index as CGFloat * tab_w;
            let label = ivars.sessions.get().and_then(|s| s.get(index))
                .map(|s| s.title()).unwrap_or_else(|| format!("Tab {}", index + 1));
            draw_tab_label(ctx, Some(label_font), &label, x, chin_y, tab_w);
        }
      }
      _ => {}
    }

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
fn tab_strip(h: CGFloat, w: CGFloat) -> Option<(CGFloat, CGFloat)> {
    if h <= CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT {
        return None
    }
    let inset = chin_radius(h) + TAB_INSET;
    let left = (PANEL_WIDTH - w) / 2.0 + inset;
    let right = (PANEL_WIDTH + w) / 2.0 - inset;
    let span = right - left - TAB_PLUS_W;
    if span <= 0.0 {
        return None
    }
    Some((left, span / TAB_COUNT as CGFloat))
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

/// Draw one tab label centered in the tab `(x, y, w)`, cut off at the tab
/// width when it does not fit. The line is shaped from a
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
) {
    if label.is_empty() || w <= 0.0 {
        return
    }
    let font = font.expect("a label font");
    let text = CFString::from_str(label);
    let line = unsafe { shaped_line(font, &text) };

    let line_width = unsafe { line.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
    let line = if line_width as CGFloat > w {
        match unsafe { line.truncated_line(w as c_double, CTLineTruncationType::End, None) } {
          Some(truncated) => truncated,
          _ => return,
        }
    } else {
        line
    };

    // Measure the line we will actually draw (a truncated line is narrower
    // than the original), so the label centers on its own width.
    let drawn_width = unsafe { line.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) } as CGFloat;

    let ascent = unsafe { font.ascent() };
    let descent = unsafe { font.descent() };
    let baseline = y + (TAB_HEIGHT - (ascent + descent)) / 2.0 + ascent;
    let cx = x + (w - drawn_width) / 2.0;

    CGContext::save_g_state(ctx);
    CGContext::translate_ctm(ctx, cx, baseline);
    CGContext::set_fill_color_with_color(ctx, Some(&CGColor::new_generic_gray(0.85, 1.0)));
    CGContext::set_text_position(ctx, 0.0, 0.0);
    unsafe { line.draw(ctx.expect("a current context")) };
    CGContext::restore_g_state(ctx);
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
        )
    };
    unsafe { CTLine::with_attributed_string(&**attr) }
}

/// The (font, (cell width, cell height)) pair, created once.
fn ensure_font(ivars: &NotchViewIvars) -> (CFRetained<CTFont>, (CGFloat, CGFloat)) {
    let font = ivars.font.get_or_init(|| monospace(FONT_SIZE));
    let cell = ivars.cell.get_or_init(|| cell_metrics(FONT_SIZE));
    (font.clone(), cell.clone())
}

/// Spawn the three login-shell PTY sessions at the fixed grid size.
fn spawn_sessions(cell: (CGFloat, CGFloat)) -> Vec<Session> {
    let (cell_width, cell_height) = cell;
    let usable = PANEL_WIDTH - 2.0 * GRID_PAD;
    let columns = ((usable / cell_width).floor() as usize).max(1);
    let rows = (((OPEN_HEIGHT - TAB_HEIGHT - 2.0 * TAB_INSET) / cell_height).floor() as usize).max(1);
    (0..TAB_COUNT).map(|_| Session::new(
        rows,
        columns,
        cell_width,
        cell_height,
    )).collect()
}

/// Translate an `NSEvent` keydown into PTY bytes.
fn key_bytes(event: &NSEvent) -> Vec<u8> {
    let keyCode = event.keyCode();
    let flags = event.modifierFlags();
    let alt = flags.contains(NSEventModifierFlags::Option);
    let ctrl = flags.contains(NSEventModifierFlags::Control);

    // Control chords the shell understands directly.
    if ctrl && keyCode >= 1 && keyCode <= 26 {
        return vec![keyCode as u8 - 1]
    }
    match keyCode {
        53 => return vec![0x1b],
        13 => return vec![0x0d],
        10 => return vec![0x0d],
        2 => return vec![0x08],
        4 => return vec![0x04],
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
                sessions: OnceCell::new(),
                selected: Cell::new(0),
                hovered: Cell::new(None),
                label_font: OnceCell::new(),
                tracking: OnceCell::new(),
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

    /// Route a keystroke event to the selected session's PTY.
    pub fn handle_key(&self, event: &NSEvent) {
        let sessions = self.ivars().sessions.get();
        if sessions.is_none() {
            return
        }
        let bytes = key_bytes(event);
        if bytes.is_empty() {
            return
        }
        let selected = self.ivars().selected.get();
        match sessions.expect("sessions").get(selected) {
          Some(session) => session.send_input(bytes),
          _ => {}
    }
    }

    /// Handle a click observed by the GLOBAL mouse monitor. A non-activating
    /// panel never owns the click (it passes through to the window below, or
    /// — for synthetic/test clicks — is delivered to the frontmost app), so
    /// the event has no owning window in our app. Use the absolute screen mouse
    /// location and hit-test. (Real clicks ON the panel go to `mouseDown:`
    /// instead; the two paths don't double-fire.)
    pub fn handle_click(&self, _event: &NSEvent) {
        let screen = NSEvent::mouseLocation();
        self.select_at_screen_point(&screen);
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
        let new = match tab_strip(h, w) {
          Some((left, tab_w)) => {
            let chin_y = PANEL_HEIGHT - h;
            if py < chin_y || py > chin_y + TAB_HEIGHT {
                None
            } else {
                (0..TAB_COUNT).find(|&i| {
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

    /// Hit-test an AppKit screen point against the panel silhouette and tab
    /// bar. Shared by the click monitor and the debug self-check.
    pub fn select_at_screen_point(&self, screen: &NSPoint) {
        let panel = self.as_super().window().expect("a view always lives in a window");
        let panel_frame = panel.frame();
        let x = screen.x - panel_frame.origin.x;
        let y_up = screen.y - panel_frame.origin.y;
        if x < 0.0 || x > panel_frame.size.width || y_up < 0.0 || y_up > panel_frame.size.height {
            return
        }
        // The view is NOT flipped, so the drawing space is the window's own
        // y-up space: the panel top (menu bar) is at y = PANEL_HEIGHT.
        let y = y_up;
        let ivars = self.ivars();
        let h = ivars.height.get();
        let top = PANEL_HEIGHT - h;
        if y < top || y > PANEL_HEIGHT {
            return
        }
        if h <= CLOSED_HEIGHT + TAB_INSET + TAB_HEIGHT {
            // Closed panel: a click inside the pill toggles (same as the hotkey).
            self.toggle();
            return
        }
        let w = ivars.width.get();
        match tab_strip(h, w) {
          Some((left, tab_w)) => {
            let chin_y = PANEL_HEIGHT - h;
            if y < chin_y || y > chin_y + TAB_HEIGHT {
                return
            }
            for index in 0..TAB_COUNT {
                let x0i = left + index as CGFloat * tab_w;
                if x >= x0i && x <= x0i + tab_w {
                    ivars.selected.set(index);
                    self.mark_dirty();
                    return
                }
            }
          }
          _ => {}
        }
    }
}

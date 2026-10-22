//! The panel window: a fixed-size, non-activating utility window pinned to the
//! top-centre of the main screen, just above the menu bar.
//!
//! The window is NEVER resized — the silhouette inside it animates instead.
//!
//! `NotchPanel` is an `NSPanel` subclass whose only job is to override
//! `canBecomeKeyWindow -> true`. A `NonactivatingPanel`-styled `NSPanel`
//! defaults `canBecomeKey` to `false`, which makes `makeKey()` a no-op — so
//! the panel never becomes the key window, the local `NSEvent` key monitor
//! never fires, and you cannot type into the terminal. Forcing key-eligibility
//! on (the Spotlight/Maccy/SaneClip pattern) lets a click make the panel key
//! WITHOUT activating the app, so the previous frontmost app keeps focus.
//! `canBecomeMainWindow -> false` prevents us from ever taking over main.

use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2::{ClassType, DefinedClass, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSMainMenuWindowLevel, NSPanel, NSScreen, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{MainThreadMarker, NSEdgeInsets, NSObjectProtocol, NSPoint, NSRect, NSSize};

define_class!(
    // SAFETY:
    // - The superclass `NSPanel` is a normal `NSObject` subclass; we override
    //   only no-arg bool getters, which have no subclassing requirements.
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "NotchPanel"]
    pub struct NotchPanel;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for NotchPanel {}

    impl NotchPanel {
        // SAFETY: `canBecomeKeyWindow` is `@property(readonly) BOOL` — a
        // no-arg `BOOL`-return method. Overriding it to return YES is the
        // documented way to make a non-activating panel key-eligible.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        // SAFETY: `canBecomeMainWindow` is the matching main-window getter;
        // keep it NO so the notch panel never takes over main.
        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }
    }
);

/// The screen's visible frame (excludes the menu-bar strip).
fn visible_frame(screen: &NSScreen) -> NSRect {
    screen.visibleFrame()
}

use crate::consts::{PANEL_HEIGHT, PANEL_WIDTH_FALLBACK};

/// True when the display has a physical notch: the two menu-bar halves
/// leave a real gap in the middle (on a flat display they span the full
/// width, so the computed "notch" is nearly the whole screen).
fn physical_notch(screen: &NSScreen) -> bool {
    let (w, _) = notch_size(screen);
    w < screen.frame().size.width / 2.0
}

/// The display the notch lives on: prefer a display with a physical notch
/// (`safeAreaInsets.top > 0`), else the one with the lowest frame height.
pub fn notch_screen(mtm: MainThreadMarker) -> Option<Retained<NSScreen>> {
    let screens = NSScreen::screens(mtm);
    let notched = screens
        .iter()
        .filter(|screen| physical_notch(screen))
        .reduce(|best, other| {
            if other.frame().size.height < best.frame().size.height {
                other
            } else {
                best
            }
        });
    if notched.is_some() {
        return notched;
    }
    screens.iter().reduce(|best, other| {
        if other.frame().size.height < best.frame().size.height {
            other
        } else {
            best
        }
    })
}

/// The physical notch size in logical points: the gap between the two
/// menu-bar halves, plus the menu-bar height. Matches boring.notch's
/// measurement (`frame.width - auxTopLeft - auxTopRight + 4`).
pub fn notch_size(screen: &NSScreen) -> (CGFloat, CGFloat) {
    let left = screen.auxiliaryTopLeftArea();
    let right = screen.auxiliaryTopRightArea();
    let width = screen.frame().size.width - left.size.width - right.size.width + 4.0;
    let insets = screen.safeAreaInsets();
    let height = if insets.top > 0.0 { insets.top } else { 33.0 };
    (width, height)
}

/// The panel's frame in AppKit (bottom-left origin) screen coordinates, given
/// the main screen's frame. The panel hangs from the top edge of the screen,
/// horizontally centred.
pub fn panel_origin(screen: NSRect, canvas: CGFloat) -> NSPoint {
    NSPoint::new(
        screen.origin.x + screen.size.width / 2.0 - canvas / 2.0,
        screen.origin.y + screen.size.height - PANEL_HEIGHT,
    )
}

/// Creates (but does not show) the panel window, and returns it together with
/// its content view.
pub fn create_panel(mtm: MainThreadMarker) -> (Retained<NSPanel>, Retained<NSView>) {
    let screen = notch_screen(mtm).expect("there is always a screen");
    let canvas = canvas_width(&screen);
    let origin = panel_origin(screen.frame(), canvas);
    let this = NotchPanel::alloc(mtm).set_ivars(());
    // Call the inherited `NSPanel` designated initialiser via the superclass
    // (`msg_send![super(this), ...]`). `set_ivars(())` advances
    // `Allocated<NotchPanel>` to the `PartialInit<NotchPanel>` the super-init
    // family requires (NotchPanel has no ivars). The inherent
    // `NSPanel::initWithContentRect:...` can't take `Allocated<NotchPanel>`,
    // so we dispatch through super. Returns `Retained<NotchPanel>`; upcast to
    // `Retained<NSPanel>` for the caller.
    let panel: Retained<NotchPanel> = unsafe {
        msg_send![super(this), initWithContentRect: NSRect::new(origin, NSSize::new(canvas, PANEL_HEIGHT)) styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel | NSWindowStyleMask::UtilityWindow | NSWindowStyleMask::HUDWindow backing: NSBackingStoreType::Buffered defer: false]
    };
    let panel: Retained<NSPanel> = panel.into_super();

    unsafe {
        panel.setFloatingPanel(true);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&*NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setMovable(false);
        panel.setReleasedWhenClosed(false);
        panel.setBecomesKeyOnlyIfNeeded(false);
        panel.setWorksWhenModal(true);
        panel.setAlphaValue(1.0);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        // Above the menu bar, so the silhouette can overlap it.
        panel.setLevel(NSMainMenuWindowLevel + 3);
    }

    panel.orderFrontRegardless();

    // Re-assert the canvas size before the content view is taken: `NSView::setFrame`
    // is parent-relative, so an explicit size avoids inheriting a content rect
    // AppKit derived against the notch inset. (`NSWindow` has no `setFrameSize:` —
    // that is an `NSView` method — so the full-frame setter `setFrame:display:` is
    // the API-faithful equivalent of the plan's re-assert.)
    panel.setFrame_display(NSRect::new(origin, NSSize::new(canvas, PANEL_HEIGHT)), true);

    let content_view = panel
        .contentView()
        .expect("a panel always has a content view");
    (panel, content_view)
}

/// The canvas (window) width for `screen`: the display's logical width.
pub fn canvas_width(screen: &NSScreen) -> CGFloat {
    screen.frame().size.width
}

/// The canvas width to size the panel window to: the notch display's logical
/// width, falling back to `PANEL_WIDTH_FALLBACK` when no screen resolves
/// (mirrors `notch_screen_origin`'s `expect`).
pub fn canvas_size(mtm: MainThreadMarker) -> CGFloat {
    match notch_screen(mtm) {
        Some(screen) => canvas_width(&screen),
        _ => PANEL_WIDTH_FALLBACK,
    }
}

/// The window's frame origin for the notch display (see `panel_origin`).
fn notch_screen_origin(mtm: MainThreadMarker) -> NSPoint {
    let screen = notch_screen(mtm).expect("there is always a screen");
    panel_origin(screen.frame(), canvas_width(&screen))
}

//! The panel window: a fixed-size, non-activating utility window pinned to the
//! top-centre of the main screen, just above the menu bar.
//!
//! The window is NEVER resized — the silhouette inside it animates instead.

use objc2::rc::Retained;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSMainMenuWindowLevel, NSPanel, NSScreen, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{MainThreadMarker, NSEdgeInsets, NSPoint, NSRect, NSSize};

/// The screen's visible frame (excludes the menu-bar strip).
fn visible_frame(screen: &NSScreen) -> NSRect {
    screen.visibleFrame()
}

use crate::consts::{PANEL_HEIGHT, PANEL_WIDTH};

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
    let notched = screens.iter().filter(|screen| physical_notch(screen))
        .reduce(|best, other| if other.frame().size.height < best.frame().size.height { other } else { best });
    if notched.is_some() {
        return notched
    }
    screens.iter().reduce(|best, other|
        if other.frame().size.height < best.frame().size.height { other } else { best }
    )
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
pub fn panel_origin(screen: NSRect) -> NSPoint {
    NSPoint::new(
        screen.origin.x + screen.size.width / 2.0 - PANEL_WIDTH / 2.0,
        screen.origin.y + screen.size.height - PANEL_HEIGHT,
    )
}

/// Creates (but does not show) the panel window, and returns it together with
/// its content view.
pub fn create_panel(mtm: MainThreadMarker) -> (Retained<NSPanel>, Retained<NSView>) {
    let origin = notch_screen_origin(mtm);
    let panel = unsafe {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            NSRect::new(
                origin,
                NSSize::new(PANEL_WIDTH, PANEL_HEIGHT),
            ),
            NSWindowStyleMask::Borderless
                | NSWindowStyleMask::NonactivatingPanel
                | NSWindowStyleMask::UtilityWindow
                | NSWindowStyleMask::HUDWindow,
            NSBackingStoreType::Buffered,
            false,
        )
    };

    unsafe {
        panel.setFloatingPanel(true);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&*NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setMovable(false);
        panel.setReleasedWhenClosed(false);
        panel.setBecomesKeyOnlyIfNeeded(true);
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

    let content_view = panel.contentView().expect("a panel always has a content view");
    (panel, content_view)
}

/// The window's frame origin for the notch display (see `panel_origin`).
fn notch_screen_origin(mtm: MainThreadMarker) -> NSPoint {
    let screen = notch_screen(mtm).expect("there is always a screen");
    panel_origin(screen.frame())
}


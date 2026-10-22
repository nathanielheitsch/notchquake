//! The settings surface: a SEPARATE, normal titled `NSWindow` (not an
//! `NSPanel`), owned by one retained `SettingsOwner` object.
//!
//! This replaces the hand-drawn in-panel settings overlay entirely. The panel
//! is a non-activating layer-27 `NSPanel` that can never host a real control
//! surface; a normal layer-0 window can, so every knob becomes a native AppKit
//! control and the panel only keeps the gear affordance that opens this window.
//!
//! Ownership discipline (the lesson-16 analogue): AppKit does not retain the
//! window tree for us. The one `Retained<SettingsOwner>` lives in a
//! `NotchViewIvars` `OnceCell` (the notch view is itself retained for the whole
//! process lifetime by `AppDelegate.view`), and the owner's ivars retain the
//! window, the controller that owns it, the grid that lays the controls out,
//! and — critically — every `SettingsCell` TARGET. The target cells are NOT in
//! the view tree, so nothing else keeps them alive: drop one and `setTarget:`'s
//! reference lets it die and that control's clicks silently no-op forever.
//!
//! Click-operability, verified live: a real pixel click on a control flips it
//! (an `NSSwitch` went off → on). Background/synthetic input into the window IS
//! refused (`minimized_or_hidden_window`) because the app is `Accessory`
//! policy, but that only rules out TYPING into the window — the window itself
//! can be brought to front and activated. So the UI is built from click-driven
//! native controls only: sliders, popup menus, switches, segmented controls,
//! color wells, swatch boxes. No editable `NSTextField` for values, no typed
//! chord entry; `NSTextField` appears only as static labels.
//!
//! Unlike the notch panel (a non-activating `NSPanel` at layer 27), this is a
//! normal layer-0 window: window tools can see it, and it can take key focus.

use core::ffi::c_double;
use objc2::ClassType;
use objc2::DefinedClass;
use objc2::MainThreadOnly;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::runtime::{AnyObject, NSObject};
use objc2::sel;
use objc2_app_kit::{
    NSBackingStoreType, NSBox, NSBoxType, NSColor, NSColorSpace, NSColorWell, NSColorWellStyle,
    NSControlStateValueOff, NSControlStateValueOn, NSGridView, NSMenu, NSMenuItem, NSPopUpButton,
    NSSegmentSwitchTracking, NSSegmentedControl, NSSlider, NSSwitch, NSTabView, NSTabViewItem,
    NSTabViewType, NSTextField, NSTitlePosition, NSView, NSWindow, NSWindowController,
    NSWindowStyleMask,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{
    MainThreadMarker, NSArray, NSInteger, NSMutableArray, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString,
};

use std::cell::OnceCell;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use crate::consts::{CLOSED_HEIGHT, NOTCH_WIDTH, PANEL_HEIGHT, PANEL_WIDTH};
use crate::notch_view::NotchView;
use crate::settings::{
    CHORD_PRESETS, Color, FONT_FAMILIES, PALETTE_PRESETS, SCROLLBACK_PRESETS, Settings,
    chord_index, chord_name, cursor_shape_index, cursor_shape_of, family_index, parse_hex,
    scrollback_index, scrollback_of,
};
use crate::window::{notch_screen, panel_origin};

// ── Window geometry ───────────────────────────────────────────────────────
/// The window is sized to its TALLEST tab, not to the whole control surface:
/// the knobs live in an `NSTabView` with one `NSGridView` per section, and the
/// tallest tab (Colors: 3 color wells + 2 popup pickers + the 16-swatch palette
/// row = 6 rows) plus the tab row and bezel is what must fit. A FIXED frame is
/// deliberate: a resize on tab switch would move the window frame, which the
/// collapse guard in `notch_view.rs` hit-tests.
const WINDOW_W: CGFloat = 560.0;
const WINDOW_H: CGFloat = 288.0;
/// Placement of the `NSTabView` inside the window's content view. The titlebar
/// sits ABOVE the content view (it is not part of it), so the content view is
/// exactly the content rect in the window's own y-up space: the tab view's
/// bottom-left is `(inset, inset)`.
const GRID_INSET_X: CGFloat = 20.0;
const GRID_INSET_Y: CGFloat = 16.0;
/// Margin between the tab view's content rect and the section grid inside it.
/// `TopTabsBezelBorder` draws a bezel around the content area; the grid is its
/// child, so this inset keeps the first row of controls off that border.
const GRID_BEZEL_MARGIN: CGFloat = 4.0;

/// Slider value ranges. Height is clamped to the panel canvas (`PANEL_HEIGHT`);
/// width is clamped to the RUNTIME notch display canvas — the notch window is
/// sized to the display's logical width at startup (see `window.rs::canvas_width`
/// and `notch_view.rs::reposition`), so the width slider's rail must physically
/// reach the screen width on a wider display, not stop at the 640pt default.
/// Width therefore comes from `width_max()` (which reads the same `NSScreen`
/// helper as `Settings::load`), so the slider rail and the `apply_knob_value`
/// clamp cannot drift. Height stays compile-time because `PANEL_HEIGHT` never
/// changes. Kept in sync with `settings.rs`.
const HEIGHT_MIN: CGFloat = CLOSED_HEIGHT + 80.0;
const HEIGHT_MAX: CGFloat = PANEL_HEIGHT - 20.0;
const WIDTH_MIN: CGFloat = NOTCH_WIDTH;
const FONT_MIN: CGFloat = 8.0;
const FONT_MAX: CGFloat = 32.0;
/// The runtime upper bound for the open-width knob: the live notch canvas
/// (the notch display's logical width), floored at `WIDTH_MIN` so a degenerate
/// screen lookup cannot invert the range. Consumed by BOTH the slider rail
/// (`grid_for_section`) and the `apply_knob_value` clamp, and it is the same
/// `NSScreen` read `Settings::load` clamps against, so the three sites —
/// rail, clamp and settings load — cannot disagree.
pub fn width_max() -> CGFloat {
    WIDTH_MIN.max(crate::settings::display_width())
}

const SECS_MIN: CGFloat = 0.0;
const SECS_MAX: CGFloat = 3600.0;

/// Swatch metrics for the ANSI-16 display row.
const SWATCH: CGFloat = 16.0;
/// Gap between adjacent ANSI swatches in the palette row.
const SWATCH_GAP: CGFloat = 4.0;
/// The ANSI picker's two-step menu sizes: 16 slots, 12 preset colors.
const ANSI_SLOTS: usize = 16;
const ANSI_COLORS: usize = PALETTE_PRESETS.len();

/// The three cursor shapes a segmented control can select, in the same order
/// `settings.rs::cursor_shape_of` decodes.
const CURSOR_SHAPES: [&'static str; 3] = ["Block", "Bar", "Underline"];

/// The five sections of the settings pane, each one `NSTabViewItem` of the
/// `NSTabView` with its own `NSGridView`. Every knob is placed by exactly one
/// section (see `grid_for_section`); the order here is the tab order.
const SECTIONS: [Section; 5] = [
    Section::General,
    Section::Appearance,
    Section::Colors,
    Section::Keys,
    Section::About,
];

#[derive(Clone, Copy)]
enum Section {
    General,
    Appearance,
    Colors,
    Keys,
    About,
}

impl Section {
    /// The tab label.
    pub fn title(&self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Colors => "Colors",
            Section::Keys => "Keys",
            Section::About => "About",
        }
    }
}

// ── Knob identity ─────────────────────────────────────────────────────────
/// The knob a `SettingsCell` mutates. One index per knob, consumed by
/// [`apply_knob_value`] and [`knob_value_from_sender`] (which uses the same
/// index space to decide which control class sent the action). The ANSI-slot
/// cell changes no knob of its own: it records which of the 16 slots the next
/// color click edits.
const KNOB_OPEN_HEIGHT: usize = 0;
const KNOB_OPEN_WIDTH: usize = 1;
const KNOB_FONT_FAMILY: usize = 2;
const KNOB_FONT_SIZE: usize = 3;
const KNOB_LIGATURES: usize = 4;
const KNOB_BACKGROUND: usize = 5;
const KNOB_FOREGROUND: usize = 6;
const KNOB_CURSOR_COLOR: usize = 7;
const KNOB_CURSOR_SHAPE: usize = 8;
const KNOB_SCROLLBACK: usize = 9;
const KNOB_TOGGLE_CHORD: usize = 10;
const KNOB_NEW_TAB_CHORD: usize = 11;
const KNOB_BELL: usize = 12;
const KNOB_NOTIFY_SECS: usize = 13;
const KNOB_ANSI_SLOT: usize = 14;
const KNOB_ANSI_COLOR: usize = 15;

/// The ANSI slot the two-step picker currently points at. The slot popup
/// writes it, the color popup reads it, so the two clicks compose into one
/// `(slot, color)` edit without either control holding a reference to the
/// other (a control retains its `target`, so a back-pointer would be a
/// reference cycle that never frees).
///
/// A `static` rather than an ivar for the same reason and with the same
/// discipline as `hotkey.rs::HOT_REFS`: it is process-wide state touched only
/// from AppKit control dispatch on the main run loop.
static ANSI_SLOT: AtomicUsize = AtomicUsize::new(0);

// ── Knob values ───────────────────────────────────────────────────────────
/// What a control currently holds: a continuous number (a slider), a preset
/// index (a popup menu, a switch, a segmented control, an ANSI slot), or a
/// color (a color well, an ANSI color popup). Unused fields stay zero.
pub struct KnobValue {
    number: CGFloat,
    index: usize,
    color: Color,
}

/// The zero color. `Color`'s fields are private to the `settings` module, so
/// the only portable way to name it is the hex parser.
fn black() -> Color {
    parse_hex("#000000")
}

impl KnobValue {
    pub fn number(value: CGFloat) -> KnobValue {
        KnobValue {
            number: value,
            index: 0,
            color: black(),
        }
    }

    pub fn index(index: usize) -> KnobValue {
        KnobValue {
            number: 0.0,
            index,
            color: black(),
        }
    }

    pub fn color(color: Color) -> KnobValue {
        KnobValue {
            number: 0.0,
            index: 0,
            color,
        }
    }
}

/// The sRGB triple of an `NSColor`, re-expressed in sRGB so a color the user
/// picked in another colorspace (a well's picker may hand back Display P3)
/// still stores the same visible color. A color that cannot be re-expressed
/// falls back to its own components. Quantized to 8-bit channels, which is
/// what `Settings` stores and `render.rs` consumes anyway.
fn color_of(color: &NSColor) -> Color {
    let srgb = NSColorSpace::sRGBColorSpace();
    let converted = color.colorUsingColorSpace(&*srgb);
    let (r, g, b) = match converted {
        Some(c) => (c.redComponent(), c.greenComponent(), c.blueComponent()),
        None => (
            color.redComponent(),
            color.greenComponent(),
            color.blueComponent(),
        ),
    };
    let hex = format!(
        "#{:02X}{:02X}{:02X}",
        (r.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u32,
        (g.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u32,
        (b.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u32,
    );
    parse_hex(&hex)
}

/// An `NSColor` for a settings `Color`, the form a swatch box or a color well
/// is painted with.
pub fn srgb_color(color: Color) -> Retained<NSColor> {
    let (r, g, b, a) = color.rgba();
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}

/// Apply one knob change to a settings bundle. `value` is what the control
/// that sent this action currently holds.
pub fn apply_knob_value(settings: Settings, knob: usize, value: KnobValue) -> Settings {
    let mut s = settings;
    match knob {
        KNOB_OPEN_HEIGHT => s.open_height = value.number.clamp(HEIGHT_MIN, HEIGHT_MAX),
        KNOB_OPEN_WIDTH => s.open_width = value.number.clamp(WIDTH_MIN, width_max()),
        KNOB_FONT_FAMILY => s.font_family = FONT_FAMILIES[value.index.min(FONT_FAMILIES.len() - 1)],
        KNOB_FONT_SIZE => s.font_size = value.number.clamp(FONT_MIN, FONT_MAX),
        KNOB_LIGATURES => s.ligatures = value.index != 0,
        KNOB_BACKGROUND => s.background = value.color,
        KNOB_FOREGROUND => s.foreground = value.color,
        KNOB_CURSOR_COLOR => s.cursor_color = value.color,
        KNOB_CURSOR_SHAPE => s.cursor_shape = cursor_shape_of(value.index),
        KNOB_SCROLLBACK => s.scrollback_lines = scrollback_of(value.index),
        KNOB_TOGGLE_CHORD => {
            s.toggle_chord = CHORD_PRESETS[value.index.min(CHORD_PRESETS.len() - 1)]
        }
        KNOB_NEW_TAB_CHORD => {
            s.new_tab_chord = CHORD_PRESETS[value.index.min(CHORD_PRESETS.len() - 1)]
        }
        KNOB_BELL => s.bell_enabled = value.index != 0,
        KNOB_NOTIFY_SECS => s.finished_notify_secs = value.number.clamp(SECS_MIN, SECS_MAX),
        KNOB_ANSI_COLOR => {
            let slot = ANSI_SLOT.load(Ordering::SeqCst).min(ANSI_SLOTS - 1);
            s.ansi[slot] = PALETTE_PRESETS[value.index.min(ANSI_COLORS - 1)];
        }
        _ => {}
    }
    s
}

// ── The target cell ───────────────────────────────────────────────────────
#[derive(Default)]
pub struct SettingsCellIvars {
    knob: usize,
    /// The control that sends this cell its action. Raw because a control
    /// retains its `target`: a `Retained` back-pointer would be a cycle that
    /// never frees. The grid tree retains every control for the whole process
    /// lifetime (the owner retains the grid), so the pointer cannot dangle,
    /// and every read here is AppKit control dispatch on the main run loop —
    /// the same discipline as the stored `NotchView` pointer below and the
    /// Carbon handler's `userData`.
    control: OnceCell<*const AnyObject>,
    /// The notch view whose `Cell<Settings>` this cell mutates. Raw for the
    /// same reason and with the same safety argument as the Carbon handler's
    /// `userData`: `AppDelegate.view` retains the view for the whole process
    /// lifetime, and every handler that reads this is AppKit control dispatch
    /// on the main run loop.
    view: OnceCell<*const NotchView>,
}

define_class!(
    // SAFETY:
    // - The superclass `NSObject` does not have any subclassing requirements.
    // - `SettingsCellIvars` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SettingsCell"]
    #[ivars = SettingsCellIvars]
    pub struct SettingsCell;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for SettingsCell {}

    impl SettingsCell {
        // SAFETY: `applyKnob` is our own zero-argument selector, sent only by the controls
        // we create below; the signature takes no argument.
        #[unsafe(method(applyKnob))]
        fn apply_knob(&self) {
            let ivars = self.ivars();
            let view: &NotchView = unsafe { &**(ivars.view.get().expect("a cell is built with its view")) };
            let control: *const AnyObject = *ivars.control.get().expect("a cell knows its control");
            if ivars.knob == KNOB_ANSI_SLOT {
                // The slot popup changes no knob: it records which ANSI row
                // the next color click edits.
                let popup: &NSPopUpButton = unsafe { &*(control as *const NSPopUpButton) };
                ANSI_SLOT.store((popup.indexOfSelectedItem().max(0) as usize).min(ANSI_SLOTS - 1), Ordering::SeqCst);
                return
            }
            let value = knob_value_from_sender(ivars.knob, control);
            NotchView::apply_settings(view, apply_knob_value(view.current_settings(), ivars.knob, value));
        }
    }
);

impl SettingsCell {
    /// A cell for one knob, bound to the notch view whose `Cell<Settings>` it
    /// mutates.
    pub fn new(mtm: MainThreadMarker, knob: usize, view: &NotchView) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SettingsCellIvars {
            knob,
            control: OnceCell::new(),
            view: OnceCell::new(),
        });
        // SAFETY: `NSObject`'s `init` signature is correct.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        // SAFETY: see the `view` field's comment — the view is retained for
        // the process lifetime, and this runs on the main thread.
        let _ = this.ivars().view.set(view as *const NotchView);
        this
    }
}

/// Read the value a control currently holds, by the class that sent the
/// action. The cast is sound because the cell's `knob` fixes which control
/// class was built for it, and every control is created in [`grid_for_section`]
/// with the matching knob.
fn knob_value_from_sender(knob: usize, control: *const AnyObject) -> KnobValue {
    match knob {
        KNOB_OPEN_HEIGHT | KNOB_OPEN_WIDTH | KNOB_FONT_SIZE | KNOB_NOTIFY_SECS => {
            let slider: &NSSlider = unsafe { &*(control as *const NSSlider) };
            KnobValue::number(slider.doubleValue() as CGFloat)
        }
        KNOB_FONT_FAMILY | KNOB_SCROLLBACK | KNOB_TOGGLE_CHORD | KNOB_NEW_TAB_CHORD
        | KNOB_ANSI_COLOR => {
            let popup: &NSPopUpButton = unsafe { &*(control as *const NSPopUpButton) };
            KnobValue::index(popup.indexOfSelectedItem().max(0) as usize)
        }
        KNOB_CURSOR_SHAPE => {
            let segmented: &NSSegmentedControl =
                unsafe { &*(control as *const NSSegmentedControl) };
            KnobValue::index(segmented.selectedSegment().max(0) as usize)
        }
        KNOB_LIGATURES | KNOB_BELL => {
            let sw: &NSSwitch = unsafe { &*(control as *const NSSwitch) };
            KnobValue::index(if sw.state() == NSControlStateValueOn {
                1
            } else {
                0
            })
        }
        KNOB_BACKGROUND | KNOB_FOREGROUND | KNOB_CURSOR_COLOR => {
            let well: &NSColorWell = unsafe { &*(control as *const NSColorWell) };
            KnobValue::color(color_of(&*well.color()))
        }
        _ => KnobValue::index(0),
    }
}

// ── The owner ─────────────────────────────────────────────────────────────
#[derive(Default)]
pub struct SettingsOwnerIvars {
    window: OnceCell<Retained<NSWindow>>,
    controller: OnceCell<Retained<NSWindowController>>,
    tab_view: OnceCell<Retained<NSTabView>>,
    /// One grid per tab item. The tab view retains its items, which retain
    /// their views, so the grids are already alive; keeping them here too is
    /// the same belt-and-braces discipline as `cells`.
    grids: OnceCell<Vec<Retained<NSGridView>>>,
    /// Every `SettingsCell` TARGET. These are NOT in the view tree, so this
    /// `Vec` is the only thing keeping them alive — without it a control's
    /// clicks silently no-op (the lesson-16 analogue: a dropped object means
    /// a silently dead click path). The controls themselves ARE in the grid
    /// tree and need no extra retention.
    cells: OnceCell<Vec<Retained<SettingsCell>>>,
}

define_class!(
    // SAFETY:
    // - The superclass `NSObject` does not have any subclassing requirements.
    // - `SettingsOwnerIvars` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SettingsOwner"]
    #[ivars = SettingsOwnerIvars]
    pub struct SettingsOwner;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for SettingsOwner {}
);

impl SettingsOwner {
    /// A fresh, empty owner. The caller fills its ivars via
    /// [`build_and_show`], then keeps the returned retention alive.
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SettingsOwnerIvars::default());
        // SAFETY: `NSObject`'s `init` signature is correct.
        unsafe { msg_send![super(this), init] }
    }

    /// Show the window if it is hidden, then claim key-window focus.
    ///
    /// A window the user closed is re-shown through the retained controller
    /// (`showWindow:` is bound in objc2-app-kit 0.3.2, so no `msg_send!`
    /// fallback is needed for `show`).
    pub fn show_or_focus(&self) {
        let ivars = self.ivars();
        let window = ivars
            .window
            .get()
            .expect("an owner always retains its window");
        if !window.isVisible() {
            let controller = ivars
                .controller
                .get()
                .expect("an owner always retains its controller");
            unsafe { controller.showWindow(None) };
        }
        window.makeKeyWindow();
    }

    /// The window's frame in AppKit screen space, so the notch view can tell
    /// whether a click observed by its global monitor landed inside this
    /// window. Without that guard a click on a control hit-tests as "outside
    /// the panel" and COLLAPSES an open notch.
    pub fn window_frame(&self) -> Option<NSRect> {
        match self.ivars().window.get() {
            None => None,
            Some(window) => Some(window.frame()),
        }
    }

    /// Build the window tree (window → controller → grid → controls), fill
    /// every ivar so the owner retains the whole tree, then show it.
    pub fn build_and_show(&self, mtm: MainThreadMarker, view: &NotchView) {
        let ivars = self.ivars();

        // Centre horizontally exactly like `window.rs::panel_origin` centres
        // the notch panel, and hang the window just BELOW the panel's canvas
        // so it never covers the notch (the panel is layer 27 and paints
        // above it anyway).
        let screen = notch_screen(mtm).expect("there is always a screen");
        let frame = screen.frame();
        // The window hangs BELOW the panel canvas: its top edge sits at the
        // panel's bottom (`panel_origin` is that bottom edge in AppKit y-up
        // space), so it never covers the notch. Clamped to the screen's bottom
        // edge so a short display cannot push it off-screen.
        let panel_bottom = frame.origin.y + frame.size.height - PANEL_HEIGHT;
        let content_origin = NSPoint::new(
            panel_origin(frame, frame.size.width).x + (frame.size.width - WINDOW_W) / 2.0,
            (panel_bottom - WINDOW_H).max(frame.origin.y),
        );

        // A NORMAL titled window: no `NonactivatingPanel` / `UtilityWindow` /
        // `Borderless` bit — those are exactly what keep the notch panel from
        // ever taking focus, and this window must not inherit them.
        let window: Retained<NSWindow> = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(content_origin, NSSize::new(WINDOW_W, WINDOW_H)),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Resizable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };

        unsafe {
            window.setTitle(&*NSString::from_str("NotchQuake Settings"));
            // A user close must not free the window while the owner still
            // points at it (`setReleasedWhenClosed:` is `unsafe`).
            window.setReleasedWhenClosed(false);
            window.setMovableByWindowBackground(true);
            // Do not pollute the Window menu with our settings surface.
            window.setExcludedFromWindowsMenu(true);
        }

        // `NSWindowController` is the documented owner for close/show
        // plumbing; it is retained by the ivars below.
        let controller: Retained<NSWindowController> =
            NSWindowController::initWithWindow(NSWindowController::alloc(mtm), Some(&*window));

        let mut cells: Vec<Retained<SettingsCell>> = vec![];

        // One `NSGridView` per section, each wrapped as an `NSTabViewItem`
        // (label + view) and added to one plain `NSTabView` with top tabs —
        // the iTerm prefs-pane chrome. A plain `NSTabView`, not an
        // `NSTabViewController`: we need no per-tab view controller and no
        // toolbar/segmented style, only a label and an arbitrary view per tab.
        let tab_view: Retained<NSTabView> = NSTabView::new(mtm);
        tab_view.setTabViewType(NSTabViewType::TopTabsBezelBorder);
        let grids: Vec<Retained<NSGridView>> = SECTIONS
            .map(|section| {
                let grid =
                    self.grid_for_section(mtm, view, view.current_settings(), section, &mut cells);
                let item: Retained<NSTabViewItem> = NSTabViewItem::new();
                item.setLabel(&*NSString::from_str(section.title()));
                item.setView(Some(grid.as_super()));
                tab_view.addTabViewItem(&*item);
                grid
            })
            .to_vec();
        tab_view.selectTabViewItemAtIndex(0);

        // The tab view fills the content view inset by the margins. The
        // content view is the content rect (the titlebar sits ABOVE it and is
        // not part of it), so its own size — not the window's — is what the
        // tab view must fit inside.
        let content_view = window
            .contentView()
            .expect("a titled window always has a content view");
        let content = content_view.frame();
        tab_view.as_super().setFrame(NSRect::new(
            NSPoint::new(GRID_INSET_X, GRID_INSET_Y),
            NSSize::new(
                content.size.width - GRID_INSET_X * 2.0,
                content.size.height - GRID_INSET_Y * 2.0,
            ),
        ));
        content_view.addSubview(tab_view.as_super());

        // Size every section grid to its NATURAL height, anchored to the TOP
        // of the tab view's content rect. The grid IS the `NSTabViewItem`'s
        // view (`setView` above), but `NSTabView` ONLY lays out the SELECTED
        // item's view and NEVER resizes it: it leaves every grid at whatever
        // frame it was built with, and a grid at its own natural content size
        // is taller than the content area, so its first row painted straight
        // OVER the tab labels (measured: the General grid's "Open height" row
        // overlapped the tab strip). `contentRect()` is the area under the
        // tabs and inside the bezel, in the tab view's own coordinates —
        // exactly the region a section grid belongs in. Stretching the grid to
        // the FULL content height instead made `NSGridView` push its rows
        // apart to fill the spare space — a visible void between rows 0 and 1.
        // So size it to `fittingSize` (its own minimum natural size: rows at
        // their control heights joined by the default spacing) and anchor that
        // to the top, leaving any leftover height as empty space BELOW the
        // last row.
        let content = tab_view.contentRect();
        for item in tab_view.tabViewItems().iter() {
            match item.view(mtm) {
                None => {}
                Some(view_ret) => {
                    let view: &NSView = &*view_ret;
                    let fit = view.fittingSize();
                    // The item view lives in a FLIPPED container (y grows
                    // downward), so the TOP of the content rect is the SMALL y:
                    // anchoring the grid there leaves the slack BELOW the last
                    // row, iTerm-style. (Measured: the opposite formula parked
                    // General's rows at the bottom with a 65pt void above row 0.)
                    view.setFrame(NSRect::new(
                        NSPoint::new(
                            content.origin.x + GRID_BEZEL_MARGIN,
                            content.origin.y + GRID_BEZEL_MARGIN,
                        ),
                        NSSize::new(
                            (content.size.width - GRID_BEZEL_MARGIN * 2.0).max(1.0),
                            fit.height,
                        ),
                    ));
                    view.setNeedsLayout(true);
                    view.layout();
                }
            }
        }

        let _ = ivars.window.set(window);
        let _ = ivars.controller.set(controller);
        let _ = ivars.tab_view.set(tab_view);
        let _ = ivars.grids.set(grids);
        let _ = ivars.cells.set(cells);

        self.show_or_focus();
    }

    /// The rows of ONE section, as its own `NSGridView`: a static label plus
    /// one click-driven control per knob, the ANSI-16 swatch row, or the About
    /// labels. Built from the live settings bundle so the initial slider
    /// positions, popup selections, well colors and swatch fills match what
    /// the panel is already using. Every target cell is pushed into `cells`,
    /// which the owner keeps alive — the factories are unchanged, so every
    /// `SettingsCell::new` + `keep()` + `cells.push` still runs.
    fn grid_for_section(
        &self,
        mtm: MainThreadMarker,
        view: &NotchView,
        settings: Settings,
        section: Section,
        cells: &mut Vec<Retained<SettingsCell>>,
    ) -> Retained<NSGridView> {
        let rows: Retained<NSMutableArray<NSArray<NSView>>> = NSMutableArray::new();

        match section {
            Section::General => {
                push_row(
                    &rows,
                    &[
                        label(mtm, "Open height"),
                        slider(
                            mtm,
                            view,
                            cells,
                            KNOB_OPEN_HEIGHT,
                            settings.open_height,
                            HEIGHT_MIN,
                            HEIGHT_MAX,
                        ),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "Open width"),
                        slider(
                            mtm,
                            view,
                            cells,
                            KNOB_OPEN_WIDTH,
                            settings.open_width,
                            WIDTH_MIN,
                            width_max(),
                        ),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "Scrollback lines"),
                        popup(
                            mtm,
                            view,
                            cells,
                            KNOB_SCROLLBACK,
                            scrollback_titles(),
                            scrollback_index(settings.scrollback_lines),
                        ),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "Bell enabled"),
                        switch(mtm, view, cells, KNOB_BELL, settings.bell_enabled),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "Notify after (s)"),
                        slider(
                            mtm,
                            view,
                            cells,
                            KNOB_NOTIFY_SECS,
                            settings.finished_notify_secs,
                            SECS_MIN,
                            SECS_MAX,
                        ),
                    ],
                );
            }

            Section::Appearance => {
                push_row(
                    &rows,
                    &[
                        label(mtm, "Font family"),
                        popup(
                            mtm,
                            view,
                            cells,
                            KNOB_FONT_FAMILY,
                            family_titles(),
                            family_index(settings.font_family),
                        ),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "Font size"),
                        slider(
                            mtm,
                            view,
                            cells,
                            KNOB_FONT_SIZE,
                            settings.font_size,
                            FONT_MIN,
                            FONT_MAX,
                        ),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "Ligatures"),
                        switch(mtm, view, cells, KNOB_LIGATURES, settings.ligatures),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "Cursor shape"),
                        segmented(
                            mtm,
                            view,
                            cells,
                            KNOB_CURSOR_SHAPE,
                            &CURSOR_SHAPES,
                            cursor_shape_index(settings.cursor_shape),
                        ),
                    ],
                );
            }

            Section::Colors => {
                push_row(
                    &rows,
                    &[
                        label(mtm, "Background"),
                        well(mtm, view, cells, KNOB_BACKGROUND, settings.background),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "Foreground"),
                        well(mtm, view, cells, KNOB_FOREGROUND, settings.foreground),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "Cursor color"),
                        well(mtm, view, cells, KNOB_CURSOR_COLOR, settings.cursor_color),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "ANSI slot"),
                        popup(
                            mtm,
                            view,
                            cells,
                            KNOB_ANSI_SLOT,
                            ansi_slot_titles(),
                            ANSI_SLOT.load(Ordering::SeqCst),
                        ),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "ANSI color"),
                        popup(
                            mtm,
                            view,
                            cells,
                            KNOB_ANSI_COLOR,
                            palette_titles(),
                            ansi_color_index(settings),
                        ),
                    ],
                );
                push_row(&rows, &swatch_row(mtm, settings));
            }

            Section::Keys => {
                push_row(
                    &rows,
                    &[
                        label(mtm, "Toggle chord"),
                        popup(
                            mtm,
                            view,
                            cells,
                            KNOB_TOGGLE_CHORD,
                            chord_titles(),
                            chord_index(settings.toggle_chord),
                        ),
                    ],
                );
                push_row(
                    &rows,
                    &[
                        label(mtm, "New tab chord"),
                        popup(
                            mtm,
                            view,
                            cells,
                            KNOB_NEW_TAB_CHORD,
                            chord_titles(),
                            chord_index(settings.new_tab_chord),
                        ),
                    ],
                );
            }

            Section::About => {
                for text in about_rows(settings) {
                    push_row(&rows, &[label(mtm, &text)]);
                }
            }
        }

        NSGridView::gridViewWithViews(&rows.into_super(), mtm)
    }
}

// ── Control factories ─────────────────────────────────────────────────────
/// One grid row. `NSGridView` takes an array of rows, each an array of the
/// views in that row; every view is upcast to `NSView` (every control here is
/// an `NSControl`/`NSBox` subclass of `NSView`) and the grid retains it.
fn push_row(rows: &Retained<NSMutableArray<NSArray<NSView>>>, views: &[Retained<NSView>]) {
    let row: Retained<NSMutableArray<NSView>> = NSMutableArray::new();
    for view in views {
        row.addObject(&*view);
    }
    rows.addObject(&*row.into_super());
}

/// A static label. `labelWithString:` is a non-editable `NSTextField` — the
/// only text in this window that is not a control the user operates.
fn label(mtm: MainThreadMarker, text: &str) -> Retained<NSView> {
    let field: Retained<NSTextField> = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.clone().into_super().into_super()
}

/// A slider for a continuous knob. `NSSlider::sliderWithValue:minValue:maxValue:target:action:`
/// starts the knob at its current value; the cell reads the dragged value back
/// from the slider (`NSControl::doubleValue`) when the action fires.
fn slider(
    mtm: MainThreadMarker,
    view: &NotchView,
    cells: &mut Vec<Retained<SettingsCell>>,
    knob: usize,
    value: CGFloat,
    lo: CGFloat,
    hi: CGFloat,
) -> Retained<NSView> {
    let cell = SettingsCell::new(mtm, knob, view);
    // Normalize the rail HERE so a runtime `hi` (the width rail is the live
    // display canvas, so `hi` can shrink between builds) can never invert the
    // range or leave the start value off the rail: the slider and the
    // `apply_knob_value` clamp then agree by construction.
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let value = value.clamp(lo, hi);
    let control: Retained<NSSlider> = unsafe {
        NSSlider::sliderWithValue_minValue_maxValue_target_action(
            value as c_double,
            lo as c_double,
            hi as c_double,
            Some(&*ProtocolObject::<dyn NSObjectProtocol>::from_ref(&*cell).as_ref()),
            Some(sel!(applyKnob)),
            mtm,
        )
    };
    keep(
        &cell,
        ProtocolObject::<dyn NSObjectProtocol>::from_ref(&*control).as_ref(),
    );
    cells.push(cell);
    control.clone().into_super().into_super()
}

/// A two-state switch for a boolean knob. `NSSwitch` has no target/action
/// factory in objc2-app-kit 0.3.2, so the pair is set with the bound
/// `NSControl::setTarget:`/`setAction:`.
fn switch(
    mtm: MainThreadMarker,
    view: &NotchView,
    cells: &mut Vec<Retained<SettingsCell>>,
    knob: usize,
    on: bool,
) -> Retained<NSView> {
    let cell = SettingsCell::new(mtm, knob, view);
    let control: Retained<NSSwitch> = NSSwitch::new(mtm);
    control.setState(if on {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    let target: &AnyObject = ProtocolObject::<dyn NSObjectProtocol>::from_ref(&*cell).as_ref();
    unsafe {
        control.setTarget(Some(target));
        control.setAction(Some(sel!(applyKnob)));
    }
    keep(
        &cell,
        ProtocolObject::<dyn NSObjectProtocol>::from_ref(&*control).as_ref(),
    );
    cells.push(cell);
    control.clone().into_super().into_super()
}

/// A popup menu for a preset knob. `NSPopUpButton::popUpButtonWithMenu:target:action:`
/// with the menu built from `NSMenuItem`s that each carry the same action and
/// the cell as their `target:` — that is the pair AppKit dispatches when the
/// user clicks an item in the pulled-down menu. The button itself also gets
/// the pair (the factory's `target:`/`action:`), so a click on the button
/// body fires too; both routes are idempotent because the handler re-reads the
/// control's current selection rather than an argument. `setPullsDown(true)`
/// so the menu opens beside the button (a closed "choose" popup would otherwise
/// snap back and show nothing).
fn popup(
    mtm: MainThreadMarker,
    view: &NotchView,
    cells: &mut Vec<Retained<SettingsCell>>,
    knob: usize,
    titles: Vec<Retained<NSString>>,
    selected: usize,
) -> Retained<NSView> {
    let cell = SettingsCell::new(mtm, knob, view);
    let menu: Retained<NSMenu> = NSMenu::new(mtm);
    let no_key: Retained<NSString> = NSString::from_str("");
    let target: &AnyObject = ProtocolObject::<dyn NSObjectProtocol>::from_ref(&*cell).as_ref();
    for title in titles {
        let item: Retained<NSMenuItem> = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &*title,
                Some(sel!(applyKnob)),
                &*no_key,
            )
        };
        // SAFETY: `setTarget:` is unretained — the cell is kept alive by the
        // owner's `cells` vec for the whole process lifetime.
        unsafe { item.setTarget(Some(target)) };
        menu.addItem(&*item);
    }
    let control: Retained<NSPopUpButton> = unsafe {
        NSPopUpButton::popUpButtonWithMenu_target_action(
            &*menu,
            Some(target),
            Some(sel!(applyKnob)),
        )
    };
    control.setPullsDown(true);
    control.selectItemAtIndex(selected as NSInteger);
    keep(
        &cell,
        ProtocolObject::<dyn NSObjectProtocol>::from_ref(&*control).as_ref(),
    );
    cells.push(cell);
    control.clone().into_super().into_super().into_super()
}

/// A segmented control for a small enumerated knob (cursor shape).
/// `NSSegmentedControl::segmentedControlWithLabels:trackingMode:target:action:`
/// with `SelectOne` (the plan's "POINTING_MODE"): one click picks one label,
/// and the cell reads it back with `selectedSegment`.
fn segmented(
    mtm: MainThreadMarker,
    view: &NotchView,
    cells: &mut Vec<Retained<SettingsCell>>,
    knob: usize,
    labels: &[&'static str],
    selected: usize,
) -> Retained<NSView> {
    let cell = SettingsCell::new(mtm, knob, view);
    let titles: Retained<NSMutableArray<NSString>> = NSMutableArray::new();
    for name in labels {
        titles.addObject(&*NSString::from_str(name));
    }
    let control: Retained<NSSegmentedControl> = unsafe {
        NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(
            &*titles.into_super(),
            NSSegmentSwitchTracking::SelectOne,
            Some(&*ProtocolObject::<dyn NSObjectProtocol>::from_ref(&*cell).as_ref()),
            Some(sel!(applyKnob)),
            mtm,
        )
    };
    control.setSelectedSegment(selected as NSInteger);
    keep(
        &cell,
        ProtocolObject::<dyn NSObjectProtocol>::from_ref(&*control).as_ref(),
    );
    cells.push(cell);
    control.clone().into_super().into_super()
}

/// A real color well for a color knob. `NSColorWell::colorWellWithStyle:` in
/// the `Minimal` style: a click shows a popover picker (the `Expanded`/`Default`
/// styles drag an `NSColorPanel`, which needs app activation this app does
/// not have). The cell reads the well back with `NSColorWell::color`.
fn well(
    mtm: MainThreadMarker,
    view: &NotchView,
    cells: &mut Vec<Retained<SettingsCell>>,
    knob: usize,
    color: Color,
) -> Retained<NSView> {
    let cell = SettingsCell::new(mtm, knob, view);
    let control: Retained<NSColorWell> =
        NSColorWell::colorWellWithStyle(NSColorWellStyle::Minimal, mtm);
    control.setColor(&*srgb_color(color));
    control.setSupportsAlpha(false);
    keep(
        &cell,
        ProtocolObject::<dyn NSObjectProtocol>::from_ref(&*control).as_ref(),
    );
    cells.push(cell);
    control.clone().into_super().into_super()
}

/// One ANSI swatch: an `NSBox` filled with the slot's current color, so the
/// 16-slot palette is visible without a picker. Display only — editing goes
/// through the two-step slot/color popup pair (the lazier of the plan's two
/// ANSI options: no `NSScrollView`, no per-slot well).
///
/// The `NSBox` default type is `Primary`, which IGNORES `fillColor` (AppKit's
/// `NSBox.h`: "Custom boxes draw based entirely on user parameters"), so the
/// box type must be `Custom` or every swatch renders as the default gray
/// frame. The default title is the literal string `Title`, so the title
/// position must be cleared too or each swatch also paints a stray label.
fn swatch(mtm: MainThreadMarker, color: Color) -> Retained<NSView> {
    let swatch_box: Retained<NSBox> = NSBox::new(mtm);
    swatch_box.setBoxType(NSBoxType::Custom);
    swatch_box.setTitlePosition(NSTitlePosition::NoTitle);
    swatch_box.setBorderWidth(0.0);
    swatch_box.setCornerRadius(3.0);
    swatch_box.setFillColor(&*srgb_color(color));
    swatch_box.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(SWATCH, SWATCH),
    ));
    swatch_box.clone().into_super()
}

/// The palette entry that matches the ANSI slot the two-step picker currently
/// points at, so the color popup starts on the color that slot already has
/// (otherwise it would claim slot 0 is palette 0 and the first click would
/// silently change a slot the user never chose). Falls back to palette 0.
fn ansi_color_index(settings: Settings) -> usize {
    let slot = ANSI_SLOT.load(Ordering::SeqCst).min(ANSI_SLOTS - 1);
    let current = settings.ansi[slot];
    let hex = current.hex();
    let found = PALETTE_PRESETS
        .iter()
        .enumerate()
        .filter(|(_, color)| color.hex() == hex)
        .reduce(|best, other| if other.0 < best.0 { other } else { best });
    match found {
        None => 0,
        Some((index, _)) => index,
    }
}

/// The 16 swatches, in one grid row. Wrapped in a single plain `NSView` so
/// the row is ONE grid cell: a 16-view row would otherwise force `NSGridView`
/// to 16 columns, which then stretch to the grid width and render the palette
/// as 16 ragged slabs (the pre-existing defect this replaces). The container
/// sizes itself to `16*SWATCH + 15*SWATCH_GAP` and lays the swatches out with
/// explicit frames, so `NSGridView` keeps its two columns (label + control),
/// the sliders stay long, and every swatch stays a `SWATCH`-sized square.
fn swatch_row(mtm: MainThreadMarker, settings: Settings) -> Vec<Retained<NSView>> {
    let container: Retained<NSView> = NSView::new(mtm);
    let step = SWATCH + SWATCH_GAP;
    for i in 0..ANSI_SLOTS {
        let tile = swatch(mtm, settings.ansi[i]);
        tile.setFrame(NSRect::new(
            NSPoint::new(step * i as CGFloat, 0.0),
            NSSize::new(SWATCH, SWATCH),
        ));
        container.addSubview(&*tile);
    }
    container.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(step * ANSI_SLOTS as CGFloat - SWATCH_GAP, SWATCH),
    ));
    vec![container]
}

/// Record the control a cell will receive its action from, so the action
/// handler can read the control's current value back. Raw because a control
/// retains its `target`: a `Retained` back-pointer would be a cycle.
fn keep(cell: &Retained<SettingsCell>, control: &AnyObject) {
    let _ = cell.ivars().control.set(control as *const AnyObject);
}

// ── Menu titles ───────────────────────────────────────────────────────────
fn family_titles() -> Vec<Retained<NSString>> {
    FONT_FAMILIES.map(|name| NSString::from_str(name)).to_vec()
}

fn scrollback_titles() -> Vec<Retained<NSString>> {
    SCROLLBACK_PRESETS
        .map(|lines| NSString::from_str(&format!("{0}", lines)))
        .to_vec()
}

fn chord_titles() -> Vec<Retained<NSString>> {
    CHORD_PRESETS
        .map(|chord| NSString::from_str(&chord_name(chord)))
        .to_vec()
}

fn palette_titles() -> Vec<Retained<NSString>> {
    PALETTE_PRESETS
        .map(|color| NSString::from_str(&color.hex()))
        .to_vec()
}

fn ansi_slot_titles() -> Vec<Retained<NSString>> {
    (0..ANSI_SLOTS)
        .map(|i| NSString::from_str(&format!("{0}", i)))
        .collect()
}

/// The About section: app name, version, the live chord names, the plist
/// path, and why chords are preset-only. Static labels — the window cannot
/// take typed input under `Accessory` policy, and Carbon cannot record an
/// arbitrary chord.
fn about_rows(settings: Settings) -> Vec<String> {
    vec![
        "NotchQuake — notch terminal, v0.1-alpha".to_string(),
        format!("Toggle chord: {0}", chord_name(settings.toggle_chord)),
        format!("New tab chord: {0}", chord_name(settings.new_tab_chord)),
        "Saved in ~/Library/Preferences/local.notchquake.plist".to_string(),
        "Chords are limited to the registered presets above: the Carbon Event".to_string(),
        "Manager can only deliver chords it registered at launch, and recording".to_string(),
        "an arbitrary chord needs an Input Monitoring grant this app does not have.".to_string(),
    ]
}

// ── Entry points used by the notch view ───────────────────────────────────
/// Open the settings window, or re-focus (and re-show) it if it is already
/// open. The owner is created at most once, so a second gear click cannot
/// duplicate the window.
///
/// `slot` is the `NotchViewIvars` `OnceCell` that keeps the owner alive for
/// the process lifetime.
pub fn open_or_focus(
    mtm: MainThreadMarker,
    slot: &OnceCell<Retained<SettingsOwner>>,
    view: &NotchView,
) {
    if let Some(owner) = slot.get() {
        owner.show_or_focus();
        return;
    }
    let owner = SettingsOwner::new(mtm);
    owner.build_and_show(mtm, view);
    let _ = slot.set(owner);
}

/// The settings window's frame in AppKit screen space, or `None` while the
/// window has never been created. The notch view hit-tests this FIRST for
/// every click its global monitor sees: a click inside the settings window is
/// for the window, not for the panel, and without this guard an open panel
/// collapses from a click on any control.
pub fn window_frame(slot: &OnceCell<Retained<SettingsOwner>>) -> Option<NSRect> {
    match slot.get() {
        None => None,
        Some(owner) => owner.window_frame(),
    }
}

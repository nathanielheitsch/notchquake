//! App bootstrap: an `NSApplicationDelegate` that installs the notch panel,
//! its silhouette view, and the Option+Space global hotkey.
//!
//! Follows objc2's own `examples/hello_world_app.rs`: a `define_class!`
//! delegate, `sharedApplication`, `setDelegate`, `activateIgnoringOtherApps`
//! (required when the binary is launched unbundled, as Cargo does), then
//! `run()`.

pub mod consts;

pub mod animation;
pub mod effects;
pub mod font;
pub mod hotkey;
pub mod input;
pub mod notch_view;
pub mod render;
pub mod settings;
pub mod settings_window;
pub mod terminal;
pub mod window;

use std::cell::OnceCell;
type Monitor = Option<Retained<AnyObject>>;
use std::ptr::NonNull;

use objc2::ClassType;
use objc2::DefinedClass;
use objc2::MainThreadOnly;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::sel;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationDidChangeScreenParametersNotification, NSEvent, NSEventMask, NSEventType, NSMenu,
    NSMenuItem, NSPanel, NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint, NSRect,
    NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};

use block2::RcBlock;

use crate::consts::PANEL_HEIGHT;
use crate::notch_view::NotchView;
use crate::window::create_panel;

/// The delegate keeps the panel, the silhouette view, and the event monitor
/// alive for the whole process lifetime.
#[derive(Default)]
struct AppDelegateIvars {
    panel: OnceCell<Retained<NSPanel>>,
    view: OnceCell<Retained<NotchView>>,
    monitors: OnceCell<(Monitor, Monitor, Monitor)>,
    // Held so the screen-parameter observer stays registered for the app's
    // life. `addObserverForName:...usingBlock:` returns an opaque observer
    // token; NSNotificationCenter holds it weakly-to-nothing, so dropping it
    // silently unregisters the block.
    screen_observer: OnceCell<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    tick_timer: OnceCell<Retained<NSTimer>>,
    motion_observer: OnceCell<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

define_class!(
    // SAFETY:
    // - The superclass `NSObject` does not have any subclassing requirements.
    // - `AppDelegateIvars` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppDelegate"]
    #[ivars = AppDelegateIvars]
    struct AppDelegate;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for AppDelegate {}

    // SAFETY: `NSApplicationDelegate` has no safety requirements.
    unsafe impl NSApplicationDelegate for AppDelegate {
        // SAFETY: `applicationDidFinishLaunching:` takes an
        // `NSNotification *`; the signature is correct.
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, notification: &NSNotification) {
            let mtm = self.mtm();
            let ivars = self.ivars();

            // The panel window and its content view.
            let (panel, content_view) = create_panel(mtm);
            let _ = ivars.panel.set(panel);

            // The silhouette view: a layer-backed, flipped subview sized to
            // the whole panel, so its drawing uses a top-left origin, y-down
            // space.
            let view = NotchView::new(mtm);
            // Deliberately NOT layer-backed: a layer-backed view draws via
            // `updateLayer`, whose graphics context was nil here; the plain
            // `drawRect:` path (below) always has a current context.
            content_view.addSubview(view.as_super());
            view.as_super().setFrame(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(crate::window::canvas_size(mtm), PANEL_HEIGHT),
            ));
            let _ = ivars.view.set(view.clone());
            let screen = crate::window::notch_screen(mtm);
            if screen.is_some() {
                let sc = screen.unwrap();
                let (w, h) = crate::window::notch_size(&sc);
                view.set_notch((w, h));
            }
            // Guarantee the first paint: a newly inserted layer is not
            // always marked dirty by AppKit.
            view.as_super().setNeedsDisplay(true);

            // Only toggle is global, and only the EFFECTIVE one is registered:
            // a stored chord that cannot be registered stays stored (reported by
            // `Settings::load_with_status`) and never becomes active.
            for warning in crate::hotkey::register_hotkeys(
                mtm,
                &view,
                view.current_settings().effective_chords().0,
            ) {
                view.record_status(warning);
            }
            install_menus(mtm, self);
            // Startup warnings (malformed prefs, font/hotkey fallbacks) are logged, never
            // shown with a modal here: `NSAlert::runModal` before `app.run()` blocks the run
            // loop until dismissed, which hangs launch. The recorded status stays available
            // through `NotchView::settings_status` for the settings UI to surface.
            for warning in view.settings_status() {
                eprintln!("NotchQuake: {warning}");
            }
            let view_for_keys = view.clone();
            let key_block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
                if MainThreadMarker::new().is_some() {
                    let event_ref = unsafe { event.as_ref() };
                    if event_ref.r#type() == NSEventType::KeyDown && view_for_keys.handle_key(event_ref) {
                        return std::ptr::null_mut();
                    }
                }
                event.as_ptr()
            });
            let local_monitor = unsafe {
                NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                    NSEventMask::KeyDown,
                    &key_block,
                )
            };
            if local_monitor.is_none() {
                eprintln!("notch-terminal: local event monitor was not installed");
            }
            // Clicks: a non-activating panel never becomes key, so real clicks ON
            // the panel go to the view's `mouseDown:`. Clicks that pass through a
            // transparent region — and synthetic/test clicks delivered to the
            // frontmost app — are observed by this global monitor and hit-tested
            // via the absolute mouse location. The two paths don't double-fire
            // (a click either hits the panel or is dispatched elsewhere).
            let view_for_clicks = view.clone();
            let click_block = RcBlock::new(move |event: NonNull<NSEvent>| {
                if MainThreadMarker::new().is_some() {
                    view_for_clicks.handle_click(unsafe { event.as_ref() });
                }
            });
            let click_monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                NSEventMask::LeftMouseDown,
                &click_block,
            );
            // Scroll wheel: one global monitor. macOS keeps only ONE global
            // monitor per event type, so a second ScrollWheel monitor would
            // silently drop this one (that was the old bug that killed the
            // wheel).
            let view_for_wheel = view.clone();
            let wheel_block = RcBlock::new(move |event: NonNull<NSEvent>| {
                if MainThreadMarker::new().is_some() {
                    view_for_wheel.scroll_wheel(unsafe { event.as_ref() }.deltaY());
                }
            });
            let wheel_monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                NSEventMask::ScrollWheel,
                &wheel_block,
            );

            // Display geometry change (resolution switch, monitor
            // plug/unplug, menu-bar/Dock rearrangement). The panel is placed
            // once at startup and AppKit does NOT re-centre a borderless
            // panel on a resize, so it keeps its old origin and drifts off
            // centre (usually horizontally).
            //
            // The previous approach watched for a ScrollWheel event with
            // subtype ScreenChanged. That NEVER fires in a global monitor:
            // measured on a 1920->2048 resize the panel origin stayed exactly
            // (640,623) — `reposition` was simply never called. Use the
            // documented NSApplication notification instead.
            let center = NSNotificationCenter::defaultCenter();
            let view_for_screen = view.clone();
            let screen_block = RcBlock::new(move |_note: NonNull<NSNotification>| {
                view_for_screen.reposition();
            });
            let screen_observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSApplicationDidChangeScreenParametersNotification),
                    None,
                    None,
                    &screen_block,
                )
            };
            let _ = ivars.screen_observer.set(screen_observer);

            let motion_view = view.clone();
            let motion_block = RcBlock::new(move |_note: NonNull<NSNotification>| {
                motion_view.update_reduce_motion();
            });
            let workspace = NSWorkspace::sharedWorkspace();
            let observer = unsafe { workspace.notificationCenter().addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification), None, None, &motion_block,
            ) };
            let _ = ivars.motion_observer.set(observer);
            // Cheap common-mode pump: still wakes at 60Hz, but does not repaint idle terminals.
            let tick_proto: &ProtocolObject<dyn NSObjectProtocol> =
                ProtocolObject::from_ref(&*view);
            let tick_target: &AnyObject = tick_proto.as_ref();
            let tick_timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    1.0 / 60.0,
                    tick_target,
                    sel!(tick),
                    None,
                    true,
                )
            };
            unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&tick_timer, NSRunLoopCommonModes) };
            let _ = ivars.tick_timer.set(tick_timer);
            let _ = ivars
                .monitors
                .set((local_monitor, click_monitor, wheel_monitor));
            // Take over from any other app and activate. Required because
            // Cargo builds an unbundled binary (no .app bundle / no launch
            // request).
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _notification: &NSNotification) {
            if let Some(view) = self.ivars().view.get()
                && let Err(error) = view.flush_settings()
            {
                view.record_status(error.to_string());
            }
        }
    }

    impl AppDelegate {
        #[unsafe(method(showSettings:))]
        fn show_settings_action(&self, _sender: Option<&AnyObject>) {
            if let Some(view) = self.ivars().view.get() { view.show_settings(); }
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppDelegateIvars::default());
        // SAFETY: `NSObject`'s `init` signature is correct.
        unsafe { msg_send![super(this), init] }
    }
}

fn main() {
    let mtm = MainThreadMarker::new().unwrap();

    let app = NSApplication::sharedApplication(mtm);
    let delegate = AppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    app.run();
}

fn install_menus(mtm: MainThreadMarker, delegate: &AppDelegate) {
    let app = NSApplication::sharedApplication(mtm);
    let root = NSMenu::new(mtm);
    let app_menu = NSMenu::new(mtm);
    let app_item = NSMenuItem::new(mtm);
    app_item.setSubmenu(Some(&app_menu));
    root.addItem(&app_item);
    for (title, action, key) in [
        ("About NotchQuake", sel!(orderFrontStandardAboutPanel:), ""),
        ("Settings…", sel!(showSettings:), ","),
        ("Quit NotchQuake", sel!(terminate:), "q"),
    ] {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        if key == "," {
            unsafe { item.setTarget(Some(delegate)) };
        } else {
            unsafe { item.setTarget(Some(&app)) };
        }
        app_menu.addItem(&item);
    }
    let edit_menu = NSMenu::new(mtm);
    edit_menu.setTitle(&NSString::from_str("Edit"));
    let edit_item = NSMenuItem::new(mtm);
    edit_item.setTitle(&NSString::from_str("Edit"));
    edit_item.setSubmenu(Some(&edit_menu));
    root.addItem(&edit_item);
    for (title, action, key) in [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
        ("Close Window", sel!(performClose:), "w"),
    ] {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        if key == "Z" {
            item.setKeyEquivalentModifierMask(
                objc2_app_kit::NSEventModifierFlags::Command
                    | objc2_app_kit::NSEventModifierFlags::Shift,
            );
        }
        edit_menu.addItem(&item);
    }
    app.setMainMenu(Some(&root));
}

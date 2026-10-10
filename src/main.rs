//! App bootstrap: an `NSApplicationDelegate` that installs the notch panel,
//! its silhouette view, and the Option+Space global hotkey.
//!
//! Follows objc2's own `examples/hello_world_app.rs`: a `define_class!`
//! delegate, `sharedApplication`, `setDelegate`, `activateIgnoringOtherApps`
//! (required when the binary is launched unbundled, as Cargo does), then
//! `run()`.

pub mod consts;

pub mod window;
pub mod hotkey;
pub mod animation;
pub mod font;
pub mod notch_view;
pub mod terminal;
pub mod render;


use std::cell::OnceCell;
type Monitor = Option<Retained<AnyObject>>;
use std::ptr::NonNull;

use objc2::ClassType;
use objc2::define_class;
use objc2::DefinedClass;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSApplication,
    NSApplicationActivationPolicy,
    NSApplicationDelegate,
    NSPanel,
    NSEvent,
    NSEventMask,
    NSEventType,
    NSEventModifierFlags,
    NSEventSubtype,
    NSScreen,
};
use objc2_foundation::{
    MainThreadMarker,
    NSObjectProtocol,
    NSNotification,
    NSPoint,
    NSRect,
    NSSize,
    NSTimer,
    NSTimeInterval,
};
use objc2::sel;

use block2::RcBlock;

use crate::consts::{PANEL_HEIGHT, PANEL_WIDTH, SPACE_KEY_CODE};
use crate::notch_view::NotchView;
use crate::window::create_panel;

/// The delegate keeps the panel, the silhouette view, and the event monitor
/// alive for the whole process lifetime.
#[derive(Default)]
struct AppDelegateIvars {
    panel: OnceCell<Retained<NSPanel>>,
    view: OnceCell<Retained<NotchView>>,
    monitors: OnceCell<(Monitor, Monitor, Monitor, Monitor)>,
    tick_timer: OnceCell<Retained<NSTimer>>,
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
            let (panel, content_view) = create_panel(mtm);            let _ = ivars.panel.set(panel);

            // The silhouette view: a layer-backed, flipped subview sized to
            // the whole panel, so its drawing uses a top-left origin, y-down
            // space.
            let view = NotchView::new(mtm);
            // Deliberately NOT layer-backed: a layer-backed view draws via
            // `updateLayer`, whose graphics context was nil here; the plain
            // `drawRect:` path (below) always has a current context.
            content_view.addSubview(view.as_super());
            view.as_super().setFrame(
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(PANEL_WIDTH, PANEL_HEIGHT),
                ),
            );
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

            // The Option+Space global hotkey. A non-activating panel can never
            // take key focus, so the hotkey is observed system-wide. We use
            // the Carbon Event Manager (`RegisterEventHotKey`): it delivers a
            // dedicated hotkey event to this app on the main run loop with NO
            // Accessibility / Input Monitoring grant and without being
            // frontmost. `NSEvent::addGlobalMonitorForEvents` cannot see key
            // events without an accessibility grant, which an adhoc-signed dev
            // bundle does not get — so the NSEvent monitor silently receives
            // zero callbacks. The Carbon path sees only this chord, never
            // other keystrokes.
            let view_ptr: *const crate::notch_view::NotchView = &*view;
            let hotkey_ok = crate::hotkey::register_option_space(mtm, &*view);
            let _ = view_ptr;
            if !hotkey_ok {
                eprintln!("notch-terminal: global Option+Space hotkey not registered");
            }
            // Keystrokes: a LOCAL monitor on the panel. A local monitor only
            // fires while the panel window has key focus (clicking inside it
            // claims focus); returning nil lets the event keep going to the
            // rest of the desktop.
            let view_for_keys = view.clone();
            let key_block = RcBlock::new(
                move |event: NonNull<NSEvent>| -> *mut NSEvent {
                    if MainThreadMarker::new().is_some() {
                        let event_ref = unsafe { event.as_ref() };
                        if event_ref.r#type() == NSEventType::KeyDown {
                            view_for_keys.handle_key(event_ref);
                        }
                    }
                    std::ptr::null_mut()
                },
            );
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
            let click_block = RcBlock::new(
                move |event: NonNull<NSEvent>| {
                    if MainThreadMarker::new().is_some() {
                        view_for_clicks.handle_click(unsafe { event.as_ref() });
                    }
                },
            );
            let click_monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                NSEventMask::LeftMouseDown,
                &click_block,
            );
            // Scroll wheel + display change: ONE global monitor on the
            // ScrollWheel mask. macOS keeps only ONE global monitor per event
            // type — registering a second one for the same mask silently drops
            // the first, which is why the wheel stopped working once the
            // display-change monitor was added. A display change (monitor
            // unplugged / resolution changed) arrives as a ScrollWheel event
            // with subtype ScreenChanged carrying a bogus deltaY, so dispatch
            // on the subtype inside this single handler: reposition the panel,
            // never scroll.
            let view_for_wheel = view.clone();
            let wheel_block = RcBlock::new(
                move |event: NonNull<NSEvent>| {
                    if MainThreadMarker::new().is_some() {
                        let event_ref = unsafe { event.as_ref() };
                        if event_ref.subtype() == NSEventSubtype::ScreenChanged {
                            view_for_wheel.reposition();
                        } else {
                            view_for_wheel.scroll_wheel(event_ref.deltaY());
                        }
                    }
                },
            );
            let wheel_monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                NSEventMask::ScrollWheel,
                &wheel_block,
            );

            // dirty from an ivar change, so a repeating timer drives the
            // animation passes. `tick` no-ops once the spring settles.
            let tick_proto: &ProtocolObject<dyn NSObjectProtocol> = ProtocolObject::from_ref(&*view);
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
            let _ = ivars.tick_timer.set(tick_timer);
            let _ = ivars.monitors.set((None, local_monitor, click_monitor, wheel_monitor));
            // Take over from any other app and activate. Required because
            // Cargo builds an unbundled binary (no .app bundle / no launch
            // request).
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
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

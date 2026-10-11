//! Global hotkey via the Carbon Event Manager (`RegisterEventHotKey`).
//!
//! A non-activating `NSPanel` can never take key focus, so the Option+Space
//! hotkey must be observed system-wide. `NSEvent::addGlobalMonitorForEvents`
//! needs an Accessibility / Input Monitoring grant to see *key* events — our
//! adhoc-signed dev bundle gets neither, so the monitor silently receives
//! zero callbacks. The Carbon hotkey API registers a chord system-wide and
//! delivers a dedicated hotkey event to *this* app on the main run loop
//! without any TCC grant and without the app being frontmost. It sees only
//! the registered chord, never other keystrokes.

use objc2::msg_send;
use objc2::sel;
use objc2::ClassType;
use objc2_foundation::MainThreadMarker;

use crate::notch_view::NotchView;

/// `OSType` four-character code: `b"keyb"` -> `0x6B657962`.
const K_EVENT_CLASS_KEYBOARD: u32 = 0x6B657962;
/// `kEventHotKeyPressed`.
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
/// `kEventParamDirectObject`: `b"----"` -> `0x2D2D2D2D`.
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = 0x2D2D2D2D;
/// `typeEventHotKeyID`: `b"hkid"` -> `0x686B6964`.
const TYPE_EVENT_HOT_KEY_ID: u32 = 0x686B6964;

/// Carbon key-modifier bit constants (`<Carbon Events.h>`).
const ACTIVE_FLAG_BIT: u32 = 0;
const CMD_KEY_BIT: u32 = 8;
const SHIFT_KEY_BIT: u32 = 9;
const ALPHA_LOCK_BIT: u32 = 10;
const OPTION_KEY_BIT: u32 = 11;
const CONTROL_KEY_BIT: u32 = 12;

fn modifier_mask(option: bool, cmd: bool, shift: bool, control: bool) -> u32 {
    let mut m = 0u32;
    if cmd {
        m |= 1 << CMD_KEY_BIT;
    }
    if option {
        m |= 1 << OPTION_KEY_BIT;
    }
    if shift {
        m |= 1 << SHIFT_KEY_BIT;
    }
    if control {
        m |= 1 << CONTROL_KEY_BIT;
    }
    // Carbon expects the active-flag bit set alongside any modifier (it is the
    // "modifiers pressed" flag). Without it some chord registrations no-op.
    if m != 0 {
        m |= 1 << ACTIVE_FLAG_BIT;
    }
    m
}

#[repr(C)]
#[derive(Copy, Clone)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

type EventTargetRef = *mut std::ffi::c_void;
type EventHandlerRef = *mut std::ffi::c_void;
type EventHandlerUPP =
    unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void) -> u32;
type EventHotKeyRef = *mut std::ffi::c_void;

// Suppress the unused `active_flag_bit` for clarity; it IS used in the mask.
#[allow(dead_code)]
const _USED: u32 = ALPHA_LOCK_BIT;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerUPP,
        num_types: u32,
        type_list: *const EventTypeSpec,
        user_data: *mut std::ffi::c_void,
        out_ref: *mut EventHandlerRef,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> i32;
    fn GetEventParameter(
        event: *mut std::ffi::c_void,
        name: u32,
        type_: u32,
        out_type: *mut u32,
        buffer_size: usize,
        actual_size: *mut usize,
        out_value: *mut std::ffi::c_void,
    ) -> i32;
}

/// Carbon hotkey ids we register, dispatched by `hot_key_handler`.
const HOTKEY_ID_TOGGLE: u32 = 1;
/// iTerm's "new tab" chord (Cmd+T).
const HOTKEY_ID_NEW_TAB: u32 = 2;

/// The Carbon event handler. `userData` is the raw `NotchView *` we passed to
/// `InstallEventHandler`. Carbon dispatches this on the main run loop, so
/// the main-thread marker is available. Dispatches on the hotkey `id` so one
/// handler serves every registered chord.
unsafe extern "C" fn hot_key_handler(
    _caller: *mut std::ffi::c_void,
    event: *mut std::ffi::c_void,
    user_data: *mut std::ffi::c_void,
) -> u32 {
    // Confirm this is the hot-key event we registered, then dispatch on its id.
    let mut hot_id = EventHotKeyID { signature: 0, id: 0 };
    let status = unsafe {
        GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>(),
            std::ptr::null_mut(),
            &mut hot_id as *mut EventHotKeyID as *mut std::ffi::c_void,
        )
    };
    if status != 0 {
        return status as u32;
    }
    if let Some(mtm) = MainThreadMarker::new() {
        // SAFETY: `user_data` is the raw pointer to a `NotchView` retained for
        // the whole process lifetime by the delegate (kept in `OnceCell`). It
        // is a valid, live `NotchView` on the main thread.
        let view: &NotchView = unsafe { &*(user_data as *const NotchView) };
        let _ = mtm;
        match hot_id.id {
            HOTKEY_ID_TOGGLE => view.toggle(),
            HOTKEY_ID_NEW_TAB => view.spawn_tab(),
            _ => {},
        }
    }
    0
}

/// Register the global hotkeys. The returned value is only non-zero on
/// failure (matching the old monitor's `is_none()` check).
///
/// `view` is retained by the caller for the process lifetime; we hand Carbon
/// its raw pointer as the handler's `userData`.
pub fn register_hotkeys(mtm: MainThreadMarker, view: &NotchView) -> bool {
    let _ = mtm;
    let target = unsafe { GetApplicationEventTarget() };
    if target.is_null() {
        eprintln!("notch-terminal: GetApplicationEventTarget() returned null");
        return false;
    }
    let spec = EventTypeSpec {
        event_class: K_EVENT_CLASS_KEYBOARD,
        event_kind: K_EVENT_HOT_KEY_PRESSED,
    };
    let mut handler_ref: EventHandlerRef = std::ptr::null_mut();
    let status = unsafe {
        InstallEventHandler(
            target,
            hot_key_handler,
            1,
            &spec,
            view as *const NotchView as *mut std::ffi::c_void,
            &mut handler_ref,
        )
    };
    if status != 0 {
        eprintln!("notch-terminal: InstallEventHandler failed: {status}");
        return false;
    }
    // Space virtual key code is 49; Option is the only modifier we want.
    let modifiers = modifier_mask(true, false, false, false);
    let id = EventHotKeyID {
        signature: u32::from_be_bytes(*b"BNTh"),
        id: HOTKEY_ID_TOGGLE,
    };
    let mut hot_ref: EventHotKeyRef = std::ptr::null_mut();
    let status = unsafe {
        RegisterEventHotKey(49, modifiers, id, target, 0, &mut hot_ref)
    };
    if status != 0 {
        eprintln!("notch-terminal: RegisterEventHotKey failed: {status}");
        return false;
    }
    // iTerm's "new tab": Cmd+T. kVK_ANSI_T is 0x11 (17) in Carbon's virtual-
    // key table (Events.h) - NOT the letter's position. Same handler, other id.
    let cmd_t = modifier_mask(false, true, false, false);
    let new_tab_id = EventHotKeyID {
        signature: u32::from_be_bytes(*b"BNTh"),
        id: HOTKEY_ID_NEW_TAB,
    };
    let mut hot_ref2: EventHotKeyRef = std::ptr::null_mut();
    let status = unsafe {
        RegisterEventHotKey(0x11, cmd_t, new_tab_id, target, 0, &mut hot_ref2)
    };
    if status != 0 {
        eprintln!("notch-terminal: RegisterEventHotKey (Cmd+T) failed: {status}");
        return false;
    }
    true
}

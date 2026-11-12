//! Transactional, main-thread Carbon toggle registration. New tab is terminal-local.
use std::cell::{Cell, RefCell};
use std::fmt;

use objc2_foundation::MainThreadMarker;

use crate::notch_view::NotchView;
use crate::settings::{Chord, DEFAULT_TOGGLE, chord_mask, validate_toggle_chord};

const SIGNATURE: u32 = u32::from_be_bytes(*b"NQhk");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrationError(pub String);

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub trait Backend {
    fn register(&mut self, chord: Chord, generation: u32) -> Result<usize, i32>;
    fn unregister(&mut self, handle: usize) -> Result<(), i32>;
}

pub struct Registration<B> {
    backend: B,
    active: Option<(Chord, u32, usize)>,
    next_generation: u32,
    // Failed cleanup handles remain owned and never dispatch as active.
    retained: Vec<usize>,
}

impl<B: Backend> Registration<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            active: None,
            next_generation: 1,
            retained: vec![],
        }
    }

    pub fn active_generation(&self) -> Option<u32> {
        self.active.map(|(_, generation, _)| generation)
    }

    pub fn change(&mut self, chord: Chord) -> Result<(), RegistrationError> {
        validate_toggle_chord(chord).map_err(|e| RegistrationError(e.to_string()))?;
        if self.active.is_some_and(|(old, _, _)| old == chord) {
            return Ok(());
        }
        let generation = self.next_generation;
        self.next_generation = self.next_generation.checked_add(1).ok_or_else(|| {
            RegistrationError("Registration generation exhausted; restart the app".into())
        })?;
        let candidate = self.backend.register(chord, generation).map_err(|status| {
            RegistrationError(format!(
                "Carbon register failed ({status}); previous shortcut retained"
            ))
        })?;
        if let Some((_, _, old)) = self.active
            && let Err(status) = self.backend.unregister(old)
        {
            let cleanup = self.backend.unregister(candidate);
            let detail = match cleanup {
                Ok(()) => "candidate removed".into(),
                Err(code) => {
                    self.retained.push(candidate);
                    format!("candidate cleanup failed ({code}); handle retained, events ignored")
                }
            };
            return Err(RegistrationError(format!(
                "Carbon release of previous shortcut failed ({status}); {detail}; previous shortcut retained"
            )));
        }
        self.active = Some((chord, generation, candidate));
        Ok(())
    }
}

struct Carbon;
type Raw = *mut std::ffi::c_void;

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventId {
    signature: u32,
    generation: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> Raw;
    fn InstallEventHandler(
        target: Raw,
        handler: unsafe extern "C" fn(Raw, Raw, Raw) -> i32,
        count: u32,
        types: *const EventTypeSpec,
        user: Raw,
        out: *mut Raw,
    ) -> i32;
    fn RegisterEventHotKey(
        key: u32,
        modifiers: u32,
        id: EventId,
        target: Raw,
        options: u32,
        out: *mut Raw,
    ) -> i32;
    fn UnregisterEventHotKey(handle: Raw) -> i32;
    fn GetEventParameter(
        event: Raw,
        name: u32,
        kind: u32,
        out_type: *mut u32,
        size: usize,
        actual: *mut usize,
        value: Raw,
    ) -> i32;
}

impl Backend for Carbon {
    fn register(&mut self, chord: Chord, generation: u32) -> Result<usize, i32> {
        let mut handle = std::ptr::null_mut();
        let status = unsafe {
            RegisterEventHotKey(
                chord.key_code as u32,
                chord_mask(chord),
                EventId {
                    signature: SIGNATURE,
                    generation,
                },
                GetApplicationEventTarget(),
                0,
                &mut handle,
            )
        };
        if status == 0 && !handle.is_null() {
            Ok(handle as usize)
        } else {
            Err(if status == 0 { -1 } else { status })
        }
    }

    fn unregister(&mut self, handle: usize) -> Result<(), i32> {
        let status = unsafe { UnregisterEventHotKey(handle as Raw) };
        if status == 0 { Ok(()) } else { Err(status) }
    }
}

thread_local! {
    static REGISTRATION: RefCell<Registration<Carbon>> = RefCell::new(Registration::new(Carbon));
    static ACTIVE: Cell<Option<u32>> = const { Cell::new(None) };
    static HANDLER: Cell<usize> = const { Cell::new(0) };
}

pub fn accepts_event(signature: u32, generation: u32, active: Option<u32>) -> bool {
    signature == SIGNATURE && active == Some(generation)
}

unsafe extern "C" fn handler(_caller: Raw, event: Raw, user: Raw) -> i32 {
    let mut id = EventId {
        signature: 0,
        generation: 0,
    };
    let status = unsafe {
        GetEventParameter(
            event,
            u32::from_be_bytes(*b"----"),
            u32::from_be_bytes(*b"hkid"),
            std::ptr::null_mut(),
            std::mem::size_of::<EventId>(),
            std::ptr::null_mut(),
            &mut id as *mut EventId as Raw,
        )
    };
    if status != 0 {
        return status;
    }
    if MainThreadMarker::new().is_some()
        && ACTIVE.with(|a| accepts_event(id.signature, id.generation, a.get()))
    {
        // Delegate retains the view and installed handler for the application's lifetime.
        unsafe { &*(user as *const NotchView) }.toggle();
    }
    0
}

pub fn change_toggle(_mtm: MainThreadMarker, chord: Chord) -> Result<(), RegistrationError> {
    let result = REGISTRATION.with(|r| r.borrow_mut().change(chord));
    // No mutable registry borrow is held when dispatch becomes eligible.
    let active = REGISTRATION.with(|r| r.borrow().active_generation());
    ACTIVE.with(|a| a.set(active));
    result
}

/// Returns warnings; fallback never overwrites the stored preference.
pub fn register_hotkeys(mtm: MainThreadMarker, view: &NotchView, chord: Chord) -> Vec<String> {
    let mut out = vec![];
    if HANDLER.with(|h| h.get() == 0) {
        let target = unsafe { GetApplicationEventTarget() };
        let mut handle = std::ptr::null_mut();
        let status = if target.is_null() {
            -1
        } else {
            unsafe {
                InstallEventHandler(
                    target,
                    handler,
                    1,
                    &EventTypeSpec {
                        class: u32::from_be_bytes(*b"keyb"),
                        kind: 5,
                    },
                    view as *const NotchView as Raw,
                    &mut handle,
                )
            }
        };
        if status != 0 {
            return vec![format!(
                "Carbon handler installation failed ({status}); click the notch to open"
            )];
        }
        HANDLER.with(|h| h.set(handle as usize));
    }
    if let Err(e) = change_toggle(mtm, chord) {
        out.push(e.to_string());
        if chord != DEFAULT_TOGGLE {
            match change_toggle(mtm, DEFAULT_TOGGLE) {
                Ok(()) => out
                    .push("Using Option+Space temporarily; saved shortcut was not changed".into()),
                Err(e) => out.push(format!(
                    "Default fallback also failed: {e}; click the notch to open"
                )),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        failures: VecDeque<bool>,
        next: usize,
    }

    impl Backend for Fake {
        fn register(&mut self, _: Chord, generation: u32) -> Result<usize, i32> {
            self.calls.push(format!("register {generation}"));
            if self.failures.pop_front().unwrap_or(false) {
                return Err(-10);
            }
            self.next += 1;
            Ok(self.next)
        }

        fn unregister(&mut self, handle: usize) -> Result<(), i32> {
            self.calls.push(format!("unregister {handle}"));
            if self.failures.pop_front().unwrap_or(false) {
                Err(-20)
            } else {
                Ok(())
            }
        }
    }

    fn alternative() -> Chord {
        Chord {
            key_code: 50,
            ..DEFAULT_TOGGLE
        }
    }

    #[test]
    fn transactional_order_and_unchanged() {
        let mut r = Registration::new(Fake::default());
        r.change(DEFAULT_TOGGLE).unwrap();
        r.change(DEFAULT_TOGGLE).unwrap();
        assert_eq!(r.backend.calls.len(), 1);
        r.change(alternative()).unwrap();
        assert_eq!(
            r.backend.calls,
            ["register 1", "register 2", "unregister 1"]
        );
        assert!(!accepts_event(SIGNATURE, 1, r.active_generation()));
        assert!(accepts_event(SIGNATURE, 2, r.active_generation()));
        assert!(!accepts_event(0, 2, r.active_generation()));
    }

    #[test]
    fn registration_and_rollback_failures_retain_old() {
        let mut r = Registration::new(Fake::default());
        r.change(DEFAULT_TOGGLE).unwrap();
        r.backend.failures = [true].into();
        assert!(r.change(alternative()).is_err());
        assert_eq!(r.active_generation(), Some(1));
        r.backend.failures = [false, true, false].into();
        assert!(r.change(alternative()).is_err());
        assert_eq!(r.active_generation(), Some(1));
        assert!(r.retained.is_empty());
        r.backend.failures = [false, true, true].into();
        assert!(
            r.change(alternative())
                .unwrap_err()
                .0
                .contains("cleanup failed")
        );
        assert_eq!(r.active_generation(), Some(1));
        assert_eq!(r.retained.len(), 1);
    }
}

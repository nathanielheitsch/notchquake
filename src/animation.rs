//! Hand-rolled velocity-based damped spring, matching Apple's
//! `Animation.spring(response:, dampingFraction:)` model.
//!
//! For an underdamped spring (`dampingFraction < 1`) the closed form of the
//! integrated value is
//!
//! ```text
//! value(t) = target + (start - target) * e^(-zeta*omega0*t)
//!            * (cos(omegaD*t) + (zeta*omega0/omegaD) * sin(omegaD*t))
//! ```
//!
//! with `omega0 = 2*pi/response` and `omegaD = omega0*sqrt(1 - zeta^2)`.
//!
//! We advance with wall-clock deltas, one step per `drawRect:` pass, and
//! stop once both value and velocity are within half a logical pixel.

use std::cell::Cell;

use crate::consts::{SPRING_DAMPING_FRACTION, SPRING_RESPONSE};

/// Half a logical pixel: closer than this is sub-pixel and we snap to target.
const SETTLE_EPSILON: f64 = 0.5;

/// A single-track spring over one scalar (the silhouette height).
///
/// All state is in `Cell`s so the spring can live in an `NSView`'s instance
/// variables, which are only reachable through an immutable reference.
pub struct Spring {
    target: Cell<f64>,
    start: Cell<f64>,
    elapsed: Cell<f64>,
    settled: Cell<bool>,
}

impl Spring {
    pub fn new(initial: f64) -> Self {
        Self {
            target: Cell::new(initial),
            start: Cell::new(initial),
            elapsed: Cell::new(0.0),
            settled: Cell::new(true),
        }
    }

    /// Retarget the spring. The current value becomes the new start, so an
    /// interrupted animation reverses from wherever the curve currently is.
    pub fn retarget(&self, new_target: f64) {
        self.start.set(self.value_at(0.0));
        self.target.set(new_target);
        self.elapsed.set(0.0);
        self.settled.set(false);
    }

    pub fn target_value(&self) -> f64 {
        self.target.get()
    }

    pub fn settled(&self) -> bool {
        self.settled.get()
    }

    /// Advance by `delta` seconds of wall clock and return the new value.
    pub fn advance(&self, delta: f64) -> f64 {
        if self.settled.get() {
            return self.target.get();
        }

        let elapsed = self.elapsed.get() + delta;
        self.elapsed.set(elapsed);

        let (omega0, zeta) = omegas();
        let omega_d = omega0 * damping_denominator(zeta);
        let decay = exp_decay(zeta * omega0 * elapsed);
        let angle = omega_d * elapsed;

        let amplitude = self.start.get() - self.target.get();
        let value = self.target.get()
            + amplitude * decay * (angle.cos() + zeta * omega0 / omega_d * angle.sin());
        let velocity = -amplitude * (zeta * omega0 + omega_d) * decay * angle.sin();

        if (self.target.get() - value).abs() < SETTLE_EPSILON && velocity.abs() < SETTLE_EPSILON {
            self.settled.set(true);
            return self.target.get();
        }

        value
    }

    /// The value the spring has at the moment it was (re)targeted. Used by
    /// `retarget` so a reversal starts from the current value, not from the
    /// last one we drew.
    fn value_at(&self, t: f64) -> f64 {
        if self.settled.get() {
            return self.target.get();
        }

        let (omega0, zeta) = omegas();
        let omega_d = omega0 * damping_denominator(zeta);
        let amplitude = self.start.get() - self.target.get();

        self.target.get()
            + amplitude
                * exp_decay(zeta * omega0 * t)
                * ((omega_d * t).cos() + zeta * omega0 / omega_d * (omega_d * t).sin())
    }
}

/// `(omega0, zeta)` for the configured response and damping fraction.
fn omegas() -> (f64, f64) {
    (
        2.0 * std::f64::consts::PI / SPRING_RESPONSE,
        SPRING_DAMPING_FRACTION,
    )
}

/// `sqrt(1 - zeta^2)`, the underdamped-spring denominator. Zero when the
/// spring is exactly damped (`dampingFraction >= 1`).
fn damping_denominator(zeta: f64) -> f64 {
    let remaining = 1.0 - zeta * zeta;
    if remaining <= 0.0 {
        0.0
    } else {
        remaining.sqrt()
    }
}

fn exp_decay(x: f64) -> f64 {
    (-x).exp()
}

//! Hand-rolled velocity-based damped spring, matching Apple's
//! `Animation.spring(response:, dampingFraction:)` model.
//!
//! For the configured underdamped spring (`dampingFraction < 1`), each step
//! analytically evolves the current displacement `x = value - target` and
//! velocity `v`:
//!
//! ```text
//! x(t) = e^(-a*t) * (x*cos(b*t) + (v + a*x)/b * sin(b*t))
//! v(t) = e^(-a*t) * (v*cos(b*t) - (a*v + omega0^2*x)/b * sin(b*t))
//! ```
//!
//! Here `omega0 = 2*pi/response`, `a = zeta*omega0`, and
//! `b = omega0*sqrt(1 - zeta^2)`. Retargeting preserves both value and velocity.
//! Callers supply time deltas; no clock is read here. We snap to the target
//! once displacement is below half a logical pixel and speed below 0.5 pt/s.

use std::cell::Cell;

use crate::consts::{SPRING_DAMPING_FRACTION, SPRING_RESPONSE};

const SETTLE_EPSILON: f64 = 0.5;

/// A single-track spring over one scalar (the silhouette height or width).
///
/// All state is in `Cell`s so the spring can live in an `NSView`'s instance
/// variables, which are only reachable through an immutable reference.
pub struct Spring {
    target: Cell<f64>,
    value: Cell<f64>,
    velocity: Cell<f64>,
    settled: Cell<bool>,
}

impl Spring {
    pub fn new(initial: f64) -> Self {
        Self {
            target: Cell::new(initial),
            value: Cell::new(initial),
            velocity: Cell::new(0.0),
            settled: Cell::new(true),
        }
    }

    /// Change the destination without discontinuities in position or velocity.
    /// Repeating the current target leaves the ongoing animation untouched.
    pub fn retarget(&self, new_target: f64) {
        if new_target == self.target.get() {
            return;
        }
        self.target.set(new_target);
        self.settled.set(false);
    }

    /// Immediately settle at `value`, discarding any ongoing motion.
    pub fn snap_to(&self, value: f64) {
        self.target.set(value);
        self.value.set(value);
        self.velocity.set(0.0);
        self.settled.set(true);
    }

    pub fn target_value(&self) -> f64 {
        self.target.get()
    }

    pub fn settled(&self) -> bool {
        self.settled.get()
    }

    /// Advance by `delta` seconds and return the new value.
    /// Zero, negative, and nonfinite deltas leave the entire state unchanged.
    pub fn advance(&self, delta: f64) -> f64 {
        if self.settled.get() || !delta.is_finite() || delta <= 0.0 {
            return self.value.get();
        }

        let (omega0, zeta) = omegas();
        let decay_rate = zeta * omega0;
        let decay = (-decay_rate * delta).exp();
        // A sufficiently long pause exhausts the decay. Check before computing
        // the phase, which can overflow for an otherwise finite, huge delta.
        if decay == 0.0 {
            self.snap_to(self.target.get());
            return self.value.get();
        }

        let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
        let (sin, cos) = (omega_d * delta).sin_cos();
        let displacement = self.value.get() - self.target.get();
        let velocity = self.velocity.get();
        let value = self.target.get()
            + decay * (displacement * cos + (velocity + decay_rate * displacement) / omega_d * sin);
        let velocity = decay
            * (velocity * cos
                - (decay_rate * velocity + omega0 * omega0 * displacement) / omega_d * sin);

        if (self.target.get() - value).abs() < SETTLE_EPSILON && velocity.abs() < SETTLE_EPSILON {
            self.snap_to(self.target.get());
        } else {
            self.value.set(value);
            self.velocity.set(velocity);
        }
        self.value.get()
    }
}

/// `(omega0, zeta)` for the configured response and damping fraction.
fn omegas() -> (f64, f64) {
    (
        2.0 * std::f64::consts::PI / SPRING_RESPONSE,
        SPRING_DAMPING_FRACTION,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, got {actual} (tolerance {tolerance})"
        );
    }

    fn advance_until_settled(spring: &Spring) {
        for _ in 0..600 {
            let value = spring.advance(1.0 / 120.0);
            assert!(value.is_finite());
            assert!(spring.velocity.get().is_finite());
            if spring.settled() {
                assert_eq!(value, spring.target_value());
                assert_eq!(spring.velocity.get(), 0.0);
                return;
            }
        }
        panic!("spring did not settle within five seconds");
    }

    #[test]
    fn initial_state_is_settled() {
        let spring = Spring::new(33.0);
        assert!(spring.settled());
        assert_eq!(spring.target_value(), 33.0);
        assert_eq!(spring.advance(0.0), 33.0);
        assert_eq!(spring.advance(1.0), 33.0);
        assert_eq!(spring.velocity.get(), 0.0);
    }

    #[test]
    fn open_and_close_settle_at_their_targets() {
        let spring = Spring::new(33.0);
        for target in [420.0, 33.0] {
            let previous = spring.advance(0.0);
            spring.retarget(target);
            assert!(!spring.settled());
            assert_eq!(spring.advance(0.0), previous);
            advance_until_settled(&spring);
            assert_eq!(spring.target_value(), target);
        }
    }

    #[test]
    fn midflight_reversal_preserves_position_and_velocity() {
        let spring = Spring::new(33.0);
        spring.retarget(420.0);
        let position = spring.advance(0.1);
        let velocity = spring.velocity.get();
        assert!(position > 33.0 && position < 420.0);
        assert!(velocity > 0.0);

        spring.retarget(33.0);
        assert_eq!(spring.advance(0.0), position);
        assert_eq!(spring.velocity.get(), velocity);
        // Inertia still carries the panel forward before it reverses.
        let next = spring.advance(0.000001);
        assert!(next > position);
        assert_close((next - position) / 0.000001, velocity, 0.2);
        advance_until_settled(&spring);
    }

    #[test]
    fn repeated_target_changes_preserve_motion_and_eventually_settle() {
        let spring = Spring::new(33.0);
        for target in [420.0, 33.0, 300.0, 190.0, 600.0, 33.0] {
            let position = spring.advance(0.0);
            let velocity = spring.velocity.get();
            spring.retarget(target);
            assert_eq!(spring.advance(0.0), position);
            assert_eq!(spring.velocity.get(), velocity);
            assert!(spring.advance(0.04).is_finite());
            assert!(spring.velocity.get().is_finite());
        }
        advance_until_settled(&spring);
        assert_eq!(spring.advance(0.0), 33.0);
    }

    #[test]
    fn same_target_does_not_restart_or_unsettle_the_spring() {
        let spring = Spring::new(33.0);
        let reference = Spring::new(33.0);
        spring.retarget(420.0);
        reference.retarget(420.0);
        spring.advance(0.1);
        reference.advance(0.1);

        for _ in 0..120 {
            let position = spring.advance(0.0);
            let velocity = spring.velocity.get();
            let settled = spring.settled();
            spring.retarget(420.0);
            assert_eq!(spring.advance(0.0), position);
            assert_eq!(spring.velocity.get(), velocity);
            assert_eq!(spring.settled(), settled);
            assert_eq!(spring.advance(1.0 / 60.0), reference.advance(1.0 / 60.0));
            assert_eq!(spring.velocity.get(), reference.velocity.get());
        }
        assert!(spring.settled());
    }

    #[test]
    fn snap_to_immediately_settles_and_clears_velocity() {
        let spring = Spring::new(33.0);
        spring.retarget(420.0);
        spring.advance(0.1);
        assert_ne!(spring.velocity.get(), 0.0);

        spring.snap_to(200.0);
        assert!(spring.settled());
        assert_eq!(spring.target_value(), 200.0);
        assert_eq!(spring.advance(0.0), 200.0);
        assert_eq!(spring.advance(1.0), 200.0);
        assert_eq!(spring.velocity.get(), 0.0);

        spring.retarget(33.0);
        assert_eq!(spring.advance(0.0), 200.0);
        advance_until_settled(&spring);
    }

    #[test]
    fn zero_backwards_and_nonfinite_deltas_do_not_change_state() {
        let spring = Spring::new(33.0);
        let reference = Spring::new(33.0);
        for spring in [&spring, &reference] {
            spring.retarget(420.0);
            spring.advance(0.1);
        }
        let position = spring.advance(0.0);
        let velocity = spring.velocity.get();
        for delta in [
            0.0,
            -0.0,
            -1.0,
            -f64::MAX,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert_eq!(spring.advance(delta), position);
            assert_eq!(spring.velocity.get(), velocity);
            assert_eq!(spring.target_value(), 420.0);
            assert!(!spring.settled());
        }
        assert_eq!(spring.advance(0.05), reference.advance(0.05));
        assert_eq!(spring.velocity.get(), reference.velocity.get());
        advance_until_settled(&spring);
    }

    #[test]
    fn huge_finite_delta_settles_without_nonfinite_phase_math() {
        let spring = Spring::new(33.0);
        spring.retarget(420.0);
        assert_eq!(spring.advance(f64::MAX), 420.0);
        assert!(spring.settled());
        assert_eq!(spring.velocity.get(), 0.0);
    }

    #[test]
    fn velocity_matches_the_position_derivative() {
        let spring = Spring::new(33.0);
        let before = Spring::new(33.0);
        let after = Spring::new(33.0);
        for spring in [&spring, &before, &after] {
            spring.retarget(420.0);
        }
        spring.advance(0.1);
        let derivative = (after.advance(0.100001) - before.advance(0.099999)) / 0.000002;
        assert_close(spring.velocity.get(), derivative, 0.0001);
    }

    #[test]
    fn crossing_target_at_speed_does_not_settle() {
        let spring = Spring::new(33.0);
        spring.retarget(420.0);
        let (omega0, zeta) = omegas();
        let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
        let crossing_time = (std::f64::consts::PI - (omega_d / (zeta * omega0)).atan()) / omega_d;
        assert_close(spring.advance(crossing_time), 420.0, 0.000001);
        assert!(spring.velocity.get().abs() > SETTLE_EPSILON);
        assert!(!spring.settled());
        advance_until_settled(&spring);
    }

    #[test]
    fn analytical_steps_are_consistent_across_frame_sizes() {
        let whole = Spring::new(33.0);
        let split = Spring::new(33.0);
        whole.retarget(420.0);
        split.retarget(420.0);
        let expected = whole.advance(0.15);
        split.advance(0.1);
        assert_close(split.advance(0.05), expected, 0.0000000001);
        assert_close(split.velocity.get(), whole.velocity.get(), 0.00000001);
    }
}

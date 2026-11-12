//! Pure latest-value deadlines. The AppKit adapter owns callbacks, never this state.
use std::rc::Rc;

use crate::settings::{
    EditPhase, Settings, SettingsError, SettingsSnapshot, validate_chord_change,
    validate_toggle_chord,
};

pub struct SettingsState {
    pub current: SettingsSnapshot,
    saved: SettingsSnapshot,
    resize_at: Option<f64>,
    save_at: Option<f64>,
    pub status: Vec<String>,
}

impl SettingsState {
    pub fn new(settings: Settings, status: Vec<String>) -> Self {
        let current = Rc::new(settings);
        Self {
            current: current.clone(),
            saved: current,
            resize_at: None,
            save_at: None,
            status,
        }
    }

    /// Field validation plus the chord policy. A chord change is acceptable only
    /// in [`EditPhase::Commit`] (a preview must never re-register), and it is
    /// checked against the toggle the runtime will actually register — so a
    /// legacy stored chord that cannot be registered does not block an unrelated
    /// edit, while an explicit chord edit still has to be usable and must not
    /// duplicate the toggle.
    pub fn validate_proposal(
        &self,
        proposal: &Settings,
        phase: EditPhase,
        canvas: f64,
    ) -> Result<(), SettingsError> {
        proposal.validate(canvas)?;
        let (effective_toggle, _) = proposal.effective_chords();
        let toggle_changed = proposal.toggle_chord != self.current.toggle_chord;
        let local_changed = proposal.new_tab_chord != self.current.new_tab_chord;
        if (toggle_changed || local_changed) && phase != EditPhase::Commit {
            return Err(SettingsError::Validation(
                "Shortcut edits must be completed before registration".into(),
            ));
        }
        if toggle_changed {
            validate_toggle_chord(proposal.toggle_chord)?;
        }
        validate_chord_change(proposal.new_tab_chord, local_changed, effective_toggle)
    }

    /// Call only after validation and shortcut registration have succeeded.
    pub fn accept(&mut self, proposal: Settings, phase: EditPhase, now: f64) -> bool {
        let changed = *self.current != proposal;
        if changed {
            self.current = Rc::new(proposal);
            // Trailing/latest-value coalescing, not one resize per slider action.
            self.resize_at.get_or_insert(now + 0.085);
            self.save_at = Some(now + 0.300);
        }
        if phase == EditPhase::Commit {
            self.flush(now);
        }
        changed
    }

    pub fn flush(&mut self, now: f64) {
        if self.resize_at.is_some() {
            self.resize_at = Some(now);
        }
        if self.current != self.saved {
            self.save_at = Some(now);
        }
    }

    pub fn resize_due(&mut self, now: f64, open: bool) -> bool {
        if open && self.resize_at.is_some_and(|deadline| deadline <= now) {
            self.resize_at = None;
            true
        } else {
            false
        }
    }

    pub fn require_resize(&mut self, now: f64) {
        self.resize_at = Some(now);
    }

    pub fn save_due(&mut self, now: f64) -> Option<SettingsSnapshot> {
        if self.save_at.is_some_and(|deadline| deadline <= now) {
            self.save_at = None;
            if self.current != self.saved {
                return Some(self.current.clone());
            }
        }
        None
    }

    pub fn saved(&mut self, snapshot: SettingsSnapshot) {
        self.saved = snapshot;
    }
}

/// Displayed geometry is bounded even though the physics can overshoot.
pub fn displayed(value: f64, minimum: f64, maximum: f64) -> f64 {
    if value.is_finite() {
        value.clamp(minimum.min(maximum), maximum)
    } else {
        minimum.min(maximum)
    }
}

/// One pure frame decision shared by drawing and accessibility changes.
pub fn motion_frame(
    height: &crate::animation::Spring,
    width: &crate::animation::Spring,
    delta: f64,
    reduce_motion: bool,
    notch: (f64, f64),
    bounds: (f64, f64),
) -> (f64, f64) {
    if reduce_motion {
        height.snap_to(displayed(height.target_value(), notch.1, bounds.1));
        width.snap_to(displayed(width.target_value(), notch.0, bounds.0));
    }
    (
        displayed(height.advance(delta), notch.1, bounds.1),
        displayed(width.advance(delta), notch.0, bounds.0),
    )
}

pub fn animation_active(height_settled: bool, width_settled: bool) -> bool {
    !height_settled || !width_settled
}

/// Identity-preserving selection remap shared by close and multi-reap.
pub fn selection_after_removal(selected: Option<usize>, removed: &[bool]) -> Option<usize> {
    let survivors = removed.iter().filter(|dead| !**dead).count();
    if survivors == 0 {
        return None;
    }
    let selected = selected.unwrap_or(0).min(removed.len().saturating_sub(1));
    let before = removed.iter().take(selected).filter(|dead| !**dead).count();
    Some(before.min(survivors - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_and_commit_flushes() {
        let mut state = SettingsState::new(Settings::default(), vec![]);
        let mut s = (*state.current).clone();
        s.font_size = 15.0;
        state.accept(s.clone(), EditPhase::Preview, 1.0);
        s.font_size = 16.0;
        state.accept(s.clone(), EditPhase::Preview, 1.05);
        assert!(!state.resize_due(1.08, true));
        assert!(state.resize_due(1.09, true));
        assert!(state.save_due(1.34).is_none());
        state.accept(s, EditPhase::Commit, 1.35);
        let saved = state.save_due(1.35).unwrap();
        assert_eq!(saved.font_size, 16.0);
        state.saved(saved);
        assert!(!state.accept((*state.current).clone(), EditPhase::Commit, 2.0));
        assert!(state.save_due(2.0).is_none());
    }

    #[test]
    fn rejected_proposal_has_no_save_or_resize_effects() {
        let mut state = SettingsState::new(Settings::default(), vec![]);
        let before = state.current.clone();
        let mut proposal = (*before).clone();
        proposal.font_size = f64::NAN;
        assert!(
            state
                .validate_proposal(&proposal, EditPhase::Commit, 1920.0)
                .is_err()
        );
        assert_eq!(state.current, before);
        assert!(state.save_due(1000.0).is_none());
        assert!(!state.resize_due(1000.0, true));
        proposal.font_size = 14.0;
        proposal.new_tab_chord.shift = true;
        assert!(
            state
                .validate_proposal(&proposal, EditPhase::Preview, 1920.0)
                .is_err()
        );
    }

    #[test]
    fn hidden_resize_remains_pending() {
        let mut s = SettingsState::new(Settings::default(), vec![]);
        s.require_resize(0.0);
        assert!(!s.resize_due(1.0, false));
        assert!(s.resize_due(2.0, true));
    }

    #[test]
    fn selection_close_reopen_spawn_reap() {
        assert_eq!(
            selection_after_removal(Some(2), &[true, false, false]),
            Some(1)
        );
        assert_eq!(
            selection_after_removal(Some(1), &[false, true, true]),
            Some(0)
        );
        assert_eq!(selection_after_removal(None, &[false]), Some(0));
        assert_eq!(selection_after_removal(Some(0), &[true]), None);
    }

    #[test]
    fn physical_clamps_and_both_springs() {
        assert_eq!(displayed(-30.0, 33.0, 620.0), 33.0);
        assert_eq!(displayed(3000.0, 189.0, 1920.0), 1920.0);
        assert_eq!(displayed(f64::NAN, 33.0, 620.0), 33.0);
        assert!(animation_active(true, false));
        assert!(!animation_active(true, true));
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    use crate::animation::Spring;

    #[test]
    fn width_only_motion_and_reduce_motion_snap_both() {
        let height = Spring::new(420.0);
        let width = Spring::new(189.0);
        width.retarget(640.0);
        let (_, w) = motion_frame(
            &height,
            &width,
            0.016,
            false,
            (189.0, 33.0),
            (1920.0, 620.0),
        );
        assert!(w > 189.0);
        assert!(animation_active(height.settled(), width.settled()));
        height.retarget(500.0);
        assert_eq!(
            motion_frame(&height, &width, 0.0, true, (189.0, 33.0), (1920.0, 620.0)),
            (500.0, 640.0)
        );
        assert!(height.settled() && width.settled());
    }
}

#![allow(dead_code)]

use crate::geometry::{
    calculate_grab_offset, exceeds_drag_threshold, is_in_interactive_silhouette, GrabOffset, Point,
};

/// States of pointer interaction on the probe window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InteractionState {
    /// No gesture active.
    #[default]
    Idle,
    /// Pending left press: pointer capture acquired, evaluating whether this gesture
    /// is a stationary click or exceeds the threshold.
    LeftPressed {
        press_root: Point,
        press_origin: Point,
        press_time: u32,
        grab_offset: GrabOffset,
    },
    /// A gesture was cancelled or exceeded threshold while a mouse button remains physically depressed.
    /// Suppresses stale release events until the button is released.
    SuppressedUntilRelease { button: u8 },
}

/// Actions requested by the interaction state machine for the host to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAction {
    /// No host action required.
    None,
    /// Acquire short-lived pointer capture via GrabPointer.
    AcquireGrab { time: u32 },
    /// Release pointer capture via UngrabPointer.
    ReleaseGrab { time: u32 },
    /// Release pointer capture and toggle body color (valid click completed).
    ReleaseGrabAndToggleColor { time: u32 },
    /// Clean shutdown requested (e.g. idle right-click).
    ExitCleanly,
}

/// The interaction manager coordinating pointer gesture transitions.
#[derive(Debug, Default)]
pub struct InteractionManager {
    state: InteractionState,
    pending_press: Option<(Point, Point, u32, GrabOffset)>,
}

impl InteractionManager {
    pub fn new() -> Self {
        Self {
            state: InteractionState::Idle,
            pending_press: None,
        }
    }

    pub fn state(&self) -> InteractionState {
        self.state
    }

    /// Handles a left mouse button press on the window.
    pub fn handle_left_press(
        &mut self,
        pointer_root: Point,
        local: (i16, i16),
        time: u32,
        body_origin: Point,
    ) -> HostAction {
        match self.state {
            InteractionState::Idle => {
                if !is_in_interactive_silhouette(local.0, local.1) {
                    return HostAction::None;
                }

                let offset = match calculate_grab_offset(pointer_root, body_origin) {
                    Ok(off) => off,
                    Err(_) => return HostAction::None,
                };

                self.pending_press = Some((pointer_root, body_origin, time, offset));
                HostAction::AcquireGrab { time }
            }
            _ => HostAction::None,
        }
    }

    /// Called when the host successfully acquires pointer capture.
    pub fn on_grab_acquired(&mut self) {
        if let Some((press_root, press_origin, press_time, grab_offset)) = self.pending_press.take()
        {
            self.state = InteractionState::LeftPressed {
                press_root,
                press_origin,
                press_time,
                grab_offset,
            };
        }
    }

    /// Called when the host's grab acquisition request is denied.
    pub fn on_grab_denied(&mut self) {
        self.pending_press = None;
        self.state = InteractionState::Idle;
    }

    /// Handles pointer motion events.
    pub fn handle_motion(&mut self, pointer_root: Point, time: u32) -> HostAction {
        match self.state {
            InteractionState::LeftPressed { press_root, .. } => {
                if exceeds_drag_threshold(press_root, pointer_root) {
                    // At M02.2 checkpoint, exceeding 4px threshold cancels the click without moving.
                    // Full dragging will be introduced in M02.3.
                    self.state = InteractionState::SuppressedUntilRelease { button: 1 };
                    HostAction::ReleaseGrab { time }
                } else {
                    HostAction::None
                }
            }
            _ => HostAction::None,
        }
    }

    /// Handles a left mouse button release.
    pub fn handle_left_release(&mut self, local: (i16, i16), time: u32) -> HostAction {
        match self.state {
            InteractionState::LeftPressed { .. } => {
                self.state = InteractionState::Idle;
                if is_in_interactive_silhouette(local.0, local.1) {
                    HostAction::ReleaseGrabAndToggleColor { time }
                } else {
                    HostAction::ReleaseGrab { time }
                }
            }
            InteractionState::SuppressedUntilRelease { button: 1 } => {
                self.state = InteractionState::Idle;
                HostAction::None
            }
            _ => HostAction::None,
        }
    }

    /// Handles a right mouse button press.
    pub fn handle_right_press(&mut self, time: u32) -> HostAction {
        match self.state {
            InteractionState::LeftPressed { .. } => {
                // Right press during an active left gesture cancels the gesture
                self.state = InteractionState::SuppressedUntilRelease { button: 1 };
                HostAction::ReleaseGrab { time }
            }
            InteractionState::Idle => {
                // While idle, right press triggers clean exit (M01 baseline preserved)
                HostAction::ExitCleanly
            }
            InteractionState::SuppressedUntilRelease { .. } => HostAction::None,
        }
    }

    /// Handles unrelated button presses (e.g. middle click, scroll wheel).
    pub fn handle_other_button_press(&mut self, _button: u8, _time: u32) -> HostAction {
        // Ignored; does not interrupt active gestures or trigger actions.
        HostAction::None
    }

    /// Handles unrelated button releases.
    pub fn handle_other_button_release(&mut self, _button: u8, _time: u32) -> HostAction {
        // Ignored; releasing other buttons does not end the left gesture.
        HostAction::None
    }

    /// Cancels any active gesture and resets to Idle (e.g. on timeout or window destroy).
    pub fn cancel(&mut self, time: u32) -> HostAction {
        self.pending_press = None;
        match self.state {
            InteractionState::LeftPressed { .. } => {
                self.state = InteractionState::Idle;
                HostAction::ReleaseGrab { time }
            }
            _ => {
                self.state = InteractionState::Idle;
                HostAction::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_click_lifecycle() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);
        let center_local = (80, 80);

        // 1. Press on body silhouette requests grab acquisition
        let action = mgr.handle_left_press(root, center_local, 1000, origin);
        assert_eq!(action, HostAction::AcquireGrab { time: 1000 });

        // 2. Grab acquired enters LeftPressed
        mgr.on_grab_acquired();
        assert!(matches!(mgr.state(), InteractionState::LeftPressed { .. }));

        // 3. Jitter motion under 4px produces no action
        let action = mgr.handle_motion(Point::new(101, 101), 1010);
        assert_eq!(action, HostAction::None);

        // 4. Release still on body silhouette releases grab and toggles color
        let action = mgr.handle_left_release(center_local, 1020);
        assert_eq!(action, HostAction::ReleaseGrabAndToggleColor { time: 1020 });
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_release_outside_body_cancels_toggle() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);
        let center_local = (80, 80);

        // Press
        assert_eq!(
            mgr.handle_left_press(root, center_local, 1000, origin),
            HostAction::AcquireGrab { time: 1000 }
        );
        mgr.on_grab_acquired();

        // Release at transparent padding / outside shape (e.g. local (5, 5))
        let action = mgr.handle_left_release((5, 5), 1020);
        assert_eq!(action, HostAction::ReleaseGrab { time: 1020 });
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_exceeding_threshold_cancels_click() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        // Press
        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // Motion exceeding 4px threshold
        let action = mgr.handle_motion(Point::new(105, 100), 1010);
        assert_eq!(action, HostAction::ReleaseGrab { time: 1010 });
        assert_eq!(
            mgr.state(),
            InteractionState::SuppressedUntilRelease { button: 1 }
        );

        // Subsequent release produces no toggle or ungrab
        let action = mgr.handle_left_release((80, 80), 1020);
        assert_eq!(action, HostAction::None);
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_right_press_cancels_active_left_gesture() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        // Press
        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // Right press during left gesture cancels gesture and releases grab
        let action = mgr.handle_right_press(1010);
        assert_eq!(action, HostAction::ReleaseGrab { time: 1010 });
        assert_eq!(
            mgr.state(),
            InteractionState::SuppressedUntilRelease { button: 1 }
        );

        // Late left release produces no toggle
        let action = mgr.handle_left_release((80, 80), 1020);
        assert_eq!(action, HostAction::None);
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_idle_right_press_exits_cleanly() {
        let mut mgr = InteractionManager::new();
        assert_eq!(mgr.state(), InteractionState::Idle);
        assert_eq!(mgr.handle_right_press(1000), HostAction::ExitCleanly);
    }

    #[test]
    fn test_denied_grab_recovers_to_idle() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        let action = mgr.handle_left_press(root, (80, 80), 1000, origin);
        assert_eq!(action, HostAction::AcquireGrab { time: 1000 });

        mgr.on_grab_denied();
        assert_eq!(mgr.state(), InteractionState::Idle);

        // Can press again cleanly
        let action2 = mgr.handle_left_press(root, (80, 80), 1010, origin);
        assert_eq!(action2, HostAction::AcquireGrab { time: 1010 });
    }

    #[test]
    fn test_unrelated_buttons_ignored() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // Middle button press and release
        assert_eq!(mgr.handle_other_button_press(2, 1010), HostAction::None);
        assert_eq!(mgr.handle_other_button_release(2, 1015), HostAction::None);

        // Scroll wheel press
        assert_eq!(mgr.handle_other_button_press(4, 1020), HostAction::None);
        assert_eq!(mgr.handle_other_button_press(5, 1025), HostAction::None);

        // Left gesture still active!
        assert!(matches!(mgr.state(), InteractionState::LeftPressed { .. }));

        // Left release still works cleanly
        let action = mgr.handle_left_release((80, 80), 1030);
        assert_eq!(action, HostAction::ReleaseGrabAndToggleColor { time: 1030 });
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_press_outside_silhouette_ignored() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        // Local (5, 5) is transparent padding
        let action = mgr.handle_left_press(root, (5, 5), 1000, origin);
        assert_eq!(action, HostAction::None);
        assert_eq!(mgr.state(), InteractionState::Idle);
    }
}

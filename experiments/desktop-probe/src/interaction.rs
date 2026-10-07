#![allow(dead_code)]

use crate::geometry::{
    calculate_grab_offset, calculate_target_origin, exceeds_drag_threshold,
    is_in_interactive_silhouette, GrabOffset, Point, ValidOriginBounds,
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
    /// The gesture exceeded the 4-pixel drag threshold and is actively dragging the window.
    Dragging {
        press_root: Point,
        press_time: u32,
        grab_offset: GrabOffset,
        current_target: Point,
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
    /// Release pointer capture and toggle body color (valid stationary click completed).
    ReleaseGrabAndToggleColor { time: u32 },
    /// Move the window to the clamped target position.
    MoveWindow { target: Point },
    /// Apply final window placement and release pointer capture (drag completed).
    ReleaseGrabAndMoveWindow { time: u32, target: Point },
    /// Clean shutdown requested (e.g. idle right-click).
    ExitCleanly,
}

/// The interaction manager coordinating pointer gesture transitions.
#[derive(Debug, Default)]
pub struct InteractionManager {
    state: InteractionState,
    pending_press: Option<(Point, Point, u32, GrabOffset)>,
    freshness_pending: bool,
}

impl InteractionManager {
    pub fn new() -> Self {
        Self {
            state: InteractionState::Idle,
            pending_press: None,
            freshness_pending: false,
        }
    }

    pub fn state(&self) -> InteractionState {
        self.state
    }

    pub fn is_dragging(&self) -> bool {
        matches!(self.state, InteractionState::Dragging { .. })
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.state, InteractionState::Idle)
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
            InteractionState::Idle | InteractionState::SuppressedUntilRelease { .. } => {
                if !is_in_interactive_silhouette(local.0, local.1) {
                    self.state = InteractionState::Idle;
                    return HostAction::None;
                }

                let offset = match calculate_grab_offset(pointer_root, body_origin) {
                    Ok(off) => off,
                    Err(_) => {
                        self.state = InteractionState::Idle;
                        return HostAction::None;
                    }
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
            self.freshness_pending = true;
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
    pub fn handle_motion(
        &mut self,
        pointer_root: Point,
        time: u32,
        bounds: &ValidOriginBounds,
    ) -> HostAction {
        if let InteractionState::LeftPressed { press_time, .. }
        | InteractionState::Dragging { press_time, .. } = self.state
        {
            if self.freshness_pending && !timestamp_at_or_after(time, press_time) {
                return HostAction::None;
            }
        }
        match self.state {
            InteractionState::LeftPressed {
                press_root,
                press_time,
                grab_offset,
                press_origin,
            } => {
                if exceeds_drag_threshold(press_root, pointer_root) {
                    // Threshold reached (>= 4px displacement): promote to Dragging
                    let unconstrained =
                        calculate_target_origin(pointer_root, grab_offset).unwrap_or(press_origin);
                    let target = bounds.clamp(unconstrained);
                    self.state = InteractionState::Dragging {
                        press_root,
                        press_time,
                        grab_offset,
                        current_target: target,
                    };
                    HostAction::MoveWindow { target }
                } else {
                    HostAction::None
                }
            }
            InteractionState::Dragging {
                press_root,
                press_time,
                grab_offset,
                ref mut current_target,
            } => {
                let unconstrained =
                    calculate_target_origin(pointer_root, grab_offset).unwrap_or(*current_target);
                let target = bounds.clamp(unconstrained);
                if target != *current_target {
                    *current_target = target;
                    self.state = InteractionState::Dragging {
                        press_root,
                        press_time,
                        grab_offset,
                        current_target: target,
                    };
                    HostAction::MoveWindow { target }
                } else {
                    HostAction::None
                }
            }
            _ => HostAction::None,
        }
    }

    /// Handles a batch of pointer motion events, coalescing updates while ensuring
    /// threshold promotion is reliably detected.
    ///
    /// If any motion in the batch crosses the threshold while in LeftPressed, the gesture
    /// is promoted to Dragging. The returned HostAction reflects the latest position in the batch.
    pub fn handle_motion_batch(
        &mut self,
        motions: impl IntoIterator<Item = (Point, u32)>,
        bounds: &ValidOriginBounds,
    ) -> HostAction {
        let mut final_action = HostAction::None;
        for (pt, time) in motions {
            let action = self.handle_motion(pt, time, bounds);
            if action != HostAction::None {
                final_action = action;
            }
        }
        final_action
    }

    /// Handles a left mouse button release.
    pub fn handle_left_release(
        &mut self,
        pointer_root: Point,
        local: (i16, i16),
        time: u32,
        bounds: &ValidOriginBounds,
    ) -> HostAction {
        if let InteractionState::LeftPressed { press_time, .. }
        | InteractionState::Dragging { press_time, .. } = self.state
        {
            if self.freshness_pending && !timestamp_at_or_after(time, press_time) {
                return HostAction::None;
            }
        }
        match self.state {
            InteractionState::LeftPressed {
                press_root,
                press_origin,
                grab_offset,
                ..
            } => {
                self.state = InteractionState::Idle;
                if exceeds_drag_threshold(press_root, pointer_root) {
                    // Fast drag completed without intervening motion events:
                    // Complete the drag using the clamped release target, do NOT toggle color!
                    let unconstrained =
                        calculate_target_origin(pointer_root, grab_offset).unwrap_or(press_origin);
                    let final_target = bounds.clamp(unconstrained);
                    HostAction::ReleaseGrabAndMoveWindow {
                        time,
                        target: final_target,
                    }
                } else if is_in_interactive_silhouette(local.0, local.1) {
                    HostAction::ReleaseGrabAndToggleColor { time }
                } else {
                    HostAction::ReleaseGrab { time }
                }
            }
            InteractionState::Dragging {
                grab_offset,
                current_target,
                ..
            } => {
                self.state = InteractionState::Idle;
                let unconstrained =
                    calculate_target_origin(pointer_root, grab_offset).unwrap_or(current_target);
                let final_target = bounds.clamp(unconstrained);
                HostAction::ReleaseGrabAndMoveWindow {
                    time,
                    target: final_target,
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
            InteractionState::LeftPressed { .. } | InteractionState::Dragging { .. } => {
                // Right press during an active left gesture cancels the gesture
                self.cancel(time)
            }
            InteractionState::Idle | InteractionState::SuppressedUntilRelease { .. } => {
                // While idle or recovering from missed release, right press triggers clean exit
                self.state = InteractionState::Idle;
                HostAction::ExitCleanly
            }
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

    /// Cancels without completing a click; a fresh press can recover after a missed release.
    pub fn cancel(&mut self, time: u32) -> HostAction {
        self.pending_press = None;
        if self.has_left_gesture() {
            self.state = InteractionState::SuppressedUntilRelease { button: 1 };
            HostAction::ReleaseGrab { time }
        } else {
            HostAction::None
        }
    }

    pub fn accepts_gesture_time(&self, time: u32) -> bool {
        match self.state {
            InteractionState::LeftPressed { press_time, .. }
            | InteractionState::Dragging { press_time, .. } => {
                !self.freshness_pending || timestamp_at_or_after(time, press_time)
            }
            _ => true,
        }
    }

    /// A held-button observation has passed the queued-event boundary. Old events
    /// were consumed before it; the temporary timestamp guard can now end. This
    /// also permits arbitrarily long holds across X timestamp wrap.
    pub fn confirm_button_held(&mut self) {
        self.freshness_pending = false;
    }

    pub fn has_left_gesture(&self) -> bool {
        matches!(
            self.state,
            InteractionState::LeftPressed { .. } | InteractionState::Dragging { .. }
        )
    }

    pub fn cancel_on_wm_mismatch(&mut self, time: u32) -> HostAction {
        self.cancel(time)
    }
}

// X timestamps wrap at 32 bits. The temporary freshness guard ends at the first
// ordered held-button observation, rather than limiting the lifetime of a hold.
fn timestamp_at_or_after(time: u32, earlier: u32) -> bool {
    time.wrapping_sub(earlier) as i32 >= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bounds() -> ValidOriginBounds {
        ValidOriginBounds::new(10, 1000, 20, 800)
    }

    #[test]
    fn test_valid_click_lifecycle() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
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
        let action = mgr.handle_motion(Point::new(101, 101), 1010, &bounds);
        assert_eq!(action, HostAction::None);

        // 4. Release still on body silhouette releases grab and toggles color
        let action = mgr.handle_left_release(Point::new(101, 101), center_local, 1020, &bounds);
        assert_eq!(action, HostAction::ReleaseGrabAndToggleColor { time: 1020 });
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_release_outside_body_cancels_toggle() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
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
        let action = mgr.handle_left_release(root, (5, 5), 1020, &bounds);
        assert_eq!(action, HostAction::ReleaseGrab { time: 1020 });
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_left_press_promoted_to_dragging_on_threshold() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        // Press: pointer at (100, 100), origin at (80, 80) -> grab_offset is dx=20, dy=20
        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // Motion exceeding 4px threshold: pointer to (105, 100) (dx=5 >= 4)
        // Target is pointer (105, 100) - offset (20, 20) = (85, 80)
        let action = mgr.handle_motion(Point::new(105, 100), 1010, &bounds);
        assert_eq!(
            action,
            HostAction::MoveWindow {
                target: Point::new(85, 80)
            }
        );
        assert!(mgr.is_dragging());
        assert_eq!(
            mgr.state(),
            InteractionState::Dragging {
                press_root: root,
                press_time: 1000,
                grab_offset: GrabOffset::new(20, 20),
                current_target: Point::new(85, 80),
            }
        );
    }

    #[test]
    fn test_dragging_subsequent_motion_moves_window() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();
        mgr.handle_motion(Point::new(105, 100), 1010, &bounds);

        // Subsequent motion to (200, 150) -> target = (200 - 20, 150 - 20) = (180, 130)
        let action = mgr.handle_motion(Point::new(200, 150), 1020, &bounds);
        assert_eq!(
            action,
            HostAction::MoveWindow {
                target: Point::new(180, 130)
            }
        );
        assert_eq!(
            mgr.state(),
            InteractionState::Dragging {
                press_root: root,
                press_time: 1000,
                grab_offset: GrabOffset::new(20, 20),
                current_target: Point::new(180, 130),
            }
        );

        // Repeated motion to same point emits None (no redundant configure requests)
        let action_repeat = mgr.handle_motion(Point::new(200, 150), 1025, &bounds);
        assert_eq!(action_repeat, HostAction::None);
    }

    #[test]
    fn test_dragging_motion_clamped_at_boundaries() {
        let mut mgr = InteractionManager::new();
        let bounds = ValidOriginBounds::new(10, 500, 20, 400);
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // Move far past top-left boundary (-500, -500)
        let action = mgr.handle_motion(Point::new(-500, -500), 1010, &bounds);
        assert_eq!(
            action,
            HostAction::MoveWindow {
                target: Point::new(10, 20)
            }
        );

        // Moving even farther negative while already clamped at (10, 20) emits None
        let action2 = mgr.handle_motion(Point::new(-600, -600), 1020, &bounds);
        assert_eq!(action2, HostAction::None);

        // Move far past bottom-right boundary (2000, 2000)
        let action3 = mgr.handle_motion(Point::new(2000, 2000), 1030, &bounds);
        assert_eq!(
            action3,
            HostAction::MoveWindow {
                target: Point::new(500, 400)
            }
        );
    }

    #[test]
    fn test_dragging_release_emits_final_move_and_ungrab_without_color_toggle() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();
        mgr.handle_motion(Point::new(105, 100), 1010, &bounds);

        // Release at (150, 120), local coordinates arbitrary / outside original shape
        let action = mgr.handle_left_release(Point::new(150, 120), (-100, -100), 1020, &bounds);
        // target = (150 - 20, 120 - 20) = (130, 100)
        assert_eq!(
            action,
            HostAction::ReleaseGrabAndMoveWindow {
                time: 1020,
                target: Point::new(130, 100),
            }
        );
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_drag_returned_to_start_never_clicks() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // 1. Drag away to (150, 150)
        mgr.handle_motion(Point::new(150, 150), 1010, &bounds);
        assert!(mgr.is_dragging());

        // 2. Drag back to exact press point (100, 100)
        let action = mgr.handle_motion(root, 1020, &bounds);
        assert_eq!(
            action,
            HostAction::MoveWindow {
                target: Point::new(80, 80)
            }
        );
        assert!(mgr.is_dragging());

        // 3. Release at exact press point on the center of the silhouette
        let release_action = mgr.handle_left_release(root, (80, 80), 1030, &bounds);
        // Must complete as a drag, NOT toggle color!
        assert_eq!(
            release_action,
            HostAction::ReleaseGrabAndMoveWindow {
                time: 1030,
                target: Point::new(80, 80),
            }
        );
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_right_press_cancels_active_left_gesture() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);
        let bounds = sample_bounds();

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
        let action = mgr.handle_left_release(root, (80, 80), 1020, &bounds);
        assert_eq!(action, HostAction::None);
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_right_press_cancels_dragging() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);
        let bounds = sample_bounds();

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();
        mgr.handle_motion(Point::new(120, 120), 1010, &bounds);
        assert!(mgr.is_dragging());

        // Right press during active drag cancels drag and releases grab
        let action = mgr.handle_right_press(1020);
        assert_eq!(action, HostAction::ReleaseGrab { time: 1020 });
        assert_eq!(
            mgr.state(),
            InteractionState::SuppressedUntilRelease { button: 1 }
        );

        // Subsequent left release is suppressed
        let release_action = mgr.handle_left_release(Point::new(120, 120), (80, 80), 1030, &bounds);
        assert_eq!(release_action, HostAction::None);
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
        let bounds = sample_bounds();
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
        let action = mgr.handle_left_release(root, (80, 80), 1030, &bounds);
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

    #[test]
    fn test_fast_drag_without_motion_events_completes_drag() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let root_press = Point::new(160, 160);
        let origin = Point::new(80, 80);

        // Press at center (local 80, 80) -> grab offset is root - origin = (80, 80)
        mgr.handle_left_press(root_press, (80, 80), 1000, origin);
        mgr.on_grab_acquired();
        assert!(matches!(mgr.state(), InteractionState::LeftPressed { .. }));

        // Fast swipe release 20px away without intervening motion event.
        // Release root is (180, 160) (dx=20 >= 4px threshold).
        // Release local is (100, 80) which is still on the circular body (dist=20 <= 45).
        // Target origin is root - offset = (180 - 80, 160 - 80) = (100, 80).
        // Must complete as a drag to clamped target (100, 80), NOT toggle color!
        let release_action =
            mgr.handle_left_release(Point::new(180, 160), (100, 80), 1010, &bounds);
        assert_eq!(
            release_action,
            HostAction::ReleaseGrabAndMoveWindow {
                time: 1010,
                target: Point::new(100, 80),
            }
        );
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_suppressed_state_recovers_on_fresh_left_press() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        // 1. Start gesture
        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // 2. Right click cancels the gesture and releases capture
        let cancel_action = mgr.handle_right_press(1010);
        assert_eq!(cancel_action, HostAction::ReleaseGrab { time: 1010 });
        assert_eq!(
            mgr.state(),
            InteractionState::SuppressedUntilRelease { button: 1 }
        );

        // 3. User releases button 1 outside our window over another application
        // (probe receives NO button release event, so probe is still in SuppressedUntilRelease)

        // 4. User brings cursor back and initiates a fresh left press
        let fresh_action = mgr.handle_left_press(root, (80, 80), 1050, origin);
        assert_eq!(fresh_action, HostAction::AcquireGrab { time: 1050 });

        // 5. Acquisition succeeds and a new click/drag cycle proceeds normally
        mgr.on_grab_acquired();
        assert!(matches!(mgr.state(), InteractionState::LeftPressed { .. }));

        let click_action = mgr.handle_left_release(root, (80, 80), 1060, &bounds);
        assert_eq!(
            click_action,
            HostAction::ReleaseGrabAndToggleColor { time: 1060 }
        );
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_suppressed_state_recovers_on_fresh_right_press() {
        let mut mgr = InteractionManager::new();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        // Start and cancel gesture
        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();
        mgr.handle_right_press(1010);
        assert_eq!(
            mgr.state(),
            InteractionState::SuppressedUntilRelease { button: 1 }
        );

        // Fresh right press exits cleanly
        let right_action = mgr.handle_right_press(1050);
        assert_eq!(right_action, HostAction::ExitCleanly);
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_motion_batch_promotes_to_dragging_on_intermediate_threshold() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // Batch: 1st motion jitter within threshold (101, 100),
        // 2nd motion crosses threshold (105, 100) -> promotes to dragging,
        // 3rd motion travels further (120, 110).
        let motions = vec![
            (Point::new(101, 100), 1005),
            (Point::new(105, 100), 1010),
            (Point::new(120, 110), 1015),
        ];

        let action = mgr.handle_motion_batch(motions, &bounds);
        // Target for final point (120, 110) with offset (20, 20) is (100, 90)
        assert_eq!(
            action,
            HostAction::MoveWindow {
                target: Point::new(100, 90)
            }
        );
        assert!(mgr.is_dragging());
    }

    #[test]
    fn test_motion_batch_within_threshold_stays_left_pressed() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // Batch of small jitter motions all within 4px of (100, 100)
        let motions = vec![
            (Point::new(101, 100), 1005),
            (Point::new(102, 101), 1010),
            (Point::new(100, 102), 1015),
        ];

        let action = mgr.handle_motion_batch(motions, &bounds);
        assert_eq!(action, HostAction::None);
        assert!(matches!(mgr.state(), InteractionState::LeftPressed { .. }));
    }

    #[test]
    fn test_motion_batch_crossing_and_returning_preserves_dragging() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let root = Point::new(100, 100);
        let origin = Point::new(80, 80);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // Batch crosses threshold (150, 150) then returns to exact press point (100, 100)
        let motions = vec![(Point::new(150, 150), 1010), (Point::new(100, 100), 1020)];

        let action = mgr.handle_motion_batch(motions, &bounds);
        assert_eq!(
            action,
            HostAction::MoveWindow {
                target: Point::new(80, 80)
            }
        );
        assert!(mgr.is_dragging());

        // Release at press point completes as a drag, NOT a click toggle
        let release_action = mgr.handle_left_release(root, (80, 80), 1030, &bounds);
        assert_eq!(
            release_action,
            HostAction::ReleaseGrabAndMoveWindow {
                time: 1030,
                target: Point::new(80, 80),
            }
        );
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_cancel_on_wm_mismatch_from_dragging_suppresses_release() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let origin = Point::new(80, 80);
        let root = Point::new(100, 100);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // Cross threshold to enter Dragging
        let action = mgr.handle_motion(Point::new(120, 100), 1010, &bounds);
        assert!(matches!(action, HostAction::MoveWindow { .. }));
        assert!(mgr.is_dragging());

        // WM mismatch triggers cancellation
        let cancel_action = mgr.cancel_on_wm_mismatch(1020);
        assert_eq!(cancel_action, HostAction::ReleaseGrab { time: 1020 });
        assert_eq!(
            mgr.state(),
            InteractionState::SuppressedUntilRelease { button: 1 }
        );

        // Eventual release over window does NOT trigger color toggle or move window
        let release_action = mgr.handle_left_release(Point::new(120, 100), (80, 80), 1030, &bounds);
        assert_eq!(release_action, HostAction::None);
        assert_eq!(mgr.state(), InteractionState::Idle);
    }

    #[test]
    fn test_cancel_on_wm_mismatch_from_left_pressed() {
        let mut mgr = InteractionManager::new();
        let bounds = sample_bounds();
        let origin = Point::new(80, 80);
        let root = Point::new(100, 100);

        mgr.handle_left_press(root, (80, 80), 1000, origin);
        mgr.on_grab_acquired();

        // While still in LeftPressed (e.g. before motion or before threshold)
        let cancel_action = mgr.cancel_on_wm_mismatch(1010);
        assert_eq!(cancel_action, HostAction::ReleaseGrab { time: 1010 });
        assert_eq!(
            mgr.state(),
            InteractionState::SuppressedUntilRelease { button: 1 }
        );

        // Eventual release does not toggle color
        let release_action = mgr.handle_left_release(root, (80, 80), 1020, &bounds);
        assert_eq!(release_action, HostAction::None);
        assert_eq!(mgr.state(), InteractionState::Idle);
    }
    #[test]
    fn stale_events_cannot_complete_new_gesture_including_timestamp_wrap() {
        let mut manager = InteractionManager::new();
        let bounds = sample_bounds();
        let point = Point::new(100, 100);
        manager.handle_left_press(point, (80, 80), u32::MAX - 10, point);
        manager.on_grab_acquired();
        manager.cancel(0);
        manager.handle_left_press(point, (80, 80), 5, point);
        manager.on_grab_acquired();
        assert_eq!(
            manager.handle_left_release(point, (80, 80), u32::MAX - 1, &bounds),
            HostAction::None
        );
        assert_eq!(
            manager.handle_motion(Point::new(200, 200), u32::MAX - 1, &bounds),
            HostAction::None
        );
        assert!(!manager.accepts_gesture_time(u32::MAX - 1));
        assert!(manager.has_left_gesture());
        assert_eq!(
            manager.handle_left_release(point, (80, 80), 6, &bounds),
            HostAction::ReleaseGrabAndToggleColor { time: 6 }
        );
    }
    #[test]
    fn confirmed_stationary_hold_can_release_after_half_timestamp_period() {
        let mut manager = InteractionManager::new();
        let point = Point::new(100, 100);
        manager.handle_left_press(point, (80, 80), 1000, point);
        manager.on_grab_acquired();
        manager.confirm_button_held();
        let late = 1000u32.wrapping_add((i32::MAX as u32) + 1);
        assert!(manager.accepts_gesture_time(late));
        assert_eq!(
            manager.handle_left_release(point, (80, 80), late, &sample_bounds()),
            HostAction::ReleaseGrabAndToggleColor { time: late }
        );
    }
}

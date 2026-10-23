#![allow(dead_code)]

use crate::geometry::{
    calculate_grab_offset, calculate_target_origin, exceeds_drag_threshold,
    is_in_interactive_silhouette, GrabOffset, MenuHit, Point, ValidOriginBounds,
};
use crate::layer::Layer;

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
    /// The server's automatic right-button grab is pending completion.
    RightPressed { press_time: u32, chorded: bool },
    MenuOpen {
        hover: Option<MenuItem>,
        gesture: MenuGesture,
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
    AcquireGrab {
        time: u32,
    },
    /// Release pointer capture via UngrabPointer.
    ReleaseGrab {
        time: u32,
    },
    /// Release pointer capture and toggle body color (valid stationary click completed).
    ReleaseGrabAndToggleColor {
        time: u32,
    },
    /// Move the window to the clamped target position.
    MoveWindow {
        target: Point,
    },
    /// Apply final window placement and release pointer capture (drag completed).
    ReleaseGrabAndMoveWindow {
        time: u32,
        target: Point,
    },
    TrackOpening,
    /// All buttons are up: automatic ownership ends without an explicit ungrab.
    OpeningFinished {
        time: u32,
        anchor: Option<Point>,
    },
    CloseMenu {
        time: u32,
        outcome: MenuOutcome,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Above,
    Normal,
    Below,
    Dismiss,
    Quit,
}
impl MenuItem {
    pub const ALL: [Self; 5] = [
        Self::Above,
        Self::Normal,
        Self::Below,
        Self::Dismiss,
        Self::Quit,
    ];

    pub fn label(self) -> &'static [u8] {
        match self {
            Self::Above => b"Above",
            Self::Normal => b"Normal",
            Self::Below => b"Below",
            Self::Dismiss => b"Dismiss",
            Self::Quit => b"Quit",
        }
    }

    pub fn outcome(self) -> MenuOutcome {
        match self {
            Self::Above => MenuOutcome::SetLayer(Layer::Above),
            Self::Normal => MenuOutcome::SetLayer(Layer::Normal),
            Self::Below => MenuOutcome::SetLayer(Layer::Below),
            Self::Dismiss => MenuOutcome::Dismiss,
            Self::Quit => MenuOutcome::Quit,
        }
    }

    pub fn from_hit(hit: MenuHit) -> Option<Self> {
        match hit {
            MenuHit::Row(row) => Self::ALL.get(row).copied(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuOutcome {
    Dismiss,
    Quit,
    SetLayer(Layer),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuGesture {
    #[default]
    None,
    ItemPressed {
        item: MenuItem,
        press_time: u32,
    },
    OutsidePressed {
        press_time: u32,
    },
    RightDismissPressed {
        press_time: u32,
    },
    /// A chord can only return to neutral when all buttons are up.
    Suppressed {
        press_time: u32,
    },
}
impl MenuGesture {
    fn press_time(self) -> Option<u32> {
        match self {
            Self::None => None,
            Self::ItemPressed { press_time, .. }
            | Self::OutsidePressed { press_time }
            | Self::RightDismissPressed { press_time }
            | Self::Suppressed { press_time } => Some(press_time),
        }
    }
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

    /// Host validates the body hit and an unchorded genuine press first.
    pub fn handle_right_press(&mut self, time: u32) -> HostAction {
        match self.state {
            InteractionState::LeftPressed { .. } | InteractionState::Dragging { .. } => {
                self.cancel(time)
            }
            InteractionState::Idle | InteractionState::SuppressedUntilRelease { .. } => {
                self.freshness_pending = true;
                self.state = InteractionState::RightPressed {
                    press_time: time,
                    chorded: false,
                };
                HostAction::TrackOpening
            }
            _ => HostAction::None,
        }
    }

    pub fn opening_chord(&mut self, time: u32) {
        if self.accepts_gesture_time(time) {
            if let InteractionState::RightPressed {
                ref mut chorded, ..
            } = self.state
            {
                *chorded = true;
            }
        }
    }

    /// remaining_buttons describes the post-release logical button state.
    pub fn handle_opening_release(
        &mut self,
        button: u8,
        time: u32,
        remaining_buttons: u16,
        anchor: Point,
        on_body: bool,
    ) -> HostAction {
        if !self.accepts_gesture_time(time) {
            return HostAction::None;
        }
        let InteractionState::RightPressed {
            press_time,
            chorded,
        } = self.state
        else {
            return HostAction::None;
        };
        if button != 3 && !chorded {
            return HostAction::None;
        }
        if remaining_buttons != 0 {
            self.state = InteractionState::RightPressed {
                press_time,
                chorded: true,
            };
            return HostAction::None;
        }
        self.state = InteractionState::Idle;
        self.freshness_pending = false;
        HostAction::OpeningFinished {
            time,
            anchor: (button == 3 && !chorded && on_body).then_some(anchor),
        }
    }

    pub fn menu_acquired(&mut self, hit: MenuHit) {
        self.pending_press = None;
        self.freshness_pending = false;
        self.state = InteractionState::MenuOpen {
            hover: MenuItem::from_hit(hit),
            gesture: MenuGesture::None,
        };
    }

    pub fn menu_motion(&mut self, hit: MenuHit, time: u32) {
        if !self.accepts_gesture_time(time) {
            return;
        }
        if let InteractionState::MenuOpen { ref mut hover, .. } = self.state {
            *hover = MenuItem::from_hit(hit);
        }
    }

    pub fn menu_press(&mut self, button: u8, hit: MenuHit, time: u32, held_buttons: u16) {
        if !self.accepts_gesture_time(time) {
            return;
        }
        let InteractionState::MenuOpen { hover, gesture } = self.state else {
            return;
        };
        let gesture = if gesture != MenuGesture::None || held_buttons != 0 {
            MenuGesture::Suppressed {
                press_time: gesture.press_time().unwrap_or(time),
            }
        } else {
            match (button, hit) {
                (1, MenuHit::Outside) => MenuGesture::OutsidePressed { press_time: time },
                (1, _) => MenuItem::from_hit(hit).map_or(MenuGesture::None, |item| {
                    MenuGesture::ItemPressed {
                        item,
                        press_time: time,
                    }
                }),
                (3, _) => MenuGesture::RightDismissPressed { press_time: time },
                _ => MenuGesture::None,
            }
        };
        if gesture != MenuGesture::None && self.active_button().is_none() {
            self.freshness_pending = true;
        }
        self.state = InteractionState::MenuOpen { hover, gesture };
    }

    pub fn menu_release(
        &mut self,
        button: u8,
        hit: MenuHit,
        time: u32,
        remaining_buttons: u16,
    ) -> HostAction {
        if !self.accepts_gesture_time(time) {
            return HostAction::None;
        }
        let InteractionState::MenuOpen { hover, gesture } = self.state else {
            return HostAction::None;
        };
        if gesture == MenuGesture::None {
            return HostAction::None;
        }
        if remaining_buttons != 0 {
            self.state = InteractionState::MenuOpen {
                hover,
                gesture: MenuGesture::Suppressed {
                    press_time: gesture.press_time().unwrap(),
                },
            };
            return HostAction::None;
        }
        // All buttons are up. A suppressed or mismatched gesture never activates.
        self.state = InteractionState::MenuOpen {
            hover,
            gesture: MenuGesture::None,
        };
        self.freshness_pending = false;
        let outcome = match gesture {
            MenuGesture::ItemPressed { item, .. }
                if button == 1 && MenuItem::from_hit(hit) == Some(item) =>
            {
                item.outcome()
            }
            MenuGesture::OutsidePressed { .. } if button == 1 => MenuOutcome::Dismiss,
            MenuGesture::RightDismissPressed { .. } if button == 3 => MenuOutcome::Dismiss,
            _ => return HostAction::None,
        };
        HostAction::CloseMenu { time, outcome }
    }

    /// None means no safety timer; zero means observe any held button of a chord.
    pub fn active_button(&self) -> Option<u8> {
        match self.state {
            InteractionState::LeftPressed { .. } | InteractionState::Dragging { .. } => Some(1),
            InteractionState::RightPressed { chorded, .. } => Some(if chorded { 0 } else { 3 }),
            InteractionState::MenuOpen { gesture, .. } => match gesture {
                MenuGesture::None => None,
                MenuGesture::ItemPressed { .. } | MenuGesture::OutsidePressed { .. } => Some(1),
                MenuGesture::RightDismissPressed { .. } => Some(3),
                MenuGesture::Suppressed { .. } => Some(0),
            },
            _ => None,
        }
    }

    /// An ordered QueryPointer observation can reveal a missed secondary press.
    pub fn suppress_observed_chord(&mut self) {
        match self.state {
            InteractionState::RightPressed {
                ref mut chorded, ..
            } => *chorded = true,
            InteractionState::MenuOpen {
                ref mut gesture, ..
            } => {
                if let Some(press_time) = gesture.press_time() {
                    *gesture = MenuGesture::Suppressed { press_time };
                }
            }
            _ => {}
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
        let button = match self.state {
            InteractionState::LeftPressed { .. } | InteractionState::Dragging { .. } => Some(1),
            InteractionState::RightPressed { .. } => Some(3),
            InteractionState::MenuOpen { .. } => {
                self.state = InteractionState::Idle;
                Some(0)
            }
            _ => None,
        };
        self.freshness_pending = false;
        if let Some(button) = button {
            if button != 0 {
                self.state = InteractionState::SuppressedUntilRelease { button };
            }
            HostAction::ReleaseGrab { time }
        } else {
            HostAction::None
        }
    }

    pub fn accepts_gesture_time(&self, time: u32) -> bool {
        let press_time = match self.state {
            InteractionState::LeftPressed { press_time, .. }
            | InteractionState::Dragging { press_time, .. }
            | InteractionState::RightPressed { press_time, .. } => Some(press_time),
            InteractionState::MenuOpen { gesture, .. } => gesture.press_time(),
            _ => None,
        };
        !self.freshness_pending || press_time.is_none_or(|press| timestamp_at_or_after(time, press))
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
    fn test_idle_right_press_starts_pending_opening() {
        let mut mgr = InteractionManager::new();
        assert_eq!(mgr.state(), InteractionState::Idle);
        assert_eq!(mgr.handle_right_press(1000), HostAction::TrackOpening);
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

        // A fresh right press starts pending opening
        let right_action = mgr.handle_right_press(1050);
        assert_eq!(right_action, HostAction::TrackOpening);
        assert!(matches!(mgr.state(), InteractionState::RightPressed { .. }));
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

#[cfg(test)]
mod menu_tests {
    use super::*;

    #[test]
    fn each_layer_item_rejects_other_row_outside_wrong_button_and_chord_completion() {
        for row in 0..3 {
            for release in [
                MenuHit::Row((row + 1) % 5),
                MenuHit::Outside,
                MenuHit::Background,
            ] {
                let mut manager = open();
                manager.menu_press(1, MenuHit::Row(row), 100, 0);
                assert_eq!(manager.menu_release(1, release, 101, 0), HostAction::None);
            }
            let mut manager = open();
            manager.menu_press(1, MenuHit::Row(row), 100, 0);
            assert_eq!(
                manager.menu_release(3, MenuHit::Row(row), 101, 0),
                HostAction::None
            );
            manager.menu_press(1, MenuHit::Row(row), 102, 0);
            manager.menu_press(2, MenuHit::Row(row), 103, 1);
            assert_eq!(
                manager.menu_release(1, MenuHit::Row(row), 104, 2),
                HostAction::None
            );
            assert_eq!(
                manager.menu_release(2, MenuHit::Row(row), 105, 0),
                HostAction::None
            );
        }
    }
    const ANCHOR: Point = Point::new(100, 100);
    fn open() -> InteractionManager {
        let mut manager = InteractionManager::new();
        manager.menu_acquired(MenuHit::Outside);
        manager
    }

    #[test]
    fn right_press_waits_for_matched_body_release_and_opens_once() {
        let mut manager = InteractionManager::new();
        assert_eq!(manager.handle_right_press(10), HostAction::TrackOpening);
        assert_eq!(
            manager.handle_opening_release(1, 11, 4, ANCHOR, true),
            HostAction::None
        );
        assert!(matches!(
            manager.state(),
            InteractionState::RightPressed { .. }
        ));
        assert_eq!(
            manager.handle_opening_release(3, 12, 0, ANCHOR, true),
            HostAction::OpeningFinished {
                time: 12,
                anchor: Some(ANCHOR)
            }
        );
        assert_eq!(
            manager.handle_opening_release(3, 13, 0, ANCHOR, true),
            HostAction::None
        );
        assert!(manager.is_idle());
    }

    #[test]
    fn outside_cancelled_and_stale_opening_releases_cannot_open() {
        let mut manager = InteractionManager::new();
        manager.handle_right_press(100);
        assert_eq!(
            manager.handle_opening_release(3, 101, 0, ANCHOR, false),
            HostAction::OpeningFinished {
                time: 101,
                anchor: None
            }
        );
        manager.handle_right_press(200);
        assert_eq!(
            manager.handle_opening_release(3, 199, 0, ANCHOR, true),
            HostAction::None
        );
        assert_eq!(manager.cancel(201), HostAction::ReleaseGrab { time: 201 });
        assert_eq!(
            manager.handle_opening_release(3, 202, 0, ANCHOR, true),
            HostAction::None
        );
        manager.handle_right_press(300);
        assert_eq!(
            manager.handle_opening_release(3, 202, 0, ANCHOR, true),
            HostAction::None
        );
    }

    #[test]
    fn opening_wrap_and_confirmed_long_hold_are_valid() {
        let mut manager = InteractionManager::new();
        manager.handle_right_press(u32::MAX - 10);
        assert_eq!(
            manager.handle_opening_release(3, u32::MAX - 11, 0, ANCHOR, true),
            HostAction::None
        );
        assert!(matches!(
            manager.handle_opening_release(3, 5, 0, ANCHOR, true),
            HostAction::OpeningFinished {
                anchor: Some(_),
                ..
            }
        ));
        manager.handle_right_press(100);
        manager.confirm_button_held();
        assert!(matches!(
            manager.handle_opening_release(3, 0x80000100, 0, ANCHOR, true),
            HostAction::OpeningFinished {
                anchor: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn opening_chord_never_opens_and_waits_for_every_release() {
        for last_button in [1, 2, 4, 5] {
            let mut manager = InteractionManager::new();
            manager.handle_right_press(100);
            manager.opening_chord(101);
            assert_eq!(manager.active_button(), Some(0));
            assert_eq!(
                manager.handle_opening_release(3, 102, 1 << (last_button - 1), ANCHOR, true),
                HostAction::None
            );
            assert!(matches!(
                manager.state(),
                InteractionState::RightPressed { chorded: true, .. }
            ));
            assert_eq!(
                manager.handle_opening_release(last_button, 103, 0, ANCHOR, true),
                HostAction::OpeningFinished {
                    time: 103,
                    anchor: None
                }
            );
            assert!(manager.is_idle());
        }
        // A secondary press+release before Button 3 release also cancels opening.
        let mut manager = InteractionManager::new();
        manager.handle_right_press(100);
        manager.opening_chord(101);
        manager.handle_opening_release(2, 102, 4, ANCHOR, true);
        assert_eq!(
            manager.handle_opening_release(3, 103, 0, ANCHOR, true),
            HostAction::OpeningFinished {
                time: 103,
                anchor: None
            }
        );
    }

    #[test]
    fn same_row_activates_all_five_items_only_on_release() {
        for (row, outcome) in [
            (0, MenuOutcome::SetLayer(Layer::Above)),
            (1, MenuOutcome::SetLayer(Layer::Normal)),
            (2, MenuOutcome::SetLayer(Layer::Below)),
            (3, MenuOutcome::Dismiss),
            (4, MenuOutcome::Quit),
        ] {
            let mut manager = open();
            assert_eq!(
                manager.menu_release(1, MenuHit::Row(row), 99, 0),
                HostAction::None
            );
            manager.menu_press(1, MenuHit::Row(row), 100, 0);
            assert!(matches!(
                manager.state(),
                InteractionState::MenuOpen {
                    gesture: MenuGesture::ItemPressed { .. },
                    ..
                }
            ));
            assert_eq!(
                manager.menu_release(1, MenuHit::Row(row), 101, 0),
                HostAction::CloseMenu { time: 101, outcome }
            );
            assert_eq!(
                manager.menu_release(1, MenuHit::Row(row), 102, 0),
                HostAction::None
            );
        }
    }

    #[test]
    fn release_away_and_inert_background_leave_menu_open() {
        for hit in [MenuHit::Row(4), MenuHit::Outside, MenuHit::Background] {
            let mut manager = open();
            manager.menu_press(1, MenuHit::Row(3), 100, 0);
            assert_eq!(manager.menu_release(1, hit, 101, 0), HostAction::None);
            assert_eq!(manager.active_button(), None);
            assert!(matches!(
                manager.state(),
                InteractionState::MenuOpen {
                    gesture: MenuGesture::None,
                    ..
                }
            ));
        }
        let mut manager = open();
        manager.menu_press(1, MenuHit::Background, 100, 0);
        assert_eq!(
            manager.menu_release(1, MenuHit::Row(4), 101, 0),
            HostAction::None
        );
    }

    #[test]
    fn outside_dismiss_consumes_press_motion_and_matching_release() {
        let mut manager = open();
        manager.menu_press(1, MenuHit::Outside, 100, 0);
        assert_eq!(manager.active_button(), Some(1));
        manager.menu_motion(MenuHit::Row(4), 101);
        assert!(matches!(
            manager.state(),
            InteractionState::MenuOpen {
                gesture: MenuGesture::OutsidePressed { .. },
                ..
            }
        ));
        assert_eq!(
            manager.menu_release(1, MenuHit::Row(4), 102, 0),
            HostAction::CloseMenu {
                time: 102,
                outcome: MenuOutcome::Dismiss
            }
        );
    }

    #[test]
    fn right_dismiss_requires_fresh_press_and_consumes_release() {
        let mut manager = open();
        assert_eq!(
            manager.menu_release(3, MenuHit::Outside, 100, 0),
            HostAction::None
        );
        manager.menu_press(3, MenuHit::Row(4), 110, 0);
        assert_eq!(
            manager.menu_release(3, MenuHit::Outside, 109, 0),
            HostAction::None
        );
        assert_eq!(
            manager.menu_release(3, MenuHit::Outside, 111, 0),
            HostAction::CloseMenu {
                time: 111,
                outcome: MenuOutcome::Dismiss
            }
        );
    }

    #[test]
    fn middle_and_wheel_are_ignored_inside_and_outside_without_timer() {
        for button in [2, 4, 5] {
            for hit in [MenuHit::Outside, MenuHit::Row(3), MenuHit::Row(4)] {
                let mut manager = open();
                manager.menu_press(button, hit, 100, 0);
                assert_eq!(manager.active_button(), None);
                assert_eq!(manager.menu_release(button, hit, 101, 0), HostAction::None);
                assert!(matches!(
                    manager.state(),
                    InteractionState::MenuOpen {
                        gesture: MenuGesture::None,
                        ..
                    }
                ));
            }
        }
    }

    #[test]
    fn menu_chords_cancel_actions_and_keep_capture_semantics_until_all_up() {
        for (button, hit) in [
            (1, MenuHit::Row(4)),
            (1, MenuHit::Outside),
            (3, MenuHit::Row(3)),
        ] {
            for other in [1, 2, 3, 4, 5] {
                if other == button {
                    continue;
                }
                let mut manager = open();
                manager.menu_press(button, hit, 100, 0);
                manager.menu_press(other, hit, 101, 1 << (button - 1));
                assert_eq!(manager.active_button(), Some(0));
                assert_eq!(
                    manager.menu_release(button, hit, 102, 1 << (other - 1)),
                    HostAction::None
                );
                assert_eq!(manager.menu_release(other, hit, 103, 0), HostAction::None);
                assert!(matches!(
                    manager.state(),
                    InteractionState::MenuOpen {
                        gesture: MenuGesture::None,
                        ..
                    }
                ));
                manager.menu_press(1, MenuHit::Row(3), 104, 0);
                assert_eq!(
                    manager.menu_release(1, MenuHit::Row(3), 105, 0),
                    HostAction::CloseMenu {
                        time: 105,
                        outcome: MenuOutcome::Dismiss
                    }
                );
            }
        }
    }

    #[test]
    fn chord_detected_from_completion_mask_cannot_activate_or_release() {
        let mut manager = open();
        manager.menu_press(1, MenuHit::Row(4), 100, 0);
        assert_eq!(
            manager.menu_release(1, MenuHit::Row(4), 101, 2),
            HostAction::None
        );
        assert_eq!(manager.active_button(), Some(0));
        assert_eq!(
            manager.menu_release(2, MenuHit::Row(4), 102, 0),
            HostAction::None
        );
        manager.menu_press(3, MenuHit::Outside, 103, 2);
        assert_eq!(
            manager.menu_release(3, MenuHit::Outside, 104, 2),
            HostAction::None
        );
    }

    #[test]
    fn menu_freshness_wrap_long_hold_and_cancellation() {
        let mut manager = open();
        manager.menu_press(1, MenuHit::Row(4), u32::MAX - 10, 0);
        assert_eq!(
            manager.menu_release(1, MenuHit::Row(4), u32::MAX - 11, 0),
            HostAction::None
        );
        assert_eq!(
            manager.menu_release(1, MenuHit::Row(4), 5, 0),
            HostAction::CloseMenu {
                time: 5,
                outcome: MenuOutcome::Quit
            }
        );
        manager.menu_press(1, MenuHit::Row(4), 100, 0);
        manager.confirm_button_held();
        assert!(matches!(
            manager.menu_release(1, MenuHit::Row(4), 0x80000100, 0),
            HostAction::CloseMenu {
                outcome: MenuOutcome::Quit,
                ..
            }
        ));
        manager.menu_press(1, MenuHit::Row(4), 100, 0);
        manager.cancel(101);
        assert_eq!(
            manager.menu_release(1, MenuHit::Row(4), 102, 0),
            HostAction::None
        );
        assert!(manager.is_idle());
    }

    #[test]
    fn popup_state_cannot_emit_body_drag_or_click_actions() {
        let mut manager = open();
        let bounds = ValidOriginBounds::new(0, 500, 0, 500);
        assert_eq!(
            manager.handle_left_press(ANCHOR, (80, 80), 100, ANCHOR),
            HostAction::None
        );
        assert_eq!(
            manager.handle_motion(Point::new(300, 300), 101, &bounds),
            HostAction::None
        );
        assert_eq!(
            manager.handle_left_release(ANCHOR, (80, 80), 102, &bounds),
            HostAction::None
        );
        assert_eq!(manager.handle_right_press(103), HostAction::None);
    }
}

#![allow(dead_code)]

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as XprotoExt, EventMask, GrabMode, GrabStatus, Window,
};

/// Issues an explicit GrabPointer request for the window.
///
/// Parameters follow the M02 pointer ownership specification:
/// - owner_events = false
/// - pointer_mode = ASYNC, keyboard_mode = ASYNC
/// - event_mask = BUTTON_PRESS | BUTTON_RELEASE | BUTTON_MOTION | POINTER_MOTION
/// - no pointer confinement
pub fn grab_pointer(
    conn: &impl Connection,
    window: Window,
    time: u32,
) -> Result<(GrabStatus, u64), Box<dyn std::error::Error>> {
    let mask = EventMask::BUTTON_PRESS
        | EventMask::BUTTON_RELEASE
        | EventMask::BUTTON_MOTION
        | EventMask::POINTER_MOTION;
    let cookie = conn.grab_pointer(
        false, // owner_events: do not report events to other client windows
        window,
        mask,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        x11rb::NONE, // confine_to: no boundary confinement
        x11rb::NONE, // cursor: retain existing cursor
        time,
    )?;
    let sequence = cookie.sequence_number();
    let reply = cookie.reply()?;
    Ok((reply.status, sequence))
}

/// Issues a checked UngrabPointer request.
pub fn ungrab_pointer(conn: &impl Connection, time: u32) -> Result<(), Box<dyn std::error::Error>> {
    // check() flushes and receives the acknowledgement. A later flush failure must
    // not turn an already confirmed release into an outstanding obligation.
    conn.ungrab_pointer(time)?.check()?;
    Ok(())
}

/// Exactly one local release obligation; failed release retains its owner for retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOwner {
    BodyLeft,
    OpeningRight,
    Menu { window: Window, generation: u64 },
}

#[derive(Debug, Default)]
pub struct PointerCaptureTracker {
    owner: Option<CaptureOwner>,
}

impl PointerCaptureTracker {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn owner(&self) -> Option<CaptureOwner> {
        self.owner
    }
    pub fn is_grabbed(&self) -> bool {
        self.owner.is_some()
    }

    pub fn track_automatic_right(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.owner.is_some() {
            return Err("Pointer already has an owner".into());
        }
        self.owner = Some(CaptureOwner::OpeningRight);
        Ok(())
    }

    /// A matching release with no buttons remaining ends X11's automatic grab.
    /// No UngrabPointer request is necessary on this path.
    pub fn automatic_right_finished(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.owner != Some(CaptureOwner::OpeningRight) {
            return Err("Automatic opening release has no matching owner".into());
        }
        self.owner = None;
        Ok(())
    }

    /// One acquisition attempt. An unknown reply outcome retains a cleanup obligation.
    pub fn acquire_with(
        &mut self,
        owner: CaptureOwner,
        acquire: impl FnOnce() -> Result<(GrabStatus, u64), Box<dyn std::error::Error>>,
    ) -> Result<(GrabStatus, u64), Box<dyn std::error::Error>> {
        if self.owner.is_some() {
            return Err("Pointer already has an owner".into());
        }
        self.owner = Some(owner);
        let result = acquire()?;
        if result.0 != GrabStatus::SUCCESS {
            self.owner = None;
        }
        Ok(result)
    }

    #[cfg(test)]
    pub fn set_grabbed(&mut self, grabbed: bool) {
        self.owner = grabbed.then_some(CaptureOwner::BodyLeft);
    }

    pub fn release_if_held(
        &mut self,
        conn: &impl Connection,
        time: u32,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        self.release_with(|| ungrab_pointer(conn, time))
    }

    pub fn release_with(
        &mut self,
        release: impl FnOnce() -> Result<(), Box<dyn std::error::Error>>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if self.owner.is_some() {
            release()?;
            self.owner = None;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Root QueryPointer observes the initiating button even outside the selected monitor.
pub fn button_state(
    conn: &impl Connection,
    root: Window,
) -> Result<(u64, u16), Box<dyn std::error::Error>> {
    let cookie = conn.query_pointer(root)?;
    let sequence = cookie.sequence_number();
    let reply = cookie.reply()?;
    let buttons = (u16::from(reply.mask) >> 8) & 31;
    Ok((sequence, buttons))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_release_retains_obligation_and_confirmed_retry_is_idempotent() {
        let mut tracker = PointerCaptureTracker::new();
        tracker.set_grabbed(true);
        assert!(tracker
            .release_with(|| Err("connection failure".into()))
            .is_err());
        assert!(tracker.is_grabbed());
        assert!(tracker.release_with(|| Ok(())).unwrap());
        assert!(!tracker.is_grabbed());
        assert!(!tracker.release_with(|| panic!("already released")).unwrap());
    }
}

#[cfg(test)]
mod menu_tests {
    use super::*;
    use std::cell::Cell;
    const MENU: CaptureOwner = CaptureOwner::Menu {
        window: 10,
        generation: 1,
    };

    #[test]
    fn automatic_right_normal_completion_needs_no_ungrab() {
        let mut tracker = PointerCaptureTracker::new();
        tracker.track_automatic_right().unwrap();
        assert_eq!(tracker.owner(), Some(CaptureOwner::OpeningRight));
        tracker.automatic_right_finished().unwrap();
        assert!(!tracker
            .release_with(|| panic!("normal automatic release must not ungrab"))
            .unwrap());
        assert!(tracker.automatic_right_finished().is_err());
    }

    #[test]
    fn automatic_right_cancellation_uses_checked_release_and_retains_failed_obligation() {
        let mut tracker = PointerCaptureTracker::new();
        tracker.track_automatic_right().unwrap();
        assert!(tracker
            .release_with(|| Err("ungrab failed".into()))
            .is_err());
        assert_eq!(tracker.owner(), Some(CaptureOwner::OpeningRight));
        assert!(tracker.release_with(|| Ok(())).unwrap());
        assert_eq!(tracker.owner(), None);
    }

    #[test]
    fn explicit_success_and_every_denial_make_one_attempt_without_contradictory_owners() {
        for status in [
            GrabStatus::SUCCESS,
            GrabStatus::ALREADY_GRABBED,
            GrabStatus::INVALID_TIME,
            GrabStatus::NOT_VIEWABLE,
            GrabStatus::FROZEN,
        ] {
            let mut tracker = PointerCaptureTracker::new();
            let calls = Cell::new(0);
            let result = tracker
                .acquire_with(MENU, || {
                    calls.set(calls.get() + 1);
                    Ok((status, 70000))
                })
                .unwrap();
            assert_eq!(result, (status, 70000));
            assert_eq!(calls.get(), 1);
            assert_eq!(
                tracker.owner(),
                (status == GrabStatus::SUCCESS).then_some(MENU)
            );
            if status == GrabStatus::SUCCESS {
                assert!(tracker
                    .acquire_with(CaptureOwner::BodyLeft, || panic!("duplicate acquisition"))
                    .is_err());
                assert!(tracker.track_automatic_right().is_err());
            }
        }
    }

    #[test]
    fn lost_acquisition_reply_and_failed_menu_release_retain_cleanup_obligation() {
        let mut tracker = PointerCaptureTracker::new();
        assert!(tracker
            .acquire_with(MENU, || Err("reply lost".into()))
            .is_err());
        assert_eq!(tracker.owner(), Some(MENU));
        assert!(tracker.release_with(|| Err("ungrab lost".into())).is_err());
        assert_eq!(tracker.owner(), Some(MENU));
        assert!(tracker.release_with(|| Ok(())).unwrap());
        assert!(!tracker.release_with(|| panic!("duplicate ungrab")).unwrap());
    }
}

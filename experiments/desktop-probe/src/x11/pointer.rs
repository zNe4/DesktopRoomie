#![allow(dead_code)]

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt as XprotoExt, EventMask, GrabMode, GrabStatus, KeyButMask, Window,
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
) -> Result<GrabStatus, Box<dyn std::error::Error>> {
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
    let reply = cookie.reply()?;
    Ok(reply.status)
}

/// Issues a checked UngrabPointer request.
pub fn ungrab_pointer(conn: &impl Connection, time: u32) -> Result<(), Box<dyn std::error::Error>> {
    // check() flushes and receives the acknowledgement. A later flush failure must
    // not turn an already confirmed release into an outstanding obligation.
    conn.ungrab_pointer(time)?.check()?;
    Ok(())
}

/// Tracks active pointer capture ownership so that ungrab calls are centralized and idempotent.
#[derive(Debug, Default)]
pub struct PointerCaptureTracker {
    has_grab: bool,
}

impl PointerCaptureTracker {
    pub fn new() -> Self {
        Self { has_grab: false }
    }

    pub fn is_grabbed(&self) -> bool {
        self.has_grab
    }

    pub fn set_grabbed(&mut self, grabbed: bool) {
        self.has_grab = grabbed;
    }

    /// Releases pointer capture if currently held. Returns Ok(true) if ungrab was sent.
    /// Only clears internal ownership state after ungrab succeeds.
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
        if self.has_grab {
            release()?;
            self.has_grab = false;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Root QueryPointer observes the initiating button even outside the selected monitor.
pub fn left_button_pressed(
    conn: &impl Connection,
    root: Window,
) -> Result<(u64, bool), Box<dyn std::error::Error>> {
    let cookie = conn.query_pointer(root)?;
    let sequence = cookie.sequence_number();
    let reply = cookie.reply()?;
    Ok((sequence, reply.mask.contains(KeyButMask::BUTTON1)))
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

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
/// - event_mask = BUTTON_RELEASE | BUTTON_MOTION | POINTER_MOTION
/// - no pointer confinement
pub fn grab_pointer(
    conn: &impl Connection,
    window: Window,
    time: u32,
) -> Result<GrabStatus, Box<dyn std::error::Error>> {
    let mask = EventMask::BUTTON_RELEASE | EventMask::BUTTON_MOTION | EventMask::POINTER_MOTION;
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
    conn.ungrab_pointer(time)?.check()?;
    conn.flush()?;
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
    pub fn release_if_held(
        &mut self,
        conn: &impl Connection,
        time: u32,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        if self.has_grab {
            self.has_grab = false;
            ungrab_pointer(conn, time)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

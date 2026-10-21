use std::os::unix::io::AsRawFd;
use std::time::Instant;

use x11rb::connection::Connection;

use crate::HostError;

#[repr(C)]
struct PollFd {
    fd: std::os::raw::c_int,
    events: std::os::raw::c_short,
    revents: std::os::raw::c_short,
}

const POLLIN: std::os::raw::c_short = 0x0001;

extern "C" {
    fn poll(fds: *mut PollFd, nfds: usize, timeout: std::os::raw::c_int) -> std::os::raw::c_int;
}

pub(crate) fn wait_x11(
    conn: &x11rb::rust_connection::RustConnection,
    deadline: Option<Instant>,
) -> Result<(), HostError> {
    conn.flush()?;
    let timeout_ms = deadline.map_or(-1, |deadline| {
        // Round up: a sub-millisecond remaining interval should not spin on poll(0).
        deadline
            .saturating_duration_since(Instant::now())
            .as_nanos()
            .div_ceil(1_000_000)
            .min(i32::MAX as u128) as i32
    });
    let mut pfd = PollFd {
        fd: conn.stream().as_raw_fd(),
        events: POLLIN,
        revents: 0,
    };
    let result = unsafe { poll(&mut pfd, 1, timeout_ms) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error.into());
        }
    } else if pfd.revents & (0x0008 | 0x0010 | 0x0020) != 0 {
        return Err(format!("X11 socket unavailable (poll revents=0x{:x})", pfd.revents).into());
    }
    Ok(())
}

/// Drains available X11 events into a bounded batch, prepending any previously buffered event.
pub(crate) fn drain_events_bounded<T, E, F>(
    buffered_event: &mut Option<T>,
    max_batch: usize,
    mut poll_fn: F,
) -> Result<Vec<T>, E>
where
    F: FnMut() -> Result<Option<T>, E>,
{
    let mut batch = Vec::new();
    if let Some(ev) = buffered_event.take() {
        batch.push(ev);
    }
    while batch.len() < max_batch {
        match poll_fn()? {
            Some(ev) => batch.push(ev),
            None => break,
        }
    }
    Ok(batch)
}

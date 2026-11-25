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

#[cfg(feature = "managed-supervisor-fixture")]
pub(crate) fn wait_managed(
    conn: Option<&x11rb::rust_connection::RustConnection>,
    interest: &roomie_worker_supervisor::WaitInterest,
    deadline: Option<Instant>,
) -> Result<roomie_worker_supervisor::Readiness, HostError> {
    if let Some(conn) = conn {
        conn.flush()?;
    }
    poll_managed(
        conn.map(|conn| conn.stream().as_raw_fd()),
        interest,
        deadline,
    )
}

#[cfg(feature = "managed-supervisor-fixture")]
pub(crate) fn poll_managed(
    host: Option<std::os::fd::RawFd>,
    interest: &roomie_worker_supervisor::WaitInterest,
    deadline: Option<Instant>,
) -> Result<roomie_worker_supervisor::Readiness, HostError> {
    const POLLOUT: i16 = 0x0004;
    const POLLERR: i16 = 0x0008;
    const POLLHUP: i16 = 0x0010;
    const POLLNVAL: i16 = 0x0020;
    let mut descriptors = [
        PollFd {
            fd: host.unwrap_or(-1),
            events: POLLIN,
            revents: 0,
        },
        PollFd {
            fd: interest.socket.unwrap_or(-1),
            events: POLLIN | if interest.writable { POLLOUT } else { 0 },
            revents: 0,
        },
        PollFd {
            fd: interest.process.unwrap_or(-1),
            events: POLLIN,
            revents: 0,
        },
    ];
    let timeout = deadline.map_or(-1, |deadline| {
        deadline
            .saturating_duration_since(Instant::now())
            .as_nanos()
            .div_ceil(1_000_000)
            .min(i32::MAX as u128) as i32
    });
    // SAFETY: poll borrows the three live entries and writes their revents.
    let result = unsafe { poll(descriptors.as_mut_ptr(), descriptors.len(), timeout) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::Interrupted {
            Ok(roomie_worker_supervisor::Readiness::default())
        } else {
            Err(error.into())
        };
    }
    let [x11, worker, process] = descriptors.map(|descriptor| descriptor.revents);
    if x11 & (POLLERR | POLLHUP | POLLNVAL) != 0 {
        return Err(format!("X11 socket unavailable (poll revents=0x{x11:x})").into());
    }
    Ok(roomie_worker_supervisor::Readiness {
        readable: worker & POLLIN != 0,
        writable: worker & POLLOUT != 0,
        hangup: worker & POLLHUP != 0,
        error: worker & (POLLERR | POLLNVAL) != 0,
        exited: process != 0,
    })
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

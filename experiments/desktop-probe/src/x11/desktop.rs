//! Checked WM-directed desktop transport for an already admitted frozen destination.
//! X acknowledgement proves neither relocation nor any current/historical placement.
#![allow(dead_code)] // Stage C deliberately has no owner/event-loop consumer.

use std::time::Instant;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, ClientMessageEvent, ConnectionExt, EventMask, Window};

type HostError = Box<dyn std::error::Error>;

/// One application-source _NET_WM_DESKTOP envelope, with no destination reads or policy.
/// The caller supplies the already interned atom and separately validated destination.
pub struct DesktopRequest {
    destination: Window,
    propagate: bool,
    mask: EventMask,
    event: ClientMessageEvent,
}

impl DesktopRequest {
    pub fn new(
        root: Window,
        body: Window,
        wm_desktop: Atom,
        frozen_destination: u32,
    ) -> Result<Self, HostError> {
        if frozen_destination == u32::MAX {
            return Err("All-desktops is not an ordinary relocation destination".into());
        }
        Ok(Self {
            destination: root,
            propagate: false,
            mask: EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event: ClientMessageEvent::new(32, body, wm_desktop, [frozen_destination, 1, 0, 0, 0]),
        })
    }

    /// Returns false at expiry without sending; true means checked transport completion
    /// only. Cookie acquisition marks possible dispatch before check, including on error.
    /// This never clears prior dispatch evidence, confirms WM state or retries. A caller
    /// must not retry an uncertain request merely because its checked completion failed.
    /// The logical deadline assumes responsive synchronous X transport, as elsewhere.
    pub fn send_before(
        self,
        conn: &impl Connection,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<bool, HostError> {
        self.send_before_with(
            deadline,
            Instant::now,
            |request| {
                Ok(conn.send_event(
                    request.propagate,
                    request.destination,
                    request.mask,
                    request.event,
                )?)
            },
            || *dispatched = true,
            |cookie| Ok(cookie.check()?),
        )
    }

    fn send_before_with<T>(
        self,
        deadline: Instant,
        now: impl FnOnce() -> Instant,
        send: impl FnOnce(Self) -> Result<T, HostError>,
        mark_dispatched: impl FnOnce(),
        check: impl FnOnce(T) -> Result<(), HostError>,
    ) -> Result<bool, HostError> {
        if now() >= deadline {
            return Ok(false);
        }
        let cookie = send(self)?;
        mark_dispatched();
        // SendEvent acknowledgement is not a WM/body observation or command result.
        check(cookie)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::time::Duration;
    use x11rb::x11_utils::Serialize;

    const ROOT: Window = 0x1234_5678;
    const BODY: Window = 0x2345_6789;
    const WM_DESKTOP: Atom = 0x3456_789a;

    fn request() -> DesktopRequest {
        DesktopRequest::new(ROOT, BODY, WM_DESKTOP, 7).unwrap()
    }

    #[test]
    fn exact_root_directed_application_envelope_and_serialized_bytes() {
        for frozen_destination in [0, 7, u32::MAX - 1] {
            let request = DesktopRequest::new(ROOT, BODY, WM_DESKTOP, frozen_destination).unwrap();
            assert_eq!(request.destination, ROOT);
            assert!(!request.propagate);
            assert_eq!(
                request.mask,
                EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT
            );
            assert_eq!(u32::from(request.mask), (1 << 19) | (1 << 20));
            assert_eq!(request.event.window, BODY);
            assert_eq!(request.event.type_, WM_DESKTOP);
            assert_eq!(request.event.format, 32);
            assert_eq!(
                request.event.data.as_data32(),
                [frozen_destination, 1, 0, 0, 0]
            );
            let mut expected = vec![33, 32, 0, 0];
            for word in [BODY, WM_DESKTOP, frozen_destination, 1, 0, 0, 0] {
                expected.extend(word.to_ne_bytes());
            }
            assert_eq!(request.event.serialize().as_slice(), expected);
            assert_eq!(expected.len(), 32);
        }
    }

    #[test]
    fn all_desktops_cannot_construct_a_relocation_request() {
        assert!(DesktopRequest::new(ROOT, BODY, WM_DESKTOP, u32::MAX).is_err());
    }

    #[test]
    fn deadline_at_or_before_now_prevents_every_native_step() {
        let deadline = Instant::now();
        for now in [deadline, deadline + Duration::from_nanos(1)] {
            let dispatched = Cell::new(false);
            let result = request().send_before_with::<()>(
                deadline,
                || now,
                |_| panic!("expired request must not call SendEvent"),
                || {
                    dispatched.set(true);
                    panic!("expired request must not mark dispatch");
                },
                |_| panic!("expired request must not check completion"),
            );
            assert!(!result.unwrap());
            assert!(!dispatched.get());
        }
    }

    #[test]
    fn cookie_failure_checked_failure_and_transport_completion_have_exact_order() {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(1);
        for failure in [Some("send"), Some("check"), None] {
            let dispatched = Cell::new(false);
            let trace = RefCell::new(vec![]);
            let result: Result<bool, HostError> = request().send_before_with(
                deadline,
                || {
                    trace.borrow_mut().push("now");
                    start
                },
                |request| {
                    trace.borrow_mut().push("send");
                    assert!(!dispatched.get());
                    assert_eq!(request.destination, ROOT);
                    assert_eq!(request.event.data.as_data32(), [7, 1, 0, 0, 0]);
                    if failure == Some("send") {
                        Err("send".into())
                    } else {
                        Ok(77)
                    }
                },
                || {
                    assert!(!dispatched.replace(true));
                    trace.borrow_mut().push("mark");
                },
                |cookie| {
                    assert_eq!(cookie, 77);
                    assert!(dispatched.get(), "possible dispatch must precede check");
                    trace.borrow_mut().push("check");
                    if failure == Some("check") {
                        Err("check".into())
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(dispatched.get(), failure != Some("send"));
            assert_eq!(
                *trace.borrow(),
                if failure == Some("send") {
                    vec!["now", "send"]
                } else {
                    vec!["now", "send", "mark", "check"]
                }
            );
            match failure {
                Some(error) => assert_eq!(result.unwrap_err().to_string(), error),
                // Only the transport bool is returned: no placement or command proof.
                None => assert!(result.unwrap()),
            }
        }
    }

    #[test]
    fn another_send_requires_a_separate_external_invocation() {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(1);
        let sends = Cell::new(0);
        let checks = Cell::new(0);
        for invocation in 1..=2 {
            let dispatched = Cell::new(false);
            let result = request().send_before_with(
                deadline,
                || start,
                |_| {
                    sends.set(sends.get() + 1);
                    Ok(())
                },
                || dispatched.set(true),
                |()| {
                    checks.set(checks.get() + 1);
                    Err("uncertain checked completion".into())
                },
            );
            assert!(result.is_err());
            assert!(dispatched.get());
            assert_eq!(sends.get(), invocation);
            assert_eq!(checks.get(), invocation);
        }
    }
}

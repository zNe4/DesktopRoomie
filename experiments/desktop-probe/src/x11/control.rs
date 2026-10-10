//! Stage B ownership only. No caller windows or request/reply transport.
use std::fs::File;
use std::io::Read;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, CreateWindowAux, CreateWindowRequest, EventMask, PropMode, Property,
    PropertyNotifyEvent, Screen, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperExt;

use crate::control::{selection_name, Lifecycle, OwnerDescriptor, INSTANCE, TIMESTAMP};
use crate::x11::resource::{cleanup_all, OwnedResource};
use crate::HostError;

type Result<T> = std::result::Result<T, HostError>;
const PROBE_BUDGET: Duration = Duration::from_secs(1);
const ACQUISITION_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquisition {
    Owned,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Claim {
    Owned,
    Duplicate,
    Unconfirmed,
}

// Narrow injected seam for the guarded transaction and its single retry.
trait AcquisitionHost {
    fn now(&self) -> Instant;
    fn timestamp(&mut self, deadline: Instant) -> Result<NonZeroU32>;
    fn grab(&mut self) -> Result<()>;
    fn owner(&mut self) -> Result<u32>;
    fn claim(&mut self, timestamp: NonZeroU32) -> Result<()>;
    fn ungrab(&mut self) -> Result<()>;
}

fn guarded_claim(host: &mut impl AcquisitionHost, control: u32, time: NonZeroU32) -> Result<Claim> {
    // Even an unacknowledged grab can have reached the server: attempt release on that path too.
    let operation = host.grab().and_then(|()| {
        if host.owner()? != 0 {
            return Ok(Claim::Duplicate);
        }
        host.claim(time)?;
        Ok(match host.owner()? {
            0 => Claim::Unconfirmed,
            owner if owner == control => Claim::Owned,
            _ => Claim::Duplicate,
        })
    });
    let release = host.ungrab();
    if let Err(error) = &release {
        eprintln!(
            "[ERROR] Server release was not acknowledged: {error}; connection teardown required"
        );
    }
    // Preserve the arbitration error; never hide a release failure or retry after it.
    match (operation, release) {
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        (Ok(claim), Ok(())) => Ok(claim),
    }
}

fn acquire_with(
    host: &mut impl AcquisitionHost,
    control: u32,
) -> Result<(Acquisition, Option<u32>)> {
    let deadline = host.now() + ACQUISITION_BUDGET;
    for attempt in 0..2 {
        if host.now() >= deadline {
            return Err("Ownership acquisition deadline expired".into());
        }
        let time = host.timestamp((host.now() + PROBE_BUDGET).min(deadline))?;
        if host.now() >= deadline {
            return Err("Ownership acquisition deadline expired".into());
        }
        match guarded_claim(host, control, time)? {
            Claim::Owned => return Ok((Acquisition::Owned, Some(time.get()))),
            Claim::Duplicate => return Ok((Acquisition::Duplicate, None)),
            Claim::Unconfirmed if attempt == 0 => {} // released before the next timestamp probe
            Claim::Unconfirmed => {
                return Err(
                    "Selection claim still unconfirmed after one fresh-timestamp retry".into(),
                )
            }
        }
    }
    unreachable!("two attempts return above")
}

// x11rb SequenceNumber is u64. poll_for_event_with_sequence reconstructs its high bits
// in x11rb-protocol connection::extract_sequence_number, before queuing the event.
// Comparing full numbers avoids aliasing an older event across a 16-bit wrap. The
// PropertyNotify's own u16 sequence is the low wire portion, NOT the cookie number.
fn timestamp_match(
    event: &PropertyNotifyEvent,
    sequence: u64,
    request: u64,
    window: u32,
    atom: u32,
) -> Option<NonZeroU32> {
    (event.response_type & 0x80 == 0
        && event.window == window
        && event.atom == atom
        && event.state == Property::NEW_VALUE
        && sequence == request
        && event.sequence == request as u16)
        .then(|| NonZeroU32::new(event.time))
        .flatten()
}

/// A fixed deadline is checked for every event, including uninterrupted irrelevant traffic.
/// Poll the library queue before waiting: checked requests can already have buffered events.
fn wait_events_with<T>(
    deadline: Instant,
    mut now: impl FnMut() -> Instant,
    mut next: impl FnMut() -> Result<Option<(Event, u64)>>,
    mut wait: impl FnMut(Instant) -> Result<()>,
    mut inspect: impl FnMut(Event, u64) -> Result<Option<T>>,
) -> Result<Option<T>> {
    loop {
        if now() >= deadline {
            return Ok(None);
        }
        if let Some((event, sequence)) = next()? {
            if let Event::Error(error) = &event {
                return Err(format!("Asynchronous X11 error: {error:?}").into());
            }
            if let Some(value) = inspect(event, sequence)? {
                return Ok(Some(value));
            }
        } else {
            wait(deadline)?;
        }
    }
}

fn control_request(window: u32, screen: &Screen) -> CreateWindowRequest<'static> {
    CreateWindowRequest {
        depth: 0,
        wid: window,
        parent: screen.root,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        border_width: 0,
        class: WindowClass::INPUT_ONLY,
        visual: screen.root_visual,
        value_list: std::borrow::Cow::Owned(
            CreateWindowAux::new()
                .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
        ),
    }
}

fn owner_epoch() -> Result<NonZeroU32> {
    let mut random = File::open("/dev/urandom")?;
    loop {
        let mut bytes = [0; 4];
        random.read_exact(&mut bytes)?;
        if let Some(epoch) = NonZeroU32::new(u32::from_ne_bytes(bytes)) {
            return Ok(epoch);
        }
    }
}

pub struct ControlWindow {
    descriptor: OwnerDescriptor,
    selection: u32,
    instance_atom: u32,
    timestamp_atom: u32,
    resource: OwnedResource,
    exists: bool,
    owns_selection: bool,
    acquired_timestamp: Option<u32>,
}

impl ControlWindow {
    pub fn create(conn: &impl Connection, screen: &Screen, screen_num: usize) -> Result<Self> {
        let screen_num = u32::try_from(screen_num)?;
        let epoch = owner_epoch()?.get();
        let selection = conn
            .intern_atom(false, selection_name(screen_num).as_bytes())?
            .reply()?
            .atom;
        let instance_atom = conn.intern_atom(false, INSTANCE.as_bytes())?.reply()?.atom;
        let timestamp_atom = conn.intern_atom(false, TIMESTAMP.as_bytes())?.reply()?.atom;
        let window = conn.generate_id()?;
        let request = control_request(window, screen);
        conn.create_window(
            request.depth,
            request.wid,
            request.parent,
            request.x,
            request.y,
            request.width,
            request.height,
            request.border_width,
            request.class,
            request.visual,
            &request.value_list,
        )?
        .check()?;
        let endpoint = Self::candidate(
            screen_num,
            window,
            epoch,
            selection,
            instance_atom,
            timestamp_atom,
        );
        let prepared = (|| {
            endpoint.write_descriptor(conn, endpoint.descriptor)?;
            conn.change_property32(
                PropMode::REPLACE,
                window,
                timestamp_atom,
                AtomEnum::CARDINAL,
                &[0],
            )?
            .check()?;
            Ok(())
        })();
        if let Err(error) = prepared {
            let _ = cleanup_all([Box::new(|| endpoint.destroy(conn))]);
            return Err(error);
        }
        Ok(endpoint)
    }

    fn candidate(
        screen_num: u32,
        window: u32,
        epoch: u32,
        selection: u32,
        instance_atom: u32,
        timestamp_atom: u32,
    ) -> Self {
        Self {
            descriptor: OwnerDescriptor {
                screen_num,
                control_xid: window,
                epoch,
                lifecycle: Lifecycle::Starting,
                body_xid: None,
            },
            selection,
            instance_atom,
            timestamp_atom,
            resource: OwnedResource::default(),
            exists: true,
            owns_selection: false,
            acquired_timestamp: None,
        }
    }

    pub fn acquire(&mut self, conn: &RustConnection) -> Result<Acquisition> {
        let control = self.descriptor.control_xid;
        let (outcome, timestamp) = acquire_with(
            &mut NativeAcquisition {
                conn,
                endpoint: self,
            },
            control,
        )?;
        self.owns_selection = outcome == Acquisition::Owned;
        self.acquired_timestamp = timestamp;
        if let Some(time) = timestamp {
            println!("[INSTANCE] Own screen {} selection via control 0x{control:x}, server timestamp {time}; Starting", self.descriptor.screen_num);
        }
        Ok(outcome)
    }

    fn write_descriptor(&self, conn: &impl Connection, descriptor: OwnerDescriptor) -> Result<()> {
        let words = descriptor
            .encode()
            .map_err(|error| format!("Invalid owner descriptor: {error:?}"))?;
        conn.change_property32(
            PropMode::REPLACE,
            descriptor.control_xid,
            self.instance_atom,
            AtomEnum::CARDINAL,
            &words,
        )?
        .check()?;
        Ok(())
    }

    fn publish_with(
        &mut self,
        descriptor: OwnerDescriptor,
        write: impl FnOnce(OwnerDescriptor) -> Result<()>,
    ) -> Result<()> {
        if !self.exists || !self.owns_selection {
            return Err("Control endpoint ownership unavailable".into());
        }
        descriptor
            .encode()
            .map_err(|error| format!("Invalid owner descriptor: {error:?}"))?;
        if descriptor == self.descriptor {
            return Ok(());
        }
        write(descriptor)?;
        self.descriptor = descriptor;
        Ok(())
    }

    fn publish(&mut self, conn: &impl Connection, descriptor: OwnerDescriptor) -> Result<()> {
        let (window, atom) = (descriptor.control_xid, self.instance_atom);
        self.publish_with(descriptor, |descriptor| {
            let words = descriptor
                .encode()
                .map_err(|error| format!("Invalid descriptor: {error:?}"))?;
            conn.change_property32(PropMode::REPLACE, window, atom, AtomEnum::CARDINAL, &words)?
                .check()?;
            Ok(())
        })
    }

    pub fn publish_body(&mut self, conn: &impl Connection, body: u32) -> Result<()> {
        self.publish(
            conn,
            OwnerDescriptor {
                body_xid: Some(body),
                ..self.descriptor
            },
        )
    }

    pub fn publish_ready(&mut self, conn: &impl Connection) -> Result<()> {
        self.publish(
            conn,
            OwnerDescriptor {
                lifecycle: Lifecycle::Ready,
                ..self.descriptor
            },
        )
    }

    pub fn closing(&mut self, conn: &impl Connection) -> Result<()> {
        let (selection, window, atom) = (
            self.selection,
            self.descriptor.control_xid,
            self.instance_atom,
        );
        self.closing_with(
            || Ok(conn.get_selection_owner(selection)?.reply()?.owner),
            |descriptor| {
                let words = descriptor
                    .encode()
                    .map_err(|error| format!("Invalid Closing descriptor: {error:?}"))?;
                conn.change_property32(
                    PropMode::REPLACE,
                    window,
                    atom,
                    AtomEnum::CARDINAL,
                    &words,
                )?
                .check()?;
                Ok(())
            },
        )
    }

    fn closing_with(
        &mut self,
        owner: impl FnOnce() -> Result<u32>,
        write: impl FnOnce(OwnerDescriptor) -> Result<()>,
    ) -> Result<()> {
        // Recheck even when a queued SelectionClear has not yet been routed.
        if self.exists && self.owns_selection {
            self.owns_selection = owner()? == self.descriptor.control_xid;
            if self.owns_selection {
                return self.publish_with(
                    OwnerDescriptor {
                        lifecycle: Lifecycle::Closing,
                        ..self.descriptor
                    },
                    write,
                );
            }
        }
        Ok(())
    }

    pub fn is_lifecycle_event(&self, event: &Event) -> bool {
        matches!(event, Event::SelectionClear(ev) if ev.owner == self.descriptor.control_xid && ev.selection == self.selection)
            || matches!(event, Event::DestroyNotify(ev) if ev.window == self.descriptor.control_xid && ev.response_type & 0x80 == 0)
    }

    /// Returns true only for actual loss. Synthetic SelectionClear still requires a fresh lookup.
    pub fn handle_event(&mut self, conn: &impl Connection, event: &Event) -> Result<bool> {
        let selection = self.selection;
        self.handle_event_with(event, || {
            Ok(conn.get_selection_owner(selection)?.reply()?.owner)
        })
    }

    fn handle_event_with(
        &mut self,
        event: &Event,
        owner: impl FnOnce() -> Result<u32>,
    ) -> Result<bool> {
        match event {
            Event::DestroyNotify(ev)
                if ev.window == self.descriptor.control_xid && ev.response_type & 0x80 == 0 =>
            {
                self.resource.externally_destroyed();
                self.exists = false;
                self.owns_selection = false;
                Ok(true)
            }
            Event::SelectionClear(ev)
                if ev.owner == self.descriptor.control_xid
                    && ev.selection == self.selection
                    && self.owns_selection =>
            {
                if owner()? != self.descriptor.control_xid {
                    self.owns_selection = false;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            _ => Ok(false),
        }
    }

    pub fn delay(&mut self, conn: &RustConnection, seconds: u64) -> Result<()> {
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(seconds))
            .ok_or("Startup delay exceeds supported monotonic deadline")?;
        wait_events_with::<()>(
            deadline,
            Instant::now,
            || Ok(conn.poll_for_event_with_sequence()?),
            |deadline| crate::wait_x11(conn, Some(deadline)),
            |event, _| {
                if self.handle_event(conn, &event)? {
                    return Err("Control endpoint lost during startup delay".into());
                }
                Ok(None)
            },
        )?;
        Ok(())
    }

    pub fn destroy(&self, conn: &impl Connection) -> Result<()> {
        self.resource.release_with(|| {
            conn.destroy_window(self.descriptor.control_xid)?.check()?;
            Ok(())
        })
    }
}

struct NativeAcquisition<'a> {
    conn: &'a RustConnection,
    endpoint: &'a mut ControlWindow,
}
impl AcquisitionHost for NativeAcquisition<'_> {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn timestamp(&mut self, deadline: Instant) -> Result<NonZeroU32> {
        let endpoint = &mut self.endpoint;
        let request = self.conn.change_property32(
            PropMode::APPEND,
            endpoint.descriptor.control_xid,
            endpoint.timestamp_atom,
            AtomEnum::CARDINAL,
            &[],
        )?;
        let sequence = request.sequence_number();
        request.check()?;
        wait_events_with(
            deadline,
            Instant::now,
            || Ok(self.conn.poll_for_event_with_sequence()?),
            |deadline| crate::wait_x11(self.conn, Some(deadline)),
            |event, full_sequence| {
                if endpoint.handle_event(self.conn, &event)? {
                    return Err("Control endpoint destroyed during timestamp probe".into());
                }
                Ok(match event {
                    Event::PropertyNotify(ev) => timestamp_match(
                        &ev,
                        full_sequence,
                        sequence,
                        endpoint.descriptor.control_xid,
                        endpoint.timestamp_atom,
                    ),
                    _ => None,
                })
            },
        )?
        .ok_or_else(|| "X11 timestamp probe deadline expired".into())
    }
    fn grab(&mut self) -> Result<()> {
        self.conn.grab_server()?.check()?;
        Ok(())
    }
    fn owner(&mut self) -> Result<u32> {
        Ok(self
            .conn
            .get_selection_owner(self.endpoint.selection)?
            .reply()?
            .owner)
    }
    fn claim(&mut self, timestamp: NonZeroU32) -> Result<()> {
        self.conn
            .set_selection_owner(
                self.endpoint.descriptor.control_xid,
                self.endpoint.selection,
                timestamp.get(),
            )?
            .check()?;
        Ok(())
    }
    fn ungrab(&mut self) -> Result<()> {
        self.conn.ungrab_server()?.check()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;
    use x11rb::protocol::xproto::{DestroyNotifyEvent, SelectionClearEvent, PROPERTY_NOTIFY_EVENT};

    fn property(sequence: u64) -> PropertyNotifyEvent {
        PropertyNotifyEvent {
            response_type: PROPERTY_NOTIFY_EVENT,
            sequence: sequence as u16,
            window: 10,
            atom: 20,
            time: 1234,
            state: Property::NEW_VALUE,
        }
    }

    #[test]
    fn timestamp_accepts_only_exact_native_notification() {
        let valid = property(42);
        assert_eq!(timestamp_match(&valid, 42, 42, 10, 20).unwrap().get(), 1234);
        let invalid = [
            PropertyNotifyEvent {
                window: 11,
                ..valid
            },
            PropertyNotifyEvent { atom: 21, ..valid },
            PropertyNotifyEvent {
                state: Property::DELETE,
                ..valid
            },
            PropertyNotifyEvent {
                sequence: 41,
                ..valid
            },
            PropertyNotifyEvent {
                response_type: PROPERTY_NOTIFY_EVENT | 0x80,
                ..valid
            },
            PropertyNotifyEvent { time: 0, ..valid },
        ];
        for event in invalid {
            assert_eq!(timestamp_match(&event, 42, 42, 10, 20), None);
        }
        assert_eq!(timestamp_match(&valid, 41, 42, 10, 20), None);
    }

    #[test]
    fn timestamp_full_sequence_preserves_wrap_and_rejects_low_word_alias() {
        for request in [65535, 65536, 65537, u32::MAX as u64, u32::MAX as u64 + 1] {
            let event = property(request);
            assert!(timestamp_match(&event, request, request, 10, 20).is_some());
            assert!(timestamp_match(&event, request + 65536, request, 10, 20).is_none());
            if request >= 65536 {
                assert!(timestamp_match(&event, request - 65536, request, 10, 20).is_none());
            }
        }
    }

    #[test]
    fn timestamp_drains_stale_buffered_events_before_sleep() {
        let now = Instant::now();
        let mut events = VecDeque::from([
            (Event::PropertyNotify(property(41)), 41),
            (
                Event::PropertyNotify(PropertyNotifyEvent {
                    time: 0,
                    ..property(42)
                }),
                42,
            ),
            (Event::PropertyNotify(property(42)), 42),
        ]);
        let result = wait_events_with(
            now + PROBE_BUDGET,
            || now,
            || Ok(events.pop_front()),
            |_| panic!("valid event already buffered"),
            |event, seq| {
                Ok(match event {
                    Event::PropertyNotify(ev) => timestamp_match(&ev, seq, 42, 10, 20),
                    _ => None,
                })
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.get(), 1234);
        assert!(events.is_empty());
    }

    #[test]
    fn irrelevant_traffic_never_extends_probe_or_delay_deadline() {
        for budget in [PROBE_BUDGET, Duration::from_secs(3)] {
            let start = Instant::now();
            let now = Cell::new(start);
            let calls = Cell::new(0);
            let result = wait_events_with::<()>(
                start + budget,
                || now.get(),
                || {
                    calls.set(calls.get() + 1);
                    now.set(now.get() + Duration::from_millis(100));
                    Ok(Some((Event::PropertyNotify(property(1)), 1)))
                },
                |_| panic!("continuous buffered traffic"),
                |_, _| Ok(None),
            )
            .unwrap();
            assert_eq!(result, None);
            assert_eq!(calls.get(), budget.as_millis() / 100);
            assert_eq!(now.get(), start + budget);
        }
    }

    #[test]
    fn empty_probe_waits_to_fixed_deadline_and_expires() {
        let start = Instant::now();
        let now = Cell::new(start);
        let polls = Cell::new(0);
        let result = wait_events_with::<()>(
            start + PROBE_BUDGET,
            || now.get(),
            || {
                polls.set(polls.get() + 1);
                Ok(None)
            },
            |deadline| {
                assert_eq!(deadline, start + PROBE_BUDGET);
                now.set(deadline);
                Ok(())
            },
            |_, _| panic!("no event"),
        )
        .unwrap();
        assert_eq!(result, None);
        assert_eq!(polls.get(), 1);
    }

    fn endpoint() -> ControlWindow {
        ControlWindow::candidate(0, 10, 77, 30, 40, 20)
    }
    fn destroy() -> Event {
        Event::DestroyNotify(DestroyNotifyEvent {
            response_type: 17,
            sequence: 3,
            event: 10,
            window: 10,
        })
    }
    fn clear(synthetic: bool) -> Event {
        Event::SelectionClear(SelectionClearEvent {
            response_type: 29 | if synthetic { 0x80 } else { 0 },
            sequence: 3,
            time: 77,
            owner: 10,
            selection: 30,
        })
    }

    #[test]
    fn startup_wait_aborts_on_control_destruction_and_host_errors() {
        let mut endpoint = endpoint();
        let start = Instant::now();
        let error = wait_events_with::<()>(
            start + PROBE_BUDGET,
            || start,
            || Ok(Some((destroy(), 3))),
            |_| panic!("must not wait"),
            |event, _| {
                if endpoint.handle_event_with(&event, || panic!("no selection query"))? {
                    Err("endpoint lost".into())
                } else {
                    Ok(None)
                }
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "endpoint lost");
        assert!(!endpoint.exists);
        for poll_failure in [true, false] {
            let error = wait_events_with::<()>(
                start + PROBE_BUDGET,
                || start,
                || {
                    if poll_failure {
                        Err("connection failure".into())
                    } else {
                        Ok(None)
                    }
                },
                |_| Err("socket failure".into()),
                |_, _| Ok(None),
            )
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                if poll_failure {
                    "connection failure"
                } else {
                    "socket failure"
                }
            );
        }
    }

    #[test]
    fn asynchronous_protocol_error_aborts_wait_before_inspection() {
        let start = Instant::now();
        let error = x11rb::x11_utils::X11Error {
            error_kind: x11rb::protocol::ErrorKind::Window,
            error_code: 3,
            sequence: 1,
            bad_value: 10,
            minor_opcode: 0,
            major_opcode: 18,
            extension_name: None,
            request_name: None,
        };
        let result = wait_events_with::<()>(
            start + PROBE_BUDGET,
            || start,
            || Ok(Some((Event::Error(error.clone()), 1))),
            |_| panic!(),
            |_, _| panic!(),
        )
        .unwrap_err();
        assert!(result.to_string().contains("Asynchronous X11 error"));
    }

    struct FakeHost {
        now: Instant,
        owners: VecDeque<std::result::Result<u32, &'static str>>,
        timestamps: VecDeque<u32>,
        log: Vec<String>,
        grabbed: bool,
        failure: Option<&'static str>,
        release_failure: bool,
        probe_cost: Duration,
        transaction_cost: Duration,
        deadlines: Vec<Instant>,
    }
    impl FakeHost {
        fn new(owners: &[u32]) -> Self {
            Self {
                now: Instant::now(),
                owners: owners.iter().map(|v| Ok(*v)).collect(),
                timestamps: VecDeque::from([1234, 2345]),
                log: vec![],
                grabbed: false,
                failure: None,
                release_failure: false,
                probe_cost: Duration::ZERO,
                transaction_cost: Duration::ZERO,
                deadlines: vec![],
            }
        }
    }
    impl AcquisitionHost for FakeHost {
        fn now(&self) -> Instant {
            self.now
        }
        fn timestamp(&mut self, deadline: Instant) -> Result<NonZeroU32> {
            assert!(!self.grabbed, "timestamp must be outside grab");
            self.log.push("timestamp".into());
            self.deadlines.push(deadline);
            self.now += self.probe_cost;
            if self.failure == Some("timestamp") {
                return Err("timestamp".into());
            }
            NonZeroU32::new(self.timestamps.pop_front().expect("at most two probes"))
                .ok_or_else(|| "unusable zero timestamp".into())
        }
        fn grab(&mut self) -> Result<()> {
            assert!(!self.grabbed);
            self.grabbed = true;
            self.log.push("grab".into());
            if self.failure == Some("grab") {
                Err("grab".into())
            } else {
                Ok(())
            }
        }
        fn owner(&mut self) -> Result<u32> {
            assert!(self.grabbed);
            self.log.push("owner".into());
            self.owners
                .pop_front()
                .expect("unexpected lookup")
                .map_err(Into::into)
        }
        fn claim(&mut self, timestamp: NonZeroU32) -> Result<()> {
            assert!(self.grabbed);
            self.log.push(format!("claim:{}", timestamp.get()));
            if self.failure == Some("claim") {
                Err("claim".into())
            } else {
                Ok(())
            }
        }
        fn ungrab(&mut self) -> Result<()> {
            self.log.push("ungrab".into());
            self.now += self.transaction_cost;
            if self.release_failure {
                Err("release".into())
            } else {
                self.grabbed = false;
                Ok(())
            }
        }
    }

    #[test]
    fn claim_uses_exact_nonzero_timestamp_and_verified_ownership() {
        let mut host = FakeHost::new(&[0, 10]);
        assert_eq!(
            acquire_with(&mut host, 10).unwrap(),
            (Acquisition::Owned, Some(1234))
        );
        assert_eq!(
            host.log,
            [
                "timestamp",
                "grab",
                "owner",
                "claim:1234",
                "owner",
                "ungrab"
            ]
        );
        assert!(!host.grabbed);
    }

    #[test]
    fn existing_owner_refuses_without_claim_or_retry() {
        let mut host = FakeHost::new(&[99]);
        assert_eq!(
            acquire_with(&mut host, 10).unwrap(),
            (Acquisition::Duplicate, None)
        );
        assert_eq!(host.log, ["timestamp", "grab", "owner", "ungrab"]);
    }

    #[test]
    fn stale_claim_releases_then_samples_fresh_timestamp_before_second_grab() {
        let mut host = FakeHost::new(&[0, 0, 0, 10]);
        assert_eq!(
            acquire_with(&mut host, 10).unwrap(),
            (Acquisition::Owned, Some(2345))
        );
        assert_eq!(
            host.log,
            [
                "timestamp",
                "grab",
                "owner",
                "claim:1234",
                "owner",
                "ungrab",
                "timestamp",
                "grab",
                "owner",
                "claim:2345",
                "owner",
                "ungrab"
            ]
        );
    }

    #[test]
    fn second_unconfirmed_claim_stops_without_third_attempt() {
        let mut host = FakeHost::new(&[0, 0, 0, 0]);
        assert!(acquire_with(&mut host, 10)
            .unwrap_err()
            .to_string()
            .contains("one fresh-timestamp retry"));
        assert_eq!(host.log.iter().filter(|s| *s == "timestamp").count(), 2);
        assert_eq!(host.log.last().unwrap(), "ungrab");
    }

    #[test]
    fn owner_appearing_on_retry_or_verification_refuses() {
        for owners in [&[0, 0, 99][..], &[0, 99][..]] {
            let mut host = FakeHost::new(owners);
            assert_eq!(
                acquire_with(&mut host, 10).unwrap().0,
                Acquisition::Duplicate
            );
            assert_eq!(
                host.log.iter().filter(|s| s.starts_with("claim:")).count(),
                1
            );
            assert!(!host.grabbed);
        }
    }

    #[test]
    fn transaction_errors_always_attempt_release_and_never_retry() {
        for failure in ["grab", "lookup", "claim", "verification"] {
            for release_failure in [false, true] {
                let mut host = FakeHost::new(&[0, 10]);
                host.failure = Some(failure);
                host.release_failure = release_failure;
                if failure == "lookup" {
                    host.owners[0] = Err("lookup");
                }
                if failure == "verification" {
                    host.owners[1] = Err("verification");
                }
                let error = acquire_with(&mut host, 10).unwrap_err();
                assert_eq!(error.to_string(), failure, "original failure has priority");
                assert_eq!(host.log.last().unwrap(), "ungrab");
                assert_eq!(host.log.iter().filter(|s| *s == "timestamp").count(), 1);
            }
        }
    }

    #[test]
    fn release_failure_blocks_success_duplicate_and_stale_retry() {
        for owners in [&[0, 10][..], &[99][..], &[0, 0][..]] {
            let mut host = FakeHost::new(owners);
            host.release_failure = true;
            assert_eq!(
                acquire_with(&mut host, 10).unwrap_err().to_string(),
                "release"
            );
            assert_eq!(host.log.iter().filter(|s| *s == "timestamp").count(), 1);
        }
    }

    #[test]
    fn zero_or_failed_timestamp_never_enters_transaction() {
        for zero in [false, true] {
            let mut host = FakeHost::new(&[]);
            if zero {
                host.timestamps[0] = 0;
            } else {
                host.failure = Some("timestamp");
            }
            assert!(acquire_with(&mut host, 10).is_err());
            assert_eq!(host.log, ["timestamp"]);
        }
    }

    #[test]
    fn overall_budget_caps_retry_probe_and_prevents_late_claim() {
        let mut host = FakeHost::new(&[0, 0]);
        let start = host.now;
        host.probe_cost = Duration::from_millis(900);
        host.transaction_cost = Duration::from_millis(400);
        assert!(acquire_with(&mut host, 10)
            .unwrap_err()
            .to_string()
            .contains("deadline"));
        assert_eq!(
            host.deadlines,
            [start + PROBE_BUDGET, start + ACQUISITION_BUDGET]
        );
        assert_eq!(host.log.iter().filter(|s| *s == "grab").count(), 1);
        let mut host = FakeHost::new(&[0, 0]);
        host.transaction_cost = ACQUISITION_BUDGET;
        assert!(acquire_with(&mut host, 10).is_err());
        assert_eq!(host.log.iter().filter(|s| *s == "timestamp").count(), 1);
    }

    #[test]
    fn control_construction_is_input_only_without_focus_or_pointer_events() {
        let screen = Screen {
            root: 55,
            root_visual: 66,
            ..Screen::default()
        };
        let request = control_request(10, &screen);
        assert_eq!(
            (
                request.depth,
                request.border_width,
                request.width,
                request.height
            ),
            (0, 0, 1, 1)
        );
        assert_eq!(
            (request.parent, request.visual, request.class),
            (55, 66, WindowClass::INPUT_ONLY)
        );
        assert_eq!(
            request.value_list.event_mask,
            Some(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY)
        );
        // Creating a window leaves it unmapped; the endpoint owns no map operation.
        let endpoint = endpoint();
        assert_eq!(endpoint.descriptor.lifecycle, Lifecycle::Starting);
        assert_eq!(endpoint.descriptor.body_xid, None);
        assert!(!endpoint.owns_selection);
    }

    #[test]
    fn descriptor_publication_requires_ownership_body_and_checked_write() {
        let mut endpoint = endpoint();
        let body = OwnerDescriptor {
            body_xid: Some(88),
            ..endpoint.descriptor
        };
        assert!(endpoint
            .publish_with(body, |_| panic!("not acquired"))
            .is_err());
        endpoint.owns_selection = true;
        let ready_without_body = OwnerDescriptor {
            lifecycle: Lifecycle::Ready,
            ..endpoint.descriptor
        };
        assert!(endpoint
            .publish_with(ready_without_body, |_| panic!("missing body"))
            .is_err());
        endpoint
            .publish_with(body, |descriptor| {
                assert_eq!(descriptor.lifecycle, Lifecycle::Starting);
                assert_eq!(descriptor.encode().unwrap()[6], 88);
                Ok(())
            })
            .unwrap();
        let ready = OwnerDescriptor {
            lifecycle: Lifecycle::Ready,
            ..body
        };
        assert!(endpoint
            .publish_with(ready, |_| Err("write failed".into()))
            .is_err());
        assert_eq!(endpoint.descriptor.lifecycle, Lifecycle::Starting);
        endpoint.publish_with(ready, |_| Ok(())).unwrap();
        endpoint
            .publish_with(ready, |_| panic!("Ready must be idempotent"))
            .unwrap();
    }

    #[test]
    fn stale_and_fabricated_selection_clear_requery_actual_owner() {
        let mut endpoint = endpoint();
        endpoint.owns_selection = true;
        for synthetic in [false, true] {
            let queries = Cell::new(0);
            assert!(!endpoint
                .handle_event_with(&clear(synthetic), || {
                    queries.set(1);
                    Ok(10)
                })
                .unwrap());
            assert_eq!(queries.get(), 1);
            assert!(endpoint.owns_selection);
        }
        assert!(endpoint
            .handle_event_with(&clear(false), || Err("lookup".into()))
            .is_err());
    }

    #[test]
    fn actual_loss_is_terminal_and_closing_cannot_touch_replacement() {
        for owner in [0, 99] {
            let mut endpoint = endpoint();
            endpoint.owns_selection = true;
            assert!(endpoint
                .handle_event_with(&clear(false), || Ok(owner))
                .unwrap());
            assert!(!endpoint.owns_selection);
            assert!(!endpoint
                .handle_event_with(&clear(false), || panic!("no reacquisition"))
                .unwrap());
            endpoint
                .closing_with(|| panic!("lost"), |_| panic!("do not publish"))
                .unwrap();
        }
    }

    #[test]
    fn destruction_is_native_only_and_satisfies_release_obligation() {
        let mut endpoint = endpoint();
        endpoint.owns_selection = true;
        let mut synthetic = destroy();
        if let Event::DestroyNotify(ev) = &mut synthetic {
            ev.response_type |= 0x80;
        }
        assert!(!endpoint.handle_event_with(&synthetic, || panic!()).unwrap());
        assert!(endpoint.exists);
        assert!(endpoint.handle_event_with(&destroy(), || panic!()).unwrap());
        assert!(!endpoint.exists);
        assert!(!endpoint.owns_selection);
        endpoint
            .resource
            .release_with(|| panic!("already externally destroyed"))
            .unwrap();
        endpoint.closing_with(|| panic!(), |_| panic!()).unwrap();
    }

    #[test]
    fn closing_rechecks_pending_loss_and_preserves_failed_write_truth() {
        let mut endpoint = endpoint();
        endpoint.owns_selection = true;
        endpoint
            .closing_with(|| Ok(99), |_| panic!("replacement owner"))
            .unwrap();
        assert!(!endpoint.owns_selection);
        let mut endpoint = self::endpoint();
        endpoint.owns_selection = true;
        assert!(endpoint
            .closing_with(|| Ok(10), |_| Err("write".into()))
            .is_err());
        assert_eq!(endpoint.descriptor.lifecycle, Lifecycle::Starting);
        endpoint
            .closing_with(
                || Ok(10),
                |descriptor| {
                    assert_eq!(descriptor.lifecycle, Lifecycle::Closing);
                    Ok(())
                },
            )
            .unwrap();
    }

    #[test]
    fn unacknowledged_control_destruction_is_not_repeated_or_reported_successful() {
        let endpoint = endpoint();
        assert!(endpoint
            .resource
            .release_with(|| Err("destroy failed".into()))
            .is_err());
        assert!(endpoint
            .resource
            .release_with(|| panic!("unknown prior result"))
            .is_err());
    }
    #[test]
    fn delay_ignores_stale_clear_then_exits_on_verified_loss() {
        let mut endpoint = endpoint();
        endpoint.owns_selection = true;
        let start = Instant::now();
        let mut events = VecDeque::from([(clear(true), 1), (clear(false), 2)]);
        let mut owners = VecDeque::from([10, 99]);
        let error = wait_events_with::<()>(
            start + Duration::from_secs(10),
            || start,
            || Ok(events.pop_front()),
            |_| panic!("loss already buffered"),
            |event, _| {
                if endpoint.handle_event_with(&event, || Ok(owners.pop_front().unwrap()))? {
                    return Err("endpoint lost".into());
                }
                Ok(None)
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "endpoint lost");
        assert!(owners.is_empty());
        assert!(!endpoint.owns_selection);
        assert_eq!(endpoint.descriptor.body_xid, None);
    }

    #[test]
    fn unrelated_lifecycle_events_do_not_query_or_end_ownership() {
        let mut endpoint = endpoint();
        endpoint.owns_selection = true;
        let Event::SelectionClear(valid) = clear(false) else {
            unreachable!()
        };
        for event in [
            Event::SelectionClear(SelectionClearEvent { owner: 11, ..valid }),
            Event::SelectionClear(SelectionClearEvent {
                selection: 31,
                ..valid
            }),
            Event::DestroyNotify(DestroyNotifyEvent {
                response_type: 17,
                sequence: 1,
                event: 10,
                window: 11,
            }),
        ] {
            assert!(!endpoint.is_lifecycle_event(&event));
            assert!(!endpoint
                .handle_event_with(&event, || panic!("unrelated owner query"))
                .unwrap());
            assert!(endpoint.owns_selection);
        }
    }
}

//! Selection ownership and bounded same-instance request/terminal-reply transport.
use std::fs::File;
use std::io::Read;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt, CreateWindowAux,
    CreateWindowRequest, EventMask, GetPropertyReply, MapState, PropMode, Property,
    PropertyNotifyEvent, Screen, SendEventRequest, WindowClass,
};
use x11rb::protocol::{ErrorKind, Event};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperExt;

use crate::control::{
    self, selection_name, ActiveOperation, Admission, CallerMarker, Command, ExitCode, Lifecycle,
    OwnerDescriptor, Reason, ReplyCorrelation, Request, Stage, Status, TerminalResponse, INSTANCE,
    TIMESTAMP,
};
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
    root: u32,
    transport: TransportAtoms,
    active: Option<ActiveOperation>,
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
        let transport = TransportAtoms::intern(conn)?;
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
        let mut endpoint = Self::candidate(
            screen_num,
            window,
            epoch,
            selection,
            instance_atom,
            timestamp_atom,
        );
        endpoint.root = screen.root;
        endpoint.transport = transport;
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
            root: 0,
            transport: TransportAtoms::default(),
            active: None,
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
                self.route_request(conn, &event, None, false)?;
                Ok(None)
            },
        )?;
        Ok(())
    }

    /// Control messages are consumed before body/popup routing and movement flushing.
    pub fn route_request(
        &mut self,
        conn: &impl Connection,
        event: &Event,
        popup: Option<u32>,
        local_layer_busy: bool,
    ) -> Result<bool> {
        let Event::ClientMessage(message) = event else {
            return Ok(false);
        };
        let mut host = NativePeer {
            conn,
            selection: self.selection,
        };
        route_request_with(
            &mut host,
            self.descriptor,
            self.root,
            self.transport,
            popup,
            local_layer_busy,
            &mut self.active,
            message,
        )
    }

    /// Stage C completes within route_request. Keep shutdown completion explicit for
    /// an admitted record, without waiting, retrying, or skipping ordered cleanup.
    pub fn finish_remote(&mut self, conn: &impl Connection) -> Result<()> {
        let Some(operation) = self.active.as_ref() else {
            return Ok(());
        };
        let response = TerminalResponse {
            status: Status::Closing,
            request_id: operation.request.request_id,
            control_xid: self.descriptor.control_xid,
            detail: control::preflight_detail(Reason::Closing, Stage::Shutdown),
        };
        let mut host = NativePeer {
            conn,
            selection: self.selection,
        };
        finish_active(&mut host, self.transport, &mut self.active, response)
    }

    /// One existing-size batch at the Closing boundary; never wait for callers or
    /// prolong teardown with an unbounded stream. Later callers observe destruction.
    pub fn reply_while_closing(&mut self, conn: &RustConnection, popup: Option<u32>) -> Result<()> {
        if !self.exists || !self.owns_selection || self.descriptor.lifecycle != Lifecycle::Closing {
            return Ok(());
        }
        closing_events_with(
            || Ok(conn.poll_for_event()?),
            |event| {
                if self.handle_event(conn, &event)? {
                    return Ok(true);
                }
                if let Event::Error(error) = &event {
                    return Err(format!("Asynchronous X11 error: {error:?}").into());
                }
                self.route_request(conn, &event, popup, false)?;
                Ok(false)
            },
        )
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

#[derive(Debug, Default, Clone, Copy)]
struct TransportAtoms {
    request: u32,
    reply: u32,
    caller: u32,
}
impl TransportAtoms {
    fn intern(conn: &impl Connection) -> Result<Self> {
        Ok(Self {
            request: conn
                .intern_atom(false, control::REQUEST.as_bytes())?
                .reply()?
                .atom,
            reply: conn
                .intern_atom(false, control::REPLY.as_bytes())?
                .reply()?
                .atom,
            caller: conn
                .intern_atom(false, control::CALLER.as_bytes())?
                .reply()?
                .atom,
        })
    }
}

/// Only this peer's BadWindow is recoverable; unrelated protocol/connection errors
/// retain their typed error and remain fatal. No string matching.
fn peer_result<T>(result: std::result::Result<T, ReplyError>, peer: u32) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(ReplyError::X11Error(error))
            if error.error_kind == ErrorKind::Window && error.bad_value == peer =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

struct EndpointEvidence {
    root: u32,
    parent: u32,
    class: WindowClass,
    map_state: MapState,
    property: GetPropertyReply,
}
impl EndpointEvidence {
    fn endpoint_matches(&self, root: u32) -> bool {
        self.root == root
            && self.parent == root
            && self.class == WindowClass::INPUT_ONLY
            && self.map_state == MapState::UNMAPPED
    }
    fn words(&self) -> Option<Vec<u32>> {
        let p = &self.property;
        if p.type_ != u32::from(AtomEnum::CARDINAL)
            || p.format != 32
            || p.value_len != 7
            || p.bytes_after != 0
            || p.value.len() != 28
        {
            return None;
        }
        Some(p.value32()?.collect())
    }
}

/// Narrow native peer seam; it deliberately exposes no body/WM/input operations.
trait PeerHost {
    fn now(&self) -> Instant;
    fn owner(&mut self) -> Result<u32>;
    fn evidence(&mut self, window: u32, property: u32) -> Result<Option<EndpointEvidence>>;
    fn send(&mut self, event: ClientMessageEvent) -> Result<bool>;
}
struct NativePeer<'a, C> {
    conn: &'a C,
    selection: u32,
}
impl<C: Connection> PeerHost for NativePeer<'_, C> {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn owner(&mut self) -> Result<u32> {
        Ok(self
            .conn
            .get_selection_owner(self.selection)?
            .reply()?
            .owner)
    }
    fn evidence(&mut self, window: u32, property: u32) -> Result<Option<EndpointEvidence>> {
        let Some(tree) = peer_result(self.conn.query_tree(window)?.reply(), window)? else {
            return Ok(None);
        };
        let Some(attributes) =
            peer_result(self.conn.get_window_attributes(window)?.reply(), window)?
        else {
            return Ok(None);
        };
        // ANY lets validation distinguish wrong types from missing/truncated data.
        let Some(property) = peer_result(
            self.conn
                .get_property(false, window, property, AtomEnum::ANY, 0, 7)?
                .reply(),
            window,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(EndpointEvidence {
            root: tree.root,
            parent: tree.parent,
            class: attributes.class,
            map_state: attributes.map_state,
            property,
        }))
    }
    fn send(&mut self, event: ClientMessageEvent) -> Result<bool> {
        let request = direct_message(event);
        Ok(peer_result(
            self.conn
                .send_event(
                    request.propagate,
                    request.destination,
                    request.event_mask,
                    *request.event,
                )?
                .check(),
            request.destination,
        )?
        .is_some())
    }
}

fn closing_events_with(
    next: impl FnMut() -> Result<Option<Event>>,
    mut inspect: impl FnMut(Event) -> Result<bool>,
) -> Result<()> {
    for event in crate::drain_events_bounded(&mut None, 64, next)? {
        if inspect(event)? {
            break;
        }
    }
    Ok(())
}

fn direct_message(event: ClientMessageEvent) -> SendEventRequest<'static> {
    SendEventRequest {
        propagate: false,
        destination: event.window,
        event_mask: EventMask::NO_EVENT,
        event: std::borrow::Cow::Owned(event.into()),
    }
}

fn reply_event(
    atoms: TransportAtoms,
    reply: u32,
    response: TerminalResponse,
) -> Result<ClientMessageEvent> {
    Ok(ClientMessageEvent::new(
        32,
        reply,
        atoms.reply,
        response
            .encode()
            .map_err(|e| format!("Invalid terminal response: {e:?}"))?,
    ))
}
fn request_event(
    atoms: TransportAtoms,
    owner: u32,
    request: Request,
) -> Result<ClientMessageEvent> {
    Ok(ClientMessageEvent::new(
        32,
        owner,
        atoms.request,
        request
            .encode()
            .map_err(|e| format!("Invalid request: {e:?}"))?,
    ))
}

fn validated_route(
    evidence: &EndpointEvidence,
    root: u32,
    owner: OwnerDescriptor,
    popup: Option<u32>,
    reply: u32,
    words: [u32; 5],
) -> Option<(Request, Option<Reason>)> {
    if reply == 0
        || reply == root
        || reply == owner.control_xid
        || Some(reply) == owner.body_xid
        || Some(reply) == popup
        || !evidence.endpoint_matches(root)
    {
        return None;
    }
    let marker = CallerMarker::decode(&evidence.words()?).ok()?;
    let id = u64::from(words[1]) | (u64::from(words[2]) << 32);
    if marker.control_xid != owner.control_xid
        || marker.epoch != owner.epoch
        || words[4] != owner.epoch
        || marker.request_id != id
    {
        return None;
    }
    // A v1 marker supplies the safe identity for an incompatible header. Known
    // commands must still match; an unknown wire command cannot decode as v1.
    let command = Command::try_from(words[0] & 0xffff);
    if command.is_ok_and(|command| command != marker.command) {
        return None;
    }
    let error = if words[0] >> 16 != control::VERSION {
        Some(Reason::WrongVersion)
    } else if command.is_err() {
        Some(Reason::UnknownCommand)
    } else {
        None
    };
    let request = if error.is_some() {
        Request {
            command: marker.command,
            request_id: id,
            reply_xid: reply,
            epoch: words[4],
        }
    } else {
        Request::decode(&words).ok()?
    };
    marker
        .matches(owner.control_xid, request)
        .then_some((request, error))
}

fn finish_active(
    host: &mut impl PeerHost,
    atoms: TransportAtoms,
    active: &mut Option<ActiveOperation>,
    response: TerminalResponse,
) -> Result<()> {
    let operation = active.as_ref().ok_or("Missing admitted operation")?;
    let result = reply_event(atoms, operation.request.reply_xid, response)
        .and_then(|event| host.send(event).map(|_| ()));
    // Clear after the one attempt, including BadWindow and unrelated failures.
    *active = None;
    result
}

#[allow(clippy::too_many_arguments)]
fn route_request_with(
    host: &mut impl PeerHost,
    owner: OwnerDescriptor,
    root: u32,
    atoms: TransportAtoms,
    popup: Option<u32>,
    local_layer_busy: bool,
    active: &mut Option<ActiveOperation>,
    event: &ClientMessageEvent,
) -> Result<bool> {
    if event.window != owner.control_xid || event.type_ != atoms.request {
        return Ok(false);
    }
    if event.format != 32 {
        return Ok(true);
    }
    let words = event.data.as_data32();
    if words[4] != owner.epoch || (words[1] == 0 && words[2] == 0) {
        return Ok(true);
    }
    if host.owner()? != owner.control_xid {
        return Err("Control selection ownership lost before admission".into());
    }
    let Some(evidence) = host.evidence(words[3], atoms.caller)? else {
        return Ok(true);
    };
    let Some((request, protocol_error)) =
        validated_route(&evidence, root, owner, popup, words[3], words)
    else {
        return Ok(true);
    };
    // Synchronous validation may have overlapped owner loss; admission requires
    // current ownership, not just the lookup before reading the caller marker.
    if host.owner()? != owner.control_xid {
        return Err("Control selection ownership lost during validation".into());
    }
    let admission = match protocol_error {
        Some(reason) => Admission::Reply(Status::ProtocolError, reason, Stage::Preflight),
        None => control::admit(
            active,
            request,
            owner.lifecycle,
            local_layer_busy,
            host.now(),
        ),
    };
    let (status, detail) = match admission {
        Admission::Duplicate => return Ok(true),
        Admission::Reply(status, reason, stage) => {
            (status, control::preflight_detail(reason, stage))
        }
        Admission::Admitted => (Status::Failed, active.as_ref().unwrap().detail),
    };
    let response = TerminalResponse {
        status,
        request_id: request.request_id,
        control_xid: owner.control_xid,
        detail,
    };
    if admission == Admission::Admitted {
        finish_active(host, atoms, active, response)?;
    } else {
        host.send(reply_event(atoms, request.reply_xid, response)?)?;
    }
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerOutcome {
    NoInstance,
    Protocol,
    Lifecycle(Lifecycle),
    Terminal(TerminalResponse),
    Unknown(&'static str),
}
impl CallerOutcome {
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::NoInstance => ExitCode::NoInstance,
            Self::Protocol => ExitCode::Protocol,
            Self::Lifecycle(_) => ExitCode::Busy,
            Self::Terminal(response) => response.status.exit_code(),
            Self::Unknown(_) => ExitCode::OutcomeUnknown,
        }
    }
    pub fn diagnostic(&self, command: Command) -> String {
        let message = match self {
            Self::NoInstance => "no running instance".into(),
            Self::Protocol => "incompatible, stale, or malformed owner endpoint".into(),
            Self::Lifecycle(lifecycle) => format!("owner is {lifecycle:?}"),
            Self::Terminal(response) => format!(
                "{:?} — {:?} ({:?})",
                response.status, response.detail.reason, response.detail.stage
            ),
            Self::Unknown(reason) => format!("outcome unknown — {reason}"),
        };
        format!("{command}: {message}")
    }
}

fn discover(
    host: &mut impl PeerHost,
    root: u32,
    screen_num: u32,
    instance: u32,
) -> Result<std::result::Result<OwnerDescriptor, CallerOutcome>> {
    let owner = host.owner()?;
    if owner == 0 {
        return Ok(Err(CallerOutcome::NoInstance));
    }
    let Some(evidence) = host.evidence(owner, instance)? else {
        return Ok(Err(CallerOutcome::Protocol));
    };
    let descriptor = evidence
        .words()
        .and_then(|words| OwnerDescriptor::decode(&words).ok());
    let Some(descriptor) = descriptor else {
        return Ok(Err(CallerOutcome::Protocol));
    };
    if !evidence.endpoint_matches(root)
        || descriptor.screen_num != screen_num
        || descriptor.control_xid != owner
        || host.owner()? != owner
    {
        return Ok(Err(CallerOutcome::Protocol));
    }
    Ok(Ok(descriptor))
}

fn fresh_request_id(random: &mut impl Read) -> Result<u64> {
    loop {
        let mut bytes = [0; 8];
        random.read_exact(&mut bytes)?;
        let id = u64::from_ne_bytes(bytes);
        if id != 0 {
            return Ok(id);
        }
    }
}
fn caller_request(window: u32, screen: &Screen) -> CreateWindowRequest<'static> {
    let mut request = control_request(window, screen);
    request.value_list = std::borrow::Cow::Owned(CreateWindowAux::new());
    request
}

trait CallerHost: PeerHost {
    fn atoms(&mut self, screen_num: u32) -> Result<(u32, TransportAtoms)>;
    fn watch(&mut self, owner: u32) -> Result<bool>;
    fn create_reply(&mut self, marker: CallerMarker) -> Result<u32>;
    fn next(&mut self) -> Result<Option<Event>>;
    fn wait(&mut self, deadline: Instant) -> Result<()>;
    fn cleanup(&mut self) -> Result<()>;
}
struct NativeCaller<'a> {
    peer: NativePeer<'a, RustConnection>,
    screen: &'a Screen,
    atoms: TransportAtoms,
    reply: Option<(u32, OwnedResource)>,
}
impl PeerHost for NativeCaller<'_> {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn owner(&mut self) -> Result<u32> {
        self.peer.owner()
    }
    fn evidence(&mut self, window: u32, property: u32) -> Result<Option<EndpointEvidence>> {
        self.peer.evidence(window, property)
    }
    fn send(&mut self, event: ClientMessageEvent) -> Result<bool> {
        self.peer.send(event)
    }
}
impl CallerHost for NativeCaller<'_> {
    fn atoms(&mut self, screen_num: u32) -> Result<(u32, TransportAtoms)> {
        let conn = self.peer.conn;
        self.peer.selection = conn
            .intern_atom(false, selection_name(screen_num).as_bytes())?
            .reply()?
            .atom;
        let instance = conn.intern_atom(false, INSTANCE.as_bytes())?.reply()?.atom;
        self.atoms = TransportAtoms::intern(conn)?;
        Ok((instance, self.atoms))
    }
    fn watch(&mut self, owner: u32) -> Result<bool> {
        Ok(peer_result(
            self.peer
                .conn
                .change_window_attributes(
                    owner,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
                )?
                .check(),
            owner,
        )?
        .is_some())
    }
    fn create_reply(&mut self, marker: CallerMarker) -> Result<u32> {
        let conn = self.peer.conn;
        let window = conn.generate_id()?;
        let request = caller_request(window, self.screen);
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
        self.reply = Some((window, OwnedResource::default()));
        let words = marker
            .encode()
            .map_err(|e| format!("Invalid caller marker: {e:?}"))?;
        // The sole marker write for this invocation. Neither peer updates it.
        conn.change_property32(
            PropMode::REPLACE,
            window,
            self.atoms.caller,
            AtomEnum::CARDINAL,
            &words,
        )?
        .check()?;
        Ok(window)
    }
    fn next(&mut self) -> Result<Option<Event>> {
        Ok(self.peer.conn.poll_for_event()?)
    }
    fn wait(&mut self, deadline: Instant) -> Result<()> {
        self.peer.conn.flush()?;
        crate::wait_x11(self.peer.conn, Some(deadline))
    }
    fn cleanup(&mut self) -> Result<()> {
        if let Some((window, resource)) = &self.reply {
            resource.release_with(|| {
                self.peer.conn.destroy_window(*window)?.check()?;
                Ok(())
            })?;
        }
        Ok(())
    }
}

fn correlated_reply(
    event: &Event,
    atoms: TransportAtoms,
    owner: OwnerDescriptor,
    correlation: ReplyCorrelation,
) -> Option<TerminalResponse> {
    let Event::ClientMessage(event) = event else {
        return None;
    };
    if event.type_ != atoms.reply || event.format != 32 {
        return None;
    }
    let response = TerminalResponse::decode(&event.data.as_data32()).ok()?;
    correlation
        .matches(owner, event.window, response)
        .then_some(response)
}

fn caller_with(
    host: &mut impl CallerHost,
    root: u32,
    screen_num: u32,
    command: Command,
    id: impl FnOnce() -> Result<u64>,
) -> Result<CallerOutcome> {
    // This begins before atom/selection discovery and never changes.
    let deadline = host.now() + Duration::from_secs(4);
    let operation = (|| {
        let (instance, atoms) = host.atoms(screen_num)?;
        let owner = match discover(host, root, screen_num, instance)? {
            Ok(owner) => owner,
            Err(outcome) => return Ok(outcome),
        };
        if host.now() >= deadline {
            return Ok(CallerOutcome::Unknown(
                "caller deadline expired before send",
            ));
        }
        if owner.lifecycle != Lifecycle::Ready {
            return Ok(CallerOutcome::Lifecycle(owner.lifecycle));
        }
        if !host.watch(owner.control_xid)? {
            return Ok(CallerOutcome::Protocol);
        }
        let marker = CallerMarker {
            control_xid: owner.control_xid,
            epoch: owner.epoch,
            request_id: id()?,
            command,
        };
        let reply = host.create_reply(marker)?;
        // Revalidate endpoint, identity and lifecycle at the send boundary. Never
        // redirect to a replacement owner or send with a stale epoch.
        let current = match discover(host, root, screen_num, instance)? {
            Ok(owner) => owner,
            Err(outcome) => return Ok(outcome),
        };
        if current.control_xid != owner.control_xid || current.epoch != owner.epoch {
            return Ok(CallerOutcome::Protocol);
        }
        if current.lifecycle != Lifecycle::Ready {
            return Ok(CallerOutcome::Lifecycle(current.lifecycle));
        }
        if host.now() >= deadline {
            return Ok(CallerOutcome::Unknown(
                "caller deadline expired before send",
            ));
        }
        let request = Request {
            command,
            request_id: marker.request_id,
            reply_xid: reply,
            epoch: owner.epoch,
        };
        // The only send site. No timeout, stale event, or peer loss returns here.
        if !host.send(request_event(atoms, owner.control_xid, request)?)? {
            return Ok(CallerOutcome::Unknown("owner disappeared at request send"));
        }
        let correlation = ReplyCorrelation {
            control_xid: owner.control_xid,
            epoch: owner.epoch,
            request_id: marker.request_id,
            reply_xid: reply,
        };
        let mut owner_gone = false;
        loop {
            if host.now() >= deadline {
                return Ok(CallerOutcome::Unknown("caller deadline expired"));
            }
            match host.next()? {
                Some(event) => {
                    if let Some(response) = correlated_reply(&event, atoms, owner, correlation) {
                        return Ok(CallerOutcome::Terminal(response));
                    }
                    if matches!(event, Event::DestroyNotify(ev) if ev.window == owner.control_xid && ev.response_type & 0x80 == 0)
                    {
                        owner_gone = true;
                    }
                    if let Event::Error(error) = event {
                        return Err(format!("Asynchronous X11 error: {error:?}").into());
                    }
                }
                None if owner_gone => {
                    return Ok(CallerOutcome::Unknown(
                        "owner disappeared before terminal response",
                    ))
                }
                None => host.wait(deadline)?,
            }
            // After destruction, keep draining available replies before deciding
            // loss. The same deadline bounds even malicious continuous traffic.
        }
    })();
    let cleanup = host.cleanup();
    if let Err(error) = &cleanup {
        eprintln!("[ERROR] Caller cleanup unconfirmed: {error}");
    }
    match (operation, cleanup) {
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        (Ok(outcome), Ok(())) => Ok(outcome),
    }
}

pub fn invoke(command: Command) -> Result<CallerOutcome> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or("Invalid X11 screen")?;
    let mut host = NativeCaller {
        peer: NativePeer {
            conn: &conn,
            selection: 0,
        },
        screen,
        atoms: TransportAtoms::default(),
        reply: None,
    };
    caller_with(
        &mut host,
        screen.root,
        u32::try_from(screen_num)?,
        command,
        || fresh_request_id(&mut File::open("/dev/urandom")?),
    )
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
#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::collections::VecDeque;
    use x11rb::protocol::xproto::DestroyNotifyEvent;

    const ROOT: u32 = 55;
    const CONTROL: u32 = 10;
    const REPLY_WINDOW: u32 = 90;
    const ID: u64 = 0x1234_5678_9abc_def0;
    fn atoms() -> TransportAtoms {
        TransportAtoms {
            request: 40,
            reply: 41,
            caller: 42,
        }
    }
    fn owner() -> OwnerDescriptor {
        OwnerDescriptor {
            screen_num: 0,
            control_xid: CONTROL,
            epoch: 77,
            lifecycle: Lifecycle::Ready,
            body_xid: Some(88),
        }
    }
    fn request(command: Command) -> Request {
        Request {
            command,
            request_id: ID,
            reply_xid: REPLY_WINDOW,
            epoch: 77,
        }
    }
    fn marker(command: Command) -> CallerMarker {
        CallerMarker {
            control_xid: CONTROL,
            epoch: 77,
            request_id: ID,
            command,
        }
    }
    fn evidence(words: &[u32]) -> EndpointEvidence {
        EndpointEvidence {
            root: ROOT,
            parent: ROOT,
            class: WindowClass::INPUT_ONLY,
            map_state: MapState::UNMAPPED,
            property: GetPropertyReply {
                format: 32,
                type_: AtomEnum::CARDINAL.into(),
                value_len: words.len() as u32,
                value: words.iter().flat_map(|word| word.to_ne_bytes()).collect(),
                ..GetPropertyReply::default()
            },
        }
    }
    fn response() -> TerminalResponse {
        TerminalResponse {
            status: Status::Failed,
            request_id: ID,
            control_xid: CONTROL,
            detail: control::preflight_detail(Reason::Unsupported, Stage::Preflight),
        }
    }
    fn terminal() -> Event {
        Event::ClientMessage(reply_event(atoms(), REPLY_WINDOW, response()).unwrap())
    }
    fn destroyed(synthetic: bool) -> Event {
        Event::DestroyNotify(DestroyNotifyEvent {
            response_type: 17 | if synthetic { 0x80 } else { 0 },
            sequence: 0,
            event: CONTROL,
            window: CONTROL,
        })
    }
    struct Fake {
        now: Instant,
        owner: u32,
        owners: VecDeque<u32>,
        evidence: VecDeque<Option<EndpointEvidence>>,
        events: VecDeque<Event>,
        sent: Vec<ClientMessageEvent>,
        marker: Option<CallerMarker>,
        log: Vec<&'static str>,
        waits: Vec<Instant>,
        cost: Option<(&'static str, Duration)>,
        endless_events: bool,
        watch_ok: bool,
        send_ok: bool,
        failure: Option<&'static str>,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                now: Instant::now(),
                owner: CONTROL,
                owners: VecDeque::new(),
                evidence: VecDeque::new(),
                events: VecDeque::new(),
                sent: vec![],
                marker: None,
                log: vec![],
                waits: vec![],
                cost: None,
                endless_events: false,
                watch_ok: true,
                send_ok: true,
                failure: None,
            }
        }
        fn call(&mut self, name: &'static str) -> Result<()> {
            self.log.push(name);
            if let Some((target, cost)) = self.cost {
                if target == name {
                    self.now += cost;
                }
            }
            if self.failure == Some(name) {
                Err(name.into())
            } else {
                Ok(())
            }
        }
        fn route(
            &mut self,
            active: &mut Option<ActiveOperation>,
            lifecycle: Lifecycle,
            busy: bool,
            event: &ClientMessageEvent,
        ) -> Result<bool> {
            route_request_with(
                self,
                OwnerDescriptor {
                    lifecycle,
                    ..owner()
                },
                ROOT,
                atoms(),
                Some(99),
                busy,
                active,
                event,
            )
        }
        fn call_command(&mut self) -> Result<CallerOutcome> {
            caller_with(self, ROOT, 0, Command::Show, || Ok(ID))
        }
    }
    impl PeerHost for Fake {
        fn now(&self) -> Instant {
            self.now
        }
        fn owner(&mut self) -> Result<u32> {
            self.call("owner")?;
            Ok(self.owners.pop_front().unwrap_or(self.owner))
        }
        fn evidence(&mut self, window: u32, property: u32) -> Result<Option<EndpointEvidence>> {
            self.call("evidence")?;
            if let Some(evidence) = self.evidence.pop_front() {
                return Ok(evidence);
            }
            Ok(Some(if window == CONTROL {
                evidence(&owner().encode().unwrap())
            } else {
                assert_eq!(property, atoms().caller);
                evidence(&marker(Command::Show).encode().unwrap())
            }))
        }
        fn send(&mut self, event: ClientMessageEvent) -> Result<bool> {
            self.sent.push(event);
            self.call("send")?;
            Ok(self.send_ok)
        }
    }
    impl CallerHost for Fake {
        fn atoms(&mut self, _: u32) -> Result<(u32, TransportAtoms)> {
            self.call("atoms")?;
            Ok((30, atoms()))
        }
        fn watch(&mut self, _: u32) -> Result<bool> {
            self.call("watch")?;
            Ok(self.watch_ok)
        }
        fn create_reply(&mut self, marker: CallerMarker) -> Result<u32> {
            self.call("create reply")?;
            assert!(
                self.marker.replace(marker).is_none(),
                "one immutable marker only"
            );
            Ok(REPLY_WINDOW)
        }
        fn next(&mut self) -> Result<Option<Event>> {
            self.call("next")?;
            Ok(self.events.pop_front().or_else(|| {
                self.endless_events.then(|| {
                    let mut event = reply_event(atoms(), REPLY_WINDOW, response()).unwrap();
                    event.type_ += 1;
                    Event::ClientMessage(event)
                })
            }))
        }
        fn wait(&mut self, deadline: Instant) -> Result<()> {
            self.call("wait")?;
            self.waits.push(deadline);
            self.now = deadline;
            Ok(())
        }
        fn cleanup(&mut self) -> Result<()> {
            self.call("cleanup")
        }
    }

    #[test]
    fn discovery_lifecycle_and_absence_never_create_or_send() {
        for lifecycle in [Lifecycle::Starting, Lifecycle::Closing] {
            let mut host = Fake::new();
            host.evidence.push_back(Some(evidence(
                &OwnerDescriptor {
                    lifecycle,
                    ..owner()
                }
                .encode()
                .unwrap(),
            )));
            assert_eq!(
                host.call_command().unwrap(),
                CallerOutcome::Lifecycle(lifecycle)
            );
            assert!(host.sent.is_empty());
            assert!(host.marker.is_none());
            assert_eq!(host.log.last(), Some(&"cleanup"));
        }
        let mut host = Fake::new();
        host.owner = 0;
        assert_eq!(host.call_command().unwrap(), CallerOutcome::NoInstance);
        assert!(host.marker.is_none());
        assert!(host.sent.is_empty());
        assert_eq!(CallerOutcome::NoInstance.exit_code(), ExitCode::NoInstance);
    }
    #[test]
    fn discovery_rejects_every_malformed_descriptor_and_native_endpoint() {
        let valid = owner().encode().unwrap();
        let mut variants = vec![];
        for (index, word) in [(0, 0), (1, 2), (2, 1), (3, 11), (4, 0), (5, 99), (6, 0)] {
            let mut words = valid;
            words[index] = word;
            variants.push(Some(evidence(&words)));
        }
        variants.push(None);
        for case in 0..10 {
            let mut e = evidence(&valid);
            match case {
                0 => e.root += 1,
                1 => e.parent += 1,
                2 => e.class = WindowClass::INPUT_OUTPUT,
                3 => e.map_state = MapState::VIEWABLE,
                4 => e.property.type_ = AtomEnum::STRING.into(),
                5 => e.property.format = 8,
                6 => e.property.value_len = 6,
                7 => e.property.bytes_after = 4,
                8 => e.property.value.push(0),
                9 => e.property.value.clear(),
                _ => unreachable!(),
            }
            variants.push(Some(e));
        }
        for e in variants {
            let mut host = Fake::new();
            host.evidence.push_back(e);
            assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
            assert!(host.sent.is_empty());
            assert!(host.marker.is_none());
        }
        for replacement in [0, 11] {
            let mut host = Fake::new();
            host.owners = VecDeque::from([CONTROL, replacement]);
            assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
            assert!(host.sent.is_empty());
        }
    }
    #[test]
    fn ready_caller_sends_exactly_once_and_accepts_real_correlated_terminal() {
        let mut host = Fake::new();
        host.events.push_back(terminal());
        assert_eq!(
            host.call_command().unwrap(),
            CallerOutcome::Terminal(response())
        );
        assert_eq!(host.marker, Some(marker(Command::Show)));
        assert_eq!(host.sent.len(), 1);
        let event = host.sent[0];
        assert_eq!(
            (event.window, event.type_, event.format),
            (CONTROL, atoms().request, 32)
        );
        assert_eq!(
            event.data.as_data32(),
            [
                (1 << 16) | 2,
                ID as u32,
                (ID >> 32) as u32,
                REPLY_WINDOW,
                77
            ]
        );
        assert_eq!(host.log.iter().filter(|&&x| x == "create reply").count(), 1);
        assert_eq!(host.log.last(), Some(&"cleanup"));
        assert_eq!(
            CallerOutcome::Terminal(response()).exit_code(),
            ExitCode::OperationFailed
        );
    }
    #[test]
    fn caller_window_is_exact_unmapped_input_only_without_input_subscriptions() {
        let screen = Screen {
            root: ROOT,
            root_visual: 66,
            ..Screen::default()
        };
        let r = caller_request(REPLY_WINDOW, &screen);
        assert_eq!((r.wid, r.parent, r.visual), (REPLY_WINDOW, ROOT, 66));
        assert_eq!((r.depth, r.border_width, r.width, r.height), (0, 0, 1, 1));
        assert_eq!(r.class, WindowClass::INPUT_ONLY);
        assert_eq!(r.value_list.event_mask, None);
        assert_eq!(
            marker(Command::Show).encode().unwrap(),
            [
                control::MAGIC,
                1,
                CONTROL,
                77,
                ID as u32,
                (ID >> 32) as u32,
                2
            ]
        );
        // CreateWindow has no mapped flag; neither CallerHost nor PeerHost offers MapWindow,
        // selection acquisition, alpha visual, body, renderer, focus, or keyboard work.
    }
    #[test]
    fn random_ids_regenerate_zero_and_use_all_64_bits_with_io_errors_preserved() {
        let mut bytes = Vec::from(0u64.to_ne_bytes());
        bytes.extend(ID.to_ne_bytes());
        bytes.extend((ID + 1).to_ne_bytes());
        let mut bytes = bytes.as_slice();
        assert_eq!(fresh_request_id(&mut bytes).unwrap(), ID);
        assert_eq!(fresh_request_id(&mut bytes).unwrap(), ID + 1);
        assert!(fresh_request_id(&mut bytes).is_err());
    }
    #[test]
    fn caller_cleanup_runs_on_setup_send_and_wait_failure_and_retains_original_error() {
        for failure in [
            "atoms",
            "owner",
            "evidence",
            "watch",
            "create reply",
            "send",
            "next",
            "wait",
            "cleanup",
        ] {
            let mut host = Fake::new();
            host.failure = Some(failure);
            let error = host.call_command().unwrap_err();
            assert_eq!(error.to_string(), failure);
            assert_eq!(host.log.last(), Some(&"cleanup"));
            assert!(host.sent.len() <= 1);
        }
    }
    #[test]
    fn request_envelope_garbage_and_stale_identity_cannot_admit_or_reply() {
        let good = request_event(atoms(), CONTROL, request(Command::Show)).unwrap();
        for case in 0..6 {
            let mut event = good;
            match case {
                0 => event.window += 1,
                1 => event.type_ += 1,
                2 => event.format = 8,
                3 => {
                    let mut w = event.data.as_data32();
                    w[4] += 1;
                    event.data = w.into();
                }
                4 => {
                    let mut w = event.data.as_data32();
                    w[1] = 0;
                    w[2] = 0;
                    event.data = w.into();
                }
                5 => {
                    let mut w = event.data.as_data32();
                    w[3] = 0;
                    event.data = w.into();
                }
                _ => unreachable!(),
            }
            let mut host = Fake::new();
            let mut active = None;
            host.route(&mut active, Lifecycle::Ready, false, &event)
                .unwrap();
            assert!(active.is_none());
            assert!(host.sent.is_empty());
        }
    }
    #[test]
    fn reply_route_requires_exact_native_evidence_and_immutable_identity() {
        let req = request(Command::Show);
        let words = req.encode().unwrap();
        let valid = marker(Command::Show).encode().unwrap();
        let mut variants = vec![None];
        for (index, word) in [
            (0, 0),
            (1, 2),
            (2, CONTROL + 1),
            (3, 78),
            (4, 0),
            (5, 0),
            (6, 1),
        ] {
            let mut w = valid;
            w[index] = word;
            variants.push(Some(evidence(&w)));
        }
        for case in 0..11 {
            let mut e = evidence(&valid);
            match case {
                0 => e.root += 1,
                1 => e.parent += 1,
                2 => e.class = WindowClass::INPUT_OUTPUT,
                3 => e.map_state = MapState::UNVIEWABLE,
                4 => e.property.type_ = 0,
                5 => e.property.type_ = AtomEnum::STRING.into(),
                6 => e.property.format = 16,
                7 => e.property.value_len = 8,
                8 => e.property.bytes_after = 4,
                9 => e.property.value.truncate(24),
                10 => e.property.value.extend([0; 4]),
                _ => unreachable!(),
            }
            variants.push(Some(e));
        }
        for e in variants {
            let mut host = Fake::new();
            host.evidence.push_back(e);
            let mut active = None;
            host.route(
                &mut active,
                Lifecycle::Ready,
                false,
                &request_event(atoms(), CONTROL, req).unwrap(),
            )
            .unwrap();
            assert!(active.is_none());
            assert!(host.sent.is_empty());
        }
        for forbidden in [0, ROOT, CONTROL, 88, 99] {
            assert!(
                validated_route(&evidence(&valid), ROOT, owner(), Some(99), forbidden, words)
                    .is_none()
            );
        }
        assert_eq!(
            validated_route(
                &evidence(&valid),
                ROOT,
                owner(),
                Some(99),
                REPLY_WINDOW,
                words
            ),
            Some((req, None))
        );
    }
    #[test]
    fn incompatible_headers_get_protocol_error_only_via_valid_v1_route() {
        for (header, reason) in [
            ((2 << 16) | 2, Reason::WrongVersion),
            ((1 << 16) | 99, Reason::UnknownCommand),
        ] {
            for safe in [true, false] {
                let mut host = Fake::new();
                if !safe {
                    host.evidence.push_back(None);
                }
                let mut event = request_event(atoms(), CONTROL, request(Command::Show)).unwrap();
                let mut words = event.data.as_data32();
                words[0] = header;
                event.data = words.into();
                let mut active = None;
                host.route(&mut active, Lifecycle::Ready, false, &event)
                    .unwrap();
                assert!(active.is_none());
                assert_eq!(host.sent.len(), usize::from(safe));
                if safe {
                    let reply = TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap();
                    assert_eq!(
                        (reply.status, reply.detail.reason),
                        (Status::ProtocolError, reason)
                    );
                }
            }
        }
    }
    #[test]
    fn all_ready_commands_have_one_truthful_unsupported_reply_and_clear_active() {
        for command in [Command::Hide, Command::Show, Command::BringTop] {
            let mut host = Fake::new();
            host.evidence
                .push_back(Some(evidence(&marker(command).encode().unwrap())));
            let event = request_event(atoms(), CONTROL, request(command)).unwrap();
            let mut active = None;
            host.route(&mut active, Lifecycle::Ready, false, &event)
                .unwrap();
            assert!(active.is_none());
            assert_eq!(host.sent.len(), 1);
            let reply = host.sent[0];
            assert_eq!(
                (reply.window, reply.type_, reply.format),
                (REPLY_WINDOW, atoms().reply, 32)
            );
            assert_eq!(
                TerminalResponse::decode(&reply.data.as_data32()).unwrap(),
                response()
            );
            assert_eq!(
                reply.data.as_data32(),
                [(1 << 16) | 1, ID as u32, (ID >> 32) as u32, CONTROL, 11]
            );
            assert_eq!(host.log, ["owner", "evidence", "owner", "send"]);
        }
    }
    #[test]
    fn starting_closing_and_local_layer_busy_reply_without_admission() {
        for (lifecycle, busy, status, reason, stage) in [
            (
                Lifecycle::Starting,
                false,
                Status::Starting,
                Reason::Starting,
                Stage::Preflight,
            ),
            (
                Lifecycle::Closing,
                false,
                Status::Closing,
                Reason::Closing,
                Stage::Shutdown,
            ),
            (
                Lifecycle::Ready,
                true,
                Status::Busy,
                Reason::Busy,
                Stage::Preflight,
            ),
        ] {
            let mut host = Fake::new();
            let before = host.now;
            let mut active = None;
            host.route(
                &mut active,
                lifecycle,
                busy,
                &request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
            )
            .unwrap();
            let reply = TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap();
            assert_eq!(
                (reply.status, reply.detail.reason, reply.detail.stage),
                (status, reason, stage)
            );
            assert!(active.is_none());
            assert_eq!(host.now, before);
            assert_eq!(reply.detail.progress, control::Progress::default());
        }
    }
    #[test]
    fn active_duplicate_is_ignored_competitor_busy_and_completed_replay_is_new() {
        let mut host = Fake::new();
        let req = request(Command::Show);
        let mut active = None;
        assert_eq!(
            control::admit(&mut active, req, Lifecycle::Ready, false, host.now),
            Admission::Admitted
        );
        let original = active;
        host.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, req).unwrap(),
        )
        .unwrap();
        assert_eq!(active, original);
        assert!(host.sent.is_empty());
        let other = Request {
            request_id: ID + 1,
            ..req
        };
        host.evidence.push_back(Some(evidence(
            &CallerMarker {
                request_id: ID + 1,
                ..marker(Command::Show)
            }
            .encode()
            .unwrap(),
        )));
        host.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, other).unwrap(),
        )
        .unwrap();
        assert_eq!(active, original);
        assert_eq!(host.sent.len(), 1);
        assert_eq!(
            TerminalResponse::decode(&host.sent[0].data.as_data32())
                .unwrap()
                .status,
            Status::Busy
        );
        finish_active(&mut host, atoms(), &mut active, response()).unwrap();
        assert!(active.is_none());
        host.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, req).unwrap(),
        )
        .unwrap();
        assert_eq!(host.sent.len(), 3);
        assert!(active.is_none());
    }
    #[test]
    fn admission_deadline_is_fixed_at_admission_and_not_extended_by_duplicates() {
        for (command, seconds) in [
            (Command::Hide, 1),
            (Command::Show, 1),
            (Command::BringTop, 2),
        ] {
            let now = Instant::now();
            let req = request(command);
            let mut active = None;
            assert_eq!(
                control::admit(&mut active, req, Lifecycle::Ready, false, now),
                Admission::Admitted
            );
            assert_eq!(active.unwrap().deadline, now + Duration::from_secs(seconds));
            assert_eq!(active.unwrap().stage, Stage::Preflight);
            assert_eq!(
                control::admit(
                    &mut active,
                    req,
                    Lifecycle::Ready,
                    false,
                    now + Duration::from_millis(500)
                ),
                Admission::Duplicate
            );
            assert_eq!(active.unwrap().deadline, now + Duration::from_secs(seconds));
        }
    }
    #[test]
    fn ownership_loss_before_or_during_validation_never_admits() {
        for owners in [[0, 0], [CONTROL, 0], [CONTROL, 11]] {
            let mut host = Fake::new();
            host.owners = VecDeque::from(owners);
            let mut active = None;
            assert!(host
                .route(
                    &mut active,
                    Lifecycle::Ready,
                    false,
                    &request_event(atoms(), CONTROL, request(Command::Show)).unwrap()
                )
                .is_err());
            assert!(active.is_none());
            assert!(host.sent.is_empty());
        }
    }
    #[test]
    fn vanished_caller_after_admission_is_nonfatal_but_other_send_failure_is_fatal() {
        for failure in [false, true] {
            let mut host = Fake::new();
            host.send_ok = false;
            if failure {
                host.failure = Some("send");
            }
            let mut active = None;
            let result = host.route(
                &mut active,
                Lifecycle::Ready,
                false,
                &request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
            );
            assert_eq!(result.is_err(), failure);
            assert!(active.is_none());
            assert_eq!(host.sent.len(), 1);
            assert_eq!(
                TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap(),
                response()
            );
        }
    }
    #[test]
    fn only_typed_badwindow_for_the_expected_peer_is_recoverable() {
        for (kind, bad_value, recoverable) in [
            (ErrorKind::Window, REPLY_WINDOW, true),
            (ErrorKind::Window, CONTROL, false),
            (ErrorKind::Atom, REPLY_WINDOW, false),
        ] {
            let error = x11rb::x11_utils::X11Error {
                error_kind: kind,
                error_code: 3,
                sequence: 1,
                bad_value,
                minor_opcode: 0,
                major_opcode: 25,
                extension_name: None,
                request_name: None,
            };
            let result = peer_result::<()>(Err(ReplyError::X11Error(error)), REPLY_WINDOW);
            if recoverable {
                assert_eq!(result.unwrap(), None);
            } else {
                assert!(result.is_err());
            }
        }
        assert!(peer_result::<()>(
            Err(ReplyError::ConnectionError(
                x11rb::errors::ConnectionError::UnknownError
            )),
            REPLY_WINDOW
        )
        .is_err());
    }
    #[test]
    fn correlation_ignores_all_unrelated_envelopes_and_stops_at_first_valid_reply() {
        let good = reply_event(atoms(), REPLY_WINDOW, response()).unwrap();
        let mut host = Fake::new();
        for case in 0..7 {
            let mut event = good;
            match case {
                0 => event.window += 1,
                1 => event.type_ += 1,
                2 => event.format = 8,
                3 => {
                    let mut w = event.data.as_data32();
                    w[0] = (2 << 16) | 1;
                    event.data = w.into();
                }
                4 => {
                    let mut w = event.data.as_data32();
                    w[1] += 1;
                    event.data = w.into();
                }
                5 => {
                    let mut w = event.data.as_data32();
                    w[3] += 1;
                    event.data = w.into();
                }
                6 => {
                    let mut w = event.data.as_data32();
                    w[4] |= 1 << 31;
                    event.data = w.into();
                }
                _ => unreachable!(),
            }
            host.events.push_back(Event::ClientMessage(event));
        }
        host.events.extend([terminal(), terminal()]);
        assert_eq!(
            host.call_command().unwrap(),
            CallerOutcome::Terminal(response())
        );
        assert_eq!(host.events.len(), 1);
        assert_eq!(host.sent.len(), 1);
        let correlation = ReplyCorrelation {
            control_xid: CONTROL,
            epoch: 78,
            request_id: ID,
            reply_xid: REPLY_WINDOW,
        };
        assert!(correlated_reply(&terminal(), atoms(), owner(), correlation).is_none());
    }
    #[test]
    fn one_four_second_budget_includes_discovery_setup_send_and_unrelated_traffic() {
        for step in ["atoms", "owner", "evidence", "watch", "create reply"] {
            let mut host = Fake::new();
            host.cost = Some((step, Duration::from_secs(4)));
            assert!(matches!(
                host.call_command().unwrap(),
                CallerOutcome::Unknown(_)
            ));
            assert!(host.sent.is_empty(), "no send after budget used by {step}");
        }
        let mut host = Fake::new();
        let start = host.now;
        host.cost = Some(("create reply", Duration::from_secs(3)));
        assert!(matches!(
            host.call_command().unwrap(),
            CallerOutcome::Unknown(_)
        ));
        assert_eq!(host.waits, [start + Duration::from_secs(4)]);
        assert_eq!(host.sent.len(), 1);
        let mut host = Fake::new();
        let start = host.now;
        host.cost = Some(("next", Duration::from_millis(10)));
        host.endless_events = true;
        assert!(matches!(
            host.call_command().unwrap(),
            CallerOutcome::Unknown(_)
        ));
        assert_eq!(host.now, start + Duration::from_secs(4));
        assert_eq!(host.sent.len(), 1);
        assert!(host.waits.is_empty());
    }
    #[test]
    fn owner_disappearance_before_send_never_sends_to_stale_or_replacement_owner() {
        let mut host = Fake::new();
        host.watch_ok = false;
        assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
        assert!(host.sent.is_empty());
        let mut host = Fake::new();
        host.owners = VecDeque::from([CONTROL, CONTROL, 0]);
        assert_eq!(host.call_command().unwrap(), CallerOutcome::NoInstance);
        assert!(host.sent.is_empty());
        let mut host = Fake::new();
        host.evidence = VecDeque::from([Some(evidence(&owner().encode().unwrap())), None]);
        assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
        assert!(host.sent.is_empty());
        let mut host = Fake::new();
        host.evidence = VecDeque::from([
            Some(evidence(&owner().encode().unwrap())),
            Some(evidence(
                &OwnerDescriptor {
                    epoch: 78,
                    ..owner()
                }
                .encode()
                .unwrap(),
            )),
        ]);
        assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
        assert!(host.sent.is_empty());
    }
    #[test]
    fn owner_death_drains_buffered_terminal_first_without_resend() {
        for events in [
            vec![terminal(), destroyed(false)],
            vec![destroyed(false), terminal()],
            vec![destroyed(true), terminal()],
        ] {
            let mut host = Fake::new();
            host.events = events.into();
            assert_eq!(
                host.call_command().unwrap(),
                CallerOutcome::Terminal(response())
            );
            assert_eq!(host.sent.len(), 1);
        }
        let mut host = Fake::new();
        host.events.push_back(destroyed(false));
        assert_eq!(
            host.call_command().unwrap(),
            CallerOutcome::Unknown("owner disappeared before terminal response")
        );
        assert!(host.waits.is_empty());
        assert_eq!(host.sent.len(), 1);
        let mut host = Fake::new();
        host.send_ok = false;
        assert!(matches!(
            host.call_command().unwrap(),
            CallerOutcome::Unknown(_)
        ));
        assert_eq!(host.sent.len(), 1);
    }
    #[test]
    fn shutdown_completes_active_once_and_clears_even_on_reply_failure() {
        for fails in [false, true] {
            let mut host = Fake::new();
            if fails {
                host.failure = Some("send");
            }
            let mut active = None;
            control::admit(
                &mut active,
                request(Command::Show),
                Lifecycle::Ready,
                false,
                host.now,
            );
            let closing = TerminalResponse {
                status: Status::Closing,
                detail: control::preflight_detail(Reason::Closing, Stage::Shutdown),
                ..response()
            };
            assert_eq!(
                finish_active(&mut host, atoms(), &mut active, closing).is_err(),
                fails
            );
            assert!(active.is_none());
            assert_eq!(host.sent.len(), 1);
        }
    }
    #[test]
    fn direct_native_request_and_reply_use_no_propagation_or_event_mask() {
        for event in [
            request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
            reply_event(atoms(), REPLY_WINDOW, response()).unwrap(),
        ] {
            let native = direct_message(event);
            assert!(!native.propagate);
            assert_eq!(native.destination, event.window);
            assert_eq!(native.event_mask, EventMask::NO_EVENT);
            let bytes: [u8; 32] = event.into();
            assert_eq!(*native.event, bytes);
        }
    }

    #[test]
    fn direct_starting_requests_do_not_extend_the_startup_wait() {
        use std::cell::{Cell, RefCell};
        let host = RefCell::new(Fake::new());
        let start = host.borrow().now;
        let now = Cell::new(start);
        let deadline = start + Duration::from_secs(2);
        let event = request_event(atoms(), CONTROL, request(Command::Show)).unwrap();
        let mut active = None;
        let result = wait_events_with::<()>(
            deadline,
            || now.get(),
            || {
                now.set(now.get() + Duration::from_millis(100));
                Ok(Some((Event::ClientMessage(event), 1)))
            },
            |_| panic!("continuous event traffic"),
            |event, _| {
                let Event::ClientMessage(event) = event else {
                    unreachable!()
                };
                host.borrow_mut()
                    .route(&mut active, Lifecycle::Starting, false, &event)?;
                Ok(None)
            },
        )
        .unwrap();
        assert_eq!(result, None);
        assert_eq!(now.get(), deadline);
        assert!(active.is_none());
        assert_eq!(host.borrow().sent.len(), 20);
        for reply in &host.borrow().sent {
            assert_eq!(
                TerminalResponse::decode(&reply.data.as_data32())
                    .unwrap()
                    .status,
                Status::Starting
            );
        }
    }
    #[test]
    fn closing_boundary_drains_only_one_bounded_batch_without_admitting() {
        let event =
            Event::ClientMessage(request_event(atoms(), CONTROL, request(Command::Show)).unwrap());
        let mut queue = VecDeque::from(vec![event; 100]);
        let mut host = Fake::new();
        let mut active = None;
        closing_events_with(
            || Ok(queue.pop_front()),
            |event| {
                let Event::ClientMessage(event) = event else {
                    unreachable!()
                };
                host.route(&mut active, Lifecycle::Closing, false, &event)?;
                Ok(false)
            },
        )
        .unwrap();
        assert_eq!(queue.len(), 36);
        assert!(active.is_none());
        assert_eq!(host.sent.len(), 64);
        for reply in host.sent {
            let reply = TerminalResponse::decode(&reply.data.as_data32()).unwrap();
            assert_eq!(reply.status, Status::Closing);
            assert_eq!(reply.detail.stage, Stage::Shutdown);
        }
    }
}

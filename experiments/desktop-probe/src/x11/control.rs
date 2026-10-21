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

use super::monitors::destination::{CallerDesktop, CallerDesktopAtoms, DestinationError};
#[cfg(test)]
use crate::control::ExitCode;
use crate::control::{
    self, selection_name, ActiveOperation, Admission, CallerMarker, Command, Lifecycle,
    OwnerDescriptor, Reason, ReplyCorrelation, Request, Stage, Status, TerminalResponse, INSTANCE,
    TIMESTAMP,
};
use crate::x11::resource::{cleanup_all, OwnedResource};
use crate::HostError;

mod outcome;
pub use outcome::CallerOutcome;

type Result<T> = std::result::Result<T, HostError>;
const PROBE_BUDGET: Duration = Duration::from_secs(1);
const ACQUISITION_BUDGET: Duration = Duration::from_secs(2);

/// A successful native lookup proved owner loss. Query errors remain their
/// original HostError; the owner finalizer must not classify this as X11Failure.
#[derive(Debug)]
pub(crate) struct OwnershipLost(&'static str);

impl std::fmt::Display for OwnershipLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for OwnershipLost {}

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

    pub fn active(&self) -> Option<&ActiveOperation> {
        self.active.as_ref()
    }

    pub fn active_mut(&mut self) -> Option<&mut ActiveOperation> {
        self.active.as_mut()
    }

    /// Borrow the sole placement slot directly into the reviewed owner finalizer.
    /// Only that finalizer clears it; the reply closure owns correlation values,
    /// not a second operation, and cannot redirect the reply to another caller.
    pub fn service_here_with(
        &mut self,
        conn: &impl Connection,
        service: impl FnOnce(
            &mut Option<ActiveOperation>,
            u32,
            u32,
            &mut dyn FnMut(control::OperationResult) -> Result<()>,
        ) -> Result<()>,
    ) -> Result<bool> {
        let selection = self.selection;
        self.service_here_with_peer(&mut NativePeer { conn, selection }, service)
    }

    fn service_here_with_peer(
        &mut self,
        host: &mut impl PeerHost,
        service: impl FnOnce(
            &mut Option<ActiveOperation>,
            u32,
            u32,
            &mut dyn FnMut(control::OperationResult) -> Result<()>,
        ) -> Result<()>,
    ) -> Result<bool> {
        let Some(operation) = self.active.as_ref() else {
            return Ok(false);
        };
        if !matches!(
            operation.request.command,
            Command::BringHere | Command::Recover
        ) {
            return Ok(false);
        }
        let request = operation.request;
        let control_xid = self.descriptor.control_xid;
        let atoms = self.transport;
        service(
            &mut self.active,
            self.selection,
            control_xid,
            &mut |result| {
                host.send(reply_event(
                    atoms,
                    request.reply_xid,
                    TerminalResponse {
                        status: result.status(),
                        detail: result.detail(),
                        request_id: request.request_id,
                        control_xid,
                    },
                )?)?;
                Ok(())
            },
        )?;
        Ok(true)
    }

    pub fn verify_active_ownership(&mut self, conn: &impl Connection) -> Result<()> {
        if self.active.is_some()
            && conn.get_selection_owner(self.selection)?.reply()?.owner
                != self.descriptor.control_xid
        {
            self.owns_selection = false;
            return Err("Control selection ownership lost during visibility operation".into());
        }
        Ok(())
    }

    pub fn complete(
        &mut self,
        conn: &impl Connection,
        result: control::OperationResult,
    ) -> Result<()> {
        let operation = self.active.as_ref().ok_or("Missing admitted operation")?;
        let response = TerminalResponse {
            status: result.status(),
            detail: result.detail(),
            request_id: operation.request.request_id,
            control_xid: self.descriptor.control_xid,
        };
        finish_active(
            &mut NativePeer {
                conn,
                selection: self.selection,
            },
            self.transport,
            &mut self.active,
            response,
        )
    }

    /// Shutdown attempts one terminal reply without waiting or retrying.
    pub fn finish_remote(&mut self, conn: &impl Connection) -> Result<()> {
        let selection = self.selection;
        self.finish_remote_with(&mut NativePeer { conn, selection })
    }

    fn finish_remote_with(&mut self, host: &mut impl PeerHost) -> Result<()> {
        let Some(operation) = self.active.as_mut() else {
            return Ok(());
        };
        if matches!(
            operation.request.command,
            Command::BringHere | Command::Recover
        ) {
            let result = operation
                .here_shutdown(if self.owns_selection {
                    Reason::Closing
                } else {
                    Reason::OwnershipLost
                })
                .expect("Here shutdown clears current proof and retains relocation history");
            let response = TerminalResponse {
                status: result.status(),
                detail: result.detail(),
                request_id: operation.request.request_id,
                control_xid: self.descriptor.control_xid,
            };
            return finish_active(host, self.transport, &mut self.active, response);
        }
        if operation.request.command == Command::BringTop {
            operation.detail = control::Detail {
                progress: control::Progress {
                    host_mutation_dispatched: operation.detail.progress.host_mutation_dispatched,
                    ..control::Progress::default()
                },
                ..control::preflight_detail(
                    if self.owns_selection {
                        Reason::Closing
                    } else {
                        Reason::OwnershipLost
                    },
                    Stage::Shutdown,
                )
            };
            let result = control::OperationResult::bring_top(
                if operation.bring_top.minimized_at_start
                    && operation.bring_top.restoration_confirmed
                {
                    Status::Partial
                } else {
                    Status::Failed
                },
                operation.detail,
                control::Confirmation::default(),
                &operation.bring_top,
            )
            .expect("Shutdown retains restoration history without stale current evidence");
            let response = TerminalResponse {
                status: result.status(),
                detail: result.detail(),
                request_id: operation.request.request_id,
                control_xid: self.descriptor.control_xid,
            };
            return finish_active(host, self.transport, &mut self.active, response);
        }
        let response = TerminalResponse {
            status: Status::Closing,
            request_id: operation.request.request_id,
            control_xid: self.descriptor.control_xid,
            detail: control::Detail {
                reason: Reason::Closing,
                stage: Stage::Shutdown,
                ..operation.detail
            },
        };
        finish_active(host, self.transport, &mut self.active, response)
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
    fn words(&self, length: u32) -> Option<Vec<u32>> {
        let p = &self.property;
        if p.type_ != u32::from(AtomEnum::CARDINAL)
            || p.format != 32
            || p.value_len != length
            || p.bytes_after != 0
            || p.value.len() != length as usize * 4
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
                .get_property(false, window, property, AtomEnum::ANY, 0, 8)?
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
) -> Option<(Request, u32, Option<Reason>)> {
    if reply == 0
        || reply == root
        || reply == owner.control_xid
        || Some(reply) == owner.body_xid
        || Some(reply) == popup
        || !evidence.endpoint_matches(root)
    {
        return None;
    }
    let marker = CallerMarker::decode(&evidence.words(8)?).ok()?;
    let id = u64::from(words[1]) | (u64::from(words[2]) << 32);
    if marker.control_xid != owner.control_xid
        || marker.epoch != owner.epoch
        || words[4] != owner.epoch
        || marker.request_id != id
    {
        return None;
    }
    // A valid v2 marker supplies safe correlation for an incompatible request
    // header. Incompatible markers themselves are never translated or admitted.
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
    marker.matches(owner.control_xid, request).then_some((
        request,
        marker.destination_desktop,
        error,
    ))
}

fn finish_active(
    host: &mut impl PeerHost,
    atoms: TransportAtoms,
    active: &mut Option<ActiveOperation>,
    response: TerminalResponse,
) -> Result<()> {
    let operation = active.take().ok_or("Missing admitted operation")?;
    // The sole slot is already clear through encoding/send failure and cleanup.
    reply_event(atoms, operation.request.reply_xid, response)
        .and_then(|event| host.send(event).map(|_| ()))
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
        return Err(Box::new(OwnershipLost(
            "Control selection ownership lost before admission",
        )));
    }
    let Some(evidence) = host.evidence(words[3], atoms.caller)? else {
        return Ok(true);
    };
    let Some((request, destination_desktop, protocol_error)) =
        validated_route(&evidence, root, owner, popup, words[3], words)
    else {
        return Ok(true);
    };
    // Synchronous validation may have overlapped owner loss; admission requires
    // current ownership, not just the lookup before reading the caller marker.
    if host.owner()? != owner.control_xid {
        return Err(Box::new(OwnershipLost(
            "Control selection ownership lost during validation",
        )));
    }
    let admission = match protocol_error {
        Some(reason) => Admission::Reply(Status::ProtocolError, reason, Stage::Preflight),
        None => control::admit(
            active,
            request,
            destination_desktop,
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
        Admission::Admitted => {
            let operation = active.as_mut().unwrap();
            operation.stage = match request.command {
                Command::Hide => Stage::Hide,
                Command::BringHere | Command::Recover => Stage::Placement,
                _ => Stage::Show,
            };
            operation.detail = control::preflight_detail(Reason::None, operation.stage);
            return Ok(true);
        }
    };
    let response = TerminalResponse {
        status,
        request_id: request.request_id,
        control_xid: owner.control_xid,
        detail,
    };
    host.send(reply_event(atoms, request.reply_xid, response)?)?;
    Ok(true)
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
        .words(7)
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
    fn capture_desktop(&mut self) -> std::result::Result<CallerDesktop, DestinationError>;
    fn revalidate_desktop(
        &mut self,
        captured: CallerDesktop,
    ) -> std::result::Result<(), DestinationError>;
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
    here_atoms: Option<CallerDesktopAtoms>,
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
    fn capture_desktop(&mut self) -> std::result::Result<CallerDesktop, DestinationError> {
        let atoms = CallerDesktopAtoms::intern(self.peer.conn)?;
        let captured = atoms.capture(self.peer.conn, self.screen.root)?;
        self.here_atoms = Some(atoms);
        Ok(captured)
    }
    fn revalidate_desktop(
        &mut self,
        captured: CallerDesktop,
    ) -> std::result::Result<(), DestinationError> {
        self.here_atoms
            .as_ref()
            .expect("Here capture precedes revalidation")
            .revalidate(self.peer.conn, self.screen.root, captured)
    }
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

fn caller_budget(command: Command) -> Duration {
    command.caller_budget()
}

/// The immutable v2 marker is built once from the invoking caller's capture.
fn placement_caller_marker(
    command: Command,
    captured: super::monitors::destination::CallerDesktop,
    owner: OwnerDescriptor,
    request_id: u64,
) -> Result<CallerMarker> {
    if !matches!(command, Command::BringHere | Command::Recover) {
        return Err("Only placement commands carry a frozen caller destination".into());
    }
    Ok(CallerMarker {
        control_xid: owner.control_xid,
        epoch: owner.epoch,
        request_id,
        command,
        destination_desktop: captured.frozen_destination(),
    })
}

fn caller_with(
    host: &mut impl CallerHost,
    root: u32,
    screen_num: u32,
    command: Command,
    id: impl FnOnce() -> Result<u64>,
) -> Result<CallerOutcome> {
    // This begins before atom/selection discovery and never changes.
    let deadline = host.now() + caller_budget(command);
    let operation = (|| {
        let captured = if matches!(command, Command::BringHere | Command::Recover) {
            match host.capture_desktop() {
                Ok(captured) => Some(captured),
                Err(error) => {
                    return Ok(CallerOutcome::InvalidDestination(format!(
                        "caller workspace capture failed: {error}"
                    )))
                }
            }
        } else {
            None
        };
        let expired =
            || CallerOutcome::Unknown("caller deadline expired before send; request not sent");
        if host.now() >= deadline {
            return Ok(expired());
        }
        let (instance, atoms) = host.atoms(screen_num)?;
        if host.now() >= deadline {
            return Ok(expired());
        }
        let owner = match discover(host, root, screen_num, instance)? {
            Ok(owner) => owner,
            Err(outcome) => return Ok(outcome),
        };
        if host.now() >= deadline {
            return Ok(expired());
        }
        if owner.lifecycle != Lifecycle::Ready {
            return Ok(CallerOutcome::Lifecycle(owner.lifecycle));
        }
        if !host.watch(owner.control_xid)? {
            return Ok(CallerOutcome::Protocol);
        }
        if host.now() >= deadline {
            return Ok(expired());
        }
        let request_id = id()?;
        let marker = if let Some(captured) = captured {
            placement_caller_marker(command, captured, owner, request_id)?
        } else {
            CallerMarker {
                control_xid: owner.control_xid,
                epoch: owner.epoch,
                request_id,
                command,
                destination_desktop: control::NO_DESTINATION,
            }
        };
        if host.now() >= deadline {
            return Ok(expired());
        }
        let reply = host.create_reply(marker)?;
        if host.now() >= deadline {
            return Ok(expired());
        }
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
            return Ok(expired());
        }
        if let Some(captured) = captured {
            if let Err(error) = host.revalidate_desktop(captured) {
                return Ok(CallerOutcome::InvalidDestination(format!("frozen caller workspace changed or could not be revalidated before dispatch: {error}")));
            }
        }
        if host.now() >= deadline {
            return Ok(expired());
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
            let event = host.next()?;
            // A synchronous read may consume the remaining fixed budget. Its
            // return does not renew the caller's permission to accept a reply.
            if host.now() >= deadline {
                return Ok(CallerOutcome::Unknown("caller deadline expired"));
            }
            match event {
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
        here_atoms: None,
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
mod tests;
#[cfg(test)]
pub(crate) mod transport_tests;

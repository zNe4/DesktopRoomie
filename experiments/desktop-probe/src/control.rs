//! Pure M03.2 wire values and observations. No X11 resources or operations.
// These foundations are intentionally inert until the later integration stages.
#![allow(dead_code)]

use std::fmt;

pub const MAGIC: u32 = 0x4452_4d31;
pub const VERSION: u32 = 1;
pub const INSTANCE: &str = "_DESKTOPROOMIE_INSTANCE";
pub const TIMESTAMP: &str = "_DESKTOPROOMIE_TIMESTAMP";
pub const REQUEST: &str = "_DESKTOPROOMIE_REQUEST";
pub const REPLY: &str = "_DESKTOPROOMIE_REPLY";
pub const CALLER: &str = "_DESKTOPROOMIE_CALLER";

pub fn selection_name(screen_num: u32) -> String {
    format!("_DESKTOPROOMIE_INSTANCE_S{screen_num}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Length,
    Magic,
    Version,
    ZeroIdentity,
    InvalidBody,
    UnknownValue(&'static str, u32),
    ReservedBits,
}

// Each enum's decoder rejects all unassigned wire values.
macro_rules! wire_enum {
    ($name:ident { $($variant:ident = $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u32)]
        pub enum $name { $($variant = $value),+ }
        impl TryFrom<u32> for $name {
            type Error = DecodeError;
            fn try_from(value: u32) -> Result<Self, Self::Error> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    _ => Err(DecodeError::UnknownValue(stringify!($name), value)),
                }
            }
        }
    };
}

wire_enum!(Command { Hide = 1, Show = 2, BringTop = 3 });
wire_enum!(Lifecycle { Starting = 1, Ready = 2, Closing = 3 });
wire_enum!(Status { Success = 0, Failed = 1, Partial = 2, Busy = 3, Starting = 4, Closing = 5, ProtocolError = 6 });
wire_enum!(Reason {
    None = 0, MalformedRequest = 1, WrongVersion = 2, UnknownCommand = 3,
    Starting = 4, Busy = 5, Closing = 6, OwnershipLost = 7, BodyDestroyed = 8,
    Withdrawn = 9, Unverifiable = 10, Unsupported = 11, ConfirmationTimeout = 12,
    LayoutChanged = 13, LayerChanged = 14, ReleaseFailed = 15, X11Failure = 16,
    StaleInstance = 17,
});
wire_enum!(Visibility { Unknown = 0, Minimized = 1, NotMinimizedHere = 2, NotMinimizedElsewhere = 3, Transitional = 4, Withdrawn = 5, Destroyed = 6 });
wire_enum!(LayerObservation { Unknown = 0, Normal = 1, Above = 2, Below = 3, Conflict = 4 });
wire_enum!(Workspace { Unknown = 0, Current = 1, Other = 2, AllDesktops = 3 });
wire_enum!(Stage { Preflight = 0, Hide = 1, Show = 2, Geometry = 3, Layer = 4, Shutdown = 5 });
wire_enum!(ExitCode {
    Success = 0, LocalFailure = 1, Usage = 2, NoInstance = 3, DuplicateLaunch = 4,
    Busy = 5, Protocol = 6, OperationFailed = 7, OutcomeUnknown = 8, Partial = 9,
});

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Hide => "hide",
            Self::Show => "show",
            Self::BringTop => "bring-top",
        })
    }
}

impl Status {
    // Applies to a validated terminal response, never receipt or host dispatch.
    pub fn exit_code(self) -> ExitCode {
        match self {
            Self::Success => ExitCode::Success,
            Self::Failed => ExitCode::OperationFailed,
            Self::Partial => ExitCode::Partial,
            Self::Busy | Self::Starting | Self::Closing => ExitCode::Busy,
            Self::ProtocolError => ExitCode::Protocol,
        }
    }
}

fn nonzero(values: &[u32]) -> Result<(), DecodeError> {
    if values.contains(&0) {
        Err(DecodeError::ZeroIdentity)
    } else {
        Ok(())
    }
}
fn identity_header(words: &[u32]) -> Result<(), DecodeError> {
    if words[0] != MAGIC {
        return Err(DecodeError::Magic);
    }
    if words[1] != VERSION {
        return Err(DecodeError::Version);
    }
    Ok(())
}
fn unpack_header(word: u32) -> Result<u32, DecodeError> {
    if word >> 16 != VERSION {
        return Err(DecodeError::Version);
    }
    Ok(word & 0xffff)
}
fn request_id(low: u32, high: u32) -> Result<u64, DecodeError> {
    let id = u64::from(low) | (u64::from(high) << 32);
    if id == 0 {
        Err(DecodeError::ZeroIdentity)
    } else {
        Ok(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerDescriptor {
    pub screen_num: u32,
    pub control_xid: u32,
    pub epoch: u32,
    pub lifecycle: Lifecycle,
    // None is legal before body creation and during teardown, but never Ready.
    pub body_xid: Option<u32>,
}
impl OwnerDescriptor {
    pub fn encode(self) -> Result<[u32; 7], DecodeError> {
        nonzero(&[self.control_xid, self.epoch])?;
        if self.body_xid == Some(0)
            || (self.lifecycle == Lifecycle::Ready && self.body_xid.is_none())
        {
            return Err(DecodeError::InvalidBody);
        }
        Ok([
            MAGIC,
            VERSION,
            self.screen_num,
            self.control_xid,
            self.epoch,
            self.lifecycle as u32,
            self.body_xid.unwrap_or(0),
        ])
    }
    pub fn decode(words: &[u32]) -> Result<Self, DecodeError> {
        let w: &[u32; 7] = words.try_into().map_err(|_| DecodeError::Length)?;
        identity_header(w)?;
        let descriptor = Self {
            screen_num: w[2],
            control_xid: w[3],
            epoch: w[4],
            lifecycle: w[5].try_into()?,
            body_xid: if w[6] == 0 { None } else { Some(w[6]) },
        };
        descriptor.encode()?;
        Ok(descriptor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallerMarker {
    pub control_xid: u32,
    pub epoch: u32,
    pub request_id: u64,
    pub command: Command,
}
impl CallerMarker {
    pub fn encode(self) -> Result<[u32; 7], DecodeError> {
        nonzero(&[self.control_xid, self.epoch])?;
        request_id(self.request_id as u32, (self.request_id >> 32) as u32)?;
        Ok([
            MAGIC,
            VERSION,
            self.control_xid,
            self.epoch,
            self.request_id as u32,
            (self.request_id >> 32) as u32,
            self.command as u32,
        ])
    }
    pub fn decode(words: &[u32]) -> Result<Self, DecodeError> {
        let w: &[u32; 7] = words.try_into().map_err(|_| DecodeError::Length)?;
        identity_header(w)?;
        let marker = Self {
            control_xid: w[2],
            epoch: w[3],
            request_id: request_id(w[4], w[5])?,
            command: w[6].try_into()?,
        };
        marker.encode()?;
        Ok(marker)
    }
    // Destination/format/root/window-class validation belongs to native transport.
    pub fn matches(self, control_xid: u32, request: Request) -> bool {
        self.encode().is_ok()
            && request.encode().is_ok()
            && self.control_xid == control_xid
            && self.epoch == request.epoch
            && self.request_id == request.request_id
            && self.command == request.command
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub command: Command,
    pub request_id: u64,
    pub reply_xid: u32,
    pub epoch: u32,
}
impl Request {
    pub fn encode(self) -> Result<[u32; 5], DecodeError> {
        nonzero(&[self.reply_xid, self.epoch])?;
        request_id(self.request_id as u32, (self.request_id >> 32) as u32)?;
        Ok([
            (VERSION << 16) | self.command as u32,
            self.request_id as u32,
            (self.request_id >> 32) as u32,
            self.reply_xid,
            self.epoch,
        ])
    }
    pub fn decode(words: &[u32]) -> Result<Self, DecodeError> {
        let w: &[u32; 5] = words.try_into().map_err(|_| DecodeError::Length)?;
        let request = Self {
            command: unpack_header(w[0])?.try_into()?,
            request_id: request_id(w[1], w[2])?,
            reply_xid: w[3],
            epoch: w[4],
        };
        request.encode()?;
        Ok(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub not_minimized_confirmed: bool,
    pub above_confirmed: bool,
    pub placement_deferred: bool,
    pub host_mutation_dispatched: bool,
    pub input_ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detail {
    pub reason: Reason,
    pub visibility: Visibility,
    pub layer: LayerObservation,
    pub workspace: Workspace,
    pub progress: Progress,
    pub stage: Stage,
}
impl Detail {
    pub fn encode(self) -> u32 {
        let p = self.progress;
        self.reason as u32
            | (self.visibility as u32) << 8
            | (self.layer as u32) << 11
            | (self.workspace as u32) << 14
            | u32::from(p.not_minimized_confirmed) << 16
            | u32::from(p.above_confirmed) << 17
            | u32::from(p.placement_deferred) << 18
            | u32::from(p.host_mutation_dispatched) << 19
            | u32::from(p.input_ready) << 20
            | (self.stage as u32) << 21
    }
    pub fn decode(word: u32) -> Result<Self, DecodeError> {
        if word & 0xff00_0000 != 0 {
            return Err(DecodeError::ReservedBits);
        }
        Ok(Self {
            reason: (word & 0xff).try_into()?,
            visibility: ((word >> 8) & 7).try_into()?,
            layer: ((word >> 11) & 7).try_into()?,
            workspace: ((word >> 14) & 3).try_into()?,
            progress: Progress {
                not_minimized_confirmed: word & (1 << 16) != 0,
                above_confirmed: word & (1 << 17) != 0,
                placement_deferred: word & (1 << 18) != 0,
                host_mutation_dispatched: word & (1 << 19) != 0,
                input_ready: word & (1 << 20) != 0,
            },
            stage: ((word >> 21) & 7).try_into()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalResponse {
    pub status: Status,
    pub request_id: u64,
    pub control_xid: u32,
    pub detail: Detail,
}
impl TerminalResponse {
    pub fn encode(self) -> Result<[u32; 5], DecodeError> {
        nonzero(&[self.control_xid])?;
        request_id(self.request_id as u32, (self.request_id >> 32) as u32)?;
        Ok([
            (VERSION << 16) | self.status as u32,
            self.request_id as u32,
            (self.request_id >> 32) as u32,
            self.control_xid,
            self.detail.encode(),
        ])
    }
    pub fn decode(words: &[u32]) -> Result<Self, DecodeError> {
        let w: &[u32; 5] = words.try_into().map_err(|_| DecodeError::Length)?;
        let response = Self {
            status: unpack_header(w[0])?.try_into()?,
            request_id: request_id(w[1], w[2])?,
            control_xid: w[3],
            detail: Detail::decode(w[4])?,
        };
        response.encode()?;
        Ok(response)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyCorrelation {
    pub control_xid: u32,
    pub epoch: u32,
    pub request_id: u64,
    pub reply_xid: u32,
}
impl ReplyCorrelation {
    // Epoch is absent from the reply wire. Verify it against the discovered owner,
    // never pretend to recover it from the terminal response.
    pub fn matches(
        self,
        owner: OwnerDescriptor,
        destination: u32,
        response: TerminalResponse,
    ) -> bool {
        owner.encode().is_ok()
            && response.encode().is_ok()
            && self.control_xid != 0
            && self.epoch != 0
            && self.request_id != 0
            && self.reply_xid != 0
            && owner.control_xid == self.control_xid
            && owner.epoch == self.epoch
            && destination == self.reply_xid
            && response.control_xid == self.control_xid
            && response.request_id == self.request_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence<T> {
    Readable(T),
    Missing,
    Malformed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WmState {
    Absent,
    Withdrawn,
    Normal,
    Iconic,
    Malformed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapState {
    Unmapped,
    Unviewable,
    Viewable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Supported,
    Unsupported,
    Unverifiable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibilityEvidence {
    pub destroyed: bool,
    pub wm_state: WmState,
    pub map_state: Evidence<MapState>,
    pub hidden: Evidence<bool>,
    pub workspace: Workspace,
    // HIDDEN and both desktop properties must all be advertised.
    pub capability: Capability,
    // Caller has checked relevant repeated reads for agreement, not atomicity.
    pub coherent: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibilityCandidate {
    pub visibility: Visibility,
    pub reason: Reason,
}

// A candidate is never an operation confirmation. Later stages must reobserve
// and validate preservation, placement, input obligations, and deadlines.
pub fn classify(e: VisibilityEvidence) -> VisibilityCandidate {
    use Visibility::*;
    let (visibility, reason) = if e.destroyed {
        (Destroyed, Reason::BodyDestroyed)
    } else if matches!(e.wm_state, WmState::Absent | WmState::Withdrawn) {
        (Withdrawn, Reason::Withdrawn)
    } else if e.capability == Capability::Unsupported {
        (Unknown, Reason::Unsupported)
    } else if e.capability == Capability::Unverifiable
        || e.wm_state == WmState::Malformed
        || e.workspace == Workspace::Unknown
    {
        (Unknown, Reason::Unverifiable)
    } else if let (Evidence::Readable(map), Evidence::Readable(hidden)) = (e.map_state, e.hidden) {
        if !e.coherent {
            (Transitional, Reason::Unverifiable)
        } else if hidden {
            if e.wm_state == WmState::Iconic && map == MapState::Unmapped {
                (Minimized, Reason::None)
            } else {
                (Transitional, Reason::Unverifiable)
            }
        } else {
            match (e.wm_state, map, e.workspace) {
                (WmState::Iconic, MapState::Unmapped, Workspace::Other)
                | (WmState::Normal, MapState::Unmapped | MapState::Unviewable, Workspace::Other) => {
                    (NotMinimizedElsewhere, Reason::None)
                }
                (
                    WmState::Normal,
                    MapState::Viewable,
                    Workspace::Current | Workspace::AllDesktops,
                ) => (NotMinimizedHere, Reason::None),
                (
                    WmState::Iconic,
                    MapState::Unmapped,
                    Workspace::Current | Workspace::AllDesktops,
                )
                | (
                    WmState::Normal,
                    MapState::Unmapped | MapState::Unviewable,
                    Workspace::Current | Workspace::AllDesktops,
                ) => (Transitional, Reason::Unverifiable),
                _ => (Unknown, Reason::Unverifiable),
            }
        }
    } else {
        (Unknown, Reason::Unverifiable)
    };
    VisibilityCandidate { visibility, reason }
}

// Explicit operation-level confirmation supplied only after fresh WM evidence
// and the command's preservation/readiness checks. No conversion from Candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmedVisibility {
    Minimized,
    NotMinimizedHere,
    NotMinimizedElsewhere,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Confirmation {
    pub visibility: Option<ConfirmedVisibility>,
    pub above: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultError {
    UnconfirmedSuccess,
    InconsistentConfirmation,
    InvalidPartial,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationResult {
    status: Status,
    detail: Detail,
}
impl OperationResult {
    pub fn new(
        command: Command,
        status: Status,
        detail: Detail,
        confirmed: Confirmation,
    ) -> Result<Self, ResultError> {
        let visibility = match confirmed.visibility {
            Some(ConfirmedVisibility::Minimized) => Some(Visibility::Minimized),
            Some(ConfirmedVisibility::NotMinimizedHere) => Some(Visibility::NotMinimizedHere),
            Some(ConfirmedVisibility::NotMinimizedElsewhere) => {
                Some(Visibility::NotMinimizedElsewhere)
            }
            None => None,
        };
        let not_minimized = matches!(
            confirmed.visibility,
            Some(
                ConfirmedVisibility::NotMinimizedHere | ConfirmedVisibility::NotMinimizedElsewhere
            )
        );
        if visibility.is_some_and(|v| v != detail.visibility)
            || detail.progress.not_minimized_confirmed != not_minimized
            || detail.progress.above_confirmed != confirmed.above
            || (confirmed.above && detail.layer != LayerObservation::Above)
            || matches!(
                confirmed.visibility,
                Some(ConfirmedVisibility::NotMinimizedHere)
            ) && !matches!(
                detail.workspace,
                Workspace::Current | Workspace::AllDesktops
            )
            || matches!(
                confirmed.visibility,
                Some(ConfirmedVisibility::NotMinimizedElsewhere)
            ) && detail.workspace != Workspace::Other
        {
            return Err(ResultError::InconsistentConfirmation);
        }
        if status == Status::Success
            && (detail.reason != Reason::None
                || !match command {
                    Command::Hide => confirmed.visibility == Some(ConfirmedVisibility::Minimized),
                    Command::Show => not_minimized,
                    Command::BringTop => not_minimized && confirmed.above,
                })
        {
            return Err(ResultError::UnconfirmedSuccess);
        }
        if status == Status::Success && not_minimized {
            let ready = match confirmed.visibility {
                Some(ConfirmedVisibility::NotMinimizedHere) => {
                    detail.progress.input_ready && !detail.progress.placement_deferred
                }
                Some(ConfirmedVisibility::NotMinimizedElsewhere) => {
                    detail.progress.placement_deferred && !detail.progress.input_ready
                }
                _ => false,
            };
            if !ready {
                return Err(ResultError::UnconfirmedSuccess);
            }
        }
        // Partial is an explicitly observed result, never just a dispatched request.
        if status == Status::Partial
            && (!detail.progress.host_mutation_dispatched
                || (confirmed.visibility.is_none() && !confirmed.above))
        {
            return Err(ResultError::InvalidPartial);
        }
        Ok(Self { status, detail })
    }
    pub fn status(self) -> Status {
        self.status
    }
    pub fn detail(self) -> Detail {
        self.detail
    }
    pub fn exit_code(self) -> ExitCode {
        self.status.exit_code()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> OwnerDescriptor {
        OwnerDescriptor {
            screen_num: 0,
            control_xid: 0x1234,
            epoch: 0x89ab_cdef,
            lifecycle: Lifecycle::Ready,
            body_xid: Some(0x5678),
        }
    }
    fn request() -> Request {
        Request {
            command: Command::BringTop,
            request_id: 0xfedc_ba98_7654_3210,
            reply_xid: 0x9876,
            epoch: owner().epoch,
        }
    }
    fn marker() -> CallerMarker {
        let r = request();
        CallerMarker {
            control_xid: owner().control_xid,
            epoch: r.epoch,
            request_id: r.request_id,
            command: r.command,
        }
    }
    fn detail() -> Detail {
        Detail {
            reason: Reason::None,
            visibility: Visibility::Unknown,
            layer: LayerObservation::Unknown,
            workspace: Workspace::Unknown,
            progress: Progress::default(),
            stage: Stage::Preflight,
        }
    }
    fn response() -> TerminalResponse {
        TerminalResponse {
            status: Status::Failed,
            request_id: request().request_id,
            control_xid: owner().control_xid,
            detail: detail(),
        }
    }
    fn evidence(
        wm_state: WmState,
        map: MapState,
        hidden: bool,
        workspace: Workspace,
    ) -> VisibilityEvidence {
        VisibilityEvidence {
            destroyed: false,
            wm_state,
            map_state: Evidence::Readable(map),
            hidden: Evidence::Readable(hidden),
            workspace,
            capability: Capability::Supported,
            coherent: true,
        }
    }

    #[test]
    fn private_atom_roles_are_exact() {
        assert_eq!(
            [selection_name(0), selection_name(u32::MAX)],
            [
                "_DESKTOPROOMIE_INSTANCE_S0",
                "_DESKTOPROOMIE_INSTANCE_S4294967295"
            ]
        );
        assert_eq!(
            [INSTANCE, TIMESTAMP, REQUEST, REPLY, CALLER],
            [
                "_DESKTOPROOMIE_INSTANCE",
                "_DESKTOPROOMIE_TIMESTAMP",
                "_DESKTOPROOMIE_REQUEST",
                "_DESKTOPROOMIE_REPLY",
                "_DESKTOPROOMIE_CALLER"
            ]
        );
    }
    #[test]
    fn descriptor_and_marker_interoperability() {
        let words = [0x4452_4d31, 1, 0, 0x1234, 0x89ab_cdef, 2, 0x5678];
        assert_eq!(owner().encode(), Ok(words));
        assert_eq!(OwnerDescriptor::decode(&words), Ok(owner()));
        let words = [
            0x4452_4d31,
            1,
            0x1234,
            0x89ab_cdef,
            0x7654_3210,
            0xfedc_ba98,
            3,
        ];
        assert_eq!(marker().encode(), Ok(words));
        assert_eq!(CallerMarker::decode(&words), Ok(marker()));
    }
    #[test]
    fn request_and_response_interoperability() {
        let words = [0x0001_0003, 0x7654_3210, 0xfedc_ba98, 0x9876, 0x89ab_cdef];
        assert_eq!(request().encode(), Ok(words));
        assert_eq!(Request::decode(&words), Ok(request()));
        let words = [0x0001_0001, 0x7654_3210, 0xfedc_ba98, 0x1234, 0];
        assert_eq!(response().encode(), Ok(words));
        assert_eq!(TerminalResponse::decode(&words), Ok(response()));
    }
    #[test]
    fn every_command_lifecycle_status_and_reason_wire_value() {
        for (command, value) in [
            (Command::Hide, 1),
            (Command::Show, 2),
            (Command::BringTop, 3),
        ] {
            assert_eq!(
                Request {
                    command,
                    ..request()
                }
                .encode()
                .unwrap()[0],
                0x10000 | value
            );
            assert_eq!(Command::try_from(value), Ok(command));
            assert_eq!(
                CallerMarker {
                    command,
                    ..marker()
                }
                .encode()
                .unwrap()[6],
                value
            );
        }
        for (lifecycle, value) in [
            (Lifecycle::Starting, 1),
            (Lifecycle::Ready, 2),
            (Lifecycle::Closing, 3),
        ] {
            let d = OwnerDescriptor {
                lifecycle,
                ..owner()
            };
            let words = d.encode().unwrap();
            assert_eq!(words[5], value);
            assert_eq!(OwnerDescriptor::decode(&words), Ok(d));
        }
        for (status, value) in [
            (Status::Success, 0),
            (Status::Failed, 1),
            (Status::Partial, 2),
            (Status::Busy, 3),
            (Status::Starting, 4),
            (Status::Closing, 5),
            (Status::ProtocolError, 6),
        ] {
            let r = TerminalResponse {
                status,
                ..response()
            };
            let words = r.encode().unwrap();
            assert_eq!(words[0], 0x10000 | value);
            assert_eq!(TerminalResponse::decode(&words), Ok(r));
        }
        let reasons = [
            Reason::None,
            Reason::MalformedRequest,
            Reason::WrongVersion,
            Reason::UnknownCommand,
            Reason::Starting,
            Reason::Busy,
            Reason::Closing,
            Reason::OwnershipLost,
            Reason::BodyDestroyed,
            Reason::Withdrawn,
            Reason::Unverifiable,
            Reason::Unsupported,
            Reason::ConfirmationTimeout,
            Reason::LayoutChanged,
            Reason::LayerChanged,
            Reason::ReleaseFailed,
            Reason::X11Failure,
            Reason::StaleInstance,
        ];
        for (value, reason) in reasons.into_iter().enumerate() {
            let d = Detail { reason, ..detail() };
            assert_eq!(d.encode(), value as u32);
            assert_eq!(Detail::decode(value as u32), Ok(d));
        }
    }
    #[test]
    fn diagnostic_fields_and_progress_bits_are_exact() {
        for (visibility, value) in [
            (Visibility::Unknown, 0),
            (Visibility::Minimized, 1),
            (Visibility::NotMinimizedHere, 2),
            (Visibility::NotMinimizedElsewhere, 3),
            (Visibility::Transitional, 4),
            (Visibility::Withdrawn, 5),
            (Visibility::Destroyed, 6),
        ] {
            let d = Detail {
                visibility,
                ..detail()
            };
            assert_eq!(d.encode(), value << 8);
            assert_eq!(Detail::decode(value << 8), Ok(d));
        }
        for (layer, value) in [
            (LayerObservation::Unknown, 0),
            (LayerObservation::Normal, 1),
            (LayerObservation::Above, 2),
            (LayerObservation::Below, 3),
            (LayerObservation::Conflict, 4),
        ] {
            let d = Detail { layer, ..detail() };
            assert_eq!(d.encode(), value << 11);
            assert_eq!(Detail::decode(value << 11), Ok(d));
        }
        for (workspace, value) in [
            (Workspace::Unknown, 0),
            (Workspace::Current, 1),
            (Workspace::Other, 2),
            (Workspace::AllDesktops, 3),
        ] {
            let d = Detail {
                workspace,
                ..detail()
            };
            assert_eq!(d.encode(), value << 14);
            assert_eq!(Detail::decode(value << 14), Ok(d));
        }
        for (stage, value) in [
            (Stage::Preflight, 0),
            (Stage::Hide, 1),
            (Stage::Show, 2),
            (Stage::Geometry, 3),
            (Stage::Layer, 4),
            (Stage::Shutdown, 5),
        ] {
            let d = Detail { stage, ..detail() };
            assert_eq!(d.encode(), value << 21);
            assert_eq!(Detail::decode(value << 21), Ok(d));
        }
        for bits in 0..32 {
            let d = Detail::decode(bits << 16).unwrap();
            assert_eq!(
                d.progress,
                Progress {
                    not_minimized_confirmed: bits & 1 != 0,
                    above_confirmed: bits & 2 != 0,
                    placement_deferred: bits & 4 != 0,
                    host_mutation_dispatched: bits & 8 != 0,
                    input_ready: bits & 16 != 0
                }
            );
            assert_eq!(d.encode(), bits << 16);
        }
        let combined = Detail {
            reason: Reason::StaleInstance,
            visibility: Visibility::Destroyed,
            layer: LayerObservation::Conflict,
            workspace: Workspace::AllDesktops,
            progress: Progress {
                not_minimized_confirmed: true,
                above_confirmed: true,
                placement_deferred: true,
                host_mutation_dispatched: true,
                input_ready: true,
            },
            stage: Stage::Shutdown,
        };
        assert_eq!(combined.encode(), 0x00bf_e611);
        assert_eq!(Detail::decode(0x00bf_e611), Ok(combined));
    }
    #[test]
    fn rejects_reserved_bits_and_all_unknown_discriminants() {
        for bit in 24..32 {
            assert_eq!(Detail::decode(1 << bit), Err(DecodeError::ReservedBits));
        }
        for word in [18, 255, 7 << 8, 5 << 11, 6 << 11, 7 << 11, 6 << 21, 7 << 21] {
            assert!(Detail::decode(word).is_err());
        }
        for value in [0, 4, u32::MAX] {
            assert!(Command::try_from(value).is_err());
            assert!(Lifecycle::try_from(value).is_err());
        }
        for value in [7, u32::MAX] {
            assert!(Status::try_from(value).is_err());
        }
        assert!(Workspace::try_from(4).is_err());
        assert!(ExitCode::try_from(10).is_err());
    }
    #[test]
    fn codecs_reject_wrong_lengths_headers_and_unknown_fields() {
        for len in 0..9 {
            if len != 7 {
                assert_eq!(
                    OwnerDescriptor::decode(&vec![0; len]),
                    Err(DecodeError::Length)
                );
                assert_eq!(
                    CallerMarker::decode(&vec![0; len]),
                    Err(DecodeError::Length)
                );
            }
            if len != 5 {
                assert_eq!(Request::decode(&vec![0; len]), Err(DecodeError::Length));
                assert_eq!(
                    TerminalResponse::decode(&vec![0; len]),
                    Err(DecodeError::Length)
                );
            }
        }
        for (index, bad, expected) in [
            (0, 0, DecodeError::Magic),
            (1, 0, DecodeError::Version),
            (1, 2, DecodeError::Version),
        ] {
            let mut d = owner().encode().unwrap();
            d[index] = bad;
            assert_eq!(OwnerDescriptor::decode(&d), Err(expected));
            let mut m = marker().encode().unwrap();
            m[index] = bad;
            assert_eq!(CallerMarker::decode(&m), Err(expected));
        }
        for header in [3, 0x20003, 0xffff0003] {
            let mut r = request().encode().unwrap();
            r[0] = header;
            assert_eq!(Request::decode(&r), Err(DecodeError::Version));
            let mut r = response().encode().unwrap();
            r[0] = header;
            assert_eq!(TerminalResponse::decode(&r), Err(DecodeError::Version));
        }
        let mut d = owner().encode().unwrap();
        d[5] = 4;
        assert!(OwnerDescriptor::decode(&d).is_err());
        let mut m = marker().encode().unwrap();
        m[6] = 4;
        assert!(CallerMarker::decode(&m).is_err());
        let mut r = request().encode().unwrap();
        r[0] = 0x10004;
        assert!(Request::decode(&r).is_err());
        let mut r = response().encode().unwrap();
        r[0] = 0x10007;
        assert!(TerminalResponse::decode(&r).is_err());
        r[0] = 0x10001;
        r[4] = 1 << 24;
        assert_eq!(TerminalResponse::decode(&r), Err(DecodeError::ReservedBits));
    }
    #[test]
    fn identities_are_checked_on_encode_and_decode() {
        for d in [
            OwnerDescriptor {
                control_xid: 0,
                ..owner()
            },
            OwnerDescriptor {
                epoch: 0,
                ..owner()
            },
        ] {
            assert_eq!(d.encode(), Err(DecodeError::ZeroIdentity));
        }
        for index in [3, 4] {
            let mut w = owner().encode().unwrap();
            w[index] = 0;
            assert_eq!(OwnerDescriptor::decode(&w), Err(DecodeError::ZeroIdentity));
        }
        assert_eq!(
            OwnerDescriptor {
                body_xid: None,
                ..owner()
            }
            .encode(),
            Err(DecodeError::InvalidBody)
        );
        assert_eq!(
            OwnerDescriptor {
                body_xid: Some(0),
                ..owner()
            }
            .encode(),
            Err(DecodeError::InvalidBody)
        );
        for lifecycle in [Lifecycle::Starting, Lifecycle::Closing] {
            let d = OwnerDescriptor {
                lifecycle,
                body_xid: None,
                ..owner()
            };
            assert_eq!(OwnerDescriptor::decode(&d.encode().unwrap()), Ok(d));
        }
        for m in [
            CallerMarker {
                control_xid: 0,
                ..marker()
            },
            CallerMarker {
                epoch: 0,
                ..marker()
            },
            CallerMarker {
                request_id: 0,
                ..marker()
            },
        ] {
            assert_eq!(m.encode(), Err(DecodeError::ZeroIdentity));
        }
        for indices in [&[2][..], &[3], &[4, 5]] {
            let mut w = marker().encode().unwrap();
            for &i in indices {
                w[i] = 0;
            }
            assert_eq!(CallerMarker::decode(&w), Err(DecodeError::ZeroIdentity));
        }
        for r in [
            Request {
                reply_xid: 0,
                ..request()
            },
            Request {
                epoch: 0,
                ..request()
            },
            Request {
                request_id: 0,
                ..request()
            },
        ] {
            assert_eq!(r.encode(), Err(DecodeError::ZeroIdentity));
        }
        for indices in [&[3][..], &[4], &[1, 2]] {
            let mut w = request().encode().unwrap();
            for &i in indices {
                w[i] = 0;
            }
            assert_eq!(Request::decode(&w), Err(DecodeError::ZeroIdentity));
        }
        for r in [
            TerminalResponse {
                control_xid: 0,
                ..response()
            },
            TerminalResponse {
                request_id: 0,
                ..response()
            },
        ] {
            assert_eq!(r.encode(), Err(DecodeError::ZeroIdentity));
        }
        for indices in [&[3][..], &[1, 2]] {
            let mut w = response().encode().unwrap();
            for &i in indices {
                w[i] = 0;
            }
            assert_eq!(TerminalResponse::decode(&w), Err(DecodeError::ZeroIdentity));
        }
    }
    #[test]
    fn boundary_round_trips_preserve_full_width_identities() {
        for id in [1, u64::from(u32::MAX), 1 << 32, u64::MAX] {
            let r = Request {
                request_id: id,
                reply_xid: u32::MAX,
                epoch: u32::MAX,
                ..request()
            };
            assert_eq!(Request::decode(&r.encode().unwrap()), Ok(r));
            let m = CallerMarker {
                request_id: id,
                control_xid: u32::MAX,
                epoch: u32::MAX,
                ..marker()
            };
            assert_eq!(CallerMarker::decode(&m.encode().unwrap()), Ok(m));
            let r = TerminalResponse {
                request_id: id,
                control_xid: u32::MAX,
                ..response()
            };
            assert_eq!(TerminalResponse::decode(&r.encode().unwrap()), Ok(r));
        }
        let d = OwnerDescriptor {
            screen_num: u32::MAX,
            control_xid: u32::MAX,
            epoch: u32::MAX,
            body_xid: Some(u32::MAX),
            ..owner()
        };
        assert_eq!(OwnerDescriptor::decode(&d.encode().unwrap()), Ok(d));
    }
    #[test]
    fn marker_correlation_checks_command_epoch_control_and_request() {
        let m = marker();
        let r = request();
        assert!(m.matches(owner().control_xid, r));
        assert!(!m.matches(1, r));
        assert!(!m.matches(owner().control_xid, Request { epoch: 1, ..r }));
        assert!(!m.matches(owner().control_xid, Request { request_id: 1, ..r }));
        assert!(!m.matches(
            owner().control_xid,
            Request {
                command: Command::Hide,
                ..r
            }
        ));
    }
    #[test]
    fn reply_correlation_checks_destination_and_discovered_instance() {
        let c = ReplyCorrelation {
            control_xid: owner().control_xid,
            epoch: owner().epoch,
            request_id: request().request_id,
            reply_xid: request().reply_xid,
        };
        assert!(c.matches(owner(), c.reply_xid, response()));
        assert!(!c.matches(owner(), 1, response()));
        assert!(!c.matches(
            OwnerDescriptor {
                epoch: 1,
                ..owner()
            },
            c.reply_xid,
            response()
        ));
        assert!(!c.matches(
            OwnerDescriptor {
                control_xid: 1,
                ..owner()
            },
            c.reply_xid,
            response()
        ));
        assert!(!c.matches(
            owner(),
            c.reply_xid,
            TerminalResponse {
                control_xid: 1,
                ..response()
            }
        ));
        assert!(!c.matches(
            owner(),
            c.reply_xid,
            TerminalResponse {
                request_id: 1,
                ..response()
            }
        ));
        assert!(!ReplyCorrelation { epoch: 0, ..c }.matches(owner(), c.reply_xid, response()));
    }
    #[test]
    fn classification_table() {
        use MapState::*;
        use WmState::*;
        use Workspace::*;
        let rows = [
            (Iconic, Unmapped, true, Current, Visibility::Minimized),
            (Iconic, Unmapped, true, Other, Visibility::Minimized),
            (
                Iconic,
                Unmapped,
                false,
                Other,
                Visibility::NotMinimizedElsewhere,
            ),
            (
                Normal,
                Unmapped,
                false,
                Other,
                Visibility::NotMinimizedElsewhere,
            ),
            (
                Normal,
                Unviewable,
                false,
                Other,
                Visibility::NotMinimizedElsewhere,
            ),
            (
                Normal,
                Viewable,
                false,
                Current,
                Visibility::NotMinimizedHere,
            ),
            (
                Normal,
                Viewable,
                false,
                AllDesktops,
                Visibility::NotMinimizedHere,
            ),
            (Iconic, Unmapped, false, Current, Visibility::Transitional),
            (
                Iconic,
                Unmapped,
                false,
                AllDesktops,
                Visibility::Transitional,
            ),
            (Normal, Unmapped, false, Current, Visibility::Transitional),
            (Normal, Unviewable, false, Current, Visibility::Transitional),
            (Normal, Viewable, true, Current, Visibility::Transitional),
            (Normal, Unmapped, true, Other, Visibility::Transitional),
            (Iconic, Viewable, true, Current, Visibility::Transitional),
            (Iconic, Unviewable, true, Other, Visibility::Transitional),
            (Normal, Viewable, false, Other, Visibility::Unknown),
            (Iconic, Viewable, false, Other, Visibility::Unknown),
            (Iconic, Unviewable, false, Current, Visibility::Unknown),
            (Absent, Unmapped, false, Current, Visibility::Withdrawn),
            (Withdrawn, Unmapped, false, Current, Visibility::Withdrawn),
            (Malformed, Unmapped, false, Other, Visibility::Unknown),
        ];
        for (wm, map, hidden, workspace, expected) in rows {
            let result = classify(evidence(wm, map, hidden, workspace));
            assert_eq!(
                result.visibility, expected,
                "{wm:?}/{map:?}/{hidden}/{workspace:?}"
            );
            if matches!(expected, Visibility::Unknown | Visibility::Transitional) {
                assert_eq!(result.reason, Reason::Unverifiable);
            }
        }
    }
    #[test]
    fn incomplete_evidence_never_confirms_minimization_or_restoration() {
        for base in [
            evidence(
                WmState::Iconic,
                MapState::Unmapped,
                true,
                Workspace::Current,
            ),
            evidence(
                WmState::Normal,
                MapState::Viewable,
                false,
                Workspace::Current,
            ),
            evidence(WmState::Iconic, MapState::Unmapped, false, Workspace::Other),
        ] {
            for e in [
                VisibilityEvidence {
                    hidden: Evidence::Missing,
                    ..base
                },
                VisibilityEvidence {
                    hidden: Evidence::Malformed,
                    ..base
                },
                VisibilityEvidence {
                    map_state: Evidence::Missing,
                    ..base
                },
                VisibilityEvidence {
                    map_state: Evidence::Malformed,
                    ..base
                },
                VisibilityEvidence {
                    wm_state: WmState::Malformed,
                    ..base
                },
                VisibilityEvidence {
                    workspace: Workspace::Unknown,
                    ..base
                },
                VisibilityEvidence {
                    capability: Capability::Unverifiable,
                    ..base
                },
            ] {
                assert_eq!(
                    classify(e),
                    VisibilityCandidate {
                        visibility: Visibility::Unknown,
                        reason: Reason::Unverifiable
                    }
                );
            }
            assert_eq!(
                classify(VisibilityEvidence {
                    capability: Capability::Unsupported,
                    ..base
                }),
                VisibilityCandidate {
                    visibility: Visibility::Unknown,
                    reason: Reason::Unsupported
                }
            );
            assert_eq!(
                classify(VisibilityEvidence {
                    coherent: false,
                    ..base
                })
                .visibility,
                Visibility::Transitional
            );
            assert_eq!(
                classify(VisibilityEvidence {
                    destroyed: true,
                    hidden: Evidence::Missing,
                    ..base
                }),
                VisibilityCandidate {
                    visibility: Visibility::Destroyed,
                    reason: Reason::BodyDestroyed
                }
            );
        }
    }
    #[test]
    fn requested_mutation_and_candidate_are_insufficient_for_success() {
        for command in [Command::Hide, Command::Show, Command::BringTop] {
            for visibility in [
                Visibility::Unknown,
                Visibility::Minimized,
                Visibility::NotMinimizedHere,
                Visibility::NotMinimizedElsewhere,
            ] {
                let d = Detail {
                    visibility,
                    progress: Progress {
                        host_mutation_dispatched: true,
                        ..Progress::default()
                    },
                    ..detail()
                };
                assert_eq!(
                    OperationResult::new(command, Status::Success, d, Confirmation::default()),
                    Err(ResultError::UnconfirmedSuccess)
                );
                assert_eq!(
                    OperationResult::new(command, Status::Failed, d, Confirmation::default())
                        .unwrap()
                        .exit_code(),
                    ExitCode::OperationFailed
                );
                assert_eq!(
                    OperationResult::new(command, Status::Partial, d, Confirmation::default()),
                    Err(ResultError::InvalidPartial)
                );
            }
        }
    }
    #[test]
    fn confirmed_results_and_partial_progress_remain_distinct() {
        let d = Detail {
            visibility: Visibility::Minimized,
            stage: Stage::Hide,
            ..detail()
        };
        let c = Confirmation {
            visibility: Some(ConfirmedVisibility::Minimized),
            above: false,
        };
        assert_eq!(
            OperationResult::new(Command::Hide, Status::Success, d, c)
                .unwrap()
                .exit_code(),
            ExitCode::Success
        );
        assert_eq!(
            OperationResult::new(Command::Show, Status::Success, d, c),
            Err(ResultError::UnconfirmedSuccess)
        );
        for (visibility, confirmed, workspace) in [
            (
                Visibility::NotMinimizedHere,
                ConfirmedVisibility::NotMinimizedHere,
                Workspace::Current,
            ),
            (
                Visibility::NotMinimizedElsewhere,
                ConfirmedVisibility::NotMinimizedElsewhere,
                Workspace::Other,
            ),
        ] {
            let d = Detail {
                visibility,
                workspace,
                progress: Progress {
                    not_minimized_confirmed: true,
                    placement_deferred: workspace == Workspace::Other,
                    input_ready: workspace == Workspace::Current,
                    ..Progress::default()
                },
                ..detail()
            };
            let c = Confirmation {
                visibility: Some(confirmed),
                above: false,
            };
            let result = OperationResult::new(Command::Show, Status::Success, d, c).unwrap();
            assert_eq!(result.status(), Status::Success);
            assert_eq!(result.detail(), d);
            assert_eq!(
                OperationResult::new(Command::BringTop, Status::Success, d, c),
                Err(ResultError::UnconfirmedSuccess)
            );
            let partial = Detail {
                reason: Reason::ConfirmationTimeout,
                progress: Progress {
                    host_mutation_dispatched: true,
                    ..d.progress
                },
                ..d
            };
            assert_eq!(
                OperationResult::new(Command::BringTop, Status::Partial, partial, c)
                    .unwrap()
                    .exit_code(),
                ExitCode::Partial
            );
            let above = Detail {
                layer: LayerObservation::Above,
                progress: Progress {
                    above_confirmed: true,
                    ..d.progress
                },
                ..d
            };
            let c = Confirmation { above: true, ..c };
            assert_eq!(
                OperationResult::new(Command::BringTop, Status::Success, above, c)
                    .unwrap()
                    .exit_code(),
                ExitCode::Success
            );
            assert_eq!(
                OperationResult::new(
                    Command::BringTop,
                    Status::Success,
                    Detail {
                        layer: LayerObservation::Below,
                        ..above
                    },
                    c
                ),
                Err(ResultError::InconsistentConfirmation)
            );
        }
    }
    #[test]
    fn result_confirmation_requires_compatible_workspace_and_readiness() {
        let c = Confirmation {
            visibility: Some(ConfirmedVisibility::NotMinimizedHere),
            above: false,
        };
        let d = Detail {
            visibility: Visibility::NotMinimizedHere,
            workspace: Workspace::Current,
            progress: Progress {
                not_minimized_confirmed: true,
                ..Progress::default()
            },
            ..detail()
        };
        assert_eq!(
            OperationResult::new(Command::Show, Status::Success, d, c),
            Err(ResultError::UnconfirmedSuccess)
        );
        assert_eq!(
            OperationResult::new(
                Command::Show,
                Status::Success,
                Detail {
                    workspace: Workspace::Other,
                    ..d
                },
                c
            ),
            Err(ResultError::InconsistentConfirmation)
        );
        let d = Detail {
            visibility: Visibility::NotMinimizedElsewhere,
            workspace: Workspace::Other,
            progress: Progress {
                not_minimized_confirmed: true,
                input_ready: true,
                placement_deferred: true,
                ..Progress::default()
            },
            ..detail()
        };
        let c = Confirmation {
            visibility: Some(ConfirmedVisibility::NotMinimizedElsewhere),
            above: false,
        };
        assert_eq!(
            OperationResult::new(Command::Show, Status::Success, d, c),
            Err(ResultError::UnconfirmedSuccess)
        );
        assert_eq!(
            OperationResult::new(
                Command::Show,
                Status::Success,
                Detail {
                    workspace: Workspace::Unknown,
                    ..d
                },
                c
            ),
            Err(ResultError::InconsistentConfirmation)
        );
    }
    #[test]
    fn terminal_and_cli_exit_mapping() {
        for (status, code) in [
            (Status::Success, 0),
            (Status::Failed, 7),
            (Status::Partial, 9),
            (Status::Busy, 5),
            (Status::Starting, 5),
            (Status::Closing, 5),
            (Status::ProtocolError, 6),
        ] {
            assert_eq!(status.exit_code() as u32, code);
        }
        for (code, value) in [
            (ExitCode::Success, 0),
            (ExitCode::LocalFailure, 1),
            (ExitCode::Usage, 2),
            (ExitCode::NoInstance, 3),
            (ExitCode::DuplicateLaunch, 4),
            (ExitCode::Busy, 5),
            (ExitCode::Protocol, 6),
            (ExitCode::OperationFailed, 7),
            (ExitCode::OutcomeUnknown, 8),
            (ExitCode::Partial, 9),
        ] {
            assert_eq!(code as u32, value);
            assert_eq!(ExitCode::try_from(value), Ok(code));
        }
    }
}

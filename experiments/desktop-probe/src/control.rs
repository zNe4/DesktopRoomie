//! Pure control protocol and operation evidence. No X11 resources or operations.
#![allow(dead_code)]

use std::fmt;

pub const MAGIC: u32 = 0x4452_4d31;
pub const VERSION: u32 = 2;
pub const NO_DESTINATION: u32 = u32::MAX;
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
    InvalidDestination,
    ContradictoryProof,
    UnknownValue(&'static str, u32),
    ReservedBits,
}

// Each enum's decoder rejects all unassigned wire values.
macro_rules! wire_enum {
    ($(#[$enum_meta:meta])* $name:ident { $($(#[$variant_meta:meta])* $variant:ident = $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $(#[$enum_meta])*
        #[repr(u32)]
        pub enum $name { $($(#[$variant_meta])* $variant = $value),+ }
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

wire_enum!(Command { Hide = 1, Show = 2, BringTop = 3, BringHere = 4, Recover = 5 });
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
wire_enum!(Stage { Preflight = 0, Hide = 1, Show = 2, Geometry = 3, Layer = 4, Shutdown = 5, Placement = 6 });
wire_enum!(#[derive(Default)] GeometryProof { #[default] Unknown = 0, Deferred = 1, Verified = 2 });

impl Command {
    pub fn validate_destination(self, destination: u32) -> Result<(), DecodeError> {
        if matches!(self, Self::BringHere | Self::Recover) == (destination != NO_DESTINATION) {
            Ok(())
        } else {
            Err(DecodeError::InvalidDestination)
        }
    }

    pub fn owner_budget(self) -> std::time::Duration {
        std::time::Duration::from_millis(match self {
            Self::Hide => 1000,
            Self::Show => 2500,
            Self::BringTop | Self::BringHere => 3500,
            Self::Recover => 7000,
        })
    }

    pub fn caller_budget(self) -> std::time::Duration {
        std::time::Duration::from_millis(match self {
            Self::Hide | Self::Show => 4000,
            Self::BringTop | Self::BringHere => 5000,
            Self::Recover => 9000,
        })
    }
}
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
            Self::BringHere => "bring-here",
            Self::Recover => "recover",
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
    pub destination_desktop: u32,
}
impl CallerMarker {
    pub fn encode(self) -> Result<[u32; 8], DecodeError> {
        nonzero(&[self.control_xid, self.epoch])?;
        request_id(self.request_id as u32, (self.request_id >> 32) as u32)?;
        self.command
            .validate_destination(self.destination_desktop)?;
        Ok([
            MAGIC,
            VERSION,
            self.control_xid,
            self.epoch,
            self.request_id as u32,
            (self.request_id >> 32) as u32,
            self.command as u32,
            self.destination_desktop,
        ])
    }
    pub fn decode(words: &[u32]) -> Result<Self, DecodeError> {
        let w: &[u32; 8] = words.try_into().map_err(|_| DecodeError::Length)?;
        identity_header(w)?;
        let marker = Self {
            control_xid: w[2],
            epoch: w[3],
            request_id: request_id(w[4], w[5])?,
            command: w[6].try_into()?,
            destination_desktop: w[7],
        };
        marker.encode()?;
        Ok(marker)
    }
    // The destination is carried only by this property, never by a request word.
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

/// Historical transitions survive loss of current evidence. Neither flag implies
/// that destination, visibility, geometry or input is currently confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecoveryHistory {
    pub relocation_confirmed: bool,
    pub restoration_confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlacementConfirmation {
    /// Body membership AND the frozen destination still being current.
    pub destination: bool,
    /// Deferred proves retained-monitor/workarea fit, but not actual geometry.
    pub geometry: GeometryProof,
    pub minimization_preserved: bool,
    pub layer_preserved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlacementProof {
    pub current: PlacementConfirmation,
    pub history: RecoveryHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detail {
    pub reason: Reason,
    pub visibility: Visibility,
    pub layer: LayerObservation,
    pub workspace: Workspace,
    pub progress: Progress,
    pub stage: Stage,
    pub placement: PlacementProof,
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
            // v2: 24 destination current; 25/26 historical relocation/restoration;
            // 27..28 geometry (0 unknown, 1 admissible/deferred, 2 verified);
            // 29 preserved genuine minimization; 30 preserved effective layer.
            // Bits 0..23 retain their M03.2 meanings. Bit 31 remains reserved.
            | u32::from(self.placement.current.destination) << 24
            | u32::from(self.placement.history.relocation_confirmed) << 25
            | u32::from(self.placement.history.restoration_confirmed) << 26
            | (self.placement.current.geometry as u32) << 27
            | u32::from(self.placement.current.minimization_preserved) << 29
            | u32::from(self.placement.current.layer_preserved) << 30
    }
    pub fn decode(word: u32) -> Result<Self, DecodeError> {
        if word & 0x8000_0000 != 0 {
            return Err(DecodeError::ReservedBits);
        }
        let detail = Self {
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
            placement: PlacementProof {
                current: PlacementConfirmation {
                    destination: word & (1 << 24) != 0,
                    geometry: ((word >> 27) & 3).try_into()?,
                    minimization_preserved: word & (1 << 29) != 0,
                    layer_preserved: word & (1 << 30) != 0,
                },
                history: RecoveryHistory {
                    relocation_confirmed: word & (1 << 25) != 0,
                    restoration_confirmed: word & (1 << 26) != 0,
                },
            },
        };
        detail.validate_placement_wire()?;
        Ok(detail)
    }

    fn validate_placement_wire(self) -> Result<(), DecodeError> {
        let p = self.placement.current;
        if (p.destination && self.workspace != Workspace::Current)
            || (p.geometry != GeometryProof::Unknown && !p.destination)
            || (p.geometry == GeometryProof::Verified && self.progress.placement_deferred)
            || (p.geometry == GeometryProof::Deferred
                && (!p.minimization_preserved || !self.progress.placement_deferred))
            || (p.minimization_preserved
                && (self.visibility != Visibility::Minimized
                    || self.progress.not_minimized_confirmed
                    || self.progress.input_ready
                    || self.placement.history.restoration_confirmed))
            || (p.layer_preserved
                && !matches!(
                    self.layer,
                    LayerObservation::Normal | LayerObservation::Above | LayerObservation::Below
                ))
            || (self.placement != PlacementProof::default()
                && ((self.progress.input_ready
                    && (!p.destination
                        || p.geometry != GeometryProof::Verified
                        || self.visibility != Visibility::NotMinimizedHere
                        || !self.progress.not_minimized_confirmed))
                    || (self.progress.above_confirmed
                        && (!self.progress.not_minimized_confirmed
                            || self.layer != LayerObservation::Above))
                    || (self.progress.not_minimized_confirmed
                        && !matches!(
                            self.visibility,
                            Visibility::NotMinimizedHere | Visibility::NotMinimizedElsewhere
                        ))))
        {
            return Err(DecodeError::ContradictoryProof);
        }
        Ok(())
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
        self.detail.validate_placement_wire()?;
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

/// One admitted operation, with no queue or completed-request history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveOperation {
    pub request: Request,
    pub stage: Stage,
    pub deadline: std::time::Instant,
    pub detail: Detail,
    pub show: ShowProgress,
    pub hide: HideProgress,
    pub bring_top: BringTopProgress,
    pub placement: PlacementProgress,
    pub recover_phase: RecoverPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementBaseline {
    pub desktop: u32,
    pub minimized: bool,
    pub layer: LayerObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementProgress {
    /// Immutable from admission; never refreshed from current desktop evidence.
    pub destination_desktop: u32,
    pub baseline: Option<PlacementBaseline>,
    pub proof: PlacementProof,
}
impl PlacementProgress {
    pub fn clear_current(&mut self) {
        self.proof.current = PlacementConfirmation::default();
    }
}

/// Phase identity only, under ActiveOperation's immutable total deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverPhase {
    Here,
    Top,
}

/// Composition facts only. The sole LayerController still owns its target,
/// phase, observations and pending deadline. None of these facts are wire bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BringTopProgress {
    pub visibility_cutoff: std::time::Instant,
    pub minimized_at_start: bool,
    pub restoration_confirmed: bool,
    /// Fixes the budget even if the first qualified layer read is transitional.
    pub layer_admitted: Option<std::time::Instant>,
    pub layer_observe: bool,
    pub layer_attempted: bool,
    pub layer_dispatched: bool,
    pub layer_checked: bool,
    /// Controller completed; only terminal evidence revalidation may remain.
    pub layer_finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HidePhase {
    #[default]
    Fence,
    Fenced,
    /// Consumed before dispatch, including a failed checked send. Never retry.
    IconifyAttempted,
    IconifyRequested,
    /// Off-workspace ADD-HIDDEN attempt consumed before dispatch. Never retry.
    HiddenAddAttempted,
    /// ADD-HIDDEN dispatched and checked, awaiting fresh actual minimization.
    HiddenAddRequested,
}

/// Only the progress matching `request.command` is serviced. Both commands share
/// the single admission identity, deadline and terminal-response owner above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HideProgress {
    pub phase: HidePhase,
    pub baseline: Option<VisibilityBaseline>,
    pub observe: bool,
    pub layout_interrupted: bool,
}

/// Owner-only progression, kept in the transport's single authoritative record.
/// These are not wire values and retain no completed-request history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShowPhase {
    #[default]
    Observe,
    Fenced,
    /// Attempt consumed, even if only user-time zero was sent. Never retry.
    RestoreAttempted,
    /// MapWindow was dispatched and its check succeeded; WM readiness is pending.
    RestoreRequested,
    /// Off-workspace REMOVE-HIDDEN attempt consumed before dispatch. Never retry.
    HiddenRemoveAttempted,
    /// REMOVE-HIDDEN dispatched and checked; no MapWindow was sent.
    HiddenRemoveRequested,
    EnablingInput,
    InputRestored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibilityBaseline {
    pub desktop: u32,
    pub current_desktop: u32,
    pub layer: LayerObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShowProgress {
    pub phase: ShowPhase,
    pub baseline: Option<VisibilityBaseline>,
    pub area: Option<crate::geometry::Rect>,
    pub correction: Option<(crate::geometry::Point, std::time::Instant)>,
    pub correction_sent: bool,
    pub observe: bool,
    pub layout_interrupted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    Duplicate,
    Reply(Status, Reason, Stage),
}

pub fn preflight_detail(reason: Reason, stage: Stage) -> Detail {
    Detail {
        reason,
        stage,
        visibility: Visibility::Unknown,
        layer: LayerObservation::Unknown,
        workspace: Workspace::Unknown,
        progress: Progress::default(),
        placement: PlacementProof::default(),
    }
}

pub fn admit(
    active: &mut Option<ActiveOperation>,
    request: Request,
    destination_desktop: u32,
    lifecycle: Lifecycle,
    local_layer_busy: bool,
    now: std::time::Instant,
) -> Admission {
    if request
        .command
        .validate_destination(destination_desktop)
        .is_err()
    {
        return Admission::Reply(
            Status::ProtocolError,
            Reason::MalformedRequest,
            Stage::Preflight,
        );
    }
    if let Some(current) = active {
        if current.request == request
            && current.placement.destination_desktop == destination_desktop
        {
            return Admission::Duplicate;
        }
    }
    match lifecycle {
        Lifecycle::Starting => {
            return Admission::Reply(Status::Starting, Reason::Starting, Stage::Preflight)
        }
        Lifecycle::Closing => {
            return Admission::Reply(Status::Closing, Reason::Closing, Stage::Shutdown)
        }
        Lifecycle::Ready => {}
    }
    if active.is_some() || local_layer_busy {
        return Admission::Reply(Status::Busy, Reason::Busy, Stage::Preflight);
    }
    *active = Some(ActiveOperation {
        request,
        stage: Stage::Preflight,
        deadline: now + request.command.owner_budget(),
        placement: PlacementProgress {
            destination_desktop,
            baseline: None,
            proof: PlacementProof::default(),
        },
        recover_phase: RecoverPhase::Here,
        detail: preflight_detail(Reason::Unsupported, Stage::Preflight),
        show: ShowProgress {
            observe: true,
            ..ShowProgress::default()
        },
        hide: HideProgress {
            observe: true,
            ..HideProgress::default()
        },
        bring_top: BringTopProgress {
            visibility_cutoff: now + std::time::Duration::from_millis(2500),
            minimized_at_start: false,
            restoration_confirmed: false,
            layer_admitted: None,
            layer_observe: false,
            layer_attempted: false,
            layer_dispatched: false,
            layer_checked: false,
            layer_finished: false,
        },
    });
    Admission::Admitted
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
    InconsistentStatus,
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
        if matches!(command, Command::BringHere | Command::Recover) {
            // Placement results require the command-specific evidence constructor.
            return Err(ResultError::InconsistentConfirmation);
        }
        Self::validated(command, status, detail, confirmed, false)
    }

    pub fn placement(
        command: Command,
        status: Status,
        detail: Detail,
        confirmed: Confirmation,
        progress: &PlacementProgress,
    ) -> Result<Self, ResultError> {
        let p = progress.proof.current;
        let h = progress.proof.history;
        let baseline = progress.baseline;
        let local = confirmed.visibility == Some(ConfirmedVisibility::NotMinimizedHere);
        let minimized = confirmed.visibility == Some(ConfirmedVisibility::Minimized);
        let not_minimized = matches!(
            confirmed.visibility,
            Some(
                ConfirmedVisibility::NotMinimizedHere | ConfirmedVisibility::NotMinimizedElsewhere
            )
        );
        let visibility = match confirmed.visibility {
            Some(ConfirmedVisibility::Minimized) => Visibility::Minimized,
            Some(ConfirmedVisibility::NotMinimizedHere) => Visibility::NotMinimizedHere,
            Some(ConfirmedVisibility::NotMinimizedElsewhere) => Visibility::NotMinimizedElsewhere,
            None => detail.visibility,
        };
        if !matches!(command, Command::BringHere | Command::Recover)
            || command
                .validate_destination(progress.destination_desktop)
                .is_err()
            || detail.placement != progress.proof
            || detail.validate_placement_wire().is_err()
            || (progress.proof != PlacementProof::default()
                && !baseline.is_some_and(|b| {
                    b.desktop != NO_DESTINATION
                        && matches!(
                            b.layer,
                            LayerObservation::Normal
                                | LayerObservation::Above
                                | LayerObservation::Below
                        )
                }))
            || detail.visibility != visibility
            || detail.progress.not_minimized_confirmed != not_minimized
            || detail.progress.above_confirmed != confirmed.above
            || (confirmed.above && (!not_minimized || detail.layer != LayerObservation::Above))
            || (local && detail.workspace != Workspace::Current)
            || (confirmed.visibility == Some(ConfirmedVisibility::NotMinimizedElsewhere)
                && detail.workspace != Workspace::Other)
            || (detail.progress.input_ready
                && (!local || !p.destination || p.geometry != GeometryProof::Verified))
            || (detail.progress.placement_deferred
                && (detail.progress.input_ready || p.geometry != GeometryProof::Deferred))
            || (p.minimization_preserved && (!minimized || !baseline.is_some_and(|b| b.minimized)))
            || (p.layer_preserved && !baseline.is_some_and(|b| b.layer == detail.layer))
            || (h.relocation_confirmed
                && !baseline.is_some_and(|b| {
                    b.desktop != NO_DESTINATION && b.desktop != progress.destination_desktop
                }))
            || (h.restoration_confirmed && !baseline.is_some_and(|b| b.minimized))
        {
            return Err(ResultError::InconsistentConfirmation);
        }
        let meaningful =
            h.relocation_confirmed || (command == Command::Recover && h.restoration_confirmed);
        let complete = p.destination
            && match command {
                Command::BringHere => {
                    p.layer_preserved
                        && baseline.is_some_and(|b| b.minimized == minimized)
                        && ((local
                            && p.geometry == GeometryProof::Verified
                            && detail.progress.input_ready)
                            || (minimized
                                && p.minimization_preserved
                                && p.geometry != GeometryProof::Unknown
                                && !detail.progress.input_ready))
                }
                Command::Recover => {
                    local
                        && confirmed.above
                        && p.geometry == GeometryProof::Verified
                        && detail.progress.input_ready
                }
                _ => unreachable!(),
            };
        let stage_valid = match command {
            Command::BringHere => matches!(
                detail.stage,
                Stage::Preflight | Stage::Placement | Stage::Geometry | Stage::Shutdown
            ),
            Command::Recover => detail.stage != Stage::Hide,
            _ => false,
        };
        if !stage_valid || (command == Command::BringHere && h.restoration_confirmed) {
            return Err(ResultError::InconsistentStatus);
        }
        match status {
            Status::Success => {
                if detail.reason != Reason::None
                    || !complete
                    || (command == Command::Recover && detail.stage != Stage::Layer)
                    || (command == Command::BringHere
                        && !matches!(detail.stage, Stage::Placement | Stage::Geometry))
                {
                    return Err(ResultError::UnconfirmedSuccess);
                }
            }
            Status::Failed | Status::Partial => {
                if matches!(
                    detail.reason,
                    Reason::None
                        | Reason::Starting
                        | Reason::Busy
                        | Reason::MalformedRequest
                        | Reason::WrongVersion
                        | Reason::UnknownCommand
                        | Reason::StaleInstance
                ) || (detail.stage == Stage::Preflight && meaningful)
                {
                    return Err(ResultError::InconsistentStatus);
                }
                if status == Status::Partial && !meaningful {
                    return Err(ResultError::InvalidPartial);
                }
                if status == Status::Failed && meaningful {
                    return Err(ResultError::InconsistentStatus);
                }
            }
            Status::Busy | Status::Starting | Status::Closing | Status::ProtocolError => {
                let (reason_valid, stage) = match status {
                    Status::Busy => (detail.reason == Reason::Busy, Stage::Preflight),
                    Status::Starting => (detail.reason == Reason::Starting, Stage::Preflight),
                    Status::Closing => (detail.reason == Reason::Closing, Stage::Shutdown),
                    Status::ProtocolError => (
                        matches!(
                            detail.reason,
                            Reason::MalformedRequest
                                | Reason::WrongVersion
                                | Reason::UnknownCommand
                                | Reason::StaleInstance
                        ),
                        Stage::Preflight,
                    ),
                    _ => unreachable!(),
                };
                if !reason_valid
                    || detail.stage != stage
                    || detail.progress != Progress::default()
                    || detail.placement != PlacementProof::default()
                    || confirmed != Confirmation::default()
                {
                    return Err(ResultError::InconsistentStatus);
                }
            }
        }
        Ok(Self { status, detail })
    }

    /// Constructor-only historical proof; current confirmation fields retain
    /// their M03.2 meanings, including after evidence loss or body destruction.
    pub fn bring_top(
        status: Status,
        detail: Detail,
        confirmed: Confirmation,
        progress: &BringTopProgress,
    ) -> Result<Self, ResultError> {
        Self::validated(
            Command::BringTop,
            status,
            detail,
            confirmed,
            progress.minimized_at_start && progress.restoration_confirmed,
        )
    }

    fn validated(
        command: Command,
        status: Status,
        detail: Detail,
        confirmed: Confirmation,
        restoration_proven: bool,
    ) -> Result<Self, ResultError> {
        if detail.placement != PlacementProof::default() {
            return Err(ResultError::InconsistentConfirmation);
        }
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
            || command == Command::BringTop
                && ((confirmed.above && !not_minimized)
                    || (detail.progress.input_ready
                        && confirmed.visibility != Some(ConfirmedVisibility::NotMinimizedHere))
                    || (detail.progress.placement_deferred
                        && confirmed.visibility
                            != Some(ConfirmedVisibility::NotMinimizedElsewhere))
                    || (detail.progress.input_ready && detail.progress.placement_deferred))
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
                    Command::Hide => {
                        confirmed.visibility == Some(ConfirmedVisibility::Minimized)
                            && detail.workspace != Workspace::Unknown
                    }
                    Command::Show => {
                        not_minimized
                            && matches!(
                                detail.layer,
                                LayerObservation::Normal
                                    | LayerObservation::Above
                                    | LayerObservation::Below
                            )
                    }
                    Command::BringTop => not_minimized && confirmed.above,
                    Command::BringHere | Command::Recover => unreachable!("separate proof rules"),
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
        let partial_valid = if command == Command::BringTop {
            restoration_proven
        } else {
            detail.progress.host_mutation_dispatched
                && (confirmed.visibility.is_some() || confirmed.above)
        };
        if status == Status::Partial && !partial_valid {
            return Err(ResultError::InvalidPartial);
        }
        if command == Command::BringTop
            && ((status == Status::Failed && restoration_proven)
                || (status == Status::Busy
                    && (restoration_proven || detail.progress.host_mutation_dispatched)))
        {
            return Err(ResultError::InconsistentStatus);
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
            destination_desktop: crate::control::NO_DESTINATION,
            control_xid: owner().control_xid,
            epoch: r.epoch,
            request_id: r.request_id,
            command: r.command,
        }
    }
    fn detail() -> Detail {
        Detail {
            placement: PlacementProof::default(),
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
        let words = [0x4452_4d31, 2, 0, 0x1234, 0x89ab_cdef, 2, 0x5678];
        assert_eq!(owner().encode(), Ok(words));
        assert_eq!(OwnerDescriptor::decode(&words), Ok(owner()));
        let words = [
            0x4452_4d31,
            2,
            0x1234,
            0x89ab_cdef,
            0x7654_3210,
            0xfedc_ba98,
            3,
            NO_DESTINATION,
        ];
        assert_eq!(marker().encode(), Ok(words));
        assert_eq!(CallerMarker::decode(&words), Ok(marker()));
    }
    #[test]
    fn request_and_response_interoperability() {
        let words = [0x0002_0003, 0x7654_3210, 0xfedc_ba98, 0x9876, 0x89ab_cdef];
        assert_eq!(request().encode(), Ok(words));
        assert_eq!(Request::decode(&words), Ok(request()));
        let words = [0x0002_0001, 0x7654_3210, 0xfedc_ba98, 0x1234, 0];
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
                0x20000 | value
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
            assert_eq!(words[0], 0x20000 | value);
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
            placement: PlacementProof::default(),
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
        for bit in 31..32 {
            assert_eq!(Detail::decode(1 << bit), Err(DecodeError::ReservedBits));
        }
        for word in [18, 255, 7 << 8, 5 << 11, 6 << 11, 7 << 11, 7 << 21] {
            assert!(Detail::decode(word).is_err());
        }
        for value in [0, 6, u32::MAX] {
            assert!(Command::try_from(value).is_err());
        }
        for value in [0, 4, u32::MAX] {
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
            }
            if len != 8 {
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
            (1, 1, DecodeError::Version),
        ] {
            let mut d = owner().encode().unwrap();
            d[index] = bad;
            assert_eq!(OwnerDescriptor::decode(&d), Err(expected));
            let mut m = marker().encode().unwrap();
            m[index] = bad;
            assert_eq!(CallerMarker::decode(&m), Err(expected));
        }
        for header in [3, 0x10003, 0xffff0003] {
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
        m[6] = 6;
        assert!(CallerMarker::decode(&m).is_err());
        let mut r = request().encode().unwrap();
        r[0] = 0x20006;
        assert!(Request::decode(&r).is_err());
        let mut r = response().encode().unwrap();
        r[0] = 0x20007;
        assert!(TerminalResponse::decode(&r).is_err());
        r[0] = 0x20001;
        r[4] = 1 << 31;
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
    fn proven_restoration() -> BringTopProgress {
        let mut active = None;
        admit(
            &mut active,
            request(),
            crate::control::NO_DESTINATION,
            Lifecycle::Ready,
            false,
            std::time::Instant::now(),
        );
        let mut progress = active.unwrap().bring_top;
        progress.minimized_at_start = true;
        progress.restoration_confirmed = true;
        progress
    }
    #[test]
    fn confirmed_results_and_partial_progress_remain_distinct() {
        let d = Detail {
            visibility: Visibility::Minimized,
            workspace: Workspace::Current,
            layer: LayerObservation::Normal,
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
                layer: LayerObservation::Normal,
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
                OperationResult::bring_top(Status::Partial, partial, c, &proven_restoration())
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
            layer: LayerObservation::Normal,
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
            layer: LayerObservation::Normal,
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
    fn hide_success_requires_known_workspace_but_failure_and_partial_do_not() {
        let d = Detail {
            visibility: Visibility::Minimized,
            layer: LayerObservation::Normal,
            stage: Stage::Hide,
            progress: Progress {
                host_mutation_dispatched: true,
                ..Progress::default()
            },
            ..detail()
        };
        let c = Confirmation {
            visibility: Some(ConfirmedVisibility::Minimized),
            above: false,
        };
        assert_eq!(
            OperationResult::new(Command::Hide, Status::Success, d, c),
            Err(ResultError::UnconfirmedSuccess)
        );
        for status in [Status::Failed, Status::Partial] {
            let result = OperationResult::new(Command::Hide, status, d, c).unwrap();
            assert_eq!(result.status(), status);
            assert_eq!(result.detail(), d);
        }
        for workspace in [Workspace::Current, Workspace::Other, Workspace::AllDesktops] {
            assert_eq!(
                OperationResult::new(Command::Hide, Status::Success, Detail { workspace, ..d }, c)
                    .unwrap()
                    .exit_code(),
                ExitCode::Success
            );
        }
    }

    #[test]
    fn show_success_requires_verifiable_layer_but_failure_and_partial_do_not() {
        for (visibility, confirmed, workspace) in [
            (
                Visibility::NotMinimizedHere,
                ConfirmedVisibility::NotMinimizedHere,
                Workspace::Current,
            ),
            (
                Visibility::NotMinimizedHere,
                ConfirmedVisibility::NotMinimizedHere,
                Workspace::AllDesktops,
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
                stage: Stage::Show,
                progress: Progress {
                    not_minimized_confirmed: true,
                    host_mutation_dispatched: true,
                    placement_deferred: workspace == Workspace::Other,
                    input_ready: workspace != Workspace::Other,
                    ..Progress::default()
                },
                ..detail()
            };
            let c = Confirmation {
                visibility: Some(confirmed),
                above: false,
            };
            for layer in [LayerObservation::Unknown, LayerObservation::Conflict] {
                let d = Detail { layer, ..d };
                assert_eq!(
                    OperationResult::new(Command::Show, Status::Success, d, c),
                    Err(ResultError::UnconfirmedSuccess)
                );
                for status in [Status::Failed, Status::Partial] {
                    let result = OperationResult::new(Command::Show, status, d, c).unwrap();
                    assert_eq!(result.status(), status);
                    assert_eq!(result.detail(), d);
                }
            }
            for layer in [
                LayerObservation::Normal,
                LayerObservation::Above,
                LayerObservation::Below,
            ] {
                assert_eq!(
                    OperationResult::new(Command::Show, Status::Success, Detail { layer, ..d }, c)
                        .unwrap()
                        .exit_code(),
                    ExitCode::Success
                );
            }
        }
    }

    #[test]
    fn bring_top_partial_requires_actual_restoration_not_dispatch_or_current_visibility() {
        let mut d = preflight_detail(Reason::ConfirmationTimeout, Stage::Layer);
        d.visibility = Visibility::NotMinimizedHere;
        d.workspace = Workspace::Current;
        d.layer = LayerObservation::Normal;
        d.progress.not_minimized_confirmed = true;
        d.progress.host_mutation_dispatched = true;
        let c = Confirmation {
            visibility: Some(ConfirmedVisibility::NotMinimizedHere),
            above: false,
        };
        assert_eq!(
            OperationResult::new(Command::BringTop, Status::Partial, d, c),
            Err(ResultError::InvalidPartial)
        );
        let mut proof = proven_restoration();
        for (minimized, restored) in [(false, false), (false, true), (true, false)] {
            proof.minimized_at_start = minimized;
            proof.restoration_confirmed = restored;
            assert_eq!(
                OperationResult::bring_top(Status::Partial, d, c, &proof),
                Err(ResultError::InvalidPartial)
            );
        }
        proof.minimized_at_start = true;
        proof.restoration_confirmed = true;
        assert_eq!(
            OperationResult::bring_top(Status::Partial, d, c, &proof)
                .unwrap()
                .exit_code(),
            ExitCode::Partial
        );
        // History remains sufficient after current evidence is lost; wire bits
        // keep current-state meaning and are not used as historical markers.
        let unknown = preflight_detail(Reason::BodyDestroyed, Stage::Layer);
        assert_eq!(
            OperationResult::bring_top(Status::Partial, unknown, Confirmation::default(), &proof)
                .unwrap()
                .status(),
            Status::Partial
        );
        assert_eq!(VERSION, 2);
        assert_eq!(Detail::decode(unknown.encode()).unwrap(), unknown);
    }
    #[test]
    fn owner_budgets_and_visibility_cutoff_are_admission_relative_and_duplicates_preserve_them() {
        let t0 = std::time::Instant::now();
        for (command, millis) in [
            (Command::Hide, 1000),
            (Command::Show, 2500),
            (Command::BringTop, 3500),
        ] {
            let req = Request {
                command,
                ..request()
            };
            let mut active = None;
            assert_eq!(
                admit(
                    &mut active,
                    req,
                    crate::control::NO_DESTINATION,
                    Lifecycle::Ready,
                    false,
                    t0
                ),
                Admission::Admitted
            );
            let original = active.unwrap();
            assert_eq!(
                original.deadline,
                t0 + std::time::Duration::from_millis(millis)
            );
            assert_eq!(
                original.bring_top.visibility_cutoff,
                t0 + std::time::Duration::from_millis(2500)
            );
            assert_eq!(
                admit(
                    &mut active,
                    req,
                    crate::control::NO_DESTINATION,
                    Lifecycle::Ready,
                    false,
                    t0 + std::time::Duration::from_millis(100)
                ),
                Admission::Duplicate
            );
            assert_eq!(active, Some(original));
        }
    }
    #[test]
    fn all_remote_command_pairings_and_local_layer_busy_preserve_single_owner() {
        let t0 = std::time::Instant::now();
        for first in [Command::Hide, Command::Show, Command::BringTop] {
            for second in [Command::Hide, Command::Show, Command::BringTop] {
                let mut active = None;
                admit(
                    &mut active,
                    Request {
                        command: first,
                        ..request()
                    },
                    crate::control::NO_DESTINATION,
                    Lifecycle::Ready,
                    false,
                    t0,
                );
                let original = active;
                let other = Request {
                    command: second,
                    request_id: 123,
                    ..request()
                };
                assert_eq!(
                    admit(
                        &mut active,
                        other,
                        crate::control::NO_DESTINATION,
                        Lifecycle::Ready,
                        false,
                        t0
                    ),
                    Admission::Reply(Status::Busy, Reason::Busy, Stage::Preflight)
                );
                assert_eq!(active, original);
            }
        }
        let mut active = None;
        assert_eq!(
            admit(
                &mut active,
                request(),
                crate::control::NO_DESTINATION,
                Lifecycle::Ready,
                true,
                t0
            ),
            Admission::Reply(Status::Busy, Reason::Busy, Stage::Preflight)
        );
        assert!(active.is_none());
    }
    #[test]
    fn bring_top_busy_and_failed_status_cannot_discard_known_operation_progress() {
        let mut proof = proven_restoration();
        let mut d = preflight_detail(Reason::Busy, Stage::Preflight);
        assert!(
            OperationResult::bring_top(Status::Busy, d, Confirmation::default(), &proof).is_err()
        );
        assert!(
            OperationResult::bring_top(Status::Failed, d, Confirmation::default(), &proof).is_err()
        );
        proof.minimized_at_start = false;
        proof.restoration_confirmed = false;
        d.progress.host_mutation_dispatched = true;
        assert!(
            OperationResult::bring_top(Status::Busy, d, Confirmation::default(), &proof).is_err()
        );
    }
    #[test]
    fn bring_top_result_cross_product_preserves_success_and_current_evidence_invariants() {
        let mut proof = proven_restoration();
        for status in [
            Status::Success,
            Status::Partial,
            Status::Failed,
            Status::Busy,
        ] {
            for reason in [Reason::None, Reason::ConfirmationTimeout] {
                for stage in [
                    Stage::Preflight,
                    Stage::Show,
                    Stage::Geometry,
                    Stage::Layer,
                    Stage::Shutdown,
                ] {
                    for vis in [
                        None,
                        Some(ConfirmedVisibility::Minimized),
                        Some(ConfirmedVisibility::NotMinimizedHere),
                        Some(ConfirmedVisibility::NotMinimizedElsewhere),
                    ] {
                        for workspace in [
                            Workspace::Unknown,
                            Workspace::Current,
                            Workspace::Other,
                            Workspace::AllDesktops,
                        ] {
                            for layer in [
                                LayerObservation::Unknown,
                                LayerObservation::Normal,
                                LayerObservation::Above,
                                LayerObservation::Below,
                                LayerObservation::Conflict,
                            ] {
                                for bits in 0..32_u32 {
                                    let p = Progress {
                                        not_minimized_confirmed: bits & 1 != 0,
                                        above_confirmed: bits & 2 != 0,
                                        placement_deferred: bits & 4 != 0,
                                        host_mutation_dispatched: bits & 8 != 0,
                                        input_ready: bits & 16 != 0,
                                    };
                                    let visibility = match vis {
                                        None => Visibility::Unknown,
                                        Some(ConfirmedVisibility::Minimized) => {
                                            Visibility::Minimized
                                        }
                                        Some(ConfirmedVisibility::NotMinimizedHere) => {
                                            Visibility::NotMinimizedHere
                                        }
                                        Some(ConfirmedVisibility::NotMinimizedElsewhere) => {
                                            Visibility::NotMinimizedElsewhere
                                        }
                                    };
                                    let d = Detail {
                                        placement: PlacementProof::default(),
                                        reason,
                                        stage,
                                        visibility,
                                        workspace,
                                        layer,
                                        progress: p,
                                    };
                                    for above in [false, true] {
                                        for history in 0..4 {
                                            proof.minimized_at_start = history & 1 != 0;
                                            proof.restoration_confirmed = history & 2 != 0;
                                            if let Ok(r) = OperationResult::bring_top(
                                                status,
                                                d,
                                                Confirmation {
                                                    visibility: vis,
                                                    above,
                                                },
                                                &proof,
                                            ) {
                                                assert_eq!(r.detail(), d);
                                                assert_eq!(Detail::decode(d.encode()).unwrap(), d);
                                                assert_eq!(p.not_minimized_confirmed,matches!(vis,Some(ConfirmedVisibility::NotMinimizedHere|ConfirmedVisibility::NotMinimizedElsewhere)));
                                                assert_eq!(p.above_confirmed, above);
                                                if above {
                                                    assert_eq!(layer, LayerObservation::Above);
                                                    assert!(p.not_minimized_confirmed);
                                                }
                                                if p.input_ready {
                                                    assert_eq!(
                                                        vis,
                                                        Some(ConfirmedVisibility::NotMinimizedHere)
                                                    );
                                                    assert!(matches!(
                                                        workspace,
                                                        Workspace::Current | Workspace::AllDesktops
                                                    ));
                                                    assert!(!p.placement_deferred);
                                                }
                                                if p.placement_deferred {
                                                    assert_eq!(vis,Some(ConfirmedVisibility::NotMinimizedElsewhere));
                                                    assert_eq!(workspace, Workspace::Other);
                                                    assert!(!p.input_ready);
                                                }
                                                match status {
                                                    Status::Success => {
                                                        assert_eq!(reason, Reason::None);
                                                        assert!(above && p.not_minimized_confirmed);
                                                        assert!(
                                                            p.input_ready || p.placement_deferred
                                                        );
                                                    }
                                                    Status::Partial => assert_eq!(history, 3),
                                                    Status::Failed => assert_ne!(history, 3),
                                                    Status::Busy => {
                                                        assert_ne!(history, 3);
                                                        assert!(!p.host_mutation_dispatched);
                                                    }
                                                    _ => unreachable!(),
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn historical_proof_not_current_state_or_aggregate_dispatch_is_the_only_partial_authority() {
        let mut proof = proven_restoration();
        for history in 0..4 {
            for dispatched in [false, true] {
                for current in [
                    Visibility::Unknown,
                    Visibility::Minimized,
                    Visibility::NotMinimizedHere,
                    Visibility::NotMinimizedElsewhere,
                ] {
                    proof.minimized_at_start = history & 1 != 0;
                    proof.restoration_confirmed = history & 2 != 0;
                    let vis = match current {
                        Visibility::Minimized => Some(ConfirmedVisibility::Minimized),
                        Visibility::NotMinimizedHere => Some(ConfirmedVisibility::NotMinimizedHere),
                        Visibility::NotMinimizedElsewhere => {
                            Some(ConfirmedVisibility::NotMinimizedElsewhere)
                        }
                        _ => None,
                    };
                    let not_minimized = matches!(
                        current,
                        Visibility::NotMinimizedHere | Visibility::NotMinimizedElsewhere
                    );
                    for above in [false, true]
                        .into_iter()
                        .filter(|above| !*above || not_minimized)
                    {
                        let d = Detail {
                            visibility: current,
                            workspace: if current == Visibility::NotMinimizedElsewhere {
                                Workspace::Other
                            } else {
                                Workspace::Current
                            },
                            layer: if above {
                                LayerObservation::Above
                            } else {
                                LayerObservation::Normal
                            },
                            progress: Progress {
                                not_minimized_confirmed: not_minimized,
                                above_confirmed: above,
                                host_mutation_dispatched: dispatched,
                                ..Progress::default()
                            },
                            ..preflight_detail(Reason::ConfirmationTimeout, Stage::Layer)
                        };
                        assert_eq!(
                            OperationResult::bring_top(
                                Status::Partial,
                                d,
                                Confirmation {
                                    visibility: vis,
                                    above
                                },
                                &proof
                            )
                            .is_ok(),
                            history == 3
                        );
                        assert_eq!(
                            OperationResult::new(
                                Command::BringTop,
                                Status::Partial,
                                d,
                                Confirmation {
                                    visibility: vis,
                                    above
                                }
                            ),
                            Err(ResultError::InvalidPartial)
                        );
                        assert_eq!(d.encode() & 0xff00_0000, 0);
                    }
                }
            }
        }
        assert_eq!(VERSION, 2);
    }
    #[test]
    fn bring_top_failed_results_cannot_claim_unqualified_current_readiness_or_above() {
        let mut d = preflight_detail(Reason::Unverifiable, Stage::Layer);
        let proof = proven_restoration();
        for status in [Status::Failed, Status::Partial] {
            for bit in [0, 1, 2] {
                d.progress = Progress::default();
                d.layer = LayerObservation::Above;
                let mut c = Confirmation::default();
                match bit {
                    0 => {
                        d.progress.above_confirmed = true;
                        c.above = true;
                    }
                    1 => d.progress.input_ready = true,
                    _ => d.progress.placement_deferred = true,
                }
                assert_eq!(
                    OperationResult::bring_top(status, d, c, &proof),
                    Err(ResultError::InconsistentConfirmation),
                    "status={status:?} bit={bit}"
                );
            }
        }
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

#[cfg(test)]
mod placement_tests {
    use super::*;
    use std::time::{Duration, Instant};

    const COMMANDS: [Command; 5] = [
        Command::Hide,
        Command::Show,
        Command::BringTop,
        Command::BringHere,
        Command::Recover,
    ];
    fn destination(command: Command) -> u32 {
        if matches!(command, Command::BringHere | Command::Recover) {
            7
        } else {
            NO_DESTINATION
        }
    }
    fn request(command: Command) -> Request {
        Request {
            command,
            request_id: u64::MAX,
            reply_xid: 9,
            epoch: 11,
        }
    }
    fn marker(command: Command) -> CallerMarker {
        CallerMarker {
            control_xid: 10,
            epoch: 11,
            request_id: u64::MAX,
            command,
            destination_desktop: destination(command),
        }
    }
    fn progress() -> PlacementProgress {
        PlacementProgress {
            destination_desktop: 7,
            baseline: Some(PlacementBaseline {
                desktop: 2,
                minimized: true,
                layer: LayerObservation::Normal,
            }),
            proof: PlacementProof::default(),
        }
    }
    fn complete(command: Command) -> (Detail, Confirmation, PlacementProgress) {
        let mut p = progress();
        p.baseline.as_mut().unwrap().minimized = command == Command::Recover;
        p.proof.current = PlacementConfirmation {
            destination: true,
            geometry: GeometryProof::Verified,
            layer_preserved: command == Command::BringHere,
            minimization_preserved: false,
        };
        let c = Confirmation {
            visibility: Some(ConfirmedVisibility::NotMinimizedHere),
            above: command == Command::Recover,
        };
        let d = Detail {
            visibility: Visibility::NotMinimizedHere,
            layer: if c.above {
                LayerObservation::Above
            } else {
                LayerObservation::Normal
            },
            workspace: Workspace::Current,
            progress: Progress {
                not_minimized_confirmed: true,
                above_confirmed: c.above,
                input_ready: true,
                ..Progress::default()
            },
            placement: p.proof,
            ..preflight_detail(
                Reason::None,
                if c.above {
                    Stage::Layer
                } else {
                    Stage::Placement
                },
            )
        };
        (d, c, p)
    }
    fn result(
        command: Command,
        status: Status,
        d: Detail,
        c: Confirmation,
        p: PlacementProgress,
    ) -> Result<OperationResult, ResultError> {
        OperationResult::placement(command, status, d, c, &p)
    }
    fn terminal(detail: Detail) -> TerminalResponse {
        TerminalResponse {
            status: Status::Success,
            request_id: u64::MAX,
            control_xid: 10,
            detail,
        }
    }

    #[test]
    fn all_five_commands_keep_exact_numeric_values_and_transport_widths() {
        for (i, command) in COMMANDS.into_iter().enumerate() {
            let r = request(command);
            let m = marker(command);
            assert_eq!(r.encode().unwrap()[0], (2 << 16) | (i as u32 + 1));
            assert_eq!(Request::decode(&r.encode().unwrap()), Ok(r));
            assert_eq!(
                m.encode().unwrap(),
                [
                    MAGIC,
                    2,
                    10,
                    11,
                    u32::MAX,
                    u32::MAX,
                    i as u32 + 1,
                    destination(command)
                ]
            );
            assert_eq!(CallerMarker::decode(&m.encode().unwrap()), Ok(m));
            // Command and destination correlate through the marker/request ID;
            // the unchanged five-word reply deliberately has no command field.
            for status in [
                Status::Success,
                Status::Failed,
                Status::Partial,
                Status::Busy,
                Status::Starting,
                Status::Closing,
                Status::ProtocolError,
            ] {
                let t = TerminalResponse {
                    status,
                    ..terminal(preflight_detail(Reason::None, Stage::Preflight))
                };
                assert_eq!(TerminalResponse::decode(&t.encode().unwrap()), Ok(t));
            }
        }
    }

    #[test]
    fn exact_marker_rejects_v1_trailing_words_wrong_headers_and_unknown_command() {
        let words = marker(Command::Recover).encode().unwrap();
        assert_eq!(CallerMarker::decode(&words[..7]), Err(DecodeError::Length));
        let mut trailing = words.to_vec();
        trailing.push(0);
        assert_eq!(CallerMarker::decode(&trailing), Err(DecodeError::Length));
        for (index, word, error) in [
            (0, 0, DecodeError::Magic),
            (1, 1, DecodeError::Version),
            (1, 3, DecodeError::Version),
            (6, 6, DecodeError::UnknownValue("Command", 6)),
        ] {
            let mut bad = words;
            bad[index] = word;
            assert_eq!(CallerMarker::decode(&bad), Err(error));
        }
    }

    #[test]
    fn destination_rules_accept_every_ordinary_u32_boundary_without_desktop_count_assumptions() {
        for command in COMMANDS {
            for destination_desktop in [0, 1, 7, u32::MAX - 1, NO_DESTINATION] {
                let m = CallerMarker {
                    destination_desktop,
                    ..marker(command)
                };
                let expected = matches!(command, Command::BringHere | Command::Recover)
                    == (destination_desktop != NO_DESTINATION);
                assert_eq!(m.encode().is_ok(), expected);
                let mut words = marker(command).encode().unwrap();
                words[7] = destination_desktop;
                if expected {
                    assert_eq!(CallerMarker::decode(&words), Ok(m));
                    assert!(m.matches(10, request(command)));
                } else {
                    assert_eq!(
                        CallerMarker::decode(&words),
                        Err(DecodeError::InvalidDestination)
                    );
                }
            }
        }
    }

    #[test]
    fn frozen_destination_is_part_of_duplicate_identity_and_is_never_retargeted() {
        for command in [Command::BringHere, Command::Recover] {
            let now = Instant::now();
            let mut active = None;
            let req = request(command);
            assert_eq!(
                admit(&mut active, req, 7, Lifecycle::Ready, false, now),
                Admission::Admitted
            );
            let original = active;
            for elapsed in [1, 3500, 7000, 9000] {
                assert_eq!(
                    admit(
                        &mut active,
                        req,
                        7,
                        Lifecycle::Closing,
                        true,
                        now + Duration::from_millis(elapsed)
                    ),
                    Admission::Duplicate
                );
                assert_eq!(active, original);
            }
            assert_ne!(
                marker(command),
                CallerMarker {
                    destination_desktop: 8,
                    ..marker(command)
                }
            );
            assert_eq!(
                admit(&mut active, req, 8, Lifecycle::Ready, false, now),
                Admission::Reply(Status::Busy, Reason::Busy, Stage::Preflight)
            );
            assert_eq!(active, original);
        }
    }

    #[test]
    fn all_five_owner_and_caller_deadlines_are_fixed_from_admission() {
        for (command, owner, caller) in [
            (Command::Hide, 1000, 4000),
            (Command::Show, 2500, 4000),
            (Command::BringTop, 3500, 5000),
            (Command::BringHere, 3500, 5000),
            (Command::Recover, 7000, 9000),
        ] {
            let now = Instant::now();
            let mut active = None;
            assert_eq!(command.caller_budget(), Duration::from_millis(caller));
            assert_eq!(
                admit(
                    &mut active,
                    request(command),
                    destination(command),
                    Lifecycle::Ready,
                    false,
                    now
                ),
                Admission::Admitted
            );
            let mut op = active.unwrap();
            assert_eq!(op.deadline, now + Duration::from_millis(owner));
            assert_eq!(
                op.bring_top.visibility_cutoff,
                now + Duration::from_millis(2500)
            );
            op.recover_phase = RecoverPhase::Top;
            op.placement.clear_current();
            assert_eq!(op.deadline, now + Duration::from_millis(owner));
        }
    }

    #[test]
    fn all_twenty_five_command_pairs_share_one_admission_and_busy_policy() {
        let now = Instant::now();
        for first in COMMANDS {
            for second in COMMANDS {
                let mut active = None;
                assert_eq!(
                    admit(
                        &mut active,
                        request(first),
                        destination(first),
                        Lifecycle::Ready,
                        false,
                        now
                    ),
                    Admission::Admitted
                );
                let original = active;
                let other = Request {
                    request_id: 1,
                    ..request(second)
                };
                assert_eq!(
                    admit(
                        &mut active,
                        other,
                        destination(second),
                        Lifecycle::Ready,
                        false,
                        now
                    ),
                    Admission::Reply(Status::Busy, Reason::Busy, Stage::Preflight)
                );
                assert_eq!(active, original);
            }
        }
    }

    #[test]
    fn new_commands_obey_starting_closing_local_busy_and_invalid_destination_admission() {
        let now = Instant::now();
        for command in [Command::BringHere, Command::Recover] {
            for (lifecycle, busy, expected) in [
                (
                    Lifecycle::Starting,
                    false,
                    Admission::Reply(Status::Starting, Reason::Starting, Stage::Preflight),
                ),
                (
                    Lifecycle::Closing,
                    false,
                    Admission::Reply(Status::Closing, Reason::Closing, Stage::Shutdown),
                ),
                (
                    Lifecycle::Ready,
                    true,
                    Admission::Reply(Status::Busy, Reason::Busy, Stage::Preflight),
                ),
            ] {
                let mut active = None;
                assert_eq!(
                    admit(&mut active, request(command), 7, lifecycle, busy, now),
                    expected
                );
                assert!(active.is_none());
            }
            let mut active = None;
            assert_eq!(
                admit(
                    &mut active,
                    request(command),
                    NO_DESTINATION,
                    Lifecycle::Ready,
                    false,
                    now
                ),
                Admission::Reply(
                    Status::ProtocolError,
                    Reason::MalformedRequest,
                    Stage::Preflight
                )
            );
            assert!(active.is_none());
        }
    }

    #[test]
    fn v2_detail_bits_preserve_old_fields_and_separate_current_from_history() {
        let (mut d, _, mut p) = complete(Command::BringHere);
        p.proof.history = RecoveryHistory {
            relocation_confirmed: true,
            restoration_confirmed: true,
        };
        d.placement = p.proof;
        assert_eq!(
            d.encode() & 0xff00_0000,
            (1 << 24) | (1 << 25) | (1 << 26) | (2 << 27) | (1 << 30)
        );
        assert_eq!(Detail::decode(d.encode()), Ok(d));
        assert_eq!(
            TerminalResponse::decode(&terminal(d).encode().unwrap()),
            Ok(terminal(d))
        );
        p.clear_current();
        d.placement = p.proof;
        d.workspace = Workspace::Unknown;
        d.visibility = Visibility::Unknown;
        d.progress = Progress::default();
        assert_eq!(d.encode() & 0xff00_0000, (1 << 25) | (1 << 26));
        assert_eq!(Detail::decode(d.encode()), Ok(d));
        assert_eq!(Detail::decode(1 << 31), Err(DecodeError::ReservedBits));
        assert_eq!(
            Detail::decode(3 << 27),
            Err(DecodeError::UnknownValue("GeometryProof", 3))
        );
    }

    #[test]
    fn ordinary_bring_here_success_requires_current_geometry_input_destination_and_preserved_layer()
    {
        let (d, c, p) = complete(Command::BringHere);
        assert!(result(Command::BringHere, Status::Success, d, c, p).is_ok());
        for field in 0..5 {
            let (mut d, c, mut p) = complete(Command::BringHere);
            match field {
                0 => p.proof.current.destination = false,
                1 => p.proof.current.geometry = GeometryProof::Unknown,
                2 => p.proof.current.layer_preserved = false,
                3 => d.progress.input_ready = false,
                4 => d.workspace = Workspace::Other,
                _ => unreachable!(),
            }
            d.placement = p.proof;
            assert!(result(Command::BringHere, Status::Success, d, c, p).is_err());
        }
    }

    fn minimized_here(geometry: GeometryProof) -> (Detail, Confirmation, PlacementProgress) {
        let (mut d, _, mut p) = complete(Command::BringHere);
        p.baseline.as_mut().unwrap().minimized = true;
        p.proof.current.geometry = geometry;
        p.proof.current.minimization_preserved = true;
        d.placement = p.proof;
        d.visibility = Visibility::Minimized;
        d.progress = Progress {
            placement_deferred: geometry == GeometryProof::Deferred,
            ..Progress::default()
        };
        (
            d,
            Confirmation {
                visibility: Some(ConfirmedVisibility::Minimized),
                above: false,
            },
            p,
        )
    }

    #[test]
    fn minimized_here_success_can_report_admissible_deferred_or_verified_geometry_with_input_disabled(
    ) {
        for geometry in [GeometryProof::Deferred, GeometryProof::Verified] {
            for layer in [
                LayerObservation::Normal,
                LayerObservation::Above,
                LayerObservation::Below,
            ] {
                let (mut d, c, mut p) = minimized_here(geometry);
                d.layer = layer;
                p.baseline.as_mut().unwrap().layer = layer;
                let r = result(Command::BringHere, Status::Success, d, c, p).unwrap();
                assert!(!r.detail().progress.input_ready);
                assert_eq!(
                    r.detail().progress.placement_deferred,
                    geometry == GeometryProof::Deferred
                );
                assert_eq!(Detail::decode(d.encode()), Ok(d));
                assert_eq!(
                    d.encode() & 0xff00_0000,
                    (1 << 24) | ((geometry as u32) << 27) | (1 << 29) | (1 << 30)
                );
            }
        }
    }

    #[test]
    fn minimized_here_cannot_claim_input_readiness_or_skip_minimization_and_fit_evidence() {
        for field in 0..7 {
            let (mut d, mut c, mut p) = minimized_here(GeometryProof::Deferred);
            match field {
                0 => d.progress.input_ready = true,
                1 => c.visibility = None,
                2 => p.proof.current.minimization_preserved = false,
                3 => p.proof.current.geometry = GeometryProof::Unknown,
                4 => p.baseline.as_mut().unwrap().minimized = false,
                5 => d.progress.placement_deferred = false,
                6 => p.proof.current.destination = false,
                _ => unreachable!(),
            }
            d.placement = p.proof;
            assert!(
                result(Command::BringHere, Status::Success, d, c, p).is_err(),
                "field {field}"
            );
        }
    }

    #[test]
    fn nonminimized_here_cannot_use_the_minimized_geometry_exception() {
        let (mut d, c, mut p) = complete(Command::BringHere);
        p.proof.current.geometry = GeometryProof::Deferred;
        d.progress.placement_deferred = true;
        d.progress.input_ready = false;
        d.placement = p.proof;
        assert!(result(Command::BringHere, Status::Success, d, c, p).is_err());
    }

    #[test]
    fn recover_success_requires_every_full_current_postcondition_without_historical_substitution() {
        for history in 0..4 {
            let (mut d, c, mut p) = complete(Command::Recover);
            p.proof.history = RecoveryHistory {
                relocation_confirmed: history & 1 != 0,
                restoration_confirmed: history & 2 != 0,
            };
            d.placement = p.proof;
            assert!(result(Command::Recover, Status::Success, d, c, p).is_ok());
            for field in 0..7 {
                let (mut bad, mut confirmation, mut proof) = (d, c, p);
                match field {
                    0 => proof.proof.current.destination = false,
                    1 => proof.proof.current.geometry = GeometryProof::Unknown,
                    2 => bad.progress.input_ready = false,
                    3 => {
                        confirmation.above = false;
                        bad.progress.above_confirmed = false;
                    }
                    4 => {
                        confirmation.visibility = None;
                        bad.progress.not_minimized_confirmed = false;
                    }
                    5 => bad.layer = LayerObservation::Normal,
                    6 => bad.stage = Stage::Placement,
                    _ => unreachable!(),
                }
                bad.placement = proof.proof;
                assert!(
                    result(Command::Recover, Status::Success, bad, confirmation, proof).is_err()
                );
            }
        }
    }

    #[test]
    fn recover_partial_accepts_each_meaningful_history_after_current_evidence_is_lost() {
        for history in 1..4 {
            let mut p = progress();
            p.proof.history = RecoveryHistory {
                relocation_confirmed: history & 1 != 0,
                restoration_confirmed: history & 2 != 0,
            };
            p.proof.current = complete(Command::Recover).2.proof.current;
            p.clear_current();
            let d = Detail {
                placement: p.proof,
                ..preflight_detail(Reason::Unverifiable, Stage::Layer)
            };
            let r = result(
                Command::Recover,
                Status::Partial,
                d,
                Confirmation::default(),
                p,
            )
            .unwrap();
            assert_eq!(r.detail().progress, Progress::default());
            assert_eq!(
                r.detail().placement.current,
                PlacementConfirmation::default()
            );
            assert_eq!(Detail::decode(d.encode()), Ok(d));
            assert!(result(
                Command::Recover,
                Status::Failed,
                d,
                Confirmation::default(),
                p
            )
            .is_err());
        }
    }

    #[test]
    fn recover_noop_geometry_dispatch_and_phase_completion_alone_never_authorize_partial() {
        for case in 0..4 {
            let (mut d, c, mut p) = complete(Command::Recover);
            p.baseline.as_mut().unwrap().desktop = p.destination_desktop;
            p.baseline.as_mut().unwrap().minimized = false;
            if case == 1 {
                d.stage = Stage::Geometry;
            }
            if case == 2 {
                d.progress.host_mutation_dispatched = true;
            }
            if case == 3 {
                d.stage = Stage::Layer;
            }
            d.reason = Reason::ConfirmationTimeout;
            assert_eq!(
                result(Command::Recover, Status::Partial, d, c, p),
                Err(ResultError::InvalidPartial)
            );
        }
    }

    #[test]
    fn historical_proof_requires_real_baseline_transitions_and_here_never_restores() {
        for command in [Command::BringHere, Command::Recover] {
            let mut p = progress();
            p.proof.history.relocation_confirmed = true;
            let mut d = Detail {
                placement: p.proof,
                ..preflight_detail(Reason::LayoutChanged, Stage::Placement)
            };
            assert!(result(command, Status::Partial, d, Confirmation::default(), p).is_ok());
            p.baseline.as_mut().unwrap().desktop = p.destination_desktop;
            assert!(result(command, Status::Partial, d, Confirmation::default(), p).is_err());
            p = progress();
            p.proof.history.restoration_confirmed = true;
            d.placement = p.proof;
            assert_eq!(
                result(command, Status::Partial, d, Confirmation::default(), p).is_ok(),
                command == Command::Recover
            );
            p.baseline.as_mut().unwrap().minimized = false;
            assert!(result(command, Status::Partial, d, Confirmation::default(), p).is_err());
        }
    }

    #[test]
    fn stale_history_cannot_fabricate_current_destination_or_input_flags() {
        let mut p = progress();
        p.proof.history.relocation_confirmed = true;
        let d = Detail {
            placement: p.proof,
            ..preflight_detail(Reason::Unverifiable, Stage::Layer)
        };
        let mut bad = d;
        bad.progress.input_ready = true;
        assert!(result(
            Command::Recover,
            Status::Partial,
            bad,
            Confirmation::default(),
            p
        )
        .is_err());
        bad = d;
        bad.placement.current.destination = true;
        assert!(result(
            Command::Recover,
            Status::Partial,
            bad,
            Confirmation::default(),
            p
        )
        .is_err());
        p.proof.current.destination = true;
        assert!(result(
            Command::Recover,
            Status::Partial,
            bad,
            Confirmation::default(),
            p
        )
        .is_err());
        assert_eq!(
            Detail::decode(bad.encode()),
            Err(DecodeError::ContradictoryProof)
        );
    }

    #[test]
    fn contradictory_wire_proofs_are_rejected_on_both_terminal_encode_and_decode() {
        let (d, _, _) = minimized_here(GeometryProof::Deferred);
        for field in 0..6 {
            let mut bad = d;
            match field {
                0 => bad.progress.input_ready = true,
                1 => bad.progress.placement_deferred = false,
                2 => bad.placement.current.destination = false,
                3 => bad.placement.current.geometry = GeometryProof::Verified,
                4 => bad.placement.history.restoration_confirmed = true,
                5 => bad.layer = LayerObservation::Unknown,
                _ => unreachable!(),
            }
            assert_eq!(
                Detail::decode(bad.encode()),
                Err(DecodeError::ContradictoryProof)
            );
            assert_eq!(terminal(bad).encode(), Err(DecodeError::ContradictoryProof));
            let mut words = terminal(d).encode().unwrap();
            words[4] = bad.encode();
            assert_eq!(
                TerminalResponse::decode(&words),
                Err(DecodeError::ContradictoryProof)
            );
        }
        let (d, _, _) = complete(Command::BringHere);
        for field in 0..5 {
            let mut bad = d;
            match field {
                0 => bad.placement.current.geometry = GeometryProof::Unknown,
                1 => bad.placement.current = PlacementConfirmation::default(),
                2 => bad.progress.above_confirmed = true,
                3 => bad.progress.not_minimized_confirmed = false,
                4 => bad.visibility = Visibility::Unknown,
                _ => unreachable!(),
            }
            // Retain historical proof so old current-readiness bits cannot
            // acquire new placement meaning merely through stale history.
            bad.placement.history.relocation_confirmed = true;
            assert_eq!(
                Detail::decode(bad.encode()),
                Err(DecodeError::ContradictoryProof)
            );
            assert_eq!(terminal(bad).encode(), Err(DecodeError::ContradictoryProof));
        }
    }

    #[test]
    fn placement_status_reason_stage_cross_products_reject_inconsistent_lifecycle_and_progress() {
        for command in [Command::BringHere, Command::Recover] {
            for status in [
                Status::Success,
                Status::Failed,
                Status::Partial,
                Status::Busy,
                Status::Starting,
                Status::Closing,
                Status::ProtocolError,
            ] {
                for reason in 0..=17 {
                    let reason = Reason::try_from(reason).unwrap();
                    for stage in [
                        Stage::Preflight,
                        Stage::Hide,
                        Stage::Show,
                        Stage::Geometry,
                        Stage::Layer,
                        Stage::Shutdown,
                        Stage::Placement,
                    ] {
                        let d = preflight_detail(reason, stage);
                        let valid =
                            result(command, status, d, Confirmation::default(), progress()).is_ok();
                        let expected = match status {
                            Status::Success | Status::Partial => false,
                            Status::Busy => reason == Reason::Busy && stage == Stage::Preflight,
                            Status::Starting => {
                                reason == Reason::Starting && stage == Stage::Preflight
                            }
                            Status::Closing => {
                                reason == Reason::Closing && stage == Stage::Shutdown
                            }
                            Status::ProtocolError => {
                                matches!(
                                    reason,
                                    Reason::MalformedRequest
                                        | Reason::WrongVersion
                                        | Reason::UnknownCommand
                                        | Reason::StaleInstance
                                ) && stage == Stage::Preflight
                            }
                            Status::Failed => {
                                matches!(
                                    reason,
                                    Reason::Closing
                                        | Reason::OwnershipLost
                                        | Reason::BodyDestroyed
                                        | Reason::Withdrawn
                                        | Reason::Unverifiable
                                        | Reason::Unsupported
                                        | Reason::ConfirmationTimeout
                                        | Reason::LayoutChanged
                                        | Reason::LayerChanged
                                        | Reason::ReleaseFailed
                                        | Reason::X11Failure
                                ) && stage != Stage::Hide
                                    && (command == Command::Recover
                                        || !matches!(stage, Stage::Show | Stage::Layer))
                            }
                        };
                        assert_eq!(
                            valid, expected,
                            "{command:?} {status:?} {reason:?} {stage:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn old_commands_reject_new_proofs_without_weakening_their_accepted_result_rules() {
        for command in [Command::Hide, Command::Show, Command::BringTop] {
            let d = Detail {
                placement: PlacementProof {
                    history: RecoveryHistory {
                        relocation_confirmed: true,
                        restoration_confirmed: false,
                    },
                    ..PlacementProof::default()
                },
                ..preflight_detail(Reason::Unsupported, Stage::Preflight)
            };
            assert_eq!(
                OperationResult::new(command, Status::Failed, d, Confirmation::default()),
                Err(ResultError::InconsistentConfirmation)
            );
        }
        for command in [Command::BringHere, Command::Recover] {
            let (d, c, _) = complete(command);
            assert!(OperationResult::new(command, Status::Success, d, c).is_err());
        }
    }
    #[test]
    fn here_success_never_silently_undoes_genuine_minimization() {
        let (d, c, mut p) = complete(Command::BringHere);
        p.baseline.as_mut().unwrap().minimized = true;
        assert!(result(Command::BringHere, Status::Success, d, c, p).is_err());
    }
    #[test]
    fn placement_proof_cross_product_keeps_current_readiness_and_historical_partial_independent() {
        for command in [Command::BringHere, Command::Recover] {
            for minimized in [false, true] {
                for geometry in [
                    GeometryProof::Unknown,
                    GeometryProof::Deferred,
                    GeometryProof::Verified,
                ] {
                    // Every combination of destination, preserved layer/minimization,
                    // historical relocation/restoration and current input readiness.
                    for bits in 0..64 {
                        let destination = bits & 1 != 0;
                        let layer = bits & 2 != 0;
                        let preserved_min = bits & 4 != 0;
                        let relocated = bits & 8 != 0;
                        let restored = bits & 16 != 0;
                        let input = bits & 32 != 0;
                        let above = command == Command::Recover && !minimized;
                        let mut p = progress();
                        p.baseline.as_mut().unwrap().minimized =
                            minimized || command == Command::Recover;
                        p.baseline.as_mut().unwrap().layer = if command == Command::Recover {
                            LayerObservation::Above
                        } else {
                            LayerObservation::Normal
                        };
                        p.proof = PlacementProof {
                            current: PlacementConfirmation {
                                destination,
                                geometry,
                                minimization_preserved: preserved_min,
                                layer_preserved: layer,
                            },
                            history: RecoveryHistory {
                                relocation_confirmed: relocated,
                                restoration_confirmed: restored,
                            },
                        };
                        let c = Confirmation {
                            visibility: Some(if minimized {
                                ConfirmedVisibility::Minimized
                            } else {
                                ConfirmedVisibility::NotMinimizedHere
                            }),
                            above,
                        };
                        let d = Detail {
                            visibility: if minimized {
                                Visibility::Minimized
                            } else {
                                Visibility::NotMinimizedHere
                            },
                            workspace: Workspace::Current,
                            layer: p.baseline.unwrap().layer,
                            progress: Progress {
                                not_minimized_confirmed: !minimized,
                                above_confirmed: above,
                                input_ready: input,
                                placement_deferred: geometry == GeometryProof::Deferred,
                                host_mutation_dispatched: false,
                            },
                            placement: p.proof,
                            ..preflight_detail(
                                Reason::None,
                                if command == Command::Recover {
                                    Stage::Layer
                                } else {
                                    Stage::Placement
                                },
                            )
                        };
                        let consistent = (!preserved_min || (minimized && !restored))
                            && (geometry == GeometryProof::Unknown || destination)
                            && (geometry != GeometryProof::Deferred || preserved_min)
                            && (!input
                                || (!minimized
                                    && destination
                                    && geometry == GeometryProof::Verified))
                            && (command != Command::BringHere || !restored);
                        let complete = destination
                            && if command == Command::Recover {
                                !minimized && geometry == GeometryProof::Verified && input
                            } else {
                                layer
                                    && if minimized {
                                        preserved_min
                                            && geometry != GeometryProof::Unknown
                                            && !input
                                    } else {
                                        geometry == GeometryProof::Verified && input
                                    }
                            };
                        let meaningful = relocated || (command == Command::Recover && restored);
                        for status in [Status::Success, Status::Partial, Status::Failed] {
                            let detail = Detail {
                                reason: if status == Status::Success {
                                    Reason::None
                                } else {
                                    Reason::ConfirmationTimeout
                                },
                                ..d
                            };
                            let expected = consistent
                                && match status {
                                    Status::Success => complete,
                                    Status::Partial => meaningful,
                                    Status::Failed => !meaningful,
                                    _ => unreachable!(),
                                };
                            assert_eq!(
                                result(command, status, detail, c, p).is_ok(),
                                expected,
                                "{command:?} {status:?} min={minimized} {geometry:?} bits={bits}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn placement_proofs_need_an_ordinary_known_baseline_but_unsupported_needs_none() {
        for command in [Command::BringHere, Command::Recover] {
            for field in 0..3 {
                let (d, c, mut p) = complete(command);
                match field {
                    0 => p.baseline = None,
                    1 => p.baseline.as_mut().unwrap().desktop = NO_DESTINATION,
                    2 => p.baseline.as_mut().unwrap().layer = LayerObservation::Unknown,
                    _ => unreachable!(),
                }
                assert!(result(command, Status::Success, d, c, p).is_err());
            }
            let mut p = progress();
            p.baseline = None;
            assert!(result(
                command,
                Status::Failed,
                preflight_detail(Reason::Unsupported, Stage::Preflight),
                Confirmation::default(),
                p
            )
            .is_ok());
        }
    }
}

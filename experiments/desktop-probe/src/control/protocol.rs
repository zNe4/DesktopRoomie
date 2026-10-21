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

    pub(super) fn validate_placement_wire(self) -> Result<(), DecodeError> {
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

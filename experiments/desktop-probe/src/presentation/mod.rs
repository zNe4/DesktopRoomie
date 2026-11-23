//! Pure frame production and publication validation for a single client target.
//!
//! Geometry is in client-local pixels, with the origin at the top left and Y
//! increasing downward. The host owns timing, lifecycle and input policy.
//! Validation establishes common coordinate limits. Native target limits and
//! the correspondence between coverage and rendered content are host checks.

pub mod procedural;
mod region;

use std::time::Duration;

pub use crate::geometry::{Point, Rect, Size};
pub use region::{HitRegion, RowSpan};

/// Nonempty client dimensions whose exclusive edges fit in signed coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientExtent(Size);

impl ClientExtent {
    pub fn new(size: Size) -> Result<Self, ValidationError> {
        if size.width == 0 || size.height == 0 {
            return Err(ValidationError::EmptyExtent);
        }
        if i32::try_from(size.width).is_err() || i32::try_from(size.height).is_err() {
            return Err(ValidationError::CoordinateOverflow);
        }
        Ok(Self(size))
    }

    pub fn size(self) -> Size {
        self.0
    }

    /// Whether `point` identifies a pixel inside this extent.
    pub fn contains(self, point: Point) -> bool {
        point.x >= 0 && point.x < self.right() && point.y >= 0 && point.y < self.bottom()
    }

    fn right(self) -> i32 {
        self.0.width as i32
    }

    fn bottom(self) -> i32 {
        self.0.height as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Idle,
    Walk,
}

/// Appearance relative to the viewer. Front faces toward the viewer, back
/// faces away, and left and right use the client's horizontal directions.
/// Each producer maps these intents to its own poses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    /// No directional pose requested.
    Neutral,
    FrontRight,
    FrontLeft,
    BackRight,
    BackLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionIntent {
    pub action: Action,
    pub facing: Facing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationCapabilities {
    initial_extent: ClientExtent,
    supported_intents: Vec<ActionIntent>,
    frame_period: Option<Duration>,
}

impl PresentationCapabilities {
    /// A supplied frame period must be positive. `None` requests no recurring
    /// animation frames. The host decides when to issue frame requests.
    pub fn new(
        initial_extent: ClientExtent,
        supported_intents: Vec<ActionIntent>,
        frame_period: Option<Duration>,
    ) -> Result<Self, ValidationError> {
        if frame_period == Some(Duration::ZERO) {
            return Err(ValidationError::ZeroFramePeriod);
        }
        Ok(Self {
            initial_extent,
            supported_intents,
            frame_period,
        })
    }

    pub fn initial_extent(&self) -> ClientExtent {
        self.initial_extent
    }

    pub fn supported_intents(&self) -> &[ActionIntent] {
        &self.supported_intents
    }

    pub fn frame_period(&self) -> Option<Duration> {
        self.frame_period
    }

    /// Tests the exact pair, including `Neutral` when requested.
    pub fn supports(&self, intent: ActionIntent) -> bool {
        self.supported_intents.contains(&intent)
    }
}

/// Host-issued identity scoped to one target lifetime. Sequences strictly
/// increase within a generation. Ordinary root-position dragging preserves
/// the generation while the client-local presentation remains valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameIdentity {
    pub generation: u64,
    pub sequence: u64,
}

impl FrameIdentity {
    /// Returns `None` on exhaustion. The host must stop issuing frames for
    /// this generation. A new sequence requires a fresh target generation.
    pub fn checked_next(self) -> Option<Self> {
        Some(Self {
            sequence: self.sequence.checked_add(1)?,
            ..self
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRequest {
    identity: FrameIdentity,
    intent: ActionIntent,
    extent: ClientExtent,
    active_elapsed: Duration,
    active_delta: Duration,
}

impl FrameRequest {
    /// The host checks intent support and supplies the elapsed active time and
    /// the already capped active delta. A redraw can use a zero delta.
    pub fn new(
        identity: FrameIdentity,
        intent: ActionIntent,
        extent: ClientExtent,
        active_elapsed: Duration,
        active_delta: Duration,
    ) -> Result<Self, ValidationError> {
        if active_delta > active_elapsed {
            return Err(ValidationError::DeltaExceedsElapsed);
        }
        Ok(Self {
            identity,
            intent,
            extent,
            active_elapsed,
            active_delta,
        })
    }

    pub fn identity(&self) -> FrameIdentity {
        self.identity
    }

    pub fn intent(&self) -> ActionIntent {
        self.intent
    }

    pub fn extent(&self) -> ClientExtent {
        self.extent
    }

    pub fn active_elapsed(&self) -> Duration {
        self.active_elapsed
    }

    pub fn active_delta(&self) -> Duration {
        self.active_delta
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameMetadata {
    extent: ClientExtent,
    visible_bounds: Option<Rect>,
    anchor: Option<Point>,
    hit_region: HitRegion,
}

impl FrameMetadata {
    /// Supplied visible bounds enclose the visible content and hit coverage.
    /// `None` omits the bounds. A supplied anchor identifies a client pixel
    /// and may sit outside the visible bounds.
    pub fn new(
        extent: ClientExtent,
        visible_bounds: Option<Rect>,
        anchor: Option<Point>,
        hit_region: HitRegion,
    ) -> Result<Self, ValidationError> {
        if hit_region.extent() != extent {
            return Err(ValidationError::RegionExtentMismatch);
        }
        if anchor.is_some_and(|point| !extent.contains(point)) {
            return Err(ValidationError::AnchorOutOfBounds);
        }
        if let Some(bounds) = visible_bounds {
            let right = bounds
                .checked_right()
                .ok_or(ValidationError::CoordinateOverflow)?;
            let bottom = bounds
                .checked_bottom()
                .ok_or(ValidationError::CoordinateOverflow)?;
            if !extent.contains(bounds.origin())
                || bounds.width == 0
                || bounds.height == 0
                || right > extent.right()
                || bottom > extent.bottom()
            {
                return Err(ValidationError::InvalidVisibleBounds);
            }
            if hit_region.spans().iter().any(|span| {
                span.y < bounds.y
                    || span.y >= bottom
                    || span.start_x < bounds.x
                    || span.end_x > right
            }) {
                return Err(ValidationError::CoverageOutsideVisibleBounds);
            }
        }
        Ok(Self {
            extent,
            visible_bounds,
            anchor,
            hit_region,
        })
    }

    pub fn extent(&self) -> ClientExtent {
        self.extent
    }

    pub fn visible_bounds(&self) -> Option<Rect> {
        self.visible_bounds
    }

    pub fn anchor(&self) -> Option<Point> {
        self.anchor
    }

    pub fn hit_region(&self) -> &HitRegion {
        &self.hit_region
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    EmptyExtent,
    CoordinateOverflow,
    ZeroFramePeriod,
    DeltaExceedsElapsed,
    EmptySpan,
    SpanOutOfBounds,
    UnsortedSpans,
    OverlappingSpans,
    RegionExtentMismatch,
    AnchorOutOfBounds,
    InvalidVisibleBounds,
    CoverageOutsideVisibleBounds,
}

/// Validated geometry and backend-specific data prepared for one request.
#[derive(Debug)]
pub struct PreparedFrame<P> {
    pub identity: FrameIdentity,
    pub metadata: FrameMetadata,
    pub payload: P,
}

/// Produces frame descriptions for the host. The associated types let a
/// producer return its own payload and error independently of the mutable
/// borrow used to prepare the frame.
pub trait FrameProducer {
    type Payload;
    type Error;

    fn capabilities(&self) -> &PresentationCapabilities;

    /// The caller checks the requested intent against the producer's
    /// capabilities. The returned frame uses the request's identity and extent.
    fn prepare_frame(
        &mut self,
        request: &FrameRequest,
    ) -> Result<PreparedFrame<Self::Payload>, Self::Error>;
}

/// What is known about visible effects of a publication failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationFailure {
    BeforeVisibleMutation,
    PossiblyVisible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationState {
    Prepared,
    Submitted,
    TransportCompleted,
    HostCommitted,
    Stale,
    Failed(PublicationFailure),
}

impl PublicationState {
    fn is_unsettled(self) -> bool {
        matches!(self, Self::Submitted | Self::TransportCompleted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationEvent {
    /// The host enters the publication attempt, before it can affect the target.
    Submit,
    /// The selected transport reported completion within its own guarantees.
    TransportComplete,
    /// The host completed its publication checks and accepted the frame.
    Commit,
    /// Ends the attempt. For a submitted frame, the host has established that
    /// native work is settled. Its visible effects may still be uncertain.
    Fail(PublicationFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationError {
    Stale,
    IdentityMismatch,
    ExtentMismatch,
    UnsettledCandidate,
    InvalidTransition,
    GenerationExhausted,
}

/// Validates the host's publication decisions for one target. Tracks the
/// admitted candidate, the last committed identity and visibility uncertainty.
/// This carries no payloads.
/// The host still owns input safety and recovery. Transport completion and
/// host commitment convey no compositor or scanout acknowledgment and make
/// no guarantee of atomic visual and input-region updates.
#[derive(Debug)]
pub struct Publication {
    generation: Option<u64>,
    candidate: Option<(FrameIdentity, PublicationState)>,
    last_committed: Option<FrameIdentity>,
    possibly_visible_failure: bool,
}

impl Publication {
    pub fn new(generation: u64) -> Self {
        Self {
            generation: Some(generation),
            candidate: None,
            last_committed: None,
            possibly_visible_failure: false,
        }
    }

    /// Historical identity of the last successful host commitment.
    pub fn last_committed(&self) -> Option<FrameIdentity> {
        self.last_committed
    }

    /// Committed identity eligible for use in this generation when publication
    /// is settled and free of unresolved visibility uncertainty.
    pub fn current_committed(&self) -> Option<FrameIdentity> {
        self.last_committed.filter(|identity| {
            self.generation == Some(identity.generation) && !self.visibility_uncertain()
        })
    }

    /// Whether an unsettled publication or a possibly visible failure leaves
    /// the target's content uncertain. A successful commit resolves that
    /// failure uncertainty.
    pub fn visibility_uncertain(&self) -> bool {
        self.unsettled_candidate().is_some() || self.possibly_visible_failure
    }

    /// Invalidate frame authority when the lifecycle or presentation context
    /// changes. The host still has to settle any outstanding native work.
    /// Generation exhaustion closes admission and commitment permanently and
    /// returns `GenerationExhausted`.
    pub fn invalidate(&mut self) -> Result<u64, PublicationError> {
        self.generation = self
            .generation
            .and_then(|generation| generation.checked_add(1));
        self.generation.ok_or(PublicationError::GenerationExhausted)
    }

    /// Admits a host-selected candidate whose identity and extent match the
    /// original request. An unsubmitted candidate can be superseded. A
    /// submitted candidate requires a terminal transition first. Failed and
    /// committed frames still consume sequence numbers in their generation.
    /// Rejected candidates leave the tracker unchanged.
    pub fn prepare<P>(
        &mut self,
        request: &FrameRequest,
        frame: &PreparedFrame<P>,
    ) -> Result<(), PublicationError> {
        let identity = request.identity();
        if frame.identity != identity {
            return Err(PublicationError::IdentityMismatch);
        }
        if frame.metadata.extent() != request.extent() {
            return Err(PublicationError::ExtentMismatch);
        }
        self.check_generation(identity)?;
        if self.candidate.is_some_and(|(candidate, _)| {
            identity.generation == candidate.generation && identity.sequence <= candidate.sequence
        }) {
            return Err(PublicationError::Stale);
        }
        if self.unsettled_candidate().is_some() {
            return Err(PublicationError::UnsettledCandidate);
        }
        self.candidate = Some((identity, PublicationState::Prepared));
        Ok(())
    }

    /// Reports the admitted candidate's state in the current generation.
    /// An obsolete identity returns `Stale`. A future or unadmitted identity
    /// returns an error. Use
    /// `current_committed` for the authoritative committed identity.
    pub fn state(&self, identity: FrameIdentity) -> Result<PublicationState, PublicationError> {
        match self.check_generation(identity) {
            Err(PublicationError::Stale) => return Ok(PublicationState::Stale),
            result => result?,
        }
        match self.candidate {
            Some((candidate, state)) if identity == candidate => Ok(state),
            Some((candidate, _))
                if identity.generation == candidate.generation
                    && identity.sequence < candidate.sequence =>
            {
                Ok(PublicationState::Stale)
            }
            _ => Err(PublicationError::IdentityMismatch),
        }
    }

    /// Records a transition for the matching candidate. After invalidation,
    /// only `Fail` for the exact unsettled candidate can retire it, once the
    /// host has settled its native work. Stale completion and commit events
    /// are rejected.
    pub fn advance(
        &mut self,
        identity: FrameIdentity,
        event: PublicationEvent,
    ) -> Result<(), PublicationError> {
        use PublicationEvent as Event;
        use PublicationState as State;

        let next = match (self.state(identity)?, event) {
            (State::Stale, Event::Fail(failure))
                if self.unsettled_candidate() == Some(identity) =>
            {
                State::Failed(failure)
            }
            (State::Stale, _) => return Err(PublicationError::Stale),
            (State::Prepared, Event::Submit) => State::Submitted,
            (State::Submitted, Event::TransportComplete) => State::TransportCompleted,
            (State::TransportCompleted, Event::Commit) => State::HostCommitted,
            (State::Prepared, Event::Fail(PublicationFailure::BeforeVisibleMutation)) => {
                State::Failed(PublicationFailure::BeforeVisibleMutation)
            }
            (State::Submitted | State::TransportCompleted, Event::Fail(failure)) => {
                State::Failed(failure)
            }
            _ => return Err(PublicationError::InvalidTransition),
        };
        match next {
            State::HostCommitted => {
                self.last_committed = Some(identity);
                self.possibly_visible_failure = false;
            }
            State::Failed(PublicationFailure::PossiblyVisible) => {
                self.possibly_visible_failure = true;
            }
            _ => {}
        }
        self.candidate = Some((identity, next));
        Ok(())
    }

    fn unsettled_candidate(&self) -> Option<FrameIdentity> {
        self.candidate
            .filter(|(_, state)| state.is_unsettled())
            .map(|(identity, _)| identity)
    }

    fn check_generation(&self, identity: FrameIdentity) -> Result<(), PublicationError> {
        match self.generation {
            Some(generation) if identity.generation == generation => Ok(()),
            Some(generation) if identity.generation > generation => {
                Err(PublicationError::IdentityMismatch)
            }
            _ => Err(PublicationError::Stale),
        }
    }
}

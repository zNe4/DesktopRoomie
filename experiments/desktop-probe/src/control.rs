//! Pure control protocol and operation evidence. No X11 resources or operations.
#![allow(dead_code)]

use std::time::{Duration, Instant};

use crate::geometry::{Point, Rect, Size, ValidOriginBounds};

mod protocol;

#[allow(unused_imports)]
pub use protocol::{
    selection_name, CallerMarker, Command, DecodeError, Detail, ExitCode, GeometryProof,
    LayerObservation, Lifecycle, OwnerDescriptor, PlacementConfirmation, PlacementProof, Progress,
    Reason, RecoveryHistory, ReplyCorrelation, Request, Stage, Status, TerminalResponse,
    Visibility, Workspace, CALLER, INSTANCE, MAGIC, NO_DESTINATION, REPLY, REQUEST, TIMESTAMP,
    VERSION,
};

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
    // Recover composition is pure and unwired; standalone Here is accepted.
    here_phase: HerePhase,
    layout: Option<DestinationIdentity>,
    observe: bool,
    invalidated: Option<Reason>,
    desktop_attempted: bool,
    desktop_dispatched: bool,
    desktop_checked: bool,
    /// Original membership is needed for send eligibility, not a wire proof.
    original_confirmed: bool,
    /// Fresh local non-minimized evidence gates correction; destination proof
    /// alone can also describe a minimized or transitional body.
    local_geometry_eligible: bool,
    /// Some means consumed, including failure before a native cookie exists.
    correction: Option<(Point, Instant)>,
}

/// Progression/eligibility only: no phase proves any native postcondition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HerePhase {
    Observe,
    Fenced,
    DesktopAttempted,
    AwaitDestination,
    Geometry,
    EnablingInput,
    InputRestored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationViewport {
    Absent,
    AllZero,
}

/// Projection of accepted Stage-B equality facts, not raw property replies.
/// D2 supplies validated values; monitor ordering/primary/outputs are excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DestinationIdentity {
    pub desktop_count: u32,
    pub destination_desktop: u32,
    pub current_desktop: u32,
    pub monitor_name: u32,
    pub monitor_bounds: Rect,
    pub target_workarea: Option<Rect>,
    pub viewport: DestinationViewport,
    pub usable_area: Rect,
    pub body_size: Size,
    pub valid_origins: ValidOriginBounds,
}

/// Fresh host evidence, never PropertyNotify payloads. Membership/current must
/// independently pass supported, coherent repeated reads, even if visibility is
/// transitional. None means membership/current could not be established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HereObservation {
    pub membership: Option<(u32, u32)>, // (body desktop, current desktop)
    pub visibility: Visibility,
    pub layer: LayerObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HereDecision {
    Original,
    Destination,
    Pending,
    Invalid(Reason),
}

impl PlacementProgress {
    // Read-only access prevents consumers from rearming attempts via a phase
    // reset or taking/clearing correction facts after geometry confirmation.
    pub fn here_phase(&self) -> HerePhase {
        self.here_phase
    }

    pub fn layout(&self) -> Option<DestinationIdentity> {
        self.layout
    }

    pub fn observation_due(&self) -> bool {
        self.observe
    }

    pub fn invalidation_reason(&self) -> Option<Reason> {
        self.invalidated
    }

    pub fn correction(&self) -> Option<(Point, Instant)> {
        self.correction
    }

    pub fn new(destination_desktop: u32) -> Self {
        Self {
            destination_desktop,
            baseline: None,
            proof: PlacementProof::default(),
            here_phase: HerePhase::Observe,
            layout: None,
            observe: true,
            invalidated: None,
            desktop_attempted: false,
            desktop_dispatched: false,
            desktop_checked: false,
            original_confirmed: false,
            local_geometry_eligible: false,
            correction: None,
        }
    }

    pub fn clear_current(&mut self) {
        self.proof.current = PlacementConfirmation::default();
        self.original_confirmed = false;
        self.local_geometry_eligible = false;
    }

    pub fn request_here_observation(&mut self) {
        self.observe = true;
        self.clear_current();
    }

    pub fn invalidate_here(&mut self, reason: Reason) -> HereDecision {
        let reason = *self.invalidated.get_or_insert(reason);
        self.clear_current();
        HereDecision::Invalid(reason)
    }

    /// Rebuild current proof; neither later evidence nor an error replaces the
    /// first admitted layout/baseline. History is established independently of
    /// layout/layer/minimization readiness, and survives their invalidation.
    pub fn observe_here(
        &mut self,
        layout: Result<DestinationIdentity, Reason>,
        observation: HereObservation,
    ) -> HereDecision {
        self.clear_current();
        self.observe = false;
        if let (Some(baseline), Some((body, current))) = (self.baseline, observation.membership) {
            if baseline.desktop != self.destination_desktop
                && self.desktop_attempted
                && self.desktop_dispatched
                && body == self.destination_desktop
                && current == self.destination_desktop
            {
                // Checked transport is neither necessary nor sufficient for this.
                self.proof.history.relocation_confirmed = true;
            }
        }
        if let Some(reason) = self.invalidated {
            return HereDecision::Invalid(reason);
        }
        let layout = match layout {
            Ok(layout) => layout,
            Err(reason) => return self.invalidate_here(reason),
        };
        if self.destination_desktop == NO_DESTINATION
            || self.destination_desktop >= layout.desktop_count
            || layout.destination_desktop != self.destination_desktop
            || layout.current_desktop != self.destination_desktop
            || self.layout.is_some_and(|admitted| admitted != layout)
        {
            return self.invalidate_here(Reason::LayoutChanged);
        }
        match observation.visibility {
            Visibility::Destroyed => return self.invalidate_here(Reason::BodyDestroyed),
            Visibility::Withdrawn => return self.invalidate_here(Reason::Withdrawn),
            _ => {}
        }
        let Some((body, current)) = observation.membership else {
            return if self.baseline.is_none() {
                self.invalidate_here(Reason::Unverifiable)
            } else {
                HereDecision::Pending
            };
        };
        if body == NO_DESTINATION {
            return self.invalidate_here(Reason::Unsupported);
        }
        if current != self.destination_desktop || body >= layout.desktop_count {
            return self.invalidate_here(Reason::LayoutChanged);
        }
        let minimized = match observation.visibility {
            Visibility::Minimized => Some(true),
            Visibility::NotMinimizedHere if body == current => Some(false),
            Visibility::NotMinimizedElsewhere if body != current => Some(false),
            _ => None,
        };
        let layer_known = matches!(
            observation.layer,
            LayerObservation::Normal | LayerObservation::Above | LayerObservation::Below
        );
        if self.baseline.is_none() {
            let Some(minimized) = minimized.filter(|_| layer_known) else {
                return self.invalidate_here(Reason::Unverifiable);
            };
            self.baseline = Some(PlacementBaseline {
                desktop: body,
                minimized,
                layer: observation.layer,
            });
            self.layout = Some(layout);
        }
        let baseline = self.baseline.unwrap();
        let destination = body == self.destination_desktop;
        if body != baseline.desktop && !destination
            || (destination && baseline.desktop != body && !self.desktop_dispatched)
            || (!destination && self.proof.history.relocation_confirmed)
        {
            return self.invalidate_here(Reason::LayoutChanged);
        }
        if layer_known && baseline.layer != observation.layer {
            return self.invalidate_here(Reason::LayerChanged);
        }
        if minimized.is_some_and(|minimized| minimized != baseline.minimized) {
            return self.invalidate_here(Reason::Unverifiable);
        }
        self.proof.current = PlacementConfirmation {
            destination,
            layer_preserved: layer_known,
            minimization_preserved: minimized == Some(true),
            geometry: GeometryProof::Unknown,
        };
        self.original_confirmed = !destination && minimized.is_some() && layer_known;
        self.local_geometry_eligible = destination && minimized == Some(false) && layer_known;
        if minimized.is_none() || !layer_known {
            HereDecision::Pending
        } else if destination {
            HereDecision::Destination
        } else {
            HereDecision::Original
        }
    }

    /// Called only after the host reports a fully settled checked empty fence.
    pub fn here_fenced(&mut self) -> bool {
        if self.here_phase != HerePhase::Observe
            || self.baseline.is_none()
            || self.invalidated.is_some()
        {
            return false;
        }
        self.here_phase = HerePhase::Fenced;
        self.request_here_observation();
        true
    }

    /// Called after cookie acquisition, before checking, then again on success.
    /// Consumed authority is independent of both possible dispatch and its check.
    pub fn record_here_desktop_transport(
        &mut self,
        possible_dispatch: bool,
        checked: bool,
    ) -> bool {
        if !self.desktop_attempted || (checked && !possible_dispatch) {
            return false;
        }
        self.desktop_dispatched |= possible_dispatch;
        self.desktop_checked |= checked;
        if checked && self.here_phase == HerePhase::DesktopAttempted {
            self.here_phase = HerePhase::AwaitDestination;
        }
        self.request_here_observation();
        true
    }

    pub fn begin_here_geometry(&mut self) -> bool {
        if !matches!(
            self.here_phase,
            HerePhase::Fenced | HerePhase::AwaitDestination
        ) || self.invalidated.is_some()
            || self.observe
            || !self.proof.current.destination
            || !self.proof.current.layer_preserved
            || !self.local_geometry_eligible
            || !self.baseline.is_some_and(|b| !b.minimized)
        {
            return false;
        }
        self.here_phase = HerePhase::Geometry;
        true
    }

    /// Fresh actual fixed-size root geometry, not ConfigureNotify coordinates.
    /// Also usable on the safe local no-op path without pretending to fence.
    pub fn confirm_here_geometry(&mut self, origin: Point, size: Size) -> bool {
        let valid = self.invalidated.is_none()
            && !self.observe
            && self.proof.current.destination
            && self.proof.current.layer_preserved
            && self.local_geometry_eligible
            && self.baseline.is_some_and(|b| !b.minimized)
            && self
                .layout
                .is_some_and(|l| l.body_size == size && l.valid_origins.contains(origin))
            && self.correction.is_none_or(|(target, _)| target == origin);
        self.proof.current.geometry = if valid {
            GeometryProof::Verified
        } else {
            GeometryProof::Unknown
        };
        valid
    }

    pub fn defer_here_geometry(&mut self) -> bool {
        if self.invalidated.is_some()
            || self.observe
            || self.here_phase == HerePhase::Observe
            || !self.proof.current.destination
            || !self.proof.current.layer_preserved
            || !self.proof.current.minimization_preserved
        {
            return false;
        }
        self.proof.current.geometry = GeometryProof::Deferred;
        true
    }

    /// Select the exact accepted partition, then use the unchanged validator.
    /// The caller supplies coherent fresh visibility/input evidence in detail
    /// and confirmation; phases, dispatch and correction never supply it.
    pub fn here_result(
        &self,
        mut detail: Detail,
        confirmed: Confirmation,
    ) -> Result<OperationResult, ResultError> {
        detail.placement = self.proof;
        detail.progress.host_mutation_dispatched |= self.desktop_dispatched;
        let success = Detail {
            reason: Reason::None,
            ..detail
        };
        if let Ok(result) = OperationResult::placement(
            Command::BringHere,
            Status::Success,
            success,
            confirmed,
            self,
        ) {
            return Ok(result);
        }
        OperationResult::placement(
            Command::BringHere,
            if self.proof.history.relocation_confirmed {
                Status::Partial
            } else {
                Status::Failed
            },
            detail,
            confirmed,
            self,
        )
    }
}

impl ActiveOperation {
    /// Derive Recover's Here partition from its immutable owner deadline,
    /// rather than retaining a second admission timestamp.
    pub fn here_cutoff(&self) -> Option<Instant> {
        match self.request.command {
            Command::BringHere => Some(self.deadline),
            Command::Recover => self.deadline.checked_sub(Duration::from_millis(3500)),
            _ => None,
        }
    }

    fn owns_here(&self) -> bool {
        self.request.command == Command::BringHere
            || (self.request.command == Command::Recover
                && self.recover_phase == RecoverPhase::Here)
    }

    pub fn here_mutation_eligible(&self, now: Instant) -> bool {
        self.owns_here()
            && self.here_cutoff().is_some_and(|cutoff| now < cutoff)
            && self.placement.invalidated.is_none()
            && !self.placement.observe
    }

    pub fn consume_here_desktop_attempt(&mut self, now: Instant) -> bool {
        let eligible = self.here_mutation_eligible(now);
        let p = &mut self.placement;
        if !eligible
            || p.here_phase != HerePhase::Fenced
            || p.desktop_attempted
            || !p.original_confirmed
            || !p
                .baseline
                .is_some_and(|b| b.desktop != p.destination_desktop)
        {
            return false;
        }
        p.desktop_attempted = true;
        p.here_phase = HerePhase::DesktopAttempted;
        p.request_here_observation();
        true
    }

    pub fn consume_here_correction(&mut self, target: Point, now: Instant) -> bool {
        // Reserve the entire future host confirmation interval before consuming.
        let Some(deadline) = now.checked_add(Duration::from_millis(150)) else {
            return false;
        };
        let eligible = self.here_mutation_eligible(now)
            && self.here_cutoff().is_some_and(|cutoff| deadline <= cutoff);
        let p = &mut self.placement;
        if !eligible
            || p.here_phase != HerePhase::Geometry
            || p.correction.is_some()
            || !p.proof.current.destination
            || !p.proof.current.layer_preserved
            || !p.local_geometry_eligible
            || !p.layout.is_some_and(|l| l.valid_origins.contains(target))
            || !p.baseline.is_some_and(|b| !b.minimized)
        {
            return false;
        }
        p.correction = Some((target, deadline));
        p.request_here_observation();
        true
    }

    pub fn consume_here_input_enable(&mut self, now: Instant) -> bool {
        if !self.here_mutation_eligible(now)
            || self.placement.here_phase != HerePhase::Geometry
            || !self.placement.proof.current.destination
            || !self.placement.proof.current.layer_preserved
            || self.placement.proof.current.geometry != GeometryProof::Verified
        {
            return false;
        }
        self.placement.here_phase = HerePhase::EnablingInput;
        true
    }

    /// Checked native restoration is historical bookkeeping, not input readiness
    /// proof. Final fresh observation and runtime safety still decide Success.
    pub fn here_input_restored(&mut self) -> bool {
        if !self.owns_here()
            || self.placement.here_phase != HerePhase::EnablingInput
            || self.placement.invalidated.is_some()
        {
            return false;
        }
        self.placement.here_phase = HerePhase::InputRestored;
        self.placement.request_here_observation();
        true
    }

    pub fn here_shutdown(&mut self, reason: Reason) -> Result<OperationResult, ResultError> {
        if self.request.command == Command::Recover {
            return self.recover_shutdown(reason);
        }
        self.placement.invalidate_here(reason);
        let dispatched = self.detail.progress.host_mutation_dispatched;
        self.stage = Stage::Shutdown;
        self.detail = preflight_detail(reason, Stage::Shutdown);
        self.detail.progress.host_mutation_dispatched = dispatched;
        self.detail.placement = self.placement.proof;
        self.placement
            .here_result(self.detail, Confirmation::default())
    }

    /// Recover Here uses the accepted preservation algorithm. The adapter must
    /// supply fresh coherent observations of the same owned body, as for Here.
    pub fn observe_recover_here(
        &mut self,
        layout: Result<DestinationIdentity, Reason>,
        observation: HereObservation,
    ) -> HereDecision {
        if self.request.command != Command::Recover || self.recover_phase != RecoverPhase::Here {
            return HereDecision::Invalid(Reason::Unverifiable);
        }
        self.record_recover_restoration(observation);
        let stage = if self.stage == Stage::Shutdown {
            Stage::Shutdown
        } else {
            Stage::Placement
        };
        self.clear_recover_current(self.detail.reason, stage);
        let decision = self.placement.observe_here(layout, observation);
        self.bring_top.minimized_at_start = self.placement.baseline.is_some_and(|b| b.minimized);
        if matches!(decision, HereDecision::Destination | HereDecision::Original) {
            self.detail.visibility = observation.visibility;
            self.detail.layer = observation.layer;
            self.detail.workspace = if decision == HereDecision::Destination {
                Workspace::Current
            } else {
                Workspace::Other
            };
            self.detail.progress.not_minimized_confirmed = matches!(
                observation.visibility,
                Visibility::NotMinimizedHere | Visibility::NotMinimizedElsewhere
            );
        }
        if let Some(reason) = self.placement.invalidated {
            self.detail.reason = reason;
        }
        self.detail.placement = self.placement.proof;
        decision
    }

    /// Historical visibility is independent of destination/layer readiness.
    /// Membership and visibility must agree; a dispatch or geometry read cannot
    /// establish restoration. Membership is independently supported by the host
    /// adapter; a changed layout must not hide a demonstrated visibility change.
    fn record_recover_restoration(&mut self, observation: HereObservation) {
        let coherent = observation.membership.is_some_and(|(body, current)| {
            body != NO_DESTINATION
                && current != NO_DESTINATION
                && matches!(
                    (observation.visibility, body == current),
                    (Visibility::NotMinimizedHere, true)
                        | (Visibility::NotMinimizedElsewhere, false)
                )
        });
        if self.placement.baseline.is_some_and(|b| b.minimized) && coherent {
            self.placement.proof.history.restoration_confirmed = true;
        }
        self.bring_top.minimized_at_start = self.placement.baseline.is_some_and(|b| b.minimized);
        self.bring_top.restoration_confirmed = self.placement.proof.history.restoration_confirmed;
    }

    /// The supplied detail/confirmation must come from final fresh Here
    /// verification, including the settled fence or safe visible no-op. Use the
    /// existing Here validator, without publishing its intermediate Success.
    pub fn handoff_recover_top(
        &mut self,
        now: Instant,
        mut detail: Detail,
        confirmed: Confirmation,
    ) -> bool {
        if self.request.command != Command::Recover
            || self.recover_phase != RecoverPhase::Here
            || !self.here_mutation_eligible(now)
            || self.placement.layout.is_none()
            || detail.reason != Reason::None
        {
            return false;
        }
        detail.placement = self.placement.proof;
        if OperationResult::placement(
            Command::BringHere,
            Status::Success,
            detail,
            confirmed,
            &self.placement,
        )
        .is_err()
        {
            return false;
        }
        self.recover_phase = RecoverPhase::Top;
        self.bring_top.visibility_cutoff = (now + Duration::from_millis(2500)).min(self.deadline);
        self.bring_top.minimized_at_start = self.placement.baseline.unwrap().minimized;
        self.bring_top.restoration_confirmed = self.placement.proof.history.restoration_confirmed;
        // Here never owns layer work. Do not reset any attempted/dispatched facts.
        self.show = ShowProgress {
            observe: true,
            correction_sent: self.placement.correction.is_some(),
            ..ShowProgress::default()
        };
        self.detail.progress.host_mutation_dispatched |= detail.progress.host_mutation_dispatched;
        self.clear_recover_current(Reason::None, Stage::Show);
        true
    }

    /// Clear *all* current assertions, preserving original identity, consumed
    /// attempts, historical transitions and aggregate possible dispatch.
    fn clear_recover_current(&mut self, reason: Reason, stage: Stage) {
        let dispatched = self.detail.progress.host_mutation_dispatched
            || self.placement.desktop_dispatched
            || self.bring_top.layer_dispatched;
        self.placement.request_here_observation();
        self.stage = stage;
        self.detail = preflight_detail(reason, stage);
        self.detail.progress.host_mutation_dispatched = dispatched;
        self.detail.placement = self.placement.proof;
    }

    /// Rebuild Top proof without Here's preservation requirements. No timestamp
    /// changes here: even expiry/fatal history reads grant no mutation authority.
    pub fn observe_recover_top(
        &mut self,
        layout: Result<DestinationIdentity, Reason>,
        observation: HereObservation,
    ) -> HereDecision {
        if self.request.command != Command::Recover || self.recover_phase != RecoverPhase::Top {
            return HereDecision::Invalid(Reason::Unverifiable);
        }
        self.record_recover_restoration(observation);
        let stage = self.stage;
        self.clear_recover_current(self.detail.reason, stage);
        let p = &mut self.placement;
        p.observe = false;
        // History can advance on a bounded fatal read, but current proof cannot.
        if let Some(reason) = p.invalidated {
            return HereDecision::Invalid(reason);
        }
        let reason = match layout {
            Err(reason) => Some(reason),
            Ok(layout) if p.layout != Some(layout) || p.baseline.is_none() => {
                Some(Reason::LayoutChanged)
            }
            Ok(_) => match observation.visibility {
                Visibility::Destroyed => Some(Reason::BodyDestroyed),
                Visibility::Withdrawn => Some(Reason::Withdrawn),
                _ if observation.layer == LayerObservation::Conflict => Some(Reason::LayerChanged),
                _ if observation.membership.is_some_and(|(body, current)| {
                    body != p.destination_desktop || current != p.destination_desktop
                }) =>
                {
                    Some(Reason::LayoutChanged)
                }
                _ => None,
            },
        };
        if let Some(reason) = reason {
            let decision = p.invalidate_here(reason);
            self.detail.reason = reason;
            self.detail.placement = p.proof;
            return decision;
        }
        let local = observation.visibility == Visibility::NotMinimizedHere;
        let coherent = observation.membership
            == Some((p.destination_desktop, p.destination_desktop))
            && matches!(
                observation.visibility,
                Visibility::Minimized | Visibility::NotMinimizedHere
            );
        let known_layer = matches!(
            observation.layer,
            LayerObservation::Normal | LayerObservation::Above | LayerObservation::Below
        );
        if !coherent || !known_layer {
            return HereDecision::Pending;
        }
        p.proof.current.destination = true;
        p.proof.current.layer_preserved = p.baseline.is_some_and(|b| b.layer == observation.layer);
        // Top intentionally permits restoration; never assert preserved HIDDEN.
        p.local_geometry_eligible = local;
        self.detail.visibility = observation.visibility;
        self.detail.layer = observation.layer;
        self.detail.workspace = Workspace::Current;
        self.detail.progress.not_minimized_confirmed = local;
        self.detail.progress.above_confirmed =
            local && observation.layer == LayerObservation::Above;
        self.detail.placement = p.proof;
        HereDecision::Destination
    }

    /// Fresh fixed-size root geometry and runtime-validated exact input safety.
    /// Neither a correction request nor SHAPE acknowledgement supplies these.
    pub fn confirm_recover_top_readiness(
        &mut self,
        origin: Point,
        size: Size,
        input_ready: bool,
    ) -> bool {
        if self.request.command != Command::Recover || self.recover_phase != RecoverPhase::Top {
            return false;
        }
        let p = &mut self.placement;
        let valid = p.invalidated.is_none()
            && !p.observe
            && p.local_geometry_eligible
            && p.proof.current.destination
            && p.layout
                .is_some_and(|l| l.body_size == size && l.valid_origins.contains(origin))
            // A consumed Here correction is history, not current Top geometry.
            // Only a pending Top correction imposes its confirmation target.
            && self.show.correction.is_none_or(|(target, _)| target == origin);
        p.proof.current.geometry = if valid {
            GeometryProof::Verified
        } else {
            GeometryProof::Unknown
        };
        self.detail.progress.input_ready = valid && input_ready;
        self.detail.placement = p.proof;
        valid
    }

    pub fn recover_visibility_mutation_eligible(&self, now: Instant) -> bool {
        self.request.command == Command::Recover
            && self.recover_phase == RecoverPhase::Top
            && self.placement.invalidated.is_none()
            && !self.placement.observe
            && self.placement.proof.current.destination
            && now < self.bring_top.visibility_cutoff
            && now < self.deadline
            && self.bring_top.layer_admitted.is_none()
    }

    /// PlacementProgress is the sole Recover-wide consumed-correction record.
    /// Show fields are only a projection for reuse of its existing mechanism.
    pub fn consume_recover_top_correction(&mut self, target: Point, now: Instant) -> bool {
        let Some(deadline) = now.checked_add(Duration::from_millis(150)) else {
            return false;
        };
        if !self.recover_visibility_mutation_eligible(now)
            || deadline > self.bring_top.visibility_cutoff
            || self.placement.correction.is_some()
            || !self.placement.local_geometry_eligible
            || !self
                .placement
                .layout
                .is_some_and(|l| l.valid_origins.contains(target))
        {
            return false;
        }
        self.placement.correction = Some((target, deadline));
        self.show.correction = Some((target, deadline));
        self.show.correction_sent = true;
        self.clear_recover_current(Reason::None, Stage::Geometry);
        true
    }

    pub fn admit_recover_layer(&mut self, now: Instant) -> bool {
        if !self.recover_visibility_mutation_eligible(now)
            || !self.detail.progress.not_minimized_confirmed
            || !self.detail.progress.input_ready
            || self.placement.proof.current.geometry != GeometryProof::Verified
        {
            return false;
        }
        self.bring_top.layer_admitted = Some(now);
        self.bring_top.layer_observe = true;
        self.stage = Stage::Layer;
        self.detail.stage = Stage::Layer;
        true
    }

    pub fn recover_layer_cutoff(&self) -> Option<Instant> {
        (self.request.command == Command::Recover && self.recover_phase == RecoverPhase::Top)
            .then_some(self.bring_top.layer_admitted)
            .flatten()
            .map(|admitted| (admitted + Duration::from_millis(1000)).min(self.deadline))
    }

    pub fn recover_layer_mutation_eligible(&self, now: Instant) -> bool {
        self.recover_layer_cutoff()
            .is_some_and(|cutoff| now < cutoff)
            && self.placement.invalidated.is_none()
            && !self.placement.observe
            && self.placement.proof.current.geometry == GeometryProof::Verified
            && self.detail.progress.input_ready
            && self.detail.progress.not_minimized_confirmed
            && !self.bring_top.layer_finished
    }

    /// Only the existing placement validator decides complete/partial/failed.
    pub fn recover_result(
        &self,
        reason: Reason,
        stage: Stage,
    ) -> Result<OperationResult, ResultError> {
        if self.request.command != Command::Recover {
            return Err(ResultError::InconsistentStatus);
        }
        let mut detail = self.detail;
        detail.reason = self.placement.invalidated.unwrap_or(reason);
        detail.stage = stage;
        detail.placement = self.placement.proof;
        detail.progress.placement_deferred =
            self.placement.proof.current.geometry == GeometryProof::Deferred;
        detail.progress.host_mutation_dispatched |=
            self.placement.desktop_dispatched || self.bring_top.layer_dispatched;
        let confirmed = Confirmation {
            visibility: match detail.visibility {
                Visibility::Minimized => Some(ConfirmedVisibility::Minimized),
                Visibility::NotMinimizedHere if detail.progress.not_minimized_confirmed => {
                    Some(ConfirmedVisibility::NotMinimizedHere)
                }
                Visibility::NotMinimizedElsewhere if detail.progress.not_minimized_confirmed => {
                    Some(ConfirmedVisibility::NotMinimizedElsewhere)
                }
                _ => None,
            },
            above: detail.progress.above_confirmed,
        };
        if let Ok(result) = OperationResult::placement(
            Command::Recover,
            Status::Success,
            detail,
            confirmed,
            &self.placement,
        ) {
            return Ok(result);
        }
        OperationResult::placement(
            Command::Recover,
            if self.placement.proof.history.relocation_confirmed
                || self.placement.proof.history.restoration_confirmed
            {
                Status::Partial
            } else {
                Status::Failed
            },
            detail,
            confirmed,
            &self.placement,
        )
    }

    pub fn recover_shutdown(&mut self, reason: Reason) -> Result<OperationResult, ResultError> {
        if self.request.command != Command::Recover {
            return Err(ResultError::InconsistentStatus);
        }
        self.placement.invalidate_here(reason);
        let reason = self.placement.invalidated.unwrap();
        self.clear_recover_current(reason, Stage::Shutdown);
        self.show.observe = false;
        self.bring_top.layer_observe = false;
        self.bring_top.layer_finished = false;
        self.recover_result(reason, Stage::Shutdown)
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
        placement: PlacementProgress::new(destination_desktop),
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
                if status == Status::Partial && (complete || !meaningful) {
                    return Err(ResultError::InvalidPartial);
                }
                if status == Status::Failed && (complete || meaningful) {
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
mod tests;

#[cfg(test)]
mod here_progression_tests;

#[cfg(test)]
mod placement_tests;

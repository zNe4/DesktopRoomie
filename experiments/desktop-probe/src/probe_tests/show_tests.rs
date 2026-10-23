use super::*;
use crate::control::{
    self, ActiveOperation, Capability, Command, Evidence, LayerObservation as L, Lifecycle,
    MapState, Reason, ShowPhase, Status, Visibility, VisibilityEvidence, WmState, Workspace,
};
use crate::geometry::{Rect, ValidOriginBounds};
use std::cell::RefCell;
use std::collections::VecDeque;

#[derive(Clone, Copy)]
struct Reading {
    evidence: VisibilityEvidence,
    layer: L,
    desktop: u32,
    current: u32,
}
fn reading(visibility: Visibility, layer: L, desktop: u32, current: u32) -> Reading {
    let workspace = if desktop == u32::MAX {
        Workspace::AllDesktops
    } else if desktop == current {
        Workspace::Current
    } else {
        Workspace::Other
    };
    let (wm_state, map_state, hidden) = match visibility {
        Visibility::Minimized => (WmState::Iconic, MapState::Unmapped, true),
        Visibility::NotMinimizedElsewhere => (WmState::Iconic, MapState::Unmapped, false),
        Visibility::Withdrawn => (WmState::Withdrawn, MapState::Unmapped, false),
        _ => (WmState::Normal, MapState::Viewable, false),
    };
    Reading {
        evidence: VisibilityEvidence {
            destroyed: visibility == Visibility::Destroyed,
            wm_state,
            map_state: Evidence::Readable(map_state),
            hidden: Evidence::Readable(hidden),
            workspace,
            capability: Capability::Supported,
            coherent: true,
        },
        layer,
        desktop,
        current,
    }
}
struct Fake {
    now: Instant,
    current: Reading,
    reads: VecDeque<Reading>,
    runtime: ProbeRuntime,
    window: ManagedProbeWindow,
    geometry: Rect,
    bounds: ValidOriginBounds,
    area: Rect,
    monitor: u32,
    log: Vec<&'static str>,
    fail: Option<&'static str>,
    advance: Option<(&'static str, Duration)>,
    refuse_correction: bool,
    layer_mode: u8, // 0 immediate, 1 unchanged, 2 removal only, 3 spontaneous Above
    layer_issue: Option<RemoteLayerIssue>,
    layer_support: LayerSupport,
    mutations: Vec<Mutation>,
    changes: VecDeque<(&'static str, Reading)>,
    step_failures: VecDeque<&'static str>,
    layer_reads: VecDeque<Reading>,
    support_reads: VecDeque<PropertyResult<LayerSupport>>,
    supported: Evidence<Vec<u32>>,
    fail_before_layer_dispatch: bool,
}
impl Fake {
    fn new(current: Reading) -> Self {
        let mut runtime = ProbeRuntime::new();
        runtime.body = BodyAvailability::Ready;
        runtime.arm_map_input_sync(500);
        Self {
            now: Instant::now(),
            current,
            reads: VecDeque::new(),
            runtime,
            window: tests::test_window(),
            geometry: Rect::new(100, 100, 160, 160),
            bounds: ValidOriginBounds::new(0, 500, 0, 500),
            area: Rect::new(0, 0, 660, 660),
            monitor: 20,
            log: vec![],
            fail: None,
            advance: None,
            refuse_correction: false,
            layer_mode: 0,
            layer_issue: None,
            layer_support: LayerSupport {
                state: true,
                above: true,
                below: true,
            },
            mutations: vec![],
            changes: VecDeque::new(),
            step_failures: VecDeque::new(),
            layer_reads: VecDeque::new(),
            support_reads: VecDeque::new(),
            supported: Evidence::Readable(vec![12, 13, 14, 15, 16, 17]),
            fail_before_layer_dispatch: false,
        }
    }
    fn step(&mut self, name: &'static str) -> Result<(), HostError> {
        self.log.push(name);
        if self.changes.front().is_some_and(|(step, _)| *step == name) {
            self.current = self.changes.pop_front().unwrap().1;
        }
        if self.step_failures.front() == Some(&name) {
            self.step_failures.pop_front();
            return Err(format!("injected {name}").into());
        }
        if let Some((step, duration)) = self.advance {
            if name == step {
                self.now += duration;
                self.advance = None;
            }
        }
        if self.fail == Some(name) {
            return Err(format!("injected {name}").into());
        }
        Ok(())
    }
    fn operation(&self) -> ActiveOperation {
        self.command_operation(Command::Show)
    }
    fn command_operation(&self, command: Command) -> ActiveOperation {
        let mut active = None;
        control::admit(
            &mut active,
            control::Request {
                command,
                request_id: 99,
                reply_xid: 88,
                epoch: 7,
            },
            crate::control::NO_DESTINATION,
            Lifecycle::Ready,
            false,
            self.now,
        );
        let mut operation = active.unwrap();
        operation.stage = if command == Command::Hide {
            control::Stage::Hide
        } else {
            control::Stage::Show
        };
        operation.detail = control::preflight_detail(Reason::None, operation.stage);
        operation
    }
    fn run(&mut self, operation: &mut ActiveOperation) -> Option<control::OperationResult> {
        service_show_with(operation, self).unwrap()
    }
    fn count(&self, step: &str) -> usize {
        self.log.iter().filter(|s| **s == step).count()
    }
}
impl VisibilityHost for Fake {
    fn layer_atoms(&self) -> LayerAtoms {
        fake_layer_atoms()
    }
    fn now(&self) -> Instant {
        self.now
    }
    fn observe(&mut self) -> Result<x11::visibility::VisibilitySnapshot, HostError> {
        self.step("observe")?;
        if let Some(read) = self.reads.pop_front() {
            self.current = read;
        }
        let r = self.current;
        Ok(x11::visibility::VisibilitySnapshot {
            evidence: r.evidence,
            layer: r.layer,
            candidate: control::classify(r.evidence),
            observation: Some(x11::visibility::Observation {
                wm_state: r.evidence.wm_state,
                map_state: r.evidence.map_state,
                state: match r.evidence.hidden {
                    Evidence::Readable(true) => Evidence::Readable(vec![13]),
                    Evidence::Readable(false) => Evidence::Readable(vec![]),
                    Evidence::Missing => Evidence::Missing,
                    Evidence::Malformed => Evidence::Malformed,
                },
                desktop: Evidence::Readable(r.desktop),
                current_desktop: Evidence::Readable(r.current),
                supported: self.supported.clone(),
            }),
        })
    }
    fn noop_ready(&self) -> bool {
        self.runtime.body == BodyAvailability::Ready
            && !self.runtime.input_fence.blocked
            && self.runtime.correction.is_none()
    }
    fn placement_busy(&self) -> bool {
        self.runtime.correction.is_some()
    }
    fn fenced(&self) -> bool {
        self.runtime.input_fence.blocked
    }
    fn fence_settled(&self) -> bool {
        self.runtime.visibility_dispatch_eligible()
    }
    fn deferred_placement(&mut self, p: &ShowPlacement) -> Result<bool, HostError> {
        if self.runtime.correction.is_some() {
            return Ok(false);
        }
        self.runtime.body = BodyAvailability::Validating;
        let mut target = None;
        self.runtime.placement_with(
            &mut self.window,
            p.input.geometry,
            &p.input.bounds,
            |_, point| {
                target = Some(point);
                Ok(())
            },
        )?;
        if let Some(target) = target {
            self.log.push("ordinary_correct");
            self.geometry.x = target.x;
            self.geometry.y = target.y;
        }
        Ok(self.runtime.body == BodyAvailability::Ready)
    }
    fn fence(&mut self) -> Result<(), HostError> {
        self.step("fence")?;
        let log = RefCell::new(vec![]);
        let fail = self.fail;
        let outcome = self.runtime.fence_visibility_input_with(
            &mut self.window,
            || {
                log.borrow_mut().push("empty");
                if fail == Some("empty") {
                    Err("injected empty".into())
                } else {
                    Ok(70000)
                }
            },
            |r, w| {
                log.borrow_mut().push("release");
                assert!(r.pending_move.is_none());
                assert!(w.in_flight_moves.is_empty());
                if fail == Some("release") {
                    Err("injected release".into())
                } else {
                    Ok(())
                }
            },
            |_| {
                log.borrow_mut().push("popup");
                if fail == Some("popup") {
                    Err("injected popup".into())
                } else {
                    Ok(())
                }
            },
        );
        self.log.extend(log.into_inner());
        outcome
    }
    fn iconify(&mut self, deadline: Instant, dispatched: &mut bool) -> Result<bool, HostError> {
        assert!(self.runtime.visibility_dispatch_eligible());
        if self.now >= deadline {
            return Ok(false);
        }
        *dispatched = true;
        self.step("iconify")?;
        Ok(true)
    }
    fn restore(&mut self, deadline: Instant, dispatched: &mut bool) -> Result<bool, HostError> {
        assert!(self.runtime.visibility_dispatch_eligible());
        if self.now >= deadline {
            return Ok(false);
        }
        *dispatched = true;
        self.step("user_time_zero")?;
        if self.now >= deadline {
            return Ok(false);
        }
        self.step("map")?;
        Ok(true) // acknowledgement alone deliberately changes no observation
    }
    fn hidden(
        &mut self,
        hidden: bool,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<bool, HostError> {
        assert!(self.runtime.visibility_dispatch_eligible());
        if self.now >= deadline {
            return Ok(false);
        }
        *dispatched = true;
        self.step(if hidden {
            "add_hidden"
        } else {
            "remove_hidden"
        })?;
        Ok(true) // the WM's state must arrive through a later fresh observation
    }
    fn placement(&mut self) -> Result<ShowPlacement, HostError> {
        self.step("geometry")?;
        Ok(ShowPlacement {
            area: self.area,
            current_desktop: self.current.current,
            input: InputPlacement {
                selected_monitor: 20,
                observed_monitor: self.monitor,
                geometry: self.geometry,
                bounds: self.bounds,
            },
        })
    }
    fn correct(
        &mut self,
        target: Point,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<Option<Instant>, HostError> {
        if self.now + CONFIRMATION_TIMEOUT > deadline {
            return Ok(None);
        }
        *dispatched = true;
        let confirmation = self.now + CONFIRMATION_TIMEOUT;
        self.step("correct")?;
        self.runtime.body = BodyAvailability::Validating;
        if !self.refuse_correction {
            self.geometry.x = target.x;
            self.geometry.y = target.y;
        }
        Ok(Some(confirmation))
    }
    fn enable(
        &mut self,
        snapshot: &x11::visibility::VisibilitySnapshot,
        p: &InputPlacement,
        deadline: Option<Instant>,
    ) -> Result<bool, HostError> {
        if deadline.is_some_and(|dl| self.now >= dl) {
            return Ok(false);
        }
        self.step("enable")?;
        self.window.confirmed_origin = p.geometry.origin();
        self.window.cancel_in_flight_moves();
        self.runtime.body = BodyAvailability::Ready;
        let fail = self.fail;
        self.runtime
            .restore_visibility_input_with(&self.window, snapshot.evidence, p, || {
                if fail == Some("shape") {
                    Err("injected shape".into())
                } else {
                    Ok(4 * 65536 + 123)
                }
            })?;
        Ok(true)
    }
}
impl BringTopHost for Fake {
    fn qualify_layer(
        &self,
        snapshot: &x11::visibility::VisibilitySnapshot,
        baseline: control::VisibilityBaseline,
    ) -> Result<ObservedLayer, RemoteLayerIssue> {
        fake_layer_atoms().qualify_remote(snapshot, baseline)
    }
    fn service_above(
        &mut self,
        operation: &mut ActiveOperation,
    ) -> Result<(LayerOutcome, Option<RemoteLayerIssue>), HostError> {
        let baseline = operation.show.baseline.unwrap();
        let mut layers = std::mem::take(&mut self.runtime.layers);
        let host = RefCell::new(&mut *self);
        let result = service_remote_layer_with(
            &mut layers,
            operation,
            |_| {
                let mut h = host.borrow_mut();
                h.step("layer_read")?;
                if let Some(issue) = h.layer_issue {
                    return Ok(Err(issue));
                }
                if let Some(read) = h.layer_reads.pop_front() {
                    h.current = read;
                }
                let snapshot = h.observe()?;
                Ok(fake_layer_atoms().qualify_remote(&snapshot, baseline))
            },
            || {
                let mut h = host.borrow_mut();
                h.step("layer_support")?;
                let default = h.layer_support;
                Ok(h.support_reads.pop_front().unwrap_or(Ok(default)))
            },
            |mutation, _, dispatched| {
                let mut h = host.borrow_mut();
                if h.fail_before_layer_dispatch {
                    return Err("injected before layer dispatch".into());
                }
                *dispatched = true;
                h.mutations.push(mutation);
                h.step(match mutation {
                    Mutation::Remove(_) => "remove_below",
                    Mutation::AddAbove => "add_above",
                    Mutation::AddBelow => panic!("G targets only Above"),
                })?;
                match mutation {
                    Mutation::Remove(_) if h.layer_mode == 3 => h.current.layer = L::Above,
                    Mutation::Remove(_) if h.layer_mode != 1 => h.current.layer = L::Normal,
                    Mutation::AddAbove if h.layer_mode == 0 => h.current.layer = L::Above,
                    _ => {}
                }
                Ok(true)
            },
            || host.borrow().now,
        );
        self.runtime.layers = layers;
        result
    }
    fn abort_above(&mut self) {
        self.runtime.layers.abort();
    }
    fn layer_deadline(&self) -> Option<Instant> {
        self.runtime.layers.deadline()
    }
}
fn fake_layer_atoms() -> LayerAtoms {
    LayerAtoms {
        state: 12,
        supported: 18,
        above: 16,
        below: 17,
    }
}
fn minimized() -> Fake {
    Fake::new(reading(Visibility::Minimized, L::Normal, 2, 2))
}
fn visible() -> Reading {
    reading(Visibility::NotMinimizedHere, L::Normal, 2, 2)
}
#[test]
fn recover_service_cannot_call_any_body_host_method() {
    let command = Command::Recover;
    let mut host = Fake::new(visible());
    let mut active = None;
    control::admit(
        &mut active,
        control::Request {
            command,
            request_id: 99,
            reply_xid: 88,
            epoch: 7,
        },
        2,
        Lifecycle::Ready,
        false,
        host.now,
    );
    let mut operation = active.unwrap();
    let original = operation;
    assert_eq!(
        service_remote_with(&mut operation, &mut host)
            .unwrap_err()
            .to_string(),
        "Recover requires the dedicated placement owner service"
    );
    assert_eq!(
        service_visibility_with(&mut operation, &mut host)
            .unwrap_err()
            .to_string(),
        "Recover requires the dedicated placement owner service"
    );
    assert!(host.log.is_empty());
    assert!(host.mutations.is_empty());
    assert_eq!(operation, original);
    assert_eq!(host.runtime.body, BodyAvailability::Ready);
}

#[test]
fn here_cannot_enter_legacy_visibility_or_bring_top_service() {
    let mut host = Fake::new(visible());
    let mut active = None;
    control::admit(
        &mut active,
        control::Request {
            command: Command::BringHere,
            request_id: 99,
            reply_xid: 88,
            epoch: 7,
        },
        2,
        Lifecycle::Ready,
        false,
        host.now,
    );
    let operation = active.as_mut().unwrap();
    assert!(service_remote_with(operation, &mut host).is_err());
    assert!(service_visibility_with(operation, &mut host).is_err());
    assert!(host.log.is_empty());
    assert!(host.mutations.is_empty());
    assert!(host.runtime.layers.pending().is_none());
}

fn assert_success(result: control::OperationResult, here: bool, dispatched: bool) {
    assert_eq!(result.status(), Status::Success);
    assert_eq!(result.exit_code(), control::ExitCode::Success);
    let d = result.detail();
    assert_eq!(d.reason, Reason::None);
    assert!(d.progress.not_minimized_confirmed);
    assert_eq!(d.progress.input_ready, here);
    assert_eq!(d.progress.placement_deferred, !here);
    assert_eq!(d.progress.host_mutation_dispatched, dispatched);
    assert_eq!(
        d.visibility,
        if here {
            Visibility::NotMinimizedHere
        } else {
            Visibility::NotMinimizedElsewhere
        }
    );
    control::Detail::decode(d.encode()).unwrap();
}

/// Reuse the existing visibility fake and production owner progression in
/// the transport's coupled caller-timeout test, without exposing its internals.
pub(crate) struct CallerTimeoutShow {
    host: Fake,
}

impl CallerTimeoutShow {
    pub(crate) fn new(now: Instant) -> Self {
        let mut host = minimized();
        host.now = now;
        Self { host }
    }

    pub(crate) fn service_at(
        &mut self,
        operation: &mut ActiveOperation,
        now: Instant,
    ) -> Result<Option<control::OperationResult>, HostError> {
        self.host.now = now;
        if !remote_service_due(operation, now) {
            return Ok(None);
        }
        service_remote_with(operation, &mut self.host)
    }

    pub(crate) fn observe_restored(&mut self, operation: &mut ActiveOperation) {
        // Model a WM notification and subsequent fresh readable state, not
        // a terminal result. Production Show still verifies placement/input.
        self.host.current = visible();
        mark_remote_observation(operation, false);
    }

    pub(crate) fn assert_pending(&self) {
        assert_eq!(
            control::classify(self.host.current.evidence).visibility,
            Visibility::Minimized
        );
        assert_eq!(self.host.count("map"), 1);
        assert_eq!(self.host.count("user_time_zero"), 1);
        assert_eq!(self.host.count("enable"), 0);
        assert!(self.host.runtime.input_fence.blocked);
        assert!(!self.host.runtime.accepts_input());
    }

    pub(crate) fn assert_completed(&self, result: control::OperationResult) {
        assert_success(result, true, true);
        assert_eq!(
            control::classify(self.host.current.evidence).visibility,
            Visibility::NotMinimizedHere
        );
        assert_eq!(self.host.count("map"), 1);
        assert_eq!(self.host.count("user_time_zero"), 1);
        assert_eq!(self.host.count("fence"), 1);
        assert_eq!(self.host.count("enable"), 1);
        assert!(self.host.runtime.accepts_input());
        assert!(!self.host.runtime.pointer.is_grabbed());
        assert!(self.host.runtime.menu.is_none());
    }

    pub(crate) fn trace(&self) -> &[&'static str] {
        &self.host.log
    }
}

/// Standalone Hide/Show snapshots for the Section-12 lifecycle regressions.
/// Keep the fake alive through terminal delivery so shutdown can be checked
/// against actual dispatch, geometry and input state, not just phase labels.
pub(crate) mod h_c2 {
    use super::*;
    use control::{HidePhase, Stage};

    pub(crate) struct Fixture {
        host: Fake,
        original_error: Option<HostError>,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct HostState {
        body: u32,
        geometry: Rect,
        availability: BodyAvailability,
        blocked: bool,
        empty_confirmed: bool,
        input_ready: bool,
    }

    impl Fixture {
        pub(crate) fn take_original_error(&mut self) -> Option<HostError> {
            self.original_error.take()
        }

        pub(crate) fn state(&self) -> HostState {
            HostState {
                body: self.body(),
                geometry: self.host.geometry,
                availability: self.host.runtime.body,
                blocked: self.host.runtime.input_fence.blocked,
                empty_confirmed: self.host.runtime.input_fence.empty_confirmed,
                input_ready: self.input_ready(),
            }
        }

        pub(crate) fn body(&self) -> u32 {
            self.host.window.window
        }

        pub(crate) fn trace(&self) -> &[&'static str] {
            &self.host.log
        }

        pub(crate) fn input_ready(&self) -> bool {
            self.host.runtime.accepts_input()
        }

        /// Same active-record boundary as owner servicing: notifications
        /// can only schedule existing work, never recreate a terminal record.
        pub(crate) fn notify_and_service(&mut self, active: Option<&mut ActiveOperation>) {
            if let Some(op) = active {
                mark_remote_observation(op, false);
                if remote_service_due(op, self.host.now) {
                    service_remote_with(op, &mut self.host).unwrap();
                }
            }
        }

        pub(crate) fn late_notifications(&mut self, active: Option<&mut ActiveOperation>) {
            // Even convincing late WM state and an expired correction/owner
            // deadline cannot restart a completed operation.
            self.host.current = reading(Visibility::Minimized, L::Normal, 2, 2);
            self.host.now += Duration::from_secs(10);
            self.notify_and_service(active);
        }

        pub(crate) fn cleanup(&mut self) -> Result<(), HostError> {
            let h = &mut self.host;
            abort_layer(&mut h.runtime.layers, "shutdown");
            h.runtime.correction = None;
            h.runtime.cancel_menu_with(
                &mut h.window,
                |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                |_| Ok(()),
            )?;
            assert!(h.runtime.interaction.is_idle());
            assert!(!h.runtime.pointer.is_grabbed());
            assert!(h.runtime.menu.is_none());
            assert!(h.runtime.layers.pending().is_none());
            Ok(())
        }
    }

    pub(crate) fn hide_loss_after_dispatch(
        destroyed: bool,
        elsewhere: bool,
    ) -> (ActiveOperation, control::OperationResult, Fixture) {
        let desktop = if elsewhere { 7 } else { 2 };
        let initial = if elsewhere {
            Visibility::NotMinimizedElsewhere
        } else {
            Visibility::NotMinimizedHere
        };
        let mut h = Fake::new(reading(initial, L::Normal, desktop, 2));
        let body = h.window.window;
        let geometry = h.geometry;
        let mut op = h.command_operation(Command::Hide);
        let deadline = op.deadline;
        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
        assert_eq!(
            op.hide.phase,
            if elsewhere {
                HidePhase::HiddenAddRequested
            } else {
                HidePhase::IconifyRequested
            }
        );
        assert_eq!(h.count("iconify"), usize::from(!elsewhere));
        assert_eq!(h.count("add_hidden"), usize::from(elsewhere));
        assert_eq!(
            h.log,
            [
                "fence",
                "empty",
                "release",
                "observe",
                if elsewhere { "add_hidden" } else { "iconify" }
            ]
        );
        assert!(h.fence_settled());
        assert!(op.detail.progress.host_mutation_dispatched);

        // Availability changes only AFTER the checked dispatch above.
        let (visibility, reason) = if destroyed {
            (Visibility::Destroyed, Reason::BodyDestroyed)
        } else {
            (Visibility::Withdrawn, Reason::Withdrawn)
        };
        let mut lost = reading(visibility, L::Unknown, desktop, 2);
        if destroyed {
            lost.evidence.wm_state = WmState::Absent;
            lost.evidence.map_state = Evidence::Missing;
            lost.evidence.hidden = Evidence::Missing;
            lost.evidence.coherent = false;
        } else {
            assert_eq!(
                lost.evidence.map_state,
                Evidence::Readable(MapState::Unmapped)
            );
            assert_eq!(lost.evidence.hidden, Evidence::Readable(false));
        }
        h.current = lost;
        mark_remote_observation(&mut op, false);
        assert!(remote_service_due(&op, h.now));
        let before = h.log.len();
        let result = service_remote_with(&mut op, &mut h).unwrap().unwrap();
        // Fresh failure observation plus independent safe-input observation;
        // no retry, remanagement, geometry work or enabling is performed.
        assert_eq!(&h.log[before..], ["observe", "observe"]);
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.exit_code(), control::ExitCode::OperationFailed);
        let d = result.detail();
        assert_eq!(
            (d.reason, d.visibility, d.stage),
            (reason, visibility, Stage::Hide)
        );
        assert_eq!(
            d.progress,
            control::Progress {
                host_mutation_dispatched: true,
                ..control::Progress::default()
            }
        );
        assert_eq!(control::Detail::decode(d.encode()).unwrap(), d);
        assert_eq!(op.deadline, deadline);
        assert!(h.runtime.input_fence.blocked);
        assert!(!h.runtime.accepts_input());
        assert!(h.fence_settled());
        assert_eq!((h.window.window, h.geometry), (body, geometry));
        (
            op,
            result,
            Fixture {
                host: h,
                original_error: None,
            },
        )
    }

    pub(crate) fn hide_phases() -> Vec<(ActiveOperation, Fixture)> {
        [
            HidePhase::Fence,
            HidePhase::Fenced,
            HidePhase::IconifyAttempted,
            HidePhase::IconifyRequested,
            HidePhase::HiddenAddAttempted,
            HidePhase::HiddenAddRequested,
        ]
        .into_iter()
        .map(|phase| {
            let elsewhere = matches!(
                phase,
                HidePhase::HiddenAddAttempted | HidePhase::HiddenAddRequested
            );
            let mut h = Fake::new(reading(
                if elsewhere {
                    Visibility::NotMinimizedElsewhere
                } else {
                    Visibility::NotMinimizedHere
                },
                L::Normal,
                if elsewhere { 7 } else { 2 },
                2,
            ));
            let mut op = h.command_operation(Command::Hide);
            let deadline = op.deadline;
            let mut original_error = None;
            if phase == HidePhase::Fenced {
                let mut transient = h.current;
                transient.evidence.coherent = false;
                h.reads.push_back(transient);
            }
            let uncertain = matches!(
                phase,
                HidePhase::IconifyAttempted | HidePhase::HiddenAddAttempted
            );
            if uncertain {
                h.fail = Some(if elsewhere { "add_hidden" } else { "iconify" });
            }
            if phase != HidePhase::Fence {
                let outcome = service_remote_with(&mut op, &mut h);
                if uncertain {
                    let error = outcome.unwrap_err();
                    assert!(error.to_string().contains(h.fail.unwrap()));
                    original_error = Some(error);
                } else {
                    assert!(outcome.unwrap().is_none());
                }
                assert!(h.fence_settled());
                assert!(!h.runtime.accepts_input());
            }
            assert_eq!(op.hide.phase, phase);
            assert_eq!(op.deadline, deadline);
            let sent = !matches!(phase, HidePhase::Fence | HidePhase::Fenced);
            assert_eq!(h.count("iconify"), usize::from(sent && !elsewhere));
            assert_eq!(h.count("add_hidden"), usize::from(sent && elsewhere));
            assert_eq!(op.detail.progress.host_mutation_dispatched, sent);
            assert_eq!(
                h.count("map") + h.count("remove_hidden") + h.count("enable"),
                0
            );
            if sent {
                assert_eq!(
                    op.hide.baseline.unwrap().desktop,
                    if elsewhere { 7 } else { 2 }
                );
                assert_eq!(op.hide.baseline.unwrap().current_desktop, 2);
                assert_eq!(
                    op.detail.workspace,
                    if elsewhere {
                        Workspace::Other
                    } else {
                        Workspace::Current
                    }
                );
            }
            (
                op,
                Fixture {
                    host: h,
                    original_error,
                },
            )
        })
        .collect()
    }

    pub(crate) fn show_phases() -> Vec<(ActiveOperation, Fixture)> {
        [
            ShowPhase::Observe,
            ShowPhase::Fenced,
            ShowPhase::RestoreAttempted,
            ShowPhase::RestoreRequested,
            ShowPhase::HiddenRemoveAttempted,
            ShowPhase::HiddenRemoveRequested,
            ShowPhase::EnablingInput,
            ShowPhase::InputRestored,
        ]
        .into_iter()
        .map(|phase| {
            let elsewhere = matches!(
                phase,
                ShowPhase::HiddenRemoveAttempted | ShowPhase::HiddenRemoveRequested
            );
            let mut h = Fake::new(reading(
                Visibility::Minimized,
                L::Normal,
                if elsewhere { 7 } else { 2 },
                2,
            ));
            h.runtime.body = BodyAvailability::Unavailable;
            let mut op = h.operation();
            let deadline = op.deadline;
            let mut original_error = None;
            if phase == ShowPhase::Fenced {
                let mut transient = h.current;
                transient.evidence.coherent = false;
                h.reads = VecDeque::from([h.current, transient]);
            }
            let uncertain = matches!(
                phase,
                ShowPhase::RestoreAttempted | ShowPhase::HiddenRemoveAttempted
            );
            if uncertain {
                h.fail = Some(if elsewhere {
                    "remove_hidden"
                } else {
                    "user_time_zero"
                });
            }
            if phase != ShowPhase::Observe {
                let outcome = service_remote_with(&mut op, &mut h);
                if uncertain {
                    let error = outcome.unwrap_err();
                    assert!(error.to_string().contains(h.fail.unwrap()));
                    original_error = Some(error);
                } else {
                    assert!(outcome.unwrap().is_none());
                }
            }
            if matches!(phase, ShowPhase::EnablingInput | ShowPhase::InputRestored) {
                h.current = visible();
                mark_remote_observation(&mut op, false);
                if phase == ShowPhase::EnablingInput {
                    h.fail = Some("enable");
                    let error = service_remote_with(&mut op, &mut h).unwrap_err();
                    assert_eq!(error.to_string(), "injected enable");
                    original_error = Some(error);
                } else {
                    let result = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                    assert_success(result, true, true);
                }
            }
            assert_eq!(op.show.phase, phase);
            assert_eq!(op.deadline, deadline);
            let sent = !matches!(phase, ShowPhase::Observe | ShowPhase::Fenced);
            assert_eq!(op.detail.progress.host_mutation_dispatched, sent);
            assert_eq!(h.count("remove_hidden"), usize::from(sent && elsewhere));
            assert_eq!(h.count("user_time_zero"), usize::from(sent && !elsewhere));
            assert_eq!(
                h.count("map"),
                usize::from(sent && !elsewhere && phase != ShowPhase::RestoreAttempted)
            );
            assert_eq!(
                h.count("iconify") + h.count("add_hidden") + h.count("correct"),
                0
            );
            assert_eq!(h.runtime.accepts_input(), phase == ShowPhase::InputRestored);
            assert_eq!(
                op.detail.progress.input_ready,
                phase == ShowPhase::InputRestored
            );
            assert_eq!(
                op.detail.progress.not_minimized_confirmed,
                phase == ShowPhase::InputRestored
            );
            if elsewhere {
                assert_eq!(op.show.baseline.unwrap().desktop, 7);
                assert_eq!(op.show.baseline.unwrap().current_desktop, 2);
                assert_eq!(op.detail.workspace, Workspace::Other);
                assert!(op.show.area.is_none());
                assert!(op.show.correction.is_none());
                assert!(!op.show.correction_sent);
                assert_eq!(h.count("geometry") + h.count("enable"), 0);
                assert!(h.runtime.input_fence.blocked);
                assert!(!op.detail.progress.placement_deferred); // restoration still unconfirmed
            }
            (
                op,
                Fixture {
                    host: h,
                    original_error,
                },
            )
        })
        .collect()
    }

    pub(crate) fn geometry_snapshots() -> Vec<(&'static str, ActiveOperation, Fixture)> {
        let mut fixtures = vec![];
        for name in [
            "placement-pending",
            "correction-pending",
            "correction-confirmed-input-pending",
        ] {
            let mut h = minimized();
            h.runtime.body = BodyAvailability::Unavailable;
            let mut op = h.operation();
            let deadline = op.deadline;
            let mut original_error = None;
            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
            h.current = visible();
            mark_remote_observation(&mut op, false);
            if name == "placement-pending" {
                // Snapshot at the first geometry read after restoration.
                h.fail = Some("geometry");
                let error = service_remote_with(&mut op, &mut h).unwrap_err();
                assert_eq!(error.to_string(), "injected geometry");
                original_error = Some(error);
                assert!(op.show.area.is_none());
                assert!(op.show.correction.is_none());
                assert!(!op.show.correction_sent);
            } else {
                h.area = Rect::new(200, 200, 460, 460);
                h.bounds = checked_probe_bounds(h.area).unwrap();
                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                let (target, confirmation) = op.show.correction.unwrap();
                assert_eq!(target, Point::new(200, 200));
                assert_eq!(confirmation, h.now + CONFIRMATION_TIMEOUT);
                assert_eq!(op.show.area, Some(h.area));
                assert!(op.show.correction_sent);
                assert_eq!(h.count("correct"), 1);
                if name == "correction-confirmed-input-pending" {
                    h.now = confirmation;
                    mark_remote_observation(&mut op, false);
                    // No distinct correction-confirmed phase exists: the
                    // same production call proceeds to checked input enable.
                    h.fail = Some("enable");
                    let error = service_remote_with(&mut op, &mut h).unwrap_err();
                    assert_eq!(error.to_string(), "injected enable");
                    original_error = Some(error);
                    assert!(op.show.correction.is_none());
                    assert_eq!(op.show.phase, ShowPhase::EnablingInput);
                    assert_eq!(h.geometry.origin(), target);
                } else {
                    assert_eq!(h.count("enable"), 0);
                    assert_eq!(op.show.phase, ShowPhase::RestoreRequested);
                }
            }
            assert_eq!(
                (op.stage, op.detail.stage),
                (Stage::Geometry, Stage::Geometry)
            );
            assert_eq!(op.deadline, deadline);
            assert!(!op.show.observe);
            assert!(!op.show.layout_interrupted);
            assert!(op.detail.progress.host_mutation_dispatched);
            assert!(!op.detail.progress.input_ready);
            assert!(!op.detail.progress.not_minimized_confirmed);
            assert!(h.runtime.input_fence.blocked);
            assert!(!h.runtime.accepts_input());
            assert_eq!(h.count("map"), 1);
            assert_eq!(h.count("add_hidden") + h.count("remove_hidden"), 0);
            fixtures.push((
                name,
                op,
                Fixture {
                    host: h,
                    original_error,
                },
            ));
        }
        fixtures
    }
}

pub(crate) mod g3_hardening {
    use super::*;
    use control::Stage;

    fn fresh(min: bool, layer: L, desktop: u32) -> (Fake, ActiveOperation) {
        let h = Fake::new(reading(
            if min {
                Visibility::Minimized
            } else if desktop == 7 {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::NotMinimizedHere
            },
            layer,
            desktop,
            2,
        ));
        let op = h.command_operation(Command::BringTop);
        (h, op)
    }
    fn restored(h: &mut Fake, op: &mut ActiveOperation) {
        assert!(service_remote_with(op, h).unwrap().is_none());
        let r = h.current;
        h.current = reading(
            if r.desktop == 7 {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::NotMinimizedHere
            },
            r.layer,
            r.desktop,
            r.current,
        );
        mark_remote_observation(op, false);
    }
    fn assert_terminal(r: control::OperationResult, status: Status, reason: Reason, stage: Stage) {
        assert_eq!(
            (r.status(), r.detail().reason, r.detail().stage),
            (status, reason, stage)
        );
        let d = r.detail();
        assert_eq!(
            d.progress.not_minimized_confirmed,
            matches!(
                d.visibility,
                Visibility::NotMinimizedHere | Visibility::NotMinimizedElsewhere
            )
        );
        if d.progress.above_confirmed {
            assert_eq!(d.layer, L::Above);
            assert!(d.progress.not_minimized_confirmed);
        }
        if d.progress.input_ready {
            assert_eq!(d.visibility, Visibility::NotMinimizedHere);
            assert!(!d.progress.placement_deferred);
        }
        if d.progress.placement_deferred {
            assert_eq!(d.visibility, Visibility::NotMinimizedElsewhere);
            assert!(!d.progress.input_ready);
        }
        assert_eq!(control::Detail::decode(d.encode()).unwrap(), d);
        assert_eq!(d.encode() & 0xff00_0000, 0);
    }
    fn assert_before_layer(h: &Fake) {
        assert!(h.mutations.is_empty());
        assert_eq!(h.count("layer_read"), 0);
    }

    #[test]
    fn owner_deadline_edges_allow_only_final_confirmation_without_new_layer_work() {
        for delta in [-1_i64, 0, 1] {
            for above in [false, true] {
                let (mut h, mut op) = fresh(false, L::Normal, 2);
                observe_visibility(&mut op, &mut h).unwrap();
                op.stage = Stage::Layer;
                op.detail.stage = Stage::Layer;
                h.now += Duration::from_millis(3000);
                op.bring_top.layer_admitted = Some(h.now);
                h.layer_mode = 1;
                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                assert_eq!(h.layer_deadline(), Some(op.deadline));
                h.now = if delta < 0 {
                    op.deadline - Duration::from_nanos(1)
                } else {
                    op.deadline + Duration::from_nanos(delta as u64)
                };
                h.current.layer = if above { L::Above } else { L::Normal };
                let result = service_remote_with(&mut op, &mut h).unwrap();
                if above {
                    assert_eq!(result.unwrap().status(), Status::Success);
                } else if delta < 0 {
                    assert!(result.is_none());
                } else {
                    assert_terminal(
                        result.unwrap(),
                        Status::Failed,
                        Reason::ConfirmationTimeout,
                        Stage::Layer,
                    );
                }
                assert_eq!(h.mutations, [Mutation::AddAbove]);
            }
        }
    }
    #[test]
    fn incoherent_final_state_times_out_without_stale_bits_or_erasing_restoration_history() {
        let (mut h, mut op) = fresh(true, L::Normal, 7);
        restored(&mut h, &mut op);
        h.layer_mode = 1;
        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
        let deadline = h.layer_deadline().unwrap();
        h.current.evidence.coherent = false;
        h.current.layer = L::Unknown;
        for _ in 0..20 {
            h.now += Duration::from_millis(20);
            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
            assert_eq!(h.layer_deadline(), Some(deadline));
        }
        h.now = deadline;
        let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
        assert_terminal(
            r,
            Status::Partial,
            Reason::ConfirmationTimeout,
            Stage::Layer,
        );
        assert_eq!(
            r.detail().progress,
            control::Progress {
                host_mutation_dispatched: true,
                ..control::Progress::default()
            }
        );
        assert!(op.bring_top.restoration_confirmed);
        assert_eq!(h.mutations, [Mutation::AddAbove]);
    }
    #[test]
    fn selecting_popup_during_g_visibility_cleans_before_busy_and_stale_release_is_inert() {
        let (mut h, op) = fresh(false, L::Normal, 2);
        h.runtime
            .acquire_menu_with(MenuPopup::test_popup(10, 1), Point::new(0, 0), || {
                Ok((GrabStatus::SUCCESS, 100))
            })
            .unwrap();
        let log = RefCell::new(vec![]);
        h.runtime
            .complete_menu_with(
                &mut h.window,
                MenuOutcome::SetLayer(Layer::Below),
                |p| {
                    log.borrow_mut().push("release");
                    p.release_with(|| Ok(())).map(|_| ())
                },
                |_| {
                    log.borrow_mut().push("destroy");
                    Ok(())
                },
                |_, target| {
                    log.borrow_mut().push("busy");
                    local_layer_after_menu_with(
                        op.request.command == Command::BringTop,
                        target,
                        || panic!("no local layer work during remote visibility"),
                    )
                },
            )
            .unwrap();
        assert_eq!(*log.borrow(), ["release", "destroy", "busy"]);
        assert!(h.runtime.menu.is_none());
        assert!(!h.runtime.pointer.is_grabbed());
        assert!(h.runtime.layers.pending().is_none());
        assert_eq!(
            h.runtime
                .interaction
                .menu_release(1, crate::geometry::MenuHit::Row(2), 101, 0),
            HostAction::None
        );
        let (mut h, mut op) = fresh(true, L::Normal, 2);
        h.runtime = visibility_fence_tests::runtime(4);
        restored(&mut h, &mut op);
        assert_eq!(
            h.runtime
                .interaction
                .menu_release(1, crate::geometry::MenuHit::Row(4), 101, 0),
            HostAction::None
        );
        assert!(h.runtime.menu.is_none());
        assert!(h.mutations.is_empty());
    }
    #[test]
    fn timeout_final_reread_can_confirm_already_achieved_above_without_resend() {
        let (mut h, mut op) = fresh(true, L::Normal, 2);
        restored(&mut h, &mut op);
        h.layer_mode = 1;
        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
        h.now = h.layer_deadline().unwrap();
        h.changes = VecDeque::from([
            (
                "observe",
                reading(Visibility::NotMinimizedHere, L::Normal, 2, 2),
            ),
            (
                "observe",
                reading(Visibility::NotMinimizedHere, L::Above, 2, 2),
            ),
        ]);
        let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
        assert_terminal(r, Status::Success, Reason::None, Stage::Layer);
        assert_eq!(h.mutations, [Mutation::AddAbove]);
        assert!(r.detail().progress.above_confirmed);
    }
    #[test]
    fn user_time_only_and_checked_layer_effect_with_current_above_do_not_invent_partial() {
        let (mut h, mut op) = fresh(true, L::Normal, 2);
        h.advance = Some(("user_time_zero", op.bring_top.visibility_cutoff - h.now));
        let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
        assert_terminal(r, Status::Failed, Reason::ConfirmationTimeout, Stage::Show);
        assert!(r.detail().progress.host_mutation_dispatched);
        assert_eq!(h.count("map"), 0);
        assert!(!op.bring_top.restoration_confirmed);
        assert_before_layer(&h);
        let (mut h, mut op) = fresh(false, L::Normal, 2);
        h.fail = Some("add_above");
        h.changes.push_back((
            "add_above",
            reading(Visibility::NotMinimizedHere, L::Above, 2, 2),
        ));
        assert!(service_remote_with(&mut op, &mut h).is_err());
        let r = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
        assert_terminal(r, Status::Failed, Reason::X11Failure, Stage::Layer);
        assert!(r.detail().progress.above_confirmed);
        assert!(!op.bring_top.minimized_at_start);
        assert!(!op.bring_top.restoration_confirmed);
        assert_eq!(h.mutations, [Mutation::AddAbove]);
    }
    #[test]
    fn shutdown_cleanup_of_later_layer_phase_pointer_owners_keeps_original_failure() {
        for gesture in [1, 2, 3, 4] {
            let (mut h, mut op) = fresh(false, L::Normal, 2);
            h.layer_mode = 1;
            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
            let layers = std::mem::take(&mut h.runtime.layers);
            h.runtime = visibility_fence_tests::runtime(gesture);
            h.runtime.layers = layers;
            let release = std::cell::Cell::new(0);
            let popup = std::cell::Cell::new(0);
            let result = finish_probe_with(Err("original G shutdown cause".into()), || {
                assert!(abort_layer(&mut h.runtime.layers, "shutdown").is_some());
                assert!(abort_layer(&mut h.runtime.layers, "shutdown repeated").is_none());
                h.runtime.cancel_menu_with(
                    &mut h.window,
                    |p| {
                        release.set(release.get() + 1);
                        p.release_with(|| Ok(())).map(|_| ())
                    },
                    |_| {
                        popup.set(popup.get() + 1);
                        Ok(())
                    },
                )
            });
            assert_eq!(result.unwrap_err().to_string(), "original G shutdown cause");
            assert!(!h.runtime.pointer.is_grabbed());
            assert!(h.runtime.menu.is_none());
            assert!(h.runtime.pending_move.is_none());
            assert!(h.runtime.layers.pending().is_none());
            assert_eq!(release.get(), 1);
            assert_eq!(popup.get(), usize::from(gesture >= 3));
            assert_eq!(h.mutations, [Mutation::AddAbove]);
        }
    }
    /// Real production progression snapshots shared with transport lifecycle
    /// tests. No copied Show/Layer state machine or fabricated proof flags.
    pub(crate) fn lifecycle_fixtures() -> Vec<(&'static str, ActiveOperation, LayerController)> {
        let mut fixtures = vec![];
        for name in [
            "before-baseline",
            "after-fence",
            "restore-attempt",
            "restore-request",
            "after-proof",
            "geometry",
            "input-restored-before-layer",
            "before-layer-send",
            "after-remove",
            "after-add",
            "checked-layer-uncertainty",
            "layer-confirmed-before-reply",
            "transitional-final-read",
        ] {
            let layer = if name == "after-remove" {
                L::Below
            } else if name == "transitional-final-read" {
                L::Above
            } else {
                L::Normal
            };
            let (mut h, mut op) = fresh(true, layer, 2);
            match name {
                "before-baseline" => {}
                "after-fence" => {
                    let mut transient = h.current;
                    transient.evidence.coherent = false;
                    transient.layer = L::Unknown;
                    h.reads = VecDeque::from([h.current, transient]);
                    assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                    assert_eq!(op.show.phase, ShowPhase::Fenced);
                }
                "restore-attempt" => {
                    h.fail = Some("map");
                    assert!(service_remote_with(&mut op, &mut h).is_err());
                }
                "restore-request" => {
                    assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                }
                _ => {
                    restored(&mut h, &mut op);
                    match name {
                        "after-proof" => {
                            h.fail = Some("geometry");
                            assert!(service_remote_with(&mut op, &mut h).is_err());
                        }
                        "geometry" => {
                            h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
                            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                        }
                        "input-restored-before-layer" => {
                            assert!(matches!(
                                service_visibility_stage_with(&mut op, &mut h).unwrap(),
                                Some(VisibilityCompletion::Ready(_))
                            ));
                        }
                        "before-layer-send" => {
                            h.layer_issue = Some(RemoteLayerIssue::Waiting);
                            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                        }
                        "after-remove" | "after-add" => {
                            h.layer_mode = 1;
                            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                        }
                        "checked-layer-uncertainty" => {
                            h.fail = Some("add_above");
                            assert!(service_remote_with(&mut op, &mut h).is_err());
                            assert!(op.bring_top.layer_dispatched);
                            assert!(!op.bring_top.layer_checked);
                        }
                        "layer-confirmed-before-reply" => {
                            assert_eq!(
                                service_remote_with(&mut op, &mut h)
                                    .unwrap()
                                    .unwrap()
                                    .status(),
                                Status::Success
                            );
                        }
                        "transitional-final-read" => {
                            let above = h.current;
                            let mut transient = above;
                            transient.evidence.coherent = false;
                            transient.layer = L::Unknown;
                            h.reads =
                                VecDeque::from([above, above, above, above, above, transient]);
                            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                            assert!(op.bring_top.layer_finished);
                        }
                        _ => unreachable!(),
                    }
                    assert!(op.bring_top.minimized_at_start && op.bring_top.restoration_confirmed);
                }
            }
            fixtures.push((name, op, h.runtime.layers));
        }
        fixtures
    }
    #[test]
    fn destroyed_and_withdrawn_at_each_visibility_boundary_keep_exact_history() {
        for unavailable in [Visibility::Destroyed, Visibility::Withdrawn] {
            for point in 0..4 {
                let (mut h, mut op) = fresh(true, L::Normal, 2);
                let lost = reading(unavailable, L::Unknown, 2, 2);
                match point {
                    0 => h.current = lost,
                    1 => h.reads = VecDeque::from([h.current, lost]),
                    2 => {
                        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                        h.current = lost;
                    }
                    _ => {
                        restored(&mut h, &mut op);
                        h.changes.push_back(("geometry", lost));
                    }
                }
                let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                assert_terminal(
                    r,
                    if point == 3 {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    if unavailable == Visibility::Destroyed {
                        Reason::BodyDestroyed
                    } else {
                        Reason::Withdrawn
                    },
                    if point == 3 {
                        Stage::Geometry
                    } else {
                        Stage::Show
                    },
                );
                assert_eq!(op.bring_top.restoration_confirmed, point == 3);
                assert!(!r.detail().progress.not_minimized_confirmed);
                assert!(!r.detail().progress.above_confirmed);
                assert!(!r.detail().progress.input_ready);
                assert_eq!(h.count("map"), usize::from(point >= 2));
                assert_before_layer(&h);
            }
        }
    }
    #[test]
    fn visibility_bad_capability_properties_and_incoherence_are_bounded_without_effects() {
        for kind in 0..5 {
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            match kind {
                0 => h.current.evidence.capability = Capability::Unsupported,
                1 => h.current.evidence.hidden = Evidence::Malformed,
                2 => h.current.evidence.hidden = Evidence::Missing,
                3 => h.current.evidence.coherent = false,
                _ => h.current.evidence.wm_state = WmState::Malformed,
            }
            h.current.layer = L::Unknown;
            let early = service_remote_with(&mut op, &mut h).unwrap();
            if kind == 0 {
                assert_terminal(
                    early.unwrap(),
                    Status::Failed,
                    Reason::Unsupported,
                    Stage::Show,
                );
            } else {
                assert!(early.is_none());
                for _ in 0..3 {
                    mark_remote_observation(&mut op, false);
                    assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                }
                h.now = op.bring_top.visibility_cutoff;
                assert_terminal(
                    service_remote_with(&mut op, &mut h).unwrap().unwrap(),
                    Status::Failed,
                    Reason::ConfirmationTimeout,
                    Stage::Show,
                );
            }
            assert!(!op.bring_top.restoration_confirmed);
            assert_eq!(h.count("fence"), 0);
            assert_eq!(h.count("map"), 0);
            assert_before_layer(&h);
        }
    }
    #[test]
    fn workspace_drift_before_mutation_after_dispatch_and_after_proof_is_not_repaired() {
        for body_changes in [false, true] {
            for point in 0..3 {
                let (mut h, mut op) = fresh(true, L::Normal, 2);
                let changed = reading(
                    if point == 0 {
                        Visibility::Minimized
                    } else {
                        Visibility::NotMinimizedElsewhere
                    },
                    L::Normal,
                    if body_changes { 7 } else { 2 },
                    if body_changes { 2 } else { 7 },
                );
                match point {
                    0 => h.reads = VecDeque::from([h.current, changed]),
                    1 => {
                        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                        h.current = changed;
                    }
                    _ => {
                        restored(&mut h, &mut op);
                        h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
                        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                        h.current = changed;
                    }
                }
                let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                assert_terminal(
                    r,
                    if point == 2 {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    Reason::LayoutChanged,
                    if point == 2 {
                        Stage::Geometry
                    } else {
                        Stage::Show
                    },
                );
                assert_eq!(op.bring_top.restoration_confirmed, point == 2);
                assert_eq!(h.count("map"), usize::from(point > 0));
                assert_eq!(h.count("correct"), usize::from(point == 2));
                assert_before_layer(&h);
            }
        }
    }
    #[test]
    fn geometry_layout_interruption_and_checked_correction_failure_never_start_layer() {
        for failure in ["layout", "geometry", "correct"] {
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            restored(&mut h, &mut op);
            h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
            if failure == "layout" {
                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                mark_remote_observation(&mut op, true);
                assert_terminal(
                    service_remote_with(&mut op, &mut h).unwrap().unwrap(),
                    Status::Partial,
                    Reason::LayoutChanged,
                    Stage::Geometry,
                );
            } else {
                h.fail = Some(failure);
                let err = service_remote_with(&mut op, &mut h).unwrap_err();
                assert_eq!(err.to_string(), format!("injected {failure}"));
                let r = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
                assert_terminal(r, Status::Partial, Reason::X11Failure, Stage::Geometry);
            }
            assert!(op.bring_top.restoration_confirmed);
            assert_eq!(h.count("map"), 1);
            assert_before_layer(&h);
        }
    }
    #[test]
    fn g_fence_failure_and_enable_refence_failure_keep_primary_error_and_capture_debt() {
        for failure in ["empty", "release", "popup"] {
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            h.runtime = visibility_fence_tests::runtime(4);
            h.fail = Some(failure);
            let err = service_remote_with(&mut op, &mut h).unwrap_err();
            assert_eq!(err.to_string(), format!("injected {failure}"));
            let reason = if failure == "release" {
                Reason::ReleaseFailed
            } else {
                Reason::X11Failure
            };
            let r = finish_bring_top_with(&mut op, &mut h, reason, false).unwrap();
            assert_terminal(r, Status::Failed, reason, Stage::Show);
            assert!(!op.bring_top.restoration_confirmed);
            assert_eq!(h.count("release"), 1);
            assert_eq!(h.count("popup"), 1);
            assert_eq!(h.count("map"), 0);
            assert_before_layer(&h);
            assert_eq!(h.runtime.pointer.is_grabbed(), failure == "release");
        }
        let (mut h, mut op) = fresh(true, L::Normal, 2);
        restored(&mut h, &mut op);
        h.step_failures = VecDeque::from(["enable", "fence"]);
        let err = service_remote_with(&mut op, &mut h).unwrap_err();
        assert_eq!(err.to_string(), "injected enable");
        assert_eq!(h.count("fence"), 2);
        assert_eq!(h.count("enable"), 1);
        let r = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
        assert_terminal(r, Status::Partial, Reason::X11Failure, Stage::Geometry);
        assert!(!r.detail().progress.input_ready);
        assert_before_layer(&h);
    }
    #[test]
    fn layer_loss_or_conflict_before_first_send_and_after_dispatch_is_truthful() {
        for min in [false, true] {
            for desktop in [2, 7] {
                for kind in 0..6 {
                    for after_send in [false, true] {
                        let (mut h, mut op) = fresh(min, L::Normal, desktop);
                        if min {
                            restored(&mut h, &mut op);
                        }
                        if after_send {
                            h.layer_mode = 1;
                            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                        }
                        let mut bad = reading(
                            if desktop == 7 {
                                Visibility::NotMinimizedElsewhere
                            } else {
                                Visibility::NotMinimizedHere
                            },
                            L::Normal,
                            desktop,
                            2,
                        );
                        let reason = match kind {
                            0 => {
                                bad = reading(Visibility::Destroyed, L::Unknown, desktop, 2);
                                Reason::BodyDestroyed
                            }
                            1 => {
                                bad = reading(Visibility::Withdrawn, L::Unknown, desktop, 2);
                                Reason::Withdrawn
                            }
                            2 => {
                                bad.desktop = 8;
                                bad.evidence.workspace = Workspace::Other;
                                Reason::LayoutChanged
                            }
                            3 => {
                                bad.current = 8;
                                bad.evidence.workspace = Workspace::Other;
                                Reason::LayoutChanged
                            }
                            4 => {
                                bad.layer = L::Conflict;
                                Reason::Unverifiable
                            }
                            _ => {
                                bad.evidence.hidden = Evidence::Malformed;
                                bad.layer = L::Unknown;
                                Reason::Unverifiable
                            }
                        };
                        if after_send {
                            h.current = bad;
                        } else {
                            h.layer_reads.push_back(bad);
                        }
                        let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                        assert_terminal(
                            r,
                            if min { Status::Partial } else { Status::Failed },
                            reason,
                            Stage::Layer,
                        );
                        assert_eq!(h.mutations.len(), usize::from(after_send));
                        assert!(!r.detail().progress.above_confirmed);
                        assert!(h.runtime.layers.pending().is_none());
                    }
                }
            }
        }
    }
    #[test]
    fn support_refusal_or_malformed_support_before_removal_and_before_addition() {
        let good = LayerSupport {
            state: true,
            above: true,
            below: true,
        };
        for min in [false, true] {
            for after_remove in [false, true] {
                for malformed in [false, true] {
                    let (mut h, mut op) = fresh(min, L::Below, 2);
                    if min {
                        restored(&mut h, &mut op);
                    }
                    if after_remove {
                        h.support_reads.push_back(Ok(good));
                    }
                    h.support_reads.push_back(if malformed {
                        Err("malformed _NET_SUPPORTED")
                    } else {
                        Ok(LayerSupport {
                            above: false,
                            ..good
                        })
                    });
                    let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                    assert_terminal(
                        r,
                        if min { Status::Partial } else { Status::Failed },
                        if malformed {
                            Reason::Unverifiable
                        } else {
                            Reason::Unsupported
                        },
                        Stage::Layer,
                    );
                    assert_eq!(h.count("remove_below"), usize::from(after_remove));
                    assert_eq!(h.count("add_above"), 0);
                    assert_eq!(
                        r.detail().layer,
                        if after_remove { L::Normal } else { L::Below }
                    );
                    assert!(!r.detail().progress.above_confirmed);
                }
            }
        }
    }
    #[test]
    fn remote_property_support_loss_and_native_errors_preserve_dispatch_marks() {
        for min in [false, true] {
            for before_marker in [false, true] {
                let (mut h, mut op) = fresh(min, L::Normal, 2);
                if min {
                    restored(&mut h, &mut op);
                }
                h.fail_before_layer_dispatch = before_marker;
                if !before_marker {
                    h.fail = Some("add_above");
                }
                let err = service_remote_with(&mut op, &mut h).unwrap_err();
                assert!(err.to_string().contains(if before_marker {
                    "before layer dispatch"
                } else {
                    "add_above"
                }));
                assert!(op.bring_top.layer_attempted);
                assert_eq!(op.bring_top.layer_dispatched, !before_marker);
                assert!(!op.bring_top.layer_checked);
                let r = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
                assert_terminal(
                    r,
                    if min { Status::Partial } else { Status::Failed },
                    Reason::X11Failure,
                    Stage::Layer,
                );
                assert_eq!(h.mutations.len(), usize::from(!before_marker));
                assert!(h.runtime.layers.pending().is_none());
            }
            for support in [
                Evidence::Malformed,
                Evidence::Missing,
                Evidence::Readable(vec![12, 13, 14, 15]),
            ] {
                let (mut h, mut op) = fresh(min, L::Normal, 7);
                if min {
                    restored(&mut h, &mut op);
                }
                h.layer_mode = 1;
                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                h.supported = support.clone();
                h.current.layer = L::Unknown;
                let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                assert_terminal(
                    r,
                    if min { Status::Partial } else { Status::Failed },
                    if matches!(support, Evidence::Readable(_)) {
                        Reason::Unsupported
                    } else {
                        Reason::Unverifiable
                    },
                    Stage::Layer,
                );
                assert_eq!(h.mutations, [Mutation::AddAbove]);
                assert!(!r.detail().progress.above_confirmed);
                for name in ["map", "user_time_zero", "geometry", "correct", "enable"] {
                    assert_eq!(h.count(name), 0);
                }
            }
        }
    }
    #[test]
    fn exact_visibility_geometry_and_layer_edges_never_renew_budgets() {
        for delta in [-1_i64, 0, 1] {
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            let cutoff = op.bring_top.visibility_cutoff;
            h.now = if delta < 0 {
                cutoff - Duration::from_nanos(1)
            } else {
                cutoff + Duration::from_nanos(delta as u64)
            };
            let r = service_remote_with(&mut op, &mut h).unwrap();
            assert_eq!(h.count("map"), usize::from(delta < 0));
            assert_eq!(r.is_none(), delta < 0);
            assert_before_layer(&h);
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            restored(&mut h, &mut op);
            h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
            let boundary = op.bring_top.visibility_cutoff - CONFIRMATION_TIMEOUT;
            h.now = if delta < 0 {
                boundary - Duration::from_nanos(1)
            } else {
                boundary + Duration::from_nanos(delta as u64)
            };
            let r = service_remote_with(&mut op, &mut h).unwrap();
            assert_eq!(h.count("correct"), usize::from(delta <= 0));
            assert_eq!(r.is_none(), delta <= 0);
            assert_before_layer(&h);
            for final_above in [false, true] {
                let (mut h, mut op) = fresh(false, L::Below, 2);
                h.layer_mode = 1;
                assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
                let deadline = h.layer_deadline().unwrap();
                h.now = if delta < 0 {
                    deadline - Duration::from_nanos(1)
                } else {
                    deadline + Duration::from_nanos(delta as u64)
                };
                h.current.layer = if final_above { L::Above } else { L::Normal };
                let r = service_remote_with(&mut op, &mut h).unwrap();
                assert_eq!(h.count("add_above"), usize::from(!final_above && delta < 0));
                if final_above {
                    assert_eq!(r.unwrap().status(), Status::Success);
                } else if delta >= 0 {
                    assert_eq!(r.unwrap().status(), Status::Failed);
                } else {
                    assert!(r.is_none());
                }
                assert_eq!(
                    op.deadline - op.bring_top.visibility_cutoff,
                    Duration::from_millis(1000)
                );
            }
        }
    }
    #[test]
    fn all_desktops_all_layers_visible_and_minimized_preserve_sentinel() {
        for min in [false, true] {
            for layer in [L::Normal, L::Below, L::Above] {
                let (mut h, mut op) = fresh(min, layer, u32::MAX);
                if min {
                    restored(&mut h, &mut op);
                }
                let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
                assert_success(r, true, min || layer != L::Above);
                assert_eq!(r.detail().workspace, Workspace::AllDesktops);
                assert_eq!(h.current.desktop, u32::MAX);
                assert_eq!(op.show.baseline.unwrap().desktop, u32::MAX);
                assert_eq!(h.count("map"), usize::from(min));
                assert_eq!(h.count("remove_hidden"), 0);
            }
        }
    }
    #[test]
    fn g_right_opening_preservation_and_restoration_fence_invalidate_stale_input() {
        let (mut h, mut op) = fresh(false, L::Normal, 2);
        h.runtime = visibility_fence_tests::runtime(2);
        assert_eq!(
            service_remote_with(&mut op, &mut h)
                .unwrap()
                .unwrap()
                .status(),
            Status::Success
        );
        assert!(h.runtime.pointer.is_grabbed());
        assert!(matches!(
            h.runtime.interaction.state(),
            InteractionState::RightPressed { .. }
        ));
        assert_eq!(h.count("fence"), 0);
        for gesture in 1..=4 {
            let (mut h, mut op) = fresh(true, L::Normal, 2);
            h.runtime = visibility_fence_tests::runtime(gesture);
            h.runtime.pending_move = Some(Point::new(200, 200));
            h.window.in_flight_moves.push_back(Point::new(200, 200));
            let generation = h.runtime.menu_generation;
            restored(&mut h, &mut op);
            assert!(h.runtime.visibility_dispatch_eligible());
            assert_eq!(h.runtime.menu_generation, generation + 1);
            assert!(!h.runtime.pointer.is_grabbed());
            assert!(h.runtime.menu.is_none());
            assert_eq!(
                service_remote_with(&mut op, &mut h)
                    .unwrap()
                    .unwrap()
                    .status(),
                Status::Success
            );
            assert_eq!(h.count("fence"), 1);
            let release = Event::ButtonRelease(x11rb::protocol::xproto::ButtonPressEvent {
                event: h.window.window,
                detail: 1,
                ..Default::default()
            });
            assert!(h
                .runtime
                .input_fence
                .rejects(&release, h.window.window, 70000));
            assert!(!h.runtime.interaction.has_left_gesture());
            assert!(h.runtime.pending_move.is_none());
            let reads = h.count("observe");
            restore_deferred_input(&mut h).unwrap();
            assert_eq!(h.count("observe"), reads);
        }
    }
    #[test]
    fn completed_elsewhere_g_defers_input_and_never_resurrects_on_local_return() {
        let (mut h, mut op) = fresh(true, L::Below, 7);
        restored(&mut h, &mut op);
        let r = service_remote_with(&mut op, &mut h).unwrap().unwrap();
        assert_success(r, false, true);
        assert!(h.runtime.input_fence.blocked);
        assert_eq!(h.count("geometry"), 0);
        assert_eq!(h.count("enable"), 0);
        let mutations = h.mutations.clone();
        h.current = reading(Visibility::NotMinimizedHere, L::Above, 7, 7);
        restore_deferred_input(&mut h).unwrap();
        assert!(h.runtime.accepts_input());
        assert_eq!(h.count("enable"), 1);
        assert_eq!(h.mutations, mutations);
        restore_deferred_input(&mut h).unwrap();
        assert_eq!(h.count("enable"), 1);
        assert_eq!(h.count("remove_hidden"), 1);
    }
}
mod bring_top {
    use super::*;
    use control::{ExitCode, Stage};

    fn run(h: &mut Fake, op: &mut ActiveOperation) -> Option<control::OperationResult> {
        service_bring_top_with(op, h).unwrap()
    }
    fn setup(minimized: bool, layer: L, elsewhere: bool) -> (Fake, ActiveOperation) {
        let h = Fake::new(reading(
            if minimized {
                Visibility::Minimized
            } else if elsewhere {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::NotMinimizedHere
            },
            layer,
            if elsewhere { 7 } else { 2 },
            2,
        ));
        let op = h.command_operation(Command::BringTop);
        (h, op)
    }
    fn restore(h: &mut Fake, op: &mut ActiveOperation, elsewhere: bool) {
        assert!(run(h, op).is_none());
        assert!(op.bring_top.minimized_at_start);
        assert!(!op.bring_top.restoration_confirmed);
        assert!(h.mutations.is_empty());
        h.current = reading(
            if elsewhere {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::NotMinimizedHere
            },
            h.current.layer,
            h.current.desktop,
            h.current.current,
        );
        mark_remote_observation(op, false);
    }
    fn expected_mutations(layer: L) -> Vec<Mutation> {
        match layer {
            L::Above => vec![],
            L::Normal => vec![Mutation::AddAbove],
            L::Below => vec![
                Mutation::Remove(layer::Flags {
                    above: false,
                    below: true,
                }),
                Mutation::AddAbove,
            ],
            _ => unreachable!(),
        }
    }
    fn assert_prohibitions(h: &Fake, elsewhere: bool) {
        for name in [
            "workspace_write",
            "current_desktop_request",
            "active_window",
            "set_input_focus",
            "map_raised",
            "restack",
            "second_body",
            "retry",
            "direct_state_write",
        ] {
            assert_eq!(h.count(name), 0, "prohibited {name}");
        }
        if elsewhere {
            for name in [
                "map",
                "user_time_zero",
                "geometry",
                "correct",
                "ordinary_correct",
                "enable",
            ] {
                assert_eq!(h.count(name), 0, "off-workspace {name}");
            }
        }
        assert!(h.mutations.iter().all(|m| matches!(
            m,
            Mutation::AddAbove
                | Mutation::Remove(layer::Flags {
                    above: false,
                    below: true
                })
        )));
    }

    #[test]
    fn local_visible_normal_below_above_matrix_preserves_ready_interaction() {
        for layer in [L::Normal, L::Below, L::Above] {
            let (mut h, mut op) = setup(false, layer, false);
            h.runtime.interaction.handle_left_press(
                Point::new(180, 180),
                (80, 80),
                100,
                Point::new(100, 100),
            );
            h.runtime.interaction.on_grab_acquired();
            h.runtime
                .pointer
                .acquire_with(CaptureOwner::BodyLeft, || Ok((GrabStatus::SUCCESS, 5)))
                .unwrap();
            let result = run(&mut h, &mut op).unwrap();
            assert_success(result, true, layer != L::Above);
            assert!(result.detail().progress.above_confirmed);
            assert_eq!(h.mutations, expected_mutations(layer));
            assert_eq!(h.count("fence"), 0);
            assert_eq!(h.count("map"), 0);
            assert!(h.runtime.pointer.is_grabbed());
            assert!(h.runtime.interaction.has_left_gesture());
            assert!(!op.bring_top.restoration_confirmed);
            assert_prohibitions(&h, false);
        }
    }
    #[test]
    fn local_minimized_normal_below_above_restore_then_compose_matrix() {
        for layer in [L::Normal, L::Below, L::Above] {
            let (mut h, mut op) = setup(true, layer, false);
            restore(&mut h, &mut op, false);
            let result = run(&mut h, &mut op).unwrap();
            assert_success(result, true, true);
            assert!(op.bring_top.restoration_confirmed);
            assert!(result.detail().progress.above_confirmed);
            assert_eq!(h.mutations, expected_mutations(layer));
            for name in ["fence", "map", "user_time_zero", "enable"] {
                assert_eq!(h.count(name), 1);
            }
            if layer != L::Above {
                let index = |name| h.log.iter().position(|s| *s == name).unwrap();
                assert!(index("enable") < index("add_above"));
            }
            assert_prohibitions(&h, false);
        }
    }
    #[test]
    fn off_workspace_visible_three_layer_matrix_no_visibility_or_local_mutation() {
        for layer in [L::Normal, L::Below, L::Above] {
            let (mut h, mut op) = setup(false, layer, true);
            let result = run(&mut h, &mut op).unwrap();
            assert_success(result, false, layer != L::Above);
            assert_eq!(h.mutations, expected_mutations(layer));
            assert_eq!(h.count("remove_hidden"), 0);
            assert_eq!((h.current.desktop, h.current.current), (7, 2));
            assert!(h.runtime.input_fence.blocked);
            assert_prohibitions(&h, true);
        }
    }
    #[test]
    fn off_workspace_minimized_three_layer_matrix_remove_hidden_then_above() {
        for layer in [L::Normal, L::Below, L::Above] {
            let (mut h, mut op) = setup(true, layer, true);
            restore(&mut h, &mut op, true);
            let result = run(&mut h, &mut op).unwrap();
            assert_success(result, false, true);
            assert!(op.bring_top.restoration_confirmed);
            assert_eq!(h.count("remove_hidden"), 1);
            assert_eq!(h.mutations, expected_mutations(layer));
            assert_eq!((h.current.desktop, h.current.current), (7, 2));
            let response = control::TerminalResponse {
                status: result.status(),
                detail: result.detail(),
                request_id: 99,
                control_xid: 50,
            };
            let diagnostic =
                x11::control::CallerOutcome::Terminal(response).diagnostic(Command::BringTop);
            assert!(diagnostic.contains("another workspace"));
            assert!(!diagnostic.contains("visible here"));
            assert_prohibitions(&h, true);
        }
    }
    #[test]
    fn approved_normal_nonviewable_elsewhere_variants_are_deterministic_only() {
        for map in [MapState::Unmapped, MapState::Unviewable] {
            let (mut h, mut op) = setup(false, L::Normal, true);
            h.current.evidence.wm_state = WmState::Normal;
            h.current.evidence.map_state = Evidence::Readable(map);
            let result = run(&mut h, &mut op).unwrap();
            assert_success(result, false, true);
            assert_eq!(h.mutations, [Mutation::AddAbove]);
            assert_prohibitions(&h, true);
        }
    }
    #[test]
    fn restoration_timeout_never_starts_layer_or_creates_partial() {
        for elsewhere in [false, true] {
            let (mut h, mut op) = setup(true, L::Normal, elsewhere);
            assert!(run(&mut h, &mut op).is_none());
            h.now = op.bring_top.visibility_cutoff;
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.exit_code(), ExitCode::OperationFailed);
            assert!(!op.bring_top.restoration_confirmed);
            assert_eq!(h.count("layer_read"), 0);
            assert!(h.mutations.is_empty());
            assert_eq!(h.count(if elsewhere { "remove_hidden" } else { "map" }), 1);
        }
    }
    #[test]
    fn layer_timeout_is_partial_only_after_actual_restoration() {
        for minimized in [false, true] {
            for elsewhere in [false, true] {
                let (mut h, mut op) = setup(minimized, L::Normal, elsewhere);
                if minimized {
                    restore(&mut h, &mut op, elsewhere);
                }
                h.layer_mode = 1;
                assert!(run(&mut h, &mut op).is_none());
                assert_eq!(op.stage, Stage::Layer);
                h.now = h.runtime.layers.deadline().unwrap();
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(
                    result.status(),
                    if minimized {
                        Status::Partial
                    } else {
                        Status::Failed
                    }
                );
                assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
                assert!(result.detail().progress.not_minimized_confirmed);
                assert!(!result.detail().progress.above_confirmed);
                assert_eq!(result.detail().layer, L::Normal);
                assert_eq!(h.mutations, [Mutation::AddAbove]);
                assert!(h.runtime.layers.pending().is_none());
                assert_prohibitions(&h, elsewhere);
            }
        }
    }
    #[test]
    fn layer_unsupported_and_unverifiable_use_restoration_proof() {
        for minimized in [false, true] {
            for issue in [
                RemoteLayerIssue::Unsupported,
                RemoteLayerIssue::Unverifiable,
            ] {
                let (mut h, mut op) = setup(minimized, L::Normal, false);
                if minimized {
                    restore(&mut h, &mut op, false);
                }
                h.layer_issue = Some(issue);
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(
                    result.status(),
                    if minimized {
                        Status::Partial
                    } else {
                        Status::Failed
                    }
                );
                assert_eq!(result.detail().reason, remote_layer_reason(issue));
                assert!(h.mutations.is_empty());
            }
        }
    }
    #[test]
    fn below_removal_then_checked_add_error_is_failed_or_partial_without_rollback() {
        for minimized in [false, true] {
            let (mut h, mut op) = setup(minimized, L::Below, false);
            if minimized {
                restore(&mut h, &mut op, false);
            }
            h.fail = Some("add_above");
            let original = service_bring_top_with(&mut op, &mut h).unwrap_err();
            assert_eq!(original.to_string(), "injected add_above");
            assert_eq!(h.current.layer, L::Normal);
            assert!(h.runtime.layers.pending().is_none());
            assert!(
                op.bring_top.layer_attempted
                    && op.bring_top.layer_dispatched
                    && op.bring_top.layer_checked
            );
            assert!(op.detail.progress.host_mutation_dispatched);
            let result = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
            assert_eq!(
                result.status(),
                if minimized {
                    Status::Partial
                } else {
                    Status::Failed
                }
            );
            assert_eq!(result.detail().layer, L::Normal);
            assert!(!result.detail().progress.above_confirmed);
            assert_eq!(h.mutations, expected_mutations(L::Below));
            assert_prohibitions(&h, false);
        }
    }
    #[test]
    fn checked_first_layer_error_preserves_dispatch_uncertainty_and_native_priority() {
        let (mut h, mut op) = setup(true, L::Normal, false);
        restore(&mut h, &mut op, false);
        h.fail = Some("add_above");
        assert!(service_bring_top_with(&mut op, &mut h).is_err());
        assert!(op.bring_top.layer_dispatched);
        assert!(!op.bring_top.layer_checked);
        let result = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
        assert_eq!(result.status(), Status::Partial);
        assert!(result.detail().progress.host_mutation_dispatched);
        assert!(!result.detail().progress.above_confirmed);
    }
    #[test]
    fn restoration_proof_precedes_geometry_failure_and_layer_is_never_started() {
        for failure in ["geometry", "enable", "shape"] {
            let (mut h, mut op) = setup(true, L::Normal, false);
            restore(&mut h, &mut op, false);
            h.fail = Some(failure);
            let original = service_bring_top_with(&mut op, &mut h).unwrap_err();
            assert!(original.to_string().contains(failure));
            assert!(op.bring_top.restoration_confirmed);
            h.fail = None;
            let result = finish_bring_top_with(&mut op, &mut h, Reason::X11Failure, false).unwrap();
            assert_eq!(result.status(), Status::Partial);
            assert!(result.detail().progress.not_minimized_confirmed);
            assert!(!result.detail().progress.input_ready);
            assert_eq!(h.count("layer_read"), 0);
            assert_eq!(h.count("fence"), if failure == "geometry" { 1 } else { 2 });
        }
    }
    #[test]
    fn geometry_correction_and_input_enable_finish_before_layer() {
        let (mut h, mut op) = setup(true, L::Normal, false);
        restore(&mut h, &mut op, false);
        h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        assert!(run(&mut h, &mut op).is_none());
        let deadline = op.show.correction.unwrap().1;
        assert_eq!(deadline, h.now + Duration::from_millis(150));
        assert_eq!(h.count("layer_read"), 0);
        h.now = deadline;
        assert_success(run(&mut h, &mut op).unwrap(), true, true);
        let index = |name| h.log.iter().position(|s| *s == name).unwrap();
        assert!(index("correct") < index("enable"));
        assert!(index("enable") < index("add_above"));
    }
    #[test]
    fn insufficient_geometry_budget_is_partial_with_no_correction_or_layer() {
        let (mut h, mut op) = setup(true, L::Normal, false);
        restore(&mut h, &mut op, false);
        h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        h.now = op.bring_top.visibility_cutoff - Duration::from_millis(149);
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.status(), Status::Partial);
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert_eq!(h.count("correct"), 0);
        assert_eq!(h.count("layer_read"), 0);
    }
    #[test]
    fn g_allows_coherent_layer_change_during_visibility_but_show_does_not() {
        for command in [Command::Show, Command::BringTop] {
            let (mut h, _) = setup(true, L::Below, false);
            let mut op = h.command_operation(command);
            assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
            h.current = reading(Visibility::NotMinimizedHere, L::Above, 2, 2);
            let result = service_remote_with(&mut op, &mut h).unwrap().unwrap();
            assert_eq!(
                result.status(),
                if command == Command::Show {
                    Status::Failed
                } else {
                    Status::Success
                }
            );
            assert_eq!(
                result.detail().reason,
                if command == Command::Show {
                    Reason::LayerChanged
                } else {
                    Reason::None
                }
            );
            assert!(h.mutations.is_empty());
        }
    }
    #[test]
    fn workspace_drift_during_either_phase_is_terminal_without_compensation() {
        for minimized in [false, true] {
            for phase in [Stage::Show, Stage::Layer] {
                let (mut h, mut op) = setup(minimized, L::Normal, false);
                if minimized {
                    restore(&mut h, &mut op, false);
                }
                if phase == Stage::Layer {
                    h.layer_mode = 1;
                    assert!(run(&mut h, &mut op).is_none());
                }
                if phase == Stage::Show && !minimized {
                    observe_visibility(&mut op, &mut h).unwrap();
                }
                h.current.current = 3;
                h.current.evidence.workspace = Workspace::Other;
                h.current.evidence.wm_state = WmState::Iconic;
                h.current.evidence.map_state = Evidence::Readable(MapState::Unmapped);
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(result.detail().reason, Reason::LayoutChanged);
                assert_eq!(
                    result.status(),
                    if minimized && phase == Stage::Layer {
                        Status::Partial
                    } else {
                        Status::Failed
                    }
                );
                assert!(result.detail().progress.not_minimized_confirmed);
                assert!(!result.detail().progress.placement_deferred);
                assert!(!result.detail().progress.above_confirmed);
                assert_prohibitions(&h, false);
            }
        }
    }
    #[test]
    fn historical_restoration_survives_evidence_loss_without_stale_current_bits() {
        for final_visibility in [
            Visibility::Destroyed,
            Visibility::Withdrawn,
            Visibility::Minimized,
        ] {
            let (mut h, mut op) = setup(true, L::Normal, false);
            restore(&mut h, &mut op, false);
            h.layer_mode = 1;
            assert!(run(&mut h, &mut op).is_none());
            h.current = reading(final_visibility, L::Normal, 2, 2);
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Partial);
            assert!(!result.detail().progress.not_minimized_confirmed);
            assert!(!result.detail().progress.above_confirmed);
            assert!(!result.detail().progress.input_ready);
            assert!(!result.detail().progress.placement_deferred);
        }
    }
    #[test]
    fn spontaneous_above_after_removal_skips_redundant_addition() {
        let (mut h, mut op) = setup(false, L::Below, false);
        h.layer_mode = 3;
        assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Success);
        assert_eq!(h.mutations.len(), 1);
        assert_eq!(h.count("add_above"), 0);
    }
    #[test]
    fn below_reappearance_after_removal_and_conflict_are_terminal() {
        for layer in [L::Below, L::Conflict] {
            let (mut h, mut op) = setup(false, L::Below, false);
            h.layer_mode = 2;
            assert!(run(&mut h, &mut op).is_none());
            h.current.layer = layer;
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(
                result.detail().reason,
                if layer == L::Below {
                    Reason::LayerChanged
                } else {
                    Reason::Unverifiable
                }
            );
            assert_eq!(h.mutations.len(), 2);
            assert!(!result.detail().progress.above_confirmed);
            assert!(h.runtime.layers.pending().is_none());
        }
    }
    #[test]
    fn layer_admission_caps_one_second_and_never_resets_owner_deadline() {
        for millis in [0, 2400, 2499] {
            let (mut h, mut op) = setup(true, L::Normal, false);
            let t0 = h.now;
            restore(&mut h, &mut op, false);
            h.now = t0 + Duration::from_millis(millis);
            h.layer_mode = 1;
            assert!(run(&mut h, &mut op).is_none());
            assert_eq!(op.deadline, t0 + Duration::from_millis(3500));
            assert_eq!(
                op.bring_top.visibility_cutoff,
                t0 + Duration::from_millis(2500)
            );
            assert_eq!(
                h.layer_deadline(),
                Some((h.now + Duration::from_millis(1000)).min(op.deadline))
            );
            for _ in 0..3 {
                mark_remote_observation(&mut op, false);
                assert!(run(&mut h, &mut op).is_none());
                assert_eq!(op.deadline, t0 + Duration::from_millis(3500));
            }
        }
    }
    #[test]
    fn late_layer_entry_is_capped_by_total_owner_budget() {
        let (mut h, mut op) = setup(false, L::Normal, false);
        op.stage = Stage::Layer;
        op.detail.stage = Stage::Layer;
        op.show.baseline = Some(control::VisibilityBaseline {
            desktop: 2,
            current_desktop: 2,
            layer: L::Normal,
        });
        h.now += Duration::from_millis(3100);
        op.bring_top.layer_admitted = Some(h.now);
        h.layer_mode = 1;
        assert!(run(&mut h, &mut op).is_none());
        assert_eq!(h.layer_deadline(), Some(op.deadline));
        h.now = op.deadline;
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(h.mutations.len(), 1);
    }
    #[test]
    fn visibility_cutoff_allows_final_reads_but_no_new_mutation_or_layer_start() {
        for minimized in [false, true] {
            for layer in [L::Normal, L::Above] {
                let (mut h, mut op) = setup(minimized, layer, false);
                h.now = op.bring_top.visibility_cutoff;
                let result = run(&mut h, &mut op).unwrap();
                assert_eq!(
                    result.status(),
                    if !minimized && layer == L::Above {
                        Status::Success
                    } else {
                        Status::Failed
                    }
                );
                for name in ["fence", "map", "user_time_zero", "correct", "enable"] {
                    assert_eq!(h.count(name), 0);
                }
                assert!(h.mutations.is_empty());
            }
        }
    }
    #[test]
    fn final_layer_read_at_cutoff_can_confirm_achieved_above_without_resend() {
        let (mut h, mut op) = setup(false, L::Normal, false);
        h.layer_mode = 1;
        assert!(run(&mut h, &mut op).is_none());
        h.now = h.layer_deadline().unwrap();
        h.current.layer = L::Above;
        assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Success);
        assert_eq!(h.mutations, [Mutation::AddAbove]);
    }
    #[test]
    fn transient_qualification_preserves_pending_controller_and_fixed_deadline() {
        let (mut h, mut op) = setup(false, L::Normal, false);
        h.layer_mode = 1;
        assert!(run(&mut h, &mut op).is_none());
        let pending = h.runtime.layers.pending();
        h.layer_issue = Some(RemoteLayerIssue::Waiting);
        h.now += Duration::from_millis(200);
        assert!(run(&mut h, &mut op).is_none());
        assert_eq!(h.runtime.layers.pending(), pending);
        h.now = pending.unwrap().deadline;
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert_eq!(h.mutations, [Mutation::AddAbove]);
    }
    #[test]
    fn layer_property_evidence_schedules_remote_read_and_configure_stays_ordinary() {
        use x11rb::protocol::xproto::{ConfigureNotifyEvent, PropertyNotifyEvent};
        let (mut h, mut op) = setup(false, L::Normal, false);
        h.layer_mode = 1;
        assert!(run(&mut h, &mut op).is_none());
        let event = Event::PropertyNotify(PropertyNotifyEvent {
            window: 100,
            atom: 12,
            ..Default::default()
        });
        assert!(remote_consumes_visibility_event(&op, &event, 100));
        mark_remote_observation(&mut op, false);
        assert!(remote_service_due(&op, h.now));
        let reads = h.count("layer_read");
        h.current.layer = L::Above;
        assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Success);
        assert!(h.count("layer_read") > reads);
        // The production route leaves ConfigureNotify on the existing
        // translated-root movement reconciliation path while G owns Layer.
        let event = ConfigureNotifyEvent {
            window: 100,
            width: 160,
            height: 160,
            ..Default::default()
        };
        assert!(!remote_consumes_visibility_event(
            &op,
            &Event::ConfigureNotify(event),
            100
        ));
        let target = Point::new(150, 160);
        h.window.in_flight_moves.push_back(target);
        h.window.requested_origin = target;
        assert!(
            matches!(h.window.handle_configure_notify_with(&event, || Ok(target)).unwrap(), ConfigureReconciliation::Confirmed {origin,..} if origin == target)
        );
        assert!(h.window.in_flight_moves.is_empty());
    }
    #[test]
    fn local_menu_cleanup_precedes_busy_and_cannot_steal_g_controller() {
        let (mut h, mut op) = setup(false, L::Normal, false);
        h.layer_mode = 1;
        assert!(run(&mut h, &mut op).is_none());
        let pending = h.runtime.layers.pending();
        h.runtime
            .acquire_menu_with(MenuPopup::test_popup(10, 1), Point::new(0, 0), || {
                Ok((GrabStatus::SUCCESS, 100))
            })
            .unwrap();
        let log = RefCell::new(vec![]);
        h.runtime
            .complete_menu_with(
                &mut h.window,
                MenuOutcome::SetLayer(Layer::Below),
                |p| {
                    log.borrow_mut().push("release");
                    p.release_with(|| Ok(())).map(|_| ())
                },
                |_| {
                    log.borrow_mut().push("destroy");
                    Ok(())
                },
                |_, target| {
                    log.borrow_mut().push("busy");
                    local_layer_after_menu_with(true, target, || {
                        panic!("remote controller must not be stolen")
                    })
                },
            )
            .unwrap();
        assert_eq!(*log.borrow(), ["release", "destroy", "busy"]);
        assert_eq!(h.runtime.layers.pending(), pending);
        assert!(!h.runtime.pointer.is_grabbed());
        assert!(h.runtime.menu.is_none());
    }
    #[test]
    fn an_aborted_remote_controller_never_restarts_or_retries() {
        let (mut h, mut op) = setup(false, L::Normal, false);
        h.layer_mode = 1;
        assert!(run(&mut h, &mut op).is_none());
        h.runtime.layers.abort();
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.detail().reason, Reason::LayerChanged);
        assert_eq!(h.mutations, [Mutation::AddAbove]);
    }
    #[test]
    fn already_ready_popup_is_preserved_by_layer_only_g() {
        let (mut h, mut op) = setup(false, L::Normal, false);
        h.runtime
            .acquire_menu_with(MenuPopup::test_popup(10, 1), Point::new(0, 0), || {
                Ok((GrabStatus::SUCCESS, 100))
            })
            .unwrap();
        assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Success);
        assert!(h.runtime.menu.is_some());
        assert!(h.runtime.pointer.is_grabbed());
        assert_eq!(h.count("fence"), 0);
    }
    #[test]
    fn expiry_during_layer_support_or_removal_never_sends_late_addition() {
        for step in ["layer_support", "remove_below"] {
            let (mut h, mut op) = setup(false, L::Below, false);
            h.advance = Some((step, Duration::from_millis(1000)));
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
            assert_eq!(h.count("add_above"), 0);
            assert_eq!(h.count("remove_below"), usize::from(step == "remove_below"));
            assert!(!result.detail().progress.above_confirmed);
        }
    }
    #[test]
    fn visibility_read_crossing_cutoff_cannot_enable_input_or_start_layer() {
        let (mut h, mut op) = setup(true, L::Normal, false);
        restore(&mut h, &mut op, false);
        h.advance = Some(("geometry", op.bring_top.visibility_cutoff - h.now));
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.status(), Status::Partial);
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert_eq!(h.count("enable"), 0);
        assert!(h.mutations.is_empty());
    }
    #[test]
    fn placement_busy_is_preflight_busy_without_any_g_mutation() {
        let (mut h, mut op) = setup(false, L::Normal, false);
        h.runtime.correction = Some((Point::new(200, 200), h.now + CONFIRMATION_TIMEOUT));
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.status(), Status::Busy);
        assert!(!result.detail().progress.host_mutation_dispatched);
        assert!(h.mutations.is_empty());
        assert_eq!(h.count("fence"), 0);
        assert_eq!(h.count("observe"), 0);
    }
    #[test]
    fn all_desktop_body_follows_local_path_and_preserves_desktop_sentinel() {
        let (mut h, mut op) = setup(true, L::Below, false);
        h.current.desktop = u32::MAX;
        h.current.evidence.workspace = Workspace::AllDesktops;
        assert!(run(&mut h, &mut op).is_none());
        h.current = reading(Visibility::NotMinimizedHere, L::Below, u32::MAX, 2);
        let result = run(&mut h, &mut op).unwrap();
        assert_success(result, true, true);
        assert_eq!(result.detail().workspace, Workspace::AllDesktops);
        assert_eq!(op.show.baseline.unwrap().desktop, u32::MAX);
        assert_eq!(h.count("map"), 1);
        assert_eq!(h.count("remove_hidden"), 0);
    }
    #[test]
    fn transitional_final_read_keeps_operation_without_restarting_completed_layer() {
        let (mut h, mut op) = setup(false, L::Above, false);
        let above = h.current;
        let mut transitional = above;
        transitional.evidence.coherent = false;
        transitional.layer = L::Unknown;
        h.reads = VecDeque::from([above, above, above, above, above, transitional]);
        assert!(run(&mut h, &mut op).is_none());
        assert!(op.bring_top.layer_finished);
        assert!(h.runtime.layers.pending().is_none());
        assert!(!remote_service_due(&op, h.now));
        assert_eq!(h.count("layer_read"), 1);
        h.current = above;
        mark_remote_observation(&mut op, false);
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.status(), Status::Success);
        assert_eq!(h.count("layer_read"), 1);
        assert!(h.mutations.is_empty());
    }
    #[test]
    fn final_layer_drift_cannot_report_failed_with_reason_none() {
        let (mut h, mut op) = setup(false, L::Above, false);
        observe_visibility(&mut op, &mut h).unwrap();
        op.stage = Stage::Layer;
        op.detail.stage = Stage::Layer;
        h.current.layer = L::Normal;
        let result = finish_bring_top_with(&mut op, &mut h, Reason::None, true).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.detail().reason, Reason::LayerChanged);
        assert_eq!(result.detail().layer, L::Normal);
        assert!(result.detail().progress.not_minimized_confirmed);
        assert!(!result.detail().progress.above_confirmed);
    }
    #[test]
    fn local_layer_deadline_route_is_excluded_only_for_remote_layer_phase() {
        let (mut h, mut op) = setup(false, L::Normal, false);
        assert!(local_layer_service_allowed(None));
        assert!(local_layer_service_allowed(Some(&op)));
        h.layer_mode = 1;
        assert!(run(&mut h, &mut op).is_none());
        assert!(!local_layer_service_allowed(Some(&op)));
        assert_eq!(remote_operation_deadline(&op), h.layer_deadline().unwrap());
        assert_ne!(
            remote_operation_deadline(&op),
            op.bring_top.visibility_cutoff
        );
    }
}
mod scheduling {
    use super::*;

    #[derive(Clone, Copy)]
    enum Queued {
        Request(control::Request),
        Ordinary(&'static str),
        Visible,
        Minimized,
        Observed(Reading),
    }

    #[derive(Default)]
    struct Owner {
        active: Option<ActiveOperation>,
        replies: Vec<(Command, Status, Reason, Instant)>,
    }
    impl Owner {
        // Admission is injected at the validated transport boundary. The
        // scheduling seam and Show readiness/deadline logic are production code.
        fn route(&mut self, host: &mut Fake, event: Queued) -> Result<bool, HostError> {
            let Queued::Request(request) = event else {
                return Ok(false);
            };
            host.step("request")?;
            match control::admit(
                &mut self.active,
                request,
                crate::control::NO_DESTINATION,
                Lifecycle::Ready,
                false,
                host.now,
            ) {
                control::Admission::Admitted => {
                    host.step("admit")?;
                    let op = self.active.as_mut().unwrap();
                    op.stage = if request.command == Command::Hide {
                        control::Stage::Hide
                    } else {
                        control::Stage::Show
                    };
                    op.detail = control::preflight_detail(Reason::None, op.stage);
                }
                control::Admission::Duplicate => host.step("duplicate")?,
                control::Admission::Reply(status, reason, _) => {
                    self.replies
                        .push((request.command, status, reason, host.now));
                }
            }
            Ok(true)
        }
        fn service(&mut self, host: &mut Fake) -> Result<(), HostError> {
            let Some(op) = self.active.as_mut() else {
                return Ok(());
            };
            if !remote_service_due(op, host.now) {
                return Ok(());
            }
            if let Some(result) = service_remote_with(op, host)? {
                self.replies.push((
                    op.request.command,
                    result.status(),
                    result.detail().reason,
                    host.now,
                ));
                self.active = None;
            }
            Ok(())
        }
    }

    struct Loop {
        owner: Owner,
        host: Fake,
        waits: Vec<Option<Instant>>,
        event_cost: Duration,
    }
    impl Loop {
        fn new() -> Self {
            Self {
                owner: Owner::default(),
                host: minimized(),
                waits: vec![],
                event_cost: Duration::ZERO,
            }
        }
        fn route(&mut self, event: Queued) -> Result<bool, HostError> {
            route_owner_request_with(
                &mut self.owner,
                &mut self.host,
                |owner, host| owner.route(host, event),
                Owner::service,
            )
        }
        // One outer iteration, including the real bounded drain, both
        // routing sites, and an injected socket wait (no X server or sleeps).
        fn iteration(&mut self, queue: &mut VecDeque<Queued>) -> Result<(), HostError> {
            self.owner.service(&mut self.host)?;
            for event in
                drain_events_bounded(&mut None, 64, || Ok::<_, HostError>(queue.pop_front()))?
            {
                if self.route(event)? {
                    continue;
                }
                if self
                    .owner
                    .active
                    .as_ref()
                    .is_some_and(|op| self.host.now >= remote_operation_deadline(op))
                {
                    self.owner.service(&mut self.host)?;
                }
                match event {
                    Queued::Ordinary(name) => self.host.step(name)?,
                    Queued::Visible | Queued::Minimized => {
                        self.host.current = if matches!(event, Queued::Visible) {
                            visible()
                        } else {
                            reading(Visibility::Minimized, L::Normal, 2, 2)
                        };
                        if let Some(op) = self.owner.active.as_mut() {
                            mark_remote_observation(op, false);
                        }
                    }
                    Queued::Observed(reading) => {
                        self.host.current = reading;
                        if let Some(op) = self.owner.active.as_mut() {
                            mark_remote_observation(op, false);
                        }
                    }
                    Queued::Request(_) => unreachable!("control event consumed"),
                }
                self.host.now += self.event_cost;
            }
            self.owner.service(&mut self.host)?;
            if let Some(event) = queue.pop_front() {
                if !self.route(event)? {
                    queue.push_front(event); // one-event lookahead retained
                }
                return Ok(()); // lookahead continues the outer loop
            }
            let deadline = probe_deadline(&self.host.runtime, None)
                .into_iter()
                .chain(self.owner.active.as_ref().map(remote_operation_deadline))
                .min();
            self.host.step("wait")?;
            self.waits.push(deadline);
            Ok(())
        }
    }

    fn request(command: Command, id: u64) -> Queued {
        Queued::Request(control::Request {
            command,
            request_id: id,
            reply_xid: 88,
            epoch: 7,
        })
    }

    fn off_workspace_loop(command: Command) -> Loop {
        let mut s = Loop::new();
        s.host.current = reading(
            if command == Command::Hide {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::Minimized
            },
            L::Normal,
            7,
            2,
        );
        s
    }

    #[test]
    fn g_more_than_64_events_before_visibility_or_layer_evidence_finish_without_wait() {
        for layer_phase in [false, true] {
            let mut s = Loop::new();
            if layer_phase {
                s.host.current = visible();
                s.host.layer_mode = 1;
            }
            s.route(request(Command::BringTop, 99)).unwrap();
            s.event_cost = Duration::from_millis(1);
            let mut q = VecDeque::from(vec![Queued::Ordinary("traffic"); 130]);
            q.push_back(Queued::Observed(reading(
                Visibility::NotMinimizedHere,
                if layer_phase { L::Above } else { L::Normal },
                2,
                2,
            )));
            while s.owner.active.is_some() {
                s.iteration(&mut q).unwrap();
            }
            assert_eq!(
                s.owner.replies,
                [(Command::BringTop, Status::Success, Reason::None, s.host.now)]
            );
            assert_eq!(s.host.count("traffic"), 130);
            assert_eq!(s.host.count("map"), usize::from(!layer_phase));
            assert_eq!(s.host.count("add_above"), 1);
            assert_eq!(s.waits, [None]);
        }
    }
    #[test]
    fn sustained_g_traffic_cannot_starve_visibility_layer_or_total_deadlines() {
        for phase in 0..3 {
            let mut s = Loop::new();
            let t0 = s.host.now;
            if phase > 0 {
                s.host.current = visible();
                s.host.layer_mode = 1;
            }
            if phase == 2 {
                // Adversarial late Layer admission exercises the total cap
                // through the production adapter, with no controller renewal.
                let mut op = s.host.command_operation(Command::BringTop);
                observe_visibility(&mut op, &mut s.host).unwrap();
                s.host.now = t0 + Duration::from_millis(3000);
                op.stage = control::Stage::Layer;
                op.detail.stage = control::Stage::Layer;
                op.bring_top.layer_admitted = Some(s.host.now);
                op.bring_top.layer_observe = true;
                s.owner.active = Some(op);
                s.owner.service(&mut s.host).unwrap();
            } else {
                s.route(request(Command::BringTop, 99)).unwrap();
            }
            let expected = if phase == 0 {
                2500
            } else if phase == 1 {
                1000
            } else {
                3500
            };
            s.event_cost = Duration::from_millis(if phase == 2 { 1 } else { 10 });
            let mut q = VecDeque::from(vec![Queued::Ordinary("traffic"); 1000]);
            while s.owner.active.is_some() {
                s.iteration(&mut q).unwrap();
            }
            assert_eq!(
                s.owner.replies,
                [(
                    Command::BringTop,
                    Status::Failed,
                    Reason::ConfirmationTimeout,
                    t0 + Duration::from_millis(expected)
                )]
            );
            assert!(!q.is_empty());
            assert!(s.waits.is_empty());
            assert_eq!(s.host.count("map"), usize::from(phase == 0));
            assert_eq!(s.host.count("add_above"), usize::from(phase > 0));
        }
    }
    #[test]
    fn g_wait_uses_earliest_movement_pointer_or_remote_deadline_without_idle_reads() {
        let mut s = Loop::new();
        s.host.current = visible();
        s.host.layer_mode = 1;
        let t0 = s.host.now;
        s.route(request(Command::BringTop, 99)).unwrap();
        let reads = s.host.count("observe");
        s.host.runtime.confirmation_deadline = Some(t0 + Duration::from_millis(150));
        s.host.runtime.safety.deadline = Some(t0 + Duration::from_millis(250));
        s.iteration(&mut VecDeque::new()).unwrap();
        assert_eq!(s.waits.last(), Some(&Some(t0 + Duration::from_millis(150))));
        s.host.runtime.confirmation_deadline = None;
        s.iteration(&mut VecDeque::new()).unwrap();
        assert_eq!(s.waits.last(), Some(&Some(t0 + Duration::from_millis(250))));
        s.host.runtime.safety.deadline = None;
        s.iteration(&mut VecDeque::new()).unwrap();
        assert_eq!(s.waits.last(), Some(&s.host.layer_deadline()));
        assert_eq!(s.host.count("observe"), reads);
        assert_eq!(s.host.count("add_above"), 1);
    }
    #[test]
    fn checked_restore_with_buffered_visibility_event_is_serviced_before_wait() {
        let mut s = Loop::new();
        let visible = visible();
        s.host.changes.push_back(("map", visible));
        s.route(request(Command::BringTop, 99)).unwrap();
        assert_eq!(s.host.count("map"), 1);
        assert!(s.owner.replies.is_empty());
        // The checked request buffered its notification; drain the real
        // bounded-batch seam rather than inventing another wake event.
        s.iteration(&mut VecDeque::from([Queued::Observed(visible)]))
            .unwrap();
        assert_eq!(s.owner.replies[0].1, Status::Success);
        assert_eq!(s.host.count("map"), 1);
        assert_eq!(s.waits, [None]);
    }
    #[test]
    fn all_remote_pairings_progress_original_only_and_keep_duplicate_deadlines() {
        for first in [Command::Hide, Command::Show, Command::BringTop] {
            for second in [Command::Hide, Command::Show, Command::BringTop] {
                let mut s = Loop::new();
                if first == Command::Hide {
                    s.host.current = visible();
                }
                s.route(request(first, 99)).unwrap();
                let original = s.owner.active.unwrap();
                let maps = s.host.count("map");
                let hides = s.host.count("iconify");
                let mutations = s.host.mutations.clone();
                s.host.now += Duration::from_millis(20);
                s.route(request(first, 99)).unwrap();
                s.route(request(second, 100)).unwrap();
                assert_eq!(s.owner.active, Some(original));
                assert_eq!(s.host.count("map"), maps);
                assert_eq!(s.host.count("iconify"), hides);
                assert_eq!(s.host.mutations, mutations);
                assert_eq!(s.owner.replies.len(), 1);
                assert_eq!(s.owner.replies[0].1, Status::Busy);
                s.host.current = reading(
                    if first == Command::Hide {
                        Visibility::Minimized
                    } else {
                        Visibility::NotMinimizedHere
                    },
                    L::Normal,
                    2,
                    2,
                );
                mark_remote_observation(s.owner.active.as_mut().unwrap(), false);
                s.owner.service(&mut s.host).unwrap();
                s.owner.service(&mut s.host).unwrap();
                assert_eq!(s.owner.replies.len(), 2);
                assert_eq!(s.owner.replies[1].0, first);
                assert_eq!(s.owner.replies[1].1, Status::Success);
                assert!(s.owner.active.is_none());
            }
        }
    }
    #[test]
    fn bring_top_lookahead_begins_before_outer_continue() {
        let mut s = Loop::new();
        let mut queue = VecDeque::from(vec![Queued::Ordinary("before"); 64]);
        queue.push_back(request(Command::BringTop, 99));
        s.iteration(&mut queue).unwrap();
        assert_eq!(s.host.count("map"), 1);
        assert!(s.waits.is_empty());
        assert_eq!(s.host.log.last(), Some(&"map"));
        assert_eq!(s.owner.active.unwrap().request.command, Command::BringTop);
    }
    #[test]
    fn bring_top_visibility_ready_and_immediate_layer_complete_in_same_service() {
        for layer in [L::Normal, L::Below, L::Above] {
            let mut s = Loop::new();
            s.host.current.layer = layer;
            s.iteration(&mut VecDeque::from([
                request(Command::BringTop, 99),
                Queued::Observed(reading(Visibility::NotMinimizedHere, layer, 2, 2)),
            ]))
            .unwrap();
            assert!(s.owner.active.is_none());
            assert_eq!(
                s.owner.replies,
                [(Command::BringTop, Status::Success, Reason::None, s.host.now)]
            );
            assert_eq!(s.waits, [None]);
            assert_eq!(s.host.count("map"), 1);
            assert_eq!(s.host.count("enable"), 1);
            assert_eq!(s.host.count("add_above"), usize::from(layer != L::Above));
        }
    }
    #[test]
    fn already_ready_g_layer_work_needs_no_future_event_or_poll() {
        for layer in [L::Normal, L::Below, L::Above] {
            let mut s = Loop::new();
            s.host.current = reading(Visibility::NotMinimizedHere, layer, 2, 2);
            s.iteration(&mut VecDeque::from([request(Command::BringTop, 99)]))
                .unwrap();
            assert!(s.owner.active.is_none());
            assert_eq!(s.owner.replies[0].1, Status::Success);
            assert_eq!(s.waits, [None]);
            assert_eq!(s.host.count("fence"), 0);
            let count = s.host.count("observe");
            s.iteration(&mut VecDeque::new()).unwrap();
            assert_eq!(s.host.count("observe"), count);
        }
    }
    #[test]
    fn g_retains_identity_across_visibility_layer_and_competitors_are_busy() {
        let mut s = Loop::new();
        s.host.layer_mode = 1;
        let t0 = s.host.now;
        s.iteration(&mut VecDeque::from([
            request(Command::BringTop, 99),
            Queued::Visible,
        ]))
        .unwrap();
        let original = s.owner.active.unwrap();
        assert_eq!(original.request.command, Command::BringTop);
        assert_eq!(original.stage, control::Stage::Layer);
        assert_eq!(original.deadline, t0 + Duration::from_millis(3500));
        assert!(s.owner.replies.is_empty());
        s.host.now += Duration::from_millis(100);
        s.iteration(&mut VecDeque::from([
            request(Command::BringTop, 99),
            request(Command::BringTop, 100),
            request(Command::Hide, 101),
            request(Command::Show, 102),
        ]))
        .unwrap();
        assert_eq!(s.owner.active, Some(original));
        assert_eq!(s.host.count("duplicate"), 1);
        assert!(s.owner.replies.iter().all(|r| r.1 == Status::Busy));
        assert_eq!(s.host.count("map"), 1);
        assert_eq!(s.host.count("add_above"), 1);
        s.host.current.layer = L::Above;
        mark_remote_observation(s.owner.active.as_mut().unwrap(), false);
        s.owner.service(&mut s.host).unwrap();
        s.owner.service(&mut s.host).unwrap();
        assert_eq!(
            s.owner
                .replies
                .iter()
                .filter(|r| r.0 == Command::BringTop && r.1 == Status::Success)
                .count(),
            1
        );
    }
    #[test]
    fn last_batch_layer_evidence_completes_g_before_socket_wait() {
        let mut s = Loop::new();
        s.host.current = visible();
        s.host.layer_mode = 1;
        s.iteration(&mut VecDeque::from([request(Command::BringTop, 99)]))
            .unwrap();
        let mut queue = VecDeque::from(vec![Queued::Ordinary("traffic"); 63]);
        queue.push_back(Queued::Observed(reading(
            Visibility::NotMinimizedHere,
            L::Above,
            2,
            2,
        )));
        s.iteration(&mut queue).unwrap();
        assert!(s.owner.active.is_none());
        assert_eq!(s.owner.replies[0].1, Status::Success);
        assert_eq!(s.waits.last(), Some(&None));
        assert_eq!(s.host.count("add_above"), 1);
    }
    #[test]
    fn off_workspace_only_event_dispatches_hidden_request_before_socket_wait() {
        for command in [Command::Hide, Command::Show] {
            let mut s = off_workspace_loop(command);
            let admitted = s.host.now;
            s.iteration(&mut VecDeque::from([request(command, 99)]))
                .unwrap();
            let step = if command == Command::Hide {
                "add_hidden"
            } else {
                "remove_hidden"
            };
            assert_eq!(s.host.count(step), 1);
            let index = |step| s.host.log.iter().position(|s| *s == step).unwrap();
            assert!(index(step) < index("wait"));
            assert_eq!(
                s.waits,
                [Some(
                    admitted
                        + if command == Command::Hide {
                            Duration::from_millis(1000)
                        } else {
                            Duration::from_millis(2500)
                        }
                )]
            );
            assert!(s.owner.replies.is_empty());
            for step in ["iconify", "user_time_zero", "map", "geometry", "enable"] {
                assert_eq!(s.host.count(step), 0);
            }
        }
    }

    #[test]
    fn off_workspace_lookahead_dispatches_hidden_request_before_continue() {
        for command in [Command::Hide, Command::Show] {
            let mut s = off_workspace_loop(command);
            let mut queue = VecDeque::from(vec![Queued::Ordinary("before"); 64]);
            queue.push_back(request(command, 99));
            s.iteration(&mut queue).unwrap();
            let step = if command == Command::Hide {
                "add_hidden"
            } else {
                "remove_hidden"
            };
            assert_eq!(s.host.log.last(), Some(&step));
            assert_eq!(s.host.count(step), 1);
            assert!(s.waits.is_empty());
        }
    }

    #[test]
    fn off_workspace_sustained_traffic_cannot_starve_or_reset_fixed_deadlines() {
        for command in [Command::Hide, Command::Show] {
            let mut s = off_workspace_loop(command);
            let admitted = s.host.now;
            s.event_cost = Duration::from_millis(10);
            let mut queue = VecDeque::from([request(command, 99)]);
            queue.extend(vec![Queued::Ordinary("traffic"); 400]);
            while s.owner.replies.is_empty() {
                s.iteration(&mut queue).unwrap();
            }
            assert_eq!(
                s.owner.replies,
                [(
                    command,
                    Status::Failed,
                    Reason::ConfirmationTimeout,
                    admitted
                        + if command == Command::Hide {
                            Duration::from_millis(1000)
                        } else {
                            Duration::from_millis(2500)
                        }
                )]
            );
            assert!(s.host.count("traffic") > 64);
            assert!(!queue.is_empty());
            assert!(s.waits.is_empty());
            assert_eq!(
                s.host.count("add_hidden"),
                usize::from(command == Command::Hide)
            );
            assert_eq!(
                s.host.count("remove_hidden"),
                usize::from(command == Command::Show)
            );
            for step in ["iconify", "user_time_zero", "map", "geometry", "enable"] {
                assert_eq!(s.host.count(step), 0);
            }
        }
    }

    #[test]
    fn off_workspace_last_batch_notification_confirms_before_wait_without_restart() {
        for command in [Command::Hide, Command::Show] {
            let mut s = off_workspace_loop(command);
            let mut queue = VecDeque::from([request(command, 99)]);
            queue.extend(vec![Queued::Ordinary("traffic"); 62]);
            queue.push_back(Queued::Observed(reading(
                if command == Command::Hide {
                    Visibility::Minimized
                } else {
                    Visibility::NotMinimizedElsewhere
                },
                L::Normal,
                7,
                2,
            )));
            s.iteration(&mut queue).unwrap();
            assert_eq!(
                s.owner.replies,
                [(command, Status::Success, Reason::None, s.host.now)]
            );
            assert!(s.owner.active.is_none());
            assert_eq!(s.waits, [None]);
            assert_eq!(
                s.host.count("add_hidden"),
                usize::from(command == Command::Hide)
            );
            assert_eq!(
                s.host.count("remove_hidden"),
                usize::from(command == Command::Show)
            );
            let observations = s.host.count("observe");
            s.owner.service(&mut s.host).unwrap();
            assert_eq!(s.host.count("observe"), observations);
            assert_eq!(s.owner.replies.len(), 1);
        }
    }

    #[test]
    fn hide_only_event_fences_and_dispatches_before_wait_without_polling() {
        let mut s = Loop::new();
        s.host.current = visible();
        let admitted = s.host.now;
        s.iteration(&mut VecDeque::from([request(Command::Hide, 99)]))
            .unwrap();
        assert_eq!(
            s.host.log,
            ["request", "admit", "fence", "empty", "release", "observe", "iconify", "wait"]
        );
        assert_eq!(s.waits, [Some(admitted + Duration::from_millis(1000))]);
        assert_eq!(
            s.owner.active.unwrap().hide.phase,
            control::HidePhase::IconifyRequested
        );
        let reads = s.host.count("observe");
        s.iteration(&mut VecDeque::new()).unwrap();
        assert_eq!(s.host.count("observe"), reads);
        assert_eq!(s.host.count("iconify"), 1);
        assert_eq!(s.host.count("map"), 0);
    }
    #[test]
    fn hide_lookahead_dispatches_before_outer_continue_and_discards_pending_motion() {
        let mut s = Loop::new();
        s.host.current = visible();
        s.host.runtime.pending_move = Some(Point::new(200, 200));
        let mut queue = VecDeque::from(vec![Queued::Ordinary("before"); 64]);
        queue.push_back(request(Command::Hide, 99));
        s.iteration(&mut queue).unwrap();
        assert_eq!(s.host.count("before"), 64);
        assert_eq!(s.host.count("iconify"), 1);
        assert!(s.host.runtime.pending_move.is_none());
        assert!(s.waits.is_empty());
        assert_eq!(s.host.log.last(), Some(&"iconify"));
    }
    #[test]
    fn hide_notifications_at_end_of_batch_complete_before_wait_in_event_order() {
        let mut s = Loop::new();
        s.host.current = visible();
        let mut queue = VecDeque::from([
            Queued::Ordinary("before"),
            request(Command::Hide, 99),
            Queued::Ordinary("after"),
        ]);
        queue.extend(vec![Queued::Ordinary("traffic"); 60]);
        queue.push_back(Queued::Minimized);
        s.iteration(&mut queue).unwrap();
        assert!(s.owner.active.is_none());
        assert_eq!(s.owner.replies[0].0, Command::Hide);
        assert_eq!(s.owner.replies[0].1, Status::Success);
        assert_eq!(s.waits, [None]);
        assert_eq!(s.host.count("iconify"), 1);
        let index = |name| s.host.log.iter().position(|s| *s == name).unwrap();
        assert!(index("before") < index("iconify"));
        assert!(index("iconify") < index("after"));
        assert!(index("after") < index("wait"));
    }
    #[test]
    fn hide_duplicates_do_not_extend_deadline_or_retry_and_show_competes_as_busy() {
        let mut s = Loop::new();
        s.host.current = visible();
        let admitted = s.host.now;
        s.iteration(&mut VecDeque::from([
            request(Command::Hide, 99),
            request(Command::Hide, 99),
            request(Command::Show, 100),
        ]))
        .unwrap();
        assert_eq!(s.host.count("duplicate"), 1);
        assert_eq!(
            s.owner.replies,
            [(Command::Show, Status::Busy, Reason::Busy, admitted)]
        );
        assert_eq!(
            s.owner.active.unwrap().deadline,
            admitted + Duration::from_millis(1000)
        );
        assert_eq!(s.host.count("iconify"), 1);
        assert_eq!(s.host.count("map"), 0);
    }
    #[test]
    fn sustained_traffic_cannot_starve_hide_deadline_or_failed_input_restoration() {
        let mut s = Loop::new();
        s.host.current = visible();
        let admitted = s.host.now;
        s.event_cost = Duration::from_millis(10);
        let mut queue = VecDeque::from([request(Command::Hide, 99)]);
        queue.extend(vec![Queued::Ordinary("traffic"); 400]);
        while s.owner.replies.is_empty() {
            s.iteration(&mut queue).unwrap();
        }
        assert_eq!(
            s.owner.replies,
            [(
                Command::Hide,
                Status::Failed,
                Reason::ConfirmationTimeout,
                admitted + Duration::from_millis(1000)
            )]
        );
        assert!(s.host.count("traffic") > 64);
        assert!(!queue.is_empty());
        assert!(s.waits.is_empty());
        assert_eq!(s.host.count("iconify"), 1);
        assert!(s.host.runtime.accepts_input());
    }
    #[test]
    fn expected_unmap_and_state_notifications_schedule_hide_without_layout_abort() {
        let layout = x11::monitors::LayoutAtoms {
            number_of_desktops: 90,
            desktop_viewport: 91,
            workarea: 10,
            current_desktop: 11,
            wm_desktop: 12,
        };
        let visibility = x11::visibility::VisibilityAtoms {
            wm_state: 20,
            change_state: 21,
            state: 22,
            hidden: 23,
            above: 24,
            below: 25,
            desktop: 12,
            current_desktop: 11,
            supported: 26,
            user_time: 27,
        };
        for event in [
            Event::UnmapNotify(x11rb::protocol::xproto::UnmapNotifyEvent {
                window: 100,
                ..Default::default()
            }),
            Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
                window: 100,
                atom: 20,
                ..Default::default()
            }),
            Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
                window: 100,
                atom: 22,
                ..Default::default()
            }),
        ] {
            assert!(visibility_event(&event, 1, 100, &visibility, &layout));
            let h = Fake::new(visible());
            let mut op = h.command_operation(Command::Hide);
            op.hide.observe = false;
            mark_remote_observation(&mut op, visibility_layout_interruption(&event, 1, &layout));
            assert!(remote_service_due(&op, h.now));
            assert!(!op.hide.layout_interrupted);
        }
        for atom in [layout.number_of_desktops, layout.desktop_viewport] {
            let event = Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
                window: 1,
                atom,
                ..Default::default()
            });
            assert!(!visibility_event(&event, 1, 100, &visibility, &layout));
            assert!(!visibility_layout_interruption(&event, 1, &layout));
        }
    }

    #[test]
    fn only_show_event_starts_restore_before_socket_wait_without_another_event() {
        let mut scheduler = Loop::new();
        let admission = scheduler.host.now;
        assert!(scheduler.owner.active.is_none());
        let mut queue = VecDeque::from([request(Command::Show, 99)]);
        scheduler.iteration(&mut queue).unwrap();
        assert!(queue.is_empty());
        assert_eq!(scheduler.host.count("user_time_zero"), 1);
        assert_eq!(scheduler.host.count("map"), 1);
        assert_eq!(scheduler.host.now, admission);
        assert_eq!(
            scheduler.owner.active.unwrap().show.phase,
            ShowPhase::RestoreRequested
        );
        assert_eq!(
            scheduler.waits,
            [Some(admission + Duration::from_millis(2500))]
        );
        assert_eq!(scheduler.host.log.last(), Some(&"wait"));
    }

    #[test]
    fn early_show_preserves_events_before_and_after_it_in_the_drained_batch() {
        let mut scheduler = Loop::new();
        let mut queue = VecDeque::from([
            Queued::Ordinary("before_first"),
            Queued::Ordinary("before_second"),
            request(Command::Show, 99),
            Queued::Ordinary("after_first"),
            Queued::Ordinary("after_second"),
        ]);
        scheduler.iteration(&mut queue).unwrap();
        assert!(queue.is_empty());
        assert_eq!(
            scheduler.host.log,
            [
                "before_first",
                "before_second",
                "request",
                "admit",
                "observe",
                "fence",
                "empty",
                "release",
                "observe",
                "user_time_zero",
                "map",
                "after_first",
                "after_second",
                "wait",
            ]
        );
    }

    #[test]
    fn duplicate_does_not_retry_map_and_competing_request_is_busy() {
        let mut scheduler = Loop::new();
        let admission = scheduler.host.now;
        let mut queue = VecDeque::from([
            request(Command::Show, 99),
            request(Command::Show, 99),
            request(Command::Show, 100),
        ]);
        scheduler.iteration(&mut queue).unwrap();
        assert_eq!(scheduler.host.count("map"), 1);
        assert_eq!(scheduler.host.count("user_time_zero"), 1);
        assert_eq!(scheduler.host.count("duplicate"), 1);
        assert_eq!(scheduler.host.count("observe"), 2);
        assert_eq!(
            scheduler.owner.replies,
            [(Command::Show, Status::Busy, Reason::Busy, admission)]
        );
        let op = scheduler.owner.active.unwrap();
        assert_eq!(op.request.request_id, 99);
        assert_eq!(op.deadline, admission + Duration::from_millis(2500));
    }

    #[test]
    fn bring_top_request_only_batch_starts_restore_before_wait() {
        let mut scheduler = Loop::new();
        let admitted = scheduler.host.now;
        scheduler
            .iteration(&mut VecDeque::from([request(Command::BringTop, 2)]))
            .unwrap();
        let op = scheduler.owner.active.unwrap();
        assert_eq!(op.request.command, Command::BringTop);
        assert_eq!(op.deadline, admitted + Duration::from_millis(3500));
        assert_eq!(scheduler.host.count("map"), 1);
        assert!(scheduler.host.mutations.is_empty());
        assert_eq!(
            scheduler.waits,
            [Some(admitted + Duration::from_millis(2500))]
        );
        assert!(scheduler.owner.replies.is_empty());
    }

    #[test]
    fn lookahead_request_gets_the_same_immediate_progression_before_outer_continue() {
        let mut scheduler = Loop::new();
        let admission = scheduler.host.now;
        let mut queue = VecDeque::from(vec![Queued::Ordinary("before"); 64]);
        queue.push_back(request(Command::Show, 99));
        scheduler.iteration(&mut queue).unwrap();
        assert!(queue.is_empty());
        assert_eq!(scheduler.host.count("before"), 64);
        assert_eq!(scheduler.host.count("map"), 1);
        assert_eq!(scheduler.host.now, admission);
        assert!(scheduler.waits.is_empty()); // lookahead continues, after servicing
        assert_eq!(scheduler.host.log.last(), Some(&"map"));
        scheduler.iteration(&mut queue).unwrap();
        assert_eq!(
            scheduler.waits,
            [Some(admission + Duration::from_millis(2500))]
        );
        assert_eq!(scheduler.host.count("map"), 1);
    }

    #[test]
    fn idle_and_pending_show_reach_socket_wait_without_repeated_work_or_spin() {
        let mut scheduler = Loop::new();
        scheduler.iteration(&mut VecDeque::new()).unwrap();
        assert_eq!(scheduler.waits, [None]);
        assert_eq!(scheduler.host.log, ["wait"]);
        scheduler
            .iteration(&mut VecDeque::from([request(Command::Show, 99)]))
            .unwrap();
        let observations = scheduler.host.count("observe");
        let deadline = scheduler.owner.active.unwrap().deadline;
        scheduler.iteration(&mut VecDeque::new()).unwrap();
        assert_eq!(scheduler.waits, [None, Some(deadline), Some(deadline)]);
        assert_eq!(scheduler.host.count("observe"), observations);
        assert_eq!(scheduler.host.count("map"), 1);
        assert!(!remote_service_due(
            scheduler.owner.active.as_ref().unwrap(),
            scheduler.host.now
        ));
    }

    #[test]
    fn last_batch_notification_finishes_show_before_socket_wait() {
        let mut scheduler = Loop::new();
        scheduler
            .iteration(&mut VecDeque::from([
                request(Command::Show, 99),
                Queued::Visible,
            ]))
            .unwrap();
        assert!(scheduler.owner.active.is_none());
        assert_eq!(scheduler.owner.replies[0].1, Status::Success);
        assert_eq!(scheduler.host.count("map"), 1);
        assert_eq!(scheduler.host.count("enable"), 1);
        assert_eq!(scheduler.waits, [None]);
    }

    #[test]
    fn correction_wait_uses_150_ms_and_wakes_without_another_event() {
        let mut scheduler = Loop::new();
        let admission = scheduler.host.now;
        scheduler.host.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        scheduler
            .iteration(&mut VecDeque::from([
                request(Command::Show, 99),
                Queued::Visible,
            ]))
            .unwrap();
        let op = scheduler.owner.active.unwrap();
        let confirmation = admission + Duration::from_millis(150);
        assert_eq!(op.show.correction.unwrap().1, confirmation);
        assert_eq!(scheduler.waits, [Some(confirmation)]);
        assert_eq!(scheduler.host.count("enable"), 0);
        scheduler.host.now = confirmation - Duration::from_millis(1);
        assert!(!remote_service_due(&op, scheduler.host.now));
        scheduler.host.now = confirmation;
        scheduler.iteration(&mut VecDeque::new()).unwrap();
        assert!(scheduler.owner.active.is_none());
        assert_eq!(scheduler.owner.replies[0].1, Status::Success);
        assert_eq!(scheduler.host.count("correct"), 1);
        assert_eq!(scheduler.host.count("enable"), 1);
        assert_eq!(scheduler.host.count("map"), 1);
        assert_eq!(scheduler.waits.last(), Some(&None));
    }

    #[test]
    fn sustained_batches_cannot_starve_fixed_remote_deadline() {
        let mut scheduler = Loop::new();
        let admission = scheduler.host.now;
        scheduler.event_cost = Duration::from_millis(10);
        let mut queue = VecDeque::from([request(Command::Show, 99)]);
        queue.extend(vec![Queued::Ordinary("traffic"); 400]);
        while scheduler.owner.replies.is_empty() {
            scheduler.iteration(&mut queue).unwrap();
        }
        assert_eq!(
            scheduler.owner.replies,
            [(
                Command::Show,
                Status::Failed,
                Reason::ConfirmationTimeout,
                admission + Duration::from_millis(2500)
            )]
        );
        assert!(!queue.is_empty());
        assert!(scheduler.host.count("traffic") > 64);
        assert!(scheduler.waits.is_empty());
        assert_eq!(scheduler.host.count("user_time_zero"), 1);
        assert_eq!(scheduler.host.count("map"), 1);
        assert_eq!(scheduler.host.count("correct"), 0);
        assert_eq!(scheduler.host.count("enable"), 0);
    }
}

#[test]
fn acknowledged_map_is_pending_until_fresh_wm_geometry_and_input_confirmation() {
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    assert_eq!(
        h.log,
        [
            "observe",
            "fence",
            "empty",
            "release",
            "observe",
            "user_time_zero",
            "map"
        ]
    );
    assert_eq!(op.show.phase, ShowPhase::RestoreRequested);
    assert!(!op.detail.progress.not_minimized_confirmed);
    assert!(!op.detail.progress.input_ready);
    assert!(h.run(&mut op).is_none()); // notification is not proof either
    assert_eq!(h.count("map"), 1);
    h.current = visible();
    assert_success(h.run(&mut op).unwrap(), true, true);
    assert_eq!(h.count("map"), 1);
    assert_eq!(h.count("user_time_zero"), 1);
    assert!(
        h.log.iter().position(|s| *s == "geometry").unwrap()
            < h.log.iter().position(|s| *s == "enable").unwrap()
    );
    assert_eq!(h.runtime.input_fence.enabled_after, Some(4 * 65536 + 123));
    assert_eq!(h.runtime.pending_map_input, None);
}
#[test]
fn fresh_above_normal_below_and_all_desktops_are_preserved_for_restore_and_noop() {
    for layer in [L::Above, L::Normal, L::Below] {
        for desktop in [2, u32::MAX] {
            for restore in [false, true] {
                let mut h = Fake::new(reading(
                    if restore {
                        Visibility::Minimized
                    } else {
                        Visibility::NotMinimizedHere
                    },
                    layer,
                    desktop,
                    2,
                ));
                let mut op = h.operation();
                if restore {
                    assert!(h.run(&mut op).is_none());
                    h.current = reading(Visibility::NotMinimizedHere, layer, desktop, 2);
                }
                let result = h.run(&mut op).unwrap();
                assert_success(result, true, restore);
                assert_eq!(op.show.baseline.unwrap().desktop, desktop);
                assert_eq!(result.detail().layer, layer);
                assert_eq!(h.count("map"), usize::from(restore));
            }
        }
    }
}
#[test]
fn local_noop_preserves_idle_drag_opening_and_popup_without_shape_or_release() {
    for gesture in 0..=4 {
        let mut h = Fake::new(visible());
        h.runtime = visibility_fence_tests::runtime(gesture);
        let state = h.runtime.interaction.state();
        let owner = h.runtime.pointer.owner();
        let mut op = h.operation();
        assert_success(h.run(&mut op).unwrap(), true, false);
        assert_eq!(h.runtime.interaction.state(), state);
        assert_eq!(h.runtime.pointer.owner(), owner);
        for step in [
            "fence", "empty", "release", "popup", "map", "enable", "correct",
        ] {
            assert_eq!(h.count(step), 0);
        }
    }
}
#[test]
fn off_workspace_iconic_and_normal_nonviewable_noops_never_map_or_query_geometry() {
    for (wm, map) in [
        (WmState::Iconic, MapState::Unmapped),
        (WmState::Normal, MapState::Unmapped),
        (WmState::Normal, MapState::Unviewable),
    ] {
        let mut r = reading(Visibility::NotMinimizedElsewhere, L::Below, 7, 2);
        r.evidence.wm_state = wm;
        r.evidence.map_state = Evidence::Readable(map);
        let mut h = Fake::new(r);
        let mut op = h.operation();
        assert_success(h.run(&mut op).unwrap(), false, false);
        assert_eq!(h.count("map"), 0);
        assert_eq!(h.count("remove_hidden"), 0);
        assert_eq!(h.count("geometry"), 0);
        assert_eq!(h.count("correct"), 0);
        assert!(h.runtime.input_fence.blocked);
        assert!(!h.runtime.accepts_input());
    }
}
#[test]
fn actually_minimized_elsewhere_can_finish_iconic_unmapped_without_local_normal_transition() {
    let mut h = Fake::new(reading(Visibility::Minimized, L::Above, 7, 2));
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    assert_eq!(op.show.phase, ShowPhase::HiddenRemoveRequested);
    assert_eq!(h.count("remove_hidden"), 1);
    assert_eq!(h.count("user_time_zero"), 0);
    assert_eq!(h.count("map"), 0);
    assert_eq!(h.runtime.pending_map_input, None);
    assert!(h.run(&mut op).is_none()); // checked REMOVE is not confirmation
    h.current = reading(Visibility::NotMinimizedElsewhere, L::Above, 7, 2);
    assert_success(h.run(&mut op).unwrap(), false, true);
    assert_eq!(h.count("geometry"), 0);
    assert_eq!(h.count("enable"), 0);
    assert_eq!(h.count("correct"), 0);
    assert_eq!(h.count("remove_hidden"), 1);
    assert!(h.runtime.input_fence.blocked);
    assert!(!h.runtime.accepts_input());
    assert_eq!(op.show.baseline.unwrap().desktop, 7);
}

#[test]
fn off_workspace_remove_refusal_times_out_without_retry_map_or_input() {
    let mut h = Fake::new(reading(Visibility::Minimized, L::Below, 7, 2));
    let mut op = h.operation();
    let deadline = op.deadline;
    assert!(h.run(&mut op).is_none());
    for _ in 0..3 {
        mark_remote_observation(&mut op, false);
        assert!(h.run(&mut op).is_none());
        assert_eq!(op.deadline, deadline);
    }
    h.now = deadline;
    let result = h.run(&mut op).unwrap();
    assert_eq!(result.status(), Status::Failed);
    assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
    assert_eq!(h.count("remove_hidden"), 1);
    for step in ["user_time_zero", "map", "geometry", "correct", "enable"] {
        assert_eq!(h.count(step), 0);
    }
    assert!(h.runtime.input_fence.blocked);
}

#[test]
fn off_workspace_show_needs_fresh_final_no_hidden_and_never_arms_map_shape_sync() {
    let mut h = Fake::new(reading(Visibility::Minimized, L::Above, 7, 2));
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    let remote = reading(Visibility::NotMinimizedElsewhere, L::Above, 7, 2);
    h.reads = VecDeque::from([remote, h.current]);
    assert!(h.run(&mut op).is_none()); // HIDDEN returned on the final fresh read
    assert!(!op.detail.progress.not_minimized_confirmed);
    assert_eq!(h.count("remove_hidden"), 1);
    let event = MapNotifyEvent {
        response_type: x11rb::protocol::xproto::MAP_NOTIFY_EVENT,
        event: h.window.window,
        window: h.window.window,
        ..Default::default()
    };
    h.runtime
        .synchronize_map_input_with(&event, h.window.window, 70001, || {
            panic!("REMOVE-HIDDEN must not arm post-map SHAPE or enable remote input")
        })
        .unwrap();
    assert_eq!(h.runtime.pending_map_input, None);
    assert!(h.runtime.input_fence.blocked);
    // Another approved coherent representation may finish without mapping.
    h.current = remote;
    h.current.evidence.wm_state = WmState::Normal;
    h.current.evidence.map_state = Evidence::Readable(MapState::Unviewable);
    assert_success(h.run(&mut op).unwrap(), false, true);
    for step in ["map", "user_time_zero", "geometry", "enable"] {
        assert_eq!(h.count(step), 0);
    }
}

#[test]
fn off_workspace_checked_failures_consume_distinct_attempts_without_retry() {
    for command in [Command::Hide, Command::Show] {
        let (visibility, step) = if command == Command::Hide {
            (Visibility::NotMinimizedElsewhere, "add_hidden")
        } else {
            (Visibility::Minimized, "remove_hidden")
        };
        let mut h = Fake::new(reading(visibility, L::Normal, 7, 2));
        h.fail = Some(step);
        let mut op = h.command_operation(command);
        let deadline = op.deadline;
        assert!(service_visibility_with(&mut op, &mut h).is_err());
        if command == Command::Hide {
            assert_eq!(op.hide.phase, control::HidePhase::HiddenAddAttempted);
        } else {
            assert_eq!(op.show.phase, ShowPhase::HiddenRemoveAttempted);
        }
        assert!(op.detail.progress.host_mutation_dispatched);
        h.fail = None;
        assert!(service_visibility_with(&mut op, &mut h).unwrap().is_none());
        assert_eq!(h.count(step), 1);
        assert_eq!(op.deadline, deadline);
        for step in ["iconify", "user_time_zero", "map", "geometry", "enable"] {
            assert_eq!(h.count(step), 0);
        }
    }
}

#[test]
fn off_workspace_workspace_current_and_layer_drift_fail_without_compensation() {
    for command in [Command::Hide, Command::Show] {
        for case in 0..3 {
            let before = if command == Command::Hide {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::Minimized
            };
            let after = if command == Command::Hide {
                Visibility::Minimized
            } else {
                Visibility::NotMinimizedElsewhere
            };
            let mut h = Fake::new(reading(before, L::Above, 7, 2));
            let mut op = h.command_operation(command);
            assert!(service_visibility_with(&mut op, &mut h).unwrap().is_none());
            h.current = match case {
                0 => reading(after, L::Above, 8, 2),
                1 => reading(after, L::Above, 7, 3),
                _ => reading(after, L::Below, 7, 2),
            };
            let result = service_visibility_with(&mut op, &mut h).unwrap().unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(
                result.detail().reason,
                if case == 2 {
                    Reason::LayerChanged
                } else {
                    Reason::LayoutChanged
                }
            );
            assert_eq!(h.count("add_hidden"), usize::from(command == Command::Hide));
            assert_eq!(
                h.count("remove_hidden"),
                usize::from(command == Command::Show)
            );
            for step in [
                "iconify",
                "user_time_zero",
                "map",
                "geometry",
                "correct",
                "enable",
            ] {
                assert_eq!(h.count(step), 0);
            }
        }
    }
}

#[test]
fn off_workspace_expiry_before_dispatch_never_sends_hidden_request() {
    for command in [Command::Hide, Command::Show] {
        for step in ["admission", "fence", "observe"] {
            let visibility = if command == Command::Hide {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::Minimized
            };
            let mut h = Fake::new(reading(visibility, L::Normal, 7, 2));
            let mut op = h.command_operation(command);
            if step == "admission" {
                h.now = op.deadline;
            } else {
                h.advance = Some((step, op.deadline.duration_since(h.now)));
            }
            let result = service_visibility_with(&mut op, &mut h).unwrap().unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
            assert!(!result.detail().progress.host_mutation_dispatched);
            for step in [
                "add_hidden",
                "remove_hidden",
                "iconify",
                "user_time_zero",
                "map",
                "enable",
            ] {
                assert_eq!(h.count(step), 0);
            }
        }
    }
}

#[test]
fn off_workspace_fence_cancels_every_gesture_before_hidden_dispatch() {
    for command in [Command::Hide, Command::Show] {
        for gesture in 0..5 {
            let visibility = if command == Command::Hide {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::Minimized
            };
            let mut h = Fake::new(reading(visibility, L::Normal, 7, 2));
            h.runtime = visibility_fence_tests::runtime(gesture);
            h.runtime.pending_move = Some(Point::new(200, 200));
            h.runtime.final_target = Some(Point::new(210, 210));
            let mut op = h.command_operation(command);
            assert!(service_visibility_with(&mut op, &mut h).unwrap().is_none());
            let request = if command == Command::Hide {
                "add_hidden"
            } else {
                "remove_hidden"
            };
            let index = |step| h.log.iter().position(|s| *s == step).unwrap();
            assert!(index("empty") < index("release"));
            assert!(index("release") < index(request));
            if gesture == 3 || gesture == 4 {
                assert!(index("popup") < index(request));
            }
            assert_eq!(h.count(request), 1);
            assert!(h.runtime.visibility_dispatch_eligible());
            assert!(h.runtime.input_fence.blocked);
            assert!(!h.runtime.pointer.is_grabbed());
            assert!(h.runtime.pending_move.is_none());
            assert!(h.runtime.final_target.is_none());
            assert_eq!(h.runtime.pending_map_input, None);
        }
    }
}

#[test]
fn off_workspace_fence_failure_blocks_both_hidden_requests() {
    for command in [Command::Hide, Command::Show] {
        for fail in ["empty", "release", "popup"] {
            let visibility = if command == Command::Hide {
                Visibility::NotMinimizedElsewhere
            } else {
                Visibility::Minimized
            };
            let mut h = Fake::new(reading(visibility, L::Normal, 7, 2));
            h.runtime = visibility_fence_tests::runtime(4);
            h.fail = Some(fail);
            let mut op = h.command_operation(command);
            assert!(service_visibility_with(&mut op, &mut h).is_err());
            assert_eq!(h.count("popup"), 1);
            assert_eq!(h.count("add_hidden"), 0);
            assert_eq!(h.count("remove_hidden"), 0);
            assert!(!op.detail.progress.host_mutation_dispatched);
            assert!(!h.runtime.visibility_dispatch_eligible());
        }
    }
}
#[test]
fn malformed_missing_unsupported_and_incoherent_evidence_never_dispatch() {
    for case in 0..8 {
        let mut h = minimized();
        match case {
            0 => h.current.evidence.wm_state = WmState::Malformed,
            1 => h.current.evidence.hidden = Evidence::Missing,
            2 => h.current.evidence.hidden = Evidence::Malformed,
            3 => h.current.evidence.capability = Capability::Unsupported,
            4 => h.current.evidence.workspace = Workspace::Unknown,
            5 => h.current.evidence.coherent = false,
            6 => h.current.evidence.hidden = Evidence::Readable(false),
            _ => h.current.evidence.capability = Capability::Unverifiable,
        }
        let mut op = h.operation();
        let first = h.run(&mut op);
        if case == 3 {
            assert_eq!(first.unwrap().detail().reason, Reason::Unsupported);
        } else {
            assert!(first.is_none());
            h.now = op.deadline;
            assert_eq!(
                h.run(&mut op).unwrap().detail().reason,
                Reason::ConfirmationTimeout
            );
        }
        assert_eq!(h.count("map"), 0);
        assert_eq!(h.count("fence"), 0);
    }
}
#[test]
fn withdrawn_destroyed_and_conflicting_layer_baseline_fail_without_mutation() {
    for (visibility, layer, reason) in [
        (Visibility::Withdrawn, L::Normal, Reason::Withdrawn),
        (Visibility::Destroyed, L::Normal, Reason::BodyDestroyed),
        (Visibility::Minimized, L::Conflict, Reason::Unverifiable),
        (Visibility::Minimized, L::Unknown, Reason::Unverifiable),
    ] {
        let mut h = Fake::new(reading(visibility, layer, 2, 2));
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.detail().reason, reason);
        assert_eq!(h.count("map"), 0);
        assert_eq!(h.count("fence"), 0);
    }
}
#[test]
fn layer_workspace_and_current_desktop_drift_fail_without_compensating_writes() {
    for case in 0..4 {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.current = visible();
        match case {
            0 => h.current.layer = L::Above,
            1 => {
                h.current.desktop = 7;
                h.current.evidence.workspace = Workspace::Other;
            }
            2 => {
                h.current.current = 3;
                h.current.evidence.workspace = Workspace::Other;
            }
            _ => h.current.layer = L::Conflict,
        }
        let result = h.run(&mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(
            result.detail().reason,
            if case == 0 || case == 3 {
                Reason::LayerChanged
            } else {
                Reason::LayoutChanged
            }
        );
        assert_eq!(h.count("map"), 1);
        assert_eq!(h.count("enable"), 0);
        assert_eq!(h.count("correct"), 0);
    }
}
#[test]
fn minimized_fence_settles_idle_stale_drag_automatic_right_and_popup_before_map() {
    for gesture in 0..=4 {
        let mut h = minimized();
        h.runtime = visibility_fence_tests::runtime(gesture);
        h.runtime.pending_move = Some(Point::new(120, 120));
        h.runtime.final_target = Some(Point::new(120, 120));
        h.runtime.confirmation_deadline = Some(h.now);
        h.window.in_flight_moves.push_back(Point::new(120, 120));
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        assert!(h.runtime.visibility_dispatch_eligible());
        assert!(h.runtime.menu.is_none());
        assert!(!h.runtime.pointer.is_grabbed());
        assert!(
            h.log.iter().position(|s| *s == "release").unwrap()
                < h.log.iter().position(|s| *s == "map").unwrap()
        );
        if gesture >= 3 {
            assert!(
                h.log.iter().position(|s| *s == "popup").unwrap()
                    < h.log.iter().position(|s| *s == "map").unwrap()
            );
        }
    }
}
#[test]
fn failed_release_empty_shape_or_popup_cleanup_blocks_map_and_preserves_debt() {
    for failure in ["empty", "release", "popup"] {
        let mut h = minimized();
        h.runtime = visibility_fence_tests::runtime(3);
        h.fail = Some(failure);
        let mut op = h.operation();
        let error = service_show_with(&mut op, &mut h).unwrap_err();
        assert!(error.to_string().contains(failure));
        assert_eq!(h.count("popup"), 1);
        assert_eq!(h.count("release"), 1);
        assert_eq!(h.count("map"), 0);
        assert!(h.runtime.input_fence.blocked);
        assert!(!h.runtime.visibility_dispatch_eligible());
        if failure == "release" {
            assert!(h.runtime.pointer.is_grabbed());
        }
    }
    // No tracked owner is not proof of no server automatic capture.
    let mut h = minimized();
    h.fail = Some("release");
    let mut op = h.operation();
    assert!(service_show_with(&mut op, &mut h).is_err());
    assert!(h.runtime.pointer.is_grabbed());
    assert_eq!(h.count("map"), 0);
}
#[test]
fn native_restore_error_is_fatal_and_never_retried_or_confirmed() {
    for failure in ["user_time_zero", "map"] {
        let mut h = minimized();
        h.fail = Some(failure);
        let mut op = h.operation();
        assert!(service_show_with(&mut op, &mut h).is_err());
        assert_eq!(op.show.phase, ShowPhase::RestoreAttempted);
        assert_eq!(op.detail.reason, Reason::X11Failure);
        assert!(!op.detail.progress.not_minimized_confirmed);
        assert!(op.detail.progress.host_mutation_dispatched);
        assert_eq!(h.count("user_time_zero"), 1);
        assert_eq!(h.count("map"), usize::from(failure == "map"));
        assert_eq!(h.count("enable"), 0);
        h.fail = None;
        assert!(h.run(&mut op).is_none());
        assert_eq!(h.count("user_time_zero"), 1);
        assert_eq!(h.count("map"), usize::from(failure == "map"));
    }
}
#[test]
fn changed_workarea_requires_one_correction_and_full_confirmation_before_enable() {
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.current = visible();
    h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
    h.area = Rect::new(200, 200, 460, 460);
    assert!(h.run(&mut op).is_none());
    assert_eq!(h.count("correct"), 1);
    assert_eq!(h.count("enable"), 0);
    let (_, confirmation) = op.show.correction.unwrap();
    h.now = confirmation - Duration::from_millis(1);
    assert!(h.run(&mut op).is_none());
    h.now = confirmation;
    assert_success(h.run(&mut op).unwrap(), true, true);
    assert_eq!(h.count("correct"), 1);
    assert_eq!(h.geometry.origin(), Point::new(200, 200));
}
#[test]
fn insufficient_budget_never_begins_correction_or_enables_input() {
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.current = visible();
    h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
    h.now = op.deadline - Duration::from_millis(149);
    assert_eq!(
        h.run(&mut op).unwrap().detail().reason,
        Reason::ConfirmationTimeout
    );
    assert_eq!(h.count("correct"), 0);
    assert_eq!(h.count("enable"), 0);
}
#[test]
fn refused_correction_removed_monitor_and_fixed_size_change_remain_fatal() {
    for case in 0..3 {
        let mut h = minimized();
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.current = visible();
        match case {
            0 => {
                h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
                h.refuse_correction = true;
                assert!(h.run(&mut op).is_none());
                h.now = op.show.correction.unwrap().1;
            }
            1 => h.monitor = 99,
            _ => h.geometry.width = 161,
        }
        assert!(service_show_with(&mut op, &mut h).is_err());
        assert_eq!(h.count("enable"), 0);
        assert!(h.count("correct") <= 1);
    }
}
#[test]
fn layout_drift_during_correction_invalidates_success_without_second_move() {
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.current = visible();
    h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
    assert!(h.run(&mut op).is_none());
    h.area.x += 1;
    h.now = op.show.correction.unwrap().1;
    assert_eq!(
        h.run(&mut op).unwrap().detail().reason,
        Reason::LayoutChanged
    );
    assert_eq!(h.count("correct"), 1);
    assert_eq!(h.count("enable"), 0);
}
#[test]
fn pending_local_placement_is_busy_before_show_effects() {
    let mut h = minimized();
    h.runtime.correction = Some((Point::new(100, 100), h.now + CONFIRMATION_TIMEOUT));
    let mut op = h.operation();
    let result = h.run(&mut op).unwrap();
    assert_eq!(result.status(), Status::Busy);
    assert_eq!(result.exit_code(), control::ExitCode::Busy);
    assert_eq!(h.count("fence"), 0);
    assert_eq!(h.count("map"), 0);
}
#[test]
fn reenable_failure_never_reports_ready_and_retains_safe_fence() {
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.current = visible();
    h.fail = Some("shape");
    assert!(service_show_with(&mut op, &mut h).is_err());
    assert!(h.runtime.input_fence.blocked);
    assert!(!h.runtime.input_fence.empty_confirmed);
    assert!(!op.detail.progress.input_ready);
    assert_eq!(h.count("map"), 1);
}
#[test]
fn repeated_events_keep_single_deadline_and_final_expiry_observation_cannot_mutate() {
    let mut h = minimized();
    let mut op = h.operation();
    let deadline = op.deadline;
    assert!(h.run(&mut op).is_none());
    for _ in 0..62 {
        h.now += Duration::from_millis(40);
        op.show.observe = true;
        assert!(h.run(&mut op).is_none());
        assert_eq!(op.deadline, deadline);
    }
    h.now = deadline;
    let before = h.log.len();
    let result = h.run(&mut op).unwrap();
    assert_eq!(result.status(), Status::Failed);
    assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
    assert_eq!(&h.log[before..], ["observe"]);
    assert_eq!(h.count("map"), 1);
}
#[test]
fn expiry_before_restore_admission_effects_is_observation_only() {
    let mut h = minimized();
    let mut op = h.operation();
    h.now = op.deadline;
    let result = h.run(&mut op).unwrap();
    assert_eq!(result.status(), Status::Failed);
    assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
    assert!(!result.detail().progress.host_mutation_dispatched);
    assert_eq!(op.show.phase, ShowPhase::Observe);
    assert_eq!(h.log, ["observe"]);
    assert_eq!(h.count("user_time_zero"), 0);
    assert_eq!(h.count("map"), 0);
}
#[test]
fn expiry_between_user_time_and_map_never_claims_restore_request() {
    let mut h = minimized();
    let mut op = h.operation();
    h.advance = Some(("user_time_zero", op.deadline - h.now));
    let result = h.run(&mut op).unwrap();
    assert_eq!(result.status(), Status::Failed);
    assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
    assert!(result.detail().progress.host_mutation_dispatched);
    assert_eq!(op.show.phase, ShowPhase::RestoreAttempted);
    assert_eq!(h.count("user_time_zero"), 1);
    assert_eq!(h.count("map"), 0);
    assert_eq!(
        h.log,
        [
            "observe",
            "fence",
            "empty",
            "release",
            "observe",
            "user_time_zero",
            "observe"
        ]
    );
    assert!(h.runtime.input_fence.blocked);
    let before = h.log.len();
    op.show.observe = true;
    assert_eq!(h.run(&mut op).unwrap().status(), Status::Failed);
    assert_eq!(&h.log[before..], ["observe"]);
}
#[test]
fn expiry_after_remove_hidden_can_only_succeed_from_fresh_elsewhere_evidence() {
    let mut h = Fake::new(reading(Visibility::Minimized, L::Below, 7, 2));
    let mut op = h.operation();
    h.advance = Some(("remove_hidden", op.deadline - h.now));
    h.reads = VecDeque::from([
        h.current,
        h.current,
        reading(Visibility::NotMinimizedElsewhere, L::Below, 7, 2),
    ]);
    assert_success(h.run(&mut op).unwrap(), false, true);
    assert_eq!(op.show.phase, ShowPhase::HiddenRemoveRequested);
    assert_eq!(h.count("remove_hidden"), 1);
    assert_eq!(h.count("user_time_zero"), 0);
    assert_eq!(h.count("map"), 0);
    assert_eq!(h.count("enable"), 0);
    assert_eq!(h.count("geometry"), 0);
}
#[test]
fn slower_host_completes_map_correction_and_input_within_fixed_2500_ms() {
    assert_eq!(CONFIRMATION_TIMEOUT, Duration::from_millis(150));
    let mut h = minimized();
    let start = h.now;
    let mut op = h.operation();
    let deadline = start + Duration::from_millis(2500);
    assert_eq!(op.deadline, deadline);
    h.advance = Some(("user_time_zero", Duration::from_millis(1200)));
    assert!(h.run(&mut op).is_none());
    assert_eq!(h.now - start, Duration::from_millis(1200));
    assert_eq!(op.show.phase, ShowPhase::RestoreRequested);
    assert_eq!(h.count("map"), 1);
    h.now += Duration::from_millis(300); // WM response
    h.current = visible();
    h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
    h.advance = Some(("geometry", Duration::from_millis(300)));
    assert!(h.run(&mut op).is_none());
    let confirmation = op.show.correction.unwrap().1;
    assert_eq!(confirmation, start + Duration::from_millis(1950));
    h.now = confirmation;
    h.advance = Some(("enable", Duration::from_millis(300)));
    assert_success(h.run(&mut op).unwrap(), true, true);
    assert_eq!(h.now - start, Duration::from_millis(2250));
    assert_eq!(op.deadline, deadline);
    assert_eq!(op.show.phase, ShowPhase::InputRestored);
    assert_eq!(h.count("user_time_zero"), 1);
    assert_eq!(h.count("map"), 1);
    assert_eq!(h.count("correct"), 1);
    assert_eq!(h.count("enable"), 1);
    assert_eq!(h.log.last(), Some(&"observe"));
    assert!(h.runtime.accepts_input());
}
#[test]
fn just_over_2500_ms_never_dispatches_map_correction_or_input_late() {
    for step in ["user_time_zero", "geometry"] {
        let mut h = minimized();
        let start = h.now;
        let mut op = h.operation();
        if step == "geometry" {
            assert!(h.run(&mut op).is_none());
            h.current = visible();
            h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        }
        h.advance = Some((step, Duration::from_millis(2501)));
        let result = h.run(&mut op).unwrap();
        assert_eq!(h.now - start, Duration::from_millis(2501));
        assert_eq!(op.deadline, start + Duration::from_millis(2500));
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert!(result.detail().progress.host_mutation_dispatched);
        assert_eq!(h.count("map"), usize::from(step == "geometry"));
        assert_eq!(h.count("correct"), 0);
        assert_eq!(h.count("enable"), 0);
        assert_eq!(h.log.last(), Some(&"observe"));
    }
}
#[test]
fn expiry_during_user_time_or_geometry_uses_final_read_without_new_mutations() {
    for step in ["user_time_zero", "geometry"] {
        let mut h = minimized();
        let mut op = h.operation();
        if step == "geometry" {
            assert!(h.run(&mut op).is_none());
            h.current = visible();
        }
        h.advance = Some((step, op.deadline - h.now));
        let result = h.run(&mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert_eq!(h.log.last(), Some(&"observe"));
        assert_eq!(h.count("enable"), 0);
        assert_eq!(h.count("correct"), 0);
        assert_eq!(h.count("map"), usize::from(step == "geometry"));
    }
}
#[test]
fn final_read_at_expiry_can_confirm_existing_ready_noop_but_never_restore_input() {
    let mut h = Fake::new(visible());
    let mut op = h.operation();
    h.now = op.deadline;
    assert_success(h.run(&mut op).unwrap(), true, false);
    assert_eq!(h.count("enable"), 0);
    assert_eq!(h.count("fence"), 0);
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.current = visible();
    h.now = op.deadline;
    assert_eq!(h.run(&mut op).unwrap().status(), Status::Failed);
    assert_eq!(h.count("enable"), 0);
}
#[test]
fn buffered_traffic_over_64_events_cannot_starve_remote_deadline() {
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    let mut events = VecDeque::from(vec![(); 400]);
    let mut count = 0;
    let mut final_result = None;
    while !events.is_empty() {
        let batch =
            drain_events_bounded(&mut None, 64, || Ok::<_, HostError>(events.pop_front())).unwrap();
        for _ in batch {
            h.now += Duration::from_millis(10);
            count += 1;
            if h.now >= op.deadline {
                final_result = h.run(&mut op);
                break;
            }
        }
        if final_result.is_some() {
            break;
        }
    }
    assert_eq!(count, 250);
    assert!(!events.is_empty());
    assert_eq!(final_result.unwrap().status(), Status::Failed);
    assert_eq!(h.count("map"), 1);
}
#[test]
fn wm_delete_requires_body_protocols_format_and_protocol_atom() {
    let w = tests::test_window();
    let good = x11rb::protocol::xproto::ClientMessageEvent::new(
        32,
        w.window,
        w.wm_protocols,
        [w.wm_delete_window, 0, 0, 0, 0],
    );
    assert!(wm_delete_event(&good, &w));
    for case in 0..4 {
        let mut e = good;
        match case {
            0 => e.window += 1,
            1 => e.type_ += 1,
            2 => e.format = 8,
            _ => e.data = [w.wm_delete_window + 1, 0, 0, 0, 0].into(),
        }
        assert!(!wm_delete_event(&e, &w));
    }
}
#[test]
fn completed_elsewhere_show_defers_then_ordinary_workspace_return_validates_before_input() {
    for was_minimized in [false, true] {
        let mut h = Fake::new(reading(
            if was_minimized {
                Visibility::Minimized
            } else {
                Visibility::NotMinimizedElsewhere
            },
            L::Normal,
            7,
            2,
        ));
        let mut op = h.operation();
        if was_minimized {
            assert!(h.run(&mut op).is_none());
            h.current = reading(Visibility::NotMinimizedElsewhere, L::Normal, 7, 2);
        }
        assert_success(h.run(&mut op).unwrap(), false, was_minimized);
        assert_eq!(h.count("remove_hidden"), usize::from(was_minimized));
        assert_eq!(h.runtime.pending_map_input, None);
        let maps = h.count("map");
        restore_deferred_input(&mut h).unwrap();
        assert_eq!(h.count("geometry"), 0);
        // This is a later ordinary workspace event, with no active remote record.
        h.current = reading(Visibility::NotMinimizedHere, L::Normal, 7, 7);
        h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        restore_deferred_input(&mut h).unwrap();
        assert_eq!(h.count("ordinary_correct"), 1);
        assert_eq!(h.count("enable"), 0);
        restore_deferred_input(&mut h).unwrap();
        assert_eq!(h.count("ordinary_correct"), 1);
        let (_, deadline) = h.runtime.correction.unwrap();
        h.runtime
            .check_correction(deadline, h.geometry.origin(), &h.bounds)
            .unwrap();
        h.window.cancel_in_flight_moves();
        restore_deferred_input(&mut h).unwrap();
        assert_eq!(h.count("enable"), 1);
        assert!(h.runtime.accepts_input());
        assert_eq!(h.count("map"), maps);
        assert!(op.detail.progress.placement_deferred); // old terminal result is unchanged
        restore_deferred_input(&mut h).unwrap();
        assert_eq!(h.count("enable"), 1);
        assert_eq!(h.count("user_time_zero"), 0);
        assert_eq!(h.count("map"), 0);
        assert_eq!(h.count("remove_hidden"), usize::from(was_minimized));
    }
}
#[test]
fn show_map_notify_cannot_consume_old_startup_allowance_or_enable_early() {
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    let event = MapNotifyEvent {
        response_type: x11rb::protocol::xproto::MAP_NOTIFY_EVENT,
        event: h.window.window,
        window: h.window.window,
        ..Default::default()
    };
    h.runtime
        .synchronize_map_input_with(&event, h.window.window, 600, || {
            panic!("Show must stay fenced")
        })
        .unwrap();
    assert_eq!(h.runtime.pending_map_input, None);
    assert!(h.runtime.input_fence.blocked);
    h.current = visible();
    assert_success(h.run(&mut op).unwrap(), true, true);
    h.runtime
        .synchronize_map_input_with(&event, h.window.window, 4 * 65536 + 200, || {
            panic!("no second SHAPE allowance")
        })
        .unwrap();
    assert_eq!(h.runtime.input_fence.enabled_after, Some(4 * 65536 + 123));
}
#[test]
fn layout_notifications_invalidate_show_without_visibility_or_geometry_retries() {
    let atoms = x11::monitors::LayoutAtoms {
        number_of_desktops: 90,
        desktop_viewport: 91,
        workarea: 10,
        current_desktop: 11,
        wm_desktop: 12,
    };
    let event = Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
        window: 1,
        atom: 10,
        ..Default::default()
    });
    assert!(visibility_layout_interruption(&event, 1, &atoms));
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    op.show.layout_interrupted = true;
    h.current = visible();
    assert_eq!(
        h.run(&mut op).unwrap().detail().reason,
        Reason::LayoutChanged
    );
    assert_eq!(h.count("map"), 1);
    assert_eq!(h.count("enable"), 0);
    assert_eq!(h.count("correct"), 0);
}
#[test]
fn untracked_transition_debt_blocks_even_off_workspace_completion() {
    let mut h = Fake::new(reading(Visibility::NotMinimizedElsewhere, L::Normal, 7, 2));
    h.runtime.input_fence.blocked = true;
    h.runtime.input_fence.empty_confirmed = true;
    h.runtime.pointer.require_transition_release();
    let mut op = h.operation();
    assert!(service_show_with(&mut op, &mut h).is_err());
    assert_eq!(op.detail.reason, Reason::ReleaseFailed);
    assert_eq!(h.count("map"), 0);
}
#[test]
fn late_drift_after_enabling_is_failure_and_must_not_overstate_ready() {
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    let mut changed = visible();
    changed.layer = L::Above;
    h.reads = VecDeque::from([visible(), visible(), changed]);
    let result = service_show_safely_with(&mut op, &mut h).unwrap().unwrap();
    assert_eq!(result.detail().reason, Reason::LayerChanged);
    assert_eq!(op.show.phase, ShowPhase::InputRestored);
    assert!(!result.detail().progress.input_ready);
    assert!(h.runtime.input_fence.blocked);
    assert_eq!(h.count("empty"), 2);
}
#[test]
fn failed_enabling_attempt_re_fences_once_and_keeps_original_error() {
    let mut h = minimized();
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.current = visible();
    h.fail = Some("shape");
    let error = service_show_safely_with(&mut op, &mut h).unwrap_err();
    assert!(error.to_string().contains("shape"));
    assert!(h.runtime.input_fence.blocked);
    assert!(h.runtime.input_fence.empty_confirmed);
    assert!(h.runtime.visibility_dispatch_eligible());
    assert_eq!(h.count("empty"), 2);
    assert_eq!(h.count("enable"), 1);
    assert_eq!(h.count("map"), 1);
}
#[test]
fn local_layer_busy_is_rejected_only_after_popup_and_pointer_cleanup() {
    let log = RefCell::new(vec![]);
    let mut r = visibility_fence_tests::runtime(3);
    let mut w = tests::test_window();
    let mut active = Some(minimized().operation());
    let desired = r.layers.desired;
    r.complete_menu_with(
        &mut w,
        MenuOutcome::SetLayer(Layer::Above),
        |p| {
            p.release_with(|| {
                log.borrow_mut().push("release");
                Ok(())
            })
            .map(|_| ())
        },
        |_| {
            log.borrow_mut().push("popup");
            Ok(())
        },
        |_, target| {
            local_layer_after_menu_with(active.is_some(), target, || {
                log.borrow_mut().push("layer");
                Ok(())
            })
        },
    )
    .unwrap();
    assert_eq!(*log.borrow(), ["release", "popup"]);
    assert!(r.menu.is_none());
    assert!(!r.pointer.is_grabbed());
    assert_eq!(r.layers.desired, desired);
    active.take();
    local_layer_after_menu_with(active.is_some(), Layer::Above, || {
        log.borrow_mut().push("layer");
        Ok(())
    })
    .unwrap();
    assert_eq!(*log.borrow(), ["release", "popup", "layer"]);
}
mod hide_tests {
    use super::*;
    use control::HidePhase;
    use x11rb::protocol::xproto::{ButtonPressEvent, ButtonReleaseEvent, MotionNotifyEvent};

    fn run(h: &mut Fake, op: &mut ActiveOperation) -> Option<control::OperationResult> {
        service_visibility_with(op, h).unwrap()
    }
    fn assert_hidden(r: control::OperationResult, sent: bool) {
        assert_eq!(r.status(), Status::Success);
        assert_eq!(r.exit_code(), control::ExitCode::Success);
        let d = r.detail();
        assert_eq!(d.visibility, Visibility::Minimized);
        assert_ne!(d.workspace, Workspace::Unknown);
        assert_eq!(d.stage, control::Stage::Hide);
        assert_eq!(d.progress.host_mutation_dispatched, sent);
        assert!(!d.progress.input_ready);
        assert!(!d.progress.not_minimized_confirmed);
        assert!(!d.progress.above_confirmed);
    }
    #[test]
    fn local_and_all_desktops_require_fresh_minimization_after_checked_dispatch() {
        for desktop in [2, u32::MAX] {
            let mut h = Fake::new(reading(Visibility::NotMinimizedHere, L::Below, desktop, 2));
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            assert_eq!(h.log, ["fence", "empty", "release", "observe", "iconify"]);
            assert_eq!(op.hide.phase, HidePhase::IconifyRequested);
            assert_eq!(h.count("add_hidden"), 0);
            assert!(run(&mut h, &mut op).is_none()); // acknowledgement is not success
            assert_eq!(h.count("iconify"), 1);
            h.current = reading(Visibility::Minimized, L::Below, desktop, 2);
            assert_hidden(run(&mut h, &mut op).unwrap(), true);
            assert!(h.runtime.input_fence.blocked);
            assert_eq!(h.count("map"), 0);
            assert_eq!(h.count("geometry"), 0);
            assert_eq!(h.count("enable"), 0);
        }
    }
    #[test]
    fn both_other_workspace_representations_must_request_and_confirm_hidden() {
        for normal in [false, true] {
            let mut r = reading(Visibility::NotMinimizedElsewhere, L::Above, 7, 2);
            if normal {
                r.evidence.wm_state = WmState::Normal;
                r.evidence.map_state = Evidence::Readable(MapState::Unviewable);
            }
            let mut h = Fake::new(r);
            let xid = h.window.window;
            let geometry = h.geometry;
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            assert!(run(&mut h, &mut op).is_none());
            assert_eq!(op.hide.phase, HidePhase::HiddenAddRequested);
            assert_eq!(h.count("add_hidden"), 1);
            assert_eq!(h.count("iconify"), 0);
            assert_eq!(h.count("map"), 0);
            assert_eq!(h.count("geometry"), 0);
            assert_eq!(h.count("correct"), 0);
            h.current = reading(Visibility::Minimized, L::Above, 7, 2);
            let result = run(&mut h, &mut op).unwrap();
            assert_hidden(result, true);
            assert_eq!(result.detail().workspace, Workspace::Other);
            assert_eq!((h.window.window, h.geometry), (xid, geometry));
        }
    }
    #[test]
    fn off_workspace_refusal_fails_without_placement_or_input_recovery() {
        let mut h = Fake::new(reading(Visibility::NotMinimizedElsewhere, L::Normal, 7, 2));
        let mut op = h.command_operation(Command::Hide);
        assert!(run(&mut h, &mut op).is_none());
        h.now = op.deadline;
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert_eq!(result.exit_code(), control::ExitCode::OperationFailed);
        assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
        assert_eq!(
            result.detail().visibility,
            Visibility::NotMinimizedElsewhere
        );
        assert!(h.runtime.input_fence.blocked);
        assert_eq!(h.count("add_hidden"), 1);
        assert_eq!(h.count("iconify"), 0);
        assert_eq!(h.count("geometry"), 0);
    }
    #[test]
    fn already_minimized_still_fences_untracked_capture_and_never_enables_input() {
        let mut h = minimized();
        h.runtime.pointer.require_transition_release();
        let mut op = h.command_operation(Command::Hide);
        assert_hidden(run(&mut h, &mut op).unwrap(), false);
        assert_eq!(h.log, ["fence", "empty", "release", "observe", "observe"]);
        assert!(!h.runtime.pointer.is_grabbed());
        assert!(h.runtime.input_fence.blocked);
    }
    #[test]
    fn every_gesture_and_pending_correction_is_invalidated_before_iconification() {
        for case in 0..6 {
            let mut h = Fake::new(visible());
            h.runtime = visibility_fence_tests::runtime(case);
            if case == 5 {
                h.runtime.interaction.handle_left_press(
                    Point::new(180, 180),
                    (80, 80),
                    100,
                    Point::new(100, 100),
                );
                h.runtime.interaction.on_grab_acquired();
                h.runtime.pointer.set_grabbed(true);
            }
            h.runtime.pending_move = Some(Point::new(200, 200));
            h.runtime.final_target = Some(Point::new(210, 210));
            h.runtime.confirmation_deadline = Some(h.now + CONFIRMATION_TIMEOUT);
            h.runtime.correction = Some((Point::new(220, 220), h.now + CONFIRMATION_TIMEOUT));
            let generation = h.runtime.menu_generation;
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            assert!(h.runtime.visibility_dispatch_eligible());
            assert_eq!(h.runtime.menu_generation, generation + 1);
            assert_eq!(h.runtime.correction, None);
            assert!(h.window.in_flight_moves.is_empty());
            assert_eq!(h.count("iconify"), 1);
            assert_eq!(h.count("geometry"), 0);
            assert!(
                h.log.iter().position(|s| *s == "release").unwrap()
                    < h.log.iter().position(|s| *s == "iconify").unwrap()
            );
            if case == 3 || case == 4 {
                assert_eq!(h.count("popup"), 1);
            }
        }
    }
    #[test]
    fn fence_failures_attempt_cleanup_preserve_error_and_block_dispatch() {
        for fail in ["empty", "release", "popup"] {
            let mut h = minimized();
            h.runtime = visibility_fence_tests::runtime(4);
            h.fail = Some(fail);
            let mut op = h.command_operation(Command::Hide);
            let err = service_visibility_with(&mut op, &mut h).unwrap_err();
            assert_eq!(err.to_string(), format!("injected {fail}"));
            assert_eq!(h.log, ["fence", "empty", "release", "popup"]);
            assert!(!h.runtime.visibility_dispatch_eligible());
            assert_eq!(h.count("iconify"), 0);
            assert_eq!(h.count("enable"), 0);
            if fail == "release" {
                assert!(h.runtime.pointer.is_grabbed());
                assert!(h.runtime.menu.is_none());
            }
            if fail == "popup" {
                assert!(h.runtime.menu.is_some());
            }
        }
    }
    #[test]
    fn missing_malformed_unsupported_and_contradictory_evidence_never_dispatches() {
        for case in 0..8 {
            let mut r = visible();
            match case {
                0 => r.evidence.hidden = Evidence::Missing,
                1 => r.evidence.hidden = Evidence::Malformed,
                2 => r.evidence.capability = Capability::Unsupported,
                3 => r.evidence.capability = Capability::Unverifiable,
                4 => r.evidence.coherent = false,
                5 => r.evidence.hidden = Evidence::Readable(true),
                6 => r.evidence.workspace = Workspace::Unknown,
                _ => r.evidence.wm_state = WmState::Malformed,
            }
            let mut h = Fake::new(r);
            let mut op = h.command_operation(Command::Hide);
            let initial = run(&mut h, &mut op);
            h.now = op.deadline;
            let result = initial.unwrap_or_else(|| run(&mut h, &mut op).unwrap());
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(h.count("iconify"), 0);
            assert_eq!(h.count("enable"), 0);
            assert!(h.runtime.input_fence.blocked);
        }
    }
    #[test]
    fn transitional_evidence_waits_for_notification_then_dispatches_once() {
        let mut h = Fake::new(visible());
        h.current.evidence.coherent = false;
        let mut op = h.command_operation(Command::Hide);
        assert!(run(&mut h, &mut op).is_none());
        assert!(!remote_service_due(&op, h.now));
        h.current = visible();
        mark_remote_observation(&mut op, false);
        assert!(remote_service_due(&op, h.now));
        assert!(run(&mut h, &mut op).is_none());
        assert_eq!(h.count("iconify"), 1);
        assert_eq!(h.count("fence"), 1);
    }
    #[test]
    fn withdrawn_and_destroyed_fail_without_remanagement() {
        for (visibility, reason) in [
            (Visibility::Withdrawn, Reason::Withdrawn),
            (Visibility::Destroyed, Reason::BodyDestroyed),
        ] {
            let mut h = Fake::new(reading(visibility, L::Normal, 2, 2));
            let mut op = h.command_operation(Command::Hide);
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.detail().reason, reason);
            assert_eq!(h.count("iconify"), 0);
            assert_eq!(h.count("map"), 0);
        }
    }
    #[test]
    fn checked_dispatch_error_retains_attempt_and_dispatch_without_retry() {
        let mut h = Fake::new(visible());
        h.fail = Some("iconify");
        let mut op = h.command_operation(Command::Hide);
        assert!(service_visibility_with(&mut op, &mut h).is_err());
        assert_eq!(op.hide.phase, HidePhase::IconifyAttempted);
        assert!(op.detail.progress.host_mutation_dispatched);
        h.fail = None;
        assert!(run(&mut h, &mut op).is_none());
        assert_eq!(h.count("iconify"), 1);
    }
    #[test]
    fn no_iconification_at_expiry_or_after_fence_or_observation_consumes_budget() {
        for step in ["admission", "fence", "observe"] {
            let mut h = Fake::new(visible());
            let mut op = h.command_operation(Command::Hide);
            if step == "admission" {
                h.now = op.deadline;
            } else {
                h.advance = Some((step, Duration::from_millis(1000)));
            }
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(result.detail().reason, Reason::ConfirmationTimeout);
            assert!(!result.detail().progress.host_mutation_dispatched);
            assert_eq!(h.count("iconify"), 0);
        }
    }
    #[test]
    fn expiry_can_confirm_fresh_minimization_but_cannot_skip_input_obligations() {
        for settled in [false, true] {
            let mut h = Fake::new(visible());
            let mut op = h.command_operation(Command::Hide);
            if settled {
                assert!(run(&mut h, &mut op).is_none());
            }
            h.current = reading(Visibility::Minimized, L::Normal, 2, 2);
            h.now = op.deadline;
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(
                result.status(),
                if settled {
                    Status::Success
                } else {
                    Status::Failed
                }
            );
            assert_eq!(h.count("iconify"), usize::from(settled));
            assert_eq!(h.count("enable"), 0);
        }
    }
    #[test]
    fn final_minimized_candidate_requires_another_coherent_read() {
        let mut h = minimized();
        h.reads = VecDeque::from([h.current, visible()]);
        let mut op = h.command_operation(Command::Hide);
        assert!(run(&mut h, &mut op).is_none());
        assert_eq!(h.count("iconify"), 0);
        h.now = op.deadline;
        assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Failed);
        assert!(h.runtime.accepts_input());
    }
    #[test]
    fn layer_workspace_and_layout_drift_fail_without_compensating_requests() {
        for case in 0..4 {
            let mut h = Fake::new(visible());
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            h.current = reading(Visibility::Minimized, L::Normal, 2, 2);
            match case {
                0 => h.current.layer = L::Above,
                1 => h.current.desktop = 3,
                2 => h.current.current = 3,
                _ => mark_remote_observation(&mut op, true),
            }
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert_eq!(
                result.detail().reason,
                if case == 0 {
                    Reason::LayerChanged
                } else {
                    Reason::LayoutChanged
                }
            );
            assert_eq!(h.count("iconify"), 1);
            assert_eq!(h.count("map"), 0);
            assert_eq!(h.count("geometry"), 0);
        }
    }
    #[test]
    fn local_refusal_restores_input_only_after_fresh_placement_and_keeps_failed_result() {
        let mut h = Fake::new(visible());
        h.runtime = visibility_fence_tests::runtime(1);
        let mut op = h.command_operation(Command::Hide);
        assert!(run(&mut h, &mut op).is_none());
        h.now = op.deadline;
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert!(!result.detail().progress.input_ready);
        assert!(h.runtime.accepts_input());
        assert_eq!(h.count("iconify"), 1);
        assert_eq!(h.count("enable"), 1);
        assert_eq!(h.count("geometry"), 2);
        assert!(!h.runtime.interaction.is_dragging());
        assert!(h.runtime.pending_move.is_none());
        assert_eq!(h.runtime.input_fence.enabled_after, Some(4 * 65536 + 123));
        for event in [
            Event::ButtonPress(ButtonPressEvent {
                event: h.window.window,
                ..Default::default()
            }),
            Event::ButtonRelease(ButtonReleaseEvent {
                event: h.window.window,
                ..Default::default()
            }),
            Event::MotionNotify(MotionNotifyEvent {
                event: h.window.window,
                ..Default::default()
            }),
        ] {
            assert!(h
                .runtime
                .input_fence
                .rejects(&event, h.window.window, 70001));
        }
    }
    #[test]
    fn failure_recovery_defers_uncertain_minimized_and_off_workspace_until_local_return() {
        for case in 0..3 {
            let mut h = Fake::new(visible());
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            op.hide.layout_interrupted = true;
            h.current = match case {
                0 => reading(Visibility::Minimized, L::Normal, 2, 2),
                1 => reading(Visibility::NotMinimizedElsewhere, L::Normal, 2, 7),
                _ => {
                    let mut r = visible();
                    r.evidence.coherent = false;
                    r
                }
            };
            let result = run(&mut h, &mut op).unwrap();
            assert_eq!(result.status(), Status::Failed);
            assert!(h.runtime.input_fence.blocked);
            assert_eq!(h.count("geometry"), 0);
            h.current = visible();
            restore_deferred_input(&mut h).unwrap();
            assert!(h.runtime.accepts_input());
            assert_eq!(h.count("iconify"), 1);
            assert_eq!(result.status(), Status::Failed);
        }
    }
    #[test]
    fn failed_hide_reuses_bounded_deferred_correction_before_enabling_input() {
        let mut h = Fake::new(visible());
        let mut op = h.command_operation(Command::Hide);
        assert!(run(&mut h, &mut op).is_none());
        h.bounds = ValidOriginBounds::new(200, 500, 200, 500);
        h.now = op.deadline;
        assert_eq!(run(&mut h, &mut op).unwrap().status(), Status::Failed);
        assert_eq!(h.count("ordinary_correct"), 1);
        assert_eq!(h.count("enable"), 0);
        let (_, deadline) = h.runtime.correction.unwrap();
        h.runtime
            .check_correction(deadline, h.geometry.origin(), &h.bounds)
            .unwrap();
        h.window.cancel_in_flight_moves();
        restore_deferred_input(&mut h).unwrap();
        assert!(h.runtime.accepts_input());
        assert_eq!(h.count("ordinary_correct"), 1);
        assert_eq!(h.count("iconify"), 1);
    }
    #[test]
    fn failed_recovery_shape_is_refenced_with_original_fatal_error() {
        let mut h = Fake::new(visible());
        let mut op = h.command_operation(Command::Hide);
        assert!(run(&mut h, &mut op).is_none());
        h.fail = Some("shape");
        h.now = op.deadline;
        let err = service_visibility_with(&mut op, &mut h).unwrap_err();
        assert!(err.to_string().contains("shape"));
        assert!(h.runtime.visibility_dispatch_eligible());
        assert!(h.runtime.input_fence.blocked);
        assert_eq!(h.count("empty"), 2);
        assert_eq!(h.count("iconify"), 1);
    }
    #[test]
    fn recovery_rechecks_visibility_after_placement_before_enabling() {
        let mut h = Fake::new(visible());
        let mut op = h.command_operation(Command::Hide);
        assert!(run(&mut h, &mut op).is_none());
        h.now = op.deadline;
        // Terminal failure, local recovery candidate, then unavailable again
        // after geometry reads. No silhouette may be enabled from the old read.
        h.reads = VecDeque::from([
            visible(),
            visible(),
            reading(Visibility::Minimized, L::Normal, 2, 2),
        ]);
        let result = run(&mut h, &mut op).unwrap();
        assert_eq!(result.status(), Status::Failed);
        assert!(h.runtime.input_fence.blocked);
        assert_eq!(h.count("enable"), 0);
        h.current = visible();
        restore_deferred_input(&mut h).unwrap();
        assert!(h.runtime.accepts_input());
        assert_eq!(h.count("iconify"), 1);
    }
    #[test]
    fn invalid_retained_monitor_or_resized_body_never_recovers_input() {
        for resized in [false, true] {
            let mut h = Fake::new(visible());
            let mut op = h.command_operation(Command::Hide);
            assert!(run(&mut h, &mut op).is_none());
            if resized {
                h.geometry.width += 1;
            } else {
                h.monitor += 1;
            }
            h.now = op.deadline;
            assert!(service_visibility_with(&mut op, &mut h).is_err());
            assert!(h.runtime.input_fence.blocked);
            assert_eq!(op.detail.reason, Reason::X11Failure);
            assert_eq!(h.count("iconify"), 1);
        }
    }
    #[test]
    fn repeated_hide_show_cycles_preserve_body_workspace_layer_and_fresh_input() {
        for layer in [L::Normal, L::Above, L::Below] {
            let mut h = Fake::new(reading(Visibility::NotMinimizedHere, layer, 2, 2));
            let xid = h.window.window;
            for _ in 0..20 {
                let mut hide = h.command_operation(Command::Hide);
                assert!(run(&mut h, &mut hide).is_none());
                h.current = reading(Visibility::Minimized, layer, 2, 2);
                assert_hidden(run(&mut h, &mut hide).unwrap(), true);
                let mut noop = h.command_operation(Command::Hide);
                assert_hidden(run(&mut h, &mut noop).unwrap(), false);
                let mut show = h.operation();
                assert!(run(&mut h, &mut show).is_none());
                assert!(h.runtime.input_fence.blocked);
                h.current = reading(Visibility::NotMinimizedHere, layer, 2, 2);
                assert_success(run(&mut h, &mut show).unwrap(), true, true);
                assert!(h.runtime.accepts_input());
                assert_eq!(h.window.window, xid);
                assert!(h.runtime.pending_move.is_none());
                assert!(h.runtime.final_target.is_none());
                assert_eq!(h.current.layer, layer);
                assert_eq!(h.current.desktop, 2);
            }
            assert_eq!(h.count("iconify"), 20);
            assert_eq!(h.count("map"), 20);
            assert_eq!(h.count("correct"), 0);
        }
    }

    #[test]
    fn repeated_off_workspace_cycles_preserve_xid_layer_workspaces_and_fence() {
        for layer in [L::Normal, L::Above, L::Below] {
            let mut h = Fake::new(reading(Visibility::NotMinimizedElsewhere, layer, 7, 2));
            let xid = h.window.window;
            let geometry = h.geometry;
            for _ in 0..20 {
                let mut hide = h.command_operation(Command::Hide);
                assert!(run(&mut h, &mut hide).is_none());
                h.current = reading(Visibility::Minimized, layer, 7, 2);
                assert_hidden(run(&mut h, &mut hide).unwrap(), true);
                let mut noop_hide = h.command_operation(Command::Hide);
                assert_hidden(run(&mut h, &mut noop_hide).unwrap(), false);
                let mut show = h.operation();
                assert!(run(&mut h, &mut show).is_none());
                h.current = reading(Visibility::NotMinimizedElsewhere, layer, 7, 2);
                assert_success(run(&mut h, &mut show).unwrap(), false, true);
                let mut noop_show = h.operation();
                assert_success(run(&mut h, &mut noop_show).unwrap(), false, false);
                assert_eq!((h.window.window, h.geometry), (xid, geometry));
                assert_eq!(
                    (h.current.desktop, h.current.current, h.current.layer),
                    (7, 2, layer)
                );
                assert!(h.runtime.input_fence.blocked);
                assert!(!h.runtime.accepts_input());
                assert!(!h.runtime.pointer.is_grabbed());
                assert!(h.runtime.menu.is_none());
                assert!(h.runtime.pending_move.is_none());
                assert!(h.runtime.final_target.is_none());
                assert_eq!(h.runtime.pending_map_input, None);
            }
            assert_eq!(h.count("add_hidden"), 20);
            assert_eq!(h.count("remove_hidden"), 20);
            for step in [
                "iconify",
                "user_time_zero",
                "map",
                "geometry",
                "correct",
                "enable",
            ] {
                assert_eq!(h.count(step), 0);
            }
        }
    }

    #[test]
    fn local_hide_then_manual_workspace_change_selects_remote_show() {
        let mut h = Fake::new(visible());
        let xid = h.window.window;
        let mut hide = h.command_operation(Command::Hide);
        assert!(run(&mut h, &mut hide).is_none());
        h.current = reading(Visibility::Minimized, L::Normal, 2, 2);
        assert_hidden(run(&mut h, &mut hide).unwrap(), true);
        // The owner has completed Hide. The user changes workspace before
        // admitting a new Show; its fresh baseline must select REMOVE-HIDDEN.
        h.current = reading(Visibility::Minimized, L::Normal, 2, 7);
        let mut show = h.operation();
        assert!(run(&mut h, &mut show).is_none());
        h.current = reading(Visibility::NotMinimizedElsewhere, L::Normal, 2, 7);
        assert_success(run(&mut h, &mut show).unwrap(), false, true);
        assert_eq!(h.count("iconify"), 1);
        assert_eq!(h.count("remove_hidden"), 1);
        assert_eq!(h.count("add_hidden"), 0);
        assert_eq!(h.window.window, xid);
        assert_eq!((h.current.desktop, h.current.current), (2, 7));
        for step in ["user_time_zero", "map", "geometry", "enable"] {
            assert_eq!(h.count(step), 0);
        }
    }
}
#[test]
fn here_restricted_deferred_recovery_cannot_correct_but_can_restore_fresh_in_bounds_input() {
    let mut h = Fake::new(visible());
    h.fence().unwrap();
    h.runtime.input_fence.automatic_correction_blocked = true;
    h.geometry = Rect::new(999, 999, 160, 160);
    for _ in 0..3 {
        restore_deferred_input(&mut h).unwrap();
        assert_eq!(h.count("deferred_correct"), 0);
        assert_eq!(h.count("enable"), 0);
        assert!(h.runtime.input_fence.blocked);
        assert!(h.runtime.correction.is_none());
    }
    h.geometry = Rect::new(100, 100, 160, 160);
    restore_deferred_input(&mut h).unwrap();
    assert_eq!(h.count("enable"), 1);
    assert!(!h.runtime.input_fence.blocked);
    assert!(!h.runtime.input_fence.automatic_correction_blocked);
}

#[test]
fn new_explicit_show_and_bring_top_have_fresh_correction_budget_after_here_restriction() {
    for command in [Command::Show, Command::BringTop] {
        let mut h = Fake::new(visible());
        h.fence().unwrap();
        h.runtime.input_fence.automatic_correction_blocked = true;
        h.geometry = Rect::new(999, 999, 160, 160);
        let mut op = h.command_operation(command);
        assert!(service_remote_with(&mut op, &mut h).unwrap().is_none());
        assert_eq!(h.count("correct"), 1);
        h.now = op.show.correction.unwrap().1;
        let result = service_remote_with(&mut op, &mut h).unwrap().unwrap();
        assert_eq!(result.status(), Status::Success);
        assert!(!h.runtime.input_fence.automatic_correction_blocked);
        assert_eq!(h.count("correct"), 1);
    }
}

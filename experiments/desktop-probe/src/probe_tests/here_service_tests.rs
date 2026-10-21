use super::*;
use control::{
    ActiveOperation, Command, HerePhase, LayerObservation as L, MapState, Reason, Stage, Status,
    WmState,
};
use geometry::Rect;
use std::cell::RefCell;
use std::collections::VecDeque;
use x11::monitors::destination::DestinationError;

#[derive(Debug, PartialEq, Eq)]
struct NativeFault(&'static str, usize);
impl std::fmt::Display for NativeFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native {} #{}", self.0, self.1)
    }
}
impl std::error::Error for NativeFault {}

#[derive(Clone, Copy)]
struct Reading {
    desktop: u32,
    layer: L,
    minimized: bool,
    transitional: bool,
}
impl Reading {
    fn snapshot(self) -> x11::visibility::VisibilitySnapshot {
        here_adapter_tests::snapshot(
            if self.minimized || self.desktop != 2 || self.transitional {
                WmState::Iconic
            } else {
                WmState::Normal
            },
            if self.minimized || self.desktop != 2 {
                MapState::Unmapped
            } else {
                MapState::Viewable
            },
            self.minimized,
            self.desktop,
            self.layer,
        )
    }
}
fn layout() -> control::DestinationIdentity {
    let area = Rect::new(0, 0, 800, 600);
    control::DestinationIdentity {
        desktop_count: 5,
        destination_desktop: 2,
        current_desktop: 2,
        monitor_name: 20,
        monitor_bounds: area,
        target_workarea: Some(area),
        viewport: control::DestinationViewport::AllZero,
        usable_area: area,
        body_size: Size::new(160, 160),
        valid_origins: checked_probe_bounds(area).unwrap(),
    }
}
struct Fake {
    now: Instant,
    runtime: ProbeRuntime,
    window: ManagedProbeWindow,
    current: Reading,
    reads: VecDeque<Reading>,
    layout: Result<control::DestinationIdentity, DestinationError>,
    layouts: VecDeque<Result<control::DestinationIdentity, DestinationError>>,
    actual: Rect,
    geometries: VecDeque<Rect>,
    log: Vec<&'static str>,
    faults: Vec<(&'static str, usize)>,
    changes: VecDeque<(&'static str, Reading)>,
    advances: VecDeque<(&'static str, Duration)>,
    owned: bool,
    refuse_correction: bool,
    send_before_cookie: bool,
    terminal: Option<control::Visibility>,
    lose_at: Option<(&'static str, usize)>,
}
impl Fake {
    fn new(remote: bool, minimized: bool, layer: L) -> Self {
        let mut runtime = visibility_fence_tests::runtime(0);
        runtime.input_fence.enabled_after = Some(10);
        Self {
            now: Instant::now(),
            runtime,
            window: tests::test_window(),
            current: Reading {
                desktop: if remote { 0 } else { 2 },
                minimized,
                layer,
                transitional: false,
            },
            reads: VecDeque::new(),
            layout: Ok(layout()),
            layouts: VecDeque::new(),
            actual: Rect::new(100, 100, 160, 160),
            geometries: VecDeque::new(),
            log: vec![],
            faults: vec![],
            changes: VecDeque::new(),
            advances: VecDeque::new(),
            owned: true,
            refuse_correction: false,
            send_before_cookie: false,
            terminal: None,
            lose_at: None,
        }
    }
    pub(super) fn operation(&self) -> ActiveOperation {
        let mut active = None;
        assert_eq!(
            control::admit(
                &mut active,
                control::Request {
                    reply_xid: 9,
                    request_id: 99,
                    epoch: 11,
                    command: Command::BringHere,
                },
                2,
                control::Lifecycle::Ready,
                false,
                self.now
            ),
            control::Admission::Admitted
        );
        active.unwrap()
    }
    fn count(&self, step: &str) -> usize {
        self.log.iter().filter(|&&s| s == step).count()
    }
    fn step(&mut self, step: &'static str) -> Result<(), HostError> {
        self.log.push(step);
        if self.changes.front().is_some_and(|(s, _)| *s == step) {
            self.current = self.changes.pop_front().unwrap().1;
        }
        if self.advances.front().is_some_and(|(s, _)| *s == step) {
            self.now += self.advances.pop_front().unwrap().1;
        }
        let count = self.count(step);
        if self.lose_at == Some((step, count)) {
            self.owned = false;
        }
        if self.faults.contains(&(step, count)) {
            return Err(Box::new(NativeFault(step, count)));
        }
        Ok(())
    }
    fn read_layout(
        &mut self,
    ) -> Result<Result<control::DestinationIdentity, DestinationError>, HostError> {
        self.step("layout")?;
        Ok(self
            .layouts
            .pop_front()
            .unwrap_or_else(|| self.layout.clone()))
    }
    fn target(&mut self) {
        self.current.desktop = 2;
        self.current.transitional = false;
    }
    fn run(&mut self, op: &mut ActiveOperation) -> Option<control::OperationResult> {
        service_bring_here_with(op, self).unwrap()
    }
    fn arrived(&mut self, op: &mut ActiveOperation) -> control::OperationResult {
        self.target();
        mark_remote_observation(op, false);
        self.run(op).unwrap()
    }
    fn assert_no_mutation(&self) {
        for step in ["empty", "release", "popup", "desktop", "correct", "shape"] {
            assert_eq!(self.count(step), 0, "{step}");
        }
    }
}
impl BringHereHost for Fake {
    fn now(&self) -> Instant {
        self.now
    }
    fn owned(&mut self) -> Result<bool, HostError> {
        self.step("owner")?;
        Ok(self.owned)
    }
    fn live(&self) -> bool {
        self.runtime.body != BodyAvailability::Destroyed
    }
    fn evidence(&mut self, _: u32) -> Result<HereEvidence, HostError> {
        let cell = RefCell::new(&mut *self);
        here_evidence_with(
            || cell.borrow_mut().read_layout(),
            || {
                let mut h = cell.borrow_mut();
                h.step("observe")?;
                let reading = h.reads.pop_front().unwrap_or(h.current);
                let mut snapshot = reading.snapshot();
                if let Some(terminal) = h.terminal {
                    if terminal == control::Visibility::Destroyed {
                        snapshot.evidence.destroyed = true;
                        h.window.window_resource.externally_destroyed();
                        h.runtime.unavailable(true);
                    } else {
                        snapshot.evidence.wm_state = WmState::Withdrawn;
                        snapshot.observation.as_mut().unwrap().wm_state = WmState::Withdrawn;
                    }
                    snapshot.candidate = control::classify(snapshot.evidence);
                }
                Ok(snapshot)
            },
        )
    }
    fn placement_busy(&self) -> bool {
        self.runtime.correction.is_some()
    }
    fn input_active(&self) -> bool {
        !self.runtime.input_fence.blocked
    }
    fn visible_ready(&self) -> bool {
        here_runtime_ready(&self.runtime, &self.window)
    }
    fn fence_settled(&self) -> bool {
        self.runtime.visibility_dispatch_eligible()
    }
    fn fence(&mut self) -> Result<(), HostError> {
        self.log.push("fence");
        let empty = self.step("empty");
        let release = self.step("release");
        let popup = self.step("popup");
        self.runtime.fence_visibility_input_with(
            &mut self.window,
            || empty.map(|()| 100),
            |_, _| release,
            |_| popup,
        )
    }
    fn desktop(
        &mut self,
        target: u32,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<bool, HostError> {
        assert_eq!(target, 2);
        assert!(self.fence_settled());
        if self.now >= deadline {
            return Ok(false);
        }
        if !self.send_before_cookie {
            *dispatched = true;
        }
        self.step("desktop")?;
        Ok(true)
    }
    fn geometry(&mut self) -> Result<Rect, HostError> {
        self.step("geometry")?;
        Ok(self.geometries.pop_front().unwrap_or(self.actual))
    }
    fn restrict_automatic_correction(&mut self) {
        self.runtime.input_fence.automatic_correction_blocked = true;
        self.log.push("restrict");
    }
    fn correct(
        &mut self,
        target: Point,
        deadline: Instant,
        dispatched: &mut bool,
    ) -> Result<Option<Instant>, HostError> {
        assert!(self.runtime.input_fence.automatic_correction_blocked);
        assert!(self.fence_settled());
        if self.now + CONFIRMATION_TIMEOUT > deadline {
            return Ok(None);
        }
        *dispatched = true;
        self.step("correct")?;
        if !self.refuse_correction {
            self.actual = Rect::new(target.x, target.y, 160, 160);
        }
        self.runtime.body = BodyAvailability::Validating;
        Ok(Some(self.now + CONFIRMATION_TIMEOUT))
    }
    fn enable(
        &mut self,
        evidence: &HereEvidence,
        actual: Rect,
        deadline: Instant,
    ) -> Result<bool, HostError> {
        assert!(self.now < deadline);
        self.step("enable")?;
        let shape = self.step("shape");
        self.window.confirmed_origin = actual.origin();
        self.window.cancel_in_flight_moves();
        self.runtime.body = BodyAvailability::Ready;
        self.runtime.restore_visibility_input_with(
            &self.window,
            evidence.snapshot.evidence,
            &InputPlacement {
                selected_monitor: 20,
                observed_monitor: evidence.layout.as_ref().unwrap().monitor_name,
                geometry: actual,
                bounds: evidence.layout.as_ref().unwrap().valid_origins,
            },
            || shape.map(|()| 1000),
        )?;
        Ok(true)
    }
}
fn check_result(result: control::OperationResult, status: Status, history: bool) {
    assert_eq!(result.status(), status);
    assert_eq!(
        result.detail().placement.history.relocation_confirmed,
        history
    );
    assert!(!result.detail().placement.history.restoration_confirmed);
    let reply = control::TerminalResponse {
        status,
        detail: result.detail(),
        request_id: 99,
        control_xid: 10,
    };
    assert_eq!(
        control::TerminalResponse::decode(&reply.encode().unwrap()).unwrap(),
        reply
    );
    if status != Status::Success {
        assert_eq!(
            result.detail().placement.current,
            control::PlacementConfirmation::default()
        );
    }
}

#[test]
fn safe_local_noop_is_fresh_and_has_zero_mutations_for_all_layers() {
    for layer in [L::Normal, L::Above, L::Below] {
        let mut h = Fake::new(false, false, layer);
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        check_result(result, Status::Success, false);
        h.assert_no_mutation();
        assert!(h.count("layout") >= 6);
        assert!(h.count("geometry") >= 2);
        assert_eq!(
            result.detail().placement.current.geometry,
            control::GeometryProof::Verified
        );
    }
}
#[test]
fn local_unsafe_fence_cancels_every_gesture_popup_and_release_debt_neutrally() {
    for case in 0..6 {
        let mut h = Fake::new(false, false, L::Below);
        h.runtime = visibility_fence_tests::runtime(case);
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        check_result(result, Status::Success, false);
        assert_eq!(h.count("fence"), 1);
        assert_eq!(h.count("desktop"), 0);
        assert_eq!(h.count("correct"), 0);
        assert_eq!(h.count("shape"), 1);
        assert!(!h.runtime.pointer.is_grabbed());
        assert!(h.runtime.menu.is_none());
        assert!(h.runtime.pending_move.is_none());
        assert!(h.runtime.final_target.is_none());
    }
}
#[test]
fn stronger_noop_checks_all_runtime_obligations() {
    for case in 0..11 {
        let mut h = Fake::new(false, false, L::Normal);
        match case {
            0 => h.runtime.pending_map_input = Some(1),
            1 => h.runtime.pending_move = Some(Point::new(120, 120)),
            2 => h.runtime.final_target = Some(Point::new(120, 120)),
            3 => h.runtime.confirmation_deadline = Some(h.now),
            4 => h.runtime.safety.deadline = Some(h.now),
            5 => h.runtime.body = BodyAvailability::Validating,
            6 => h.runtime.pointer.require_transition_release(),
            7 => h.runtime.input_fence.enabled_after = None,
            8 => h.window.in_flight_moves.push_back(Point::new(120, 120)),
            9 => h.runtime.menu = Some(MenuPopup::test_popup(200, 1)),
            _ => h.runtime.input_fence.blocked = true,
        }
        let mut op = h.operation();
        check_result(h.run(&mut op).unwrap(), Status::Success, false);
        assert_eq!(h.count("fence"), 1, "case {case}");
    }
}
#[test]
fn existing_ordinary_correction_is_busy_and_not_stolen() {
    let mut h = Fake::new(false, false, L::Normal);
    let mut op = h.operation();
    h.runtime.correction = Some((Point::new(0, 0), h.now));
    assert_eq!(h.run(&mut op).unwrap().status(), Status::Busy);
    h.assert_no_mutation();
    assert!(h.runtime.correction.is_some());
    assert_eq!(h.count("layout"), 0);
}
#[test]
fn local_correction_clamps_live_origin_once_and_cannot_authorize_partial() {
    let mut h = Fake::new(false, false, L::Normal);
    h.actual = Rect::new(900, -30, 160, 160);
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    assert_eq!(op.placement.correction().unwrap().0, Point::new(640, 0));
    check_result(h.run(&mut op).unwrap(), Status::Success, false);
    assert_eq!(h.count("correct"), 1);
    assert_eq!(h.count("desktop"), 0);
    assert_eq!(h.count("shape"), 1);
}
#[test]
fn local_and_remote_minimized_preserve_each_layer_and_defer_without_geometry() {
    for remote in [false, true] {
        for layer in [L::Normal, L::Above, L::Below] {
            let mut h = Fake::new(remote, true, layer);
            h.actual = Rect::new(9999, 9999, 160, 160);
            let mut op = h.operation();
            let result = if remote {
                assert!(h.run(&mut op).is_none());
                h.arrived(&mut op)
            } else {
                h.run(&mut op).unwrap()
            };
            check_result(result, Status::Success, remote);
            let d = result.detail();
            assert!(d.progress.placement_deferred);
            assert!(!d.progress.input_ready);
            assert!(!d.progress.above_confirmed);
            assert_eq!(d.layer, layer);
            assert_eq!(
                d.placement.current.geometry,
                control::GeometryProof::Deferred
            );
            for step in ["geometry", "correct", "shape"] {
                assert_eq!(h.count(step), 0);
            }
            assert!(h.fence_settled());
            assert!(!h.runtime.input_fence.automatic_correction_blocked);
        }
    }
}
#[test]
fn remote_visible_each_layer_including_iconic_unmapped_without_hidden() {
    for layer in [L::Normal, L::Above, L::Below] {
        let mut h = Fake::new(true, false, layer);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        assert!(!op.placement.baseline.unwrap().minimized);
        assert!(!op.placement.proof.history.relocation_confirmed);
        assert_eq!(h.count("desktop"), 1);
        let log = &h.log;
        let send = log.iter().position(|s| *s == "desktop").unwrap();
        let release = log.iter().position(|s| *s == "release").unwrap();
        assert!(send > release);
        assert!(log[release..send].contains(&"layout"));
        check_result(h.arrived(&mut op), Status::Success, true);
        assert_eq!(h.count("shape"), 1);
        assert_eq!(h.count("desktop"), 1);
        assert_eq!(h.count("correct"), 0);
    }
}
#[test]
fn initial_transitional_or_invalid_evidence_never_establishes_mutation_authority() {
    for variant in 0..5 {
        let mut h = Fake::new(true, false, L::Normal);
        match variant {
            0 => {
                h.current.desktop = 2;
                h.current.transitional = true;
            }
            1 => h.current.layer = L::Unknown,
            2 => h.layout = Err(DestinationError::UnsupportedViewport),
            3 => h.layout = Err(DestinationError::Changed),
            _ => h.layout = Err(DestinationError::Unverifiable("bad monitor".into())),
        }
        let mut op = h.operation();
        check_result(h.run(&mut op).unwrap(), Status::Failed, false);
        h.assert_no_mutation();
    }
}
#[test]
fn every_fence_failure_stops_send_but_attempts_release_and_popup_cleanup() {
    for step in ["empty", "release", "popup"] {
        let mut h = Fake::new(true, false, L::Normal);
        h.runtime = visibility_fence_tests::runtime(3);
        h.faults.push((step, 1));
        let mut op = h.operation();
        let error = service_bring_here_with(&mut op, &mut h).unwrap_err();
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault(step, 1))
        );
        assert_eq!(h.count("release"), 1);
        assert_eq!(h.count("popup"), 1);
        assert_eq!(h.count("desktop"), 0);
        assert!(h.runtime.input_fence.blocked);
        assert_eq!(op.detail.stage, Stage::Shutdown);
    }
}
#[test]
fn failed_enable_gets_exactly_one_refence_and_original_error_priority() {
    for remote in [false, true] {
        for cleanup_fails in [false, true] {
            let mut h = Fake::new(remote, false, L::Normal);
            h.runtime.input_fence.enabled_after = None;
            let mut op = h.operation();
            if remote {
                assert!(h.run(&mut op).is_none());
                h.target();
            }
            h.faults.push(("shape", 1));
            if cleanup_fails {
                h.faults.push(("empty", 2));
            }
            let error = service_bring_here_with(&mut op, &mut h).unwrap_err();
            assert_eq!(
                error.downcast_ref::<NativeFault>(),
                Some(&NativeFault("shape", 1))
            );
            assert_eq!(h.count("fence"), 2);
            assert_eq!(h.count("shape"), 1);
            assert!(h.runtime.input_fence.blocked);
            assert_eq!(op.detail.reason, Reason::X11Failure);
            check_result(
                op.placement
                    .here_result(op.detail, control::Confirmation::default())
                    .unwrap(),
                if remote {
                    Status::Partial
                } else {
                    Status::Failed
                },
                remote,
            );
        }
    }
}
#[test]
fn final_loss_after_enable_refences_once_and_never_reenables() {
    for remote in [false, true] {
        let mut h = Fake::new(remote, false, L::Normal);
        h.runtime.input_fence.enabled_after = None;
        let mut op = h.operation();
        if remote {
            assert!(h.run(&mut op).is_none());
            h.target();
        }
        let mut changed = h.current;
        changed.layer = L::Below;
        h.changes.push_back(("shape", changed));
        let result = h.run(&mut op).unwrap();
        check_result(
            result,
            if remote {
                Status::Partial
            } else {
                Status::Failed
            },
            remote,
        );
        assert_eq!(result.detail().reason, Reason::LayerChanged);
        assert_eq!(h.count("shape"), 1);
        assert_eq!(h.count("fence"), 2);
        assert!(h.runtime.input_fence.blocked);
    }
}
#[test]
fn lost_final_noop_safety_also_fences_without_creating_relocation_history() {
    let mut h = Fake::new(false, false, L::Normal);
    let mut op = h.operation();
    h.geometries = VecDeque::from([h.actual, Rect::new(999, 999, 160, 160)]);
    check_result(h.run(&mut op).unwrap(), Status::Failed, false);
    assert_eq!(h.count("fence"), 1);
    assert_eq!(h.count("desktop"), 0);
    assert_eq!(h.count("shape"), 0);
}
#[test]
fn uncertain_send_can_only_gain_history_from_bounded_fresh_membership() {
    for before_cookie in [false, true] {
        for reaches_target in [false, true] {
            let mut h = Fake::new(true, false, L::Normal);
            h.send_before_cookie = before_cookie;
            h.faults.push(("desktop", 1));
            let mut changed = h.current;
            changed.desktop = 2;
            if reaches_target {
                h.changes.push_back(("desktop", changed));
            }
            let mut op = h.operation();
            let err = service_bring_here_with(&mut op, &mut h).unwrap_err();
            assert_eq!(
                err.downcast_ref::<NativeFault>(),
                Some(&NativeFault("desktop", 1))
            );
            let history = !before_cookie && reaches_target;
            check_result(
                op.placement
                    .here_result(op.detail, control::Confirmation::default())
                    .unwrap(),
                if history {
                    Status::Partial
                } else {
                    Status::Failed
                },
                history,
            );
            assert_eq!(h.count("desktop"), 1);
            assert_eq!(h.count("correct"), 0);
            assert_eq!(h.count("shape"), 0);
        }
    }
}
#[test]
fn outer_layout_native_failure_before_send_after_possible_send_and_after_history() {
    for phase in 0..3 {
        for final_fails in [false, true] {
            let mut h = Fake::new(true, false, L::Normal);
            let mut op = h.operation();
            if phase > 0 {
                assert!(h.run(&mut op).is_none());
            }
            if phase == 2 {
                h.target();
                h.current.transitional = true;
                assert!(h.run(&mut op).is_none());
                assert!(op.placement.proof.history.relocation_confirmed);
            }
            let fault_at = h.count("layout") + 1;
            h.faults.push(("layout", fault_at));
            if final_fails {
                h.faults.push(("layout", fault_at + 1));
            } else if phase > 0 {
                h.target();
            }
            let err = service_bring_here_with(&mut op, &mut h).unwrap_err();
            assert_eq!(
                err.downcast_ref::<NativeFault>(),
                Some(&NativeFault("layout", fault_at))
            );
            let history = phase == 2 || (phase == 1 && !final_fails);
            let result = op
                .placement
                .here_result(op.detail, control::Confirmation::default())
                .unwrap();
            check_result(
                result,
                if history {
                    Status::Partial
                } else {
                    Status::Failed
                },
                history,
            );
            assert_eq!(result.detail().stage, Stage::Shutdown);
            assert_eq!(result.detail().reason, Reason::X11Failure);
            assert_eq!(h.count("desktop"), usize::from(phase > 0));
            assert_eq!(h.count("shape"), 0);
            if phase == 0 {
                h.assert_no_mutation();
                assert_eq!(h.count("layout"), 1);
            }
        }
    }
}
#[test]
fn inner_destination_errors_are_ordinary_results_at_all_corresponding_phases() {
    for phase in 0..3 {
        for error in [
            DestinationError::Changed,
            DestinationError::UnsupportedViewport,
            DestinationError::Unverifiable("invalid viewport/monitor".into()),
        ] {
            let mut h = Fake::new(true, false, L::Normal);
            let mut op = h.operation();
            if phase > 0 {
                assert!(h.run(&mut op).is_none());
            }
            if phase == 2 {
                h.target();
                h.current.transitional = true;
                assert!(h.run(&mut op).is_none());
            }
            h.layout = Err(error.clone());
            let result = h.run(&mut op).unwrap();
            check_result(
                result,
                if phase == 2 {
                    Status::Partial
                } else {
                    Status::Failed
                },
                phase == 2,
            );
            assert_eq!(result.detail().reason, here_destination_reason(&error));
            assert_ne!(result.detail().stage, Stage::Shutdown);
            assert_eq!(h.count("desktop"), usize::from(phase > 0));
        }
    }
}
#[test]
fn native_primary_survives_final_inner_error_and_terminal_reply_error() {
    let mut h = Fake::new(true, false, L::Normal);
    let mut active = Some(h.operation());
    h.faults.push(("desktop", 1));
    h.target();
    h.current.desktop = 0;
    h.layouts = VecDeque::from([
        Ok(layout()),
        Ok(layout()),
        Ok(layout()),
        Ok(layout()),
        Err(DestinationError::Changed),
        Err(DestinationError::Changed),
    ]);
    let mut target = h.current;
    target.desktop = 2;
    h.changes.push_back(("desktop", target));
    let mut replies = vec![];
    let err = service_here_owner_with(&mut active, &mut h, |r| {
        replies.push(r);
        Err("reply failed".into())
    })
    .unwrap_err();
    assert_eq!(
        err.downcast_ref::<NativeFault>(),
        Some(&NativeFault("desktop", 1))
    );
    assert!(active.is_none());
    assert_eq!(replies.len(), 1);
    check_result(replies[0], Status::Partial, true);
    assert_eq!(replies[0].detail().reason, Reason::X11Failure);
}
#[test]
fn layout_and_preservation_drift_never_retarget_or_mutate_layer() {
    for phase in 0..3 {
        for drift in 0..10 {
            let mut h = Fake::new(true, false, L::Above);
            let mut op = h.operation();
            if phase > 0 {
                assert!(h.run(&mut op).is_none());
            }
            if phase == 2 {
                h.target();
                h.current.transitional = true;
                assert!(h.run(&mut op).is_none());
                h.current.transitional = false;
            }
            let mut changed = layout();
            match drift {
                0 => changed.desktop_count += 1,
                1 => changed.current_desktop = 3,
                2 => changed.target_workarea = None,
                3 => changed.viewport = control::DestinationViewport::Absent,
                4 => changed.monitor_name += 1,
                5 => changed.monitor_bounds = Rect::new(1, 0, 800, 600),
                6 => {
                    h.current.layer = L::Below;
                }
                7 => {
                    h.current.minimized = true;
                }
                8 => {
                    h.current.desktop = 3;
                }
                _ => {
                    h.current.desktop = 0;
                }
            }
            if phase == 0 && drift < 6 {
                // changes are detected against the admitted round, not a fabricated baseline
                h.layouts = VecDeque::from([Ok(layout()), Ok(layout()), Ok(changed), Ok(changed)]);
            } else if drift < 6 {
                h.layout = Ok(changed);
            }
            if phase == 0 && drift == 9 {
                continue;
            }
            if phase == 0 && drift >= 6 {
                h.reads.push_back(Reading {
                    desktop: 0,
                    layer: L::Above,
                    minimized: false,
                    transitional: false,
                });
            }
            if phase == 1 && drift == 9 {
                h.now = op.deadline;
            }
            let result = h
                .run(&mut op)
                .unwrap_or_else(|| panic!("phase {phase}, drift {drift}"));
            check_result(
                result,
                if phase == 2 {
                    Status::Partial
                } else {
                    Status::Failed
                },
                phase == 2,
            );
            assert_eq!(h.count("desktop"), usize::from(phase > 0));
            assert_eq!(h.count("correct"), 0);
            assert_eq!(h.count("shape"), 0);
        }
    }
}
#[test]
fn target_membership_survives_same_read_invalid_layout_or_layer_as_history_only() {
    for drift in 0..3 {
        let mut h = Fake::new(true, false, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.target();
        match drift {
            0 => h.layout = Err(DestinationError::Changed),
            1 => h.current.layer = L::Above,
            _ => h.current.minimized = true,
        }
        check_result(h.run(&mut op).unwrap(), Status::Partial, true);
    }
}
#[test]
fn correction_failure_or_mismatch_never_sends_twice_or_claims_geometry() {
    for remote in [false, true] {
        for native_error in [false, true] {
            let mut h = Fake::new(remote, false, L::Normal);
            h.actual = Rect::new(-30, 999, 160, 160);
            h.refuse_correction = true;
            let mut op = h.operation();
            if remote {
                assert!(h.run(&mut op).is_none());
                h.target();
            }
            if native_error {
                h.faults.push(("correct", 1));
            }
            if native_error {
                assert!(service_bring_here_with(&mut op, &mut h).is_err());
            } else {
                assert!(h.run(&mut op).is_none());
                assert!(!remote_service_due(&op, h.now) || op.placement.observation_due());
                assert!(h.run(&mut op).is_none());
                assert!(!remote_service_due(&op, h.now));
                h.now = op.placement.correction().unwrap().1;
                check_result(
                    h.run(&mut op).unwrap(),
                    if remote {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    remote,
                );
            }
            assert_eq!(h.count("correct"), 1);
            assert_eq!(h.count("shape"), 0);
            assert!(h.runtime.input_fence.automatic_correction_blocked);
        }
    }
}
#[test]
fn fixed_size_and_full_correction_interval_are_required() {
    for invalid_size in [false, true] {
        let mut h = Fake::new(true, false, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.target();
        h.actual = Rect::new(999, 999, if invalid_size { 159 } else { 160 }, 160);
        if !invalid_size {
            h.now = op.deadline - Duration::from_millis(149);
        }
        let result = h.run(&mut op).unwrap();
        check_result(result, Status::Partial, true);
        assert_eq!(h.count("correct"), 0);
        assert_eq!(h.count("shape"), 0);
    }
}
#[test]
fn crossing_each_read_boundary_never_starts_an_expired_mutation() {
    for step in ["owner", "layout", "observe", "geometry", "empty"] {
        let mut h = Fake::new(false, false, L::Normal);
        h.runtime.input_fence.enabled_after = None;
        let mut op = h.operation();
        h.advances.push_back((step, op.deadline - h.now));
        let result = h.run(&mut op).unwrap();
        check_result(result, Status::Failed, false);
        assert_eq!(h.count("desktop"), 0);
        assert_eq!(h.count("correct"), 0);
        assert_eq!(h.count("shape"), 0);
    }
}
#[test]
fn deadline_final_read_distinguishes_complete_minimized_and_incomplete_without_notifications() {
    for minimized in [false, true] {
        for target in [false, true] {
            let mut h = Fake::new(true, minimized, L::Above);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.now = op.deadline;
            if target {
                h.target();
            }
            let result = h.run(&mut op).unwrap();
            check_result(
                result,
                if target && minimized {
                    Status::Success
                } else if target {
                    Status::Partial
                } else {
                    Status::Failed
                },
                target,
            );
            assert_eq!(h.count("desktop"), 1);
            assert_eq!(h.count("shape"), 0);
            assert_eq!(h.count("correct"), 0);
        }
    }
}
#[test]
fn ownership_loss_and_body_destruction_stop_all_mutation_and_keep_history() {
    for history in [false, true] {
        for destroyed in [false, true] {
            let mut h = Fake::new(true, false, L::Normal);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            if history {
                h.target();
                h.current.transitional = true;
                assert!(h.run(&mut op).is_none());
            }
            if destroyed {
                h.window.window_resource.externally_destroyed();
                h.runtime.unavailable(true);
            } else {
                h.owned = false;
            }
            let before = h.log.len();
            let result = h.run(&mut op).unwrap();
            check_result(
                result,
                if history {
                    Status::Partial
                } else {
                    Status::Failed
                },
                history,
            );
            assert_eq!(
                result.detail().reason,
                if destroyed {
                    Reason::BodyDestroyed
                } else {
                    Reason::OwnershipLost
                }
            );
            assert!(!h.log[before..]
                .iter()
                .any(|s| ["empty", "desktop", "correct", "shape"].contains(s)));
        }
    }
}
#[test]
fn caller_disappearance_and_failed_terminal_send_clear_once_without_undoing_host_progress() {
    for fail_reply in [false, true] {
        let mut h = Fake::new(true, false, L::Below);
        let mut active = Some(h.operation());
        service_here_owner_with(&mut active, &mut h, |_| panic!("pending")).unwrap();
        h.target();
        mark_remote_observation(active.as_mut().unwrap(), false);
        let mut replies = vec![];
        let done = service_here_owner_with(&mut active, &mut h, |r| {
            replies.push(r);
            if fail_reply {
                Err("caller gone".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(done.is_err(), fail_reply);
        assert!(active.is_none());
        assert_eq!(replies.len(), 1);
        check_result(replies[0], Status::Success, true);
        assert!(h.visible_ready());
        service_here_owner_with(&mut active, &mut h, |_| panic!("second reply")).unwrap();
        assert_eq!(h.count("desktop"), 1);
        assert_eq!(h.count("shape"), 1);
    }
}
#[test]
fn restricted_fence_survives_completion_and_every_ordinary_placement_path() {
    let mut h = Fake::new(false, false, L::Normal);
    h.actual = Rect::new(999, 999, 160, 160);
    h.refuse_correction = true;
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.now = op.placement.correction().unwrap().1;
    check_result(h.run(&mut op).unwrap(), Status::Failed, false);
    for _ in 0..4 {
        h.runtime.body = BodyAvailability::Validating;
        h.runtime
            .placement_with(&mut h.window, h.actual, &layout().valid_origins, |_, _| {
                panic!("automatic second correction")
            })
            .unwrap();
        assert_ne!(h.runtime.body, BodyAvailability::Ready);
        assert!(h.runtime.correction.is_none());
        assert!(h.runtime.input_fence.automatic_correction_blocked);
    }
    h.actual = Rect::new(100, 100, 160, 160);
    h.runtime
        .placement_with(
            &mut h.window,
            h.actual,
            &layout().valid_origins,
            |_, _| panic!(),
        )
        .unwrap();
    assert_eq!(h.runtime.body, BodyAvailability::Ready);
    let evidence = h.evidence(2).unwrap();
    assert!(h
        .enable(&evidence, h.actual, h.now + Duration::from_secs(1))
        .unwrap());
    assert!(!h.runtime.input_fence.automatic_correction_blocked);
}
#[test]
fn restricted_fence_does_not_take_a_new_here_commands_correction_authority() {
    let mut h = Fake::new(false, false, L::Normal);
    h.fence().unwrap();
    h.restrict_automatic_correction();
    let mut op = h.operation();
    h.actual = Rect::new(999, 999, 160, 160);
    assert!(h.run(&mut op).is_none());
    assert_eq!(h.count("correct"), 1);
    check_result(h.run(&mut op).unwrap(), Status::Success, false);
}
#[test]
fn minimized_deferral_leaves_ordinary_future_correction_available() {
    let mut h = Fake::new(false, true, L::Normal);
    let mut op = h.operation();
    check_result(h.run(&mut op).unwrap(), Status::Success, false);
    h.current.minimized = false;
    h.runtime.body = BodyAvailability::Validating;
    let mut requests = 0;
    h.runtime
        .placement_with(
            &mut h.window,
            Rect::new(999, 999, 160, 160),
            &layout().valid_origins,
            |_, _| {
                requests += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(requests, 1);
}
fn atoms() -> (x11::visibility::VisibilityAtoms, x11::monitors::LayoutAtoms) {
    (
        x11::visibility::VisibilityAtoms {
            change_state: 11,
            state: 12,
            hidden: 13,
            above: 14,
            below: 15,
            desktop: 16,
            current_desktop: 17,
            supported: 18,
            wm_state: 19,
            user_time: 20,
        },
        x11::monitors::LayoutAtoms {
            current_desktop: 17,
            workarea: 21,
            wm_desktop: 16,
            number_of_desktops: 22,
            desktop_viewport: 23,
        },
    )
}
fn property(window: u32, atom: u32) -> Event {
    Event::PropertyNotify(x11rb::protocol::xproto::PropertyNotifyEvent {
        window,
        atom,
        ..Default::default()
    })
}
fn event(kind: usize) -> Event {
    use x11rb::protocol::xproto::*;
    match kind {
        0 => property(100, 16),
        1 => property(100, 12),
        2 => property(100, 19),
        3 => Event::MapNotify(MapNotifyEvent {
            window: 100,
            event: 100,
            ..Default::default()
        }),
        4 => Event::UnmapNotify(UnmapNotifyEvent {
            window: 100,
            event: 100,
            ..Default::default()
        }),
        5 => Event::ConfigureNotify(ConfigureNotifyEvent {
            window: 100,
            x: 999,
            y: 999,
            width: 160,
            height: 160,
            ..Default::default()
        }),
        6 => property(1, 17),
        7 => property(1, 22),
        8 => property(1, 21),
        9 => property(1, 23),
        10 => property(1, 18),
        11 => Event::ConfigureNotify(ConfigureNotifyEvent {
            window: 1,
            ..Default::default()
        }),
        12 => Event::DestroyNotify(DestroyNotifyEvent {
            window: 100,
            event: 100,
            ..Default::default()
        }),
        _ => Event::RandrScreenChangeNotify(Default::default()),
    }
}
#[test]
fn every_here_evidence_event_is_consumed_before_legacy_placement_and_map_paths() {
    let (visibility, layout) = atoms();
    let mut h = Fake::new(true, false, L::Normal);
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    let history = op.placement.proof.history;
    for kind in 0..14 {
        let e = event(kind);
        assert!(route_here_evidence_event(
            Some(&mut op),
            &e,
            1,
            100,
            &visibility,
            &layout
        ));
        assert!(op.placement.observation_due());
        assert_eq!(op.placement.proof.history, history);
    }
    assert!(!route_here_evidence_event(
        Some(&mut op),
        &property(999, 22),
        1,
        100,
        &visibility,
        &layout
    ));
    assert!(!route_here_evidence_event(
        None,
        &event(0),
        1,
        100,
        &visibility,
        &layout
    ));
    assert!(!local_layer_service_allowed(Some(&op)));
}
#[test]
fn desktop_map_state_event_permutations_duplicates_delays_converge_only_through_reads() {
    let (visibility, layout) = atoms();
    for order in [
        [0, 3, 2],
        [0, 2, 3],
        [3, 0, 2],
        [3, 2, 0],
        [2, 0, 3],
        [2, 3, 0],
    ] {
        let mut h = Fake::new(true, false, L::Below);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        for &kind in &order {
            assert!(route_here_evidence_event(
                Some(&mut op),
                &event(kind),
                1,
                100,
                &visibility,
                &layout
            ));
            assert!(h.run(&mut op).is_none()); // notifications alone cannot prove membership
            assert!(!remote_service_due(&op, h.now));
        }
        h.target();
        h.current.transitional = true;
        route_here_evidence_event(Some(&mut op), &event(4), 1, 100, &visibility, &layout);
        assert!(h.run(&mut op).is_none());
        assert!(op.placement.proof.history.relocation_confirmed);
        assert_eq!(h.count("shape"), 0);
        assert_eq!(h.count("geometry"), 0);
        h.current.transitional = false;
        route_here_evidence_event(
            Some(&mut op),
            &event(order[2]),
            1,
            100,
            &visibility,
            &layout,
        );
        check_result(h.run(&mut op).unwrap(), Status::Success, true);
        let mut active = None;
        for &kind in &order {
            assert!(!route_here_evidence_event(
                active.as_mut(),
                &event(kind),
                1,
                100,
                &visibility,
                &layout
            ));
        }
        assert_eq!(h.count("desktop"), 1);
        assert_eq!(h.count("correct"), 0);
        assert_eq!(h.count("shape"), 1);
        // A late map has no new silhouette synchronization allowance.
        let Event::MapNotify(map) = event(3) else {
            unreachable!()
        };
        h.runtime
            .synchronize_map_input_with(&map, 100, 9999, || {
                panic!("late map must not reassert shape")
            })
            .unwrap();
    }
}
fn owner_tick(
    active: &mut Option<ActiveOperation>,
    h: &mut Fake,
    replies: &mut Vec<control::OperationResult>,
) {
    service_here_owner_with(active, h, |r| {
        replies.push(r);
        Ok(())
    })
    .unwrap();
}
#[test]
fn sustained_traffic_over_64_events_cannot_starve_total_or_correction_deadline() {
    for correction in [false, true] {
        let (visibility, layout) = atoms();
        let mut h = Fake::new(!correction, false, L::Normal);
        if correction {
            h.actual = Rect::new(999, 999, 160, 160);
            h.refuse_correction = true;
        }
        let mut active = Some(h.operation());
        let immutable = active.unwrap().deadline;
        let mut replies = vec![];
        owner_tick(&mut active, &mut h, &mut replies);
        owner_tick(&mut active, &mut h, &mut replies); // consumes correction's immediate observation
        let expected = remote_operation_deadline(active.as_ref().unwrap());
        assert_eq!(
            expected,
            if correction {
                h.now + CONFIRMATION_TIMEOUT
            } else {
                immutable
            }
        );
        let mut queue = VecDeque::from(vec![property(999, 99); 5000]);
        let mut processed = 0;
        while active.is_some() {
            let batch: Vec<_> =
                drain_events_bounded(&mut None, 64, || Ok::<_, HostError>(queue.pop_front()))
                    .unwrap();
            for e in batch {
                h.now += Duration::from_millis(1);
                processed += 1;
                if active
                    .as_ref()
                    .is_some_and(|op| h.now >= remote_operation_deadline(op))
                {
                    owner_tick(&mut active, &mut h, &mut replies);
                    if active.is_none() {
                        assert_eq!(h.now, expected);
                    }
                }
                route_here_evidence_event(active.as_mut(), &e, 1, 100, &visibility, &layout);
            }
            owner_tick(&mut active, &mut h, &mut replies);
            if queue.is_empty() {
                // no notifications: wait advances to the fixed deadline, no sleep
                if let Some(op) = active {
                    h.now = remote_operation_deadline(&op);
                }
                owner_tick(&mut active, &mut h, &mut replies);
            }
        }
        assert!(processed > 64);
        assert_eq!(replies.len(), 1);
        check_result(replies[0], Status::Failed, false);
        assert!(h.now >= expected);
        assert_eq!(h.count("correct"), usize::from(correction));
        assert_eq!(h.count("desktop"), usize::from(!correction));
        assert!(
            h.count("observe") < 12,
            "irrelevant traffic must not cause polling"
        );
    }
}
#[test]
fn correction_deadline_also_finishes_transitional_readiness_without_spinning() {
    let mut h = Fake::new(false, false, L::Normal);
    h.actual = Rect::new(999, 999, 160, 160);
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.current.transitional = true;
    h.now = op.placement.correction().unwrap().1;
    check_result(h.run(&mut op).unwrap(), Status::Failed, false);
    assert_eq!(h.count("correct"), 1);
    assert_eq!(h.count("shape"), 0);
}
#[test]
fn both_consumed_request_sites_and_end_of_batch_progress_before_wait() {
    for site in ["batch", "lookahead"] {
        let mut h = Fake::new(true, false, L::Normal);
        let mut active = None;
        route_owner_request_with(
            &mut active,
            &mut h,
            |active, h| {
                h.log.push(site);
                *active = Some(h.operation());
                Ok(true)
            },
            |active, h| service_here_owner_with(active, h, |_| panic!("pending")),
        )
        .unwrap();
        assert_eq!(h.count("desktop"), 1);
        assert!(active.is_some());
        let reads = h.count("observe");
        let mut replies = vec![];
        owner_tick(&mut active, &mut h, &mut replies); // one immediate post-send observation
        assert!(!remote_service_due(active.as_ref().unwrap(), h.now));
        let settled = h.count("observe");
        owner_tick(&mut active, &mut h, &mut replies);
        assert_eq!(h.count("observe"), settled);
        assert_eq!(settled, reads + 1);
        h.target();
        mark_remote_observation(active.as_mut().unwrap(), false);
        owner_tick(&mut active, &mut h, &mut replies);
        assert!(active.is_none());
        assert_eq!(replies.len(), 1);
    }
}
#[test]
fn sticky_is_unsupported_and_cannot_be_converted_to_an_ordinary_desktop() {
    let mut h = Fake::new(false, false, L::Normal);
    h.current.desktop = u32::MAX;
    let mut op = h.operation();
    let result = h.run(&mut op).unwrap();
    assert_eq!(result.detail().reason, Reason::Unsupported);
    h.assert_no_mutation();
}
#[test]
fn final_native_error_after_enable_has_one_safety_refence_and_never_success() {
    let mut h = Fake::new(true, false, L::Normal);
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.target();
    // arrival read, geometry bracket, then the final proof's first layout read
    let at = h.count("layout") + 5;
    h.faults.push(("layout", at));
    h.faults.push(("empty", 2));
    let err = service_bring_here_with(&mut op, &mut h).unwrap_err();
    assert_eq!(
        err.downcast_ref::<NativeFault>(),
        Some(&NativeFault("layout", at))
    );
    assert_eq!(h.count("shape"), 1);
    assert_eq!(h.count("fence"), 2);
    assert!(h.runtime.input_fence.blocked);
    assert_eq!(op.detail.stage, Stage::Shutdown);
    check_result(
        op.placement
            .here_result(op.detail, control::Confirmation::default())
            .unwrap(),
        Status::Partial,
        true,
    );
}
pub(super) fn lifecycle_snapshots() -> Vec<(&'static str, ActiveOperation)> {
    let mut result = vec![];
    let mut h = Fake::new(true, false, L::Normal);
    let op = h.operation();
    result.push(("observe", op));
    let mut fenced = op;
    h.reads = VecDeque::from([
        h.current,
        Reading {
            transitional: true,
            desktop: 2,
            ..h.current
        },
    ]);
    // Use independent unreadable layer evidence after the fence; baseline remains original.
    h.reads[1] = Reading {
        layer: L::Unknown,
        ..h.current
    };
    assert!(h.run(&mut fenced).is_none());
    assert_eq!(fenced.placement.here_phase(), HerePhase::Fenced);
    result.push(("fenced", fenced));
    let mut h = Fake::new(true, false, L::Normal);
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    result.push(("await", op));
    h.target();
    h.current.transitional = true;
    assert!(h.run(&mut op).is_none());
    result.push(("membership", op));
    h.current.transitional = false;
    h.actual = Rect::new(999, 999, 160, 160);
    assert!(h.run(&mut op).is_none());
    result.push(("correction", op));
    h.faults.push(("shape", 1));
    assert!(service_bring_here_with(&mut op, &mut h).is_err());
    result.push(("enable-failed", op));
    let mut h = Fake::new(true, false, L::Normal);
    let mut op = h.operation();
    h.faults.push(("desktop", 1));
    assert!(service_bring_here_with(&mut op, &mut h).is_err());
    result.push(("desktop-attempted", op));
    let mut h = Fake::new(true, false, L::Normal);
    let mut op = h.operation();
    assert!(h.run(&mut op).is_none());
    h.target();
    let mut changed = h.current;
    changed.layer = L::Below;
    h.changes.push_back(("shape", changed));
    h.run(&mut op).unwrap();
    result.push(("input-restored", op));
    result
}
#[test]
fn final_ownership_loss_cannot_report_success_in_visible_or_minimized_branches() {
    for (remote, minimized, lost_at) in [(false, false, 6), (true, false, 12), (true, true, 8)] {
        let mut h = Fake::new(remote, minimized, L::Above);
        let mut op = h.operation();
        if remote {
            assert!(h.run(&mut op).is_none());
            h.target();
        }
        h.lose_at = Some(("layout", lost_at));
        let result = h.run(&mut op).unwrap();
        check_result(
            result,
            if remote {
                Status::Partial
            } else {
                Status::Failed
            },
            remote,
        );
        assert_eq!(result.detail().reason, Reason::OwnershipLost);
        assert_eq!(result.detail().stage, Stage::Shutdown);
        assert_eq!(h.count("fence"), usize::from(remote));
    }
}
#[test]
fn ownership_loss_ends_owner_service_after_one_reply_even_if_reply_fails() {
    for fail_reply in [false, true] {
        let mut h = Fake::new(true, false, L::Normal);
        let mut active = Some(h.operation());
        service_here_owner_with(&mut active, &mut h, |_| panic!("pending")).unwrap();
        h.owned = false;
        mark_remote_observation(active.as_mut().unwrap(), false);
        let mut replies = vec![];
        let error = service_here_owner_with(&mut active, &mut h, |result| {
            replies.push(result);
            if fail_reply {
                Err("reply lost".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Control selection ownership lost during Bring Here"
        );
        assert!(active.is_none());
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].detail().reason, Reason::OwnershipLost);
        assert_eq!(h.count("desktop"), 1);
        assert_eq!(h.count("fence"), 1);
        assert_eq!(h.count("shape"), 0);
    }
}
#[test]
fn withdrawn_and_destroyed_observations_clear_current_proof_and_keep_real_history() {
    for history in [false, true] {
        for terminal in [
            control::Visibility::Withdrawn,
            control::Visibility::Destroyed,
        ] {
            let mut h = Fake::new(true, false, L::Normal);
            let mut op = h.operation();
            if history {
                assert!(h.run(&mut op).is_none());
                h.target();
                h.current.transitional = true;
                assert!(h.run(&mut op).is_none());
            }
            h.terminal = Some(terminal);
            let mutations = (h.count("desktop"), h.count("correct"), h.count("shape"));
            let result = h.run(&mut op).unwrap();
            check_result(
                result,
                if history {
                    Status::Partial
                } else {
                    Status::Failed
                },
                history,
            );
            assert_eq!(
                result.detail().reason,
                if terminal == control::Visibility::Destroyed {
                    Reason::BodyDestroyed
                } else {
                    Reason::Withdrawn
                }
            );
            assert_eq!(
                (h.count("desktop"), h.count("correct"), h.count("shape")),
                mutations
            );
            if terminal == control::Visibility::Destroyed {
                assert!(!h.live());
            }
        }
    }
}
#[test]
fn original_native_error_survives_cleanup_final_native_read_and_reply_all_failing() {
    let mut h = Fake::new(true, false, L::Normal);
    let mut active = Some(h.operation());
    service_here_owner_with(&mut active, &mut h, |_| panic!("pending")).unwrap();
    h.target();
    h.faults = vec![("shape", 1), ("empty", 2), ("layout", 9)];
    let mut replies = vec![];
    let error = service_here_owner_with(&mut active, &mut h, |result| {
        replies.push(result);
        Err("reply also failed".into())
    })
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<NativeFault>(),
        Some(&NativeFault("shape", 1))
    );
    assert_eq!(h.count("fence"), 2);
    assert_eq!(h.count("shape"), 1);
    assert_eq!(h.count("layout"), 9);
    assert!(active.is_none());
    assert_eq!(replies.len(), 1);
    check_result(replies[0], Status::Partial, true);
    assert_eq!(replies[0].detail().reason, Reason::X11Failure);
}
#[test]
fn all_service_phase_snapshots_have_truthful_shutdown_and_monotonic_history() {
    for (_, mut op) in lifecycle_snapshots() {
        let history = op.placement.proof.history.relocation_confirmed;
        let result = op.here_shutdown(Reason::Closing).unwrap();
        check_result(
            result,
            if history {
                Status::Partial
            } else {
                Status::Failed
            },
            history,
        );
        assert_eq!(result.detail().stage, Stage::Shutdown);
        assert_eq!(result.detail().reason, Reason::Closing);
    }
}
use x11::control::transport_tests::HereOwner;

pub(crate) fn public_roundtrip_service(
    active: &mut Option<ActiveOperation>,
    remote: bool,
    minimized: bool,
    layer: L,
    reply: &mut dyn FnMut(control::OperationResult) -> Result<(), HostError>,
) -> Result<(), HostError> {
    let mut host = Fake::new(remote, minimized, layer);
    host.window.window = 88;
    assert_eq!(active.as_ref().unwrap().placement.destination_desktop, 2);
    service_here_owner_with(active, &mut host, &mut *reply)?;
    if remote {
        host.target();
        mark_remote_observation(active.as_mut().unwrap(), false);
        service_here_owner_with(active, &mut host, &mut *reply)?;
    }
    assert!(active.is_none());
    assert_eq!(host.count("desktop"), usize::from(remote));
    assert_eq!(host.window.window, 88);
    Ok(())
}

fn public_tick(owner: &mut HereOwner, host: &mut Fake) -> Result<bool, HostError> {
    // Same scoped transport callback and D3 finalizer as production; only
    // the borrowed NativeHere adapter is replaced with its injected host.
    owner.service(|active, selection, control_xid, reply| {
        assert_eq!((selection, control_xid), (50, 10));
        service_here_owner_with(active, host, reply)
    })
}
fn public_owner(host: &mut Fake) -> HereOwner {
    // The fixture publishes 88 as its existing body; no create/map API is
    // available in the Here service. Track that exact resource throughout.
    host.window.window = 88;
    HereOwner::new(host.now, 2)
}
fn public_terminal(owner: &HereOwner, status: Status, history: bool) -> control::TerminalResponse {
    assert!(owner.active().is_none());
    let replies = owner.replies();
    assert_eq!(replies.len(), 1);
    let reply = replies[0];
    assert_eq!(reply.request_id, owner.original_id());
    assert_eq!(reply.status, status);
    assert_eq!(reply.detail.placement.history.relocation_confirmed, history);
    assert!(!reply.detail.placement.history.restoration_confirmed);
    let caller = x11::control::CallerOutcome::Terminal(reply);
    let before = caller.exit_code();
    let text = caller.diagnostic(Command::BringHere);
    assert!(text.starts_with("bring-here:"));
    assert_eq!(before, status.exit_code());
    assert_eq!(
        caller.exit_code(),
        before,
        "diagnostics cannot reclassify results"
    );
    reply
}
#[test]
fn public_admission_dispatch_and_completion_preserve_each_layer_and_minimization() {
    for remote in [false, true] {
        for minimized in [false, true] {
            for layer in [L::Normal, L::Above, L::Below] {
                let mut host = Fake::new(remote, minimized, layer);
                let mut owner = public_owner(&mut host);
                assert!(public_tick(&mut owner, &mut host).unwrap());
                if remote {
                    assert!(owner.active().is_some());
                    assert_eq!(host.count("desktop"), 1);
                    let deadline = owner.active().unwrap().deadline;
                    // D3 schedules one fresh observation after checked dispatch.
                    assert!(public_tick(&mut owner, &mut host).unwrap());
                    let calls = host.log.len();
                    assert!(public_tick(&mut owner, &mut host).unwrap());
                    assert_eq!(host.log.len(), calls);
                    assert_eq!(owner.active().unwrap().deadline, deadline);
                    let send = host.log.iter().position(|s| *s == "desktop").unwrap();
                    let release = host.log.iter().position(|s| *s == "release").unwrap();
                    assert!(release < send);
                    assert!(host.log[release..send].contains(&"layout"));
                    host.target();
                    mark_remote_observation(owner.active_mut().unwrap(), false);
                    assert!(public_tick(&mut owner, &mut host).unwrap());
                }
                let reply = public_terminal(&owner, Status::Success, remote);
                assert_eq!(reply.detail.layer, layer);
                assert_eq!(reply.detail.progress.placement_deferred, minimized);
                assert_eq!(reply.detail.progress.input_ready, !minimized);
                let diagnostic =
                    x11::control::CallerOutcome::Terminal(reply).diagnostic(Command::BringHere);
                assert!(diagnostic.contains("layer"));
                assert!(diagnostic.contains("preserved"));
                if minimized {
                    assert!(diagnostic.contains("remaining genuinely minimized"));
                    assert!(diagnostic.contains("placement/input deferred"));
                    assert!(!diagnostic.contains("restored"));
                    assert_eq!(host.count("shape"), 0);
                    assert_eq!(host.count("geometry"), 0);
                } else {
                    assert!(diagnostic.contains("placement verified, input ready"));
                    assert_eq!(diagnostic.contains("relocated"), remote);
                    assert_eq!(host.count("shape"), usize::from(remote));
                }
                assert_eq!(host.count("desktop"), usize::from(remote));
                assert_eq!(host.count("correct"), 0);
                if !remote && !minimized {
                    host.assert_no_mutation();
                }
                assert_eq!(host.window.window, 88);
                assert!(host.runtime.layers.pending().is_none());
                let log = host.log.clone();
                assert!(!public_tick(&mut owner, &mut host).unwrap());
                assert_eq!(host.log, log);
                assert_eq!(owner.replies().len(), 1);
            }
        }
    }
}
#[test]
fn public_local_repair_uses_d3_correction_without_relocation_or_false_moved_diagnostic() {
    let mut host = Fake::new(false, false, L::Below);
    host.actual = Rect::new(900, -30, 160, 160);
    let mut owner = public_owner(&mut host);
    assert!(public_tick(&mut owner, &mut host).unwrap());
    assert_eq!(host.count("correct"), 1);
    mark_remote_observation(owner.active_mut().unwrap(), false);
    assert!(public_tick(&mut owner, &mut host).unwrap());
    let reply = public_terminal(&owner, Status::Success, false);
    let text = x11::control::CallerOutcome::Terminal(reply).diagnostic(Command::BringHere);
    assert!(!text.contains("relocated"));
    assert!(!text.contains("moved"));
    assert!(text.contains("confirmed on caller's frozen workspace"));
    assert_eq!(host.actual.origin(), Point::new(640, 0));
    assert_eq!(host.count("desktop"), 0);
    assert_eq!(host.count("correct"), 1);
    assert_eq!(host.count("shape"), 1);
}
#[test]
fn public_existing_placement_is_busy_without_body_calls() {
    let mut host = Fake::new(true, false, L::Normal);
    host.runtime.correction = Some((Point::new(0, 0), host.now));
    let mut owner = public_owner(&mut host);
    assert!(public_tick(&mut owner, &mut host).unwrap());
    public_terminal(&owner, Status::Busy, false);
    assert_eq!(host.count("layout"), 0);
    host.assert_no_mutation();
    assert!(host.runtime.correction.is_some());
}
#[test]
fn public_out_of_range_destination_never_falls_back_to_zero() {
    let mut host = Fake::new(true, false, L::Normal);
    let mut owner = HereOwner::new(host.now, 9);
    assert_eq!(owner.active().unwrap().placement.destination_desktop, 9);
    public_tick(&mut owner, &mut host).unwrap();
    let reply = public_terminal(&owner, Status::Failed, false);
    assert_eq!(reply.detail.reason, Reason::LayoutChanged);
    host.assert_no_mutation();
}
#[test]
fn public_duplicate_busy_and_changed_marker_cannot_restart_or_retarget_here() {
    let mut host = Fake::new(true, false, L::Normal);
    let mut owner = public_owner(&mut host);
    public_tick(&mut owner, &mut host).unwrap();
    public_tick(&mut owner, &mut host).unwrap(); // D3 post-dispatch observation
    let original = *owner.active().unwrap();
    owner.advance_peer(Duration::from_millis(800));
    owner.request(owner.original_id(), 2, false);
    assert_eq!(*owner.active().unwrap(), original);
    assert!(owner.replies().is_empty());
    let log = host.log.clone();
    public_tick(&mut owner, &mut host).unwrap();
    assert_eq!(host.log, log);
    owner.request(owner.original_id(), 3, false);
    owner.request(owner.original_id() + 1, 2, false);
    assert_eq!(*owner.active().unwrap(), original);
    assert_eq!(owner.replies().len(), 2);
    assert!(owner.replies().iter().all(|r| r.status == Status::Busy));
    host.target();
    mark_remote_observation(owner.active_mut().unwrap(), false);
    public_tick(&mut owner, &mut host).unwrap();
    assert!(owner.active().is_none());
    assert_eq!(host.count("desktop"), 1);
    assert_eq!(owner.replies().len(), 3);
    assert_eq!(owner.replies()[2].status, Status::Success);
}
#[test]
fn public_frozen_target_and_current_drift_invalidate_without_chasing() {
    for phase in 0..3 {
        let after_history = phase == 2;
        let mut host = Fake::new(true, false, L::Above);
        let mut owner = public_owner(&mut host);
        if phase > 0 {
            public_tick(&mut owner, &mut host).unwrap();
        }
        if after_history {
            host.target();
            host.current.transitional = true;
            mark_remote_observation(owner.active_mut().unwrap(), false);
            public_tick(&mut owner, &mut host).unwrap();
            assert!(
                owner
                    .active()
                    .unwrap()
                    .placement
                    .proof
                    .history
                    .relocation_confirmed
            );
        }
        host.layout = Err(DestinationError::Changed);
        if let Some(op) = owner.active_mut() {
            mark_remote_observation(op, false);
        }
        public_tick(&mut owner, &mut host).unwrap();
        let reply = public_terminal(
            &owner,
            if after_history {
                Status::Partial
            } else {
                Status::Failed
            },
            after_history,
        );
        assert_eq!(reply.detail.reason, Reason::LayoutChanged);
        let text = x11::control::CallerOutcome::Terminal(reply).diagnostic(Command::BringHere);
        assert!(text.contains("workspace/layout changed"));
        assert_eq!(host.count("desktop"), usize::from(phase > 0));
        assert_eq!(host.count("correct"), 0);
        assert_eq!(host.count("shape"), 0);
    }
}
#[test]
fn public_native_failure_uses_d3_history_and_preserves_original_error_over_reply_failure() {
    for history in [false, true] {
        let mut host = Fake::new(true, false, L::Normal);
        let mut owner = public_owner(&mut host);
        let fault = if history {
            public_tick(&mut owner, &mut host).unwrap();
            host.target();
            mark_remote_observation(owner.active_mut().unwrap(), false);
            ("shape", 1)
        } else {
            ("desktop", 1)
        };
        host.faults.push(fault);
        owner.fail_reply(true);
        let error = public_tick(&mut owner, &mut host).unwrap_err();
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault(fault.0, fault.1))
        );
        let reply = public_terminal(
            &owner,
            if history {
                Status::Partial
            } else {
                Status::Failed
            },
            history,
        );
        assert_eq!(reply.detail.reason, Reason::X11Failure);
        let text = x11::control::CallerOutcome::Terminal(reply).diagnostic(Command::BringHere);
        assert!(text.contains("X11 failure"));
        if history {
            assert!(text.contains("actual relocation was confirmed earlier"));
        } else {
            assert!(text.contains("no accepted relocation history"));
            assert!(text.contains("later WM movement remains possible"));
        }
        assert_eq!(host.count("desktop"), 1);
        assert!(!public_tick(&mut owner, &mut host).unwrap());
    }
}
#[test]
fn public_owner_loss_keeps_history_and_never_mutates_after_loss() {
    for history in [false, true] {
        let mut host = Fake::new(true, false, L::Below);
        let mut owner = public_owner(&mut host);
        if history {
            public_tick(&mut owner, &mut host).unwrap();
            host.target();
            host.current.transitional = true;
            mark_remote_observation(owner.active_mut().unwrap(), false);
            public_tick(&mut owner, &mut host).unwrap();
        }
        host.owned = false;
        mark_remote_observation(owner.active_mut().unwrap(), false);
        let mutations = (
            host.count("desktop"),
            host.count("correct"),
            host.count("shape"),
            host.count("fence"),
        );
        assert!(public_tick(&mut owner, &mut host).is_err());
        let reply = public_terminal(
            &owner,
            if history {
                Status::Partial
            } else {
                Status::Failed
            },
            history,
        );
        assert_eq!(reply.detail.reason, Reason::OwnershipLost);
        assert_eq!(
            mutations,
            (
                host.count("desktop"),
                host.count("correct"),
                host.count("shape"),
                host.count("fence")
            )
        );
    }
}
#[test]
fn public_caller_disappearance_reply_failure_and_shutdown_clear_once() {
    for fatal in [false, true] {
        let mut host = Fake::new(true, true, L::Above);
        let mut owner = public_owner(&mut host);
        public_tick(&mut owner, &mut host).unwrap();
        owner.fail_reply(fatal);
        host.target();
        mark_remote_observation(owner.active_mut().unwrap(), false);
        assert_eq!(public_tick(&mut owner, &mut host).is_err(), fatal);
        public_terminal(&owner, Status::Success, true);
        assert_eq!(host.count("desktop"), 1);
        assert!(!public_tick(&mut owner, &mut host).unwrap());
    }
    for history in [false, true] {
        for owned in [false, true] {
            let mut host = Fake::new(true, false, L::Normal);
            let mut owner = public_owner(&mut host);
            public_tick(&mut owner, &mut host).unwrap();
            if history {
                host.target();
                host.current.transitional = true;
                mark_remote_observation(owner.active_mut().unwrap(), false);
                public_tick(&mut owner, &mut host).unwrap();
            }
            owner.shutdown(owned);
            let reply = public_terminal(
                &owner,
                if history {
                    Status::Partial
                } else {
                    Status::Failed
                },
                history,
            );
            assert_eq!(reply.detail.stage, Stage::Shutdown);
            assert_eq!(
                reply.detail.reason,
                if owned {
                    Reason::Closing
                } else {
                    Reason::OwnershipLost
                }
            );
            owner.shutdown(owned);
            assert_eq!(owner.replies().len(), 1);
            assert_eq!(host.count("desktop"), 1);
        }
    }
}
#[test]
fn public_here_partial_and_failed_diagnostics_explain_reason_without_exit_changes() {
    for history in [false, true] {
        for (reason, expected) in [
            (Reason::LayoutChanged, "workspace/layout changed"),
            (Reason::LayerChanged, "effective layer changed"),
            (Reason::ConfirmationTimeout, "confirmation timed out"),
            (Reason::X11Failure, "X11 failure"),
            (Reason::OwnershipLost, "owner selection lost"),
            (Reason::Closing, "owner shutdown"),
        ] {
            let mut host = Fake::new(true, false, L::Normal);
            let mut op = host.operation();
            host.run(&mut op);
            if history {
                host.target();
                host.current.transitional = true;
                host.run(&mut op);
            }
            let result = op.here_shutdown(reason).unwrap();
            let reply = control::TerminalResponse {
                status: result.status(),
                detail: result.detail(),
                request_id: 99,
                control_xid: 10,
            };
            let outcome = x11::control::CallerOutcome::Terminal(reply);
            let text = outcome.diagnostic(Command::BringHere);
            assert!(text.contains(expected));
            assert!(text.contains(if history {
                "actual relocation was confirmed earlier"
            } else {
                "no accepted relocation history"
            }));
            assert!(!text.contains("rollback"));
            assert_eq!(
                outcome.exit_code(),
                if history {
                    control::ExitCode::Partial
                } else {
                    control::ExitCode::OperationFailed
                }
            );
        }
    }
}
fn external_public_tick(owner: &mut HereOwner, host: &mut Fake, original: HostError) -> HostError {
    // The production loop boundary and ControlWindow dispatch are real;
    // only NativeHere's borrowed X11 operations use the existing fake.
    finish_probe_run_with(Err(original), |original| {
        let mut original = Some(original);
        match owner.service(|active, _, _, reply| {
            fail_here_owner_with(active, host, reply, original.take().unwrap())
        }) {
            Err(original) => original,
            Ok(false) => original.unwrap(),
            Ok(true) => panic!("fatal finalization must return its original error"),
        }
    })
    .unwrap_err()
}
fn error_address(error: &HostError) -> *const () {
    &**error as *const dyn std::error::Error as *const ()
}
fn assert_fatal_terminal(owner: &mut HereOwner, history: bool) {
    let reply = public_terminal(
        owner,
        if history {
            Status::Partial
        } else {
            Status::Failed
        },
        history,
    );
    assert_eq!(reply.detail.reason, Reason::X11Failure);
    assert_eq!(reply.detail.stage, Stage::Shutdown);
    assert_eq!(
        reply.detail.placement.current,
        control::PlacementConfirmation::default()
    );
    assert!(!reply.detail.progress.input_ready);
    // Actual global cleanup uses finish_remote; neither Closing nor a
    // later selection-loss classification can emit a second response.
    owner.shutdown(true);
    owner.shutdown(false);
    assert!(owner.active().is_none());
    assert_eq!(owner.replies().len(), 1);
}
#[test]
fn production_first_ownership_query_failure_is_failed_x11_and_clears_before_cleanup() {
    let mut host = Fake::new(true, false, L::Normal);
    let mut owner = public_owner(&mut host);
    host.faults.push(("owner", 1));
    let error = public_tick(&mut owner, &mut host).unwrap_err();
    assert_eq!(
        error.downcast_ref::<NativeFault>(),
        Some(&NativeFault("owner", 1))
    );
    assert_fatal_terminal(&mut owner, false);
    host.assert_no_mutation();
    assert_eq!(host.log, ["owner"]);
}
#[test]
fn production_ownership_query_failure_after_dispatch_can_observe_history_once() {
    let mut host = Fake::new(true, false, L::Below);
    let mut owner = public_owner(&mut host);
    public_tick(&mut owner, &mut host).unwrap();
    assert!(
        !owner
            .active()
            .unwrap()
            .placement
            .proof
            .history
            .relocation_confirmed
    );
    let query = host.count("owner") + 1;
    let reads = host.count("layout");
    host.target();
    host.faults.push(("owner", query));
    let error = public_tick(&mut owner, &mut host).unwrap_err();
    assert_eq!(
        error.downcast_ref::<NativeFault>(),
        Some(&NativeFault("owner", query))
    );
    assert_fatal_terminal(&mut owner, true);
    assert_eq!(
        host.count("layout"),
        reads + 2,
        "one bounded Here evidence round"
    );
    assert_eq!(host.count("desktop"), 1);
    assert_eq!(host.count("correct"), 0);
    assert_eq!(host.count("shape"), 0);
}
#[test]
fn production_first_ownership_mismatch_remains_ownership_lost_without_mutation() {
    let mut host = Fake::new(true, false, L::Normal);
    let mut owner = public_owner(&mut host);
    host.owned = false;
    assert!(public_tick(&mut owner, &mut host).is_err());
    let reply = public_terminal(&owner, Status::Failed, false);
    assert_eq!(reply.detail.reason, Reason::OwnershipLost);
    host.assert_no_mutation();
    assert_eq!(host.log, ["owner"]);
    owner.shutdown(false);
    assert_eq!(owner.replies().len(), 1);
}
#[test]
fn external_async_before_mutation_preserves_original_error_and_replies_failed_once() {
    let mut host = Fake::new(true, false, L::Above);
    let mut owner = public_owner(&mut host);
    let original: HostError = Box::new(NativeFault("async before service", 1));
    let address = error_address(&original);
    let error = external_public_tick(&mut owner, &mut host, original);
    assert_eq!(error_address(&error), address);
    assert_eq!(
        error.downcast_ref::<NativeFault>(),
        Some(&NativeFault("async before service", 1))
    );
    assert_fatal_terminal(&mut owner, false);
    host.assert_no_mutation();
    assert!(
        host.log.is_empty(),
        "no baseline or dispatch permits no final evidence read"
    );
}
#[test]
fn external_async_after_dispatch_uses_one_final_history_round_without_retry() {
    for (target, prior_history) in [(false, false), (true, false), (true, true)] {
        let mut host = Fake::new(true, false, L::Normal);
        let mut owner = public_owner(&mut host);
        public_tick(&mut owner, &mut host).unwrap();
        if target {
            host.target();
        }
        if prior_history {
            host.current.transitional = true;
            mark_remote_observation(owner.active_mut().unwrap(), false);
            public_tick(&mut owner, &mut host).unwrap();
            assert!(
                owner
                    .active()
                    .unwrap()
                    .placement
                    .proof
                    .history
                    .relocation_confirmed
            );
        }
        let reads = host.count("layout");
        let original: HostError = Box::new(NativeFault("async after dispatch", 1));
        let address = error_address(&original);
        let error = external_public_tick(&mut owner, &mut host, original);
        assert_eq!(error_address(&error), address);
        assert_fatal_terminal(&mut owner, target);
        assert!(owner.replies()[0].detail.progress.host_mutation_dispatched);
        assert_eq!(host.count("layout"), reads + 2);
        assert_eq!(host.count("desktop"), 1);
        assert_eq!(host.count("correct"), 0);
        assert_eq!(host.count("shape"), 0);
    }
}
fn public_input_enable_boundary(owner: &mut HereOwner, host: &mut Fake, enabled: bool) {
    public_tick(owner, host).unwrap();
    host.target();
    let operation = owner.active_mut().unwrap();
    let (evidence, decision) = read_here(operation, host).unwrap();
    assert_eq!(decision, control::HereDecision::Destination);
    assert!(operation.placement.begin_here_geometry());
    assert!(operation
        .placement
        .confirm_here_geometry(host.actual.origin(), host.actual.size()));
    assert!(operation.consume_here_input_enable(host.now));
    if enabled {
        assert!(host
            .enable(&evidence, host.actual, operation.deadline)
            .unwrap());
        assert!(operation.here_input_restored());
    }
}
#[test]
fn external_async_during_or_after_input_enable_refences_once_and_never_enables_again() {
    for enabled in [false, true] {
        let mut host = Fake::new(true, false, L::Above);
        let mut owner = public_owner(&mut host);
        public_input_enable_boundary(&mut owner, &mut host, enabled);
        let fences = host.count("fence");
        let original: HostError = Box::new(NativeFault("async enable boundary", 1));
        let address = error_address(&original);
        let error = external_public_tick(&mut owner, &mut host, original);
        assert_eq!(error_address(&error), address);
        assert_fatal_terminal(&mut owner, true);
        assert_eq!(host.count("fence"), fences + 1);
        assert_eq!(host.count("shape"), usize::from(enabled));
        assert_eq!(host.count("enable"), usize::from(enabled));
        assert_eq!(host.count("desktop"), 1);
        assert!(host.fence_settled());
        assert!(!host.input_active());
    }
}
#[test]
fn external_error_wins_over_safety_final_read_and_reply_failures_together() {
    let mut host = Fake::new(true, false, L::Below);
    let mut owner = public_owner(&mut host);
    public_input_enable_boundary(&mut owner, &mut host, true);
    let empty = host.count("empty");
    let reads = host.count("layout");
    host.faults = vec![("empty", empty + 1), ("layout", reads + 1)];
    owner.fail_reply(true);
    let original: HostError = Box::new(NativeFault("original external error", 99));
    let address = error_address(&original);
    let error = external_public_tick(&mut owner, &mut host, original);
    assert_eq!(error_address(&error), address);
    assert_eq!(
        error.downcast_ref::<NativeFault>(),
        Some(&NativeFault("original external error", 99))
    );
    assert_fatal_terminal(&mut owner, true);
    assert_eq!(host.count("empty"), empty + 1);
    assert_eq!(host.count("release"), 2);
    assert_eq!(host.count("popup"), 2);
    assert_eq!(
        host.count("layout"),
        reads + 1,
        "failed read is never retried"
    );
    assert_eq!(host.count("desktop"), 1);
    assert_eq!(host.count("shape"), 1);
}
#[test]
fn loop_boundary_keeps_clean_shutdown_and_proven_control_loss_out_of_fatal_policy() {
    for termination in [
        Some(ProbeTermination::DurationExpired),
        Some(ProbeTermination::WmClose),
        Some(ProbeTermination::ControlEndpointLost),
        None,
    ] {
        let mut host = Fake::new(true, false, L::Normal);
        let mut owner = public_owner(&mut host);
        public_tick(&mut owner, &mut host).unwrap();
        let result = finish_probe_run_with(Ok(termination), |_| {
            panic!("intentional lifecycle exit is not a native failure")
        });
        let lost = matches!(termination, Some(ProbeTermination::ControlEndpointLost));
        assert_eq!(result.is_err(), lost);
        owner.shutdown(!lost);
        let reply = public_terminal(&owner, Status::Failed, false);
        assert_eq!(
            reply.detail.reason,
            if lost {
                Reason::OwnershipLost
            } else {
                Reason::Closing
            }
        );
        assert_eq!(host.count("desktop"), 1);
    }
}
#[test]
fn external_fatal_policy_leaves_legacy_commands_untouched() {
    for command in [Command::Hide, Command::Show, Command::BringTop] {
        let mut host = Fake::new(true, false, L::Normal);
        let mut active = None;
        let destination = if command == Command::Recover {
            2
        } else {
            control::NO_DESTINATION
        };
        control::admit(
            &mut active,
            control::Request {
                command,
                request_id: 99,
                reply_xid: 88,
                epoch: 7,
            },
            destination,
            control::Lifecycle::Ready,
            false,
            host.now,
        );
        let original_active = active;
        let original: HostError = Box::new(NativeFault("legacy external error", 1));
        let address = error_address(&original);
        let error = finish_probe_run_with(Err(original), |error| {
            fail_here_owner_with(
                &mut active,
                &mut host,
                |_| panic!("no Here reply for another command"),
                error,
            )
            .unwrap_err()
        })
        .unwrap_err();
        assert_eq!(error_address(&error), address);
        assert_eq!(active, original_active);
        assert!(host.log.is_empty());
        host.assert_no_mutation();
    }
}
#[test]
fn every_production_async_error_return_is_inside_the_shared_fatal_loop_boundary() {
    let source = include_str!("../main.rs");
    let probe = source
        .split("fn run_probe(")
        .nth(1)
        .unwrap()
        .split("fn finish_probe_run_with(")
        .next()
        .unwrap();
    let loop_start = probe.find("let mut outcome = (|| loop {").unwrap();
    let loop_end = probe.rfind("    })();").unwrap();
    let sites: Vec<_> = probe.match_indices("Event::Error(error)").collect();
    assert_eq!(
        sites.len(),
        4,
        "buffered, batch, later routing, final match"
    );
    for (site, _) in sites {
        assert!(site > loop_start && site < loop_end);
        assert!(probe[site..].starts_with("Event::Error(error)"));
    }
    assert_eq!(
        probe[loop_start..loop_end]
            .matches("return Err(format!(\"Asynchronous X11 error:")
            .count(),
        4
    );
    assert!(probe[loop_end..].contains("finish_probe_run_with(outcome"));
    assert!(probe[loop_end..].contains("fail_owner_operation("));
    assert!(!probe[loop_start..loop_end].contains("return probe_termination_result("));
    let service = source
        .split("fn service_owner_operation<")
        .nth(1)
        .unwrap()
        .split("fn fail_owner_operation<")
        .next()
        .unwrap();
    assert!(
        service.find("owner.service_here_with").unwrap()
            < service.find("owner.verify_active_ownership").unwrap()
    );
    assert_eq!(service.matches("owner.verify_active_ownership").count(), 1);
}

pub(crate) mod recover_tests {
    use super::*;
    use control::{RecoverPhase, Visibility};
    use std::cell::{Cell, RefCell};

    struct Host {
        here: Fake,
        layer_changes: bool,
        mutations: Vec<Mutation>,
        fail_before_cookie: Option<&'static str>,
    }
    impl Host {
        fn new(remote: bool, minimized: bool, layer: L) -> Self {
            Self {
                here: Fake::new(remote, minimized, layer),
                layer_changes: true,
                mutations: vec![],
                fail_before_cookie: None,
            }
        }
        fn operation(&self) -> ActiveOperation {
            let mut active = None;
            assert_eq!(
                control::admit(
                    &mut active,
                    control::Request {
                        command: Command::Recover,
                        request_id: 99,
                        reply_xid: 9,
                        epoch: 11
                    },
                    2,
                    control::Lifecycle::Ready,
                    false,
                    self.here.now,
                ),
                control::Admission::Admitted
            );
            active.unwrap()
        }
        fn run(&mut self, op: &mut ActiveOperation) -> Option<control::OperationResult> {
            service_recover_with(op, self).unwrap()
        }
        fn restored(&mut self) {
            self.here.current.minimized = false;
            self.here.current.transitional = false;
        }
        fn atoms() -> LayerAtoms {
            LayerAtoms {
                state: 12,
                supported: 18,
                above: 14,
                below: 15,
            }
        }
    }
    impl BringHereHost for Host {
        fn now(&self) -> Instant {
            self.here.now
        }
        fn owned(&mut self) -> Result<bool, HostError> {
            self.here.owned()
        }
        fn live(&self) -> bool {
            self.here.live()
        }
        fn evidence(&mut self, destination: u32) -> Result<HereEvidence, HostError> {
            self.here.evidence(destination)
        }
        fn placement_busy(&self) -> bool {
            self.here.placement_busy()
        }
        fn input_active(&self) -> bool {
            self.here.input_active()
        }
        fn visible_ready(&self) -> bool {
            self.here.visible_ready()
        }
        fn fence_settled(&self) -> bool {
            self.here.fence_settled()
        }
        fn fence(&mut self) -> Result<(), HostError> {
            self.here.fence()
        }
        fn abort_recovery_layer(&mut self) {
            self.here.runtime.layers.abort();
        }
        fn desktop(
            &mut self,
            destination: u32,
            deadline: Instant,
            dispatched: &mut bool,
        ) -> Result<bool, HostError> {
            self.here.desktop(destination, deadline, dispatched)
        }
        fn geometry(&mut self) -> Result<Rect, HostError> {
            self.here.geometry()
        }
        fn restrict_automatic_correction(&mut self) {
            self.here.restrict_automatic_correction();
        }
        fn correct(
            &mut self,
            target: Point,
            deadline: Instant,
            dispatched: &mut bool,
        ) -> Result<Option<Instant>, HostError> {
            self.here.correct(target, deadline, dispatched)
        }
        fn enable(
            &mut self,
            evidence: &HereEvidence,
            geometry: Rect,
            deadline: Instant,
        ) -> Result<bool, HostError> {
            self.here.enable(evidence, geometry, deadline)
        }
    }
    impl RecoverHost for Host {
        type Top = Self;
        fn top(&mut self) -> &mut Self {
            self
        }
    }
    impl VisibilityHost for Host {
        fn now(&self) -> Instant {
            self.here.now
        }
        fn layer_atoms(&self) -> LayerAtoms {
            Self::atoms()
        }
        fn observe(&mut self) -> Result<x11::visibility::VisibilitySnapshot, HostError> {
            self.here.step("top_observe")?;
            Ok(self.here.current.snapshot())
        }
        fn noop_ready(&self) -> bool {
            self.here.visible_ready()
        }
        fn placement_busy(&self) -> bool {
            self.here.placement_busy()
        }
        fn fenced(&self) -> bool {
            self.here.runtime.input_fence.blocked
        }
        fn fence_settled(&self) -> bool {
            self.here.fence_settled()
        }
        fn deferred_placement(&mut self, _: &ShowPlacement) -> Result<bool, HostError> {
            panic!("Recover never defers local readiness")
        }
        fn fence(&mut self) -> Result<(), HostError> {
            self.here.fence()
        }
        fn iconify(&mut self, _: Instant, _: &mut bool) -> Result<bool, HostError> {
            panic!("Recover never hides")
        }
        fn restore(&mut self, _: Instant, _: &mut bool) -> Result<bool, HostError> {
            panic!("Recover requires guarded restore")
        }
        fn hidden(&mut self, _: bool, _: Instant, _: &mut bool) -> Result<bool, HostError> {
            panic!("Recover never accepts off-workspace Top")
        }
        fn placement(&mut self) -> Result<ShowPlacement, HostError> {
            panic!("Recover must not use legacy placement")
        }
        fn correct(
            &mut self,
            target: Point,
            deadline: Instant,
            dispatched: &mut bool,
        ) -> Result<Option<Instant>, HostError> {
            self.here.correct(target, deadline, dispatched)
        }
        fn enable(
            &mut self,
            snapshot: &x11::visibility::VisibilitySnapshot,
            input: &InputPlacement,
            deadline: Option<Instant>,
        ) -> Result<bool, HostError> {
            self.here.step("top_enable")?;
            if deadline.is_some_and(|dl| self.here.now >= dl) {
                return Ok(false);
            }
            let shape = self.here.step("shape");
            self.here.window.confirmed_origin = input.geometry.origin();
            self.here.window.cancel_in_flight_moves();
            self.here.runtime.body = BodyAvailability::Ready;
            self.here.runtime.restore_visibility_input_with(
                &self.here.window,
                snapshot.evidence,
                input,
                || shape.map(|()| 2000),
            )?;
            Ok(true)
        }
        fn recover_owned(&mut self) -> Result<bool, HostError> {
            self.here.owned()
        }
        fn recover_evidence(&mut self, destination: u32) -> Result<HereEvidence, HostError> {
            self.here.evidence(destination)
        }
        fn recover_geometry(&mut self, _: control::DestinationIdentity) -> Result<Rect, HostError> {
            self.here.geometry()
        }
        fn restrict_recover_correction(&mut self) {
            self.here.restrict_automatic_correction();
        }
        fn restore_recover(
            &mut self,
            op: &mut ActiveOperation,
            deadline: Instant,
            dispatched: &mut bool,
        ) -> Result<bool, HostError> {
            let clock = Cell::new(self.here.now);
            let host = RefCell::new(&mut *self);
            let atoms = x11::visibility::VisibilityAtoms {
                wm_state: 10,
                change_state: 11,
                state: 12,
                hidden: 13,
                above: 14,
                below: 15,
                desktop: 16,
                current_desktop: 17,
                supported: 18,
                user_time: 19,
            };
            atoms.restore_guarded_before_with(
                1,
                deadline,
                dispatched,
                || clock.get(),
                |request, sent| {
                    let mut h = host.borrow_mut();
                    let name = match request {
                        x11::visibility::VisibilityRequest::UserTime { .. } => "zero",
                        x11::visibility::VisibilityRequest::Map { .. } => "map",
                        _ => panic!("Unexpected restoration mutation"),
                    };
                    if h.fail_before_cookie != Some(name) {
                        *sent = true;
                    }
                    let result = h.here.step(name);
                    clock.set(h.here.now);
                    result
                },
                Some(|| {
                    let mut h = host.borrow_mut();
                    h.here.step("restore_guard")?;
                    let allowed = recover_visibility_guard(op, &mut **h)?
                        && op.detail.visibility == Visibility::Minimized;
                    clock.set(h.here.now);
                    Ok(allowed)
                }),
            )
        }
    }
    impl BringTopHost for Host {
        fn qualify_layer(
            &self,
            snapshot: &x11::visibility::VisibilitySnapshot,
            baseline: control::VisibilityBaseline,
        ) -> Result<ObservedLayer, RemoteLayerIssue> {
            Self::atoms().qualify_remote(snapshot, baseline)
        }
        fn service_above(
            &mut self,
            op: &mut ActiveOperation,
        ) -> Result<(LayerOutcome, Option<RemoteLayerIssue>), HostError> {
            let baseline = op.show.baseline.unwrap();
            let mut layers = std::mem::take(&mut self.here.runtime.layers);
            let host = RefCell::new(&mut *self);
            let result = service_remote_layer_with(
                &mut layers,
                op,
                |op| {
                    let mut h = host.borrow_mut();
                    h.here.step("layer_read")?;
                    let (evidence, decision) = read_recover_top(op, &mut **h)?;
                    Ok(match decision {
                        control::HereDecision::Destination => {
                            Self::atoms().qualify_remote(&evidence.snapshot, baseline)
                        }
                        control::HereDecision::Pending => Err(RemoteLayerIssue::Waiting),
                        _ => Err(RemoteLayerIssue::WorkspaceChanged),
                    })
                },
                || {
                    host.borrow_mut().here.step("support")?;
                    Ok(Ok(LayerSupport {
                        state: true,
                        above: true,
                        below: true,
                    }))
                },
                |mutation, op, dispatched| {
                    let mut h = host.borrow_mut();
                    h.here.step("layer_guard")?;
                    if !recover_layer_dispatch_eligible(op, &mut **h)? {
                        return Ok(false);
                    }
                    let step = match mutation {
                        Mutation::Remove(_) => "remove",
                        _ => "above",
                    };
                    if h.fail_before_cookie != Some(step) {
                        *dispatched = true;
                    }
                    h.here.step(match mutation {
                        Mutation::Remove(_) => "remove",
                        Mutation::AddAbove => "above",
                        _ => panic!("Above only"),
                    })?;
                    h.mutations.push(mutation);
                    if h.layer_changes {
                        h.here.current.layer = match mutation {
                            Mutation::Remove(_) => L::Normal,
                            _ => L::Above,
                        };
                    }
                    Ok(true)
                },
                || host.borrow().here.now,
            );
            self.here.runtime.layers = layers;
            result
        }
        fn abort_above(&mut self) {
            self.here.runtime.layers.abort();
        }
        fn layer_deadline(&self) -> Option<Instant> {
            self.here.runtime.layers.deadline()
        }
    }

    fn owner(h: &Host) -> HereOwner {
        HereOwner::recover(h.here.now, 2)
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum PublicFault {
        None,
        Layout,
        AfterRelocation,
        AfterRestoration,
        AfterBoth,
        OwnerLost,
        Native,
        NativeDispatch,
    }

    /// Borrow the actually admitted public slot into the production dispatcher
    /// and scheduler. Only WM observations and native resources are injected.
    pub(crate) fn public_roundtrip_service(
        active: &mut Option<ActiveOperation>,
        remote: bool,
        minimized: bool,
        layer: L,
        fault: PublicFault,
        reply: &mut dyn FnMut(control::OperationResult) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        let mut h = Host::new(remote, minimized, layer);
        h.here.window.window = 88;
        let original = *active.as_ref().unwrap();
        h.here.now = original.deadline - Command::Recover.owner_budget();
        assert_eq!(original.request.command, Command::Recover);
        assert_eq!(original.placement.destination_desktop, 2);
        assert_eq!(original.stage, Stage::Placement);
        match fault {
            PublicFault::Layout => h.here.layout = Err(DestinationError::Changed),
            PublicFault::OwnerLost => h.here.owned = false,
            PublicFault::Native => h.here.faults.push(("layout", 1)),
            PublicFault::NativeDispatch => h.here.faults.push(("desktop", 1)),
            PublicFault::AfterRestoration | PublicFault::AfterBoth => h.layer_changes = false,
            _ => {}
        }
        let mut injected = false;
        for _ in 0..16 {
            let Some(op) = active.as_mut() else {
                assert_eq!(h.here.window.window, 88);
                assert!(h.here.count("desktop") <= 1);
                assert!(h.here.count("map") <= 1);
                assert!(h.here.count("correct") <= 1);
                return Ok(());
            };
            assert_eq!(op.request, original.request);
            assert_eq!(op.deadline, original.deadline);
            let history = op.placement.proof.history;
            let fail_now = match fault {
                PublicFault::AfterRelocation => history.relocation_confirmed,
                PublicFault::AfterRestoration => history.restoration_confirmed,
                PublicFault::AfterBoth => {
                    history.relocation_confirmed && history.restoration_confirmed
                }
                _ => false,
            };
            if fail_now && !injected {
                h.here.layout = Err(DestinationError::Changed);
                mark_remote_observation(op, false);
                injected = true;
            }
            if !remote_service_due(op, h.here.now) {
                // A real WM notification supplies the next fresh observation.
                if op.recover_phase == RecoverPhase::Here && h.here.count("desktop") > 0 {
                    h.here.target();
                } else if h.here.count("map") > 0 && fault != PublicFault::AfterRelocation {
                    h.restored();
                }
                mark_remote_observation(op, false);
            }
            let outcome = service_admitted_placement_with(active, &mut h, &mut *reply);
            if let Err(error) = outcome {
                // Fatal owner errors still send their one correlated terminal
                // result before propagating to global cleanup.
                assert!(active.is_none());
                if !matches!(
                    fault,
                    PublicFault::Native | PublicFault::NativeDispatch | PublicFault::OwnerLost
                ) {
                    return Err(error);
                }
            }
        }
        panic!("public Recover did not terminate: {fault:?}");
    }

    fn tick(owner: &mut HereOwner, h: &mut Host) -> Result<(), HostError> {
        owner.service(|active, _, _, reply| service_recover_owner_with(active, h, reply))?;
        Ok(())
    }

    fn wake(owner: &mut HereOwner) {
        mark_remote_observation(owner.active_mut().unwrap(), false);
    }

    fn finish_owner(owner: &mut HereOwner, h: &mut Host) -> Result<(), HostError> {
        for _ in 0..8 {
            if owner.active().is_none() {
                return Ok(());
            }
            tick(owner, h)?;
            if let Some(op) = owner.active_mut() {
                if let Some((_, deadline)) = op.show.correction.or_else(|| {
                    (op.recover_phase == RecoverPhase::Here)
                        .then(|| op.placement.correction())
                        .flatten()
                }) {
                    h.here.now = h.here.now.max(deadline);
                }
                if op.recover_phase == RecoverPhase::Here && h.here.count("desktop") > 0 {
                    h.here.target();
                } else if h.here.count("map") > 0 {
                    h.restored();
                }
                mark_remote_observation(op, false);
            }
        }
        panic!("Recover owner failed to terminate");
    }

    fn assert_terminal(
        owner: &mut HereOwner,
        h: &Host,
        reason: Reason,
    ) -> control::TerminalResponse {
        assert!(owner.active().is_none());
        let replies = owner.replies();
        assert_eq!(replies.len(), 1);
        let reply = replies[0];
        assert_eq!(reply.detail.reason, reason);
        if reply.status != Status::Success {
            assert_eq!(
                reply.detail.placement.current,
                control::PlacementConfirmation::default()
            );
            assert!(!reply.detail.progress.input_ready);
            assert!(!reply.detail.progress.above_confirmed);
            assert!(!reply.detail.progress.not_minimized_confirmed);
            let history = reply.detail.placement.history;
            assert_eq!(
                reply.status,
                if history.relocation_confirmed || history.restoration_confirmed {
                    Status::Partial
                } else {
                    Status::Failed
                }
            );
        }
        assert!(h.here.runtime.layers.pending().is_none());
        owner.shutdown(true);
        owner.shutdown(false);
        assert_eq!(
            owner.replies(),
            replies,
            "global cleanup cannot reply twice"
        );
        reply
    }

    #[test]
    fn actual_owner_handoff_progresses_before_sleep_and_then_waits_without_polling() {
        let mut h = Host::new(true, true, L::Below);
        let mut owner = owner(&h);
        let total = owner.active().unwrap().deadline;
        tick(&mut owner, &mut h).unwrap();
        tick(&mut owner, &mut h).unwrap(); // settle possible-dispatch observation
        assert!(!remote_service_due(owner.active().unwrap(), h.here.now));
        let log = h.here.log.clone();
        for _ in 0..8 {
            tick(&mut owner, &mut h).unwrap();
        }
        assert_eq!(h.here.log, log);
        h.here.target();
        wake(&mut owner);
        tick(&mut owner, &mut h).unwrap();
        let op = owner.active().unwrap();
        assert_eq!(op.recover_phase, RecoverPhase::Top);
        assert_eq!(
            h.here.count("map"),
            1,
            "handoff starts Top in the same service turn"
        );
        assert_eq!(op.deadline, total);
        assert_eq!(
            remote_operation_deadline(op),
            op.bring_top.visibility_cutoff
        );
        assert!(!remote_service_due(op, h.here.now));
        let log = h.here.log.clone();
        tick(&mut owner, &mut h).unwrap();
        assert_eq!(h.here.log, log);
        h.restored();
        wake(&mut owner);
        finish_owner(&mut owner, &mut h).unwrap();
        let reply = assert_terminal(&mut owner, &h, Reason::None);
        assert_eq!(reply.status, Status::Success);
        assert_eq!(
            reply.detail.placement.history,
            control::RecoveryHistory {
                relocation_confirmed: true,
                restoration_confirmed: true,
            }
        );
        assert!(local_layer_service_allowed(owner.active()));
    }

    #[test]
    fn actual_scheduler_honors_each_phase_and_correction_boundary_exactly() {
        for top in [false, true] {
            let mut h = Host::new(false, top, L::Normal);
            h.here.refuse_correction = true;
            let mut owner = owner(&h);
            if top {
                tick(&mut owner, &mut h).unwrap();
                h.restored();
                wake(&mut owner);
            }
            h.here.actual.x = -100;
            tick(&mut owner, &mut h).unwrap();
            tick(&mut owner, &mut h).unwrap();
            let op = owner.active().unwrap();
            let cutoff = if top {
                op.bring_top.visibility_cutoff
            } else {
                here_cutoff(op)
            };
            let correction = op.placement.correction().unwrap().1;
            assert!(correction < cutoff);
            assert_eq!(remote_operation_deadline(op), correction);
            assert!(!remote_service_due(
                op,
                correction - Duration::from_nanos(1)
            ));
            assert!(remote_service_due(op, correction));
            h.here.now = correction;
            tick(&mut owner, &mut h).unwrap();
            assert_terminal(&mut owner, &h, Reason::Unverifiable);
            assert_eq!(h.here.count("correct"), 1);
        }
        for phase in 0..3 {
            let mut h = Host::new(phase == 0, phase == 1, L::Normal);
            h.layer_changes = false;
            let mut owner = owner(&h);
            tick(&mut owner, &mut h).unwrap();
            tick(&mut owner, &mut h).unwrap();
            let op = owner.active().unwrap();
            let cutoff = match phase {
                0 => op.deadline - Duration::from_millis(3500),
                1 => op.bring_top.visibility_cutoff,
                _ => op.recover_layer_cutoff().unwrap(),
            };
            assert_eq!(remote_operation_deadline(op), cutoff);
            assert!(!remote_service_due(op, cutoff - Duration::from_nanos(1)));
            assert!(remote_service_due(op, cutoff));
            h.here.now = cutoff;
            tick(&mut owner, &mut h).unwrap();
            assert_terminal(&mut owner, &h, Reason::ConfirmationTimeout);
        }
    }

    #[test]
    fn strict_event_families_are_phase_owned_and_layer_configure_reconciles_movement() {
        let (visibility, layout_atoms) = atoms();
        for phase in 0..3 {
            let mut h = Host::new(phase == 0, phase == 1, L::Normal);
            h.layer_changes = false;
            let mut owner = owner(&h);
            tick(&mut owner, &mut h).unwrap();
            tick(&mut owner, &mut h).unwrap();
            let original = *owner.active().unwrap();
            assert!(!remote_service_due(&original, h.here.now));
            assert!(!local_layer_service_allowed(Some(&original)));
            assert_eq!(remote_owns_placement(&original), phase != 2);
            assert_eq!(remote_owns_layer(&original), phase == 2);
            for kind in 0..15 {
                let mut op = original;
                let e = if kind == 14 {
                    Event::RandrNotify(x11rb::protocol::randr::NotifyEvent {
                        response_type: 0,
                        sub_code: x11rb::protocol::randr::Notify::RESOURCE_CHANGE,
                        sequence: 0,
                        u: x11rb::protocol::randr::ResourceChange {
                            timestamp: 0,
                            window: 1,
                        }
                        .into(),
                    })
                } else {
                    event(kind)
                };
                let consumed = route_here_evidence_event(
                    Some(&mut op),
                    &e,
                    1,
                    100,
                    &visibility,
                    &layout_atoms,
                );
                assert_eq!(
                    consumed,
                    !(phase == 2 && kind == 5),
                    "phase={phase} kind={kind}"
                );
                assert!(remote_service_due(&op, h.here.now));
                assert_eq!(op.deadline, original.deadline);
                assert_eq!(
                    op.bring_top.layer_admitted,
                    original.bring_top.layer_admitted
                );
                assert_eq!(op.placement.proof.history, original.placement.proof.history);
                assert_eq!(
                    op.placement.proof.current.geometry,
                    control::GeometryProof::Unknown
                );
                assert!(!op.detail.progress.input_ready);
                assert!(
                    !op.show.layout_interrupted,
                    "equal strict layout must be re-read, not rejected"
                );
            }
            for e in [property(999, 22), property(1, 999)] {
                let mut op = original;
                assert!(!route_here_evidence_event(
                    Some(&mut op),
                    &e,
                    1,
                    100,
                    &visibility,
                    &layout_atoms
                ));
                assert!(!remote_service_due(&op, h.here.now));
            }
        }
    }

    #[test]
    fn owner_native_fault_matrix_keeps_original_error_one_read_one_reply_and_no_retry() {
        for corrected in [false, true] {
            let mut reference = Host::new(!corrected, true, L::Below);
            if corrected {
                reference.here.actual.x = -100;
            }
            let mut reference_owner = owner(&reference);
            finish_owner(&mut reference_owner, &mut reference).unwrap();
            let mut counts = std::collections::HashMap::new();
            for &step in reference
                .here
                .log
                .iter()
                .filter(|&&s| !["fence", "restrict", "popup"].contains(&s))
            {
                let nth = counts.entry(step).or_insert(0);
                *nth += 1;
                // Includes ownership at/after handoff, user-time/map, every
                // strict read, correction, SHAPE, removal/addition and final proof.
                let mut h = Host::new(!corrected, true, L::Below);
                if corrected {
                    h.here.actual.x = -100;
                }
                h.here.faults.push((step, *nth));
                let mut owner = owner(&h);
                let error = finish_owner(&mut owner, &mut h)
                    .err()
                    .unwrap_or_else(|| panic!("fault not reached: {step}/{nth}"));
                assert_eq!(
                    error.downcast_ref::<NativeFault>(),
                    Some(&NativeFault(step, *nth))
                );
                let index = h
                    .here
                    .log
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| **s == step)
                    .nth(*nth - 1)
                    .unwrap()
                    .0;
                let after = &h.here.log[index + 1..];
                assert!(
                    !after.iter().any(|s| [
                        "desktop", "zero", "map", "correct", "shape", "remove", "above"
                    ]
                    .contains(s)),
                    "retry after {step}/{nth}: {after:?}"
                );
                assert!(
                    after.iter().filter(|&&s| s == "observe").count() <= 1,
                    "one history round"
                );
                assert!(after.iter().filter(|&&s| s == "layout").count() <= 2);
                assert!(
                    after.iter().filter(|&&s| s == "fence").count() <= 1,
                    "one safety cleanup"
                );
                let reply = assert_terminal(&mut owner, &h, Reason::X11Failure);
                assert_eq!(reply.detail.stage, Stage::Shutdown);
            }
        }
    }

    #[test]
    fn every_owner_query_mismatch_stops_mutation_and_is_distinct_from_native_failure() {
        let mut reference = Host::new(true, true, L::Below);
        let mut reference_owner = owner(&reference);
        finish_owner(&mut reference_owner, &mut reference).unwrap();
        for nth in 1..=reference.here.count("owner") {
            let mut h = Host::new(true, true, L::Below);
            h.here.lose_at = Some(("owner", nth));
            let mut owner = owner(&h);
            assert!(finish_owner(&mut owner, &mut h).is_err());
            assert_terminal(&mut owner, &h, Reason::OwnershipLost);
            let index = h
                .here
                .log
                .iter()
                .enumerate()
                .filter(|(_, s)| **s == "owner")
                .nth(nth - 1)
                .unwrap()
                .0;
            assert!(
                !h.here.log[index + 1..].iter().any(|s| [
                    "empty", "desktop", "zero", "map", "correct", "shape", "remove", "above"
                ]
                .contains(s)),
                "mutation after ownership loss #{nth}"
            );
        }
    }

    /// Real owner turns prepare boundaries; no public request can seed them.
    fn checkpoint(phase: usize) -> (Host, HereOwner) {
        let mut h = Host::new(true, phase != 2, L::Below);
        h.layer_changes = false;
        let mut owner = owner(&h);
        if phase == 0 {
            return (h, owner);
        }
        tick(&mut owner, &mut h).unwrap();
        if phase == 1 {
            return (h, owner);
        }
        h.here.target();
        if phase == 2 {
            h.here.current.transitional = true;
        }
        wake(&mut owner);
        tick(&mut owner, &mut h).unwrap();
        if phase <= 3 {
            return (h, owner);
        }
        h.restored();
        if phase == 4 {
            h.here.actual.x = -100;
        }
        wake(&mut owner);
        tick(&mut owner, &mut h).unwrap();
        if phase <= 5 {
            return (h, owner);
        }
        h.here.current.layer = L::Normal;
        wake(&mut owner);
        tick(&mut owner, &mut h).unwrap();
        if phase == 7 {
            h.here.current.layer = L::Above;
        }
        (h, owner)
    }

    /// Export only the sole active slot reached by the existing production turns:
    /// admission, desktop attempt, Here arrival, Top restore/correction, layer
    /// removal/addition, then host Above before the slot's final validation.
    pub(crate) fn lifecycle_snapshots() -> Vec<(usize, ActiveOperation)> {
        (0..8)
            .map(|phase| {
                let (_, owner) = checkpoint(phase);
                (phase, *owner.active().unwrap())
            })
            .collect()
    }

    fn external(owner: &mut HereOwner, h: &mut Host, original: HostError) -> HostError {
        finish_probe_run_with(Err(original), |original| {
            let mut original = Some(original);
            match owner.service(|active, _, _, reply| {
                fail_here_owner_with(active, h, reply, original.take().unwrap())
            }) {
                Err(original) => original,
                Ok(false) => original.unwrap(),
                Ok(true) => panic!("fatal owner must return its original error"),
            }
        })
        .unwrap_err()
    }

    /// Follow run_probe's Closing -> ordinary finish ordering, using the actual
    /// endpoint seams. An error must leave the sole slot for the fatal boundary.
    fn closing_error(owner: &mut HereOwner, publication: bool, original: HostError) -> HostError {
        let descriptor = owner.descriptor();
        let operation = *owner.active().unwrap();
        let address = error_address(&original);
        let original = RefCell::new(Some(original));
        let calls = RefCell::new(Vec::new());
        let result = owner
            .closing_with(
                || {
                    calls.borrow_mut().push("lookup");
                    if publication {
                        Ok(descriptor.control_xid)
                    } else {
                        Err(original.borrow_mut().take().unwrap())
                    }
                },
                |intended| {
                    calls.borrow_mut().push("publish");
                    assert!(publication, "failed lookup cannot publish");
                    assert_eq!(
                        intended,
                        control::OwnerDescriptor {
                            lifecycle: control::Lifecycle::Closing,
                            ..descriptor
                        }
                    );
                    // A checked write can fail after possible native dispatch.
                    // This callback makes no claim about server/WM application.
                    Err(original.borrow_mut().take().unwrap())
                },
            )
            .and_then(|()| {
                calls.borrow_mut().push("ordinary finish");
                owner.finish_remote()
            });
        let error = result.unwrap_err();
        assert_eq!(error_address(&error), address);
        assert_eq!(
            *calls.borrow(),
            if publication {
                vec!["lookup", "publish"]
            } else {
                vec!["lookup"]
            }
        );
        assert_eq!(owner.descriptor(), descriptor, "no confirmed Closing");
        assert_eq!(
            owner.active(),
            Some(&operation),
            "fatal still owns the slot"
        );
        assert!(owner.replies().is_empty(), "no ordinary Closing terminal");
        error
    }

    fn closing_fatal(
        owner: &mut HereOwner,
        h: &mut Host,
        publication: bool,
    ) -> control::TerminalResponse {
        let operation = *owner.active().unwrap();
        let prior = h.here.log.len();
        let mutations = h.mutations.clone();
        let needs_fence = operation.placement.baseline.is_some()
            && (BringHereHost::input_active(h) || !BringHereHost::fence_settled(h));
        let source = if publication {
            "Closing write"
        } else {
            "Closing query"
        };
        let original: HostError = Box::new(NativeFault(source, 1));
        let address = error_address(&original);
        let error = closing_error(owner, publication, original);
        assert_eq!(h.here.log.len(), prior, "Closing cannot progress Recover");
        // external uses finish_probe_run_with -> ControlWindow's sole slot ->
        // fail_here_owner_with -> finalize_bring_here_with -> complete_here_owner_with.
        let error = external(owner, h, error);
        assert_eq!(error_address(&error), address);
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault(source, 1))
        );
        let after = &h.here.log[prior..];
        assert!(
            !after.iter().any(|s| [
                "desktop",
                "zero",
                "map",
                "correct",
                "enable",
                "top_enable",
                "shape",
                "remove",
                "above"
            ]
            .contains(s)),
            "fatal teardown cannot retry recovery: {after:?}"
        );
        assert_eq!(h.mutations, mutations);
        assert_eq!(
            after.iter().filter(|&&s| s == "observe").count(),
            usize::from(operation.placement.baseline.is_some()),
            "one bounded history acquisition, even when it fails"
        );
        assert_eq!(
            after.iter().filter(|&&s| s == "fence").count(),
            usize::from(needs_fence),
            "only mandatory safety cleanup is permitted"
        );
        if operation.placement.baseline.is_some() {
            assert!(h.here.runtime.input_fence.blocked);
        }
        let reply = assert_terminal(owner, h, Reason::X11Failure);
        assert_eq!(reply.request_id, operation.request.request_id);
        assert_eq!(reply.control_xid, owner.descriptor().control_xid);
        assert!(control::ReplyCorrelation {
            control_xid: owner.descriptor().control_xid,
            epoch: operation.request.epoch,
            request_id: operation.request.request_id,
            reply_xid: operation.request.reply_xid,
        }
        .matches(owner.descriptor(), operation.request.reply_xid, reply));
        assert_eq!(reply.detail.stage, Stage::Shutdown);
        assert_eq!(
            reply.detail.progress,
            control::Progress {
                host_mutation_dispatched: operation.detail.progress.host_mutation_dispatched,
                ..Default::default()
            }
        );
        // Actual global error-priority helper, with an empty-slot ordinary finish
        // and a later cleanup failure. Neither can produce another terminal.
        let error = finish_probe_with(Err(error), || {
            owner.finish_remote()?;
            Err(Box::new(NativeFault("later cleanup", 1)))
        })
        .unwrap_err();
        assert_eq!(error_address(&error), address);
        let log = h.here.log.clone();
        tick(owner, h).unwrap();
        assert_eq!(h.here.log, log, "cleared slot cannot run again");
        assert_eq!(
            owner.replies(),
            [reply],
            "one attempt even if delivery failed"
        );
        reply
    }

    fn closing_failure_matrix(publication: bool) {
        for phase in 0..8 {
            for failures in 0..8 {
                let (mut h, mut owner) = checkpoint(phase);
                let history = owner.active().unwrap().placement.proof.history;
                assert_eq!(history.relocation_confirmed, phase >= 2);
                assert_eq!(history.restoration_confirmed, phase >= 4);
                if failures & 1 != 0 {
                    h.here.faults.push(("empty", h.here.count("empty") + 1));
                }
                if failures & 2 != 0 {
                    h.here.faults.push(("observe", h.here.count("observe") + 1));
                }
                if failures & 4 != 0 {
                    owner.fail_reply(true);
                }
                let reply = closing_fatal(&mut owner, &mut h, publication);
                assert_eq!(reply.detail.placement.history, history, "snapshot {phase}");
            }
        }
    }

    #[test]
    fn closing_query_failures_join_recover_fatal_policy_at_every_checkpoint() {
        closing_failure_matrix(false);
    }

    #[test]
    fn closing_publication_failures_join_recover_fatal_policy_at_every_checkpoint() {
        closing_failure_matrix(true);
    }

    #[test]
    fn closing_failure_history_read_can_confirm_transitions_without_retry() {
        for publication in [false, true] {
            for relocated in [false, true] {
                for restored in [false, true] {
                    let (mut h, mut owner) = checkpoint(1);
                    assert_eq!(h.here.count("desktop"), 1, "acknowledgment alone");
                    assert_eq!(
                        owner.active().unwrap().placement.proof.history,
                        control::RecoveryHistory::default()
                    );
                    if relocated {
                        h.here.target();
                    }
                    if restored {
                        h.restored();
                    }
                    let reply = closing_fatal(&mut owner, &mut h, publication);
                    assert_eq!(
                        reply.detail.placement.history,
                        control::RecoveryHistory {
                            relocation_confirmed: relocated,
                            restoration_confirmed: restored,
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn closing_failure_noop_and_attempted_correction_cannot_authorize_partial() {
        for publication in [false, true] {
            for correction in [false, true] {
                // Drive the local alternative through real service: either Here
                // no-ops into Top layer work, or waits for its sole correction.
                let mut h = Host::new(false, false, L::Below);
                h.layer_changes = false;
                if correction {
                    h.here.actual.x = -100;
                }
                let mut owner = owner(&h);
                tick(&mut owner, &mut h).unwrap();
                assert!(owner.active().is_some());
                assert_eq!(h.here.count("correct"), usize::from(correction));
                assert_eq!(h.here.count("desktop"), 0);
                assert_eq!(h.here.count("map"), 0);
                let reply = closing_fatal(&mut owner, &mut h, publication);
                assert_eq!(reply.status, Status::Failed);
                assert_eq!(
                    reply.detail.placement.history,
                    control::RecoveryHistory::default()
                );
            }
        }
    }

    #[test]
    fn successful_closing_uses_ordinary_finish_without_fatal_policy() {
        for termination in [ProbeTermination::WmClose, ProbeTermination::DurationExpired] {
            let (h, mut owner) = checkpoint(3);
            let descriptor = owner.descriptor();
            let history = owner.active().unwrap().placement.proof.history;
            let calls = RefCell::new(Vec::new());
            let outcome = owner
                .closing_with(
                    || {
                        calls.borrow_mut().push("lookup");
                        Ok(descriptor.control_xid)
                    },
                    |intended| {
                        calls.borrow_mut().push("publish");
                        assert_eq!(intended.lifecycle, control::Lifecycle::Closing);
                        Ok(())
                    },
                )
                .and_then(|()| {
                    calls.borrow_mut().push("ordinary finish");
                    owner.finish_remote()
                })
                .and(Ok(Some(termination)));
            finish_probe_run_with(outcome, |_| panic!("successful Closing is ordinary")).unwrap();
            assert_eq!(*calls.borrow(), ["lookup", "publish", "ordinary finish"]);
            assert_eq!(owner.descriptor().lifecycle, control::Lifecycle::Closing);
            assert_eq!(
                assert_terminal(&mut owner, &h, Reason::Closing)
                    .detail
                    .placement
                    .history,
                history
            );
        }
    }

    #[test]
    fn external_fatal_matrix_preserves_original_error_history_and_one_bounded_read() {
        for phase in 0..8 {
            for boundary in ["async", "buffered error", "batch read", "poll", "flush"] {
                for fail_reply in [false, true] {
                    let (mut h, mut owner) = checkpoint(phase);
                    let prior = h.here.log.len();
                    owner.fail_reply(fail_reply);
                    let original: HostError = Box::new(NativeFault(boundary, phase));
                    let address = error_address(&original);
                    let error = external(&mut owner, &mut h, original);
                    assert_eq!(error_address(&error), address);
                    let after = &h.here.log[prior..];
                    assert!(!after.iter().any(|s| [
                        "desktop", "zero", "map", "correct", "shape", "remove", "above"
                    ]
                    .contains(s)));
                    assert_eq!(
                        after.iter().filter(|&&s| s == "observe").count(),
                        usize::from(phase != 0)
                    );
                    assert!(after.iter().filter(|&&s| s == "fence").count() <= 1);
                    let reply = assert_terminal(&mut owner, &h, Reason::X11Failure);
                    assert_eq!(reply.detail.stage, Stage::Shutdown);
                    assert_eq!(
                        reply.detail.placement.history.relocation_confirmed,
                        phase >= 2
                    );
                    assert_eq!(
                        reply.detail.placement.history.restoration_confirmed,
                        phase >= 4
                    );
                }
            }
        }
    }

    #[test]
    fn fatal_history_uses_here_preservation_and_top_restoration_interpretations() {
        // Here can independently confirm unexpected restoration, but never hand off.
        let (mut h, mut owner) = checkpoint(1);
        h.restored();
        let original: HostError = Box::new(NativeFault("Here restoration", 1));
        external(&mut owner, &mut h, original);
        let reply = assert_terminal(&mut owner, &h, Reason::X11Failure);
        assert!(!reply.detail.placement.history.relocation_confirmed);
        assert!(reply.detail.placement.history.restoration_confirmed);
        assert_eq!(h.here.count("zero"), 0);
        assert_eq!(h.here.count("above"), 0);

        // Possibly dispatched relocation can first be confirmed on the fatal read.
        let (mut h, mut owner) = checkpoint(1);
        h.here.target();
        external(
            &mut owner,
            &mut h,
            Box::new(NativeFault("Here relocation", 1)),
        );
        assert!(
            assert_terminal(&mut owner, &h, Reason::X11Failure)
                .detail
                .placement
                .history
                .relocation_confirmed
        );

        // Top's single read can first establish restoration after a map.
        let (mut h, mut owner) = checkpoint(3);
        h.restored();
        external(
            &mut owner,
            &mut h,
            Box::new(NativeFault("Top restoration", 1)),
        );
        let reply = assert_terminal(&mut owner, &h, Reason::X11Failure);
        assert!(reply.detail.placement.history.restoration_confirmed);
        assert_eq!(h.here.count("map"), 1);
        assert_eq!(h.here.count("shape"), 0);
    }

    #[test]
    fn fatal_cleanup_read_reply_failure_combinations_keep_original_and_do_not_retry() {
        for cleanup_fails in [false, true] {
            for history_fails in [false, true] {
                for reply_fails in [false, true] {
                    let (mut h, mut owner) = checkpoint(5);
                    let fences = h.here.count("fence");
                    if cleanup_fails {
                        h.here.faults.push(("empty", h.here.count("empty") + 1));
                    }
                    if history_fails {
                        h.here.faults.push(("layout", h.here.count("layout") + 1));
                    }
                    owner.fail_reply(reply_fails);
                    let original: HostError = Box::new(NativeFault("primary", 7));
                    let address = error_address(&original);
                    let error = external(&mut owner, &mut h, original);
                    assert_eq!(error_address(&error), address);
                    assert_eq!(h.here.count("fence"), fences + 1);
                    assert_terminal(&mut owner, &h, Reason::X11Failure);
                }
            }
        }
    }

    #[test]
    fn fatal_history_is_skipped_without_live_body_or_trustworthy_ownership() {
        for destroyed in [false, true] {
            let (mut h, mut owner) = checkpoint(5);
            if destroyed {
                h.here.runtime.unavailable(true);
            } else {
                h.here.owned = false;
            }
            let reads = h.here.count("observe");
            external(&mut owner, &mut h, Box::new(NativeFault("outer", 1)));
            assert_eq!(h.here.count("observe"), reads);
            assert_terminal(&mut owner, &h, Reason::X11Failure);
        }
    }

    #[test]
    fn native_failure_after_ordinary_invalidation_has_x11_diagnostic_priority() {
        let (mut h, mut owner) = checkpoint(5);
        owner
            .active_mut()
            .unwrap()
            .placement
            .invalidate_here(Reason::LayoutChanged);
        external(
            &mut owner,
            &mut h,
            Box::new(NativeFault("outer after drift", 1)),
        );
        assert_terminal(&mut owner, &h, Reason::X11Failure);
    }

    #[test]
    fn orderly_termination_preserves_recover_history_and_never_enters_fatal_policy() {
        for phase in 0..8 {
            for termination in [
                Some(ProbeTermination::WmClose),
                Some(ProbeTermination::DurationExpired),
                Some(ProbeTermination::ControlEndpointLost),
                None,
            ] {
                let (h, mut owner) = checkpoint(phase);
                let history = owner.active().unwrap().placement.proof.history;
                let lost = matches!(termination, Some(ProbeTermination::ControlEndpointLost));
                assert_eq!(
                    finish_probe_run_with(Ok(termination), |_| panic!(
                        "orderly shutdown is not a native failure"
                    ))
                    .is_err(),
                    lost
                );
                owner.shutdown(!lost);
                let reply = owner.replies()[0];
                assert_eq!(reply.detail.placement.history, history);
                assert_eq!(
                    reply.detail.reason,
                    if lost {
                        Reason::OwnershipLost
                    } else {
                        Reason::Closing
                    }
                );
                assert_eq!(
                    reply.detail.placement.current,
                    control::PlacementConfirmation::default()
                );
                assert!(!reply.detail.progress.input_ready);
                assert!(owner.active().is_none());
                owner.shutdown(true);
                assert_eq!(owner.replies().len(), 1);
                assert_eq!(h.here.count("desktop"), usize::from(phase > 0));
            }
        }
    }

    #[test]
    fn body_destruction_is_nonfatal_and_does_not_become_x11_failure() {
        for phase in 0..8 {
            let (mut h, mut owner) = checkpoint(phase);
            h.here.runtime.unavailable(true);
            h.here.terminal = Some(Visibility::Destroyed);
            wake(&mut owner);
            tick(&mut owner, &mut h).unwrap();
            assert_terminal(&mut owner, &h, Reason::BodyDestroyed);
        }
    }

    #[test]
    fn incoming_transport_owner_loss_and_query_error_use_the_same_owner_terminal_policy() {
        for phase in [1, 3, 5, 6] {
            for during_validation in [false, true] {
                for query_error in [false, true] {
                    let (mut h, mut owner) = checkpoint(phase);
                    let prior = h.here.log.len();
                    let original = owner.request_ownership_failure(during_validation, query_error);
                    let address = error_address(&original);
                    let error = external(&mut owner, &mut h, original);
                    assert_eq!(error_address(&error), address);
                    assert_terminal(
                        &mut owner,
                        &h,
                        if query_error {
                            Reason::X11Failure
                        } else {
                            Reason::OwnershipLost
                        },
                    );
                    if !query_error {
                        assert_eq!(h.here.log.len(), prior);
                    }
                }
            }
        }
    }

    #[test]
    fn expected_layer_transitions_and_configure_require_fresh_final_geometry() {
        let (mut h, mut owner) = checkpoint(5);
        let (visibility, layout_atoms) = atoms();
        let deadline = owner.active().unwrap().deadline;
        h.here.current.layer = L::Normal;
        assert!(route_here_evidence_event(
            owner.active_mut(),
            &event(1),
            1,
            100,
            &visibility,
            &layout_atoms
        ));
        tick(&mut owner, &mut h).unwrap();
        assert_eq!(h.here.count("above"), 1);
        assert_eq!(owner.active().unwrap().deadline, deadline);
        let op = owner.active_mut().unwrap();
        op.placement.proof.current.geometry = control::GeometryProof::Verified;
        op.detail.progress.input_ready = true;
        assert!(
            !route_here_evidence_event(Some(op), &event(5), 1, 100, &visibility, &layout_atoms),
            "Configure stays on ordinary reconciliation path"
        );
        assert_eq!(
            op.placement.proof.current.geometry,
            control::GeometryProof::Unknown
        );
        assert!(!op.detail.progress.input_ready);
        h.here.current.layer = L::Above;
        h.here.actual.x = -100;
        tick(&mut owner, &mut h).unwrap();
        assert_terminal(&mut owner, &h, Reason::Unverifiable);
        assert_eq!(
            h.here.count("correct"),
            0,
            "ordinary movement cannot grant a Recover correction"
        );
    }

    #[test]
    fn recover_phase_deadlines_survive_sustained_bounded_batches_without_polling() {
        let (visibility, layout_atoms) = atoms();
        for phase in [1, 3, 5] {
            let (mut h, mut owner) = checkpoint(phase);
            tick(&mut owner, &mut h).unwrap();
            let deadline = remote_operation_deadline(owner.active().unwrap());
            let reads = h.here.count("observe");
            let mut queue = VecDeque::from(vec![property(999, 99); 4000]);
            let mut buffered = Some(property(999, 99));
            let mut processed = 0;
            while owner.active().is_some() {
                let batch = drain_events_bounded(&mut buffered, 64, || {
                    Ok::<_, HostError>(queue.pop_front())
                })
                .unwrap();
                assert!(batch.len() <= 64);
                for e in batch {
                    h.here.now += Duration::from_millis(1);
                    processed += 1;
                    if owner
                        .active()
                        .is_some_and(|op| h.here.now >= remote_operation_deadline(op))
                    {
                        tick(&mut owner, &mut h).unwrap();
                        assert_eq!(h.here.now, deadline);
                    }
                    assert!(!route_here_evidence_event(
                        owner.active_mut(),
                        &e,
                        1,
                        100,
                        &visibility,
                        &layout_atoms
                    ));
                }
                tick(&mut owner, &mut h).unwrap();
            }
            assert!(processed > 64);
            assert!(
                h.here.count("observe") <= reads + 3,
                "unrelated events do not poll"
            );
            assert_terminal(&mut owner, &h, Reason::ConfirmationTimeout);
        }
    }

    #[test]
    fn both_consumed_request_boundaries_immediately_service_internal_recover() {
        for _site in ["batch", "lookahead"] {
            let mut h = Host::new(true, true, L::Below);
            let mut active = None;
            route_owner_request_with(
                &mut active,
                &mut h,
                |active, h| {
                    *active = Some(h.operation());
                    Ok(true)
                },
                |active, h| service_recover_owner_with(active, h, |_| panic!("pending")),
            )
            .unwrap();
            assert_eq!(h.here.count("desktop"), 1);
            let deadline = active.as_ref().unwrap().deadline;
            h.here.target();
            mark_remote_observation(active.as_mut().unwrap(), false);
            service_recover_owner_with(&mut active, &mut h, |_| panic!("Top still pending"))
                .unwrap();
            assert_eq!(h.here.count("map"), 1);
            assert_eq!(active.as_ref().unwrap().deadline, deadline);
            assert!(!remote_service_due(active.as_ref().unwrap(), h.here.now));
        }
    }

    #[test]
    fn completed_recover_reply_failure_clears_the_slot_without_undoing_success() {
        for history in [false, true] {
            let mut h = Host::new(history, history, L::Below);
            let mut owner = owner(&h);
            owner.fail_reply(true);
            assert!(finish_owner(&mut owner, &mut h).is_err());
            let reply = assert_terminal(&mut owner, &h, Reason::None);
            assert_eq!(reply.status, Status::Success);
            assert!(h.here.visible_ready());
            assert_eq!(h.here.current.layer, L::Above);
            assert_eq!(
                reply.detail.placement.history,
                control::RecoveryHistory {
                    relocation_confirmed: history,
                    restoration_confirmed: history,
                }
            );
            tick(&mut owner, &mut h).unwrap();
            assert_eq!(owner.replies().len(), 1);
        }
    }

    #[test]
    fn production_owner_wait_boundaries_remain_reachable_as_specified() {
        let source = include_str!("../main.rs");
        let probe = source
            .split("fn run_probe(")
            .nth(1)
            .unwrap()
            .split("fn finish_probe_run_with(")
            .next()
            .unwrap();
        assert_eq!(probe.matches("route_owner_request(").count(), 2);
        assert!(probe.contains("drain_events_bounded(&mut buffered_event, 64"));
        assert!(probe.contains(".chain(remote_deadline(owner))"));
        assert!(probe.contains(".closing(conn)"));
        assert!(probe.contains("owner.finish_remote(conn)"));
        assert!(
            probe.rfind("service_owner_operation(").unwrap()
                < probe.rfind("wait_x11(conn, timeout)").unwrap()
        );
    }

    fn assert_result(
        op: &ActiveOperation,
        result: control::OperationResult,
        status: Status,
        relocation: bool,
        restoration: bool,
    ) {
        assert_eq!(
            op.recover_result(result.detail().reason, result.detail().stage),
            Ok(result)
        );
        assert_eq!(result.status(), status);
        assert_eq!(
            result.detail().placement.history,
            control::RecoveryHistory {
                relocation_confirmed: relocation,
                restoration_confirmed: restoration,
            }
        );
        assert_eq!(
            op.request,
            control::Request {
                command: Command::Recover,
                request_id: 99,
                reply_xid: 9,
                epoch: 11
            }
        );
        assert_eq!(op.placement.destination_desktop, 2);
        if status == Status::Success {
            assert_eq!(op.recover_phase, RecoverPhase::Top);
            assert_eq!(result.detail().stage, Stage::Layer);
            assert!(result.detail().progress.input_ready);
            assert_eq!(result.detail().visibility, Visibility::NotMinimizedHere);
        }
    }

    fn drive(
        h: &mut Host,
        op: &mut ActiveOperation,
    ) -> Result<control::OperationResult, HostError> {
        for _ in 0..4 {
            if let Some(result) = service_recover_with(op, h)? {
                return Ok(result);
            }
            if op.recover_phase == RecoverPhase::Here && h.here.count("desktop") > 0 {
                h.here.target();
            } else if op.recover_phase == RecoverPhase::Top && h.here.count("map") > 0 {
                h.restored();
            }
        }
        panic!("scripted WM evidence did not settle");
    }

    #[test]
    fn invalid_here_never_enters_top_and_safe_above_noop_has_fresh_top_proof() {
        let mut h = Host::new(false, false, L::Above);
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Success, false, false);
        h.here.assert_no_mutation();
        assert!(
            h.here.count("geometry") >= 3,
            "Here geometry cannot substitute for Top proof"
        );
        assert!(!op.handoff_recover_top(h.here.now, op.detail, control::Confirmation::default()));
        assert_eq!(op.recover_phase, RecoverPhase::Top);
        let mut h = Host::new(false, false, L::Below);
        h.here.current.transitional = true;
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Failed, false, false);
        assert_eq!(op.recover_phase, RecoverPhase::Here);
        assert_eq!(h.here.count("map"), 0);
        assert!(h.mutations.is_empty());
    }

    #[test]
    fn history_partitions_survive_layout_failure_without_current_readiness() {
        for remote in [false, true] {
            for restored in [false, true] {
                let mut h = Host::new(remote, true, L::Below);
                let mut op = h.operation();
                assert!(h.run(&mut op).is_none());
                if remote {
                    h.here.target();
                    assert!(h.run(&mut op).is_none());
                }
                assert_eq!(op.recover_phase, RecoverPhase::Top);
                if restored {
                    h.restored();
                }
                h.here.layout = Err(DestinationError::Changed);
                let result = h.run(&mut op).unwrap();
                assert_result(
                    &op,
                    result,
                    if remote || restored {
                        Status::Partial
                    } else {
                        Status::Failed
                    },
                    remote,
                    restored,
                );
                assert_eq!(result.detail().reason, Reason::LayoutChanged);
                assert!(!result.detail().progress.input_ready);
                assert_eq!(
                    result.detail().placement.current,
                    control::PlacementConfirmation::default()
                );
                assert!(h.mutations.is_empty());
            }
        }
    }

    #[test]
    fn every_strict_read_veto_latches_before_any_later_recovery_mutation() {
        let mut reference = Host::new(true, true, L::Below);
        let mut op = reference.operation();
        assert_eq!(
            drive(&mut reference, &mut op).unwrap().status(),
            Status::Success
        );
        let reads = reference.here.count("layout");
        assert!(reads > 20);
        for index in 1..=reads {
            let mut h = Host::new(true, true, L::Below);
            h.here
                .layouts
                .extend(std::iter::repeat_n(Ok(layout()), index - 1));
            h.here.layouts.push_back(Err(DestinationError::Changed));
            let mut op = h.operation();
            let result = drive(&mut h, &mut op).unwrap();
            assert_ne!(result.status(), Status::Success, "layout read {index}");
            let invalid = h
                .here
                .log
                .iter()
                .enumerate()
                .filter(|(_, step)| **step == "layout")
                .nth(index - 1)
                .unwrap()
                .0;
            for step in &h.here.log[invalid + 1..] {
                assert!(
                    !["desktop", "zero", "map", "correct", "shape", "remove", "above"]
                        .contains(step),
                    "mutation {step} after invalid layout read {index}"
                );
            }
        }
    }

    #[test]
    fn top_rejects_each_frozen_identity_component_and_invalid_destination_policy() {
        for case in 0..12 {
            let mut h = Host::new(false, true, L::Below);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.restored();
            let mut changed = layout();
            match case {
                0 => changed.desktop_count += 1,
                1 => changed.destination_desktop = 3,
                2 => changed.current_desktop = 3,
                3 => changed.target_workarea = Some(Rect::new(0, 0, 700, 600)),
                4 => changed.viewport = control::DestinationViewport::Absent,
                5 => changed.monitor_name += 1,
                6 => changed.monitor_bounds.x += 1,
                7 => changed.usable_area.width -= 1,
                8 => changed.body_size = Size::new(161, 160),
                9 => changed.valid_origins = geometry::ValidOriginBounds::new(1, 500, 0, 500),
                10 => {}
                _ => {}
            }
            h.here.layout = match case {
                10 => Err(DestinationError::UnsupportedViewport),
                11 => Err(DestinationError::Unverifiable(
                    "invalid count or workarea".into(),
                )),
                _ => Ok(changed),
            };
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Partial, false, true);
            assert_eq!(
                result.detail().reason,
                match case {
                    10 => Reason::Unsupported,
                    11 => Reason::Unverifiable,
                    _ => Reason::LayoutChanged,
                }
            );
            assert_eq!(h.here.count("shape"), 0);
            assert!(h.mutations.is_empty());
        }
    }

    #[test]
    fn guard_after_user_time_veto_retains_dispatch_and_never_maps() {
        let mut reference = Host::new(false, true, L::Normal);
        let mut op = reference.operation();
        assert!(reference.run(&mut op).is_none());
        let guard = reference
            .here
            .log
            .iter()
            .position(|&step| step == "restore_guard")
            .unwrap();
        let before = reference.here.log[..guard]
            .iter()
            .filter(|&&step| step == "layout")
            .count();
        let mut h = Host::new(false, true, L::Normal);
        h.here
            .layouts
            .extend(std::iter::repeat_n(Ok(layout()), before));
        h.here.layouts.push_back(Err(DestinationError::Changed));
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Failed, false, false);
        assert!(result.detail().progress.host_mutation_dispatched);
        assert_eq!(h.here.count("zero"), 1);
        assert_eq!(h.here.count("map"), 0);
        assert_eq!(h.here.count("restore_guard"), 1);
    }

    #[test]
    fn layer_addition_has_its_own_guard_after_successful_below_removal() {
        let mut reference = Host::new(false, false, L::Below);
        let mut op = reference.operation();
        assert_eq!(reference.run(&mut op).unwrap().status(), Status::Success);
        let addition = reference
            .here
            .log
            .iter()
            .rposition(|&step| step == "layer_guard")
            .unwrap();
        let before = reference.here.log[..addition]
            .iter()
            .filter(|&&step| step == "layout")
            .count();
        let mut h = Host::new(false, false, L::Below);
        h.here
            .layouts
            .extend(std::iter::repeat_n(Ok(layout()), before));
        h.here.layouts.push_back(Err(DestinationError::Changed));
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Failed, false, false);
        assert_eq!(h.here.count("remove"), 1);
        assert_eq!(h.here.count("above"), 0);
        assert_eq!(result.detail().reason, Reason::LayoutChanged);
    }

    #[test]
    fn here_cutoff_is_exact_and_handoff_visibility_budget_uses_actual_time() {
        for remote in [false, true] {
            let mut h = Host::new(remote, true, L::Normal);
            let mut op = h.operation();
            let start = h.here.now;
            if remote {
                assert!(h.run(&mut op).is_none());
                h.here.target();
            }
            h.here.now = start + Duration::from_millis(3500);
            let result = h.run(&mut op).unwrap();
            assert_result(
                &op,
                result,
                if remote {
                    Status::Partial
                } else {
                    Status::Failed
                },
                remote,
                false,
            );
            assert_eq!(op.recover_phase, RecoverPhase::Here);
            assert_eq!(h.here.count("zero"), 0);
        }
        let mut h = Host::new(true, true, L::Normal);
        let mut op = h.operation();
        let start = h.here.now;
        assert!(h.run(&mut op).is_none());
        h.here.target();
        h.here.now = start + Duration::from_millis(3400);
        assert!(h.run(&mut op).is_none());
        assert_eq!(
            op.bring_top.visibility_cutoff,
            start + Duration::from_millis(5900)
        );
        assert_eq!(op.deadline, start + Duration::from_millis(7000));
        h.restored();
        h.here.now = op.bring_top.visibility_cutoff;
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Partial, true, true);
        assert_eq!(h.here.count("shape"), 0);
        assert!(h.mutations.is_empty());
    }

    #[test]
    fn synchronous_restore_and_guard_reads_cannot_spend_past_visibility_cutoff() {
        for step in ["zero", "restore_guard"] {
            let mut h = Host::new(false, true, L::Normal);
            h.here
                .advances
                .push_back((step, Duration::from_millis(2500)));
            let mut op = h.operation();
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Failed, false, false);
            assert_eq!(h.here.count("zero"), 1);
            assert_eq!(h.here.count("map"), 0);
            assert!(h.mutations.is_empty());
        }
    }

    #[test]
    fn layer_deadline_and_final_read_never_authorize_late_addition() {
        let mut h = Host::new(false, false, L::Below);
        h.layer_changes = false;
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        let admitted = op.bring_top.layer_admitted.unwrap();
        assert_eq!(
            op.recover_layer_cutoff(),
            Some(admitted + Duration::from_millis(1000))
        );
        assert_eq!(h.here.count("remove"), 1);
        h.here.current.layer = L::Normal;
        h.here.now = op.recover_layer_cutoff().unwrap();
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Failed, false, false);
        assert_eq!(h.here.count("above"), 0);
        assert_eq!(h.here.count("remove"), 1);
    }

    #[test]
    fn minimized_here_leaves_one_top_correction_with_actual_confirmation() {
        let mut h = Host::new(false, true, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        assert!(op.placement.correction().is_none());
        h.restored();
        h.here.actual.x = -100;
        assert!(h.run(&mut op).is_none());
        let consumed = op.placement.correction().unwrap();
        assert_eq!(h.here.count("correct"), 1);
        assert_eq!(h.here.count("shape"), 0);
        h.here.now = consumed.1;
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Success, false, true);
        assert_eq!(op.placement.correction(), Some(consumed));
        assert_eq!(h.here.count("correct"), 1);
        assert!(h.here.runtime.input_fence.automatic_correction_blocked);
    }

    #[test]
    fn consumed_here_correction_blocks_top_correction_and_cannot_be_replenished() {
        let mut h = Host::new(false, false, L::Normal);
        h.here.actual.x = -100;
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        let consumed = op.placement.correction().unwrap();
        h.here.now = consumed.1;
        let actual = h.here.actual;
        h.here
            .geometries
            .extend([actual, actual, Rect::new(-100, 100, 160, 160)]);
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Failed, false, false);
        assert_eq!(op.recover_phase, RecoverPhase::Top);
        assert_eq!(op.placement.correction(), Some(consumed));
        assert_eq!(h.here.count("correct"), 1);
        assert!(h.here.runtime.input_fence.automatic_correction_blocked);
        assert!(h.mutations.is_empty());
    }

    #[test]
    fn top_correction_needs_full_interval_and_refused_correction_never_retries() {
        for enough_time in [false, true] {
            let mut h = Host::new(false, true, L::Normal);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.restored();
            h.here.actual.x = -100;
            h.here.refuse_correction = true;
            if !enough_time {
                h.here.now = op.bring_top.visibility_cutoff - Duration::from_millis(149);
            }
            let mut result = h.run(&mut op);
            if enough_time {
                assert!(result.is_none());
                h.here.now = op.placement.correction().unwrap().1;
                result = h.run(&mut op);
            }
            assert_result(&op, result.unwrap(), Status::Partial, false, true);
            assert_eq!(h.here.count("correct"), usize::from(enough_time));
            assert_eq!(h.here.count("shape"), 0);
            assert!(h.mutations.is_empty());
        }
    }

    #[test]
    fn strict_drift_at_here_and_top_correction_boundaries_prevents_dispatch() {
        for top in [false, true] {
            let mut reference = Host::new(false, top, L::Normal);
            let mut op = reference.operation();
            if top {
                assert!(reference.run(&mut op).is_none());
                reference.restored();
            }
            let prior = reference.here.count("layout");
            reference.here.actual.x = -100;
            assert!(reference.run(&mut op).is_none());
            assert_eq!(reference.here.count("correct"), 1);
            let last = reference.here.count("layout");
            for index in prior + 1..=last {
                let mut h = Host::new(false, top, L::Normal);
                let mut op = h.operation();
                if top {
                    assert!(h.run(&mut op).is_none());
                    h.restored();
                }
                h.here.actual.x = -100;
                h.here
                    .layouts
                    .extend(std::iter::repeat_n(Ok(layout()), index - prior - 1));
                h.here.layouts.push_back(Err(DestinationError::Changed));
                let result = h.run(&mut op).unwrap();
                assert_result(
                    &op,
                    result,
                    if top { Status::Partial } else { Status::Failed },
                    false,
                    top,
                );
                assert_eq!(h.here.count("correct"), 0, "Top={top} layout read={index}");
                assert_eq!(h.here.count("shape"), 0);
                assert!(h.mutations.is_empty());
            }
        }
    }

    #[test]
    fn minimized_body_size_is_checked_before_any_restoration_mutation() {
        let mut h = Host::new(false, true, L::Normal);
        h.here.actual.width = 161;
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Failed, false, false);
        assert_eq!(result.detail().reason, Reason::Unverifiable);
        assert_eq!(h.here.count("zero"), 0);
        assert_eq!(h.here.count("map"), 0);
        assert!(h.mutations.is_empty());
    }

    #[test]
    fn wrong_actual_body_size_and_off_workspace_top_cannot_enable_or_raise() {
        for elsewhere in [false, true] {
            let mut h = Host::new(false, true, L::Below);
            let mut op = h.operation();
            assert!(h.run(&mut op).is_none());
            h.restored();
            if elsewhere {
                h.here.current.desktop = 3;
            } else {
                h.here.actual.width = 161;
            }
            let result = h.run(&mut op).unwrap();
            assert_result(&op, result, Status::Partial, false, true);
            assert_eq!(h.here.count("shape"), 0);
            assert!(h.mutations.is_empty());
        }
    }

    #[test]
    fn fence_pointer_and_popup_obligations_precede_restore_and_input_enable() {
        let mut h = Host::new(false, true, L::Below);
        h.here.runtime = visibility_fence_tests::runtime(4);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        let zero = h.here.log.iter().position(|&step| step == "zero").unwrap();
        for step in ["empty", "release", "popup"] {
            assert!(h.here.log.iter().position(|&item| item == step).unwrap() < zero);
        }
        assert!(h.here.runtime.visibility_dispatch_eligible());
        h.restored();
        assert_eq!(h.run(&mut op).unwrap().status(), Status::Success);
        assert!(here_runtime_ready(&h.here.runtime, &h.here.window));
    }

    #[test]
    fn original_native_errors_survive_each_internal_boundary_without_retry() {
        let mut reference = Host::new(true, true, L::Below);
        let mut op = reference.operation();
        drive(&mut reference, &mut op).unwrap();
        for step in [
            "desktop", "zero", "map", "layout", "geometry", "shape", "remove", "above",
        ] {
            for before_cookie in [false, true] {
                let mut h = Host::new(true, true, L::Below);
                h.here.send_before_cookie = before_cookie && step == "desktop";
                h.fail_before_cookie = before_cookie.then_some(step);
                h.here.faults.push((step, 1));
                let mut op = h.operation();
                let result = drive(&mut h, &mut op);
                assert_eq!(
                    result.unwrap_err().downcast_ref::<NativeFault>(),
                    Some(&NativeFault(step, 1))
                );
                assert_eq!(h.here.count(step), 1);
                assert_eq!(
                    h.here.count("map"),
                    usize::from(!["desktop", "zero", "layout", "geometry"].contains(&step))
                );
                assert_eq!(op.request.command, Command::Recover);
            }
        }
    }

    #[test]
    fn uncertain_correction_is_consumed_and_cleanup_cannot_replace_original_error() {
        let mut h = Host::new(false, true, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.restored();
        h.here.actual.x = -100;
        h.here.faults.push(("correct", 1));
        let error = service_recover_with(&mut op, &mut h).unwrap_err();
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault("correct", 1))
        );
        let consumed = op.placement.correction();
        assert!(consumed.is_some());
        h.here.now = consumed.unwrap().1;
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Partial, false, true);
        assert_eq!(h.here.count("correct"), 1);
        assert_eq!(op.placement.correction(), consumed);

        let mut h = Host::new(false, true, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.restored();
        h.here
            .faults
            .extend([("shape", 1), ("empty", 2), ("empty", 3)]);
        let error = service_recover_with(&mut op, &mut h).unwrap_err();
        assert_eq!(
            error.downcast_ref::<NativeFault>(),
            Some(&NativeFault("shape", 1))
        );
        assert!(h.here.runtime.input_fence.blocked);
        assert_eq!(h.here.count("shape"), 1);
        assert!(h.mutations.is_empty());
    }

    #[test]
    fn pointer_release_or_popup_failure_prevents_handoff_and_restore() {
        for step in ["release", "popup"] {
            let mut h = Host::new(false, true, L::Below);
            h.here.runtime = visibility_fence_tests::runtime(4);
            h.here.faults.push((step, 1));
            let mut op = h.operation();
            let error = service_recover_with(&mut op, &mut h).unwrap_err();
            assert_eq!(
                error.downcast_ref::<NativeFault>(),
                Some(&NativeFault(step, 1))
            );
            assert_eq!(op.recover_phase, RecoverPhase::Here);
            assert_eq!(h.here.count("zero"), 0);
            assert_eq!(h.here.count("map"), 0);
            assert!(h.mutations.is_empty());
        }
    }

    #[test]
    fn delayed_geometry_and_layer_support_reads_cannot_authorize_mutation() {
        let mut h = Host::new(false, true, L::Normal);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        h.restored();
        h.here
            .advances
            .push_back(("geometry", Duration::from_millis(2500)));
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Partial, false, true);
        assert_eq!(h.here.count("shape"), 0);
        assert!(h.mutations.is_empty());

        let mut h = Host::new(false, false, L::Normal);
        h.here
            .advances
            .push_back(("support", Duration::from_millis(1000)));
        let mut op = h.operation();
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Failed, false, false);
        assert!(h.mutations.is_empty());
    }

    #[test]
    fn final_read_can_confirm_already_completed_layer_but_not_checked_transport() {
        let mut h = Host::new(false, false, L::Normal);
        h.layer_changes = false;
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        assert_eq!(h.here.count("above"), 1);
        h.here.current.layer = L::Above;
        h.here.now = op.recover_layer_cutoff().unwrap();
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Success, false, false);
        assert_eq!(h.here.count("above"), 1);

        let mut h = Host::new(false, true, L::Above);
        let mut op = h.operation();
        assert!(h.run(&mut op).is_none());
        assert_eq!(h.here.count("map"), 1);
        h.here.now = op.bring_top.visibility_cutoff;
        let result = h.run(&mut op).unwrap();
        assert_result(&op, result, Status::Failed, false, false);
        assert_eq!(h.here.count("map"), 1);
        assert_eq!(h.here.count("shape"), 0);
    }

    #[test]
    fn composition_local_remote_visible_minimized_all_layers_retains_one_identity() {
        for remote in [false, true] {
            for minimized in [false, true] {
                for layer in [L::Normal, L::Above, L::Below] {
                    let mut h = Host::new(remote, minimized, layer);
                    let mut op = h.operation();
                    let deadline = op.deadline;
                    let initial = h.here.now;
                    let mut result = h.run(&mut op);
                    if remote {
                        assert!(result.is_none());
                        assert_eq!(op.recover_phase, RecoverPhase::Here);
                        assert_eq!(h.here.count("map"), 0);
                        assert!(h.mutations.is_empty());
                        h.here.target();
                        result = h.run(&mut op);
                    }
                    if minimized {
                        assert!(result.is_none());
                        assert_eq!(op.recover_phase, RecoverPhase::Top);
                        assert_eq!(
                            h.here.count("map"),
                            1,
                            "handoff must immediately start Show"
                        );
                        assert!(h.mutations.is_empty());
                        assert!(!op.placement.proof.history.restoration_confirmed);
                        h.restored();
                        result = h.run(&mut op);
                    }
                    assert_result(&op, result.unwrap(), Status::Success, remote, minimized);
                    assert_eq!(op.deadline, deadline);
                    assert_eq!(
                        op.bring_top.visibility_cutoff,
                        initial + Duration::from_millis(2500)
                    );
                    assert_eq!(op.placement.layout(), Some(layout()));
                    assert_eq!(op.placement.baseline.unwrap().layer, layer);
                    assert_eq!(h.here.count("desktop"), usize::from(remote));
                    assert_eq!(h.here.count("zero"), usize::from(minimized));
                    assert_eq!(h.here.count("map"), usize::from(minimized));
                    assert_eq!(h.here.count("above"), usize::from(layer != L::Above));
                    assert_eq!(h.here.count("remove"), usize::from(layer == L::Below));
                    assert_eq!(h.here.count("correct"), 0);
                    assert!(!h.here.runtime.input_fence.blocked);
                }
            }
        }
    }
}

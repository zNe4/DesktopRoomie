use super::*;
use control::{Capability, Evidence, LayerObservation, MapState, Visibility, WmState, Workspace};
use std::cell::RefCell;
use x11::monitors::destination::{
    DestinationError, DestinationLayout, RetainedMonitor, ViewportEvidence,
};
use x11::visibility::{Observation, VisibilitySnapshot};

fn destination_layout() -> DestinationLayout {
    let area = geometry::Rect::new(-1920, -100, 1800, 1000);
    DestinationLayout {
        desktop_count: 5,
        destination_desktop: 2,
        current_desktop: 2,
        monitor: RetainedMonitor {
            name_atom: 0xf000_1234,
            bounds: area,
        },
        target_workarea: Some(area),
        viewport: ViewportEvidence::AllZero,
        usable_area: area,
        body_size: Size::new(160, 160),
        valid_origins: checked_probe_bounds(area).unwrap(),
    }
}

fn layout() -> control::DestinationIdentity {
    (&destination_layout()).into()
}

pub(super) fn snapshot(
    wm_state: WmState,
    map: MapState,
    hidden: bool,
    body: u32,
    layer: LayerObservation,
) -> VisibilitySnapshot {
    let evidence = control::VisibilityEvidence {
        destroyed: false,
        wm_state,
        map_state: Evidence::Readable(map),
        hidden: Evidence::Readable(hidden),
        workspace: if body == u32::MAX {
            Workspace::AllDesktops
        } else if body == 2 {
            Workspace::Current
        } else {
            Workspace::Other
        },
        capability: Capability::Supported,
        coherent: true,
    };
    let mut state = vec![];
    if hidden {
        state.push(13);
    }
    match layer {
        LayerObservation::Above => state.push(14),
        LayerObservation::Below => state.push(15),
        _ => {}
    }
    VisibilitySnapshot {
        candidate: control::classify(evidence),
        layer,
        observation: Some(Observation {
            wm_state,
            map_state: evidence.map_state,
            state: Evidence::Readable(state),
            desktop: Evidence::Readable(body),
            current_desktop: Evidence::Readable(2),
            supported: Evidence::Readable(vec![12, 13, 14, 15, 16, 17]),
        }),
        evidence,
    }
}

fn local() -> VisibilitySnapshot {
    snapshot(
        WmState::Normal,
        MapState::Viewable,
        false,
        2,
        LayerObservation::Normal,
    )
}

fn settled_runtime() -> ProbeRuntime {
    let mut runtime = ProbeRuntime::new();
    runtime.body = BodyAvailability::Ready;
    runtime
        .fence_visibility_input_with(
            &mut tests::test_window(),
            || Ok(1),
            |_, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap();
    assert!(runtime.visibility_dispatch_eligible());
    runtime
}

#[test]
fn here_observation_keeps_ordinary_membership_separate_from_transitional_readiness() {
    for (wm, map, hidden, body, expected) in [
        (
            WmState::Normal,
            MapState::Viewable,
            false,
            2,
            Visibility::NotMinimizedHere,
        ),
        (
            WmState::Iconic,
            MapState::Unmapped,
            false,
            3,
            Visibility::NotMinimizedElsewhere,
        ),
        (
            WmState::Normal,
            MapState::Unmapped,
            false,
            2,
            Visibility::Transitional,
        ),
        (
            WmState::Normal,
            MapState::Viewable,
            true,
            2,
            Visibility::Transitional,
        ),
    ] {
        let s = snapshot(wm, map, hidden, body, LayerObservation::Below);
        let observation = here_observation(&s);
        assert_eq!(observation.membership, Some((body, 2)));
        assert_eq!(observation.visibility, expected);
        assert_eq!(observation.layer, LayerObservation::Below);
        if expected == Visibility::Transitional {
            let mut pure = control::PlacementProgress::new(2);
            assert_eq!(
                pure.observe_here(Ok(layout()), observation),
                control::HereDecision::Invalid(control::Reason::Unverifiable)
            );
            assert_eq!(
                pure.baseline, None,
                "transitional evidence is not preflight readiness"
            );
        }
    }
}

#[test]
fn minimized_snapshot_keeps_each_preserved_layer_without_remote_layer_qualification() {
    for body in [2, 3] {
        for layer in [
            LayerObservation::Normal,
            LayerObservation::Above,
            LayerObservation::Below,
        ] {
            let s = snapshot(WmState::Iconic, MapState::Unmapped, true, body, layer);
            let observation = here_observation(&s);
            assert_eq!(observation.membership, Some((body, 2)));
            assert_eq!(observation.visibility, Visibility::Minimized);
            assert_eq!(observation.layer, layer);
            let mut pure = control::PlacementProgress::new(2);
            assert!(matches!(
                pure.observe_here(Ok(layout()), observation),
                control::HereDecision::Original | control::HereDecision::Destination
            ));
            assert_eq!(pure.baseline.unwrap().layer, layer);
            assert!(pure.baseline.unwrap().minimized);
        }
    }
}

#[test]
fn sticky_missing_malformed_or_incoherent_membership_never_becomes_ordinary() {
    for case in 0..11 {
        let mut s = local();
        match case {
            0 => s.observation.as_mut().unwrap().desktop = Evidence::Readable(u32::MAX),
            1 => s.observation.as_mut().unwrap().desktop = Evidence::Missing,
            2 => s.observation.as_mut().unwrap().desktop = Evidence::Malformed,
            3 => s.observation.as_mut().unwrap().current_desktop = Evidence::Readable(u32::MAX),
            4 => s.observation.as_mut().unwrap().current_desktop = Evidence::Missing,
            5 => s.observation.as_mut().unwrap().current_desktop = Evidence::Malformed,
            6 => s.evidence.coherent = false,
            7 => s.evidence.capability = Capability::Unsupported,
            8 => s.evidence.capability = Capability::Unverifiable,
            9 => s.evidence.destroyed = true,
            _ => s.observation = None,
        }
        assert_eq!(here_observation(&s).membership, None, "case {case}");
    }
}

#[test]
fn missing_state_never_invents_normal_or_visibility_but_membership_is_independent() {
    let mut s = local();
    s.observation.as_mut().unwrap().state = Evidence::Missing;
    s.evidence.hidden = Evidence::Missing;
    s.candidate = control::classify(s.evidence);
    s.layer = LayerObservation::Unknown; // Accepted observer's missing-state result.
    assert_eq!(
        here_observation(&s),
        control::HereObservation {
            membership: Some((2, 2)),
            visibility: Visibility::Unknown,
            layer: LayerObservation::Unknown,
        }
    );
}

#[test]
fn here_round_brackets_fresh_visibility_with_two_layout_reads_without_retry() {
    let trace = RefCell::new(vec![]);
    let evidence = here_evidence_with(
        || {
            trace.borrow_mut().push("layout");
            Ok(Ok(layout()))
        },
        || {
            trace.borrow_mut().push("visibility");
            Ok(local())
        },
    )
    .unwrap();
    assert_eq!(*trace.borrow(), ["layout", "visibility", "layout"]);
    assert_eq!(evidence.layout, Ok(layout()));
    assert_eq!(evidence.observation.membership, Some((2, 2)));
    assert!(evidence.snapshot.evidence.coherent);
    assert_eq!(evidence.snapshot.layer, LayerObservation::Normal);
}

#[test]
fn layout_drift_or_failure_withholds_layout_without_erasing_independent_membership() {
    for case in 0..4 {
        let mut reads = 0;
        let evidence = here_evidence_with(
            || {
                reads += 1;
                if (case == 0 && reads == 1) || (case == 1 && reads == 2) {
                    return Ok(Err(DestinationError::Unverifiable("bad layout".into())));
                }
                let mut l = layout();
                if reads == 2 && case == 2 {
                    l.target_workarea = None;
                }
                if reads == 2 && case == 3 {
                    l.viewport = control::DestinationViewport::Absent;
                }
                Ok(Ok(l))
            },
            || Ok(local()),
        )
        .unwrap();
        assert_eq!(reads, 2);
        if case < 2 {
            assert_eq!(
                evidence.layout,
                Err(DestinationError::Unverifiable("bad layout".into()))
            );
        } else {
            assert_eq!(evidence.layout, Err(DestinationError::Changed));
        }
        assert_eq!(evidence.observation.membership, Some((2, 2)));
    }
}

#[derive(Debug)]
struct NativeEvidenceFailure(u32);

impl std::fmt::Display for NativeEvidenceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "injected native evidence failure {}", self.0)
    }
}

impl std::error::Error for NativeEvidenceFailure {}

#[test]
fn native_failure_at_each_here_read_preserves_original_error_and_stops_immediately() {
    for (failed_step, invalid_first) in [(0, false), (1, false), (2, false), (1, true), (2, true)] {
        let original = Box::new(NativeEvidenceFailure(17));
        let original_address = std::ptr::from_ref(original.as_ref());
        let error = RefCell::new(Some(original));
        let trace = RefCell::new(vec![]);
        let result = here_evidence_with(
            || {
                let step = trace.borrow().len();
                trace.borrow_mut().push("layout");
                if step == failed_step {
                    Err(error.borrow_mut().take().unwrap())
                } else if step == 0 && invalid_first {
                    Ok(Err(DestinationError::Unverifiable("bad layout".into())))
                } else {
                    Ok(Ok(layout()))
                }
            },
            || {
                let step = trace.borrow().len();
                trace.borrow_mut().push("visibility");
                if step == failed_step {
                    Err(error.borrow_mut().take().unwrap())
                } else {
                    Ok(local())
                }
            },
        );
        let native = result.err().expect("Native failure must escape the round");
        assert!(std::ptr::eq(
            native.downcast_ref::<NativeEvidenceFailure>().unwrap(),
            original_address
        ));
        assert_eq!(
            *trace.borrow(),
            ["layout", "visibility", "layout"][..=failed_step]
        );
    }
}

#[test]
fn here_native_fit_keeps_the_existing_signed_wire_limits() {
    for origin in [i32::from(i16::MIN), i32::from(i16::MAX)] {
        let area = geometry::Rect::new(origin, origin, 160, 160);
        assert!(checked_probe_bounds(area)
            .unwrap()
            .contains(Point::new(origin, origin)));
    }
    assert!(checked_probe_bounds(geometry::Rect::new(i32::from(i16::MAX), 0, 161, 160)).is_err());
    assert!(
        checked_probe_bounds(geometry::Rect::new(i32::from(i16::MIN) - 1, 0, 160, 160)).is_err()
    );
}

#[test]
fn here_native_fit_failure_remains_inner_evidence_with_independent_membership() {
    let mut bad = destination_layout();
    bad.usable_area = geometry::Rect::new(i32::from(i16::MAX), 0, 161, 160);
    let mut reads = 0;
    let evidence = here_evidence_with(
        || {
            reads += 1;
            Ok(here_destination_identity(bad.clone()))
        },
        || Ok(local()),
    )
    .unwrap();
    assert_eq!(reads, 2);
    assert!(matches!(
        evidence.layout,
        Err(DestinationError::Unverifiable(_))
    ));
    assert_eq!(evidence.observation.membership, Some((2, 2)));
    assert_eq!(
        here_destination_identity(destination_layout()),
        Ok(layout())
    );
}

#[test]
fn here_transport_refuses_every_unsettled_fence_before_calling_native() {
    let now = Instant::now();
    for case in 0..13 {
        let mut runtime = settled_runtime();
        match case {
            0 => runtime.input_fence.blocked = false,
            1 => runtime.input_fence.empty_confirmed = false,
            2 => runtime.pointer.require_transition_release(),
            3 => runtime.pointer.set_grabbed(true),
            4 => runtime.pending_move = Some(Point::new(1, 1)),
            5 => runtime.final_target = Some(Point::new(1, 1)),
            6 => runtime.confirmation_deadline = Some(now),
            7 => runtime.safety.deadline = Some(now),
            8 => runtime.body = BodyAvailability::Destroyed,
            9 => runtime.correction = Some((Point::new(1, 1), now)),
            10 => runtime.menu = Some(MenuPopup::test_popup(200, 1)),
            11 => runtime.safety.observation = Some((1, ButtonObservation::Held, 1)),
            _ => {
                runtime.interaction = InteractionManager::new();
                runtime.interaction.handle_left_press(
                    Point::new(180, 180),
                    (80, 80),
                    1000,
                    Point::new(100, 100),
                );
                runtime.interaction.on_grab_acquired();
            }
        }
        assert!(!runtime.visibility_dispatch_eligible(), "case {case}");
        let mut dispatched = false;
        assert!(request_here_desktop_with(
            &runtime,
            11,
            22,
            33,
            2,
            now,
            &mut dispatched,
            |_, _, _, _, _, _| panic!("unsettled fence must not dispatch")
        )
        .is_err());
        assert!(!dispatched);
    }
    let mut dispatched = true;
    assert!(request_here_desktop_with(
        &settled_runtime(),
        11,
        22,
        33,
        u32::MAX,
        now,
        &mut dispatched,
        |_, _, _, _, _, _| panic!("sticky target must not dispatch")
    )
    .is_err());
    assert!(
        dispatched,
        "refusal cannot clear previous possible dispatch"
    );
}

#[test]
fn here_transport_passes_exact_admitted_envelope_deadline_and_dispatch_reference_once() {
    let now = Instant::now();
    let deadline = now + Duration::from_millis(3500);
    let runtime = settled_runtime();
    // Stage-C outcomes injected at its consuming send_before boundary.
    for outcome in ["send failure", "check failure", "expired", "checked"] {
        for prior_dispatch in [false, true] {
            let mut dispatched = prior_dispatch;
            let mut calls = 0;
            let result = request_here_desktop_with(
                &runtime,
                0x1234_5678,
                0x2345_6789,
                0x3456_789a,
                4,
                deadline,
                &mut dispatched,
                |root, body, atom, target, dl, mark| {
                    calls += 1;
                    assert_eq!(
                        (root, body, atom, target, dl),
                        (0x1234_5678, 0x2345_6789, 0x3456_789a, 4, deadline)
                    );
                    assert_eq!(*mark, prior_dispatch);
                    match outcome {
                        "send failure" => Err("send failure".into()),
                        "expired" => Ok(false),
                        _ => {
                            *mark = true;
                            if outcome == "check failure" {
                                Err("check failure".into())
                            } else {
                                Ok(true)
                            }
                        }
                    }
                },
            );
            assert_eq!(calls, 1);
            assert_eq!(
                dispatched,
                prior_dispatch || matches!(outcome, "check failure" | "checked")
            );
            match outcome {
                "expired" => assert!(!result.unwrap()),
                "checked" => assert!(result.unwrap()),
                error => assert_eq!(result.unwrap_err().to_string(), error),
            }
        }
    }
}

#[test]
fn checked_transport_is_not_relocation_but_fresh_transitional_membership_can_be() {
    let now = Instant::now();
    let mut active = None;
    control::admit(
        &mut active,
        control::Request {
            command: control::Command::BringHere,
            request_id: 99,
            reply_xid: 88,
            epoch: 7,
        },
        2,
        control::Lifecycle::Ready,
        false,
        now,
    );
    let mut op = active.unwrap();
    let original = here_observation(&snapshot(
        WmState::Iconic,
        MapState::Unmapped,
        false,
        3,
        LayerObservation::Normal,
    ));
    assert_eq!(
        op.placement.observe_here(Ok(layout()), original),
        control::HereDecision::Original
    );
    assert!(op.placement.here_fenced());
    assert_eq!(
        op.placement.observe_here(Ok(layout()), original),
        control::HereDecision::Original
    );
    assert!(op.consume_here_desktop_attempt(now));
    let mut dispatched = false;
    let checked = request_here_desktop_with(
        &settled_runtime(),
        11,
        22,
        33,
        2,
        op.deadline,
        &mut dispatched,
        |_, _, _, _, _, mark| {
            *mark = true;
            Ok(true)
        },
    )
    .unwrap();
    assert!(op
        .placement
        .record_here_desktop_transport(dispatched, checked));
    assert!(!op.placement.proof.history.relocation_confirmed);
    assert!(!op.placement.proof.current.destination);
    let transitional = here_observation(&snapshot(
        WmState::Normal,
        MapState::Unmapped,
        false,
        2,
        LayerObservation::Normal,
    ));
    assert_eq!(
        op.placement.observe_here(Ok(layout()), transitional),
        control::HereDecision::Pending
    );
    assert!(op.placement.proof.history.relocation_confirmed);
    assert!(!op.placement.begin_here_geometry());
    assert!(!op.consume_here_input_enable(now));
    assert_eq!(
        op.placement.proof.current.geometry,
        control::GeometryProof::Unknown
    );
    assert!(!op.placement.proof.history.restoration_confirmed);
}

#[test]
fn here_correction_adapter_passes_exact_target_deadline_and_uncertain_dispatch_once() {
    let now = Instant::now();
    let deadline = now + Duration::from_millis(3500);
    let target = Point::new(-1900, -80);
    for outcome in 0..3 {
        let mut runtime = settled_runtime();
        let mut dispatched = false;
        let mut calls = 0;
        let result = here_correct_with(
            &mut runtime,
            target,
            deadline,
            &mut dispatched,
            |pt, dl, mark| {
                calls += 1;
                assert_eq!((pt, dl), (target, deadline));
                if outcome == 0 {
                    return Ok(None);
                }
                *mark = true;
                if outcome == 1 {
                    Err("checked failure".into())
                } else {
                    Ok(Some(now + CONFIRMATION_TIMEOUT))
                }
            },
        );
        assert_eq!(calls, 1);
        assert_eq!(dispatched, outcome != 0);
        assert_eq!(runtime.body, BodyAvailability::Validating);
        match outcome {
            0 => assert_eq!(result.unwrap(), None),
            1 => assert_eq!(result.unwrap_err().to_string(), "checked failure"),
            _ => assert_eq!(result.unwrap(), Some(now + CONFIRMATION_TIMEOUT)),
        }
    }
    let mut runtime = ProbeRuntime::new();
    assert!(here_correct_with(
        &mut runtime,
        target,
        deadline,
        &mut false,
        |_, _, _| panic!("unsettled fence")
    )
    .is_err());
    assert_eq!(runtime.body, BodyAvailability::AwaitingMap);
}

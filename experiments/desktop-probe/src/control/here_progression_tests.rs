use super::*;

const TARGET: u32 = 1;
const ORIGINAL: u32 = 0;
const THIRD: u32 = 2;
const ORIGIN: Point = Point::new(-100, 50);
const SIZE: Size = Size::new(160, 160);

fn layout() -> DestinationIdentity {
    DestinationIdentity {
        desktop_count: 4,
        destination_desktop: TARGET,
        current_desktop: TARGET,
        monitor_name: 9,
        monitor_bounds: Rect::new(-500, 0, 1000, 800),
        target_workarea: Some(Rect::new(-500, 40, 1000, 760)),
        viewport: DestinationViewport::AllZero,
        usable_area: Rect::new(-500, 40, 1000, 760),
        body_size: SIZE,
        valid_origins: ValidOriginBounds::new(-500, 340, 40, 640),
    }
}

fn observation(body: u32, minimized: bool, layer: LayerObservation) -> HereObservation {
    HereObservation {
        membership: Some((body, TARGET)),
        visibility: if minimized {
            Visibility::Minimized
        } else if body == TARGET {
            Visibility::NotMinimizedHere
        } else {
            Visibility::NotMinimizedElsewhere
        },
        layer,
    }
}

fn operation() -> (ActiveOperation, Instant) {
    operation_for(Command::BringHere)
}

fn operation_for(command: Command) -> (ActiveOperation, Instant) {
    let now = Instant::now();
    let mut active = None;
    assert_eq!(
        admit(
            &mut active,
            Request {
                command,
                request_id: 1,
                reply_xid: 2,
                epoch: 3
            },
            TARGET,
            Lifecycle::Ready,
            false,
            now,
        ),
        Admission::Admitted
    );
    (active.unwrap(), now)
}

fn fenced(body: u32, minimized: bool, layer: LayerObservation) -> (ActiveOperation, Instant) {
    let (mut op, now) = operation();
    let sample = observation(body, minimized, layer);
    let decision = if body == TARGET {
        HereDecision::Destination
    } else {
        HereDecision::Original
    };
    assert_eq!(op.placement.observe_here(Ok(layout()), sample), decision);
    assert!(op.placement.here_fenced());
    // A fence transition requires another fresh observation before mutation.
    assert!(!op.consume_here_desktop_attempt(now));
    assert_eq!(op.placement.observe_here(Ok(layout()), sample), decision);
    (op, now)
}

fn attempted(
    minimized: bool,
    layer: LayerObservation,
    dispatched: bool,
    checked: bool,
) -> (ActiveOperation, Instant) {
    let (mut op, now) = fenced(ORIGINAL, minimized, layer);
    assert!(op.consume_here_desktop_attempt(now));
    assert!(op
        .placement
        .record_here_desktop_transport(dispatched, checked));
    (op, now)
}

fn arrived(minimized: bool, layer: LayerObservation) -> (ActiveOperation, Instant) {
    let (mut op, now) = attempted(minimized, layer, true, true);
    assert_eq!(
        op.placement
            .observe_here(Ok(layout()), observation(TARGET, minimized, layer)),
        HereDecision::Destination
    );
    (op, now)
}

fn visible_detail(layer: LayerObservation, ready: bool) -> (Detail, Confirmation) {
    (
        Detail {
            visibility: Visibility::NotMinimizedHere,
            workspace: Workspace::Current,
            layer,
            progress: Progress {
                not_minimized_confirmed: true,
                input_ready: ready,
                ..Progress::default()
            },
            ..preflight_detail(Reason::ConfirmationTimeout, Stage::Placement)
        },
        Confirmation {
            visibility: Some(ConfirmedVisibility::NotMinimizedHere),
            above: false,
        },
    )
}

fn incomplete(op: &ActiveOperation) -> OperationResult {
    let p = op.placement.proof.current;
    op.placement
        .here_result(
            Detail {
                workspace: if p.destination {
                    Workspace::Current
                } else {
                    Workspace::Unknown
                },
                layer: if p.layer_preserved {
                    op.placement.baseline.unwrap().layer
                } else {
                    LayerObservation::Unknown
                },
                ..preflight_detail(Reason::LayoutChanged, Stage::Placement)
            },
            Confirmation::default(),
        )
        .unwrap()
}

fn assert_wire(result: OperationResult) {
    let terminal = TerminalResponse {
        status: result.status(),
        detail: result.detail(),
        request_id: 1,
        control_xid: 2,
    };
    assert_eq!(
        TerminalResponse::decode(&terminal.encode().unwrap()),
        Ok(terminal)
    );
}

#[test]
fn admission_is_value_only_without_mutation_authority_and_keeps_budgets() {
    fn is_copy<T: Copy>(_: T) {}
    let (mut op, now) = operation();
    is_copy(op);
    let deadline = op.deadline;
    assert_eq!(deadline, now + Duration::from_millis(3500));
    assert_eq!(
        Command::BringHere.caller_budget(),
        Duration::from_millis(5000)
    );
    assert_eq!(op.placement.here_phase, HerePhase::Observe);
    assert!(!op.consume_here_desktop_attempt(now));
    assert!(!op.consume_here_correction(ORIGIN, now));
    assert!(!op.consume_here_input_enable(now));
    assert!(!op.placement.record_here_desktop_transport(true, true));
    assert!(!op.placement.here_fenced());
    assert_eq!(op.placement.proof, PlacementProof::default());
    let mut active = Some(op);
    assert_eq!(
        admit(
            &mut active,
            op.request,
            TARGET,
            Lifecycle::Ready,
            false,
            now + Duration::from_secs(1)
        ),
        Admission::Duplicate
    );
    assert_eq!(active.unwrap().deadline, deadline);
    let mut other = op.request;
    other.request_id += 1;
    assert_eq!(
        admit(&mut active, other, TARGET, Lifecycle::Ready, false, now),
        Admission::Reply(Status::Busy, Reason::Busy, Stage::Preflight)
    );
}

#[test]
fn every_relevant_layout_identity_change_latches_without_replacing_admission() {
    for field in 0..11 {
        let (mut op, now) = fenced(ORIGINAL, false, LayerObservation::Normal);
        let baseline = op.placement.baseline;
        let mut changed = layout();
        match field {
            0 => changed.desktop_count += 1,
            1 => changed.destination_desktop = THIRD,
            2 => changed.current_desktop = THIRD,
            3 => changed.monitor_name += 1,
            4 => changed.monitor_bounds.x += 1,
            5 => changed.target_workarea = None,
            6 => changed.target_workarea.as_mut().unwrap().height -= 1,
            7 => changed.viewport = DestinationViewport::Absent,
            8 => changed.usable_area.width -= 1,
            9 => changed.body_size.width += 1,
            10 => changed.valid_origins.max_x -= 1,
            _ => unreachable!(),
        }
        assert_eq!(
            op.placement.observe_here(
                Ok(changed),
                observation(ORIGINAL, false, LayerObservation::Normal)
            ),
            HereDecision::Invalid(Reason::LayoutChanged)
        );
        assert_eq!(
            op.placement.observe_here(
                Ok(layout()),
                observation(ORIGINAL, false, LayerObservation::Normal)
            ),
            HereDecision::Invalid(Reason::LayoutChanged)
        );
        assert_eq!(op.placement.layout, Some(layout()));
        assert_eq!(op.placement.baseline, baseline);
        assert_eq!(op.placement.destination_desktop, TARGET);
        assert!(!op.consume_here_desktop_attempt(now));
    }
}

#[test]
fn safe_local_noop_can_succeed_without_fencing_or_relocation() {
    for layer in [
        LayerObservation::Normal,
        LayerObservation::Above,
        LayerObservation::Below,
    ] {
        let (mut op, now) = operation();
        assert_eq!(
            op.placement
                .observe_here(Ok(layout()), observation(TARGET, false, layer)),
            HereDecision::Destination
        );
        assert!(!op.consume_here_desktop_attempt(now));
        assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
        let (d, c) = visible_detail(layer, true);
        let result = op.placement.here_result(d, c).unwrap();
        assert_eq!(result.status(), Status::Success);
        assert!(!result.detail().progress.host_mutation_dispatched);
        assert_eq!(op.placement.proof.history, RecoveryHistory::default());
        assert_eq!(op.placement.here_phase, HerePhase::Observe);
        assert_wire(result);
    }
}

#[test]
fn desktop_attempt_is_consumed_once_for_send_failure_check_failure_and_success() {
    for (dispatched, checked) in [(false, false), (true, false), (true, true)] {
        let (mut op, now) = attempted(false, LayerObservation::Normal, dispatched, checked);
        let baseline = op.placement.baseline;
        let deadline = op.deadline;
        assert_eq!(
            op.placement.here_phase,
            if checked {
                HerePhase::AwaitDestination
            } else {
                HerePhase::DesktopAttempted
            }
        );
        assert!(!op.placement.record_here_desktop_transport(false, true));
        for _ in 0..3 {
            assert_eq!(
                op.placement.observe_here(
                    Ok(layout()),
                    observation(ORIGINAL, false, LayerObservation::Normal)
                ),
                HereDecision::Original
            );
            assert!(!op.consume_here_desktop_attempt(now));
            assert!(!op.placement.here_fenced());
            assert!(!op.placement.proof.history.relocation_confirmed);
            op.placement.request_here_observation();
        }
        assert_eq!(op.placement.baseline, baseline);
        assert_eq!(op.deadline, deadline);
        assert!(!op.consume_here_desktop_attempt(deadline));
    }
}

#[test]
fn target_before_our_dispatch_is_external_interruption_without_history() {
    for attempted in [false, true] {
        let (mut op, now) = fenced(ORIGINAL, false, LayerObservation::Normal);
        if attempted {
            assert!(op.consume_here_desktop_attempt(now));
        }
        assert_eq!(
            op.placement.observe_here(
                Ok(layout()),
                observation(TARGET, false, LayerObservation::Normal)
            ),
            HereDecision::Invalid(Reason::LayoutChanged)
        );
        assert_eq!(incomplete(&op).status(), Status::Failed);
        assert!(!op.consume_here_desktop_attempt(now));
        assert_eq!(op.placement.baseline.unwrap().desktop, ORIGINAL);
    }
}

#[test]
fn dispatch_and_checked_completion_do_not_confirm_relocation() {
    for checked in [false, true] {
        let (mut op, now) = attempted(false, LayerObservation::Normal, true, checked);
        assert!(!op.placement.proof.history.relocation_confirmed);
        op.placement.clear_current();
        let result = incomplete(&op);
        assert_eq!(result.status(), Status::Failed);
        assert!(result.detail().progress.host_mutation_dispatched);
        // A fresh target read can justify history even after a checked error.
        assert_eq!(
            op.placement.observe_here(
                Ok(layout()),
                observation(TARGET, false, LayerObservation::Normal)
            ),
            HereDecision::Destination
        );
        assert!(op.placement.proof.history.relocation_confirmed);
        assert!(!op.placement.proof.history.restoration_confirmed);
        assert!(!op.consume_here_desktop_attempt(now));
    }
}

#[test]
fn third_or_sticky_desktop_never_creates_history_or_retry_authority() {
    for body in [THIRD, NO_DESTINATION] {
        let (mut op, now) = attempted(false, LayerObservation::Normal, true, true);
        assert_eq!(
            op.placement.observe_here(
                Ok(layout()),
                observation(body, false, LayerObservation::Normal)
            ),
            HereDecision::Invalid(if body == NO_DESTINATION {
                Reason::Unsupported
            } else {
                Reason::LayoutChanged
            })
        );
        assert_eq!(op.placement.proof, PlacementProof::default());
        assert_eq!(op.placement.baseline.unwrap().desktop, ORIGINAL);
        assert_eq!(op.placement.destination_desktop, TARGET);
        assert!(!op.consume_here_desktop_attempt(now));
    }
}

#[test]
fn destination_then_original_third_or_missing_evidence_retains_only_history() {
    for membership in [Some((ORIGINAL, TARGET)), Some((THIRD, TARGET)), None] {
        let (mut op, now) = arrived(false, LayerObservation::Normal);
        assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
        let decision = op.placement.observe_here(
            Ok(layout()),
            HereObservation {
                membership,
                ..observation(TARGET, false, LayerObservation::Normal)
            },
        );
        assert_eq!(
            decision,
            if membership.is_none() {
                HereDecision::Pending
            } else {
                HereDecision::Invalid(Reason::LayoutChanged)
            }
        );
        assert_eq!(op.placement.proof.current, PlacementConfirmation::default());
        let result = incomplete(&op);
        assert_eq!(result.status(), Status::Partial);
        assert_wire(result);
        assert!(!op.consume_here_desktop_attempt(now));
    }
}

#[test]
fn sticky_and_unsettled_initial_evidence_cannot_become_baseline() {
    for sample in [
        observation(NO_DESTINATION, false, LayerObservation::Normal),
        observation(ORIGINAL, false, LayerObservation::Conflict),
        HereObservation {
            visibility: Visibility::Transitional,
            ..observation(ORIGINAL, false, LayerObservation::Normal)
        },
        HereObservation {
            membership: None,
            ..observation(ORIGINAL, false, LayerObservation::Normal)
        },
        observation(layout().desktop_count, false, LayerObservation::Normal),
    ] {
        let (mut op, now) = operation();
        assert!(matches!(
            op.placement.observe_here(Ok(layout()), sample),
            HereDecision::Invalid(_)
        ));
        assert_eq!(op.placement.baseline, None);
        assert_eq!(op.placement.layout, None);
        assert!(!op.placement.here_fenced());
        assert!(!op.consume_here_desktop_attempt(now));
        assert_eq!(incomplete(&op).status(), Status::Failed);
    }
}

#[test]
fn layer_or_genuine_minimization_drift_invalidates_without_rewriting_baseline() {
    for minimized in [false, true] {
        for layer_drift in [false, true] {
            let (mut op, now) = attempted(minimized, LayerObservation::Above, true, true);
            let baseline = op.placement.baseline;
            let sample = observation(
                TARGET,
                if layer_drift { minimized } else { !minimized },
                if layer_drift {
                    LayerObservation::Below
                } else {
                    LayerObservation::Above
                },
            );
            assert_eq!(
                op.placement.observe_here(Ok(layout()), sample),
                HereDecision::Invalid(if layer_drift {
                    Reason::LayerChanged
                } else {
                    Reason::Unverifiable
                })
            );
            assert_eq!(op.placement.baseline, baseline);
            assert_eq!(op.placement.proof.current, PlacementConfirmation::default());
            assert!(op.placement.proof.history.relocation_confirmed);
            assert!(!op.placement.proof.history.restoration_confirmed);
            assert!(!op.consume_here_desktop_attempt(now));
            assert_eq!(incomplete(&op).status(), Status::Partial);
        }
    }
}

#[test]
fn independent_membership_history_survives_rejected_layout_but_stale_current_cannot_create_it() {
    for current in [TARGET, THIRD] {
        let (mut op, _) = attempted(false, LayerObservation::Normal, true, true);
        assert_eq!(
            op.placement.observe_here(
                Err(Reason::LayoutChanged),
                HereObservation {
                    membership: Some((TARGET, current)),
                    ..observation(TARGET, false, LayerObservation::Normal)
                }
            ),
            HereDecision::Invalid(Reason::LayoutChanged)
        );
        assert_eq!(
            op.placement.proof.history.relocation_confirmed,
            current == TARGET
        );
        assert_eq!(
            incomplete(&op).status(),
            if current == TARGET {
                Status::Partial
            } else {
                Status::Failed
            }
        );
    }
}

#[test]
fn transitional_target_can_prove_membership_without_geometry_or_input_authority() {
    let (mut op, now) = attempted(false, LayerObservation::Normal, true, true);
    assert_eq!(
        op.placement.observe_here(
            Ok(layout()),
            HereObservation {
                visibility: Visibility::Transitional,
                ..observation(TARGET, false, LayerObservation::Normal)
            }
        ),
        HereDecision::Pending
    );
    assert!(op.placement.proof.current.destination);
    assert!(op.placement.proof.history.relocation_confirmed);
    assert!(!op.placement.begin_here_geometry());
    assert!(!op.placement.confirm_here_geometry(ORIGIN, SIZE));
    assert!(!op.consume_here_correction(ORIGIN, now));
    assert!(!op.consume_here_input_enable(now));
    assert_eq!(incomplete(&op).status(), Status::Partial);
}

#[test]
fn minimized_deferred_success_preserves_above_without_above_confirmation() {
    for body in [ORIGINAL, TARGET] {
        for layer in [
            LayerObservation::Normal,
            LayerObservation::Above,
            LayerObservation::Below,
        ] {
            let (mut op, now) = if body == ORIGINAL {
                arrived(true, layer)
            } else {
                fenced(body, true, layer)
            };
            assert!(!op.placement.begin_here_geometry());
            assert!(!op.consume_here_correction(ORIGIN, now));
            assert!(!op.consume_here_input_enable(now));
            assert!(op.placement.defer_here_geometry());
            let d = Detail {
                visibility: Visibility::Minimized,
                workspace: Workspace::Current,
                layer,
                progress: Progress {
                    placement_deferred: true,
                    ..Progress::default()
                },
                ..preflight_detail(Reason::None, Stage::Placement)
            };
            let c = Confirmation {
                visibility: Some(ConfirmedVisibility::Minimized),
                above: false,
            };
            let result = op.placement.here_result(d, c).unwrap();
            assert_eq!(result.status(), Status::Success);
            assert_eq!(
                result.detail().placement.history.relocation_confirmed,
                body == ORIGINAL
            );
            assert!(!result.detail().placement.history.restoration_confirmed);
            assert!(!result.detail().progress.above_confirmed);
            assert_wire(result);
            assert!(op
                .placement
                .here_result(
                    Detail {
                        progress: Progress {
                            above_confirmed: true,
                            ..d.progress
                        },
                        ..d
                    },
                    Confirmation { above: true, ..c }
                )
                .is_err());
        }
    }
}

#[test]
fn visible_completion_with_relocation_history_is_success_and_loss_is_partial() {
    let (mut op, _) = arrived(false, LayerObservation::Normal);
    assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
    let (d, c) = visible_detail(LayerObservation::Normal, true);
    let result = op.placement.here_result(d, c).unwrap();
    assert_eq!(result.status(), Status::Success);
    assert!(result.detail().placement.history.relocation_confirmed);
    assert_wire(result);
    op.placement.clear_current();
    assert_eq!(incomplete(&op).status(), Status::Partial);
}

#[test]
fn local_correction_is_one_shot_and_never_creates_partial_authority() {
    let (mut op, now) = fenced(TARGET, false, LayerObservation::Normal);
    assert!(op.placement.begin_here_geometry());
    assert!(!op.consume_here_correction(Point::new(9999, 9999), now));
    assert!(op.consume_here_correction(ORIGIN, now));
    let correction = op.placement.correction;
    // Failure before cookie acquisition still consumes this correction.
    for _ in 0..3 {
        assert_eq!(
            op.placement.observe_here(
                Ok(layout()),
                observation(TARGET, false, LayerObservation::Normal)
            ),
            HereDecision::Destination
        );
        assert!(!op.consume_here_correction(ORIGIN, now));
        assert_eq!(op.placement.correction, correction);
        assert!(!op.placement.confirm_here_geometry(Point::new(0, 50), SIZE));
        assert!(!op
            .placement
            .confirm_here_geometry(ORIGIN, Size::new(161, 160)));
    }
    assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
    let (mut d, c) = visible_detail(LayerObservation::Normal, true);
    d.progress.host_mutation_dispatched = true; // geometry dispatch is not relocation
    assert_eq!(
        op.placement.here_result(d, c).unwrap().status(),
        Status::Success
    );
    op.placement.clear_current();
    let failed = incomplete(&op);
    assert_eq!(failed.status(), Status::Failed);
    assert_eq!(op.placement.proof.history, RecoveryHistory::default());
    assert_wire(failed);
    assert!(!op.consume_here_desktop_attempt(now));
}

#[test]
fn correction_reserves_full_interval_and_never_renews_owner_deadline() {
    for remaining in [0, 1, 149, 150, 151] {
        let (mut op, _) = fenced(TARGET, false, LayerObservation::Normal);
        assert!(op.placement.begin_here_geometry());
        let deadline = op.deadline;
        let now = deadline - Duration::from_millis(remaining);
        assert_eq!(op.consume_here_correction(ORIGIN, now), remaining >= 150);
        assert_eq!(op.deadline, deadline);
        if remaining >= 150 {
            let correction = op.placement.correction.unwrap();
            assert_eq!(correction.1, now + Duration::from_millis(150));
            assert!(correction.1 <= deadline);
            op.placement.observe_here(
                Ok(layout()),
                observation(TARGET, false, LayerObservation::Normal),
            );
            assert!(!op.consume_here_correction(ORIGIN, now));
            assert_eq!(op.placement.correction, Some(correction));
        } else {
            assert_eq!(op.placement.correction, None);
        }
    }
}

#[test]
fn all_phases_require_fresh_proof_and_input_enable_cannot_repeat() {
    let (mut op, now) = arrived(false, LayerObservation::Normal);
    assert!(!op.consume_here_input_enable(now));
    assert!(op.placement.begin_here_geometry());
    assert!(!op.consume_here_input_enable(now));
    assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
    assert!(!op.consume_here_input_enable(op.deadline));
    assert!(op.consume_here_input_enable(now));
    assert_eq!(op.placement.here_phase, HerePhase::EnablingInput);
    assert!(!op.consume_here_input_enable(now));
    assert!(op.here_input_restored());
    assert_eq!(op.placement.here_phase, HerePhase::InputRestored);
    assert_eq!(op.placement.proof.current, PlacementConfirmation::default());
    assert!(!op.here_input_restored());
    op.placement.observe_here(
        Ok(layout()),
        observation(TARGET, false, LayerObservation::Normal),
    );
    assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
    assert!(!op.consume_here_input_enable(now));
    let (d, c) = visible_detail(LayerObservation::Normal, true);
    assert_eq!(
        op.placement.here_result(d, c).unwrap().status(),
        Status::Success
    );
    assert!(!op.placement.proof.history.restoration_confirmed);
}

#[test]
fn shutdown_clears_previously_complete_proof_and_keeps_exact_history_partition() {
    for relocated in [false, true] {
        let (mut op, now) = if relocated {
            arrived(false, LayerObservation::Above)
        } else {
            fenced(TARGET, false, LayerObservation::Above)
        };
        assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
        let (d, c) = visible_detail(LayerObservation::Above, true);
        assert_eq!(
            op.placement.here_result(d, c).unwrap().status(),
            Status::Success
        );
        op.detail = d;
        for reason in [
            Reason::Closing,
            Reason::X11Failure,
            Reason::BodyDestroyed,
            Reason::OwnershipLost,
        ] {
            let mut shutdown = op;
            let result = shutdown.here_shutdown(reason).unwrap();
            assert_eq!(
                result.status(),
                if relocated {
                    Status::Partial
                } else {
                    Status::Failed
                }
            );
            assert_eq!(result.detail().stage, Stage::Shutdown);
            assert_eq!(
                result.detail().placement.current,
                PlacementConfirmation::default()
            );
            assert!(!result.detail().progress.input_ready);
            assert!(!result.detail().progress.above_confirmed);
            assert_wire(result);
            assert!(matches!(
                shutdown.placement.observe_here(
                    Ok(layout()),
                    observation(TARGET, false, LayerObservation::Above)
                ),
                HereDecision::Invalid(_)
            ));
            assert!(!shutdown.placement.confirm_here_geometry(ORIGIN, SIZE));
            assert!(!shutdown.consume_here_desktop_attempt(now));
        }
    }
}

#[test]
fn fatal_teardown_can_add_late_history_without_recreating_current_success() {
    let (mut op, _) = attempted(true, LayerObservation::Above, true, false);
    assert_eq!(
        op.here_shutdown(Reason::X11Failure).unwrap().status(),
        Status::Failed
    );
    assert_eq!(
        op.placement.observe_here(
            Ok(layout()),
            observation(TARGET, true, LayerObservation::Above)
        ),
        HereDecision::Invalid(Reason::X11Failure)
    );
    let result = op.here_shutdown(Reason::X11Failure).unwrap();
    assert_eq!(result.status(), Status::Partial);
    assert!(result.detail().progress.host_mutation_dispatched);
    assert!(!op.placement.defer_here_geometry());
    assert_wire(result);
}

#[test]
fn desktop_expiry_does_not_consume_or_renew_authority() {
    let (mut op, _) = fenced(ORIGINAL, false, LayerObservation::Normal);
    let deadline = op.deadline;
    assert!(!op.consume_here_desktop_attempt(deadline));
    assert!(!op.consume_here_desktop_attempt(deadline + Duration::from_millis(1)));
    assert!(!op.placement.desktop_attempted);
    assert!(!op.placement.record_here_desktop_transport(true, true));
    assert_eq!(op.deadline, deadline);
    assert_eq!(incomplete(&op).status(), Status::Failed);
}

#[test]
fn losing_and_refreshing_evidence_rebuilds_current_proof_without_resetting_attempts() {
    let (mut op, now) = arrived(false, LayerObservation::Normal);
    assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
    op.placement.request_here_observation();
    assert_eq!(op.placement.proof.current, PlacementConfirmation::default());
    assert_eq!(incomplete(&op).status(), Status::Partial);
    assert_eq!(
        op.placement.observe_here(
            Ok(layout()),
            observation(TARGET, false, LayerObservation::Normal)
        ),
        HereDecision::Destination
    );
    assert_eq!(op.placement.proof.current.geometry, GeometryProof::Unknown);
    assert!(!op.consume_here_desktop_attempt(now));
    assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
    let (d, c) = visible_detail(LayerObservation::Normal, true);
    assert_eq!(
        op.placement.here_result(d, c).unwrap().status(),
        Status::Success
    );
}

#[test]
fn failed_input_enable_is_consumed_even_when_fresh_evidence_recovers() {
    let (mut op, now) = arrived(false, LayerObservation::Normal);
    assert!(op.placement.begin_here_geometry());
    assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
    assert!(op.consume_here_input_enable(now));
    // No checked restoration was reported; a later read grants no retry.
    op.placement.request_here_observation();
    op.placement.observe_here(
        Ok(layout()),
        observation(TARGET, false, LayerObservation::Normal),
    );
    assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
    assert!(!op.consume_here_input_enable(now));
    let (d, c) = visible_detail(LayerObservation::Normal, false);
    assert_eq!(
        op.placement.here_result(d, c).unwrap().status(),
        Status::Partial
    );
    assert_eq!(
        op.here_shutdown(Reason::X11Failure).unwrap().status(),
        Status::Partial
    );
    assert!(!op.here_input_restored());
}

#[test]
fn shutdown_at_every_phase_preserves_only_observed_history() {
    let (observe, now) = operation();
    let (fence, _) = fenced(ORIGINAL, false, LayerObservation::Normal);
    let mut attempted = fence;
    assert!(attempted.consume_here_desktop_attempt(now));
    let mut waiting = attempted;
    assert!(waiting.placement.record_here_desktop_transport(true, true));
    let mut geometry = waiting;
    geometry.placement.observe_here(
        Ok(layout()),
        observation(TARGET, false, LayerObservation::Normal),
    );
    assert!(geometry.placement.begin_here_geometry());
    let mut enabling = geometry;
    assert!(enabling.placement.confirm_here_geometry(ORIGIN, SIZE));
    assert!(enabling.consume_here_input_enable(now));
    let mut restored = enabling;
    assert!(restored.here_input_restored());
    for mut op in [
        observe, fence, attempted, waiting, geometry, enabling, restored,
    ] {
        let history = op.placement.proof.history;
        let result = op.here_shutdown(Reason::Closing).unwrap();
        assert_eq!(
            result.status(),
            if history.relocation_confirmed {
                Status::Partial
            } else {
                Status::Failed
            }
        );
        assert_eq!(result.detail().placement.history, history);
        assert_eq!(
            result.detail().placement.current,
            PlacementConfirmation::default()
        );
        assert!(!op.consume_here_desktop_attempt(now));
        assert!(!op.consume_here_correction(ORIGIN, now));
        assert!(!op.consume_here_input_enable(now));
        assert_wire(result);
    }
}

#[test]
fn pure_result_selection_still_rejects_inconsistent_confirmation() {
    let (mut op, _) = arrived(false, LayerObservation::Normal);
    assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
    let (d, c) = visible_detail(LayerObservation::Normal, true);
    for bad in [
        Detail {
            workspace: Workspace::Other,
            ..d
        },
        Detail {
            layer: LayerObservation::Above,
            ..d
        },
        Detail {
            progress: Progress {
                above_confirmed: true,
                ..d.progress
            },
            ..d
        },
    ] {
        assert!(op.placement.here_result(bad, c).is_err());
    }
    assert!(op
        .placement
        .here_result(d, Confirmation::default())
        .is_err());
}

#[test]
fn destroyed_or_withdrawn_body_is_terminal_even_without_membership_evidence() {
    for (visibility, reason) in [
        (Visibility::Destroyed, Reason::BodyDestroyed),
        (Visibility::Withdrawn, Reason::Withdrawn),
    ] {
        let (mut op, now) = arrived(false, LayerObservation::Normal);
        assert_eq!(
            op.placement.observe_here(
                Ok(layout()),
                HereObservation {
                    membership: None,
                    visibility,
                    layer: LayerObservation::Unknown
                }
            ),
            HereDecision::Invalid(reason)
        );
        assert_eq!(op.placement.proof.current, PlacementConfirmation::default());
        assert_eq!(incomplete(&op).status(), Status::Partial);
        assert!(!op.consume_here_desktop_attempt(now));
    }
}

fn recover_ready(
    body: u32,
    minimized: bool,
    layer: LayerObservation,
) -> (ActiveOperation, Instant, Detail, Confirmation) {
    let (mut op, now) = operation_for(Command::Recover);
    let sample = observation(body, minimized, layer);
    op.observe_recover_here(Ok(layout()), sample);
    if body != TARGET || minimized {
        assert!(op.placement.here_fenced());
        op.observe_recover_here(Ok(layout()), sample);
    }
    if body != TARGET {
        assert!(op.consume_here_desktop_attempt(now));
        assert!(op.placement.record_here_desktop_transport(true, true));
        assert_eq!(
            op.observe_recover_here(Ok(layout()), observation(TARGET, minimized, layer)),
            HereDecision::Destination
        );
    }
    let (mut detail, confirmed) = if minimized {
        assert!(op.placement.defer_here_geometry());
        (
            Detail {
                visibility: Visibility::Minimized,
                workspace: Workspace::Current,
                layer,
                progress: Progress {
                    placement_deferred: true,
                    ..Progress::default()
                },
                ..preflight_detail(Reason::None, Stage::Placement)
            },
            Confirmation {
                visibility: Some(ConfirmedVisibility::Minimized),
                above: false,
            },
        )
    } else {
        assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
        visible_detail(layer, true)
    };
    detail.reason = Reason::None;
    detail.placement = op.placement.proof;
    (op, now, detail, confirmed)
}

fn recover_top(
    body: u32,
    minimized: bool,
    layer: LayerObservation,
    delay: u64,
) -> (ActiveOperation, Instant) {
    let (mut op, now, detail, confirmed) = recover_ready(body, minimized, layer);
    assert!(op.handoff_recover_top(now + Duration::from_millis(delay), detail, confirmed));
    (op, now)
}

fn top_ready(op: &mut ActiveOperation, layer: LayerObservation) {
    assert_eq!(
        op.observe_recover_top(Ok(layout()), observation(TARGET, false, layer)),
        HereDecision::Destination
    );
    assert!(op.confirm_recover_top_readiness(ORIGIN, SIZE, true));
}

#[test]
fn recover_handoff_is_one_way_and_retains_identity_history_and_consumption() {
    for minimized in [false, true] {
        for body in [ORIGINAL, TARGET] {
            let (mut op, now, mut d, c) = recover_ready(body, minimized, LayerObservation::Below);
            let before = op.placement;
            let request = op.request;
            let deadline = op.deadline;
            d.progress.host_mutation_dispatched = true; // possible host dispatch, not confirmation
            assert!(op.handoff_recover_top(now + Duration::from_millis(2000), d, c));
            assert_eq!(op.request, request);
            assert_eq!(op.deadline, deadline);
            assert_eq!(op.placement.baseline, before.baseline);
            assert_eq!(op.placement.layout, before.layout);
            assert_eq!(op.placement.destination_desktop, TARGET);
            assert_eq!(op.placement.desktop_attempted, before.desktop_attempted);
            assert_eq!(op.placement.desktop_dispatched, before.desktop_dispatched);
            assert_eq!(op.placement.desktop_checked, before.desktop_checked);
            assert_eq!(op.placement.here_phase, before.here_phase);
            assert_eq!(op.placement.correction(), before.correction());
            assert_eq!(op.placement.proof.history, before.proof.history);
            assert_eq!(op.placement.proof.current, PlacementConfirmation::default());
            assert_eq!(op.detail.visibility, Visibility::Unknown);
            assert_eq!(op.detail.layer, LayerObservation::Unknown);
            assert_eq!(op.detail.workspace, Workspace::Unknown);
            assert_eq!(
                op.detail.progress,
                Progress {
                    host_mutation_dispatched: true,
                    ..Progress::default()
                }
            );
            assert_eq!(op.recover_phase, RecoverPhase::Top);
            assert!(!op.here_mutation_eligible(now));
            let after = op;
            assert!(!op.handoff_recover_top(now + Duration::from_millis(3000), d, c));
            assert_eq!(op, after);
            assert!(!op.consume_here_desktop_attempt(now));
            assert!(!op.consume_here_correction(ORIGIN, now));
            assert!(!op.consume_here_input_enable(now));
            assert!(!op.here_input_restored());
            assert_eq!(
                op.observe_recover_here(
                    Ok(layout()),
                    observation(TARGET, minimized, LayerObservation::Below)
                ),
                HereDecision::Invalid(Reason::Unverifiable)
            );
            assert_eq!(op, after);
        }
    }
    // A visible Above Here result can already contain current Above/input
    // assertions. They must be re-established independently during Top.
    let (mut op, now, mut d, mut c) = recover_ready(TARGET, false, LayerObservation::Above);
    d.progress.above_confirmed = true;
    c.above = true;
    op.detail = d;
    assert!(op.handoff_recover_top(now, d, c));
    assert_eq!(op.detail.progress, Progress::default());
    assert_eq!(op.placement.proof.current, PlacementConfirmation::default());
}

#[test]
fn recover_handoff_rejects_incomplete_stale_failed_and_foreign_here() {
    for minimized in [false, true] {
        for defect in 0..10 {
            let (mut op, now, mut d, mut c) =
                recover_ready(ORIGINAL, minimized, LayerObservation::Normal);
            match defect {
                0 => op.placement.proof.current.destination = false,
                1 => op.placement.proof.current.geometry = GeometryProof::Unknown,
                2 => op.placement.proof.current.layer_preserved = false,
                3 => op.placement.request_here_observation(),
                4 => {
                    op.placement.invalidate_here(Reason::LayoutChanged);
                }
                5 => c = Confirmation::default(),
                6 => d.reason = Reason::Unverifiable,
                7 => {
                    if minimized {
                        op.placement.proof.current.minimization_preserved = false;
                    } else {
                        d.progress.input_ready = false;
                    }
                }
                8 => {
                    if minimized {
                        d.progress.placement_deferred = false;
                    } else {
                        d.progress.not_minimized_confirmed = false;
                    }
                }
                _ => d.progress.above_confirmed = true,
            }
            let before = op;
            assert!(
                !op.handoff_recover_top(now, d, c),
                "minimized={minimized} defect={defect}"
            );
            assert_eq!(op, before);
        }
    }
    let (mut here, now) = arrived(false, LayerObservation::Normal);
    assert!(here.placement.confirm_here_geometry(ORIGIN, SIZE));
    let (mut d, c) = visible_detail(LayerObservation::Normal, true);
    d.reason = Reason::None;
    assert!(!here.handoff_recover_top(now, d, c));
}

#[test]
fn recover_here_partition_expires_exactly_and_never_renews_admission() {
    let (mut op, now) = operation_for(Command::Recover);
    assert_eq!(op.deadline, now + Duration::from_millis(7000));
    assert_eq!(op.here_cutoff(), Some(now + Duration::from_millis(3500)));
    let sample = observation(ORIGINAL, false, LayerObservation::Normal);
    op.observe_recover_here(Ok(layout()), sample);
    assert!(op.placement.here_fenced());
    op.observe_recover_here(Ok(layout()), sample);
    let cutoff = op.here_cutoff().unwrap();
    let before = op;
    for late in [cutoff, cutoff + Duration::from_millis(1), op.deadline] {
        assert!(!op.consume_here_desktop_attempt(late));
        assert_eq!(op, before);
    }
    assert!(op.consume_here_desktop_attempt(cutoff - Duration::from_nanos(1)));
    assert_eq!(op.deadline, before.deadline);
    let mut active = Some(op);
    assert_eq!(
        admit(
            &mut active,
            op.request,
            TARGET,
            Lifecycle::Ready,
            false,
            now + Duration::from_secs(9)
        ),
        Admission::Duplicate
    );
    assert_eq!(active, Some(op));
    let mut other = op.request;
    other.request_id += 1;
    assert_eq!(
        admit(&mut active, other, TARGET, Lifecycle::Ready, false, now),
        Admission::Reply(Status::Busy, Reason::Busy, Stage::Preflight)
    );
    assert_eq!(active, Some(op));
}

#[test]
fn recover_handoff_expiry_and_top_cutoffs_use_actual_handoff_once() {
    for delay in [0, 1900, 3499] {
        let (mut op, now, d, c) = recover_ready(TARGET, false, LayerObservation::Normal);
        let total = op.deadline;
        let t = now + Duration::from_millis(delay);
        assert!(op.handoff_recover_top(t, d, c));
        let v = (t + Duration::from_millis(2500)).min(total);
        assert_eq!(op.bring_top.visibility_cutoff, v);
        top_ready(&mut op, LayerObservation::Normal);
        assert!(op.recover_visibility_mutation_eligible(v - Duration::from_nanos(1)));
        assert!(!op.recover_visibility_mutation_eligible(v));
        assert!(!op.admit_recover_layer(v));
        assert!(op.admit_recover_layer(v - Duration::from_nanos(1)));
        let b = (v - Duration::from_nanos(1) + Duration::from_millis(1000)).min(total);
        assert_eq!(op.recover_layer_cutoff(), Some(b));
        assert!(op.recover_layer_mutation_eligible(b - Duration::from_nanos(1)));
        assert!(!op.recover_layer_mutation_eligible(b));
        assert!(!op.recover_layer_mutation_eligible(total));
        assert!(!op.recover_visibility_mutation_eligible(t)); // layer owns progression now
        for time in [t, v, b, total] {
            assert!(!op.admit_recover_layer(time));
            assert!(!op.handoff_recover_top(time, d, c));
            top_ready(&mut op, LayerObservation::Normal);
            assert_eq!(op.bring_top.visibility_cutoff, v);
            assert_eq!(op.recover_layer_cutoff(), Some(b));
            assert_eq!(op.deadline, total);
        }
    }
    let (mut op, _, d, c) = recover_ready(TARGET, false, LayerObservation::Above);
    assert!(!op.handoff_recover_top(op.here_cutoff().unwrap(), d, c));
}

#[test]
fn recover_here_and_top_correction_require_full_interval() {
    let (mut here, now, _, _) = recover_ready(TARGET, false, LayerObservation::Normal);
    assert!(here.placement.here_fenced());
    here.observe_recover_here(
        Ok(layout()),
        observation(TARGET, false, LayerObservation::Normal),
    );
    assert!(here.placement.begin_here_geometry());
    let h = here.here_cutoff().unwrap();
    assert!(!here.consume_here_correction(ORIGIN, h - Duration::from_millis(149)));
    assert!(here.consume_here_correction(ORIGIN, h - Duration::from_millis(150)));
    assert_eq!(here.placement.correction(), Some((ORIGIN, h)));
    assert_eq!(here.deadline, now + Duration::from_millis(7000));
    let (mut top, _) = recover_top(TARGET, true, LayerObservation::Normal, 2000);
    top_ready(&mut top, LayerObservation::Normal);
    let v = top.bring_top.visibility_cutoff;
    for time in [v - Duration::from_millis(149), v, top.deadline] {
        assert!(!top.consume_recover_top_correction(ORIGIN, time));
        assert!(top.placement.correction().is_none());
    }
    assert!(top.consume_recover_top_correction(ORIGIN, v - Duration::from_millis(150)));
    top_ready(&mut top, LayerObservation::Normal);
    assert!(!top.consume_recover_top_correction(ORIGIN, v - Duration::from_millis(150)));
}

#[test]
fn recover_here_consumed_correction_survives_success_uncertainty_and_top_initialization() {
    for confirmed in [false, true] {
        let (mut op, now) = operation_for(Command::Recover);
        let sample = observation(TARGET, false, LayerObservation::Normal);
        op.observe_recover_here(Ok(layout()), sample);
        assert!(op.placement.here_fenced());
        op.observe_recover_here(Ok(layout()), sample);
        assert!(op.placement.begin_here_geometry());
        assert!(op.consume_here_correction(ORIGIN, now));
        assert!(!op.consume_here_correction(ORIGIN, now));
        if !confirmed {
            // An uncertain/failed send stays consumed even if an external
            // actor later supplies the required actual geometry.
            op.detail.progress.host_mutation_dispatched = true;
        }
        op.observe_recover_here(Ok(layout()), sample);
        assert!(op.placement.confirm_here_geometry(ORIGIN, SIZE));
        let (mut d, c) = visible_detail(LayerObservation::Normal, true);
        d.reason = Reason::None;
        assert!(op.handoff_recover_top(now + Duration::from_millis(200), d, c));
        assert!(op.show.correction_sent);
        let consumed = op.placement.correction();
        top_ready(&mut op, LayerObservation::Normal);
        assert!(op.confirm_recover_top_readiness(Point::new(0, 50), SIZE, true));
        // Even replacing the Show projection cannot replenish authority.
        op.show = ShowProgress::default();
        assert!(!op.consume_recover_top_correction(ORIGIN, now + Duration::from_millis(300)));
        assert_eq!(op.placement.correction(), consumed);
    }
}

#[test]
fn recover_minimized_here_defers_unused_correction_to_top_once() {
    let (mut op, now) = recover_top(ORIGINAL, true, LayerObservation::Below, 1000);
    assert!(op.placement.correction().is_none());
    assert!(!op.consume_recover_top_correction(ORIGIN, now + Duration::from_millis(1000))); // no fresh evidence
    top_ready(&mut op, LayerObservation::Below);
    assert!(op.consume_recover_top_correction(ORIGIN, now + Duration::from_millis(1000)));
    assert_eq!(op.placement.proof.current, PlacementConfirmation::default());
    assert_eq!(
        op.observe_recover_top(
            Ok(layout()),
            observation(TARGET, false, LayerObservation::Below)
        ),
        HereDecision::Destination
    );
    assert!(!op.confirm_recover_top_readiness(Point::new(0, 50), SIZE, true));
    top_ready(&mut op, LayerObservation::Below);
    assert!(!op.consume_recover_top_correction(ORIGIN, now + Duration::from_millis(1200)));
}

#[test]
fn recover_top_rebuilds_current_without_preservation_or_dispatch_inference() {
    let (mut op, now) = recover_top(ORIGINAL, true, LayerObservation::Below, 1000);
    assert_eq!(
        op.placement.proof.history,
        RecoveryHistory {
            relocation_confirmed: true,
            restoration_confirmed: false
        }
    );
    op.detail.progress.host_mutation_dispatched = true;
    op.show.phase = ShowPhase::RestoreRequested;
    op.bring_top.layer_dispatched = true;
    assert!(!op.placement.proof.history.restoration_confirmed);
    assert_eq!(
        op.recover_result(Reason::ConfirmationTimeout, Stage::Show)
            .unwrap()
            .status(),
        Status::Partial
    );
    top_ready(&mut op, LayerObservation::Above);
    assert!(op.placement.proof.history.restoration_confirmed);
    assert!(op.bring_top.minimized_at_start && op.bring_top.restoration_confirmed);
    assert!(!op.placement.proof.current.minimization_preserved);
    assert!(!op.placement.proof.current.layer_preserved);
    let result = op.recover_result(Reason::None, Stage::Layer).unwrap();
    assert_eq!(result.status(), Status::Success);
    assert_wire(result);
    // Expiry may recognize this completed postcondition, never mutate.
    assert!(!op.recover_visibility_mutation_eligible(op.deadline));
    assert!(!op.consume_recover_top_correction(ORIGIN, op.deadline));
    assert!(!op.admit_recover_layer(op.deadline));
    assert_eq!(op.deadline, now + Duration::from_millis(7000));
    op.observe_recover_top(
        Ok(layout()),
        HereObservation {
            membership: None,
            visibility: Visibility::Transitional,
            layer: LayerObservation::Unknown,
        },
    );
    assert!(!op.detail.progress.above_confirmed);
    assert!(!op.detail.progress.input_ready);
    assert_eq!(
        op.recover_result(Reason::ConfirmationTimeout, Stage::Layer)
            .unwrap()
            .status(),
        Status::Partial
    );
}

#[test]
fn recover_restoration_history_precedes_layout_layer_and_here_preservation_failures() {
    for failure in [Reason::LayoutChanged, Reason::LayerChanged] {
        let (mut op, now, d, c) = recover_ready(ORIGINAL, true, LayerObservation::Below);
        assert!(op.handoff_recover_top(now, d, c));
        let (layout, sample) = if failure == Reason::LayoutChanged {
            (
                Err(failure),
                observation(TARGET, false, LayerObservation::Above),
            )
        } else {
            (
                Ok(layout()),
                observation(TARGET, false, LayerObservation::Conflict),
            )
        };
        assert_eq!(
            op.observe_recover_top(layout, sample),
            HereDecision::Invalid(failure)
        );
        assert!(op.placement.proof.history.restoration_confirmed);
        assert_eq!(op.placement.proof.current, PlacementConfirmation::default());
        assert_eq!(
            op.recover_shutdown(Reason::Closing)
                .unwrap()
                .detail()
                .reason,
            failure
        );
    }
    let (mut here, now, d, c) = recover_ready(TARGET, true, LayerObservation::Normal);
    assert_eq!(
        here.observe_recover_here(
            Ok(layout()),
            observation(TARGET, false, LayerObservation::Normal)
        ),
        HereDecision::Invalid(Reason::Unverifiable)
    );
    assert!(here.placement.proof.history.restoration_confirmed);
    assert!(!here.handoff_recover_top(now, d, c));
    assert_eq!(
        here.recover_shutdown(Reason::Closing).unwrap().status(),
        Status::Partial
    );
    // Independently supported new desktop membership still proves the
    // visibility transition even when the frozen desktop count has drifted.
    let (mut op, _) = recover_top(TARGET, true, LayerObservation::Normal, 0);
    let sample = observation(layout().desktop_count, false, LayerObservation::Normal);
    assert_eq!(
        op.observe_recover_top(Err(Reason::LayoutChanged), sample),
        HereDecision::Invalid(Reason::LayoutChanged)
    );
    assert!(op.placement.proof.history.restoration_confirmed);
}

#[test]
fn recover_incoherent_missing_or_transitional_visibility_never_proves_restoration() {
    for sample in [
        HereObservation {
            membership: None,
            visibility: Visibility::NotMinimizedHere,
            layer: LayerObservation::Above,
        },
        HereObservation {
            membership: Some((TARGET, TARGET)),
            visibility: Visibility::NotMinimizedElsewhere,
            layer: LayerObservation::Above,
        },
        HereObservation {
            membership: Some((TARGET, THIRD)),
            visibility: Visibility::NotMinimizedHere,
            layer: LayerObservation::Above,
        },
        HereObservation {
            membership: Some((NO_DESTINATION, TARGET)),
            visibility: Visibility::NotMinimizedElsewhere,
            layer: LayerObservation::Above,
        },
        HereObservation {
            membership: Some((TARGET, TARGET)),
            visibility: Visibility::Transitional,
            layer: LayerObservation::Above,
        },
        observation(TARGET, true, LayerObservation::Above),
    ] {
        let (mut op, _) = recover_top(TARGET, true, LayerObservation::Normal, 0);
        op.observe_recover_top(Ok(layout()), sample);
        assert!(
            !op.placement.proof.history.restoration_confirmed,
            "{sample:?}"
        );
    }
}

#[test]
fn recover_results_cover_all_history_combinations_current_success_and_shutdown() {
    for body in [TARGET, ORIGINAL] {
        for minimized in [false, true] {
            let (mut op, _) = recover_top(body, minimized, LayerObservation::Above, 1000);
            top_ready(&mut op, LayerObservation::Above);
            let history = RecoveryHistory {
                relocation_confirmed: body == ORIGINAL,
                restoration_confirmed: minimized,
            };
            assert_eq!(op.placement.proof.history, history);
            let success = op.recover_result(Reason::None, Stage::Layer).unwrap();
            assert_eq!(success.status(), Status::Success);
            assert_wire(success);
            assert!(op.confirm_recover_top_readiness(ORIGIN, SIZE, false));
            assert_eq!(
                op.recover_result(Reason::ConfirmationTimeout, Stage::Layer)
                    .unwrap()
                    .status(),
                if body == ORIGINAL || minimized {
                    Status::Partial
                } else {
                    Status::Failed
                }
            );
            top_ready(&mut op, LayerObservation::Above);
            assert!(op.recover_result(Reason::None, Stage::Show).is_err());
            for reason in [Reason::Closing, Reason::X11Failure, Reason::OwnershipLost] {
                let mut failed = op;
                failed.bring_top.layer_dispatched = true;
                let result = failed.here_shutdown(reason).unwrap();
                assert_eq!(
                    result.status(),
                    if body == ORIGINAL || minimized {
                        Status::Partial
                    } else {
                        Status::Failed
                    }
                );
                assert_eq!(result.detail().reason, reason);
                assert_eq!(result.detail().stage, Stage::Shutdown);
                assert_eq!(result.detail().placement.history, history);
                assert_eq!(
                    result.detail().placement.current,
                    PlacementConfirmation::default()
                );
                assert_eq!(
                    result.detail().progress,
                    Progress {
                        host_mutation_dispatched: true,
                        ..Progress::default()
                    }
                );
                assert_wire(result);
                // A late coherent history read cannot rebuild current success.
                assert!(matches!(
                    failed.observe_recover_top(
                        Ok(layout()),
                        observation(TARGET, false, LayerObservation::Above)
                    ),
                    HereDecision::Invalid(_)
                ));
                assert!(!failed.confirm_recover_top_readiness(ORIGIN, SIZE, true));
                assert_eq!(
                    failed
                        .recover_shutdown(Reason::Unverifiable)
                        .unwrap()
                        .detail()
                        .reason,
                    reason
                );
            }
        }
    }
}

#[test]
fn recover_minimized_here_is_incomplete_and_local_visible_correction_is_not_history() {
    let (op, _, _, _) = recover_ready(ORIGINAL, true, LayerObservation::Above);
    assert_eq!(
        op.recover_result(Reason::ConfirmationTimeout, Stage::Placement)
            .unwrap()
            .status(),
        Status::Partial
    );
    assert!(op.recover_result(Reason::None, Stage::Layer).is_err());
    let (mut local, now) = recover_top(TARGET, false, LayerObservation::Normal, 0);
    top_ready(&mut local, LayerObservation::Normal);
    assert!(local.consume_recover_top_correction(ORIGIN, now));
    assert_eq!(local.placement.proof.history, RecoveryHistory::default());
    assert_eq!(
        local.recover_shutdown(Reason::X11Failure).unwrap().status(),
        Status::Failed
    );
}

#[test]
fn recover_fatal_history_read_can_add_restoration_without_current_success() {
    let (mut op, _) = recover_top(ORIGINAL, true, LayerObservation::Normal, 1000);
    assert_eq!(
        op.recover_shutdown(Reason::X11Failure).unwrap().status(),
        Status::Partial
    );
    op.observe_recover_top(
        Ok(layout()),
        observation(TARGET, false, LayerObservation::Above),
    );
    let result = op.recover_result(Reason::Closing, Stage::Shutdown).unwrap();
    assert_eq!(result.detail().reason, Reason::X11Failure);
    assert_eq!(
        result.detail().placement.history,
        RecoveryHistory {
            relocation_confirmed: true,
            restoration_confirmed: true
        }
    );
    assert_eq!(
        result.detail().placement.current,
        PlacementConfirmation::default()
    );
    assert_wire(result);
    let (mut here, _, _, _) = recover_ready(TARGET, true, LayerObservation::Normal);
    here.recover_shutdown(Reason::X11Failure).unwrap();
    here.observe_recover_here(
        Ok(layout()),
        observation(TARGET, false, LayerObservation::Above),
    );
    assert_eq!(here.stage, Stage::Shutdown);
    let late = here
        .recover_result(Reason::Closing, Stage::Shutdown)
        .unwrap();
    assert_eq!(late.status(), Status::Partial);
    assert_eq!(late.detail().reason, Reason::X11Failure);
    assert_wire(late);
}

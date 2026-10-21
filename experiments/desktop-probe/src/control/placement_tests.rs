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
        baseline: Some(PlacementBaseline {
            desktop: 2,
            minimized: true,
            layer: LayerObservation::Normal,
        }),
        ..PlacementProgress::new(7)
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
fn ordinary_bring_here_success_requires_current_geometry_input_destination_and_preserved_layer() {
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
fn minimized_here_success_can_report_admissible_deferred_or_verified_geometry_with_input_disabled()
{
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
            assert!(result(Command::Recover, Status::Success, bad, confirmation, proof).is_err());
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
                        Status::Starting => reason == Reason::Starting && stage == Stage::Preflight,
                        Status::Closing => reason == Reason::Closing && stage == Stage::Shutdown,
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
fn complete_already_local_here_without_history_accepts_success_and_rejects_failed() {
    let (d, c, mut p) = complete(Command::BringHere);
    p.baseline.as_mut().unwrap().desktop = p.destination_desktop;
    assert_eq!(p.proof.history, RecoveryHistory::default());
    assert!(result(Command::BringHere, Status::Success, d, c, p).is_ok());
    let failed = Detail {
        reason: Reason::ConfirmationTimeout,
        ..d
    };
    assert_eq!(
        result(Command::BringHere, Status::Failed, failed, c, p),
        Err(ResultError::InconsistentStatus)
    );
}

#[test]
fn complete_recover_with_relocation_history_accepts_success_and_rejects_partial() {
    let (mut d, c, mut p) = complete(Command::Recover);
    p.baseline.as_mut().unwrap().minimized = false;
    p.proof.history.relocation_confirmed = true;
    d.placement = p.proof;
    assert!(result(Command::Recover, Status::Success, d, c, p).is_ok());
    let partial = Detail {
        reason: Reason::ConfirmationTimeout,
        ..d
    };
    assert_eq!(
        result(Command::Recover, Status::Partial, partial, c, p),
        Err(ResultError::InvalidPartial)
    );
}

#[test]
fn complete_recover_with_restoration_history_accepts_success_and_rejects_partial() {
    let (mut d, c, mut p) = complete(Command::Recover);
    p.baseline.as_mut().unwrap().desktop = p.destination_desktop;
    p.proof.history.restoration_confirmed = true;
    d.placement = p.proof;
    assert!(result(Command::Recover, Status::Success, d, c, p).is_ok());
    let partial = Detail {
        reason: Reason::ConfirmationTimeout,
        ..d
    };
    assert_eq!(
        result(Command::Recover, Status::Partial, partial, c, p),
        Err(ResultError::InvalidPartial)
    );
}

#[test]
fn incomplete_recover_with_relocation_history_accepts_partial_and_rejects_failed() {
    let mut p = progress();
    p.baseline.as_mut().unwrap().minimized = false;
    p.proof.history.relocation_confirmed = true;
    let d = Detail {
        placement: p.proof,
        ..preflight_detail(Reason::Unverifiable, Stage::Layer)
    };
    let c = Confirmation::default();
    assert!(result(Command::Recover, Status::Partial, d, c, p).is_ok());
    assert_eq!(
        result(Command::Recover, Status::Failed, d, c, p),
        Err(ResultError::InconsistentStatus)
    );
}

#[test]
fn incomplete_recover_with_restoration_history_still_accepts_partial() {
    let mut p = progress();
    p.baseline.as_mut().unwrap().desktop = p.destination_desktop;
    p.proof.history.restoration_confirmed = true;
    let d = Detail {
        placement: p.proof,
        ..preflight_detail(Reason::Unverifiable, Stage::Layer)
    };
    assert!(result(
        Command::Recover,
        Status::Partial,
        d,
        Confirmation::default(),
        p
    )
    .is_ok());
}

#[test]
fn incomplete_here_without_history_accepts_failed_and_rejects_partial() {
    let mut p = progress();
    p.baseline.as_mut().unwrap().desktop = p.destination_desktop;
    p.baseline.as_mut().unwrap().minimized = false;
    let d = preflight_detail(Reason::Unverifiable, Stage::Placement);
    let c = Confirmation::default();
    assert!(result(Command::BringHere, Status::Failed, d, c, p).is_ok());
    assert_eq!(
        result(Command::BringHere, Status::Partial, d, c, p),
        Err(ResultError::InvalidPartial)
    );
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
                            || (!minimized && destination && geometry == GeometryProof::Verified))
                        && (command != Command::BringHere || !restored);
                    let complete = destination
                        && if command == Command::Recover {
                            !minimized && geometry == GeometryProof::Verified && input
                        } else {
                            layer
                                && if minimized {
                                    preserved_min && geometry != GeometryProof::Unknown && !input
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
                                Status::Partial => !complete && meaningful,
                                Status::Failed => !complete && !meaningful,
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

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
    assert!(OperationResult::bring_top(Status::Busy, d, Confirmation::default(), &proof).is_err());
    assert!(
        OperationResult::bring_top(Status::Failed, d, Confirmation::default(), &proof).is_err()
    );
    proof.minimized_at_start = false;
    proof.restoration_confirmed = false;
    d.progress.host_mutation_dispatched = true;
    assert!(OperationResult::bring_top(Status::Busy, d, Confirmation::default(), &proof).is_err());
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
                                    Some(ConfirmedVisibility::Minimized) => Visibility::Minimized,
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
                                                assert_eq!(
                                                    vis,
                                                    Some(
                                                        ConfirmedVisibility::NotMinimizedElsewhere
                                                    )
                                                );
                                                assert_eq!(workspace, Workspace::Other);
                                                assert!(!p.input_ready);
                                            }
                                            match status {
                                                Status::Success => {
                                                    assert_eq!(reason, Reason::None);
                                                    assert!(above && p.not_minimized_confirmed);
                                                    assert!(p.input_ready || p.placement_deferred);
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

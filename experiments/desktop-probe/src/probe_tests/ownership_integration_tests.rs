use super::*;
use std::cell::{Cell, RefCell};

#[test]
fn ownership_then_delay_without_body_then_initialization_and_duration() {
    let start = Instant::now();
    let mut state = (Vec::new(), start, false);
    let outcome = owner_startup_with(
        &mut state,
        |state| {
            state.0.push("acquire");
            Ok(Acquisition::Owned)
        },
        |state| {
            assert_eq!(state.0, ["acquire"]);
            assert!(!state.2, "no body during delay");
            state.0.push("delay");
            state.1 += Duration::from_secs(20);
            Ok(())
        },
        |state| {
            assert_eq!(state.0, ["acquire", "delay"]);
            state.2 = true;
            state.0.push("body/renderer");
            state.1 += Duration::from_secs(2); // ordinary initialization precedes run_probe
            let deadline = probe_duration_deadline(state.1, Some(5))?.unwrap();
            assert_eq!(deadline, start + Duration::from_secs(27));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(outcome, Acquisition::Owned);
    assert!(state.2);
}

#[test]
fn duplicate_and_acquisition_failure_cannot_delay_or_create_body() {
    assert_eq!(
        owner_startup_with(
            &mut (),
            |_| Ok(Acquisition::Duplicate),
            |_| panic!("duplicate delay"),
            |_| panic!("duplicate body")
        )
        .unwrap(),
        Acquisition::Duplicate
    );
    let error = owner_startup_with(
        &mut (),
        |_| Err("ungrab failed".into()),
        |_| panic!("failed acquisition delay"),
        |_| panic!("failed acquisition body"),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "ungrab failed");
}

#[test]
fn loss_during_delay_blocks_body_and_reaches_cleanup() {
    let calls = RefCell::new(Vec::new());
    let result = owner_startup_with(
        &mut (),
        |_| Ok(Acquisition::Owned),
        |_| Err("selection lost during delay".into()),
        |_| panic!("body after loss"),
    );
    let error = finish_probe_with(result.map(|_| ()), || {
        owner_cleanup_with(
            &mut (),
            |_| {
                calls.borrow_mut().push("closing check");
                Ok(())
            },
            || {
                calls.borrow_mut().push("empty body cleanup");
                Ok(())
            },
            |_| {
                calls.borrow_mut().push("control destroy");
                Ok(())
            },
            || {
                calls.borrow_mut().push("flush");
                Ok(())
            },
        )
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "selection lost during delay");
    assert_eq!(
        *calls.borrow(),
        [
            "closing check",
            "empty body cleanup",
            "control destroy",
            "flush"
        ]
    );
}

#[test]
fn closing_precedes_teardown_and_control_is_last_even_after_failures() {
    for original_failure in [false, true] {
        let calls = RefCell::new(Vec::new());
        let result = if original_failure {
            Err("original runtime failure".into())
        } else {
            Ok(())
        };
        let error = finish_probe_with(result, || {
            owner_cleanup_with(
                &mut (),
                |_| {
                    calls.borrow_mut().push("closing");
                    Err("closing failure".into())
                },
                || {
                    x11::resource::cleanup_all([
                        Box::new(|| {
                            calls.borrow_mut().push("input/popup");
                            Err("release failure".into())
                        }) as Box<dyn FnOnce() -> Result<(), HostError>>,
                        Box::new(|| {
                            calls.borrow_mut().push("renderer");
                            Err("renderer failure".into())
                        }),
                        Box::new(|| {
                            calls.borrow_mut().push("body");
                            Ok(())
                        }),
                    ])
                },
                |_| {
                    calls.borrow_mut().push("control");
                    Err("destroy failure".into())
                },
                || {
                    calls.borrow_mut().push("flush");
                    Ok(())
                },
            )
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            if original_failure {
                "original runtime failure"
            } else {
                "closing failure"
            }
        );
        assert_eq!(
            *calls.borrow(),
            [
                "closing",
                "input/popup",
                "renderer",
                "body",
                "control",
                "flush"
            ]
        );
    }
}

#[test]
fn runtime_control_loss_remains_primary_through_ordered_cleanup() {
    for cleanup_fails in [false, true] {
        let calls = RefCell::new(Vec::new());
        // Both the event batch and one-event lookahead use this exit classification
        // after the native ownership check has verified actual endpoint loss.
        let result = probe_termination_result(ProbeTermination::ControlEndpointLost);
        let error = finish_probe_with(result, || {
            owner_cleanup_with(
                &mut (),
                |_| {
                    calls.borrow_mut().push("closing ownership check");
                    Ok(()) // already lost: native closing skips descriptor publication
                },
                || {
                    calls.borrow_mut().push("input/popup/renderer/body");
                    if cleanup_fails {
                        Err("body cleanup failed".into())
                    } else {
                        Ok(())
                    }
                },
                |_| {
                    calls.borrow_mut().push("control cleanup");
                    if cleanup_fails {
                        Err("control cleanup failed".into())
                    } else {
                        Ok(())
                    }
                },
                || {
                    calls.borrow_mut().push("flush");
                    Ok(())
                },
            )
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Control endpoint lost during running probe"
        );
        assert_eq!(
            *calls.borrow(),
            [
                "closing ownership check",
                "input/popup/renderer/body",
                "control cleanup",
                "flush"
            ]
        );
    }
}

#[test]
fn duration_and_wm_close_remain_normal_completion_after_cleanup() {
    for reason in [ProbeTermination::DurationExpired, ProbeTermination::WmClose] {
        let cleaned = Cell::new(false);
        finish_probe_with(probe_termination_result(reason), || {
            cleaned.set(true);
            Ok(())
        })
        .unwrap();
        assert!(cleaned.get());
    }
}

#[test]
fn ready_publication_follows_existing_map_and_correction_truth() {
    let mut runtime = ProbeRuntime::new();
    let mut window = tests::test_window();
    let bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50);
    let actual = crate::geometry::Rect::new(100, 100, 160, 160);
    let writes = Cell::new(0);
    let publish = || {
        writes.set(writes.get() + 1);
        Ok(())
    };
    publish_ready_if(runtime.body, publish).unwrap();
    assert_eq!(writes.get(), 0);
    assert_eq!(
        probe_deadline(&runtime, None),
        None,
        "map wait has no timeout or idle polling"
    );
    runtime.body = BodyAvailability::Validating; // native map/workspace gate in run_probe
    runtime
        .placement_with(&mut window, actual, &bounds, |_, target| {
            assert_eq!(target, Point::new(50, 50));
            Ok(())
        })
        .unwrap();
    publish_ready_if(runtime.body, publish).unwrap();
    assert_eq!(writes.get(), 0);
    let deadline = runtime.correction.unwrap().1;
    runtime
        .check_correction(deadline, Point::new(50, 50), &bounds)
        .unwrap();
    publish_ready_if(runtime.body, publish).unwrap();
    assert_eq!(writes.get(), 1);
    assert!(publish_ready_if(runtime.body, || Err("descriptor write failed".into())).is_err());
    for state in [
        BodyAvailability::AwaitingMap,
        BodyAvailability::Unavailable,
        BodyAvailability::Validating,
        BodyAvailability::Destroyed,
    ] {
        publish_ready_if(state, || panic!("unready publication")).unwrap();
    }
}

#[test]
fn duration_is_optional_and_does_not_add_map_timeout() {
    let runtime = ProbeRuntime::new();
    assert_eq!(probe_duration_deadline(Instant::now(), None).unwrap(), None);
    assert_eq!(probe_deadline(&runtime, None), None);
    let deadline = probe_duration_deadline(Instant::now(), Some(30)).unwrap();
    assert_eq!(probe_deadline(&runtime, deadline), deadline);
    assert!(probe_duration_deadline(Instant::now(), Some(u64::MAX)).is_err());
}

use super::*;
use crate::geometry::{MenuHit, ValidOriginBounds};
use crate::interaction::MenuItem;
use crate::layer::{Flags, Phase};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use x11rb::protocol::xproto::{Property, PropertyNotifyEvent};

const SUPPORT: LayerSupport = LayerSupport {
    state: true,
    above: true,
    below: true,
};

#[test]
fn structured_layer_outcomes_distinguish_already_waiting_and_fresh_confirmation() {
    let now = Instant::now();
    let mut layers = LayerController::default();
    assert_eq!(
        service_layer_with(
            &mut layers,
            Some(Layer::Above),
            || Ok(Ok(ObservedLayer::Above)),
            || panic!("already matches"),
            |_| panic!("already matches"),
            || now,
        )
        .unwrap(),
        LayerOutcome::AlreadyMatches
    );
    let sends = Cell::new(0);
    assert_eq!(
        service_layer_with(
            &mut layers,
            Some(Layer::Above),
            || Ok(Ok(ObservedLayer::Normal)),
            || Ok(Ok(SUPPORT)),
            |mutation| {
                assert_eq!(mutation, Mutation::AddAbove);
                sends.set(sends.get() + 1);
                Ok(())
            },
            || now,
        )
        .unwrap(),
        LayerOutcome::Waiting
    );
    assert_eq!(sends.get(), 1);
    let pending = layers.pending();
    assert_eq!(
        service_layer_with(
            &mut layers,
            Some(Layer::Below),
            || panic!("busy must not read"),
            || panic!("busy"),
            |_| panic!("busy"),
            || now,
        )
        .unwrap(),
        LayerOutcome::Busy
    );
    assert_eq!(layers.pending(), pending);
    assert_eq!(
        service_layer_with(
            &mut layers,
            None,
            || Ok(Ok(ObservedLayer::Above)),
            || panic!("confirmation"),
            |_| panic!("confirmation"),
            || now,
        )
        .unwrap(),
        LayerOutcome::Confirmed
    );
    assert!(layers.pending().is_none());
}

#[test]
fn structured_support_and_property_failures_are_distinct_and_stop_mutation() {
    let now = Instant::now();
    for (support, expected) in [
        (
            Ok(LayerSupport {
                above: false,
                ..SUPPORT
            }),
            LayerOutcome::Unsupported,
        ),
        (Err("malformed support"), LayerOutcome::Unverifiable),
    ] {
        let mut layers = LayerController::default();
        assert_eq!(
            service_layer_with(
                &mut layers,
                Some(Layer::Above),
                || Ok(Ok(ObservedLayer::Below)),
                || Ok(support),
                |_| panic!("no removal before complete support"),
                || now,
            )
            .unwrap(),
            expected
        );
        assert!(layers.pending().is_none());
    }
    let mut layers = LayerController::default();
    pending_above(&mut layers, now);
    assert_eq!(
        service_layer_with(
            &mut layers,
            None,
            || Ok(Err("malformed state")),
            || panic!("unverifiable"),
            |_| panic!("unverifiable"),
            || now,
        )
        .unwrap(),
        LayerOutcome::Unverifiable
    );
    assert!(layers.pending().is_none());
}

#[test]
fn contradictory_layer_flags_never_produce_structured_above_confirmation() {
    let now = Instant::now();
    let mut layers = LayerController::default();
    service_layer_with(
        &mut layers,
        Some(Layer::Above),
        || Ok(Ok(ObservedLayer::Normal)),
        || Ok(Ok(SUPPORT)),
        |_| Ok(()),
        || now,
    )
    .unwrap();
    let deadline = layers.deadline().unwrap();
    for (time, expected) in [
        (now, LayerOutcome::Waiting),
        (deadline, LayerOutcome::TimedOut(Phase::AwaitFinal)),
    ] {
        assert_eq!(
            service_layer_with(
                &mut layers,
                None,
                || Ok(Ok(ObservedLayer::Conflict)),
                || panic!("no retry or conflict repair after dispatch"),
                |_| panic!("no retry"),
                || time,
            )
            .unwrap(),
            expected
        );
    }
}

#[test]
fn structured_timeout_and_external_interruption_retain_the_controller_phase() {
    let now = Instant::now();
    for removal_done in [false, true] {
        let mut layers = LayerController::default();
        pending_above(&mut layers, now);
        if removal_done {
            service_layer_with(
                &mut layers,
                None,
                || Ok(Ok(ObservedLayer::Normal)),
                || Ok(Ok(SUPPORT)),
                |_| Ok(()),
                || now,
            )
            .unwrap();
        }
        let phase = layers.pending().unwrap().phase;
        let deadline = layers.deadline().unwrap();
        assert_eq!(
            service_layer_with(
                &mut layers,
                None,
                || Ok(Ok(ObservedLayer::Normal)),
                || panic!("deadline"),
                |_| panic!("deadline"),
                || deadline,
            )
            .unwrap(),
            LayerOutcome::TimedOut(phase)
        );
        assert!(layers.pending().is_none());
        pending_above(&mut layers, now);
        assert_eq!(
            abort_layer(&mut layers, "external lifecycle interruption"),
            Some(LayerOutcome::Interrupted(Phase::AwaitRemoval))
        );
        assert!(layers.pending().is_none());
        assert_eq!(abort_layer(&mut layers, "already aborted"), None);
    }
}

#[test]
fn bounded_service_expiry_never_sends_and_final_fresh_read_can_confirm() {
    let now = Instant::now();
    let deadline = now + Duration::from_millis(200);
    for achieved in [false, true] {
        let mut layers = LayerController::default();
        let clock = Cell::new(now);
        let reads = Cell::new(0);
        assert_eq!(
            service_layer_bounded_with(
                &mut layers,
                Some(Layer::Above),
                Some(deadline),
                || {
                    reads.set(reads.get() + 1);
                    Ok(Ok(if achieved && reads.get() == 2 {
                        ObservedLayer::Above
                    } else {
                        ObservedLayer::Below
                    }))
                },
                || {
                    clock.set(deadline);
                    Ok(Ok(SUPPORT))
                },
                |_| panic!("support read exhausted the outer budget"),
                || clock.get(),
            )
            .unwrap(),
            if achieved {
                LayerOutcome::Confirmed
            } else {
                LayerOutcome::TimedOut(Phase::AwaitRemoval)
            }
        );
        assert_eq!(reads.get(), 2);
        assert!(layers.pending().is_none());
    }
    let mut layers = LayerController::default();
    assert_eq!(
        service_layer_bounded_with(
            &mut layers,
            Some(Layer::Above),
            Some(now),
            || Ok(Ok(ObservedLayer::Normal)),
            || panic!("expired entry"),
            |_| panic!("expired entry"),
            || now,
        )
        .unwrap(),
        LayerOutcome::TimedOut(Phase::AwaitFinal)
    );
}

#[test]
fn fresh_above_after_removal_skips_addition_and_checked_error_cannot_retry() {
    let now = Instant::now();
    let mut layers = LayerController::default();
    let mut reads = VecDeque::from([ObservedLayer::Below, ObservedLayer::Above]);
    let sends = Cell::new(0);
    assert_eq!(
        service_layer_with(
            &mut layers,
            Some(Layer::Above),
            || Ok(Ok(reads.pop_front().unwrap())),
            || Ok(Ok(SUPPORT)),
            |mutation| {
                assert!(matches!(mutation, Mutation::Remove(_)));
                sends.set(sends.get() + 1);
                Ok(())
            },
            || now,
        )
        .unwrap(),
        LayerOutcome::Confirmed
    );
    assert_eq!(sends.get(), 1);
    let result = service_layer_with(
        &mut layers,
        Some(Layer::Above),
        || Ok(Ok(ObservedLayer::Below)),
        || Ok(Ok(SUPPORT)),
        |_| {
            sends.set(sends.get() + 1);
            Err("checked native failure".into())
        },
        || now,
    );
    assert_eq!(result.unwrap_err().to_string(), "checked native failure");
    assert!(layers.pending().is_none());
    assert_eq!(
        service_layer_with(
            &mut layers,
            None,
            || Ok(Ok(ObservedLayer::Normal)),
            || panic!("no second request after uncertainty"),
            |_| panic!("no second request"),
            || now,
        )
        .unwrap(),
        LayerOutcome::Waiting
    );
    assert_eq!(sends.get(), 2);
}

fn captured_menu() -> ProbeRuntime {
    let mut runtime = ProbeRuntime::new();
    runtime.body = BodyAvailability::Ready;
    runtime
        .acquire_menu_with(MenuPopup::test_popup(10, 1), Point::new(0, 0), || {
            Ok((GrabStatus::SUCCESS, 100))
        })
        .unwrap();
    runtime
}

pub(crate) fn pending_above(layers: &mut LayerController, now: Instant) {
    let reads = Cell::new(0);
    service_layer_with(
        layers,
        Some(Layer::Above),
        || {
            reads.set(reads.get() + 1);
            Ok(Ok(ObservedLayer::Below))
        },
        || Ok(Ok(SUPPORT)),
        |mutation| {
            assert_eq!(
                mutation,
                Mutation::Remove(Flags {
                    above: false,
                    below: true
                })
            );
            Ok(())
        },
        || now,
    )
    .unwrap();
    assert_eq!(reads.get(), 2); // preflight snapshot and fresh read after checked removal
    assert_eq!(layers.pending().unwrap().phase, Phase::AwaitRemoval);
}

/// A fresh selecting gesture, only after the caller proves native input ready.
/// Acquisition and row matching use the existing runtime/interaction methods.
pub(crate) fn select_layer(runtime: &mut ProbeRuntime, target: Layer) -> MenuOutcome {
    assert!(runtime.accepts_input());
    assert!(runtime.interaction.is_idle());
    assert!(runtime.menu.is_none());
    assert!(!runtime.pointer.is_grabbed());
    runtime.menu_generation += 1;
    runtime
        .acquire_menu_with(
            MenuPopup::test_popup(10, runtime.menu_generation),
            Point::new(0, 0),
            || Ok((GrabStatus::SUCCESS, 3000)),
        )
        .unwrap();
    let row = match target {
        Layer::Above => 0,
        Layer::Normal => 1,
        Layer::Below => 2,
    };
    runtime.interaction.menu_press(1, MenuHit::Row(row), 300, 0);
    let HostAction::CloseMenu { outcome, .. } =
        runtime
            .interaction
            .menu_release(1, MenuHit::Row(row), 301, 0)
    else {
        panic!("fresh layer selection missing")
    };
    assert_eq!(outcome, MenuOutcome::SetLayer(target));
    outcome
}

#[test]
fn checked_mutations_are_each_followed_by_fresh_read_and_complete_immediately() {
    let now = Instant::now();
    let mut layers = LayerController::default();
    let mut snapshots = VecDeque::from([
        ObservedLayer::Below,
        ObservedLayer::Normal,
        ObservedLayer::Above,
    ]);
    let trace = RefCell::new(Vec::new());
    service_layer_with(
        &mut layers,
        Some(Layer::Above),
        || {
            trace.borrow_mut().push("read");
            Ok(Ok(snapshots.pop_front().unwrap()))
        },
        || {
            trace.borrow_mut().push("support");
            Ok(Ok(SUPPORT))
        },
        |mutation| {
            trace.borrow_mut().push(match mutation {
                Mutation::Remove(_) => "remove/check",
                Mutation::AddAbove => "add/check",
                _ => panic!("wrong mutation"),
            });
            Ok(())
        },
        || now,
    )
    .unwrap();
    assert_eq!(
        *trace.borrow(),
        [
            "read",
            "support",
            "remove/check",
            "read",
            "support",
            "add/check",
            "read"
        ]
    );
    assert_eq!(layers.observed, Some(ObservedLayer::Above));
    assert!(layers.deadline().is_none());
}

#[test]
fn fresh_idempotence_needs_no_support_query_or_send() {
    let mut layers = LayerController::default();
    service_layer_with(
        &mut layers,
        Some(Layer::Normal),
        || Ok(Ok(ObservedLayer::Normal)),
        || panic!("no mutation support proof required"),
        |_| panic!("idempotent request must not send"),
        Instant::now,
    )
    .unwrap();
    assert!(layers.deadline().is_none());
}

#[test]
fn whole_transition_support_is_checked_before_removal_and_again_before_addition() {
    let now = Instant::now();
    for support in [
        Ok(LayerSupport {
            above: false,
            ..SUPPORT
        }),
        Err("malformed support"),
    ] {
        let mut layers = LayerController::default();
        service_layer_with(
            &mut layers,
            Some(Layer::Above),
            || Ok(Ok(ObservedLayer::Below)),
            || Ok(support),
            |_| panic!("must reject before removal"),
            || now,
        )
        .unwrap();
        assert!(layers.pending().is_none());
        assert_eq!(layers.observed, Some(ObservedLayer::Below));
    }
    let mut layers = LayerController::default();
    pending_above(&mut layers, now);
    service_layer_with(
        &mut layers,
        None,
        || Ok(Ok(ObservedLayer::Normal)),
        || {
            Ok(Ok(LayerSupport {
                above: false,
                ..SUPPORT
            }))
        },
        |_| panic!("support lost: no addition or rollback"),
        || now,
    )
    .unwrap();
    assert_eq!(layers.observed, Some(ObservedLayer::Normal));
    assert!(layers.pending().is_none());
}

#[test]
fn absent_or_malformed_body_never_confirms_normal_or_removal() {
    let now = Instant::now();
    for target in [Layer::Normal, Layer::Above] {
        for reason in ["mapped body state absent", "malformed state"] {
            let mut layers = LayerController::default();
            layers.begin(target, ObservedLayer::Below, now);
            service_layer_with(
                &mut layers,
                None,
                || Ok(Err(reason)),
                || panic!("no support query"),
                |_| panic!("unverifiable removal cannot add"),
                || now,
            )
            .unwrap();
            assert_eq!(layers.observed, Some(ObservedLayer::Below));
            assert!(layers.deadline().is_none());
        }
    }
}

#[test]
fn later_drag_and_newer_popup_do_not_defer_addition_or_change_ownership() {
    let now = Instant::now();
    for popup in [false, true] {
        let mut runtime = ProbeRuntime::new();
        runtime.body = BodyAvailability::Ready;
        pending_above(&mut runtime.layers, now);
        if popup {
            runtime
                .acquire_menu_with(MenuPopup::test_popup(10, 2), Point::new(0, 0), || {
                    Ok((GrabStatus::SUCCESS, 200))
                })
                .unwrap();
        } else {
            runtime.interaction.handle_left_press(
                Point::new(80, 80),
                (80, 80),
                100,
                Point::new(0, 0),
            );
            runtime
                .pointer
                .acquire_with(CaptureOwner::BodyLeft, || Ok((GrabStatus::SUCCESS, 10)))
                .unwrap();
            runtime.interaction.on_grab_acquired();
            runtime.interaction.handle_motion(
                Point::new(100, 100),
                101,
                &ValidOriginBounds::new(0, 500, 0, 500),
            );
            assert!(runtime.interaction.is_dragging());
        }
        let state = runtime.interaction.state();
        let owner = runtime.pointer.owner();
        let sends = Cell::new(0);
        let mut snapshots = VecDeque::from([ObservedLayer::Normal, ObservedLayer::Above]);
        service_layer_with(
            &mut runtime.layers,
            None,
            || Ok(Ok(snapshots.pop_front().unwrap())),
            || Ok(Ok(SUPPORT)),
            |mutation| {
                assert_eq!(mutation, Mutation::AddAbove);
                sends.set(sends.get() + 1);
                Ok(())
            },
            || now,
        )
        .unwrap();
        assert_eq!(sends.get(), 1);
        assert_eq!(runtime.interaction.state(), state);
        assert_eq!(runtime.pointer.owner(), owner);
        assert_eq!(runtime.menu.is_some(), popup);
        assert!(runtime.layers.pending().is_none());
    }
}

#[test]
fn layer_selection_cleanup_precedes_dispatch_and_busy_preserves_original_operation() {
    let now = Instant::now();
    for busy in [false, true] {
        let mut runtime = captured_menu();
        if busy {
            pending_above(&mut runtime.layers, now);
        }
        let pending = runtime.layers.pending();
        runtime.interaction.menu_press(1, MenuHit::Row(2), 100, 0);
        let HostAction::CloseMenu { outcome, .. } =
            runtime.interaction.menu_release(1, MenuHit::Row(2), 101, 0)
        else {
            panic!("layer selection missing")
        };
        let trace = RefCell::new(Vec::new());
        runtime
            .complete_menu_with(
                &mut tests::test_window(),
                outcome,
                |pointer| {
                    pointer
                        .release_with(|| {
                            trace.borrow_mut().push("ungrab");
                            Ok(())
                        })
                        .map(|_| ())
                },
                |_| {
                    trace.borrow_mut().push("destroy");
                    Ok(())
                },
                |layers, target| {
                    assert_eq!(*trace.borrow(), ["ungrab", "destroy"]);
                    trace.borrow_mut().push("dispatch");
                    service_layer_with(
                        layers,
                        Some(target),
                        || {
                            assert!(!busy);
                            Ok(Ok(ObservedLayer::Below))
                        },
                        || panic!("no mutation"),
                        |_| panic!("no mutation"),
                        || now,
                    )
                    .map(|_| ())
                },
            )
            .unwrap();
        assert_eq!(*trace.borrow(), ["ungrab", "destroy", "dispatch"]);
        assert!(runtime.menu.is_none());
        assert!(!runtime.pointer.is_grabbed());
        assert!(runtime.interaction.is_idle());
        assert_eq!(runtime.layers.pending(), pending);
        assert_eq!(
            runtime.layers.desired,
            Some(if busy { Layer::Above } else { Layer::Below })
        );
    }
}

#[test]
fn failed_popup_release_or_destruction_blocks_dispatch_and_preserves_error_priority() {
    for release_fails in [false, true] {
        let mut runtime = captured_menu();
        let destroyed = Cell::new(false);
        let result = runtime.complete_menu_with(
            &mut tests::test_window(),
            MenuOutcome::SetLayer(Layer::Above),
            |pointer| {
                pointer
                    .release_with(|| {
                        if release_fails {
                            Err("release failure".into())
                        } else {
                            Ok(())
                        }
                    })
                    .map(|_| ())
            },
            |_| {
                destroyed.set(true);
                Err("destroy failure".into())
            },
            |_, _| panic!("failed cleanup must not dispatch"),
        );
        let error = result.unwrap_err();
        assert_eq!(
            error.to_string(),
            if release_fails {
                "release failure"
            } else {
                "destroy failure"
            }
        );
        assert!(destroyed.get());
        assert_eq!(runtime.pointer.is_grabbed(), release_fails);
        assert!(runtime.menu.is_some());
        assert!(runtime.layers.desired.is_none());
    }
}

#[test]
fn protocol_read_support_send_and_check_failures_are_fatal_without_rollback() {
    let now = Instant::now();
    for fail_at in [
        "read",
        "support",
        "send",
        "check",
        "post-read",
        "second-send",
        "second-post-read",
    ] {
        let mut layers = LayerController::default();
        let reads = Cell::new(0);
        let sends = Cell::new(0);
        let result = service_layer_with(
            &mut layers,
            Some(Layer::Above),
            || {
                let count = reads.get();
                reads.set(count + 1);
                if (fail_at == "read" && count == 0)
                    || (fail_at == "post-read" && count == 1)
                    || (fail_at == "second-post-read" && count == 2)
                {
                    return Err(fail_at.into());
                }
                Ok(Ok(if count == 0 {
                    ObservedLayer::Below
                } else {
                    ObservedLayer::Normal
                }))
            },
            || {
                if fail_at == "support" {
                    Err(fail_at.into())
                } else {
                    Ok(Ok(SUPPORT))
                }
            },
            |_| {
                sends.set(sends.get() + 1);
                if matches!(fail_at, "send" | "check")
                    || fail_at == "second-send" && sends.get() == 2
                {
                    Err(fail_at.into())
                } else {
                    Ok(())
                }
            },
            || now,
        );
        assert_eq!(result.unwrap_err().to_string(), fail_at);
        assert!(layers.deadline().is_none());
        if fail_at == "second-send" {
            assert_eq!(sends.get(), 2);
            assert_eq!(layers.observed, Some(ObservedLayer::Normal));
        }
    }
}

#[test]
fn layer_host_error_keeps_priority_over_later_pointer_and_resource_cleanup() {
    let now = Instant::now();
    let mut runtime = captured_menu();
    pending_above(&mut runtime.layers, now);
    let result = service_layer_with(
        &mut runtime.layers,
        None,
        || Err("original layer read error".into()),
        || panic!("no support"),
        |_| panic!("no send"),
        || now,
    );
    let cleanup_attempted = Cell::new(false);
    let outcome = finish_probe_with(result.map(|_| ()), || {
        runtime.cancel_menu_with(
            &mut tests::test_window(),
            |pointer| {
                pointer
                    .release_with(|| Err("ungrab error".into()))
                    .map(|_| ())
            },
            |_| {
                cleanup_attempted.set(true);
                Err("destroy error".into())
            },
        )
    });
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "original layer read error"
    );
    assert!(cleanup_attempted.get());
    assert!(runtime.pointer.is_grabbed());
}

fn property(window: u32, atom: u32, sequence: u16) -> Event {
    Event::PropertyNotify(PropertyNotifyEvent {
        response_type: 28,
        sequence,
        window,
        atom,
        time: 100,
        state: Property::NEW_VALUE,
    })
}
fn atoms() -> LayerAtoms {
    LayerAtoms {
        state: 20,
        supported: 21,
        above: 22,
        below: 23,
    }
}
fn layout_atoms() -> x11::monitors::LayoutAtoms {
    x11::monitors::LayoutAtoms {
        number_of_desktops: 90,
        desktop_viewport: 91,
        workarea: 30,
        current_desktop: 31,
        wm_desktop: 32,
    }
}

#[test]
fn stale_relevant_notifications_reread_truth_unrelated_events_cannot_confirm_or_cancel_input() {
    let now = Instant::now();
    let mut layers = LayerController::default();
    pending_above(&mut layers, now);
    let reads = Cell::new(0);
    for event in [
        property(99, 20, 0),
        property(100, 99, 0),
        property(100, 20, 0),
        property(100, 20, u16::MAX),
    ] {
        assert!(!interrupts_pending_movement(
            &event,
            1,
            100,
            &layout_atoms()
        ));
        assert!(!interrupts_layer(&event, 1, 100, &layout_atoms()));
        if layer_property_event(&event, 100, &atoms()) {
            service_layer_with(
                &mut layers,
                None,
                || {
                    reads.set(reads.get() + 1);
                    Ok(Ok(ObservedLayer::Below))
                },
                || panic!("not ready to add"),
                |_| panic!("stale event cannot confirm removal"),
                || now,
            )
            .unwrap();
        }
    }
    assert_eq!(reads.get(), 2);
    assert!(layers.pending().is_some());
    service_layer_with(
        &mut layers,
        None,
        || Ok(Ok(ObservedLayer::Above)),
        || panic!("already complete"),
        |_| panic!("already complete"),
        || now,
    )
    .unwrap();
    assert!(layers.pending().is_none());
}

#[test]
fn fixed_deadline_wins_under_event_traffic_and_final_read_can_confirm_or_timeout() {
    let now = Instant::now();
    for final_observed in [
        ObservedLayer::Above,
        ObservedLayer::Normal,
        ObservedLayer::Conflict,
    ] {
        let mut layers = LayerController::default();
        pending_above(&mut layers, now);
        let deadline = layers.deadline().unwrap();
        let mut queue: VecDeque<_> = (0..200).map(|_| property(99, 99, 0)).collect();
        let mut buffered = None;
        let mut reads = 0;
        while !queue.is_empty() {
            if layers.deadline().is_some_and(|time| deadline >= time) {
                service_layer_with(
                    &mut layers,
                    None,
                    || {
                        reads += 1;
                        Ok(Ok(final_observed))
                    },
                    || panic!("timeout cannot query mutation support"),
                    |_| panic!("timeout cannot add or retry"),
                    || deadline,
                )
                .unwrap();
            }
            drain_events_bounded(&mut buffered, 64, || Ok::<_, HostError>(queue.pop_front()))
                .unwrap();
            assert!(layers.deadline().is_none());
        }
        assert_eq!(reads, 1);
        assert_eq!(layers.observed, Some(final_observed));
        service_layer_with(
            &mut layers,
            None,
            || Ok(Ok(ObservedLayer::Above)),
            || panic!("late property cannot revive operation"),
            |_| panic!("no reconciliation"),
            || deadline,
        )
        .unwrap();
        assert!(layers.pending().is_none());
    }
}

#[test]
fn expired_support_event_skips_capabilities_and_only_observes_final_body_state() {
    let started = Instant::now();
    for final_observed in [ObservedLayer::Above, ObservedLayer::Normal] {
        let mut layers = LayerController::default();
        pending_above(&mut layers, started);
        let handled_at = layers.deadline().unwrap() + Duration::from_millis(1);
        let reads = Cell::new(0);
        service_layer_support_event_with(
            &mut layers,
            || panic!("expired _NET_SUPPORTED event must not read capabilities"),
            |layers| {
                service_layer_with(
                    layers,
                    None,
                    || {
                        reads.set(reads.get() + 1);
                        Ok(Ok(final_observed))
                    },
                    || panic!("final body observation must not query capabilities"),
                    |_| panic!("final body observation must not send a mutation"),
                    || handled_at,
                )
                .map(|_| ())
            },
            || handled_at,
        )
        .unwrap();
        assert_eq!(reads.get(), 1);
        assert_eq!(layers.observed, Some(final_observed));
        assert_eq!(layers.desired, Some(Layer::Above));
        assert!(layers.pending().is_none());
        assert!(layers.deadline().is_none());
    }
}

#[test]
fn expiry_during_support_query_performs_final_read_without_sending() {
    let started = Instant::now();
    for advertised in [Ok(SUPPORT), Err("support malformed at expiry")] {
        let clock = Cell::new(started);
        let mut layers = LayerController::default();
        let reads = Cell::new(0);
        service_layer_with(
            &mut layers,
            Some(Layer::Above),
            || {
                reads.set(reads.get() + 1);
                Ok(Ok(ObservedLayer::Normal))
            },
            || {
                clock.set(started + crate::layer::CONFIRMATION_TIMEOUT);
                Ok(advertised)
            },
            |_| panic!("deadline passed while reading support"),
            || clock.get(),
        )
        .unwrap();
        assert_eq!(reads.get(), 2);
        assert!(layers.pending().is_none());
    }
}

#[test]
fn quit_dismiss_duration_and_lifecycle_preserve_cleanup_and_end_only_appropriate_work() {
    use x11rb::protocol::xproto::{DestroyNotifyEvent, MapNotifyEvent, UnmapNotifyEvent};
    let now = Instant::now();
    for item in [MenuItem::Dismiss, MenuItem::Quit] {
        let mut runtime = captured_menu();
        pending_above(&mut runtime.layers, now);
        let quit = runtime
            .complete_menu_with(
                &mut tests::test_window(),
                item.outcome(),
                |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                |_| Ok(()),
                |_, _| panic!("not a layer action"),
            )
            .unwrap();
        assert_eq!(quit, item == MenuItem::Quit);
        assert_eq!(runtime.layers.pending().is_some(), !quit);
        assert!(!runtime.pointer.is_grabbed());
        assert!(runtime.menu.is_none());
    }
    let events = [
        property(1, 30, 0),
        property(1, 31, 0),
        property(100, 32, 0),
        Event::MapNotify(MapNotifyEvent {
            response_type: 19,
            sequence: 0,
            event: 1,
            window: 100,
            override_redirect: false,
        }),
        Event::UnmapNotify(UnmapNotifyEvent {
            response_type: 18,
            sequence: 0,
            event: 1,
            window: 100,
            from_configure: false,
        }),
        Event::DestroyNotify(DestroyNotifyEvent {
            response_type: 17,
            sequence: 0,
            event: 1,
            window: 100,
        }),
    ];
    for event in events {
        assert!(interrupts_layer(&event, 1, 100, &layout_atoms()));
    }
    for reason in [
        "duration expiry",
        "layout or body lifecycle change",
        "shutdown",
    ] {
        let mut runtime = captured_menu();
        pending_above(&mut runtime.layers, now);
        abort_layer(&mut runtime.layers, reason);
        runtime
            .cancel_menu_with(
                &mut tests::test_window(),
                |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                |_| Ok(()),
            )
            .unwrap();
        assert!(runtime.layers.deadline().is_none());
        assert!(!runtime.pointer.is_grabbed());
        assert!(runtime.menu.is_none());
        assert!(runtime.interaction.is_idle());
    }
}

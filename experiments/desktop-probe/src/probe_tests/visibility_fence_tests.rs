use super::*;
use crate::control::{Capability, Evidence, MapState, VisibilityEvidence, WmState, Workspace};
use crate::geometry::MenuHit;
use std::cell::RefCell;
use x11rb::protocol::xproto::{ButtonPressEvent, EnterNotifyEvent, MotionNotifyEvent};

fn evidence() -> VisibilityEvidence {
    VisibilityEvidence {
        destroyed: false,
        wm_state: WmState::Normal,
        map_state: Evidence::Readable(MapState::Viewable),
        hidden: Evidence::Readable(false),
        workspace: Workspace::Current,
        capability: Capability::Supported,
        coherent: true,
    }
}
fn placement() -> InputPlacement {
    InputPlacement {
        selected_monitor: 20,
        observed_monitor: 20,
        geometry: crate::geometry::Rect::new(100, 100, 160, 160),
        bounds: crate::geometry::ValidOriginBounds::new(0, 500, 0, 500),
    }
}
pub(super) fn runtime(case: u8) -> ProbeRuntime {
    let mut r = ProbeRuntime::new();
    r.body = BodyAvailability::Ready;
    match case {
        1 => {
            r.interaction.handle_left_press(
                Point::new(180, 180),
                (80, 80),
                100,
                Point::new(100, 100),
            );
            r.interaction.on_grab_acquired();
            r.interaction
                .handle_motion(Point::new(220, 220), 101, &placement().bounds);
            assert!(r.interaction.is_dragging());
            r.pointer.set_grabbed(true);
        }
        2 => {
            r.interaction.handle_right_press(100);
            r.pointer.track_automatic_right().unwrap();
        }
        3 | 4 => {
            r.menu_generation = 5;
            r.menu = Some(MenuPopup::test_popup(200, 5));
            r.pointer
                .acquire_with(
                    CaptureOwner::Menu {
                        window: 200,
                        generation: 5,
                    },
                    || Ok((GrabStatus::SUCCESS, 60000)),
                )
                .unwrap();
            r.interaction.menu_acquired(MenuHit::Outside);
            if case == 4 {
                r.interaction.menu_press(1, MenuHit::Row(0), 100, 0);
            }
        }
        _ => {} // Idle / queued server automatic capture with no tracked owner.
    }
    r
}
fn assert_invalidated(r: &ProbeRuntime, window: &ManagedProbeWindow) {
    assert!(matches!(
        r.interaction.state(),
        InteractionState::Idle | InteractionState::SuppressedUntilRelease { .. }
    ));
    assert!(r.pending_move.is_none());
    assert!(r.final_target.is_none());
    assert!(r.confirmation_deadline.is_none());
    assert!(r.correction.is_none());
    assert!(r.safety.deadline.is_none());
    assert!(r.safety.observation.is_none());
    assert!(window.in_flight_moves.is_empty());
    assert_eq!(window.requested_origin(), window.confirmed_origin());
    assert!(!r.accepts_input());
}
fn fence(r: &mut ProbeRuntime, w: &mut ManagedProbeWindow) {
    r.fence_visibility_input_with(
        w,
        || Ok(70000),
        |r, w| {
            assert_invalidated(r, w);
            Ok(())
        },
        |_| Ok(()),
    )
    .unwrap();
}
#[test]
fn every_gesture_is_invalidated_between_empty_shape_and_ungrab_before_popup_cleanup() {
    for case in 0..=5 {
        let mut r = runtime(case);
        let mut w = tests::test_window();
        r.pending_move = Some(Point::new(220, 220));
        r.final_target = r.pending_move;
        r.confirmation_deadline = Some(Instant::now());
        r.correction = Some((Point::new(220, 220), Instant::now()));
        r.safety.acquired(Instant::now());
        r.safety.observed(false, 60000);
        w.in_flight_moves.push_back(Point::new(220, 220));
        let generation = r.menu_generation;
        let trace = RefCell::new(vec![]);
        r.fence_visibility_input_with(
            &mut w,
            || {
                trace.borrow_mut().push("empty");
                Ok(70000)
            },
            |r, w| {
                assert_invalidated(r, w);
                assert!(r.pointer.is_grabbed()); // Even idle/queued automatic capture.
                assert_eq!(r.menu_generation, generation + 1);
                assert_eq!(*trace.borrow(), ["empty"]);
                assert!(!r.visibility_dispatch_eligible());
                trace.borrow_mut().push("ungrab");
                Ok(())
            },
            |_| {
                trace.borrow_mut().push("popup");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            *trace.borrow(),
            if case == 3 || case == 4 {
                vec!["empty", "ungrab", "popup"]
            } else {
                vec!["empty", "ungrab"]
            }
        );
        assert!(r.visibility_dispatch_eligible());
        // Fake future dispatch only; no native request helper is connected.
        if r.visibility_dispatch_eligible() {
            trace.borrow_mut().push("fake dispatch");
        }
        assert_eq!(trace.borrow().last(), Some(&"fake dispatch"));
        assert_invalidated(&r, &w);
    }
}
#[test]
fn all_failure_combinations_attempt_cleanup_preserve_primary_and_block_dispatch() {
    for failures in 1..8 {
        let mut r = runtime(4);
        let mut w = tests::test_window();
        let trace = RefCell::new(vec![]);
        let error = r
            .fence_visibility_input_with(
                &mut w,
                || {
                    trace.borrow_mut().push("empty");
                    if failures & 1 != 0 {
                        Err("shape".into())
                    } else {
                        Ok(70000)
                    }
                },
                |r, w| {
                    assert_invalidated(r, w);
                    trace.borrow_mut().push("ungrab");
                    if failures & 2 != 0 {
                        Err("release".into())
                    } else {
                        Ok(())
                    }
                },
                |popup| {
                    trace.borrow_mut().push("popup");
                    popup.window_resource.release_with(|| {
                        if failures & 4 != 0 {
                            Err("destroy".into())
                        } else {
                            Ok(())
                        }
                    })
                },
            )
            .unwrap_err();
        assert_eq!(*trace.borrow(), ["empty", "ungrab", "popup"]);
        assert_eq!(
            error.to_string(),
            if failures & 1 != 0 {
                "shape"
            } else if failures & 2 != 0 {
                "release"
            } else {
                "destroy"
            }
        );
        assert_eq!(r.pointer.is_grabbed(), failures & 2 != 0);
        assert_eq!(r.menu.is_some(), failures & 4 != 0);
        assert!(!r.visibility_dispatch_eligible());
        assert!(!r.accepts_input());
        r.body = BodyAvailability::Ready; // A later lifecycle event cannot bypass debt.
        assert!(r
            .restore_visibility_input_with(&w, evidence(), &placement(), || panic!(
                "unresolved fence"
            ))
            .is_err());
    }
}
#[test]
fn repeated_fencing_can_settle_release_but_cannot_retry_uncertain_popup_destruction() {
    let mut r = runtime(4);
    let mut w = tests::test_window();
    assert!(r
        .fence_visibility_input_with(
            &mut w,
            || Ok(1),
            |_, _| Err("release".into()),
            |p| p.window_resource.release_with(|| Err("destroy".into()))
        )
        .is_err());
    assert!(r
        .fence_visibility_input_with(
            &mut w,
            || Ok(2),
            |_, _| Ok(()),
            |p| p
                .window_resource
                .release_with(|| panic!("uncertain destroy must not be resent"))
        )
        .is_err());
    assert!(!r.pointer.is_grabbed());
    assert!(r.menu.is_some());
    assert!(!r.visibility_dispatch_eligible());
    r.menu
        .as_ref()
        .unwrap()
        .window_resource
        .externally_destroyed();
    r.fence_visibility_input_with(
        &mut w,
        || Ok(3),
        |_, _| Ok(()),
        |p| {
            p.window_resource
                .release_with(|| panic!("already destroyed"))
        },
    )
    .unwrap();
    assert!(r.visibility_dispatch_eligible());
    fence(&mut r, &mut w);
    assert!(r.visibility_dispatch_eligible());
}
#[test]
fn completed_menu_outcome_cannot_dispatch_after_transition_invalidation() {
    let mut r = runtime(4);
    let mut w = tests::test_window();
    let HostAction::CloseMenu { outcome, .. } =
        r.interaction.menu_release(1, MenuHit::Row(0), 101, 0)
    else {
        panic!("expected selection")
    };
    fence(&mut r, &mut w);
    assert!(r
        .complete_menu_with(
            &mut w,
            outcome,
            |_| Ok(()),
            |_| panic!("no popup"),
            |_, _| panic!("stale action")
        )
        .is_err());
}
#[test]
fn enabling_requires_fresh_local_managed_visibility_placement_and_every_settled_obligation() {
    for case in 0..17 {
        let mut r = runtime(0);
        let mut w = tests::test_window();
        fence(&mut r, &mut w);
        r.body = BodyAvailability::Ready;
        let mut e = evidence();
        let mut p = placement();
        match case {
            0 => e.workspace = Workspace::Other,
            1 => e.wm_state = WmState::Iconic,
            2 => e.hidden = Evidence::Readable(true),
            3 => e.map_state = Evidence::Readable(MapState::Unviewable),
            4 => e.destroyed = true,
            5 => e.wm_state = WmState::Withdrawn,
            6 => e.coherent = false,
            7 => e.capability = Capability::Unsupported,
            8 => p.observed_monitor = 21,
            9 => p.geometry.width = 159,
            10 => p.geometry.x = 101,
            11 => p.bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50),
            12 => r.correction = Some((Point::new(100, 100), Instant::now())),
            13 => r.pointer.require_transition_release(),
            14 => r.menu = Some(MenuPopup::test_popup(200, 9)),
            15 => r.body = BodyAvailability::Validating,
            16 => r.input_fence.empty_confirmed = false,
            _ => unreachable!(),
        }
        assert!(r
            .restore_visibility_input_with(&w, e, &p, || panic!("not ready: {case}"))
            .is_err());
        assert!(!r.accepts_input());
    }
}
#[test]
fn failed_enable_retains_blocking_and_requires_new_native_empty_fence() {
    let mut r = runtime(0);
    let mut w = tests::test_window();
    fence(&mut r, &mut w);
    r.body = BodyAvailability::Ready;
    assert!(r
        .restore_visibility_input_with(&w, evidence(), &placement(), || Err("enable check".into()))
        .is_err());
    assert!(!r.visibility_dispatch_eligible());
    assert!(!r.accepts_input());
    assert!(r
        .restore_visibility_input_with(&w, evidence(), &placement(), || panic!("no blind retry"))
        .is_err());
    fence(&mut r, &mut w);
    r.body = BodyAvailability::Ready;
    r.restore_visibility_input_with(&w, evidence(), &placement(), || Ok(140000))
        .unwrap();
    assert!(r.accepts_input());
    assert_eq!(r.input_fence.enabled_after, Some(140000));
}
fn pointer_events() -> Vec<Event> {
    let button = ButtonPressEvent {
        event: 100,
        detail: 1,
        time: 200,
        event_x: 80,
        event_y: 80,
        root_x: 180,
        root_y: 180,
        same_screen: true,
        ..Default::default()
    };
    vec![
        Event::ButtonPress(button),
        Event::ButtonRelease(button),
        Event::MotionNotify(MotionNotifyEvent {
            event: 100,
            ..Default::default()
        }),
        Event::EnterNotify(EnterNotifyEvent {
            event: 100,
            ..Default::default()
        }),
        Event::LeaveNotify(EnterNotifyEvent {
            event: 100,
            ..Default::default()
        }),
    ]
}
#[test]
fn full_sequence_gate_rejects_stale_presses_releases_motion_and_crossings_across_wrap() {
    let mut r = runtime(0);
    let mut w = tests::test_window();
    for event in pointer_events() {
        assert!(!r.input_fence.rejects(&event, 100, 1));
    }
    fence(&mut r, &mut w);
    for event in pointer_events() {
        assert!(r.input_fence.rejects(&event, 100, 90000));
    }
    r.body = BodyAvailability::Ready;
    let boundary = 2 * 65536 + 100;
    r.restore_visibility_input_with(&w, evidence(), &placement(), || Ok(boundary))
        .unwrap();
    for event in pointer_events() {
        for sequence in [100, 65536 + 100, boundary - 1] {
            assert!(r.input_fence.rejects(&event, 100, sequence));
        }
        assert!(!r.input_fence.rejects(&event, 100, boundary));
        assert!(!r.input_fence.rejects(&event, 100, boundary + 1));
        assert!(!r.input_fence.rejects(&event, 999, 1));
    }
    // A new legitimate press reaches the existing gesture/capture path.
    let event = pointer_events().remove(0);
    if !r.input_fence.rejects(&event, 100, boundary) && r.accepts_input() {
        assert!(matches!(
            r.interaction.handle_left_press(
                Point::new(180, 180),
                (80, 80),
                200,
                Point::new(100, 100)
            ),
            HostAction::AcquireGrab { .. }
        ));
    } else {
        panic!("fresh input lost");
    }
}
#[test]
fn stale_buffered_input_spanning_bounded_batches_cannot_recreate_actions() {
    let mut r = runtime(1);
    let mut w = tests::test_window();
    fence(&mut r, &mut w);
    r.body = BodyAvailability::Ready;
    r.restore_visibility_input_with(&w, evidence(), &placement(), || Ok(140000))
        .unwrap();
    let mut queue = std::collections::VecDeque::new();
    for _ in 0..40 {
        for event in pointer_events() {
            queue.push_back((event, 140000 - 65536));
        }
    }
    let mut buffered = None;
    let mut seen = 0;
    while !queue.is_empty() {
        let batch =
            drain_events_bounded(&mut buffered, 64, || Ok::<_, HostError>(queue.pop_front()))
                .unwrap();
        assert!(batch.len() <= 64);
        for (event, sequence) in batch {
            assert!(r.input_fence.rejects(&event, 100, sequence));
            seen += 1;
        }
    }
    assert_eq!(seen, 200);
    assert!(!r.interaction.has_left_gesture());
    assert!(r.pending_move.is_none());
    assert!(!r.pointer.is_grabbed());
}

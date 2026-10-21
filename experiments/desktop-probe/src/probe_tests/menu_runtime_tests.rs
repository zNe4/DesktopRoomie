use super::*;
use crate::geometry::MenuHit;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use x11rb::protocol::xproto::{
    ButtonReleaseEvent, DestroyNotifyEvent, KeyButMask, MotionNotifyEvent, Property,
    PropertyNotifyEvent, UnmapNotifyEvent,
};

fn captured_menu() -> ProbeRuntime {
    let mut runtime = ProbeRuntime::new();
    runtime.body = BodyAvailability::Ready;
    runtime.menu_generation = 2;
    runtime.menu = Some(MenuPopup::test_popup(10, 2));
    runtime
        .pointer
        .acquire_with(
            CaptureOwner::Menu {
                window: 10,
                generation: 2,
            },
            || Ok((GrabStatus::SUCCESS, 70000)),
        )
        .unwrap();
    runtime.interaction.menu_acquired(MenuHit::Outside);
    runtime
}

#[test]
fn cancellation_invalidates_menu_then_ungrabs_before_destroy_and_is_idempotent() {
    let mut runtime = captured_menu();
    let mut body = tests::test_window();
    runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
    runtime.safety.acquired(Instant::now());
    runtime.pending_move = Some(Point::new(200, 200));
    let trace = RefCell::new(Vec::new());
    runtime
        .cancel_menu_with(
            &mut body,
            |pointer| {
                pointer
                    .release_with(|| {
                        trace.borrow_mut().push("ungrab");
                        Ok(())
                    })
                    .map(|_| ())
            },
            |_| {
                trace.borrow_mut().push("popup");
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(*trace.borrow(), ["ungrab", "popup"]);
    assert!(runtime.interaction.is_idle());
    assert!(runtime.pending_move.is_none());
    assert!(runtime.safety.deadline.is_none());
    assert!(!runtime.pointer.is_grabbed());
    assert!(runtime.menu.is_none());
    assert!(runtime.accepts_input());
    runtime
        .cancel_menu_with(
            &mut body,
            |pointer| {
                pointer
                    .release_with(|| panic!("duplicate ungrab"))
                    .map(|_| ())
            },
            |_| panic!("duplicate popup destroy"),
        )
        .unwrap();
}

#[test]
fn failed_release_still_destroys_popup_and_cleanup_retry_retains_owner_until_ack() {
    let mut runtime = captured_menu();
    let mut body = tests::test_window();
    let calls = Cell::new(0);
    let error = runtime
        .cancel_menu_with(
            &mut body,
            |pointer| {
                pointer
                    .release_with(|| Err("release not acknowledged".into()))
                    .map(|_| ())
            },
            |popup| {
                popup.window_resource.release_with(|| {
                    calls.set(calls.get() + 1);
                    Ok(())
                })
            },
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "release not acknowledged");
    assert!(runtime.interaction.is_idle());
    assert!(runtime.pointer.is_grabbed());
    assert!(runtime.menu.is_some());
    assert!(!runtime.accepts_input());
    runtime
        .cancel_menu_with(
            &mut body,
            |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
            |popup| {
                popup
                    .window_resource
                    .release_with(|| panic!("acknowledged destroy repeated"))
            },
        )
        .unwrap();
    assert_eq!(calls.get(), 1);
    assert!(runtime.menu.is_none());
    assert!(!runtime.pointer.is_grabbed());
}

#[test]
fn fatal_error_keeps_priority_over_popup_release_and_destroy_failures() {
    let mut runtime = captured_menu();
    let mut body = tests::test_window();
    let destroyed = Cell::new(false);
    let error = finish_probe_with(Err("original X11 failure".into()), || {
        runtime.cancel_menu_with(
            &mut body,
            |pointer| {
                pointer
                    .release_with(|| Err("release failure".into()))
                    .map(|_| ())
            },
            |_| {
                destroyed.set(true);
                Err("destroy failure".into())
            },
        )
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "original X11 failure");
    assert!(destroyed.get());
    assert!(runtime.pointer.is_grabbed());
}

#[test]
fn pending_opening_cancellation_releases_automatic_grab_and_blocks_late_release() {
    let mut runtime = ProbeRuntime::new();
    let mut body = tests::test_window();
    runtime.interaction.handle_right_press(100);
    runtime.pointer.track_automatic_right().unwrap();
    runtime.safety.acquired(Instant::now());
    let releases = Cell::new(0);
    runtime
        .cancel_menu_with(
            &mut body,
            |pointer| {
                pointer
                    .release_with(|| {
                        releases.set(releases.get() + 1);
                        Ok(())
                    })
                    .map(|_| ())
            },
            |_| panic!("no popup exists"),
        )
        .unwrap();
    assert_eq!(releases.get(), 1);
    assert_eq!(
        runtime
            .interaction
            .handle_opening_release(3, 101, 0, Point::new(180, 180), true),
        HostAction::None
    );
    assert_eq!(runtime.pointer.owner(), None);
    assert!(runtime.safety.deadline.is_none());
}

#[test]
fn long_menu_hold_rearms_and_missed_release_cancels_without_activation() {
    let mut runtime = captured_menu();
    runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
    runtime.sync_safety(None);
    let now = Instant::now();
    for sequence in 70000..70100 {
        runtime.safety.observed(true, sequence);
        assert!(!runtime.apply_safety(now, None));
        assert_eq!(runtime.safety.deadline, Some(now + POINTER_CHECK_INTERVAL));
    }
    runtime.safety.observed(false, 70100);
    assert!(runtime.apply_safety(now, None));
    let mut body = tests::test_window();
    runtime
        .cancel_menu_with(
            &mut body,
            |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(
        runtime.interaction.menu_release(1, MenuHit::Row(4), 110, 0),
        HostAction::None
    );
}

#[test]
fn menu_neutral_has_no_timer_and_chord_invalidates_old_button_observation() {
    let mut runtime = captured_menu();
    assert!(runtime.safety.deadline.is_none());
    runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
    runtime.sync_safety(None);
    runtime.safety.observed(false, 70000);
    runtime.interaction.menu_press(2, MenuHit::Row(4), 101, 1);
    runtime.sync_safety(Some(1));
    assert_eq!(runtime.interaction.active_button(), Some(0));
    assert!(!runtime.apply_safety(Instant::now(), None));
    assert!(runtime.safety.observation.is_none());
    assert!(runtime.safety.deadline.is_some());
    runtime.interaction.menu_release(1, MenuHit::Row(4), 102, 2);
    runtime.interaction.menu_release(2, MenuHit::Row(4), 103, 0);
    runtime.sync_safety(Some(0));
    assert!(runtime.menu.is_some());
    assert!(runtime.pointer.is_grabbed());
    assert!(runtime.safety.deadline.is_none());
}

#[test]
fn queued_menu_completion_precedes_observation_across_bounded_batches() {
    let mut runtime = captured_menu();
    runtime.interaction.menu_press(1, MenuHit::Outside, 100, 0);
    runtime.sync_safety(None);
    runtime.safety.observed(false, 70100);
    let motion = Event::MotionNotify(MotionNotifyEvent {
        response_type: 6,
        detail: Default::default(),
        sequence: 0,
        time: 101,
        root: 1,
        event: 10,
        child: 0,
        root_x: 500,
        root_y: 500,
        event_x: 700,
        event_y: 452,
        state: KeyButMask::BUTTON1,
        same_screen: true,
    });
    let release = Event::ButtonRelease(ButtonReleaseEvent {
        response_type: 5,
        detail: 1,
        sequence: 0,
        time: 102,
        root: 1,
        event: 10,
        child: 0,
        root_x: 500,
        root_y: 500,
        event_x: 700,
        event_y: 452,
        state: KeyButMask::BUTTON1,
        same_screen: true,
    });
    let mut queue = VecDeque::from(vec![(motion, 70099); 130]);
    queue.push_back((release, 70099));
    let mut buffered = None;
    let mut completions = 0;
    while !queue.is_empty() {
        let batch =
            drain_events_bounded(&mut buffered, 64, || Ok::<_, HostError>(queue.pop_front()))
                .unwrap();
        for (event, sequence) in batch {
            assert!(!runtime.apply_safety(Instant::now(), Some(sequence)));
            if let Event::ButtonRelease(ev) = event {
                assert_eq!(
                    runtime.interaction.menu_release(
                        ev.detail,
                        MenuHit::Outside,
                        ev.time,
                        buttons_after_release(ev.state, ev.detail)
                    ),
                    HostAction::CloseMenu {
                        time: 102,
                        outcome: MenuOutcome::Dismiss
                    }
                );
                runtime.sync_safety(Some(1));
                completions += 1;
            }
        }
    }
    assert_eq!(completions, 1);
    assert!(!runtime.apply_safety(Instant::now(), None));
}

#[test]
fn layout_and_window_lifecycle_identify_interruptions_before_refresh() {
    let atoms = x11::monitors::LayoutAtoms {
        number_of_desktops: 90,
        desktop_viewport: 91,
        current_desktop: 20,
        workarea: 21,
        wm_desktop: 22,
    };
    let root = 1;
    let body = 100;
    let events = [
        Event::PropertyNotify(PropertyNotifyEvent {
            response_type: 28,
            sequence: 0,
            window: root,
            atom: 20,
            time: 100,
            state: Property::NEW_VALUE,
        }),
        Event::PropertyNotify(PropertyNotifyEvent {
            response_type: 28,
            sequence: 0,
            window: root,
            atom: 21,
            time: 100,
            state: Property::NEW_VALUE,
        }),
        Event::UnmapNotify(UnmapNotifyEvent {
            response_type: 18,
            sequence: 0,
            event: root,
            window: body,
            from_configure: false,
        }),
        Event::DestroyNotify(DestroyNotifyEvent {
            response_type: 17,
            sequence: 0,
            event: root,
            window: body,
        }),
        Event::UnmapNotify(UnmapNotifyEvent {
            response_type: 18,
            sequence: 0,
            event: root,
            window: 10,
            from_configure: false,
        }),
        Event::DestroyNotify(DestroyNotifyEvent {
            response_type: 17,
            sequence: 0,
            event: root,
            window: 10,
        }),
    ];
    for event in events {
        let mut runtime = captured_menu();
        assert!(
            interrupts_pending_movement(&event, root, body, &atoms)
                || popup_lifecycle(
                    &event,
                    runtime.menu.as_ref().map(|popup| popup.input),
                    70000
                )
        );
        let mut window = tests::test_window();
        runtime
            .cancel_menu_with(
                &mut window,
                |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
                |_| Ok(()),
            )
            .unwrap();
        assert!(runtime.menu.is_none());
        assert!(!runtime.pointer.is_grabbed());
        assert!(runtime.interaction.is_idle());
    }
    assert!(!popup_lifecycle(
        &Event::DestroyNotify(DestroyNotifyEvent {
            response_type: 17,
            sequence: 0,
            event: root,
            window: 9
        }),
        Some(MenuPopup::test_popup(10, 2).input),
        70000
    ));
    // Duration expiry uses the same cancellation path, including a held Quit row.
    let mut runtime = captured_menu();
    runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
    runtime.sync_safety(None);
    runtime
        .cancel_menu_with(
            &mut tests::test_window(),
            |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
            |_| Ok(()),
        )
        .unwrap();
    assert!(runtime.menu.is_none());
    assert!(runtime.safety.deadline.is_none());
}

#[test]
fn ordered_observed_chords_preserve_menu_and_automatic_owners_until_all_up() {
    for buttons in [2, 3] {
        let mut runtime = captured_menu();
        runtime.interaction.menu_press(1, MenuHit::Row(4), 100, 0);
        runtime.sync_safety(None);
        runtime.safety.observed_buttons(1, buttons, true, 70100);
        assert!(!runtime.apply_safety(Instant::now(), Some(70099)));
        assert_eq!(runtime.interaction.active_button(), Some(1));
        assert!(!runtime.apply_safety(Instant::now(), Some(70100)));
        assert_eq!(runtime.interaction.active_button(), Some(0));
        assert!(runtime.menu.is_some());
        assert!(runtime.pointer.is_grabbed());
        runtime.safety.observed_buttons(0, 2, true, 70200);
        assert!(!runtime.apply_safety(Instant::now(), None));
        runtime.safety.observed_buttons(0, 0, true, 70300);
        assert!(runtime.apply_safety(Instant::now(), None));
    }
    let mut runtime = ProbeRuntime::new();
    runtime.interaction.handle_right_press(100);
    runtime.pointer.track_automatic_right().unwrap();
    runtime.safety.acquired(Instant::now());
    runtime.safety.observed_buttons(3, 1, true, 1000);
    assert!(!runtime.apply_safety(Instant::now(), None));
    assert_eq!(runtime.pointer.owner(), Some(CaptureOwner::OpeningRight));
    assert_eq!(runtime.interaction.active_button(), Some(0));
    assert_eq!(
        runtime
            .interaction
            .handle_opening_release(1, 110, 0, Point::new(180, 180), true),
        HostAction::OpeningFinished {
            time: 110,
            anchor: None
        }
    );
    runtime.pointer.automatic_right_finished().unwrap();
}

#[test]
fn popup_acquisition_success_and_each_denial_have_single_attempt_and_checked_teardown() {
    for status in [
        GrabStatus::SUCCESS,
        GrabStatus::ALREADY_GRABBED,
        GrabStatus::INVALID_TIME,
        GrabStatus::NOT_VIEWABLE,
        GrabStatus::FROZEN,
    ] {
        let mut runtime = ProbeRuntime::new();
        runtime.body = BodyAvailability::Ready;
        runtime.menu_generation = 2;
        let attempts = Cell::new(0);
        assert_eq!(
            runtime
                .acquire_menu_with(MenuPopup::test_popup(10, 2), Point::new(-188, 60), || {
                    attempts.set(attempts.get() + 1);
                    Ok((status, 70000))
                })
                .unwrap(),
            status
        );
        assert_eq!(attempts.get(), 1);
        if status == GrabStatus::SUCCESS {
            assert!(matches!(
                runtime.interaction.state(),
                InteractionState::MenuOpen { .. }
            ));
            assert_eq!(
                runtime.pointer.owner(),
                Some(CaptureOwner::Menu {
                    window: 10,
                    generation: 2
                })
            );
            assert_eq!(
                runtime.menu.as_ref().unwrap().input.acquisition_sequence,
                70000
            );
            assert!(runtime.safety.deadline.is_none());
        } else {
            assert!(runtime.interaction.is_idle());
            assert_eq!(runtime.pointer.owner(), None);
            let destroys = Cell::new(0);
            runtime
                .cancel_menu_with(
                    &mut tests::test_window(),
                    |pointer| {
                        pointer
                            .release_with(|| panic!("denied grab must not ungrab"))
                            .map(|_| ())
                    },
                    |_| {
                        destroys.set(destroys.get() + 1);
                        Ok(())
                    },
                )
                .unwrap();
            assert_eq!(destroys.get(), 1);
            assert!(runtime.menu.is_none());
            assert!(runtime.accepts_input());
            assert_eq!(attempts.get(), 1);
        }
    }
}

#[test]
fn acquisition_protocol_failure_retains_resources_for_primary_error_cleanup() {
    let mut runtime = ProbeRuntime::new();
    let result = runtime
        .acquire_menu_with(MenuPopup::test_popup(10, 2), Point::new(-188, 60), || {
            Err("grab reply failure".into())
        })
        .map(|_| ());
    assert!(runtime.menu.is_some());
    assert!(runtime.pointer.is_grabbed());
    assert!(runtime.interaction.is_idle());
    let trace = RefCell::new(Vec::new());
    let error = finish_probe_with(result, || {
        runtime.cancel_menu_with(
            &mut tests::test_window(),
            |pointer| {
                pointer
                    .release_with(|| {
                        trace.borrow_mut().push("ungrab");
                        Ok(())
                    })
                    .map(|_| ())
            },
            |_| {
                trace.borrow_mut().push("popup");
                Ok(())
            },
        )
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "grab reply failure");
    assert_eq!(*trace.borrow(), ["ungrab", "popup"]);
    assert!(runtime.menu.is_none());
}

#[test]
fn queued_previous_popup_destruction_cannot_cancel_reused_window_id() {
    let gate = MenuPopup::test_popup(10, 2).input;
    let destroy = Event::DestroyNotify(DestroyNotifyEvent {
        response_type: 17,
        sequence: 0,
        event: 1,
        window: 10,
    });
    assert!(!popup_lifecycle(
        &destroy,
        Some(gate),
        gate.creation_sequence - 1
    ));
    assert!(popup_lifecycle(
        &destroy,
        Some(gate),
        gate.creation_sequence
    ));
    let fabricated = Event::DestroyNotify(DestroyNotifyEvent {
        response_type: 17 | 0x80,
        sequence: 0,
        event: 1,
        window: 10,
    });
    assert!(!popup_lifecycle(
        &fabricated,
        Some(gate),
        gate.creation_sequence
    ));
}

#[test]
fn popup_resources_block_body_input_until_cleanup_succeeds() {
    let mut runtime = captured_menu();
    assert!(!runtime.accepts_input());
    runtime.interaction.cancel(100);
    assert!(!runtime.accepts_input());
    let mut body = tests::test_window();
    assert!(runtime
        .cancel_menu_with(
            &mut body,
            |pointer| pointer.release_with(|| Ok(())).map(|_| ()),
            |_| Err("destroy unacknowledged".into())
        )
        .is_err());
    assert!(!runtime.accepts_input());
    assert!(runtime.menu.is_some());
}

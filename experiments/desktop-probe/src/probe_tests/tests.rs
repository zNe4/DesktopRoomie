use super::*;
use x11rb::protocol::xproto::{
    ButtonReleaseEvent, ConfigureNotifyEvent, KeyButMask, Motion, MotionNotifyEvent,
};

#[test]
fn test_drain_events_bounded_preserves_buffered_event() {
    let mut buffered_event = Some(Event::MotionNotify(MotionNotifyEvent {
        response_type: 6,
        detail: Motion::NORMAL,
        sequence: 1,
        time: 100,
        root: 1,
        event: 10,
        child: 0,
        root_x: 50,
        root_y: 50,
        event_x: 5,
        event_y: 5,
        state: KeyButMask::from(0u16),
        same_screen: true,
    }));

    let mut queue = vec![Event::MotionNotify(MotionNotifyEvent {
        response_type: 6,
        detail: Motion::NORMAL,
        sequence: 2,
        time: 101,
        root: 1,
        event: 10,
        child: 0,
        root_x: 60,
        root_y: 60,
        event_x: 15,
        event_y: 15,
        state: KeyButMask::from(0u16),
        same_screen: true,
    })];

    let batch: Vec<Event> = drain_events_bounded(&mut buffered_event, 64, || {
        Ok::<_, ()>(if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        })
    })
    .unwrap();

    assert_eq!(batch.len(), 2);
    assert!(buffered_event.is_none());
}

#[test]
fn test_buffered_events_exceeding_batch_limit_drains_release_last() {
    // Create 70 events: 69 motions followed by 1 release
    let mut queue = std::collections::VecDeque::new();
    for i in 0..69 {
        queue.push_back(Event::MotionNotify(MotionNotifyEvent {
            response_type: 6,
            detail: Motion::NORMAL,
            sequence: i as u16,
            time: 1000 + i,
            root: 1,
            event: 10,
            child: 0,
            root_x: 100 + i as i16,
            root_y: 100,
            event_x: 10,
            event_y: 10,
            state: KeyButMask::BUTTON1,
            same_screen: true,
        }));
    }
    queue.push_back(Event::ButtonRelease(ButtonReleaseEvent {
        response_type: 5,
        detail: 1,
        sequence: 70,
        time: 1070,
        root: 1,
        event: 10,
        child: 0,
        root_x: 168,
        root_y: 100,
        event_x: 10,
        event_y: 10,
        state: KeyButMask::from(0u16),
        same_screen: true,
    }));

    let mut buffered_event = None;
    let mut processed_events = Vec::new();

    // Simulate event loop across batches without sleeping on socket
    let mut loops = 0;
    while loops < 10 && (!queue.is_empty() || buffered_event.is_some()) {
        loops += 1;
        // 1. Drain batch
        let batch: Vec<Event> =
            drain_events_bounded(&mut buffered_event, 64, || Ok::<_, ()>(queue.pop_front()))
                .unwrap();

        let batch_len = batch.len();
        for ev in batch {
            processed_events.push(ev);
        }

        // Verify batch 1 hit max_batch limit
        if loops == 1 {
            assert_eq!(batch_len, 64);
        }

        // 2. Before blocking, check if more events remain buffered in poll_fn
        if buffered_event.is_none() {
            if let Some(next_ev) = queue.pop_front() {
                buffered_event = Some(next_ev);
                continue;
            }
        }
    }

    assert_eq!(
        loops, 2,
        "Should drain all 70 events in exactly 2 batches without blocking"
    );
    assert_eq!(processed_events.len(), 70);

    // Verify the 70th event processed is the ButtonRelease event
    match &processed_events[69] {
        Event::ButtonRelease(ev) => {
            assert_eq!(ev.detail, 1);
            assert_eq!(ev.root_x, 168);
        }
        other => panic!("Expected ButtonRelease at end of stream, found {:?}", other),
    }
}

#[test]
fn test_confirmation_timeout_with_no_further_events() {
    let mut window = ManagedProbeWindow {
        window_resource: Default::default(),
        colormap_resource: Default::default(),
        window: 100,
        colormap: 1,
        wm_delete_window: 2,
        wm_protocols: 3,
        requested_origin: Point::new(150, 150),
        confirmed_origin: Point::new(100, 100),
        in_flight_moves: std::collections::VecDeque::new(),
        superseded_moves: std::collections::VecDeque::new(),
        width: 160,
        height: 160,
    };
    window.in_flight_moves.push_back(Point::new(150, 150));

    let mut awaiting_final_confirmation = Some(Point::new(150, 150));
    let mut confirmation_deadline = Some(Instant::now() - Duration::from_millis(1));

    // When deadline has expired and no events arrived, checked live query runs.
    assert!(confirmation_deadline.unwrap() <= Instant::now());
    // Discrepancy case: live query observes refused move remaining at (100, 100)
    let live_origin = Point::new(100, 100);
    let rec = window.reconcile_live_snapshot(live_origin, Size::new(160, 160));
    assert_eq!(
        rec,
        ConfigureReconciliation::GenuineMismatch {
            requested: Point::new(150, 150),
            confirmed: Point::new(100, 100),
        }
    );
    confirmation_deadline = None;
    assert_eq!(confirmation_deadline, None);

    // Success case: live query observes target reached at (150, 150)
    window.in_flight_moves.push_back(Point::new(150, 150));
    let rec_ok = window.reconcile_live_snapshot(Point::new(150, 150), Size::new(160, 160));
    assert_eq!(
        rec_ok,
        ConfigureReconciliation::Confirmed {
            origin: Point::new(150, 150),
            size: Size::new(160, 160),
        }
    );
    if let ConfigureReconciliation::Confirmed { origin, .. } = rec_ok {
        if awaiting_final_confirmation == Some(origin) {
            awaiting_final_confirmation = None;
        }
    }
    assert_eq!(awaiting_final_confirmation, None);
}

#[test]
fn test_awaiting_final_confirmation_rejects_mismatched_target() {
    let mut window = ManagedProbeWindow {
        window_resource: Default::default(),
        colormap_resource: Default::default(),
        window: 100,
        colormap: 1,
        wm_delete_window: 2,
        wm_protocols: 3,
        requested_origin: Point::new(200, 200),
        confirmed_origin: Point::new(100, 100),
        in_flight_moves: std::collections::VecDeque::new(),
        superseded_moves: std::collections::VecDeque::new(),
        width: 160,
        height: 160,
    };

    let awaiting_final = Point::new(200, 200);

    // Suppose WM confirms position (180, 200) instead of expected (200, 200)
    let confirmed_event_origin = Point::new(180, 200);
    let rec = window.reconcile_historical_notification(confirmed_event_origin, Size::new(160, 160));

    // It is not accepted as the awaited final target
    assert_ne!(confirmed_event_origin, awaiting_final);
    assert_eq!(
        rec,
        ConfigureReconciliation::GenuineMismatch {
            requested: Point::new(200, 200),
            confirmed: confirmed_event_origin,
        }
    );
}

#[test]
fn test_expired_deadline_with_more_than_64_buffered_unrelated_events() {
    // Create 70 unrelated events (e.g. motion events on an unrelated window)
    let mut queue = std::collections::VecDeque::new();
    for i in 0..70 {
        queue.push_back(Event::MotionNotify(MotionNotifyEvent {
            response_type: 6,
            detail: Motion::NORMAL,
            sequence: i as u16,
            time: 1000 + i,
            root: 1,
            event: 999, // unrelated window
            child: 0,
            root_x: 50,
            root_y: 50,
            event_x: 50,
            event_y: 50,
            state: KeyButMask::from(0u16),
            same_screen: true,
        }));
    }

    let mut window = ManagedProbeWindow {
        window_resource: Default::default(),
        colormap_resource: Default::default(),
        window: 100,
        colormap: 1,
        wm_delete_window: 2,
        wm_protocols: 3,
        requested_origin: Point::new(150, 150),
        confirmed_origin: Point::new(100, 100),
        in_flight_moves: std::collections::VecDeque::new(),
        superseded_moves: std::collections::VecDeque::new(),
        width: 160,
        height: 160,
    };
    window.in_flight_moves.push_back(Point::new(150, 150));

    let mut awaiting_final_confirmation = Some(Point::new(150, 150));
    let mut confirmation_deadline = Some(Instant::now() - Duration::from_millis(10));
    let mut pending_move = None;
    let mut interaction = InteractionManager::new();
    // Simulate dragging state
    let bounds = crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000);
    let _ =
        interaction.handle_left_press(Point::new(100, 100), (20, 20), 1000, Point::new(100, 100));
    interaction.on_grab_acquired();
    let _ = interaction.handle_motion(Point::new(150, 150), 1010, &bounds);
    assert!(interaction.is_dragging());

    let mut grab_released = false;
    let mut buffered_event = None;
    let mut loops = 0;

    // Simulate production event loop structure
    while loops < 10 && (!queue.is_empty() || buffered_event.is_some()) {
        loops += 1;

        // Check confirmation deadline at top of loop alongside duration deadline,
        // BEFORE draining/processing another batch
        let _ = check_confirmation_timeout_with(
            &mut confirmation_deadline,
            &mut window,
            &mut awaiting_final_confirmation,
            &mut interaction,
            &mut pending_move,
            || Ok(Point::new(100, 100)), // Live query observes window remained at 100,100 (refusal)
            |_time| {
                grab_released = true;
                Ok(())
            },
        );

        // 1. Drain batch (64 events)
        let batch =
            drain_events_bounded(&mut buffered_event, 64, || Ok::<_, ()>(queue.pop_front()))
                .unwrap();
        assert!(!batch.is_empty());

        // 2. Before blocking on socket, verify no events remain buffered
        if buffered_event.is_none() {
            if let Some(next_ev) = queue.pop_front() {
                buffered_event = Some(next_ev);
                continue;
            }
        }
    }

    // Crucial assertions:
    // 1. Confirmation deadline was processed on loop 1 BEFORE batch 2 drained, not bypassed by continue
    assert_eq!(confirmation_deadline, None);
    assert!(grab_released);
    assert!(
        !interaction.is_dragging(),
        "Interaction must be cancelled on timeout discrepancy"
    );
    assert!(awaiting_final_confirmation.is_none());
    assert!(window.in_flight_moves.is_empty());
    assert_eq!(loops, 2, "Drained 70 events in exactly 2 batches");
}

#[test]
fn test_two_consecutive_drags_with_delayed_confirmation_of_first() {
    let mut window = ManagedProbeWindow {
        window_resource: Default::default(),
        colormap_resource: Default::default(),
        window: 100,
        colormap: 1,
        wm_delete_window: 2,
        wm_protocols: 3,
        requested_origin: Point::new(80, 80),
        confirmed_origin: Point::new(80, 80),
        in_flight_moves: std::collections::VecDeque::new(),
        superseded_moves: std::collections::VecDeque::new(),
        width: 160,
        height: 160,
    };

    let mut interaction = InteractionManager::new();
    let bounds = crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000);
    let mut awaiting_final_confirmation: Option<Point> = None;
    assert_eq!(awaiting_final_confirmation, None);

    let a = Point::new(100, 80);
    let b = Point::new(120, 80);

    // --- Drag 1 ---
    // Press on body at (100, 100)
    let action1 =
        interaction.handle_left_press(Point::new(100, 100), (20, 20), 1000, Point::new(80, 80));
    assert!(matches!(action1, HostAction::AcquireGrab { .. }));
    interaction.on_grab_acquired();
    awaiting_final_confirmation = None;
    assert_eq!(awaiting_final_confirmation, None);

    // Move to A
    let action_m1 = interaction.handle_motion(Point::new(120, 100), 1010, &bounds);
    assert_eq!(action_m1, HostAction::MoveWindow { target: a });
    window.in_flight_moves.push_back(a);
    window.requested_origin = a;

    // Release Drag 1 at root (120, 100)
    let action_r1 = interaction.handle_left_release(Point::new(120, 100), (20, 20), 1020, &bounds);
    assert_eq!(
        action_r1,
        HostAction::ReleaseGrabAndMoveWindow {
            time: 1020,
            target: a
        }
    );
    awaiting_final_confirmation = Some(a);
    assert_eq!(awaiting_final_confirmation, Some(a));

    // --- Drag 2 (starts before A is confirmed) ---
    // Fresh left press begins gesture 2
    let action2 = interaction.handle_left_press(Point::new(120, 100), (20, 20), 1050, a);
    assert!(matches!(action2, HostAction::AcquireGrab { .. }));
    interaction.on_grab_acquired();
    // Production logic: fresh gesture supersedes awaiting_final_confirmation
    awaiting_final_confirmation = None;
    assert_eq!(awaiting_final_confirmation, None);
    // Tracking of previous in-flight move A is preserved!
    assert_eq!(window.in_flight_moves.len(), 1);
    assert_eq!(window.in_flight_moves[0], a);

    // Drag 2 moves to B
    let action_m2 = interaction.handle_motion(Point::new(140, 100), 1060, &bounds);
    assert_eq!(action_m2, HostAction::MoveWindow { target: b });
    window.in_flight_moves.push_back(b);
    window.requested_origin = b;
    assert_eq!(window.in_flight_moves.len(), 2);

    // Release Drag 2 at root (140, 100)
    let action_r2 = interaction.handle_left_release(Point::new(140, 100), (20, 20), 1070, &bounds);
    assert_eq!(
        action_r2,
        HostAction::ReleaseGrabAndMoveWindow {
            time: 1070,
            target: b
        }
    );
    awaiting_final_confirmation = Some(b);
    assert_eq!(awaiting_final_confirmation, Some(b));

    // --- Delayed confirmation for Drag 1 (A) arrives ---
    let ev_a = ConfigureNotifyEvent {
        response_type: 22 | 0x80, // synthetic event
        sequence: 1,
        event: 100,
        window: 100,
        above_sibling: 0,
        x: a.x as i16,
        y: a.y as i16,
        width: 160,
        height: 160,
        border_width: 0,
        override_redirect: false,
    };
    let rec_a = window
        .handle_configure_notify_with(&ev_a, || unreachable!())
        .unwrap();
    // Should be in-flight catch-up, NOT Confirmed and NOT GenuineMismatch
    assert_eq!(
        rec_a,
        ConfigureReconciliation::InFlightCatchUp {
            confirmed: a,
            remaining_in_flight: 1,
        }
    );
    // Crucial: awaiting_final_confirmation remains Some(b) and Drag 2 is NOT cancelled
    assert_eq!(awaiting_final_confirmation, Some(b));
    assert!(interaction.is_idle()); // Gesture 2 released cleanly into idle

    // --- Confirmation for Drag 2 (B) arrives ---
    let ev_b = ConfigureNotifyEvent {
        response_type: 22 | 0x80,
        sequence: 2,
        event: 100,
        window: 100,
        above_sibling: 0,
        x: b.x as i16,
        y: b.y as i16,
        width: 160,
        height: 160,
        border_width: 0,
        override_redirect: false,
    };
    let rec_b = window
        .handle_configure_notify_with(&ev_b, || unreachable!())
        .unwrap();
    assert_eq!(
        rec_b,
        ConfigureReconciliation::Confirmed {
            origin: b,
            size: Size::new(160, 160),
        }
    );
    if let ConfigureReconciliation::Confirmed { origin, .. } = rec_b {
        if awaiting_final_confirmation == Some(origin) {
            awaiting_final_confirmation = None;
        }
    }
    assert_eq!(awaiting_final_confirmation, None);
    assert_eq!(window.confirmed_origin(), b);
    assert!(window.in_flight_moves.is_empty());
}

#[test]
fn test_expired_deadline_query_buffers_release_processes_before_socket_wait() {
    let mut window = ManagedProbeWindow {
        window_resource: Default::default(),
        colormap_resource: Default::default(),
        window: 100,
        colormap: 1,
        wm_delete_window: 2,
        wm_protocols: 3,
        requested_origin: Point::new(150, 150),
        confirmed_origin: Point::new(100, 100),
        in_flight_moves: std::collections::VecDeque::new(),
        superseded_moves: std::collections::VecDeque::new(),
        width: 160,
        height: 160,
    };
    window.in_flight_moves.push_back(Point::new(150, 150));

    let mut interaction = InteractionManager::new();
    let bounds = crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000);
    let _ =
        interaction.handle_left_press(Point::new(100, 100), (20, 20), 1000, Point::new(100, 100));
    interaction.on_grab_acquired();
    let _ = interaction.handle_motion(Point::new(150, 150), 1010, &bounds);
    assert!(interaction.is_dragging());

    let mut awaiting_final_confirmation = None;
    let mut confirmation_deadline = Some(Instant::now() - Duration::from_millis(10));
    let mut pending_move = None;
    let mut buffered_event: Option<Event> = None;

    // Simulated x11rb internal event queue
    let mut internal_event_queue = std::collections::VecDeque::new();
    let mut socket_wait_invoked = false;
    let mut release_processed = false;

    // Loop iteration 1:
    // Batch draining finds no events in queue initially
    let batch: Vec<Event> = drain_events_bounded(&mut buffered_event, 64, || {
        Ok::<_, ()>(internal_event_queue.pop_front())
    })
    .unwrap();
    assert!(batch.is_empty());

    // Now post-batch handling in loop runs:
    // 1. All X11 operations executed BEFORE final empty-event check:
    // Check confirmation timeout: query runs and buffers an incoming ButtonRelease
    let query_buffers_release = || {
        // When query executes synchronous round-trip (.reply()), server event arrives on socket
        internal_event_queue.push_back(Event::ButtonRelease(ButtonReleaseEvent {
            response_type: 5,
            detail: 1,
            sequence: 10,
            time: 1050,
            root: 1,
            event: 100,
            child: 0,
            root_x: 150,
            root_y: 150,
            event_x: 20,
            event_y: 20,
            state: KeyButMask::BUTTON1,
            same_screen: true,
        }));
        Ok(Point::new(100, 100)) // Live query observes discrepancy
    };

    check_confirmation_timeout_with(
        &mut confirmation_deadline,
        &mut window,
        &mut awaiting_final_confirmation,
        &mut interaction,
        &mut pending_move,
        query_buffers_release,
        |_time| Ok(()),
    )
    .unwrap();

    // 2. Final empty-event check BEFORE waiting on socket:
    let should_continue = if buffered_event.is_some() {
        true
    } else {
        match internal_event_queue.pop_front() {
            Some(ev) => {
                buffered_event = Some(ev);
                true
            }
            None => false,
        }
    };

    if !should_continue {
        socket_wait_invoked = true;
    }

    // Crucial: socket wait was NOT invoked because the buffered release was detected!
    assert!(
        !socket_wait_invoked,
        "Socket wait must not be invoked when release was buffered"
    );
    assert!(should_continue);
    assert!(buffered_event.is_some());

    // Loop iteration 2:
    // Drain batch picks up the buffered ButtonRelease event immediately
    let batch2: Vec<Event> = drain_events_bounded(&mut buffered_event, 64, || {
        Ok::<_, ()>(internal_event_queue.pop_front())
    })
    .unwrap();
    assert_eq!(batch2.len(), 1);

    for ev in batch2 {
        if let Event::ButtonRelease(rel) = ev {
            assert_eq!(rel.detail, 1);
            let _ = interaction.handle_left_release(
                Point::new(rel.root_x as i32, rel.root_y as i32),
                (rel.event_x, rel.event_y),
                rel.time,
                &bounds,
            );
            release_processed = true;
        }
    }

    assert!(
        release_processed,
        "Buffered release must be processed before any socket wait"
    );
    assert!(interaction.is_idle());
}
pub(super) fn test_window() -> ManagedProbeWindow {
    ManagedProbeWindow {
        window_resource: Default::default(),
        colormap_resource: Default::default(),
        window: 100,
        colormap: 1,
        wm_delete_window: 2,
        wm_protocols: 3,
        requested_origin: Point::new(100, 100),
        confirmed_origin: Point::new(100, 100),
        in_flight_moves: Default::default(),
        superseded_moves: Default::default(),
        width: 160,
        height: 160,
    }
}
fn captured_runtime() -> ProbeRuntime {
    let mut runtime = ProbeRuntime::new();
    runtime.body = BodyAvailability::Ready;
    runtime.interaction.handle_left_press(
        Point::new(180, 180),
        (80, 80),
        1000,
        Point::new(100, 100),
    );
    runtime.interaction.on_grab_acquired();
    runtime.pointer.set_grabbed(true);
    runtime.safety.acquired(Instant::now());
    runtime
}

#[test]
fn cancellation_invalidates_movement_before_failed_release_and_cleanup_retries() {
    let mut runtime = captured_runtime();
    let mut window = test_window();
    runtime.pending_move = Some(Point::new(200, 200));
    runtime.final_target = Some(Point::new(220, 220));
    runtime.confirmation_deadline = Some(Instant::now());
    window.in_flight_moves.push_back(Point::new(190, 190));
    assert!(runtime
        .cancel_with(&mut window, |pointer| pointer
            .release_with(|| Err("injected ungrab failure".into()))
            .map(|_| ()))
        .is_err());
    assert!(runtime.pending_move.is_none());
    assert!(runtime.final_target.is_none());
    assert!(runtime.confirmation_deadline.is_none());
    assert!(runtime.safety.deadline.is_none());
    assert!(window.in_flight_moves.is_empty());
    assert!(window.superseded_moves.contains(&Point::new(190, 190)));
    assert!(runtime.pointer.is_grabbed());
    assert_eq!(
        runtime.interaction.handle_left_release(
            Point::new(180, 180),
            (80, 80),
            1010,
            &crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000)
        ),
        HostAction::None
    );
    runtime
        .cancel_with(&mut window, |pointer| {
            pointer.release_with(|| Ok(())).map(|_| ())
        })
        .unwrap();
    assert!(!runtime.pointer.is_grabbed());
}

#[test]
fn stationary_hold_rearms_safety_indefinitely_and_release_cancels_without_position() {
    let mut runtime = captured_runtime();
    let mut now = Instant::now();
    for sequence in 1..1000 {
        now += POINTER_CHECK_INTERVAL;
        runtime.safety.observed(true, sequence);
        assert!(!runtime.apply_safety(now, None));
        assert_eq!(runtime.safety.deadline, Some(now + POINTER_CHECK_INTERVAL));
        assert!(runtime.interaction.has_left_gesture());
    }
    runtime.safety.observed(false, 1000);
    assert!(runtime.apply_safety(now, None));
    let mut window = test_window();
    runtime
        .cancel_with(&mut window, |pointer| {
            pointer.release_with(|| Ok(())).map(|_| ())
        })
        .unwrap();
    assert!(runtime.pending_move.is_none());
    assert!(runtime.final_target.is_none());
}

#[test]
fn safety_observation_waits_for_pre_query_release_but_not_later_events() {
    let mut runtime = captured_runtime();
    let now = Instant::now();
    runtime.safety.observed(false, 70000); // extended sequence, beyond 16-bit wrap
    assert!(!runtime.apply_safety(now, Some(69999)));
    let action = runtime.interaction.handle_left_release(
        Point::new(180, 180),
        (80, 80),
        1010,
        &crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000),
    );
    assert_eq!(action, HostAction::ReleaseGrabAndToggleColor { time: 1010 });
    assert!(!runtime.apply_safety(now, Some(70000)));
    assert!(runtime.safety.observation.is_none());

    let mut runtime = captured_runtime();
    runtime.safety.observed(false, 70000);
    assert!(runtime.apply_safety(now, Some(70000)));
    assert!(runtime.safety.deadline.is_none());
}

#[test]
fn buffered_new_press_invalidates_prior_safety_observation() {
    let mut runtime = captured_runtime();
    let now = Instant::now();
    runtime.safety.observed(false, 900);
    // The queued release and fresh press both preceded the query response boundary.
    runtime.interaction.handle_left_release(
        Point::new(180, 180),
        (80, 80),
        1010,
        &crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000),
    );
    assert!(!runtime.apply_safety(now, Some(899)));
    runtime.interaction.handle_left_press(
        Point::new(180, 180),
        (80, 80),
        1020,
        Point::new(100, 100),
    );
    runtime.interaction.on_grab_acquired();
    runtime.safety.acquired(now);
    assert!(!runtime.apply_safety(now, Some(900)));
    assert!(runtime.safety.deadline.is_some());
    assert!(runtime.interaction.has_left_gesture());
}

#[test]
fn interruption_discards_coalesced_move_and_release_preserves_flush_order() {
    use x11rb::protocol::xproto::{ButtonPressEvent, UnmapNotifyEvent, UNMAP_NOTIFY_EVENT};
    let atoms = x11::monitors::LayoutAtoms {
        number_of_desktops: 90,
        desktop_viewport: 91,
        workarea: 3,
        current_desktop: 4,
        wm_desktop: 5,
    };
    let release = Event::ButtonRelease(ButtonReleaseEvent {
        response_type: 5,
        detail: 1,
        sequence: 1,
        time: 1010,
        root: 1,
        event: 100,
        child: 0,
        root_x: 200,
        root_y: 200,
        event_x: 80,
        event_y: 80,
        state: KeyButMask::BUTTON1,
        same_screen: true,
    });
    assert!(!interrupts_pending_movement(&release, 1, 100, &atoms));
    let unmap = Event::UnmapNotify(UnmapNotifyEvent {
        response_type: UNMAP_NOTIFY_EVENT,
        sequence: 1,
        event: 100,
        window: 100,
        from_configure: false,
    });
    assert!(interrupts_pending_movement(&unmap, 1, 100, &atoms));
    let right = Event::ButtonPress(ButtonPressEvent {
        detail: 3,
        ..match release {
            Event::ButtonRelease(ev) => ev,
            _ => unreachable!(),
        }
    });
    assert!(interrupts_pending_movement(&right, 1, 100, &atoms));
    let mut runtime = captured_runtime();
    runtime.pending_move = Some(Point::new(200, 200));
    let mut window = test_window();
    runtime
        .cancel_with(&mut window, |pointer| {
            pointer.release_with(|| Ok(())).map(|_| ())
        })
        .unwrap();
    assert!(runtime.pending_move.is_none());
}

#[test]
fn unavailable_body_defers_placement_and_verified_remap_requires_correction_first() {
    let mut runtime = ProbeRuntime::new();
    let mut window = test_window();
    let bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50);
    let actual = crate::geometry::Rect::new(100, 100, 160, 160);
    runtime.unavailable(false);
    runtime
        .placement_with(&mut window, actual, &bounds, |_, _| {
            panic!("unavailable body must not move")
        })
        .unwrap();
    assert!(!runtime.accepts_input());
    runtime.body = BodyAvailability::Validating; // production verified map gate
    let calls = std::cell::Cell::new(0);
    runtime
        .placement_with(&mut window, actual, &bounds, |_, target| {
            assert_eq!(target, Point::new(50, 50));
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
    assert!(!runtime.accepts_input());
    runtime
        .placement_with(&mut window, actual, &bounds, |_, _| {
            panic!("must verify before requesting again")
        })
        .unwrap();
    assert_eq!(calls.get(), 1);
    let deadline = runtime.correction.unwrap().1;
    assert!(!runtime
        .check_correction(
            deadline - Duration::from_millis(1),
            Point::new(100, 100),
            &bounds
        )
        .unwrap());
    assert!(runtime
        .check_correction(deadline, Point::new(50, 50), &bounds)
        .unwrap());
    assert!(runtime.accepts_input());
    runtime.unavailable(true);
    runtime
        .placement_with(&mut window, actual, &bounds, |_, _| {
            panic!("destroyed body must not move")
        })
        .unwrap();
    assert!(!runtime.accepts_input());
}

#[test]
fn refused_correction_fails_without_retry_and_movement_failure_still_cleans_up() {
    let mut runtime = captured_runtime();
    let mut window = test_window();
    let bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50);
    let deadline = Instant::now();
    runtime.correction = Some((Point::new(50, 50), deadline));
    let result = runtime
        .check_correction(deadline, Point::new(100, 100), &bounds)
        .map(|_| ());
    let calls = std::cell::Cell::new(0);
    let error = finish_probe_with(result, || {
        runtime.cancel_with(&mut window, |pointer| {
            pointer
                .release_with(|| {
                    calls.set(1);
                    Ok(())
                })
                .map(|_| ())
        })?;
        x11::resource::cleanup_all([Box::new(|| {
            assert_eq!(calls.get(), 1);
            calls.set(2);
            Err("injected cleanup failure".into())
        })])
    })
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("WM refused necessary bounds correction"));
    assert_eq!(calls.get(), 2);
    assert!(!runtime.pointer.is_grabbed());

    runtime.body = BodyAvailability::Validating;
    let error = runtime
        .placement_with(
            &mut window,
            crate::geometry::Rect::new(100, 100, 160, 160),
            &bounds,
            |_, _| Err("injected movement failure".into()),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "injected movement failure");
    assert!(runtime.correction.is_none());
}

#[test]
fn workspace_workarea_and_root_geometry_are_relevant_but_unrelated_properties_are_not() {
    use x11rb::protocol::xproto::{Property, PropertyNotifyEvent};
    let atoms = x11::monitors::LayoutAtoms {
        number_of_desktops: 90,
        desktop_viewport: 91,
        workarea: 3,
        current_desktop: 4,
        wm_desktop: 5,
    };
    let property = |window, atom| {
        Event::PropertyNotify(PropertyNotifyEvent {
            response_type: 28,
            sequence: 1,
            window,
            atom,
            time: 1000,
            state: Property::NEW_VALUE,
        })
    };
    for event in [property(1, 3), property(1, 4), property(100, 5)] {
        assert!(relevant_layout_event(&event, 1, 100, &atoms));
        assert!(interrupts_pending_movement(&event, 1, 100, &atoms));
    }
    assert!(!relevant_layout_event(&property(100, 99), 1, 100, &atoms));
    assert!(!relevant_layout_event(&property(999, 3), 1, 100, &atoms));
    for atom in [atoms.number_of_desktops, atoms.desktop_viewport] {
        let event = property(1, atom);
        assert!(!relevant_layout_event(&event, 1, 100, &atoms));
        assert!(!interrupts_pending_movement(&event, 1, 100, &atoms));
        assert!(!interrupts_layer(&event, 1, 100, &atoms));
        assert!(!visibility_layout_interruption(&event, 1, &atoms));
    }
}
#[test]
fn safety_boundary_survives_multiple_batches_without_swallowing_ordinary_release() {
    let mut runtime = captured_runtime();
    let now = Instant::now();
    runtime.safety.observed(false, 70000);
    let mut queue = std::collections::VecDeque::new();
    for _ in 0..69 {
        queue.push_back((None, 69999));
    }
    queue.push_back((Some(1010), 69999)); // ordinary release buffered by QueryPointer
    let mut buffered = None;
    let mut completions = 0;
    for batch_index in 0..2 {
        let batch =
            drain_events_bounded(&mut buffered, 64, || Ok::<_, ()>(queue.pop_front())).unwrap();
        if batch_index == 0 {
            assert_eq!(batch.len(), 64);
        }
        for (release, sequence) in batch {
            assert!(!runtime.apply_safety(now, Some(sequence)));
            if let Some(time) = release {
                assert_eq!(
                    runtime.interaction.handle_left_release(
                        Point::new(180, 180),
                        (80, 80),
                        time,
                        &crate::geometry::ValidOriginBounds::new(0, 1000, 0, 1000)
                    ),
                    HostAction::ReleaseGrabAndToggleColor { time }
                );
                completions += 1;
            }
        }
    }
    assert!(!runtime.apply_safety(now, None));
    assert_eq!(completions, 1);
    assert!(runtime.safety.deadline.is_none());
}

#[test]
fn failed_corrective_move_preserves_primary_error_and_attempts_release_before_destruction() {
    let mut runtime = captured_runtime();
    let mut window = test_window();
    runtime.body = BodyAvailability::Validating;
    let bounds = crate::geometry::ValidOriginBounds::new(0, 50, 0, 50);
    let result = runtime.placement_with(
        &mut window,
        crate::geometry::Rect::new(100, 100, 160, 160),
        &bounds,
        |_, _| Err("injected move failure".into()),
    );
    let trace = std::cell::RefCell::new(Vec::new());
    let error = finish_probe_with(result, || {
        let cancellation = runtime.cancel_with(&mut window, |pointer| {
            pointer
                .release_with(|| {
                    trace.borrow_mut().push("ungrab");
                    Err("injected release failure".into())
                })
                .map(|_| ())
        });
        x11::resource::cleanup_all([
            Box::new(|| cancellation),
            Box::new(|| {
                trace.borrow_mut().push("renderer");
                Ok(())
            }),
            Box::new(|| {
                trace.borrow_mut().push("window");
                Ok(())
            }),
        ])
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "injected move failure");
    assert_eq!(*trace.borrow(), ["ungrab", "renderer", "window"]);
    assert!(runtime.pointer.is_grabbed()); // explicit failed release obligation
    assert!(runtime.pending_move.is_none());
    assert!(runtime.final_target.is_none());
}

#[test]
fn unsupported_x11_coordinate_range_is_rejected_before_placement() {
    assert!(checked_probe_bounds(crate::geometry::Rect::new(32700, 0, 1920, 1080)).is_err());
    assert!(checked_probe_bounds(crate::geometry::Rect::new(-40000, 0, 1920, 1080)).is_err());
    assert!(checked_probe_bounds(crate::geometry::Rect::new(-1920, 0, 1920, 1080)).is_ok());
}

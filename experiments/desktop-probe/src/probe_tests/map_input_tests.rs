use super::*;

const BODY: u32 = 100;
const MAP: u64 = 2 * 65536 + 100;
const SHAPE: u64 = MAP + 20;

fn mapped() -> MapNotifyEvent {
    MapNotifyEvent {
        response_type: x11rb::protocol::xproto::MAP_NOTIFY_EVENT,
        sequence: MAP as u16,
        event: BODY,
        window: BODY,
        override_redirect: false,
    }
}

#[test]
fn one_map_synchronizes_once_and_retains_full_shape_sequence_without_readiness() {
    let mut runtime = ProbeRuntime::new();
    runtime.arm_map_input_sync(MAP);
    runtime
        .synchronize_map_input_with(&mapped(), BODY, MAP, |_| Ok(SHAPE))
        .unwrap();
    assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE));
    assert_eq!(runtime.body, BodyAvailability::AwaitingMap);
    assert!(!runtime.accepts_input());
    for sequence in MAP..MAP + 1000 {
        runtime
            .synchronize_map_input_with(&mapped(), BODY, sequence, |_| panic!("duplicate"))
            .unwrap();
    }
    assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE));
}

#[test]
fn unrelated_synthetic_and_stale_maps_do_not_consume_sync_allowance() {
    let mut unrelated = mapped();
    unrelated.window += 1;
    let mut parent = mapped();
    parent.event += 1;
    let mut synthetic = mapped();
    synthetic.response_type |= 0x80;
    let mut unmanaged = mapped();
    unmanaged.override_redirect = true;
    for (event, sequence) in [
        (unrelated, MAP),
        (parent, MAP),
        (synthetic, MAP),
        (unmanaged, MAP),
        (mapped(), MAP - 1),
        (mapped(), MAP - 65536),
        (mapped(), MAP - 2 * 65536),
    ] {
        let mut runtime = ProbeRuntime::new();
        runtime.arm_map_input_sync(MAP);
        runtime
            .synchronize_map_input_with(&event, BODY, sequence, |_| panic!("invalid map"))
            .unwrap();
        assert_eq!(runtime.pending_map_input, Some(MAP));
        assert_eq!(runtime.input_fence.enabled_after, None);
        runtime
            .synchronize_map_input_with(&mapped(), BODY, MAP, |_| Ok(SHAPE))
            .unwrap();
        assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE));
    }
}

#[test]
fn failed_map_shape_check_is_fatal_unsynchronized_and_not_retried() {
    let mut runtime = ProbeRuntime::new();
    runtime.arm_map_input_sync(MAP);
    let error = runtime
        .synchronize_map_input_with(&mapped(), BODY, MAP, |_| Err("SHAPE check failed".into()))
        .unwrap_err();
    assert_eq!(error.to_string(), "SHAPE check failed");
    assert_eq!(runtime.input_fence.enabled_after, None);
    assert_eq!(runtime.pending_map_input, None);
    assert!(!runtime.accepts_input());
    runtime
        .synchronize_map_input_with(&mapped(), BODY, MAP + 1, |_| panic!("blind retry"))
        .unwrap();
}

#[test]
fn future_remap_needs_explicit_new_request_boundary_and_rejects_old_maps() {
    let mut runtime = ProbeRuntime::new();
    runtime
        .synchronize_map_input_with(&mapped(), BODY, MAP, |_| panic!("not armed"))
        .unwrap();
    runtime.arm_map_input_sync(MAP);
    runtime
        .synchronize_map_input_with(&mapped(), BODY, MAP, |_| Ok(SHAPE))
        .unwrap();
    runtime.unavailable(false);
    runtime
        .synchronize_map_input_with(&mapped(), BODY, SHAPE, |_| panic!("not rearmed"))
        .unwrap();
    let remap = MAP + 65536;
    runtime.arm_map_input_sync(remap);
    runtime
        .synchronize_map_input_with(&mapped(), BODY, MAP, |_| panic!("previous mapping"))
        .unwrap();
    assert_eq!(runtime.pending_map_input, Some(remap));
    assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE));
    runtime
        .synchronize_map_input_with(&mapped(), BODY, remap, |_| Ok(remap + 20))
        .unwrap();
    assert_eq!(runtime.input_fence.enabled_after, Some(remap + 20));
    assert_eq!(runtime.body, BodyAvailability::Unavailable);
    runtime
        .synchronize_map_input_with(&mapped(), BODY, remap + 21, |_| panic!("duplicate remap"))
        .unwrap();
}

#[test]
fn post_map_sync_cannot_bypass_visibility_fence_or_placement_safety() {
    for availability in [
        BodyAvailability::AwaitingMap,
        BodyAvailability::Validating,
        BodyAvailability::Ready,
    ] {
        let mut runtime = ProbeRuntime::new();
        let mut window = tests::test_window();
        runtime.arm_map_input_sync(MAP);
        runtime
            .fence_visibility_input_with(&mut window, || Ok(MAP), |_, _| Ok(()), |_| Ok(()))
            .unwrap();
        runtime.body = availability;
        runtime
            .synchronize_map_input_with(&mapped(), BODY, MAP, |_| panic!("input fenced"))
            .unwrap();
        assert!(runtime.input_fence.blocked);
        assert!(runtime.input_fence.empty_confirmed);
        assert_eq!(runtime.input_fence.enabled_after, None);
        assert!(!runtime.accepts_input());
    }
}

#[test]
fn startup_shares_committed_coverage_only_after_placement_and_settled_obligations() {
    let mut runtime = ProbeRuntime::new();
    let window = tests::test_window();
    runtime.arm_map_input_sync(MAP);
    assert_eq!(runtime.frames.initial_extent().size(), Size::new(160, 160));
    assert!(runtime.committed_size().is_err());
    assert!(!runtime.body_hit(Point::new(80, 80)));
    runtime.frames.commit_fixture(None);
    for body in [
        BodyAvailability::AwaitingMap,
        BodyAvailability::Validating,
        BodyAvailability::Unavailable,
    ] {
        runtime.body = body;
        runtime
            .synchronize_placed_map_input_with(&mapped(), &window, MAP, |_| {
                panic!("body not placed")
            })
            .unwrap();
    }
    runtime.body = BodyAvailability::Ready;
    runtime.pointer.require_transition_release();
    runtime
        .synchronize_placed_map_input_with(&mapped(), &window, MAP, |_| panic!("release debt"))
        .unwrap();
    assert!(!runtime.accepts_input());
    runtime.pointer.release_with(|| Ok(())).unwrap();
    runtime
        .synchronize_placed_map_input_with(&mapped(), &window, MAP - 1, |_| panic!("stale map"))
        .unwrap();
    runtime
        .synchronize_placed_map_input_with(&mapped(), &window, MAP, |region| {
            assert_eq!(region.extent().size(), Size::new(160, 160));
            assert!(region.contains(Point::new(80, 80)));
            assert!(!region.contains(Point::new(0, 0)));
            Ok(SHAPE)
        })
        .unwrap();
    assert!(runtime.body_hit(Point::new(80, 80)));
    let event = Event::ButtonPress(x11rb::protocol::xproto::ButtonPressEvent {
        event: BODY,
        ..Default::default()
    });
    assert!(runtime.rejects_body_event(&event, BODY, SHAPE - 1));
    assert!(!runtime.rejects_body_event(&event, BODY, SHAPE));
    runtime
        .synchronize_placed_map_input_with(&mapped(), &window, MAP + 1, |_| panic!("duplicate"))
        .unwrap();
}

#[test]
fn provisional_or_invalidated_placement_cannot_send_a_post_map_shape() {
    for destroyed in [false, true] {
        let mut runtime = ProbeRuntime::new();
        let window = tests::test_window();
        if destroyed {
            runtime.frames.commit_fixture(None);
            runtime.unavailable(true);
        }
        runtime.body = BodyAvailability::Ready;
        runtime.arm_map_input_sync(MAP);
        let result = runtime.synchronize_placed_map_input_with(&mapped(), &window, MAP, |_| {
            panic!("no committed authority")
        });
        // Destruction consumes the allowance and fences input. Without a
        // first publication, a synchronization attempt fails closed.
        assert_eq!(result.is_err(), !destroyed);
        assert_eq!(runtime.input_fence.enabled_after, None);
        assert!(!runtime.accepts_input());
    }
}

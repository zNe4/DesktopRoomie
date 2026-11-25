use super::*;
use crate::presentation::{
    Action, Facing, FrameIdentity, FrameMetadata, HitRegion, Point, PresentationCapabilities,
    RowSpan, Size,
};
use crate::x11::control::Acquisition;
use crate::x11::menu::MenuPopup;
use crate::x11::render::{checked_blit_with, paint_committed_with};
use crate::{owner_presentation_startup_with, BodyAvailability, ProbeRuntime};
use std::cell::RefCell;
use x11rb::protocol::xproto::{MapNotifyEvent, MAP_NOTIFY_EVENT};

const PERIOD: Duration = Duration::from_millis(10);
const INTENT: ActionIntent = ActionIntent {
    action: Action::Idle,
    facing: Facing::Neutral,
};

#[test]
fn split_seam_preserves_clock_order_and_skips_completion_clock_on_preparation_error() {
    let mut presentation = BodyPresentation::new(Synthetic::new(Some(PERIOD))).unwrap();
    let mut runtime =
        ProbeRuntime::with_presentation(presentation.initial_extent(), presentation.timing());
    let trace = RefCell::new(Vec::new());
    let start = Instant::now();
    let mut readings = [start, start + PERIOD].into_iter();
    runtime
        .present_frame_with(
            &mut presentation,
            FrameKind::Initial,
            true,
            || {
                if trace.borrow().is_empty() {
                    trace.borrow_mut().push("request clock");
                } else {
                    assert_eq!(
                        *trace.borrow(),
                        ["request clock", "upload", "blit", "commit"]
                    );
                    trace.borrow_mut().push("completion clock");
                }
                readings.next().expect("exactly two clock reads")
            },
            |frames, request, frame, _, _| {
                frames.publish_with(
                    request,
                    frame,
                    |_, _| {
                        trace.borrow_mut().push("upload");
                        Ok(())
                    },
                    |_| {
                        trace.borrow_mut().push("blit");
                        Ok(())
                    },
                )?;
                assert_eq!(frames.require_committed()?.identity, request.identity());
                trace.borrow_mut().push("commit");
                Ok(None)
            },
            |_| panic!("successful publication must not fence"),
        )
        .unwrap();
    assert_eq!(
        *trace.borrow(),
        [
            "request clock",
            "upload",
            "blit",
            "commit",
            "completion clock"
        ]
    );
    assert_eq!(runtime.frame_timing.deadline(), Some(start + PERIOD * 2));
    presentation.producer.fail = true;
    let mut reads = 0;
    assert!(runtime
        .present_frame_with(
            &mut presentation,
            FrameKind::Redraw,
            true,
            || {
                reads += 1;
                assert_eq!(reads, 1);
                start + PERIOD * 2
            },
            |_, _, _, _, _| panic!("preparation failed"),
            |_| panic!("trustworthy commit must remain usable"),
        )
        .is_err());
    assert_eq!(reads, 1);
    assert!(runtime.frame_timing.deadline().is_none());
    assert!(runtime.frames.committed().is_some());
    assert!(presentation
        .frame_request(
            FrameKind::Redraw,
            &runtime.frames,
            &mut runtime.frame_timing,
            || panic!("stopped check precedes request clock"),
        )
        .is_err());
}

struct Synthetic {
    caps: PresentationCapabilities,
    requests: Vec<FrameRequest>,
    fail: bool,
    truncate_payload: bool,
}

impl Synthetic {
    fn new(period: Option<Duration>) -> Self {
        Self {
            caps: PresentationCapabilities::new(
                ClientExtent::new(Size::new(3, 2)).unwrap(),
                vec![INTENT],
                period,
            )
            .unwrap(),
            requests: Vec::new(),
            fail: false,
            truncate_payload: false,
        }
    }
}

impl FrameProducer for Synthetic {
    type Payload = Vec<u8>;
    type Error = HostError;

    fn capabilities(&self) -> &PresentationCapabilities {
        &self.caps
    }

    fn prepare_frame(
        &mut self,
        request: &FrameRequest,
    ) -> Result<PreparedFrame<Vec<u8>>, HostError> {
        self.requests.push(*request);
        if self.fail {
            return Err("injected preparation failure".into());
        }
        let extent = request.extent();
        let region = HitRegion::new(
            extent,
            [
                RowSpan {
                    y: 0,
                    start_x: 0,
                    end_x: 1,
                },
                RowSpan {
                    y: 0,
                    start_x: 2,
                    end_x: 3,
                },
                RowSpan {
                    y: 1,
                    start_x: 1,
                    end_x: 3,
                },
            ],
        )
        .unwrap();
        let mut payload = vec![0; 24];
        for index in [0, 2, 4, 5] {
            payload[index * 4..index * 4 + 4].copy_from_slice(&[42, 64, 128, 128]);
        }
        if self.truncate_payload {
            payload.pop();
        }
        Ok(PreparedFrame {
            identity: request.identity(),
            metadata: FrameMetadata::new(extent, None, None, region).unwrap(),
            payload,
        })
    }
}

fn ready(runtime: &mut ProbeRuntime) {
    runtime.body = BodyAvailability::Ready;
    runtime.input_fence.enabled_after = Some(20);
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn present(
    presentation: &mut BodyPresentation<Synthetic>,
    runtime: &mut ProbeRuntime,
    kind: FrameKind,
    started: Instant,
    completed: Instant,
    eligible: bool,
) -> Result<(), HostError> {
    let mut readings = [started, completed].into_iter();
    presentation.present_with(
        kind,
        &mut runtime.frames,
        &mut runtime.frame_timing,
        eligible,
        || {
            readings
                .next()
                .expect("at most two clock reads per publication")
        },
        |frames, request, frame| frames.publish_with(request, frame, |_, _| Ok(()), |_| Ok(())),
    )
}

fn session(period: Option<Duration>, now: Instant) -> (BodyPresentation<Synthetic>, ProbeRuntime) {
    let mut presentation = BodyPresentation::new(Synthetic::new(period)).unwrap();
    let mut runtime =
        ProbeRuntime::with_presentation(presentation.initial_extent(), presentation.timing());
    present(
        &mut presentation,
        &mut runtime,
        FrameKind::Initial,
        now,
        now,
        false,
    )
    .unwrap();
    (presentation, runtime)
}

#[test]
fn producer_factory_runs_once_after_owned_delay_and_before_body_creation() {
    let trace = RefCell::new(Vec::new());
    let outcome = owner_presentation_startup_with(
        &mut (),
        |_| {
            trace.borrow_mut().push("acquired");
            Ok(Acquisition::Owned)
        },
        |_| {
            assert_eq!(*trace.borrow(), ["acquired"]);
            trace.borrow_mut().push("delay settled");
            Ok(())
        },
        || {
            assert_eq!(*trace.borrow(), ["acquired", "delay settled"]);
            trace.borrow_mut().push("factory");
            Ok(Synthetic::new(Some(PERIOD)))
        },
        |_, presentation| {
            assert_eq!(*trace.borrow(), ["acquired", "delay settled", "factory"]);
            assert_eq!(presentation.initial_extent().size(), Size::new(3, 2));
            trace.borrow_mut().push("body");
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(outcome, Acquisition::Owned);
    assert_eq!(
        trace.into_inner(),
        ["acquired", "delay settled", "factory", "body"]
    );
}

#[test]
fn duplicate_acquisition_error_and_delay_error_skip_the_producer_factory() {
    for (acquisition, delay, expected) in [
        (Ok(Acquisition::Duplicate), false, None),
        (Err("acquisition failed"), false, Some("acquisition failed")),
        (Ok(Acquisition::Owned), true, Some("delay failed")),
    ] {
        let result = owner_presentation_startup_with(
            &mut (),
            |_| acquisition.map_err(Into::into),
            |_| {
                assert!(delay, "only the confirmed owner may enter delay");
                Err("delay failed".into())
            },
            || -> Result<Synthetic, HostError> { panic!("factory before acquisition and delay") },
            |_, _| panic!("body creation before acquisition and delay"),
        );
        if let Some(expected) = expected {
            assert_eq!(result.unwrap_err().to_string(), expected);
        } else {
            assert_eq!(result.unwrap(), Acquisition::Duplicate);
        }
    }
}

#[test]
fn failed_factory_skips_body_and_retains_error_through_cleanup() {
    for empty_intents in [false, true] {
        let error = crate::finish_probe_with(
            owner_presentation_startup_with(
                &mut (),
                |_| Ok(Acquisition::Owned),
                |_| Ok(()),
                || {
                    if empty_intents {
                        let mut producer = Synthetic::new(None);
                        producer.caps = PresentationCapabilities::new(
                            producer.caps.initial_extent(),
                            Vec::new(),
                            None,
                        )
                        .unwrap();
                        Ok(producer)
                    } else {
                        Err("factory failed".into())
                    }
                },
                |_, _| panic!("bad factory must not allocate a body"),
            )
            .map(|_| ()),
            || Err("secondary cleanup error".into()),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            if empty_intents {
                "Body producer supports no presentation intents"
            } else {
                "factory failed"
            }
        );
    }
}

#[test]
fn persistent_procedural_producer_publishes_lavender_coral_lavender() {
    let mut presentation =
        BodyPresentation::new(ProceduralProducer::new(Palette::default())).unwrap();
    let mut frames = FrameGeometry::new(presentation.initial_extent());
    let mut timing = presentation.timing();
    let now = Instant::now();
    for (sequence, color) in [
        [225, 95, 165, 255],
        [80, 110, 240, 255],
        [225, 95, 165, 255],
    ]
    .into_iter()
    .enumerate()
    {
        let kind = if sequence == 0 {
            FrameKind::Initial
        } else {
            assert_eq!(
                presentation.toggle_palette(),
                if sequence == 1 {
                    Palette::Coral
                } else {
                    Palette::Lavender
                }
            );
            FrameKind::Redraw
        };
        presentation
            .present_with(
                kind,
                &mut frames,
                &mut timing,
                true,
                || now,
                |frames, request, frame| {
                    assert_eq!(
                        request.identity(),
                        FrameIdentity {
                            generation: 0,
                            sequence: sequence as u64
                        }
                    );
                    frames.publish_with(
                        request,
                        frame,
                        |_, bytes| {
                            let center = (80 * 160 + 80) * 4;
                            assert_eq!(bytes[center..center + 4], color);
                            Ok(())
                        },
                        |_| Ok(()),
                    )
                },
            )
            .unwrap();
    }
    assert_eq!(frames.require_committed().unwrap().identity.sequence, 2);
    assert_eq!(timing.deadline(), None);
}

#[test]
fn initial_request_has_exact_identity_extent_intent_and_zero_timing() {
    let now = Instant::now();
    let (presentation, runtime) = session(Some(PERIOD), now);
    assert_eq!(
        presentation.producer.requests,
        [FrameRequest::new(
            FrameIdentity {
                generation: 0,
                sequence: 0
            },
            INTENT,
            presentation.initial_extent(),
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap()]
    );
    assert_eq!(
        runtime
            .frames
            .require_committed()
            .unwrap()
            .identity
            .sequence,
        0
    );
    assert_eq!(
        runtime.frame_timing.deadline(),
        None,
        "initial shape and placement are pending"
    );
    assert!(!runtime.body_input_ready());
}

#[test]
fn no_frame_period_never_arms_even_when_visible_or_redrawn() {
    let start = Instant::now();
    let (mut presentation, mut runtime) = session(None, start);
    let window = crate::tests::test_window();
    ready(&mut runtime);
    runtime
        .refresh_frame_eligibility(start, &window, false)
        .unwrap();
    let later = start + ms(50_000);
    assert!(!runtime
        .frame_tick_ready(later, &window, false, None)
        .unwrap());
    present(
        &mut presentation,
        &mut runtime,
        FrameKind::Redraw,
        later,
        later,
        true,
    )
    .unwrap();
    let requests = &presentation.producer.requests;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].active_elapsed(), ms(50_000));
    assert_eq!(requests[1].active_delta(), Duration::ZERO);
    assert_eq!(runtime.frame_timing.deadline(), None);
}

#[test]
fn nonperiodic_redraws_accumulate_only_eligible_time_across_suspension() {
    let start = Instant::now();
    let (mut presentation, mut runtime) = session(None, start);
    let window = crate::tests::test_window();
    let assert_no_tick = |runtime: &mut ProbeRuntime, millis| {
        assert!(!runtime
            .frame_tick_ready(start + ms(millis), &window, false, None)
            .unwrap());
        assert_eq!(runtime.frame_timing.deadline(), None);
    };
    assert_no_tick(&mut runtime, 0);

    ready(&mut runtime);
    assert_no_tick(&mut runtime, 5);
    assert_no_tick(&mut runtime, 17);
    present(
        &mut presentation,
        &mut runtime,
        FrameKind::Redraw,
        start + ms(17),
        start + ms(19),
        true,
    )
    .unwrap();
    assert_eq!(runtime.frame_timing.deadline(), None);

    runtime.unavailable(false);
    assert_no_tick(&mut runtime, 25);
    ready(&mut runtime);
    runtime.input_fence.blocked = true;
    assert_no_tick(&mut runtime, 120);
    runtime.input_fence.blocked = false;
    assert_no_tick(&mut runtime, 125);
    assert_no_tick(&mut runtime, 132);
    present(
        &mut presentation,
        &mut runtime,
        FrameKind::Redraw,
        start + ms(132),
        start + ms(132),
        true,
    )
    .unwrap();
    assert_eq!(runtime.frame_timing.deadline(), None);

    let times: Vec<_> = presentation
        .producer
        .requests
        .iter()
        .map(|request| (request.active_elapsed(), request.active_delta()))
        .collect();
    assert_eq!(
        times,
        [
            (Duration::ZERO, Duration::ZERO),
            (ms(12), Duration::ZERO),
            (ms(27), Duration::ZERO),
        ]
    );
}

#[test]
fn recurring_deadline_caps_delta_and_rearms_after_completion_without_catch_up() {
    let start = Instant::now();
    let (mut presentation, mut runtime) = session(Some(PERIOD), start);
    let window = crate::tests::test_window();
    ready(&mut runtime);
    runtime
        .refresh_frame_eligibility(start + ms(5), &window, false)
        .unwrap();
    assert_eq!(runtime.frame_timing.deadline(), Some(start + ms(15)));
    assert!(!runtime
        .frame_tick_ready(start + ms(14), &window, false, None)
        .unwrap());
    assert!(runtime
        .frame_tick_ready(start + ms(15), &window, false, None)
        .unwrap());
    present(
        &mut presentation,
        &mut runtime,
        FrameKind::Scheduled,
        start + ms(15),
        start + ms(22),
        true,
    )
    .unwrap();
    assert_eq!(runtime.frame_timing.deadline(), Some(start + ms(32)));

    assert!(runtime
        .frame_tick_ready(start + ms(88), &window, false, None)
        .unwrap());
    present(
        &mut presentation,
        &mut runtime,
        FrameKind::Scheduled,
        start + ms(88),
        start + ms(110),
        true,
    )
    .unwrap();
    assert_eq!(runtime.frame_timing.deadline(), Some(start + ms(120)));
    for millis in [110, 111, 119] {
        assert!(!runtime
            .frame_tick_ready(start + ms(millis), &window, false, None)
            .unwrap());
    }
    let requests = &presentation.producer.requests;
    assert_eq!(requests.len(), 3);
    assert_eq!(
        (requests[1].active_elapsed(), requests[1].active_delta()),
        (ms(10), PERIOD)
    );
    assert_eq!(
        (requests[2].active_elapsed(), requests[2].active_delta()),
        (ms(83), PERIOD)
    );
    assert_eq!(
        runtime
            .frames
            .require_committed()
            .unwrap()
            .identity
            .sequence,
        2
    );
}

#[test]
fn redraw_uses_zero_delta_and_moves_the_deadline_from_successful_presentation() {
    let start = Instant::now();
    let (mut presentation, mut runtime) = session(Some(PERIOD), start);
    let window = crate::tests::test_window();
    ready(&mut runtime);
    runtime
        .refresh_frame_eligibility(start, &window, false)
        .unwrap();
    let started = start + ms(9);
    let completed = start + ms(13);
    present(
        &mut presentation,
        &mut runtime,
        FrameKind::Redraw,
        started,
        completed,
        true,
    )
    .unwrap();
    let redraw = presentation.producer.requests[1];
    assert_eq!(
        (redraw.active_elapsed(), redraw.active_delta()),
        (ms(9), Duration::ZERO)
    );
    assert_eq!(runtime.frame_timing.deadline(), Some(start + ms(23)));
    assert!(!runtime
        .frame_tick_ready(start + ms(22), &window, false, None)
        .unwrap());
}

#[test]
fn hidden_and_input_fenced_time_is_excluded_across_the_same_generation() {
    let start = Instant::now();
    let (mut presentation, mut runtime) = session(Some(PERIOD), start);
    let window = crate::tests::test_window();
    ready(&mut runtime);
    runtime
        .refresh_frame_eligibility(start, &window, false)
        .unwrap();
    runtime.unavailable(false);
    assert!(!runtime
        .frame_tick_ready(start + ms(8), &window, false, None)
        .unwrap());
    assert_eq!(runtime.frame_timing.deadline(), None);
    let original = runtime.frames.require_committed().unwrap().identity;
    runtime.body = BodyAvailability::Ready;
    runtime.input_fence.blocked = true;
    assert!(!runtime
        .frame_tick_ready(start + ms(100), &window, false, None)
        .unwrap());
    assert!(!runtime.body_input_ready());
    runtime.input_fence.blocked = false;
    assert!(!runtime
        .frame_tick_ready(start + ms(110), &window, false, None)
        .unwrap());
    assert_eq!(runtime.frame_timing.deadline(), Some(start + ms(120)));
    assert_eq!(
        runtime.frames.require_committed().unwrap().identity,
        original
    );
    assert!(runtime
        .frame_tick_ready(start + ms(120), &window, false, None)
        .unwrap());
    present(
        &mut presentation,
        &mut runtime,
        FrameKind::Scheduled,
        start + ms(120),
        start + ms(120),
        true,
    )
    .unwrap();
    let resumed = presentation.producer.requests[1];
    assert_eq!(resumed.identity(), original.checked_next().unwrap());
    assert_eq!(
        (resumed.active_elapsed(), resumed.active_delta()),
        (ms(18), PERIOD)
    );
    assert_eq!(runtime.input_fence.enabled_after, Some(20));
}

#[test]
fn owner_deadlines_and_pointer_or_movement_obligations_take_precedence() {
    let start = Instant::now();
    let (presentation, mut runtime) = session(Some(PERIOD), start);
    let mut window = crate::tests::test_window();
    ready(&mut runtime);
    runtime
        .refresh_frame_eligibility(start, &window, false)
        .unwrap();
    let due = start + PERIOD;
    assert!(!runtime
        .frame_tick_ready(due, &window, false, Some(due))
        .unwrap());
    assert_eq!(runtime.frame_timing.deadline(), Some(due));
    assert!(!runtime.frame_tick_ready(due, &window, true, None).unwrap());
    assert_eq!(runtime.frame_timing.deadline(), None);

    let mut now = due;
    let mut check_blocked = |runtime: &mut ProbeRuntime, window: &crate::ManagedProbeWindow| {
        now += PERIOD;
        assert!(!runtime.frame_tick_ready(now, window, false, None).unwrap());
        assert_eq!(runtime.frame_timing.deadline(), None);
    };

    assert_eq!(
        runtime.interaction.handle_right_press_on_body(20, true),
        crate::HostAction::TrackOpening
    );
    runtime.pointer.track_automatic_right().unwrap();
    check_blocked(&mut runtime, &window);
    assert!(matches!(
        runtime
            .interaction
            .handle_opening_release(3, 21, 0, Point::new(3, 4), true),
        crate::HostAction::OpeningFinished {
            anchor: Some(_),
            ..
        }
    ));
    runtime.pointer.automatic_right_finished().unwrap();
    assert!(!runtime.pointer.is_grabbed());

    runtime.menu = Some(MenuPopup::test_popup(200, 0));
    check_blocked(&mut runtime, &window);
    runtime.menu = None;

    runtime.pending_move = Some(Point::new(1, 2));
    check_blocked(&mut runtime, &window);
    runtime.pending_move = None;

    window.in_flight_moves.push_back(Point::new(5, 7));
    check_blocked(&mut runtime, &window);
    window.in_flight_moves.clear();

    runtime.final_target = Some(Point::new(5, 7));
    runtime.confirmation_deadline = Some(due);
    check_blocked(&mut runtime, &window);
    runtime.final_target = None;
    runtime.confirmation_deadline = None;

    runtime
        .layers
        .begin(crate::Layer::Above, crate::ObservedLayer::Normal, due);
    check_blocked(&mut runtime, &window);
    runtime.layers.abort();

    assert!(!runtime.frame_tick_ready(now, &window, false, None).unwrap());
    assert_eq!(runtime.frame_timing.deadline(), Some(now + PERIOD));
    assert_eq!(presentation.producer.requests.len(), 1);
}

#[test]
fn expose_and_post_map_repaint_the_commit_without_preparing_a_frame() {
    let start = Instant::now();
    let (presentation, mut runtime) = session(Some(PERIOD), start);
    let window = crate::tests::test_window();
    runtime.body = BodyAvailability::Ready;
    runtime.arm_map_input_sync(25);
    let initial_identity = runtime.frames.require_committed().unwrap().identity;
    let pending_map = MapNotifyEvent {
        response_type: MAP_NOTIFY_EVENT,
        sequence: 25,
        event: window.window,
        window: window.window,
        override_redirect: false,
    };
    let trace = RefCell::new(Vec::new());
    paint_committed_with(&runtime.frames, |dimensions| {
        assert_eq!(dimensions, (3, 2));
        trace.borrow_mut().push("remap blit");
        Ok(())
    })
    .unwrap();
    assert!(!runtime
        .frame_tick_ready(start, &window, false, None)
        .unwrap());
    runtime
        .synchronize_placed_map_input_with(&pending_map, &window, 25, |region| {
            assert!(region.contains(Point::new(0, 0)));
            assert!(!region.contains(Point::new(1, 0)));
            trace.borrow_mut().push("checked input restore");
            Ok(30)
        })
        .unwrap();
    runtime
        .refresh_frame_eligibility(start, &window, false)
        .unwrap();
    paint_committed_with(&runtime.frames, |dimensions| {
        assert_eq!(dimensions, (3, 2));
        trace.borrow_mut().push("expose blit");
        Ok(())
    })
    .unwrap();

    assert_eq!(
        trace.into_inner(),
        ["remap blit", "checked input restore", "expose blit"]
    );
    assert_eq!(presentation.producer.requests.len(), 1);
    assert_eq!(
        runtime.frames.require_committed().unwrap().identity,
        initial_identity
    );
    assert_eq!(runtime.frame_timing.deadline(), Some(start + PERIOD));
    assert_eq!(runtime.input_fence.enabled_after, Some(30));
}

#[test]
fn stopped_or_destroyed_session_cannot_prepare_recur_or_repaint() {
    let start = Instant::now();
    let (mut presentation, mut runtime) = session(Some(PERIOD), start);
    let window = crate::tests::test_window();
    ready(&mut runtime);
    runtime
        .refresh_frame_eligibility(start, &window, false)
        .unwrap();
    runtime.frame_timing.stop();
    assert!(!runtime
        .frame_tick_ready(start + PERIOD, &window, false, None)
        .unwrap());
    assert!(present(
        &mut presentation,
        &mut runtime,
        FrameKind::Redraw,
        start,
        start,
        true
    )
    .is_err());
    assert!(
        runtime.body_input_ready(),
        "shutdown closes scheduling before owner cleanup"
    );

    runtime.unavailable(true);
    assert!(runtime.input_fence.blocked);
    assert!(!runtime.body_input_ready());
    assert!(runtime.frames.committed().is_none());
    assert!(paint_committed_with(&runtime.frames, |_| panic!("destroyed target repaint")).is_err());
    assert!(present(
        &mut presentation,
        &mut runtime,
        FrameKind::Initial,
        start,
        start,
        false
    )
    .is_err());
    assert_eq!(presentation.producer.requests.len(), 1);
    assert_eq!(runtime.frame_timing.deadline(), None);
}

#[test]
fn clock_regression_and_deadline_overflow_stop_scheduling() {
    let start = Instant::now();
    let mut timing = FrameTiming::new(Some(PERIOD));
    timing.update_eligibility(start, true).unwrap();
    timing.update_eligibility(start + ms(8), false).unwrap();
    let error = timing.update_eligibility(start + ms(7), true).unwrap_err();
    assert!(error.to_string().contains("backwards"));
    assert!(timing.stopped);
    assert_eq!(timing.deadline(), None);

    let mut timing = FrameTiming::new(Some(Duration::MAX));
    let error = timing.update_eligibility(start, true).unwrap_err();
    assert!(error.to_string().contains("monotonic deadline"));
    assert!(timing.stopped);
    assert_eq!(timing.deadline(), None);
}

#[test]
fn invalid_intent_stops_before_preparation_and_preserves_commit() {
    let start = Instant::now();
    let (mut presentation, mut runtime) = session(Some(PERIOD), start);
    presentation.intent = ActionIntent {
        action: Action::Walk,
        facing: Facing::Neutral,
    };
    let error = present(
        &mut presentation,
        &mut runtime,
        FrameKind::Redraw,
        start,
        start,
        true,
    )
    .unwrap_err();
    assert!(error.to_string().contains("selected intent"));
    assert!(runtime.frame_timing.stopped);
    assert_eq!(presentation.producer.requests.len(), 1);
    assert_eq!(
        runtime
            .frames
            .require_committed()
            .unwrap()
            .identity
            .sequence,
        0
    );
}

#[test]
fn preparation_failure_never_submits_fences_or_retries() {
    let start = Instant::now();
    let (mut presentation, mut runtime) = session(Some(PERIOD), start);
    let window = crate::tests::test_window();
    ready(&mut runtime);
    runtime
        .refresh_frame_eligibility(start, &window, false)
        .unwrap();
    presentation.producer.fail = true;
    let due = start + PERIOD;
    let result = presentation.present_with(
        FrameKind::Scheduled,
        &mut runtime.frames,
        &mut runtime.frame_timing,
        true,
        || due,
        |_, _, _| panic!("preparation failed before submission"),
    );
    let error = runtime
        .finish_frame_publication_with(result, |_| panic!("no native mutation to fence"))
        .unwrap_err();
    assert_eq!(error.to_string(), "injected preparation failure");
    assert_eq!(
        runtime
            .frames
            .require_committed()
            .unwrap()
            .identity
            .sequence,
        0
    );
    assert!(runtime.body_input_ready());
    assert!(!runtime.frame_tick_ready(due, &window, false, None).unwrap());
    assert!(present(
        &mut presentation,
        &mut runtime,
        FrameKind::Redraw,
        due,
        due,
        true
    )
    .is_err());
    assert_eq!(presentation.producer.requests.len(), 2);
}

#[test]
fn malformed_producer_output_stops_before_native_upload_or_input_change() {
    let start = Instant::now();
    let (mut presentation, mut runtime) = session(Some(PERIOD), start);
    ready(&mut runtime);
    presentation.producer.truncate_payload = true;
    let result = presentation.present_with(
        FrameKind::Redraw,
        &mut runtime.frames,
        &mut runtime.frame_timing,
        true,
        || start,
        |frames, request, frame| {
            frames.publish_with(
                request,
                frame,
                |_, _| panic!("malformed frame must not upload"),
                |_| panic!("malformed frame must not blit"),
            )
        },
    );
    let error = runtime
        .finish_frame_publication_with(result, |_| panic!("pre-submit failure must not fence"))
        .unwrap_err();
    assert!(error.to_string().contains("unexpected byte length"));
    assert!(runtime.body_input_ready());
    assert_eq!(runtime.input_fence.enabled_after, Some(20));
    assert_eq!(
        runtime
            .frames
            .require_committed()
            .unwrap()
            .identity
            .sequence,
        0
    );
    assert!(runtime.frame_timing.stopped);
    assert_eq!(presentation.producer.requests.len(), 2);
}

#[test]
fn partial_upload_blit_or_flush_failure_fences_once_without_retry() {
    for (failure, expected) in [
        ("upload", &["upload"][..]),
        ("blit", &["upload", "blit"][..]),
        ("flush", &["upload", "blit", "flush"][..]),
    ] {
        let start = Instant::now();
        let (mut presentation, mut runtime) = session(Some(PERIOD), start);
        let mut window = crate::tests::test_window();
        ready(&mut runtime);
        runtime
            .refresh_frame_eligibility(start, &window, false)
            .unwrap();
        let due = start + PERIOD;
        let trace = RefCell::new(Vec::new());
        let checked = |step| -> Result<(), HostError> {
            trace.borrow_mut().push(step);
            if step == failure {
                Err(failure.into())
            } else {
                Ok(())
            }
        };
        let result = presentation.present_with(
            FrameKind::Scheduled,
            &mut runtime.frames,
            &mut runtime.frame_timing,
            true,
            || due,
            |frames, request, frame| {
                frames.publish_with(
                    request,
                    frame,
                    |_, _| checked("upload"),
                    |_| checked_blit_with(|| checked("blit"), || checked("flush")),
                )
            },
        );
        assert_eq!(*trace.borrow(), expected);
        let error = runtime
            .finish_frame_publication_with(result, |runtime| {
                assert!(runtime.frames.publication_uncertain());
                assert!(runtime.frames.committed().is_none());
                assert_eq!(runtime.frame_timing.deadline(), None);
                runtime.fence_visibility_input_with(
                    &mut window,
                    || {
                        trace.borrow_mut().push("empty input");
                        Err("secondary cleanup failure".into())
                    },
                    |_, _| {
                        trace.borrow_mut().push("pointer release");
                        Ok(())
                    },
                    |_| panic!("no popup"),
                )
            })
            .unwrap_err();
        assert_eq!(error.to_string(), failure);
        assert_eq!(
            &trace.borrow()[expected.len()..],
            ["empty input", "pointer release"]
        );
        assert!(runtime.input_fence.blocked);
        assert!(!runtime.body_input_ready());
        assert!(!runtime
            .frame_tick_ready(due + PERIOD, &window, false, None)
            .unwrap());
        assert!(present(
            &mut presentation,
            &mut runtime,
            FrameKind::Redraw,
            due,
            due,
            false
        )
        .is_err());
        assert_eq!(presentation.producer.requests.len(), 2);
    }
}

#[test]
fn live_factory_and_recurring_submission_stay_behind_owner_and_event_boundaries() {
    let source = include_str!("../lib.rs");
    let (entry, rest) = source.split_once("fn run_owner(").unwrap();
    assert!(!entry.contains("ProceduralProducer::new("));
    let owner = rest.split_once("fn owner_startup_with").unwrap().0;
    assert_eq!(owner.matches("ProceduralProducer::new(").count(), 1);
    assert!(owner.contains("owner_presentation_startup_with("));
    assert!(
        owner.find("owner.delay(conn, delay)").unwrap()
            < owner.find("owner.verify_startup_ownership(conn)").unwrap()
    );
    assert!(
        owner.find("owner.verify_startup_ownership(conn)").unwrap()
            < owner.find("ProceduralProducer::new(").unwrap()
    );

    let probe = source
        .split_once("fn run_probe(")
        .unwrap()
        .1
        .split_once("fn finish_probe_run_with(")
        .unwrap()
        .0;
    let tick = probe.find("FrameKind::Scheduled").unwrap();
    assert_eq!(probe.matches("FrameKind::Scheduled").count(), 1);
    let before_tick = &probe[..tick];
    let flush = before_tick
        .rfind("runtime.flush_move(conn, window)?")
        .unwrap();
    let lookahead = before_tick
        .rfind("conn.poll_for_event_with_sequence()?")
        .unwrap();
    assert!(before_tick.rfind("service_owner_operation(").unwrap() < flush);
    assert!(flush < lookahead);
    assert!(before_tick[lookahead..].contains("buffered_event = Some(event)"));
    assert!(before_tick[lookahead..].contains("continue;"));
    assert!(before_tick[lookahead..].contains(".chain(remote_deadline(owner))"));
    assert!(before_tick[lookahead..].contains("runtime.frame_tick_ready("));
    assert!(probe[tick..].contains(".chain(runtime.frame_timing.deadline())"));
    assert!(probe[tick..].contains("wait_x11(conn, timeout)"));
}

#[cfg(feature = "managed-supervisor-fixture")]
#[test]
fn asynchronous_opportunity_consumption_and_rearming_preserve_active_elapsed() {
    let origin = Instant::now();
    let mut timing = FrameTiming::new(Some(PERIOD));
    let at = |millis| origin + Duration::from_millis(millis);
    timing.update_eligibility(at(0), true).unwrap();
    assert!(!timing.consume_opportunity(at(9)));
    assert!(timing.consume_opportunity(at(10)));
    assert!(!timing.consume_opportunity(at(100)));
    timing.update_opportunity(at(100), false).unwrap();
    assert_eq!(timing.deadline(), None);
    timing.update_eligibility(at(150), true).unwrap();
    assert_eq!(
        timing.elapsed_at(at(150)).unwrap(),
        Duration::from_millis(150)
    );
    timing.update_opportunity(at(150), true).unwrap();
    assert_eq!(timing.deadline(), Some(at(160)));
    assert_eq!(timing.last_request_elapsed, Duration::ZERO);
    timing.update_eligibility(at(151), false).unwrap();
    assert_eq!(timing.deadline(), None);
    timing.update_eligibility(at(900), true).unwrap();
    assert!(!timing.due(at(909)));
    assert!(timing.due(at(910)));
    assert_eq!(
        timing.elapsed_at(at(910)).unwrap(),
        Duration::from_millis(161)
    );
}

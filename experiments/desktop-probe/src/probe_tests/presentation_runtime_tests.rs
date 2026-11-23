use super::*;
use crate::body_presentation::{BodyPresentation, FrameKind};
use crate::control::{Capability, Evidence, MapState, VisibilityEvidence, WmState, Workspace};
use crate::geometry::{Point, Rect};
use crate::presentation::procedural::{Palette, ProceduralProducer};
use crate::presentation::{
    Action, ActionIntent, Facing, FrameIdentity, FrameProducer, FrameRequest, PreparedFrame,
    PresentationCapabilities,
};
use crate::x11::events::drain_events_bounded;
use crate::x11::render::{checked_blit_with, paint_committed_with};
use crate::{
    BodyAvailability, HostAction, HostError, InputPlacement, ManagedProbeWindow, ProbeRuntime,
};
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::{
    ExposeEvent, GrabStatus, MapNotifyEvent, UnmapNotifyEvent, MAP_NOTIFY_EVENT, UNMAP_NOTIFY_EVENT,
};
use x11rb::protocol::Event;

const IDLE: ActionIntent = ActionIntent {
    action: Action::Idle,
    facing: Facing::Neutral,
};
const PERIOD: Duration = Duration::from_millis(10);
const MAP_SEQUENCE: u64 = 65_536 + 40;
const SHAPE_SEQUENCE: u64 = 2 * 65_536 + 10;

fn map_event(window: &ManagedProbeWindow) -> MapNotifyEvent {
    MapNotifyEvent {
        response_type: MAP_NOTIFY_EVENT,
        sequence: MAP_SEQUENCE as u16,
        event: window.window,
        window: window.window,
        override_redirect: false,
    }
}

fn placement(window: &ManagedProbeWindow) -> InputPlacement {
    let origin = window.confirmed_origin();
    InputPlacement {
        selected_monitor: 20,
        observed_monitor: 20,
        geometry: Rect::new(origin.x, origin.y, 160, 160),
        bounds: crate::geometry::ValidOriginBounds::new(0, 500, 0, 500),
    }
}

fn visible() -> VisibilityEvidence {
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

/// Uses the procedural pixels with a synthetic period and records actual producer calls.
struct RecordingProducer<'a> {
    procedural: ProceduralProducer,
    capabilities: PresentationCapabilities,
    requests: &'a RefCell<Vec<FrameRequest>>,
    fail_preparation: &'a Cell<bool>,
}

impl<'a> RecordingProducer<'a> {
    fn new(requests: &'a RefCell<Vec<FrameRequest>>, fail_preparation: &'a Cell<bool>) -> Self {
        let procedural = ProceduralProducer::new(Palette::Lavender);
        let capabilities = PresentationCapabilities::new(
            procedural.capabilities().initial_extent(),
            vec![IDLE],
            Some(PERIOD),
        )
        .unwrap();
        Self {
            procedural,
            capabilities,
            requests,
            fail_preparation,
        }
    }
}

impl FrameProducer for RecordingProducer<'_> {
    type Payload = Vec<u8>;
    type Error = HostError;

    fn capabilities(&self) -> &PresentationCapabilities {
        &self.capabilities
    }

    fn prepare_frame(
        &mut self,
        request: &FrameRequest,
    ) -> Result<PreparedFrame<Vec<u8>>, HostError> {
        self.requests.borrow_mut().push(*request);
        if self.fail_preparation.get() {
            return Err(NativeFault("prepare").into());
        }
        Ok(self.procedural.prepare_frame(request)?)
    }
}

fn runtime_for<P>(presentation: &BodyPresentation<P>) -> ProbeRuntime
where
    P: FrameProducer<Payload = Vec<u8>>,
    P::Error: Into<HostError>,
{
    ProbeRuntime::with_presentation(presentation.initial_extent(), presentation.timing())
}

fn commit<P>(
    presentation: &mut BodyPresentation<P>,
    runtime: &mut ProbeRuntime,
    kind: FrameKind,
    start: Instant,
    finish: Instant,
    eligible: bool,
) where
    P: FrameProducer<Payload = Vec<u8>>,
    P::Error: Into<HostError>,
{
    let mut clock = [start, finish].into_iter();
    presentation
        .present_with(
            kind,
            &mut runtime.frames,
            &mut runtime.frame_timing,
            eligible,
            || clock.next().unwrap(),
            |geometry, request, frame| {
                geometry.publish_with(request, frame, |_, _| Ok(()), |_| Ok(()))
            },
        )
        .unwrap();
    assert_eq!(clock.next(), None);
}

fn assert_shape_matches_upload(shape: InputShapeRequest, window: u32, pixels: &[u8]) {
    assert_eq!((shape.operation, shape.kind), (SO::SET, SK::INPUT));
    assert_eq!(shape.ordering, ClipOrdering::UNSORTED);
    assert_eq!((shape.window, shape.x, shape.y), (window, 0, 0));
    let mut covered = vec![false; 160 * 160];
    for rect in shape.rectangles {
        assert_eq!(rect.height, 1);
        let (x, y) = (rect.x as usize, rect.y as usize);
        assert!(y < 160 && x + usize::from(rect.width) <= 160);
        assert!(rect.width > 0);
        for value in &mut covered[y * 160 + x..y * 160 + x + usize::from(rect.width)] {
            assert!(!*value, "overlapping SHAPE rectangles");
            *value = true;
        }
    }
    assert_eq!(pixels.len(), covered.len() * 4);
    let (pixels, _) = pixels.as_chunks::<4>();
    for (index, (pixel, covered)) in pixels.iter().zip(covered).enumerate() {
        assert_eq!(
            pixel[3] != 0,
            covered,
            "pixel ({}, {})",
            index % 160,
            index / 160
        );
        if !covered {
            assert_eq!(*pixel, [0; 4]);
        }
    }
}

fn assert_procedural_upload(pixels: &[u8], body: [u8; 4]) {
    assert_eq!(pixels.len(), 160 * 160 * 4);
    let (pixels, _) = pixels.as_chunks::<4>();
    for (index, actual) in pixels.iter().enumerate() {
        let (x, y) = ((index % 160) as i32, (index / 160) as i32);
        let expected = if (x - 80).pow(2) + (y - 80).pow(2) <= 45 * 45 {
            if [68, 92]
                .iter()
                .any(|eye| (x - eye).pow(2) + (y - 75).pow(2) <= 16)
            {
                [255; 4]
            } else {
                body
            }
        } else if (15..=65).contains(&x) && (15..=65).contains(&y) {
            [128, 100, 0, 128]
        } else {
            [0; 4]
        };
        assert_eq!(*actual, expected, "{body:?} at ({x}, {y})");
    }
}

#[test]
fn procedural_upload_map_shape_click_and_expose_share_the_commit() {
    let t0 = Instant::now();
    let mut producer = BodyPresentation::new(ProceduralProducer::new(Palette::Lavender)).unwrap();
    let mut runtime = runtime_for(&producer);
    let mut window = crate::tests::test_window();
    let map = map_event(&window);
    let placement = placement(&window);
    let pixels = RefCell::new(Vec::new());
    let trace = RefCell::new(Vec::new());
    let step = |name| {
        trace.borrow_mut().push(name);
        Ok(())
    };
    let blit = |name, dimensions| {
        assert_eq!(dimensions, (160, 160));
        checked_blit_with(|| step(name), || step("flush"))
    };
    let initial_and_redraw = |producer: &mut BodyPresentation<ProceduralProducer>,
                              runtime: &mut ProbeRuntime,
                              kind,
                              now,
                              eligible| {
        producer
            .present_with(
                kind,
                &mut runtime.frames,
                &mut runtime.frame_timing,
                eligible,
                || now,
                |frames, request, frame| {
                    frames.publish_with(
                        request,
                        frame,
                        |dimensions, bytes| {
                            assert_eq!(dimensions, (160, 160));
                            pixels.replace(bytes.to_vec());
                            step("upload")
                        },
                        |dimensions| blit("blit", dimensions),
                    )
                },
            )
            .unwrap();
    };
    let repaint = |runtime: &ProbeRuntime| {
        paint_committed_with(&runtime.frames, |dimensions| blit("repaint", dimensions)).unwrap();
    };
    let check_shape = |region: &HitRegion| {
        restore_body_input_shape_with(map.window, region, |request| {
            step("shape")?;
            assert_shape_matches_upload(request, map.window, &pixels.borrow());
            Ok(SHAPE_SEQUENCE)
        })
    };

    initial_and_redraw(&mut producer, &mut runtime, FrameKind::Initial, t0, false);
    runtime.arm_map_input_sync(MAP_SEQUENCE);
    runtime
        .synchronize_placed_map_input_with(&map, &window, MAP_SEQUENCE, |_| {
            panic!("an initial commit alone cannot enable input")
        })
        .unwrap();
    repaint(&runtime);
    runtime.body = BodyAvailability::Validating;
    runtime
        .placement_with(
            &mut window,
            placement.geometry,
            &placement.bounds,
            |_, _| panic!("the observed placement already fits"),
        )
        .unwrap();
    runtime
        .synchronize_placed_map_input_with(&map, &window, MAP_SEQUENCE, check_shape)
        .unwrap();
    assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE_SEQUENCE));
    assert!(runtime.body_input_ready());

    assert_procedural_upload(&pixels.borrow(), [225, 95, 165, 255]);
    let frame_before = runtime.frames.require_committed().unwrap().identity;
    let initial_region = runtime
        .frames
        .require_committed()
        .unwrap()
        .metadata
        .hit_region()
        .clone();
    let origin = window.confirmed_origin();
    let overlap = Point::new(65, 65);
    let padding = Point::new(15, 66);
    assert!(!runtime.body_hit(padding));
    assert!(runtime.body_hit(Point::new(15, 15)));
    let hit = runtime.body_hit(overlap);
    let root = Point::new(origin.x + overlap.x, origin.y + overlap.y);
    assert_eq!(
        runtime
            .interaction
            .handle_left_press_on_body(root, hit, 100, origin),
        HostAction::AcquireGrab { time: 100 }
    );
    runtime
        .pointer
        .acquire_with(crate::CaptureOwner::BodyLeft, || {
            Ok((GrabStatus::SUCCESS, SHAPE_SEQUENCE + 1))
        })
        .unwrap();
    runtime.interaction.on_grab_acquired();
    assert_eq!(
        runtime
            .interaction
            .handle_left_release_on_body(root, hit, 101, &placement.bounds),
        HostAction::ReleaseGrabAndToggleColor { time: 101 }
    );
    runtime.pointer.release_with(|| step("release")).unwrap();
    assert_eq!(producer.toggle_palette(), Palette::Coral);
    let later = t0 + Duration::from_secs(90);
    initial_and_redraw(&mut producer, &mut runtime, FrameKind::Redraw, later, true);

    let coral = runtime.frames.require_committed().unwrap();
    assert_procedural_upload(&pixels.borrow(), [80, 110, 240, 255]);
    assert_eq!(coral.metadata.hit_region(), &initial_region);
    // Rebuild the checked request offline from the Coral commit. Palette redraw
    // itself must keep the existing input sequence and send no new SHAPE.
    restore_body_input_shape_with(window.window, coral.metadata.hit_region(), |request| {
        assert_shape_matches_upload(request, window.window, &pixels.borrow());
        Ok(SHAPE_SEQUENCE)
    })
    .unwrap();
    assert_eq!(coral.identity, frame_before.checked_next().unwrap());
    assert_eq!(runtime.input_fence.enabled_after, Some(SHAPE_SEQUENCE));
    repaint(&runtime);
    assert!(!runtime
        .frame_tick_ready(later + Duration::from_secs(90), &window, false, None)
        .unwrap());
    assert_eq!(runtime.frame_timing.deadline(), None);
    assert_eq!(
        trace.into_inner(),
        [
            "upload", "blit", "flush", "repaint", "flush", "shape", "release", "upload", "blit",
            "flush", "repaint", "flush",
        ]
    );
}

#[derive(Debug)]
struct NativeFault(&'static str);

impl std::fmt::Display for NativeFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for NativeFault {}

#[test]
fn presentation_faults_stop_production_and_keep_priority_through_ordered_cleanup() {
    use crate::x11::resource::{cleanup_all, OwnedResource};
    for (failure, submission) in [
        ("prepare", &[][..]),
        ("upload", &["upload"][..]),
        ("blit", &["upload", "blit"][..]),
        ("flush", &["upload", "blit", "flush"][..]),
    ] {
        let start = Instant::now();
        let requests = RefCell::new(Vec::new());
        let fail_preparation = Cell::new(false);
        let mut presentation =
            BodyPresentation::new(RecordingProducer::new(&requests, &fail_preparation)).unwrap();
        let mut runtime = runtime_for(&presentation);
        commit(
            &mut presentation,
            &mut runtime,
            FrameKind::Initial,
            start,
            start,
            false,
        );
        let mut window = crate::tests::test_window();
        runtime.body = BodyAvailability::Ready;
        runtime.input_fence.enabled_after = Some(SHAPE_SEQUENCE);
        assert!(!runtime
            .frame_tick_ready(start, &window, false, None)
            .unwrap());
        let due = start + PERIOD;
        assert!(runtime.frame_tick_ready(due, &window, false, None).unwrap());
        fail_preparation.set(failure == "prepare");
        let trace = RefCell::new(Vec::new());
        let checked = |step| -> Result<(), HostError> {
            trace.borrow_mut().push(step);
            if step == failure {
                Err(NativeFault(step).into())
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
        let error = result.unwrap_err();
        let original = error.downcast_ref::<NativeFault>().unwrap() as *const NativeFault;
        assert_eq!(*trace.borrow(), submission);
        let result = runtime.finish_frame_publication_with(Err(error), |runtime| {
            assert!(runtime.frames.committed().is_none());
            runtime.fence_visibility_input_with(
                &mut window,
                || {
                    trace.borrow_mut().push("empty SHAPE");
                    Err("secondary SHAPE failure".into())
                },
                |_, _| {
                    trace.borrow_mut().push("transition release");
                    Err("secondary release failure".into())
                },
                |_| panic!("no popup was opened"),
            )
        });
        if submission.is_empty() {
            assert!(runtime.body_input_ready());
            assert_eq!(
                runtime
                    .frames
                    .require_committed()
                    .unwrap()
                    .identity
                    .sequence,
                0
            );
        } else {
            assert!(runtime.input_fence.blocked);
            assert!(!runtime.input_fence.empty_confirmed);
            assert!(runtime.pointer.is_grabbed());
            assert!(!runtime.body_input_ready());
            assert!(paint_committed_with(&runtime.frames, |_| panic!("uncertain pixmap")).is_err());
        }
        assert!(!runtime
            .frame_tick_ready(due + PERIOD, &window, false, None)
            .unwrap());
        assert!(presentation
            .present_with(
                FrameKind::Redraw,
                &mut runtime.frames,
                &mut runtime.frame_timing,
                false,
                || panic!("terminal error must stop new requests"),
                |_, _, _| panic!("terminal error must stop uploads"),
            )
            .is_err());
        assert_eq!(requests.borrow().len(), 2);
        let gc = OwnedResource::default();
        let pixmap = OwnedResource::default();
        let error = crate::finish_probe_with(result, || {
            crate::owner_cleanup_with(
                &mut (),
                |_| checked("Closing"),
                || {
                    let release = runtime.cancel_with(&mut window, |pointer| {
                        pointer
                            .release_with(|| checked("shutdown release"))
                            .map(|_| ())
                    });
                    cleanup_all([
                        Box::new(|| release),
                        Box::new(|| {
                            gc.release_with(|| {
                                checked("checked free GC")?;
                                Err("secondary GC failure".into())
                            })
                        }),
                        Box::new(|| pixmap.release_with(|| checked("checked free pixmap"))),
                        Box::new(|| {
                            window
                                .window_resource
                                .release_with(|| checked("checked destroy body"))
                        }),
                    ])
                },
                |_| checked("checked destroy control"),
                || checked("shutdown flush"),
            )
        })
        .unwrap_err();
        assert!(std::ptr::eq(
            error.downcast_ref::<NativeFault>().unwrap(),
            original
        ));
        assert_eq!(error.to_string(), failure);
        assert!(!runtime.pointer.is_grabbed());
        assert!(gc
            .release_with(|| panic!("uncertain destruction cannot repeat"))
            .is_err());
        pixmap
            .release_with(|| panic!("acknowledged destruction cannot repeat"))
            .unwrap();
        let mut expected = submission.to_vec();
        if !submission.is_empty() {
            expected.extend(["empty SHAPE", "transition release"]);
        }
        expected.push("Closing");
        if !submission.is_empty() {
            expected.push("shutdown release");
        }
        expected.extend([
            "checked free GC",
            "checked free pixmap",
            "checked destroy body",
            "checked destroy control",
            "shutdown flush",
        ]);
        assert_eq!(trace.into_inner(), expected);
    }
}

#[test]
fn redispatched_and_unsettled_frames_cannot_upload_paint_or_reopen_input() {
    let mut procedural = ProceduralProducer::new(Palette::Lavender);
    let mut runtime = ProbeRuntime::new();
    let mut window = crate::tests::test_window();
    let request = |runtime: &ProbeRuntime| {
        runtime
            .frames
            .frame_request(IDLE, Duration::ZERO, Duration::ZERO)
            .unwrap()
    };
    let initial = request(&runtime);
    runtime
        .frames
        .publish_with(
            &initial,
            procedural.prepare_frame(&initial).unwrap(),
            |_, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap();
    let next = request(&runtime);
    let future = FrameRequest::new(
        next.identity().checked_next().unwrap(),
        IDLE,
        next.extent(),
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let reject =
        |runtime: &mut ProbeRuntime, producer: &mut ProceduralProducer, replay, expected| {
            let error = runtime
                .frames
                .publish_with(
                    &replay,
                    producer.prepare_frame(&replay).unwrap(),
                    |_, _| panic!("rejected identity must not upload"),
                    |_| panic!("rejected identity must not draw"),
                )
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Renderer publication rejected: {expected}")
            );
        };
    runtime.body = BodyAvailability::Ready;
    runtime.input_fence.enabled_after = Some(SHAPE_SEQUENCE);
    reject(&mut runtime, &mut procedural, initial, "Stale");
    assert!(runtime.body_hit(Point::new(65, 65)));
    paint_committed_with(&runtime.frames, |size| {
        assert_eq!(size, (160, 160));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        runtime.frames.require_committed().unwrap().identity,
        initial.identity()
    );

    let result = runtime.frames.publish_with(
        &next,
        procedural.prepare_frame(&next).unwrap(),
        |_, _| Ok(()),
        |_| checked_blit_with(|| Ok(()), || Err("uncertain flush".into())),
    );
    assert!(runtime.frames.publication_uncertain());
    assert!(!runtime.body_input_ready());
    assert!(paint_committed_with(&runtime.frames, |_| panic!("unsettled Expose")).is_err());
    reject(&mut runtime, &mut procedural, next, "Stale");
    reject(&mut runtime, &mut procedural, future, "UnsettledCandidate");

    runtime.arm_map_input_sync(MAP_SEQUENCE);
    let error = runtime
        .finish_frame_publication_with(result, |runtime| {
            runtime.fence_visibility_input_with(
                &mut window,
                || Ok(SHAPE_SEQUENCE),
                |_, _| Ok(()),
                |_| Ok(()),
            )
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "uncertain flush");
    reject(&mut runtime, &mut procedural, next, "Stale");
    reject(
        &mut runtime,
        &mut procedural,
        FrameRequest::new(
            FrameIdentity {
                generation: 1,
                sequence: 0,
            },
            IDLE,
            next.extent(),
            Duration::ZERO,
            Duration::ZERO,
        )
        .unwrap(),
        "UnsettledCandidate",
    );
    assert!(runtime
        .frames
        .frame_request(IDLE, Duration::ZERO, Duration::ZERO)
        .is_err());
    runtime.body = BodyAvailability::Ready; // later visibility evidence
    assert!(runtime.visibility_dispatch_eligible());
    runtime
        .synchronize_placed_map_input_with(&map_event(&window), &window, SHAPE_SEQUENCE + 1, |_| {
            panic!("stale map cannot restore SHAPE")
        })
        .unwrap();
    assert!(runtime
        .restore_visibility_input_with(&window, visible(), &placement(&window), |_| panic!(
            "fresh placement cannot restore stale coverage"
        ),)
        .is_err());
    assert!(paint_committed_with(&runtime.frames, |_| panic!("invalidated Expose")).is_err());
    assert!(runtime.input_fence.blocked);
    assert!(!runtime.body_hit(Point::new(65, 65)));
}

#[test]
fn wrong_publication_phase_and_premature_tick_stop_before_preparing() {
    for (kind, committed, diagnostic) in [
        (FrameKind::Redraw, false, "publication phase"),
        (FrameKind::Initial, true, "publication phase"),
        (FrameKind::Scheduled, true, "No recurring frame is due"),
    ] {
        let start = Instant::now();
        let requests = RefCell::new(Vec::new());
        let preparation_fault = Cell::new(false);
        let mut presentation =
            BodyPresentation::new(RecordingProducer::new(&requests, &preparation_fault)).unwrap();
        let mut runtime = runtime_for(&presentation);
        let window = crate::tests::test_window();
        if committed {
            commit(
                &mut presentation,
                &mut runtime,
                FrameKind::Initial,
                start,
                start,
                true,
            );
        }
        let failed = presentation.present_with(
            kind,
            &mut runtime.frames,
            &mut runtime.frame_timing,
            true,
            || start,
            |_, _, _| panic!("invalid request reached publication"),
        );
        let error = runtime
            .finish_frame_publication_with(failed, |_| panic!("no submission to fence"))
            .unwrap_err();
        assert!(error.to_string().contains(diagnostic), "{error}");
        assert_eq!(requests.borrow().len(), usize::from(committed));
        assert_eq!(runtime.frame_timing.deadline(), None);
        assert!(!runtime
            .frame_tick_ready(start + PERIOD, &window, false, None)
            .unwrap());
    }
}

#[test]
fn failed_post_map_shape_revokes_paint_and_cannot_retry_on_duplicate_map() {
    let mut runtime = ProbeRuntime::test_presented();
    let mut window = crate::tests::test_window();
    runtime.body = BodyAvailability::Ready;
    runtime.arm_map_input_sync(MAP_SEQUENCE);
    let map = map_event(&window);
    let trace = RefCell::new(Vec::new());
    let result = runtime.synchronize_placed_map_input_with(&map, &window, MAP_SEQUENCE, |region| {
        restore_body_input_shape_with(window.window, region, |request| {
            assert!(!request.rectangles.is_empty());
            trace.borrow_mut().push("enable SHAPE");
            Err(NativeFault("enable SHAPE").into())
        })
    });
    let error = result.unwrap_err();
    assert_eq!(runtime.pending_map_input, None);
    assert_eq!(runtime.input_fence.enabled_after, None);
    assert!(!runtime.body_input_ready());
    runtime.invalidate_frame_and_fence_with(|runtime| {
        runtime.fence_visibility_input_with(
            &mut window,
            || {
                set_input_with(map.window, vec![], |request| {
                    assert!(request.rectangles.is_empty());
                    trace.borrow_mut().push("empty SHAPE");
                    Ok(SHAPE_SEQUENCE)
                })
            },
            |_, _| {
                trace.borrow_mut().push("release");
                Ok(())
            },
            |_| panic!("no popup"),
        )
    });
    runtime.body = BodyAvailability::Ready;
    runtime
        .synchronize_placed_map_input_with(&map, &window, SHAPE_SEQUENCE + 1, |_| {
            panic!("no second enable after failed check")
        })
        .unwrap();
    assert!(
        paint_committed_with(&runtime.frames, |_| panic!("invalidated remap repaint")).is_err()
    );
    assert!(runtime
        .restore_visibility_input_with(&window, visible(), &placement(&window), |_| panic!(
            "no committed coverage to restore"
        ),)
        .is_err());
    assert!(!runtime.body_input_ready());
    assert!(runtime.input_fence.empty_confirmed);
    assert_eq!(
        error.downcast_ref::<NativeFault>().unwrap().0,
        "enable SHAPE"
    );
    assert_eq!(
        trace.into_inner(),
        ["enable SHAPE", "empty SHAPE", "release"]
    );
}

#[test]
fn unmap_at_the_batch_boundary_cancels_movement_and_rearms_only_after_restore() {
    let start = Instant::now();
    let requests = RefCell::new(Vec::new());
    let preparation_fault = Cell::new(false);
    let mut presentation =
        BodyPresentation::new(RecordingProducer::new(&requests, &preparation_fault)).unwrap();
    let mut runtime = runtime_for(&presentation);
    let mut window = crate::tests::test_window();
    commit(
        &mut presentation,
        &mut runtime,
        FrameKind::Initial,
        start,
        start,
        false,
    );
    runtime.body = BodyAvailability::Ready;
    runtime.input_fence.enabled_after = Some(SHAPE_SEQUENCE);
    assert!(!runtime
        .frame_tick_ready(start, &window, false, None)
        .unwrap());
    let deadline = start + PERIOD;
    assert_eq!(runtime.frame_timing.deadline(), Some(deadline));
    assert!(runtime
        .frame_tick_ready(deadline, &window, false, None)
        .unwrap());
    let duration = crate::probe_deadline(&runtime, Some(deadline));
    assert_eq!(duration, Some(deadline));
    assert!(!runtime
        .frame_tick_ready(deadline, &window, false, duration)
        .unwrap());

    let initial = runtime.frames.require_committed().unwrap().identity;
    let bounds = placement(&window).bounds;
    let origin = window.confirmed_origin();
    let point = Point::new(180, 180);
    let hit = runtime.body_hit(Point::new(80, 80));
    assert_eq!(
        runtime
            .interaction
            .handle_left_press_on_body(point, hit, 100, origin),
        HostAction::AcquireGrab { time: 100 }
    );
    runtime
        .pointer
        .acquire_with(crate::CaptureOwner::BodyLeft, || {
            Ok((GrabStatus::SUCCESS, SHAPE_SEQUENCE + 1))
        })
        .unwrap();
    runtime.interaction.on_grab_acquired();
    let target = runtime
        .interaction
        .handle_motion(Point::new(230, 230), 101, &bounds);
    assert_eq!(
        target,
        HostAction::MoveWindow {
            target: Point::new(150, 150)
        }
    );
    runtime.pending_move = Some(Point::new(150, 150));
    window.in_flight_moves.push_back(Point::new(140, 140));

    let expose = Event::Expose(ExposeEvent {
        window: window.window,
        count: 1,
        ..Default::default()
    });
    let unmap = Event::UnmapNotify(UnmapNotifyEvent {
        response_type: UNMAP_NOTIFY_EVENT,
        event: window.window,
        window: window.window,
        ..Default::default()
    });
    let mut incoming = (0..64)
        .map(|index| (expose.clone(), SHAPE_SEQUENCE + index + 2))
        .chain([(unmap.clone(), SHAPE_SEQUENCE + 66)]);
    let mut buffered = None;
    let mut poll = || Ok::<_, HostError>(incoming.next());
    let batch = drain_events_bounded(&mut buffered, 64, &mut poll).unwrap();
    assert_eq!(batch.len(), 64);
    assert_eq!(batch.last().unwrap().1, SHAPE_SEQUENCE + 65);
    let (event, sequence) = poll().unwrap().unwrap();
    assert!(crate::interrupts_pending_movement(
        &event,
        1,
        window.window,
        &crate::x11::monitors::LayoutAtoms {
            number_of_desktops: 90,
            desktop_viewport: 91,
            workarea: 3,
            current_desktop: 4,
            wm_desktop: 5,
        }
    ));
    // The lookahead is retained for the next bounded drain. The source check
    // below ties this seam to the native loop's interrupted flush branch.
    buffered = Some((event, sequence));
    assert!(!runtime
        .frame_tick_ready(deadline, &window, false, None)
        .unwrap());
    assert_eq!(runtime.frame_timing.deadline(), None);
    let boundary = drain_events_bounded(&mut buffered, 64, poll).unwrap();
    assert!(matches!(&boundary[..], [(Event::UnmapNotify(ev), sequence)]
        if ev.window == window.window && *sequence == SHAPE_SEQUENCE + 66));
    runtime
        .fence_visibility_input_with(
            &mut window,
            || Ok(SHAPE_SEQUENCE + 67),
            |runtime, window| {
                assert!(runtime.pending_move.is_none());
                assert!(window.in_flight_moves.is_empty());
                Ok(())
            },
            |_| panic!("a drag does not own a popup"),
        )
        .unwrap();
    assert!(!runtime.pointer.is_grabbed());
    assert!(!runtime.body_hit(Point::new(80, 80)));
    assert_eq!(
        runtime.frames.require_committed().unwrap().identity,
        initial
    );

    let remapped = start + Duration::from_secs(3);
    let input = placement(&window);
    runtime.body = BodyAvailability::Validating;
    runtime
        .placement_with(&mut window, input.geometry, &input.bounds, |_, _| {
            panic!("already in bounds")
        })
        .unwrap();
    assert!(!runtime
        .frame_tick_ready(remapped, &window, false, None)
        .unwrap());
    assert_eq!(runtime.frame_timing.deadline(), None);
    runtime
        .restore_visibility_input_with(&window, visible(), &input, |region| {
            restore_body_input_shape_with(window.window, region, |shape| {
                assert!(!shape.rectangles.is_empty());
                Ok(SHAPE_SEQUENCE + 70)
            })
        })
        .unwrap();
    assert!(!runtime
        .frame_tick_ready(remapped, &window, false, None)
        .unwrap());
    assert_eq!(runtime.frame_timing.deadline(), Some(remapped + PERIOD));
    assert_eq!(requests.borrow().len(), 1);
    assert!(runtime
        .frame_tick_ready(remapped + PERIOD, &window, false, None)
        .unwrap());
    commit(
        &mut presentation,
        &mut runtime,
        FrameKind::Scheduled,
        remapped + PERIOD,
        remapped + PERIOD * 2,
        true,
    );
    let resumed = requests.borrow()[1];
    assert_eq!(resumed.identity(), initial.checked_next().unwrap());
    assert_eq!(
        (resumed.active_elapsed(), resumed.active_delta()),
        (PERIOD * 2, PERIOD)
    );
    assert_eq!(runtime.frame_timing.deadline(), Some(remapped + PERIOD * 3));

    runtime.unavailable(true);
    assert!(!runtime
        .frame_tick_ready(remapped + PERIOD * 3, &window, false, None)
        .unwrap());
    assert!(paint_committed_with(&runtime.frames, |_| panic!("destroyed body repaint")).is_err());
    assert_eq!(requests.borrow().len(), 2);
}

#[test]
fn native_loop_routes_expose_and_buffered_events_before_frame_wait() {
    let source = include_str!("../lib.rs");
    let probe = source
        .split_once("fn run_probe(")
        .unwrap()
        .1
        .split_once("fn finish_probe_run_with(")
        .unwrap()
        .0;
    let expose = probe
        .split_once("Event::Expose(ev) if ev.window == window.window && ev.count == 0")
        .unwrap()
        .1;
    let expose = expose
        .split_once("Event::ClientMessage(ev) if wm_delete_event")
        .unwrap()
        .0;
    assert!(expose.contains("renderer.paint(conn, window.window, &runtime.frames)"));
    assert!(expose.contains("runtime.invalidate_frame_and_fence(conn, window);"));
    assert!(!expose.contains("present_frame("));

    let lookahead = probe.split_once("// Look one event ahead").unwrap().1;
    let (edge, rest) = lookahead
        .split_once("if runtime.apply_safety(Instant::now(), None)")
        .unwrap();
    assert!(edge.contains("interrupts_pending_movement(&event, root, window.window, atoms)"));
    assert!(edge.contains("buffered_event = Some((event, sequence));"));
    assert!(edge.contains("if !interrupted {\n                runtime.flush_move(conn, window)?;"));
    assert!(edge.trim_end().ends_with("continue;\n        }"));
    let (after_move, rest) = rest
        .split_once("let host_deadline = probe_deadline")
        .unwrap();
    assert!(
        after_move.find("conn.flush()?").unwrap()
            < after_move.find("buffered_event = Some(event);").unwrap()
    );
    assert!(after_move.trim_end().ends_with("continue;\n        }"));

    let (after_present, wait) = rest.split_once("let timeout = host_deadline").unwrap();
    let scheduled = after_present.split_once("FrameKind::Scheduled").unwrap().1;
    assert!(scheduled.contains("conn.poll_for_event_with_sequence()?"));
    assert!(scheduled.contains("buffered_event = Some(event);\n                continue;"));
    assert!(wait.contains(".chain(runtime.frame_timing.deadline())"));
    assert!(wait.contains("wait_x11(conn, timeout)?"));
}

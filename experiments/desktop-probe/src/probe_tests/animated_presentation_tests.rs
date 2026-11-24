use super::*;
use crate::body_presentation::{BodyPresentation, BodyProducer, FrameKind};
use crate::control::{Capability, Evidence, MapState, VisibilityEvidence, WmState, Workspace};
use crate::geometry::{Rect, ValidOriginBounds};
use crate::presentation::procedural::{Palette, ProceduralProducer};
use crate::presentation::synthetic::SyntheticProducer;
use crate::presentation::{
    Action, ActionIntent, ClientExtent, Facing, FrameIdentity, FrameMetadata, FrameProducer,
    FrameRequest, Point, PreparedFrame, PublicationState, RowSpan, Size, ValidationError,
};
use crate::x11::events::drain_events_bounded;
use crate::x11::render::{checked_blit_with, paint_committed_with, CoverageCutover, FrameGeometry};
use crate::{
    AnimatedCoverage, BodyAvailability, HostAction, HostError, InputPlacement, ManagedProbeWindow,
    ProbeRuntime,
};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::{
    ButtonPressEvent, ClientMessageEvent, DestroyNotifyEvent, MapNotifyEvent, UnmapNotifyEvent,
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, MAP_NOTIFY_EVENT, UNMAP_NOTIFY_EVENT,
};
use x11rb::protocol::Event;

#[path = "asynchronous_presentation_tests.rs"]
mod asynchronous_tests;

const OLD_BOUNDARY: u64 = 65_537;
const BOUNDARY: u64 = 131_072;
const REQUEST_LIMIT: usize = 262_140;
const IMAGE_SIZE: usize = 160 * 160 * 4;
const INITIAL: FrameIdentity = FrameIdentity {
    generation: 0,
    sequence: 0,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    Block,
    Upload,
    EmptyShape,
    Release,
    Blit,
    Flush,
    FinalShape,
    SafetyEmpty,
    SafetyRelease,
}

const CUTOVER: [Step; 7] = [
    Step::Block,
    Step::Upload,
    Step::EmptyShape,
    Step::Release,
    Step::Blit,
    Step::Flush,
    Step::FinalShape,
];
const IMAGE_ONLY: [Step; 3] = [Step::Upload, Step::Blit, Step::Flush];

#[derive(Default)]
struct CheckedTransport {
    trace: RefCell<Vec<Step>>,
    image: RefCell<Vec<u8>>,
    final_rectangles: RefCell<Option<Vec<Rectangle>>>,
    failure: Option<Step>,
    fail_safety: bool,
    during_empty: RefCell<Vec<(Event, u64)>>,
    buffered: RefCell<VecDeque<(Event, u64)>>,
}

impl CheckedTransport {
    fn check(&self, step: Step) -> Result<(), HostError> {
        self.trace.borrow_mut().push(step);
        if step == Step::EmptyShape {
            self.buffered.borrow_mut().extend(self.during_empty.take());
        }
        if self.failure == Some(step)
            || (self.fail_safety && matches!(step, Step::SafetyEmpty | Step::SafetyRelease))
        {
            return Err(format!("{step:?}").into());
        }
        Ok(())
    }

    fn set_shape(
        &self,
        body: Window,
        step: Step,
        rectangles: Vec<Rectangle>,
    ) -> Result<u64, HostError> {
        set_input_with(body, rectangles, |request| {
            assert_eq!(
                (request.operation, request.kind, request.ordering),
                (SO::SET, SK::INPUT, ClipOrdering::UNSORTED)
            );
            assert_eq!((request.window, request.x, request.y), (body, 0, 0));
            if step == Step::FinalShape {
                assert_rectangles_match_image(&request.rectangles, &self.image.borrow());
                *self.final_rectangles.borrow_mut() = Some(request.rectangles);
            } else {
                assert!(request.rectangles.is_empty());
            }
            self.check(step)?;
            Ok(BOUNDARY)
        })
    }

    fn present(
        &self,
        runtime: &mut ProbeRuntime,
        window: &mut ManagedProbeWindow,
        presentation: &mut BodyPresentation<SyntheticProducer>,
        kind: FrameKind,
        clock: [Instant; 2],
    ) -> Result<(), HostError> {
        let body = window.window;
        let limit = runtime.animated_shape_limit.unwrap();
        let eligible = runtime.frame_eligible(window, false);
        let mut clock = clock.into_iter();
        runtime.present_frame_with(
            presentation,
            kind,
            eligible,
            || clock.next().unwrap(),
            |frames, request, frame, input, pointer| {
                let old_boundary = input.enabled_after;
                let after_upload = Cell::new(false);
                let next_shape = Cell::new(Step::EmptyShape);
                let mut coverage = AnimatedCoverage {
                    input,
                    pointer,
                    maximum_request_bytes: limit,
                    set_input: |rectangles| {
                        assert!(after_upload.get());
                        let step = next_shape.replace(Step::FinalShape);
                        self.set_shape(body, step, rectangles)
                    },
                    release: || self.check(Step::Release),
                };
                let changed = frames
                    .committed()
                    .is_some_and(|old| old.metadata.hit_region() != frame.metadata.hit_region());
                let result = frames.publish_with_cutover(
                    request,
                    frame,
                    |dimensions, image| {
                        assert_eq!(dimensions, (160, 160));
                        self.check(Step::Upload)?;
                        *self.image.borrow_mut() = image.to_vec();
                        after_upload.set(true);
                        Ok(())
                    },
                    |dimensions| {
                        assert_eq!(dimensions, (160, 160));
                        checked_blit_with(|| self.check(Step::Blit), || self.check(Step::Flush))
                    },
                    Some(&mut RecordBlock(&mut coverage, &self.trace)),
                );
                assert_eq!(coverage.input.enabled_after, old_boundary);
                assert_eq!(coverage.input.blocked, changed);
                assert_eq!(
                    coverage.input.empty_confirmed,
                    changed
                        && matches!(self.failure, Some(Step::Release | Step::Blit | Step::Flush))
                );
                assert_eq!(
                    coverage.pointer.is_grabbed(),
                    matches!(self.failure, Some(Step::EmptyShape | Step::Release))
                );
                let phase = match self.failure {
                    None => PublicationState::HostCommitted,
                    Some(Step::FinalShape) => PublicationState::TransportCompleted,
                    Some(_) => PublicationState::Submitted,
                };
                assert_eq!(frames.publication_state(request.identity()), Ok(phase));
                result
            },
            |runtime| {
                assert!(runtime.input_fence.blocked);
                assert!(runtime.frames.committed().is_none());
                assert!(runtime.frame_timing.deadline().is_none());
                let body = window.window;
                runtime.fence_visibility_input_with(
                    window,
                    || self.set_shape(body, Step::SafetyEmpty, vec![]),
                    |runtime, _| {
                        assert!(runtime.pointer.is_grabbed());
                        self.check(Step::SafetyRelease)
                    },
                    |_| panic!("scheduled animation cannot own a popup"),
                )
            },
        )
    }
}

struct RecordBlock<'a, C>(&'a mut C, &'a RefCell<Vec<Step>>);

impl<C: CoverageCutover> CoverageCutover for RecordBlock<'_, C> {
    fn maximum_request_bytes(&self) -> usize {
        self.0.maximum_request_bytes()
    }
    fn block_input(&mut self) {
        self.0.block_input();
        self.1.borrow_mut().push(Step::Block);
    }
    fn empty_and_release(&mut self) -> Result<(), HostError> {
        self.0.empty_and_release()
    }
    fn enable(&mut self, rectangles: Vec<Rectangle>) -> Result<u64, HostError> {
        self.0.enable(rectangles)
    }
}

fn assert_rectangles_match_image(rectangles: &[Rectangle], image: &[u8]) {
    let mut mask = vec![false; 160 * 160];
    assert_eq!(image.len(), IMAGE_SIZE);
    for rect in rectangles {
        assert_eq!(rect.height, 1);
        let start = rect.y as usize * 160 + rect.x as usize;
        for covered in &mut mask[start..start + usize::from(rect.width)] {
            assert!(!*covered, "native spans must be disjoint");
            *covered = true;
        }
    }
    for (pixel, covered) in image.as_chunks::<4>().0.iter().zip(mask) {
        assert_eq!(pixel[3] != 0, covered);
        assert!(pixel[..3].iter().all(|&channel| channel <= pixel[3]));
    }
}

fn session(
    now: Instant,
) -> (
    ProbeRuntime,
    ManagedProbeWindow,
    BodyPresentation<SyntheticProducer>,
    Duration,
) {
    let producer = SyntheticProducer::new();
    let period = producer.capabilities().frame_period().unwrap();
    let mut presentation = BodyPresentation::new(producer).unwrap();
    let mut window = crate::tests::test_window();
    let mut runtime =
        ProbeRuntime::with_presentation(presentation.initial_extent(), presentation.timing());
    runtime.animated_shape_limit = Some(REQUEST_LIMIT);
    let transport = CheckedTransport::default();
    transport
        .present(
            &mut runtime,
            &mut window,
            &mut presentation,
            FrameKind::Initial,
            [now; 2],
        )
        .unwrap();
    assert_eq!(*transport.trace.borrow(), IMAGE_ONLY);
    assert_eq!(runtime.frames.last_committed(), Some(INITIAL));
    assert_eq!(runtime.body, BodyAvailability::AwaitingMap);
    assert!(!runtime.body_input_ready());
    assert!(runtime.frame_timing.deadline().is_none());
    runtime.body = BodyAvailability::Ready;
    runtime.input_fence.enabled_after = Some(OLD_BOUNDARY);
    assert!(!runtime.frame_tick_ready(now, &window, false, None).unwrap());
    (runtime, window, presentation, period)
}

struct NoNativeWrites {
    request_limit: usize,
}

impl CoverageCutover for NoNativeWrites {
    fn maximum_request_bytes(&self) -> usize {
        self.request_limit
    }
    fn block_input(&mut self) {
        panic!("preflight must not block candidate input");
    }
    fn empty_and_release(&mut self) -> Result<(), HostError> {
        panic!("preflight must not set empty input or release");
    }
    fn enable(&mut self, _: Vec<Rectangle>) -> Result<u64, HostError> {
        panic!("preflight must not set the candidate shape");
    }
}

fn reject(
    frames: &mut FrameGeometry,
    request: &FrameRequest,
    frame: PreparedFrame<Vec<u8>>,
    request_limit: usize,
    diagnostic: &str,
) {
    let error = frames
        .publish_with_cutover(
            request,
            frame,
            |_, _| panic!("preflight must not upload"),
            |_| panic!("preflight must not blit"),
            Some(&mut NoNativeWrites { request_limit }),
        )
        .unwrap_err();
    assert!(error.to_string().contains(diagnostic), "{error}");
}

#[test]
fn owner_admission_constructs_one_selected_producer_and_synthetic_click_is_passive() {
    use crate::x11::control::Acquisition;
    let source = include_str!("../lib.rs");
    let owner = source
        .split_once("fn run_owner(")
        .unwrap()
        .1
        .split_once("fn owner_startup_with")
        .unwrap()
        .0;
    assert_eq!(owner.matches("SyntheticProducer::new()").count(), 1);
    assert!(
        owner.find("owner.verify_startup_ownership(conn)").unwrap()
            < owner.find("SyntheticProducer::new()").unwrap()
    );
    assert!(owner.contains("owner_presentation_startup_with("));
    assert!(source.contains("if let Some(palette) = presentation.click_palette()"));
    for (synthetic, owned) in [(true, true), (false, true), (true, false), (false, false)] {
        let calls = Cell::new(0);
        let acquired = if owned {
            Acquisition::Owned
        } else {
            Acquisition::Duplicate
        };
        crate::owner_presentation_startup_with(
            &mut false,
            |_| Ok(acquired),
            |verified| {
                *verified = true;
                Ok(())
            },
            || {
                calls.set(calls.get() + 1);
                Ok(if synthetic {
                    BodyProducer::Synthetic(SyntheticProducer::new())
                } else {
                    BodyProducer::Procedural(ProceduralProducer::new(Palette::default()))
                })
            },
            |verified, mut presentation| {
                assert!(*verified);
                assert_eq!(presentation.initial_extent().size(), Size::new(160, 160));
                assert_eq!(
                    presentation.click_palette(),
                    (!synthetic).then_some(Palette::Coral)
                );
                assert_eq!(
                    presentation.click_palette(),
                    (!synthetic).then_some(Palette::Lavender)
                );
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(calls.get(), usize::from(owned));
    }
}

#[test]
fn scheduled_cutovers_follow_exact_checked_order_and_replay_only_the_commit() {
    let now = Instant::now();
    let (mut runtime, mut window, mut presentation, period) = session(now);
    let initial_bounds = runtime
        .frames
        .require_committed()
        .unwrap()
        .metadata
        .visible_bounds();
    let initial_region = runtime
        .frames
        .require_committed()
        .unwrap()
        .metadata
        .hit_region()
        .clone();
    for phase in 1..=5 {
        let when = now + period * phase;
        assert!(runtime
            .frame_tick_ready(when, &window, false, None)
            .unwrap());
        let transport = CheckedTransport::default();
        transport
            .present(
                &mut runtime,
                &mut window,
                &mut presentation,
                FrameKind::Scheduled,
                [when; 2],
            )
            .unwrap();
        let frame = runtime.frames.require_committed().unwrap();
        assert_eq!(frame.identity.sequence, u64::from(phase));
        assert_eq!(frame.metadata.extent().size(), Size::new(160, 160));
        let region = frame.metadata.hit_region();
        let bounds = frame.metadata.visible_bounds();
        if phase == 1 {
            assert_eq!(*transport.trace.borrow(), IMAGE_ONLY);
            assert!(transport.final_rectangles.borrow().is_none());
            assert_eq!(bounds, initial_bounds);
            assert_eq!(region, &initial_region);
            assert_eq!(
                &transport.image.borrow()[(45 * 160 + 20) * 4..][..4],
                &[70, 211, 150, 255]
            );
            assert_eq!(runtime.input_fence.enabled_after, Some(OLD_BOUNDARY));
        } else {
            assert_eq!(*transport.trace.borrow(), CUTOVER);
            let fields = |r: &Rectangle| (r.x, r.y, r.width, r.height);
            assert!(transport
                .final_rectangles
                .borrow()
                .as_ref()
                .unwrap()
                .iter()
                .map(fields)
                .eq(input_rectangles(region).unwrap().iter().map(fields)));
            assert_eq!(runtime.input_fence.enabled_after, Some(BOUNDARY));
        }
        assert!(runtime.body_input_ready());
        match phase {
            2 => {
                assert_ne!(bounds, initial_bounds);
                assert!(!runtime.body_hit(Point::new(30, 55)), "removed");
                assert!(runtime.body_hit(Point::new(55, 55)), "shared");
                assert!(
                    runtime.body_hit(Point::new(90, 55)),
                    "new, partially transparent"
                );
                assert!(!runtime.body_hit(Point::new(68, 73)), "hole");
            }
            3 => {
                assert!(!runtime.body_hit(Point::new(55, 55)), "disjoint move");
                assert!(runtime.body_hit(Point::new(115, 30)));
                assert!(runtime.body_hit(Point::new(145, 120)));
                assert!(!runtime.body_hit(Point::new(133, 70)), "disjoint gap");
            }
            4 => {
                assert!(bounds.is_none());
                assert!(frame.metadata.anchor().is_none());
                assert!(region.spans().is_empty());
                assert!(transport.image.borrow().iter().all(|&byte| byte == 0));
                assert!(!runtime.body_hit(Point::new(115, 30)));
            }
            5 => {
                assert_eq!(bounds, initial_bounds);
                assert_eq!(region, &initial_region);
                assert_eq!(
                    &transport.image.borrow()[(45 * 160 + 20) * 4..][..4],
                    &[230, 115, 60, 255]
                );
                assert!(runtime.body_hit(Point::new(30, 55)));
            }
            _ => {}
        }
        paint_committed_with(&runtime.frames, |size| {
            assert_eq!(size, (160, 160));
            assert_eq!(runtime.frames.last_committed(), Some(frame.identity));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            transport.trace.borrow().len(),
            if phase == 1 {
                IMAGE_ONLY.len()
            } else {
                CUTOVER.len()
            }
        );
        assert_eq!(runtime.frame_timing.deadline(), Some(when + period));
    }
}

#[test]
fn animated_preflight_rejects_identity_extent_payload_and_wire_size_without_requests() {
    let now = Instant::now();
    let (mut runtime, _, _, period) = session(now);
    let mut producer = SyntheticProducer::new();
    let idle = producer.capabilities().supported_intents()[0];
    let next = runtime
        .frames
        .frame_request(idle, period * 2, period)
        .unwrap();
    let make = || SyntheticProducer::new().prepare_frame(&next).unwrap();
    let mut mismatch = make();
    mismatch.identity = INITIAL;
    let mut pma = make();
    pma.payload[0] = 255;
    let mut alpha = make();
    alpha.payload[3] = 1;
    let mut short = make();
    short.payload.pop();
    let mut transparent_with_bounds = make();
    transparent_with_bounds.payload.fill(0);
    transparent_with_bounds.metadata = FrameMetadata::new(
        next.extent(),
        Some(Rect::new(0, 0, 1, 1)),
        None,
        HitRegion::new(next.extent(), []).unwrap(),
    )
    .unwrap();
    let mut wrong_metadata = make();
    let small = ClientExtent::new(Size::new(1, 1)).unwrap();
    wrong_metadata.metadata =
        FrameMetadata::new(small, None, None, HitRegion::new(small, []).unwrap()).unwrap();
    for (frame, diagnostic) in [
        (mismatch, "IdentityMismatch"),
        (pma, "non-premultiplied"),
        (alpha, "alpha does not match"),
        (short, "unexpected byte length"),
        (transparent_with_bounds, "must omit visible bounds"),
        (wrong_metadata, "ExtentMismatch"),
    ] {
        reject(&mut runtime.frames, &next, frame, REQUEST_LIMIT, diagnostic);
    }
    let new_request =
        |identity, extent| FrameRequest::new(identity, idle, extent, period * 2, period).unwrap();
    for identity in [
        INITIAL,
        FrameIdentity {
            generation: 1,
            sequence: 1,
        },
        FrameIdentity {
            generation: 0,
            sequence: 2,
        },
    ] {
        let request = new_request(identity, next.extent());
        reject(
            &mut runtime.frames,
            &request,
            producer.prepare_frame(&request).unwrap(),
            REQUEST_LIMIT,
            "IdentityMismatch",
        );
    }
    let resized = new_request(next.identity(), small);
    reject(
        &mut runtime.frames,
        &resized,
        make(),
        REQUEST_LIMIT,
        "extent changes are unsupported",
    );
    let rectangles = input_rectangles(make().metadata.hit_region()).unwrap();
    let shape_bytes = 16 + rectangles.len() * 8;
    reject(
        &mut runtime.frames,
        &next,
        make(),
        shape_bytes - 1,
        "Input SHAPE request requires",
    );
    reject(
        &mut runtime.frames,
        &next,
        make(),
        IMAGE_SIZE + 23,
        "PutImage request requires",
    );

    let extent = next.extent();
    let region = make().metadata.hit_region().clone();
    assert_eq!(
        FrameMetadata::new(extent, Some(Rect::new(0, 0, 1, 1)), None, region.clone()),
        Err(ValidationError::CoverageOutsideVisibleBounds)
    );
    assert_eq!(
        FrameMetadata::new(extent, None, Some(Point::new(160, 0)), region),
        Err(ValidationError::AnchorOutOfBounds)
    );
    assert!(HitRegion::new(
        extent,
        [RowSpan {
            y: 160,
            start_x: 0,
            end_x: 1
        }]
    )
    .is_err());
    assert_eq!(
        runtime.frames.require_committed().unwrap().identity,
        INITIAL
    );
    assert!(!runtime.frames.publication_uncertain());
    for (width, diagnostic) in [
        (32_768, "signed coordinate limits"),
        (65_536, "client width"),
    ] {
        let extent = ClientExtent::new(Size::new(width, 1)).unwrap();
        let request = new_request(INITIAL, extent);
        let frame = PreparedFrame {
            identity: INITIAL,
            metadata: FrameMetadata::new(extent, None, None, HitRegion::new(extent, []).unwrap())
                .unwrap(),
            payload: vec![],
        };
        reject(
            &mut FrameGeometry::new(extent),
            &request,
            frame,
            REQUEST_LIMIT,
            diagnostic,
        );
    }
}

#[test]
fn uncertain_animated_candidate_cannot_be_resent_or_replaced() {
    let now = Instant::now();
    let (mut runtime, _, _, period) = session(now);
    let mut producer = SyntheticProducer::new();
    let intent = ActionIntent {
        action: Action::Idle,
        facing: Facing::Neutral,
    };
    let request = runtime
        .frames
        .frame_request(intent, period * 2, period)
        .unwrap();
    let frame = || SyntheticProducer::new().prepare_frame(&request).unwrap();
    let error = runtime
        .frames
        .publish_with_cutover(
            &request,
            frame(),
            |_, _| Err("checked upload".into()),
            |_| panic!("upload failed"),
            Some(&mut AnimatedCoverage {
                input: &mut runtime.input_fence,
                pointer: &mut runtime.pointer,
                maximum_request_bytes: REQUEST_LIMIT,
                set_input: |_| panic!("upload failed"),
                release: || panic!("upload failed"),
            }),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "checked upload");
    reject(
        &mut runtime.frames,
        &request,
        frame(),
        REQUEST_LIMIT,
        "unsettled",
    );
    let newer = FrameRequest::new(
        request.identity().checked_next().unwrap(),
        intent,
        request.extent(),
        period * 3,
        period,
    )
    .unwrap();
    reject(
        &mut runtime.frames,
        &newer,
        producer.prepare_frame(&newer).unwrap(),
        REQUEST_LIMIT,
        "unsettled",
    );
    assert_eq!(runtime.frames.last_committed(), Some(INITIAL));
    assert!(paint_committed_with(&runtime.frames, |_| panic!("uncertain backing pixmap")).is_err());
}

#[test]
fn preflight_error_stops_animation_without_fencing_the_usable_commit() {
    let now = Instant::now();
    let (mut runtime, window, mut presentation, period) = session(now);
    let due = now + period * 2;
    let error = runtime
        .present_frame_with(
            &mut presentation,
            FrameKind::Scheduled,
            true,
            || due,
            |frames, request, frame, _, _| {
                frames.publish_with_cutover(
                    request,
                    frame,
                    |_, _| panic!("oversized SHAPE must not upload"),
                    |_| panic!("oversized SHAPE must not blit"),
                    Some(&mut NoNativeWrites { request_limit: 16 }),
                )
            },
            |_| panic!("rejected preflight must not start the native safety fence"),
        )
        .unwrap_err();
    assert!(error.to_string().contains("Input SHAPE request requires"));
    assert_eq!(runtime.frames.committed().unwrap().identity, INITIAL);
    assert_eq!(runtime.input_fence.enabled_after, Some(OLD_BOUNDARY));
    assert!(runtime.body_hit(Point::new(30, 55)));
    assert!(!runtime.pointer.is_grabbed());
    assert!(!runtime
        .frame_tick_ready(due + period, &window, false, None)
        .unwrap());
    paint_committed_with(&runtime.frames, |_| Ok(())).unwrap();
}

#[test]
fn bounds_and_anchor_updates_with_identical_coverage_do_not_send_shape() {
    let now = Instant::now();
    let (mut runtime, _, _, period) = session(now);
    let mut producer = SyntheticProducer::new();
    let intent = producer.capabilities().supported_intents()[0];
    let request = runtime
        .frames
        .frame_request(intent, period, period)
        .unwrap();
    let mut frame = producer.prepare_frame(&request).unwrap();
    let bounds = Rect::new(0, 0, 160, 160);
    let anchor = Point::new(2, 2);
    frame.metadata = FrameMetadata::new(
        request.extent(),
        Some(bounds),
        Some(anchor),
        frame.metadata.hit_region().clone(),
    )
    .unwrap();
    let trace = CheckedTransport::default();
    let sequence = runtime
        .frames
        .publish_with_cutover(
            &request,
            frame,
            |_, _| trace.check(Step::Upload),
            |_| checked_blit_with(|| trace.check(Step::Blit), || trace.check(Step::Flush)),
            Some(&mut NoNativeWrites {
                request_limit: REQUEST_LIMIT,
            }),
        )
        .unwrap();
    assert_eq!(sequence, None);
    assert_eq!(trace.trace.into_inner(), IMAGE_ONLY);
    let commit = runtime.frames.require_committed().unwrap();
    assert_eq!(commit.metadata.visible_bounds(), Some(bounds));
    assert_eq!(commit.metadata.anchor(), Some(anchor));
}

#[test]
fn every_checked_cutover_failure_fences_and_preserves_original_error_through_cleanup() {
    for (index, failure) in CUTOVER.into_iter().enumerate().skip(1) {
        let now = Instant::now();
        let (mut runtime, mut window, mut presentation, period) = session(now);
        let due = now + period * 2;
        let transport = CheckedTransport {
            failure: Some(failure),
            fail_safety: true,
            ..Default::default()
        };
        let result = transport.present(
            &mut runtime,
            &mut window,
            &mut presentation,
            FrameKind::Scheduled,
            [due; 2],
        );
        assert_eq!(runtime.frames.last_committed(), Some(INITIAL));
        assert!(!runtime.input_fence.empty_confirmed);
        assert!(runtime.input_fence.blocked);
        assert_eq!(runtime.input_fence.enabled_after, Some(OLD_BOUNDARY));
        assert!(
            runtime.pointer.is_grabbed(),
            "failed safety release retains debt"
        );
        assert!(!runtime.body_hit(Point::new(55, 55)));
        assert!(paint_committed_with(&runtime.frames, |_| panic!("obsolete Expose")).is_err());
        assert!(!runtime
            .frame_tick_ready(due + period, &window, false, None)
            .unwrap());
        let original = result.as_ref().unwrap_err().as_ref() as *const dyn std::error::Error;
        let cleanup = RefCell::new(Vec::new());
        let error = crate::finish_probe_with(result, || {
            crate::owner_cleanup_with(
                &mut (),
                |_| {
                    cleanup.borrow_mut().push("Closing");
                    Ok(())
                },
                || {
                    let release = runtime.cancel_with(&mut window, |pointer| {
                        pointer
                            .release_with(|| {
                                cleanup.borrow_mut().push("shutdown release");
                                Err("secondary shutdown release failure".into())
                            })
                            .map(|_| ())
                    });
                    crate::x11::resource::cleanup_all([
                        Box::new(|| release),
                        Box::new(|| {
                            cleanup.borrow_mut().push("renderer cleanup");
                            Err("secondary renderer failure".into())
                        }),
                        Box::new(|| {
                            cleanup.borrow_mut().push("destroy body");
                            Ok(())
                        }),
                    ])
                },
                |_| {
                    cleanup.borrow_mut().push("destroy control");
                    Ok(())
                },
                || {
                    cleanup.borrow_mut().push("flush");
                    Ok(())
                },
            )
        })
        .unwrap_err();
        assert!(std::ptr::addr_eq(original, error.as_ref()));
        assert_eq!(error.to_string(), format!("{failure:?}"));
        let mut expected = CUTOVER[..=index].to_vec();
        expected.extend([Step::SafetyEmpty, Step::SafetyRelease]);
        assert_eq!(transport.trace.into_inner(), expected);
        assert_eq!(
            cleanup.into_inner(),
            [
                "Closing",
                "shutdown release",
                "renderer cleanup",
                "destroy body",
                "destroy control",
                "flush"
            ]
        );
        assert!(runtime.pointer.is_grabbed());
    }
}

#[test]
fn post_commit_clock_error_revokes_both_image_only_and_changed_authority_before_fencing() {
    for (phase, expected) in [(1, &IMAGE_ONLY[..]), (2, &CUTOVER[..])] {
        let now = Instant::now();
        let (mut runtime, mut window, mut presentation, period) = session(now);
        let due = now + period * phase;
        let transport = CheckedTransport {
            fail_safety: true,
            ..Default::default()
        };
        let error = transport
            .present(
                &mut runtime,
                &mut window,
                &mut presentation,
                FrameKind::Scheduled,
                [due, due - Duration::from_millis(1)],
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "Frame clock moved backwards");
        assert!(
            !runtime.frames.publication_uncertain(),
            "commit settled before the clock error"
        );
        assert_eq!(runtime.frames.last_committed(), INITIAL.checked_next());
        assert!(runtime.frames.committed().is_none());
        assert_eq!(runtime.input_fence.enabled_after, Some(OLD_BOUNDARY));
        assert!(runtime.input_fence.blocked);
        assert!(runtime.pointer.is_grabbed());
        assert!(paint_committed_with(&runtime.frames, |_| panic!("invalidated Expose")).is_err());
        assert!(!runtime
            .frame_tick_ready(due + period, &window, false, None)
            .unwrap());
        assert_eq!(
            transport.trace.into_inner(),
            expected
                .iter()
                .copied()
                .chain([Step::SafetyEmpty, Step::SafetyRelease])
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn cutover_buffers_lifecycle_and_controls_and_rejects_stale_buttons_across_wrap() {
    let now = Instant::now();
    let (mut runtime, mut window, mut presentation, period) = session(now);
    let body = window.window;
    let button = |response_type: u8, sequence: u64, time| ButtonPressEvent {
        response_type,
        detail: 3,
        event: body,
        sequence: sequence as u16,
        time,
        same_screen: true,
        event_x: 55,
        event_y: 55,
        ..Default::default()
    };
    let transport = CheckedTransport::default();
    let older_with_same_wire_sequence = BOUNDARY - 65_536;
    let stale_press = Event::ButtonPress(button(
        BUTTON_PRESS_EVENT,
        older_with_same_wire_sequence,
        10,
    ));
    let press_before_empty = Event::ButtonPress(button(BUTTON_PRESS_EVENT, BOUNDARY - 2, 11));
    let stale_release = Event::ButtonRelease(button(BUTTON_RELEASE_EVENT, BOUNDARY - 1, 12));
    let newer_release = Event::ButtonRelease(button(BUTTON_RELEASE_EVENT, BOUNDARY, 13));
    let control = Event::ClientMessage(ClientMessageEvent::new(32, 777, 888u32, [0u32; 5]));
    let unmap = Event::UnmapNotify(UnmapNotifyEvent {
        response_type: UNMAP_NOTIFY_EVENT,
        event: body,
        window: body,
        ..Default::default()
    });
    let destroy = Event::DestroyNotify(DestroyNotifyEvent {
        event: body,
        window: body,
        ..Default::default()
    });
    *transport.during_empty.borrow_mut() = vec![
        (stale_press, older_with_same_wire_sequence),
        (press_before_empty, BOUNDARY - 2),
        (stale_release, BOUNDARY - 1),
        (newer_release, BOUNDARY),
        (control, BOUNDARY),
        (unmap, BOUNDARY + 1),
        (destroy, BOUNDARY + 2),
    ];
    let due = now + period * 2;
    transport
        .present(
            &mut runtime,
            &mut window,
            &mut presentation,
            FrameKind::Scheduled,
            [due; 2],
        )
        .unwrap();
    assert_eq!(*transport.trace.borrow(), CUTOVER);
    assert!(!runtime.pointer.is_grabbed());

    let source = include_str!("../lib.rs");
    let tick = source
        .split_once("FrameKind::Scheduled,\n                false,")
        .unwrap()
        .1;
    let lookahead = tick.find("conn.poll_for_event_with_sequence()?").unwrap();
    assert!(lookahead < tick.find("wait_x11(conn, timeout)?").unwrap());
    assert!(tick[lookahead..].contains("buffered_event = Some(event)"));
    let poll = || Ok::<_, HostError>(transport.buffered.borrow_mut().pop_front());
    let mut lookahead = poll().unwrap();
    let batch = drain_events_bounded(&mut lookahead, 64, poll).unwrap();
    assert_eq!(
        batch
            .iter()
            .map(|(_, sequence)| *sequence)
            .collect::<Vec<_>>(),
        [
            older_with_same_wire_sequence,
            BOUNDARY - 2,
            BOUNDARY - 1,
            BOUNDARY,
            BOUNDARY,
            BOUNDARY + 1,
            BOUNDARY + 2
        ]
    );
    assert!(
        matches!(&batch[0].0, Event::ButtonPress(ev) if ev.time == 10 && ev.sequence == BOUNDARY as u16)
    );
    assert!(matches!(&batch[1].0, Event::ButtonPress(ev) if ev.time == 11));
    assert!(matches!(&batch[2].0, Event::ButtonRelease(ev) if ev.time == 12));
    assert!(matches!(&batch[3].0, Event::ButtonRelease(ev) if ev.time == 13));
    assert!(matches!(&batch[4].0, Event::ClientMessage(ev) if ev.window == 777));
    assert!(matches!(&batch[5].0, Event::UnmapNotify(ev) if ev.window == body));
    assert!(matches!(&batch[6].0, Event::DestroyNotify(ev) if ev.window == body));
    for (event, sequence) in &batch[..3] {
        assert!(runtime.rejects_body_event(event, body, *sequence));
    }
    assert!(!runtime.rejects_body_event(&batch[3].0, body, BOUNDARY));
    assert_eq!(
        runtime
            .interaction
            .handle_opening_release(3, 13, 0, Point::new(55, 55), true),
        HostAction::None
    );
    for (event, sequence) in &batch[4..] {
        assert!(!runtime.rejects_body_event(event, body, *sequence));
    }
    // A genuine press at the new boundary may start a gesture at the new hit
    // region, with no surviving owner from the stale press.
    let press = Event::ButtonPress(button(BUTTON_PRESS_EVENT, BOUNDARY, 14));
    assert!(!runtime.rejects_body_event(&press, body, BOUNDARY));
    assert_eq!(
        runtime
            .interaction
            .handle_right_press_on_body(14, runtime.body_hit(Point::new(55, 55))),
        HostAction::TrackOpening
    );
    runtime.pointer.track_automatic_right().unwrap();
    runtime
        .fence_visibility_input_with(
            &mut window,
            || Ok(BOUNDARY + 3),
            |_, _| Ok(()),
            |_| panic!("opening did not create a menu"),
        )
        .unwrap();
    assert!(!runtime.pointer.is_grabbed());
    assert!(!runtime.body_input_ready());
    runtime.unavailable(true);
    assert!(!runtime
        .frame_tick_ready(due + period, &window, false, None)
        .unwrap());
    assert!(paint_committed_with(&runtime.frames, |_| panic!("destroyed body")).is_err());
}

#[test]
fn synthetic_tick_uses_host_deadlines_and_full_delay_after_eligibility_resumes() {
    let now = Instant::now();
    let (mut runtime, mut window, mut presentation, period) = session(now);
    let due = now + period;
    assert!(!runtime
        .frame_tick_ready(due, &window, false, Some(due))
        .unwrap());
    assert!(!runtime.frame_tick_ready(due, &window, true, None).unwrap());
    runtime.pointer.require_transition_release();
    assert!(!runtime
        .frame_tick_ready(due + period, &window, false, None)
        .unwrap());
    assert!(runtime.frame_timing.deadline().is_none());
    runtime.pointer.release_with(|| Ok(())).unwrap();

    let resumed = due + period * 2;
    assert!(!runtime
        .frame_tick_ready(resumed, &window, false, None)
        .unwrap());
    runtime.correction = Some((Point::new(1, 2), resumed + period));
    assert!(!runtime
        .frame_tick_ready(resumed + period, &window, false, None)
        .unwrap());
    runtime.correction = None;
    runtime.menu = Some(crate::MenuPopup::test_popup(200, 0));
    assert!(!runtime
        .frame_tick_ready(resumed + period * 2, &window, false, None)
        .unwrap());
    runtime.menu = None;
    assert!(!runtime
        .frame_tick_ready(resumed + period * 3, &window, true, None)
        .unwrap());
    assert!(runtime.frame_timing.deadline().is_none());
    assert_eq!(runtime.frames.last_committed(), Some(INITIAL));
    let resumed = resumed + period * 4;
    assert!(!runtime
        .frame_tick_ready(resumed, &window, false, None)
        .unwrap());
    assert_eq!(runtime.frame_timing.deadline(), Some(resumed + period));
    let late = resumed + period * 5;
    assert!(runtime
        .frame_tick_ready(late, &window, false, None)
        .unwrap());
    CheckedTransport::default()
        .present(
            &mut runtime,
            &mut window,
            &mut presentation,
            FrameKind::Scheduled,
            [late, late + period],
        )
        .unwrap();
    assert_eq!(runtime.frames.last_committed(), INITIAL.checked_next());
    assert_eq!(runtime.frame_timing.deadline(), Some(late + period * 2));
    assert!(!runtime
        .frame_tick_ready(late + period, &window, false, None)
        .unwrap());
}

#[test]
fn animated_startup_and_deferred_show_restore_the_committed_input_with_expose_independent() {
    let now = Instant::now();
    let (mut runtime, mut window, mut presentation, period) = session(now);
    let body = window.window;
    let map = MapNotifyEvent {
        response_type: MAP_NOTIFY_EVENT,
        event: body,
        window: body,
        ..Default::default()
    };
    // Use the real initial commit, then drive the same repaint, placement and
    // one-shot checked SHAPE seams as the genuine startup MapNotify route.
    runtime.body = BodyAvailability::Validating;
    runtime.input_fence.enabled_after = None;
    runtime.arm_map_input_sync(OLD_BOUNDARY - 1);
    let trace = RefCell::new(Vec::new());
    paint_committed_with(&runtime.frames, |_| {
        trace.borrow_mut().push("startup repaint");
        Ok(())
    })
    .unwrap();
    let origin = window.confirmed_origin();
    let placement = InputPlacement {
        selected_monitor: 20,
        observed_monitor: 20,
        geometry: Rect::new(origin.x, origin.y, 160, 160),
        bounds: ValidOriginBounds::new(0, 500, 0, 500),
    };
    runtime
        .placement_with(
            &mut window,
            placement.geometry,
            &placement.bounds,
            |_, _| panic!("already in bounds"),
        )
        .unwrap();
    runtime
        .synchronize_placed_map_input_with(&map, &window, OLD_BOUNDARY - 1, |region| {
            assert!(region.contains(Point::new(30, 55)));
            trace.borrow_mut().push("startup checked shape");
            Ok(OLD_BOUNDARY)
        })
        .unwrap();
    runtime
        .synchronize_placed_map_input_with(&map, &window, OLD_BOUNDARY + 1, |_| {
            panic!("duplicate map")
        })
        .unwrap();
    let transport = CheckedTransport::default();
    let due = now + period * 2;
    transport
        .present(
            &mut runtime,
            &mut window,
            &mut presentation,
            FrameKind::Scheduled,
            [due; 2],
        )
        .unwrap();
    let identity = runtime.frames.require_committed().unwrap().identity;
    runtime.frame_timing.update_eligibility(due, false).unwrap();
    runtime
        .fence_visibility_input_with(
            &mut window,
            || Ok(BOUNDARY + 1),
            |_, _| Ok(()),
            |_| panic!("no popup"),
        )
        .unwrap();
    let remap = due + Duration::from_secs(10);
    assert!(!runtime
        .frame_tick_ready(remap, &window, false, None)
        .unwrap());
    assert!(runtime.frame_timing.deadline().is_none());
    let visible = VisibilityEvidence {
        destroyed: false,
        wm_state: WmState::Normal,
        map_state: Evidence::Readable(MapState::Viewable),
        hidden: Evidence::Readable(false),
        workspace: Workspace::Current,
        capability: Capability::Supported,
        coherent: true,
    };
    runtime.body = BodyAvailability::Validating;
    runtime
        .placement_with(
            &mut window,
            placement.geometry,
            &placement.bounds,
            |_, _| panic!("already in bounds"),
        )
        .unwrap();
    let remote = VisibilityEvidence {
        workspace: Workspace::Other,
        ..visible
    };
    assert!(runtime
        .restore_visibility_input_with(&window, remote, &placement, |_| panic!(
            "off-workspace shape"
        ))
        .is_err());
    runtime
        .restore_visibility_input_with(&window, visible, &placement, |region| {
            restore_body_input_shape_with(body, region, |request| {
                assert_rectangles_match_image(&request.rectangles, &transport.image.borrow());
                trace.borrow_mut().push("Show/deferred checked shape");
                Ok(BOUNDARY + 2)
            })
        })
        .unwrap();
    assert!(runtime.body_hit(Point::new(90, 55)));
    assert!(!runtime.body_hit(Point::new(30, 55)));
    // This seam can enable input before an Expose has repainted the remap.
    // The native experiment must determine the visible pixels in that interval.
    paint_committed_with(&runtime.frames, |_| {
        trace.borrow_mut().push("Expose repaint");
        Ok(())
    })
    .unwrap();
    assert_eq!(
        *trace.borrow(),
        [
            "startup repaint",
            "startup checked shape",
            "Show/deferred checked shape",
            "Expose repaint"
        ]
    );
    assert_eq!(runtime.frames.last_committed(), Some(identity));
    assert!(!runtime
        .frame_tick_ready(remap, &window, false, None)
        .unwrap());
    assert_eq!(runtime.frame_timing.deadline(), Some(remap + period));
}

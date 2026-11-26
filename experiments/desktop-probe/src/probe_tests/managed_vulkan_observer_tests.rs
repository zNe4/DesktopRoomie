use super::*;
use crate::managed_supervisor::{Fixture, ManagedSupervisor};
use crate::presentation::PreparedFrame;
use crate::x11::render::{CoverageCutover, FrameGeometry, PreparedPublication};
use crate::{
    body_presentation::{BodyPresentation, FrameTiming},
    AnimatedCoverage, BodyAvailability, ManagedProbeWindow, ProbeRuntime,
};
use roomie_worker_supervisor::machine::{Phase, Policy};
use std::cell::Cell;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::Instant;

const SESSION: u64 = 9;
const FILE: FileIdentity = FileIdentity {
    device: 1,
    inode: 888,
    bytes: 300,
    changed_seconds: 40,
    changed_nanos: 4,
};
const OWNER: Cohort = Cohort {
    negative: false,
    hardware: false,
    session: SESSION,
    completions: 6,
    owner_proof: 127,
    worker: Some(FILE),
    driver: Some(FILE),
    last_job: None,
};

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn frame(serial: u64, elapsed: Duration) -> (FrameRequest, PreparedFrame<Vec<u8>>) {
    let request = FrameRequest::new(
        FrameIdentity {
            generation: 0,
            sequence: serial,
        },
        ActionIntent {
            action: Action::Idle,
            facing: Facing::Neutral,
        },
        ClientExtent::new(Size::new(160, 160)).unwrap(),
        elapsed,
        elapsed.min(ms(42)),
    )
    .unwrap();
    let frame = SyntheticProducer::new().prepare_frame(&request).unwrap();
    (request, frame)
}

struct Replies {
    target: Target,
    live: LiveBody,
    native: NativeReplies,
}

fn replies(frame: &PreparedFrame<Vec<u8>>, at: Duration) -> Replies {
    let target = Target {
        identity: frame.identity,
        body: 6,
        pixmap: 7,
        origin: Point::new(24, 55),
        enabled_after: 65_537,
        input_blocked: false,
        temporary_empty_confirmed: false,
        interaction_suppressed: false,
    };
    let rectangles: Vec<_> = frame
        .metadata
        .hit_region()
        .spans()
        .iter()
        .rev()
        .map(|span| Rectangle {
            x: span.start_x as i16,
            y: span.y as i16,
            width: (span.end_x - span.start_x) as u16,
            height: 1,
        })
        .collect();
    Replies {
        target,
        live: LiveBody {
            map_state: 2,
            visual: 3,
            depth: 32,
            override_redirect: false,
            root: 1,
            width: 160,
            height: 160,
            border: 0,
            same_screen: true,
            origin: target.origin,
        },
        native: NativeReplies {
            layout: Layout {
                visual: 3,
                pict_format: 4,
                masks: [255; 4],
                shifts: [0, 8, 16, 24],
                format_depth: 32,
                bpp: 32,
                image_order: 0,
                pad: 32,
                reply_depth: 0,
                reply_visual: 0,
                reply_bytes: 0,
            },
            image_sequence: 65_538,
            shape_sequence: 65_539,
            image_at: at,
            shape_at: at + ms(1),
            image: GetImageReply {
                depth: 32,
                sequence: 2,
                visual: 0,
                data: frame.payload.clone(),
            },
            shape: GetRectanglesReply {
                ordering: ClipOrdering::UNSORTED,
                sequence: 3,
                length: rectangles.len() as u32 * 2,
                rectangles,
            },
        },
    }
}

fn queue(
    capture: &mut Capture,
    serial: u64,
    elapsed: Duration,
) -> (JobIdentity, ReadyFrame<Vec<u8>>, Duration) {
    let (request, frame) = frame(serial, elapsed);
    let job = JobIdentity {
        session: SESSION,
        serial,
        epoch: 2,
        frame: frame.identity,
    };
    let at = ms(serial * 300);
    capture.admitted(job, Some(at), request, Some(at + ms(100)));
    capture.event(Kind::Frame, Some(job), Some(at + ms(2)));
    capture.event(Kind::Ready, Some(job), Some(at + ms(2)));
    let ready = ReadyFrame {
        identity: job,
        request,
        deadline: Instant::now(),
        frame,
    };
    (job, ready, at)
}

fn committed(capture: &mut Capture, serial: u64, elapsed: Duration) -> Replies {
    let (job, ready, at) = queue(capture, serial, elapsed);
    capture.select(&ready, Some(at + ms(100)));
    capture.event(Kind::Admitted, Some(job), Some(at + ms(3)));
    capture.event(Kind::Committed, Some(job), Some(at + ms(4)));
    replies(&ready.frame, at + ms(5))
}

fn observe(capture: &mut Capture, replies: Replies) {
    capture.observe_pending(
        &mut None::<u64>,
        || Ok(None),
        || Some(replies.target),
        |_| Ok(replies.live),
        |_| Ok(replies.native),
    );
}

fn phase_capture(phases: &[u64]) -> Capture {
    let mut capture = Capture::new();
    capture.event(Kind::Startup, None, None);
    for (index, phase) in phases.iter().enumerate() {
        let native = committed(&mut capture, index as u64 + 1, ms(300 * phase));
        observe(&mut capture, native);
        capture.event(Kind::Expose, None, None);
    }
    capture.event(Kind::Shutdown, None, Some(ms(9999)));
    capture
}

fn serialize(capture: &Capture, cohort: Cohort) -> Vec<u8> {
    let mut bytes = Vec::new();
    let last_job = capture
        .events
        .iter()
        .rev()
        .find(|event| event.kind == Kind::Admit)
        .and_then(|event| event.job);
    capture
        .write(&mut bytes, Cohort { last_job, ..cohort }, true)
        .unwrap();
    bytes
}

fn verification(bytes: &[u8]) -> std::result::Result<String, String> {
    verify(Cursor::new(bytes), |_| Ok(()))
}

#[test]
fn original_request_and_deadline_round_trip_without_time_quantization() {
    let (request, _) = frame(78, Duration::new(u64::MAX - 9, 299_999_999));
    let job = JobIdentity {
        session: u64::MAX,
        epoch: 444,
        serial: 112,
        frame: request.identity(),
    };
    let at = Duration::new(u64::MAX - 4, 999_999_997);
    let deadline = at + ms(100);
    let event = Event {
        kind: Kind::Admit,
        at: Some(at),
        job: Some(job),
        admission: Some((request, deadline)),
    };
    let mut bytes = Vec::new();
    event.write(&mut bytes).unwrap();
    let parsed = Event::read(&mut Cursor::new(&bytes)).unwrap();
    assert_eq!(parsed.at, Some(at));
    assert_eq!(parsed.job, Some(job));
    assert_eq!(parsed.admission, Some((request, deadline)));
    bytes[42 + 1 + 8..42 + 1 + 12].fill(255);
    assert!(Event::read(&mut Cursor::new(bytes)).is_err());
}

#[test]
fn capacity_accounts_for_container_allocations_and_checked_overflow() {
    let mut budget = usize::MAX;
    assert!(allocated::<u64>(usize::MAX, &mut budget).is_err());
    assert!(allocated::<u8>(usize::MAX, &mut budget).is_err());
    assert!(Capture::allocate(1024).is_err());
    let capture = Capture::allocate(MAX_BYTES).unwrap();
    assert!(
        size_of::<Capture>()
            + capture.events.capacity() * size_of::<Event>()
            + capture.samples.capacity() * size_of::<Sample>()
            + capture.data.capacity()
            <= MAX_BYTES
    );
}

#[test]
fn exhaustion_and_second_pending_sample_mark_capture_incomplete() {
    let mut capture = Capture::new();
    for _ in 0..EVENTS + 1 {
        capture.event(Kind::Expose, None, None);
    }
    assert_eq!(capture.events.len(), EVENTS);
    assert!(capture.failure().is_some());
    assert!(!serialize(&capture, OWNER).ends_with(FOOTER));

    let mut capture = Capture::new();
    let (_, ready, at) = queue(&mut capture, 1, ms(0));
    capture.select(&ready, Some(at + ms(100)));
    capture.select(&ready, Some(at + ms(100)));
    assert_eq!(capture.samples.len(), 1);
    assert!(capture.failure().is_some());

    let mut capture = Capture::new();
    for serial in 1..=SAMPLES as u64 {
        let native = committed(&mut capture, serial, ms(0));
        observe(&mut capture, native);
    }
    let native = committed(&mut capture, SAMPLES as u64 + 1, ms(0));
    observe(&mut capture, native);
    assert_eq!(capture.samples.len(), SAMPLES);
    assert!(capture.failure().is_some());
}

#[test]
fn injected_native_replies_must_match_the_same_target_and_sequences() {
    for mismatch in 0..6 {
        let mut capture = Capture::new();
        let mut native = committed(&mut capture, 1, ms(0));
        let before = native.target;
        let mut after = before;
        let mut live = native.live;
        match mismatch {
            0 => after.body += 1,
            1 => after.pixmap += 1,
            2 => after.identity.sequence += 1,
            3 => live.map_state = 0,
            4 => native.native.shape.sequence += 1,
            5 => after.enabled_after = 0,
            _ => unreachable!(),
        }
        let snapshots = Cell::new(0);
        let live_reads = Cell::new(0);
        capture.observe_pending(
            &mut None::<u64>,
            || Ok(None),
            || {
                snapshots.set(snapshots.get() + 1);
                Some(if snapshots.get() == 1 { before } else { after })
            },
            |_| {
                live_reads.set(live_reads.get() + 1);
                Ok(if live_reads.get() == 1 {
                    native.live
                } else {
                    live
                })
            },
            |_| Ok(native.native),
        );
        assert!(capture.failure().is_some(), "case {mismatch}");
    }
}

#[test]
fn invalid_layout_and_truncated_or_oversized_native_images_fail() {
    for mismatch in 0..9 {
        let mut capture = Capture::new();
        let mut native = committed(&mut capture, 1, ms(300));
        match mismatch {
            0 => native.native.image.depth = 24,
            1 => native.native.layout.bpp = 24,
            2 => native.native.layout.pad = 8,
            3 => native.native.layout.image_order = 1,
            4 => native.native.layout.shifts[0] = 1,
            5 => native.native.layout.masks[0] = 127,
            6 => {
                native.native.image.data.pop();
            }
            7 => native.native.image.data.push(0),
            8 => native.native.image.visual = 3,
            _ => unreachable!(),
        }
        observe(&mut capture, native);
        assert!(capture.failure().is_some(), "case {mismatch}");
    }
}

#[test]
fn rectangle_union_accepts_equivalent_decomposition_and_disjoint_coverage() {
    let whole = [
        Rectangle {
            x: 2,
            y: 7,
            width: 3,
            height: 2,
        },
        Rectangle {
            x: 125,
            y: 5,
            width: 2,
            height: 1,
        },
    ];
    let split = [
        whole[1],
        Rectangle {
            x: 2,
            y: 7,
            width: 3,
            height: 1,
        },
        Rectangle {
            x: 2,
            y: 8,
            width: 3,
            height: 1,
        },
    ];
    let mask = rectangle_mask(whole.into_iter(), whole.len()).unwrap();
    assert_eq!(
        mask,
        rectangle_mask(split.into_iter(), split.len()).unwrap()
    );
    assert_eq!(mask.iter().map(|value| value.count_ones()).sum::<u32>(), 8);
    assert_eq!(rectangle_mask(std::iter::empty(), 0).unwrap(), [0; MASK]);
}

#[test]
fn malformed_and_oversized_rectangle_replies_fail() {
    for (x, y, width, height) in [
        (-1, 0, 1, 1),
        (0, -1, 1, 1),
        (160, 0, 1, 1),
        (0, 159, 1, 2),
        (0, 0, 0, 1),
        (0, 0, 1, 0),
        (1, 1, u16::MAX, 1),
    ] {
        assert!(rectangle_mask(
            [Rectangle {
                x,
                y,
                width,
                height
            }]
            .into_iter(),
            1
        )
        .is_err());
    }
    assert!(rectangle_mask(std::iter::empty(), RECTANGLES + 1).is_err());
    let mut capture = Capture::new();
    let mut native = committed(&mut capture, 1, ms(300));
    native.native.shape.length += 1;
    observe(&mut capture, native);
    assert!(capture.failure().is_some());
}

#[test]
fn all_cpu_fixture_phases_verify_against_injected_pixmap_and_shape() {
    let capture = phase_capture(&[0, 1, 2, 3, 4, 5]);
    assert!(verification(&serialize(&capture, OWNER))
        .unwrap()
        .contains("6 commitments"));
    let empty = &capture.samples[4];
    assert_eq!(
        (empty.bounds, empty.anchor, empty.rectangles),
        (None, None, 0)
    );
}

#[test]
fn pixel_pma_bounds_anchor_and_shape_mismatches_fail_offline() {
    for mismatch in 0..6 {
        let mut capture = phase_capture(&[0, 1, 2, 3, 4, 5]);
        match mismatch {
            0 => capture.data[0] = 255,
            1 => capture.data[4 * (40 * 160 + 17)] ^= 1,
            2 => capture.data[IMAGE + MASK] ^= 1,
            3 => capture.samples[0].bounds = None,
            4 => capture.samples[0].anchor = Some(Point::new(0, 0)),
            5 => {
                capture.samples[0].rectangles = 0;
                capture.samples[0].readback.as_mut().unwrap().shape_words = 0;
            }
            _ => unreachable!(),
        }
        assert!(
            verification(&serialize(&capture, OWNER)).is_err(),
            "case {mismatch}"
        );
    }
}

#[test]
fn startup_expose_and_unobserved_phases_cannot_fill_cycle_gaps() {
    for phases in [
        &[0, 1, 2, 3, 4][..],
        &[0, 1, 2, 8, 9, 10],
        &[0, 2, 3, 4, 5],
        &[],
    ] {
        let capture = phase_capture(phases);
        assert!(verification(&serialize(
            &capture,
            Cohort {
                completions: phases.len() as u64,
                ..OWNER
            }
        ))
        .is_err());
    }
    let capture = phase_capture(&[0, 0, 1, 2, 3, 4, 5]);
    assert!(verification(&serialize(
        &capture,
        Cohort {
            completions: 7,
            ..OWNER
        }
    ))
    .is_ok());
}

#[test]
fn reconciliation_distinguishes_physical_returns_from_skips_and_commitments() {
    let mut capture = phase_capture(&[0, 1, 2, 3, 4, 5]);
    let shutdown = capture.events.pop().unwrap();
    let (request, _) = frame(7, ms(1800));
    let job = JobIdentity {
        session: SESSION,
        epoch: 2,
        serial: 7,
        frame: request.identity(),
    };
    capture.admitted(job, Some(ms(2100)), request, Some(ms(2200)));
    capture.event(Kind::CanceledPending, Some(job), Some(ms(2120)));
    capture.event(Kind::Expired, Some(job), Some(ms(2200)));
    capture.event(Kind::Obsolete, Some(job), Some(ms(2220)));
    let (job, ready, at) = queue(&mut capture, 8, ms(2100));
    capture.select(&ready, Some(at + ms(100)));
    capture.event(Kind::RefusedExpired, Some(job), Some(at + ms(100)));
    capture.event(Kind::Skipped, Some(job), None);
    capture.discard();
    capture.record(shutdown);
    let accepted = serialize(
        &capture,
        Cohort {
            completions: 8,
            ..OWNER
        },
    );
    assert!(verification(&accepted).unwrap().contains("commitments: 6"));
    assert!(verification(&serialize(&capture, OWNER)).is_err());
    capture.events.insert(
        capture.events.len() - 1,
        capture.events[capture.events.len() - 2],
    );
    assert!(verification(&serialize(
        &capture,
        Cohort {
            completions: 8,
            ..OWNER
        }
    ))
    .is_err());
}

#[test]
fn bounded_format_rejects_truncation_invalid_fields_duplicate_ids_and_extra_bytes() {
    let mut capture = phase_capture(&[0, 1, 2, 3, 4, 5]);
    let mut bytes = serialize(&capture, OWNER);
    for cut in [0, HEADER.len(), bytes.len() / 2, bytes.len() - 1] {
        assert!(verification(&bytes[..cut]).is_err());
    }
    bytes.push(0);
    assert!(verification(&bytes).is_err());
    bytes[HEADER.len() + 19..HEADER.len() + 27].fill(255);
    assert!(verification(&bytes).is_err());
    capture.samples[1].job = capture.samples[0].job;
    assert!(verification(&serialize(&capture, OWNER)).is_err());
    assert!(verify(
        Cursor::new(serialize(&phase_capture(&[0, 1, 2, 3, 4, 5]), OWNER)),
        |_| Err("worker contradiction".into())
    )
    .is_err());
}

#[test]
fn zero_work_expected_rejection_requires_distinct_owner_evidence() {
    let capture = phase_capture(&[]);
    let negative = Cohort {
        negative: true,
        completions: 0,
        owner_proof: 255,
        ..OWNER
    };
    assert!(verification(&serialize(&capture, negative))
        .unwrap()
        .contains("initialization rejection"));
    assert!(verification(&serialize(
        &capture,
        Cohort {
            owner_proof: 127,
            ..negative
        }
    ))
    .is_err());
    assert!(verification(&serialize(
        &capture,
        Cohort {
            completions: 1,
            ..negative
        }
    ))
    .is_err());
    assert!(verification(&serialize(
        &capture,
        Cohort {
            negative: false,
            owner_proof: 127,
            ..negative
        }
    ))
    .is_err());
}

fn checked_owner() -> (
    ProbeRuntime,
    ManagedProbeWindow,
    BodyPresentation<SyntheticProducer>,
    Instant,
) {
    use crate::body_presentation::{BodyPresentation, FrameKind};
    let origin = Instant::now();
    let mut presentation = BodyPresentation::new(SyntheticProducer::new()).unwrap();
    let mut runtime = ProbeRuntime::with_presentation(
        presentation.initial_extent(),
        FrameTiming::new(Some(ms(300))),
    );
    runtime.body = BodyAvailability::Ready;
    runtime.animated_shape_limit = Some(262_140);
    let window = ManagedProbeWindow {
        window_resource: Default::default(),
        colormap_resource: Default::default(),
        window: 6,
        colormap: 2,
        wm_delete_window: 3,
        wm_protocols: 4,
        requested_origin: Point::new(24, 55),
        confirmed_origin: Point::new(24, 55),
        in_flight_moves: Default::default(),
        superseded_moves: Default::default(),
        width: 160,
        height: 160,
    };
    runtime
        .present_frame_with(
            &mut presentation,
            FrameKind::Initial,
            false,
            || origin,
            |frames, request, frame, input, pointer| {
                let mut coverage = AnimatedCoverage {
                    input,
                    pointer,
                    maximum_request_bytes: 262_140,
                    set_input: |_| Ok(65_537),
                    release: || Ok(()),
                };
                frames
                    .publish_with_admission(
                        request,
                        frame,
                        |_, _| Ok(()),
                        |_| Ok(()),
                        Some(&mut coverage),
                        |_, _| Ok(true),
                    )
                    .map(|publication| match publication {
                        PreparedPublication::Committed(sequence) => sequence,
                        PreparedPublication::Skipped => panic!("startup skipped"),
                    })
            },
            |_| Ok(()),
        )
        .unwrap();
    // Simulate the accepted checked MapNotify readiness boundary.
    runtime.input_fence.enabled_after = Some(65_537);
    (runtime, window, presentation, origin)
}

#[test]
fn owner_snapshot_rejects_fenced_destroyed_invalidated_and_unsettled_targets() {
    let (mut runtime, mut window, _, _) = checked_owner();
    let capture_target = |runtime: &ProbeRuntime, window: &ManagedProbeWindow| {
        crate::observation_target(runtime, window, 7, false)
    };
    assert!(capture_target(&runtime, &window).is_some());
    runtime.input_fence.blocked = true;
    assert!(capture_target(&runtime, &window).is_none());
    runtime.input_fence.blocked = false;
    runtime.input_fence.empty_confirmed = true;
    assert!(capture_target(&runtime, &window).is_none());
    runtime.input_fence.empty_confirmed = false;
    window.requested_origin.x += 1;
    assert!(capture_target(&runtime, &window).is_none());
    window.requested_origin = window.confirmed_origin;
    runtime.body = BodyAvailability::Destroyed;
    assert!(capture_target(&runtime, &window).is_none());
    runtime.body = BodyAvailability::Ready;
    runtime.frames.invalidate();
    assert!(capture_target(&runtime, &window).is_none());
}

fn fake_worker() -> PathBuf {
    std::env::var_os("DESKTOPROOMIE_MANAGED_FIXTURE")
        .expect("explicit fake-worker required")
        .into()
}

fn supervise(
    runtime: &mut ProbeRuntime,
    origin: Instant,
    now: Instant,
    check: impl Fn(&ManagedSupervisor) -> bool,
) {
    let managed = runtime.managed.as_mut().unwrap();
    let limit = Instant::now() + Duration::from_secs(4);
    while !check(managed) {
        assert!(
            Instant::now() < limit,
            "fixture never reached expected phase"
        );
        let ready = managed.wait_until(Instant::now() + ms(20)).unwrap();
        managed.progress(ready, now.max(origin)).unwrap();
    }
}

#[test]
fn actual_managed_turn_records_refusal_and_timing_failure_without_changing_authority() {
    run_checked_turn(true, false, false);
    run_checked_turn(false, true, false);
    run_checked_turn(false, false, true);
}

fn run_checked_turn(refuse: bool, timing_failure: bool, exhausted: bool) {
    let (mut runtime, mut window, mut presentation, origin) = checked_owner();
    let worker = fake_worker();
    let mut managed = ManagedSupervisor::spawn(
        Fixture {
            executable: &worker,
            args: &["normal"],
            session: SESSION,
            policy: Policy::default(),
        },
        origin,
    )
    .unwrap();
    let mut evidence = Capture::new();
    evidence.event(Kind::Startup, None, None);
    if exhausted {
        evidence.fail("injected capture failure");
    }
    managed.evidence = Some(evidence);
    runtime.managed = Some(managed);
    supervise(&mut runtime, origin, origin, |managed| {
        managed.outcome().phase == Phase::Ready && managed.drained()
    });
    runtime
        .refresh_frame_eligibility(origin, &window, false)
        .unwrap();
    let admitted = origin + ms(300);
    let request = runtime
        .frames
        .frame_request(
            ActionIntent {
                action: Action::Idle,
                facing: Facing::Neutral,
            },
            ms(300),
            ms(40),
        )
        .unwrap();
    assert!(runtime
        .managed
        .as_mut()
        .unwrap()
        .admit(request, admitted)
        .unwrap()
        .is_some());
    let observed = admitted + ms(10);
    supervise(&mut runtime, origin, observed, |managed| {
        managed.ready().is_some() && managed.drained()
    });
    let mut buffered = None;
    let peeks = Cell::new(0);
    let uploads = Cell::new(0);
    let clock = Cell::new(observed);
    let result = runtime.managed_presentation_turn(
        &mut window,
        &mut presentation,
        None,
        || false,
        || None,
        || clock.get(),
        &mut buffered,
        || {
            peeks.set(peeks.get() + 1);
            Ok((refuse && peeks.get() == 2).then_some(91))
        },
        |frames: &mut FrameGeometry, request, frame, input, pointer, admit| {
            let mut coverage = AnimatedCoverage {
                input,
                pointer,
                maximum_request_bytes: 262_140,
                set_input: |_| Ok(65_538u64),
                release: || Ok(()),
            };
            frames.publish_with_admission(
                request,
                frame,
                |_, _| {
                    uploads.set(uploads.get() + 1);
                    Ok(())
                },
                |_| {
                    if timing_failure {
                        clock.set(origin);
                    }
                    Ok(())
                },
                Some(&mut coverage as &mut dyn CoverageCutover),
                admit,
            )
        },
        |_, _| Ok(()),
    );
    let managed = runtime.managed.as_mut().unwrap();
    let recorded = &managed.evidence.as_ref().unwrap().events;
    let has = |kind| recorded.iter().any(|event| event.kind == kind);
    if refuse {
        assert!(!result.unwrap());
        assert_eq!(buffered, Some(91));
        assert!(has(Kind::Refused) && has(Kind::Skipped) && !has(Kind::Committed));
        assert_eq!(uploads.get(), 0);
    } else if timing_failure {
        assert!(result.is_err());
        assert_eq!(uploads.get(), 1);
        assert!(
            has(Kind::Admitted)
                && has(Kind::Committed)
                && has(Kind::Unusable)
                && !has(Kind::Observed)
        );
    } else {
        assert!(result.unwrap());
        assert_eq!(uploads.get(), 1);
        assert!(managed.evidence.as_ref().unwrap().failure().is_some());
    }
    let stop = Instant::now();
    assert!(managed
        .reap_after_host_cleanup_with(|| observed + stop.elapsed())
        .is_ok());
}

#[test]
fn buffered_event_is_preserved_and_makes_observation_incomplete() {
    for found_at_end in [false, true] {
        let mut capture = Capture::new();
        let native = committed(&mut capture, 1, ms(0));
        let polls = Cell::new(0);
        let queries = Cell::new(0);
        let mut buffered = None;
        capture.observe_pending(
            &mut buffered,
            || {
                polls.set(polls.get() + 1);
                Ok((polls.get() == if found_at_end { 2 } else { 1 }).then_some(95))
            },
            || Some(native.target),
            |_| Ok(native.live),
            |_| {
                queries.set(queries.get() + 1);
                Ok(native.native)
            },
        );
        assert_eq!(buffered, Some(95));
        assert_eq!(queries.get(), usize::from(found_at_end));
        assert!(capture.failure().is_some());
        assert!(!capture
            .events
            .iter()
            .any(|event| event.kind == Kind::Observed));
    }
}

#[test]
fn launcher_flushes_a_distinct_negative_proof_and_keeps_host_error_first() {
    use crate::managed_vulkan::{verify_evidence, NativeVulkanLaunch, OwnerObservation};
    use roomie_worker_supervisor::machine::{Failure, Outcome, Shutdown};
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "roomie-negative-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    {
        let worker = fake_worker();
        let driver = root.join("unavailable.json");
        fs::write(&driver, r#"{"file_format_version":"1.0.0","ICD":{"library_path":"/nonexistent/desktoproomie-unavailable-vulkan.so","api_version":"1.3.0"}}"#).unwrap();
        let launch = |leaf: &str| {
            NativeVulkanLaunch::parse([
                "--worker",
                worker.to_str().unwrap(),
                "--driver",
                driver.to_str().unwrap(),
                "--duration",
                "2",
                "--adapter",
                "hardware",
                "--session",
                "9",
                "--report-directory",
                root.join(leaf).to_str().unwrap(),
                "--evidence",
                "expected-initialization-rejection",
            ])
            .unwrap()
        };
        let observation = || OwnerObservation {
            supervisor: Some(Outcome {
                session: SESSION,
                initialize_transmitted: true,
                job: None,
                phase: Phase::Exited,
                failure: Some(Failure::BackendFault),
                unresolved: None,
                stopped: true,
                reaped: true,
                termination_requested: false,
                shutdown: Shutdown::Confirmed,
            }),
            ..OwnerObservation::default()
        };
        let mut report = String::from("roomie_acceptance_report=1\ncomplete=0\nkind_hex=6e61746976652d76756c6b616e\nexit=Controlled\ntelemetry_available=1\n");
        for key in [
            "initialization_complete",
            "initialization_ns",
            "device_initialization_ns",
            "pipeline_initialization_ns",
            "submission_attempts",
            "accepted_submissions",
            "confirmed_completions",
            "native_gpu_completions",
            "map_requests",
            "map_callbacks",
            "successful_maps",
            "unmaps",
            "timed_operations",
            "staging_allocations",
            "staging_reuses",
            "staging_retirements",
            "peak_outstanding",
            "encode_ns",
            "submit_ns",
            "wait_ns",
            "map_ns",
            "repack_ns",
            "uncertain_release",
            "telemetry_error",
        ] {
            report.push_str(&format!("{key}=0\n"));
        }
        report.push_str("controlled_release=1\ncomplete=1\n");
        let write = |launch: &NativeVulkanLaunch| {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(launch.report_directory().join("worker.telemetry"))
                .unwrap()
                .write_all(report.as_bytes())
                .unwrap();
        };
        let good = launch("negative");
        write(&good);
        let successful = good.classify(
            Some(phase_capture(&[])),
            (true, Some(0)),
            None,
            None,
            observation(),
        );
        assert!(successful.successful(), "{:?}", successful.failures);
        assert!(verify_evidence(good.report_directory())
            .unwrap()
            .contains("initialization rejection"));
        let unrelated = launch("bad");
        write(&unrelated);
        let failure = unrelated.classify(
            Some(phase_capture(&[])),
            (false, Some(0)),
            None,
            None,
            observation(),
        );
        assert!(!failure.successful());
        assert!(verify_evidence(unrelated.report_directory()).is_err());
        // A previously written evidence path causes an exclusive-write error.
        let failure = good.classify(
            Some(phase_capture(&[])),
            (true, Some(0)),
            Some("original host failure".into()),
            Some("later reap failure".into()),
            observation(),
        );
        assert_eq!(failure.failures[0], "host: original host failure");
        assert_eq!(failure.failures[1], "reaping: later reap failure");
        assert!(failure.failures.last().unwrap().contains("evidence write"));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checked_owner_and_worker_fixture_produce_an_offline_verifiable_cycle() {
    use std::cell::RefCell;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "roomie-i2-{}-{}.sock",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let (mut runtime, mut window, mut presentation, origin) = checked_owner();
    let worker = fake_worker();
    let mut managed = ManagedSupervisor::spawn(
        Fixture {
            executable: &worker,
            args: &["managed-script", path.to_str().unwrap()],
            session: SESSION,
            policy: Policy::default(),
        },
        origin,
    )
    .unwrap();
    let mut capture = Capture::new();
    capture.event(Kind::Startup, None, None);
    managed.evidence = Some(capture);
    runtime.managed = Some(managed);
    supervise(&mut runtime, origin, origin, |managed| {
        managed.outcome().phase == Phase::Ready && managed.drained()
    });
    runtime
        .refresh_frame_eligibility(origin, &window, false)
        .unwrap();
    let connected_by = Instant::now() + Duration::from_secs(3);
    let mut gate = loop {
        match listener.accept() {
            Ok((gate, _)) => break gate,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < connected_by);
                std::thread::sleep(ms(1));
            }
            Err(error) => panic!("{error}"),
        }
    };
    std::fs::remove_file(path).unwrap();
    gate.set_read_timeout(Some(Duration::from_secs(3))).unwrap();

    // These are injected checked server states. The bytes come from the
    // transport upload and the rectangles from checked installation.
    let image = RefCell::new(Vec::<u8>::new());
    let startup_shape = runtime
        .frames
        .require_committed()
        .unwrap()
        .metadata
        .hit_region()
        .spans()
        .iter()
        .map(|span| Rectangle {
            x: span.start_x as i16,
            y: span.y as i16,
            width: (span.end_x - span.start_x) as u16,
            height: 1,
        })
        .collect::<Vec<_>>();
    let shape = RefCell::new(startup_shape);
    for interval in 5..11 {
        let now = origin + ms(interval * 300);
        let mut buffered = None::<u32>;
        let mut turn = |runtime: &mut ProbeRuntime| {
            runtime
                .managed_presentation_turn(
                    &mut window,
                    &mut presentation,
                    None,
                    || false,
                    || None,
                    || now,
                    &mut buffered,
                    || Ok(None),
                    |frames, request, frame, input, pointer, admit| {
                        let mut coverage = AnimatedCoverage {
                            input,
                            pointer,
                            maximum_request_bytes: 262_140,
                            set_input: |rectangles| {
                                *shape.borrow_mut() = rectangles;
                                Ok(65_537u64)
                            },
                            release: || Ok(()),
                        };
                        frames.publish_with_admission(
                            request,
                            frame,
                            |_, pixels| {
                                *image.borrow_mut() = pixels.to_vec();
                                Ok(())
                            },
                            |_| Ok(()),
                            Some(&mut coverage),
                            admit,
                        )
                    },
                    |_, _| panic!("checked transport should stay usable"),
                )
                .unwrap()
        };
        assert!(!turn(&mut runtime));
        supervise(&mut runtime, origin, now, |managed| {
            managed.outcome().job.is_some_and(|job| {
                job.transmission == roomie_worker_supervisor::machine::Transmission::Complete
            })
        });
        let mut marker = [0];
        gate.read_exact(&mut marker).unwrap();
        assert_eq!(marker, *b"R");
        gate.write_all(b"F").unwrap();
        gate.read_exact(&mut marker).unwrap();
        assert_eq!(marker, *b"W");
        supervise(&mut runtime, origin, now, |managed| {
            managed.ready().is_some() && managed.drained()
        });
        assert!(turn(&mut runtime));
        let target = crate::observation_target(&runtime, &window, 7, false).unwrap();
        let managed = runtime.managed.as_mut().unwrap();
        let capture = managed.evidence.as_mut().unwrap();
        let shape = shape.borrow().clone();
        let native = NativeReplies {
            layout: Layout {
                visual: 3,
                pict_format: 4,
                masks: [255; 4],
                shifts: [0, 8, 16, 24],
                format_depth: 32,
                bpp: 32,
                image_order: 0,
                pad: 32,
                reply_depth: 0,
                reply_visual: 0,
                reply_bytes: 0,
            },
            image_sequence: 65_538,
            shape_sequence: 65_539,
            image_at: ms(interval * 300),
            shape_at: ms(interval * 300),
            image: GetImageReply {
                depth: 32,
                sequence: 2,
                visual: 0,
                data: image.borrow().clone(),
            },
            shape: GetRectanglesReply {
                ordering: ClipOrdering::UNSORTED,
                sequence: 3,
                length: shape.len() as u32 * 2,
                rectangles: shape,
            },
        };
        capture.observe_pending(
            &mut buffered,
            || Ok(None),
            || Some(target),
            |_| {
                Ok(LiveBody {
                    map_state: 2,
                    visual: 3,
                    depth: 32,
                    override_redirect: false,
                    root: 1,
                    width: 160,
                    height: 160,
                    border: 0,
                    same_screen: true,
                    origin: target.origin,
                })
            },
            |_| Ok(native),
        );
        assert_eq!(capture.failure(), None);
    }
    let managed = runtime.managed.as_mut().unwrap();
    let stop = Instant::now();
    managed
        .reap_after_host_cleanup_with(|| origin + ms(3300) + stop.elapsed())
        .unwrap();
    let capture = managed.evidence.as_ref().unwrap();
    assert!(verification(&serialize(capture, OWNER))
        .unwrap()
        .contains("6 commitments"));
}

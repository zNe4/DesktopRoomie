//! A test-only adapter witness. The reusable crates have no desktop dependency.
//! This does not connect the worker or GPU reports to the desktop event loop.
use desktop_probe::presentation::{
    self as p,
    gpu_handoff::{CompletionStatus, GpuHandoff, GpuJob, JobCompletion, JobIdentity},
};
use roomie_worker_protocol::{self as wire, Body, Message};
use roomie_worker_supervisor::machine::{Machine, Policy, Report};
use std::time::{Duration, Instant};

fn to_wire(job: &GpuJob) -> Result<wire::Render, &'static str> {
    let id = job.identity();
    let request = job.request();
    if request.extent().size() != p::Size::new(160, 160) {
        return Err("synthetic extent only");
    }
    Ok(wire::Render {
        job: wire::JobKey {
            epoch: id.epoch,
            serial: id.serial,
            generation: id.frame.generation,
            sequence: id.frame.sequence,
        },
        action: match request.intent().action {
            p::Action::Idle => wire::Action::Idle,
            p::Action::Walk => wire::Action::Walk,
        },
        facing: match request.intent().facing {
            p::Facing::Neutral => wire::Facing::Neutral,
            p::Facing::FrontRight => wire::Facing::FrontRight,
            p::Facing::FrontLeft => wire::Facing::FrontLeft,
            p::Facing::BackRight => wire::Facing::BackRight,
            p::Facing::BackLeft => wire::Facing::BackLeft,
        },
        elapsed: request.active_elapsed(),
        delta: request.active_delta(),
    })
}
fn to_owner(session: u64, job: wire::JobKey) -> JobIdentity {
    JobIdentity {
        session,
        epoch: job.epoch,
        serial: job.serial,
        frame: p::FrameIdentity {
            generation: job.generation,
            sequence: job.sequence,
        },
    }
}
#[test]
fn original_frame_request_remains_in_d1_and_round_trip_keeps_every_identity_field() {
    let extent = p::ClientExtent::new(p::Size::new(160, 160)).unwrap();
    let request = p::FrameRequest::new(
        p::FrameIdentity {
            generation: 9,
            sequence: 17,
        },
        p::ActionIntent {
            action: p::Action::Idle,
            facing: p::Facing::Neutral,
        },
        extent,
        Duration::from_millis(600),
        Duration::from_millis(10),
    )
    .unwrap();
    let now = Instant::now();
    let mut owner = GpuHandoff::new(27);
    owner.resume().unwrap();
    let job = owner
        .schedule(request, now, now + Duration::from_millis(100))
        .unwrap();
    let wire = to_wire(&job).unwrap();
    assert_eq!(to_owner(27, wire.job), job.identity());
    assert_eq!(wire.elapsed, request.active_elapsed());
    assert_eq!(wire.delta, request.active_delta());
    let mut supervisor = Machine::new(27, Policy::default(), Duration::ZERO);
    supervisor.initialize_transmitted();
    supervisor.receive(
        Message {
            session: 27,
            body: Body::Ready,
        },
        Duration::ZERO,
    );
    supervisor.admit(wire, Duration::ZERO).unwrap();
    supervisor.render_transmitted(wire.job, true);
    supervisor.receive(
        Message {
            session: 27,
            body: Body::Frame {
                job: wire.job,
                bgra: vec![0; wire::FRAME_BYTES],
            },
        },
        Duration::from_millis(1),
    );
    let Some(Report::Frame { job: key, bgra }) = supervisor.take_report() else {
        panic!("frame");
    };
    let prepared = p::PreparedFrame {
        identity: request.identity(),
        payload: bgra,
        metadata: p::FrameMetadata::new(extent, None, None, p::HitRegion::new(extent, []).unwrap())
            .unwrap(),
    };
    assert_eq!(
        owner.complete(to_owner(27, key), JobCompletion::Frame(prepared), now),
        Ok(CompletionStatus::Ready)
    );
    let (original, frame) = owner.take_ready().unwrap();
    assert_eq!(original, request);
    assert_eq!(frame.payload.len(), wire::FRAME_BYTES);
}
#[test]
fn adapter_rejects_extent_without_mutating_existing_presentation_types() {
    let mut owner: GpuHandoff<()> = GpuHandoff::new(1);
    owner.resume().unwrap();
    let request = p::FrameRequest::new(
        p::FrameIdentity {
            generation: 1,
            sequence: 1,
        },
        p::ActionIntent {
            action: p::Action::Walk,
            facing: p::Facing::BackRight,
        },
        p::ClientExtent::new(p::Size::new(3, 2)).unwrap(),
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let now = Instant::now();
    assert_eq!(
        to_wire(
            &owner
                .schedule(request, now, now + Duration::from_secs(1))
                .unwrap()
        ),
        Err("synthetic extent only")
    );
}

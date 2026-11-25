use std::time::Duration;

use super::*;
use crate::presentation::{
    Action, ActionIntent, ClientExtent, Facing, FrameMetadata, HitRegion, PublicationError,
    PublicationState, Size,
};
use crate::x11::render::FrameGeometry;

const SESSION: u64 = 12;
const ELAPSED: Duration = Duration::from_millis(37);
const DELTA: Duration = Duration::from_millis(3);
const INTENT: ActionIntent = ActionIntent {
    action: Action::Idle,
    facing: Facing::Neutral,
};

fn extent() -> ClientExtent {
    ClientExtent::new(Size::new(3, 2)).unwrap()
}

fn request(sequence: u64) -> FrameRequest {
    FrameRequest::new(
        FrameIdentity {
            generation: 4,
            sequence,
        },
        INTENT,
        extent(),
        ELAPSED,
        DELTA,
    )
    .unwrap()
}

fn frame<P>(request: FrameRequest, payload: P) -> PreparedFrame<P> {
    let extent = request.extent();
    let region = HitRegion::new(extent, []).unwrap();
    PreparedFrame {
        identity: request.identity(),
        metadata: FrameMetadata::new(extent, None, None, region).unwrap(),
        payload,
    }
}

fn time_window() -> (Instant, Instant) {
    let now = Instant::now();
    (now, now + Duration::from_millis(11))
}

fn active<P>() -> GpuHandoff<P> {
    let mut handoff = GpuHandoff::new(SESSION);
    handoff.resume().unwrap();
    handoff
}

fn admit<P>(
    handoff: &mut GpuHandoff<P>,
    request: FrameRequest,
    now: Instant,
    deadline: Instant,
) -> JobIdentity {
    handoff.schedule(request, now, deadline).unwrap().identity()
}

fn report_frame(
    handoff: &mut GpuHandoff<()>,
    identity: JobIdentity,
    request: FrameRequest,
    now: Instant,
) -> Result<CompletionStatus, CompletionError> {
    handoff.complete(identity, JobCompletion::Frame(frame(request, ())), now)
}

#[test]
fn original_request_and_owned_result_are_handed_off_once() {
    let mut handoff = GpuHandoff::new(SESSION);
    let (now, deadline) = time_window();
    let original = request(9);
    assert_eq!(
        handoff.schedule(original, now, deadline),
        Err(ScheduleRejection::Paused)
    );
    handoff.resume().unwrap();
    let job = handoff.schedule(original, now, deadline).unwrap();
    assert_eq!(job.request(), original);
    let id = job.identity();
    assert_eq!(
        (id.session, id.epoch, id.serial, id.frame),
        (SESSION, 0, 1, original.identity())
    );
    assert_eq!(handoff.outstanding(), Some(id));
    assert_eq!(handoff.deadline(), Some(deadline));
    assert!(handoff.take_ready().is_none());

    assert_eq!(
        handoff.complete(id, JobCompletion::Frame(frame(original, vec![7])), now),
        Ok(CompletionStatus::Ready)
    );
    assert_eq!(handoff.outstanding(), None);
    assert_eq!(handoff.deadline(), None);
    assert_eq!(
        handoff.schedule(request(10), now, deadline),
        Err(ScheduleRejection::ResultPending)
    );
    let (returned, result) = handoff.take_ready().unwrap();
    assert_eq!(returned, original);
    assert_eq!(result.identity, original.identity());
    assert_eq!(result.payload, vec![7]);
    assert!(handoff.take_ready().is_none());
}

#[test]
fn incorrect_job_tags_and_duplicates_leave_the_slot_unchanged() {
    let mut handoff = active();
    let (now, deadline) = time_window();
    let original = request(1);
    let id = admit(&mut handoff, original, now, deadline);
    for wrong in [
        JobIdentity {
            session: SESSION + 1,
            ..id
        },
        JobIdentity {
            epoch: id.epoch + 1,
            ..id
        },
        JobIdentity {
            serial: id.serial + 1,
            ..id
        },
        JobIdentity {
            frame: request(2).identity(),
            ..id
        },
    ] {
        assert_eq!(
            handoff.complete(wrong, JobCompletion::Frame(frame(original, 0)), now),
            Err(CompletionError::UnexpectedJob)
        );
        assert_eq!(handoff.outstanding(), Some(id));
        assert_eq!(handoff.deadline(), Some(deadline));
    }
    assert_eq!(
        handoff.complete(id, JobCompletion::Frame(frame(original, 1)), now),
        Ok(CompletionStatus::Ready)
    );
    assert_eq!(
        handoff.complete(id, JobCompletion::Frame(frame(original, 2)), now),
        Err(CompletionError::UnexpectedJob)
    );
    assert_eq!(handoff.take_ready().unwrap().1.payload, 1);
    assert_eq!(
        handoff.complete(id, JobCompletion::Frame(frame(original, 3)), now),
        Err(CompletionError::UnexpectedJob)
    );
}

#[test]
fn a_settled_frame_with_the_wrong_frame_identity_is_rejected() {
    let mut handoff = active();
    let (now, deadline) = time_window();
    let id = admit(&mut handoff, request(1), now, deadline);
    assert_eq!(
        handoff.complete(id, JobCompletion::Frame(frame(request(2), ())), now),
        Err(CompletionError::FrameIdentityMismatch)
    );
    assert_eq!(handoff.outstanding(), None);
    assert!(handoff.take_ready().is_none());
    assert!(handoff.schedule(request(2), now, deadline).is_ok());
}

#[test]
fn prior_session_completion_cannot_settle_an_identical_new_request() {
    let (now, deadline) = time_window();
    let original = request(1);
    let mut previous = active::<()>();
    let old_id = previous
        .schedule(original, now, deadline)
        .unwrap()
        .identity();
    previous
        .complete(old_id, JobCompletion::RejectedBeforeSubmission, now)
        .unwrap();
    previous.shutdown();

    let mut handoff = GpuHandoff::new(SESSION + 1);
    handoff.resume().unwrap();
    let current = admit(&mut handoff, original, now, deadline);
    assert_eq!(
        current,
        JobIdentity {
            session: SESSION + 1,
            ..old_id
        }
    );
    assert_eq!(
        report_frame(&mut handoff, old_id, original, now),
        Err(CompletionError::UnexpectedJob)
    );
    assert_eq!(handoff.outstanding(), Some(current));
    assert_eq!(
        report_frame(&mut handoff, current, original, now),
        Ok(CompletionStatus::Ready)
    );
}

#[test]
fn interruption_cancels_outstanding_work_logically_and_discards_late_success() {
    let mut handoff = active();
    let (now, deadline) = time_window();
    let original = request(1);
    let id = admit(&mut handoff, original, now, deadline);
    handoff.interrupt().unwrap();
    handoff.interrupt().unwrap();
    assert_eq!(handoff.outstanding(), Some(id));
    assert_eq!(handoff.deadline(), Some(deadline));
    assert_eq!(
        handoff.schedule(request(2), now, deadline),
        Err(ScheduleRejection::Paused)
    );
    handoff.resume().unwrap();
    assert_eq!(
        handoff.schedule(request(2), now, deadline),
        Err(ScheduleRejection::Outstanding)
    );
    assert_eq!(
        report_frame(&mut handoff, id, original, now),
        Ok(CompletionStatus::Obsolete)
    );
    assert_eq!(handoff.outstanding(), None);
    assert!(handoff.take_ready().is_none());

    // The host can issue the same frame identity again before it commits one.
    let next = admit(&mut handoff, original, now, deadline);
    assert_eq!(
        next,
        JobIdentity {
            epoch: id.epoch + 1,
            serial: id.serial + 1,
            ..id
        }
    );
    assert_eq!(
        report_frame(&mut handoff, id, original, now),
        Err(CompletionError::UnexpectedJob)
    );
    assert_eq!(handoff.outstanding(), Some(next));
}

#[test]
fn interrupt_drops_a_completed_frame_even_if_resumed_before_consumption() {
    let mut handoff = active();
    let (now, deadline) = time_window();
    let original = request(1);
    let old = admit(&mut handoff, original, now, deadline);
    assert_eq!(
        report_frame(&mut handoff, old, original, now),
        Ok(CompletionStatus::Ready)
    );
    handoff.interrupt().unwrap();
    handoff.resume().unwrap();
    assert!(handoff.take_ready().is_none());
    let newer = request(2);
    let current = admit(&mut handoff, newer, now, deadline);
    assert_eq!(
        report_frame(&mut handoff, old, original, now),
        Err(CompletionError::UnexpectedJob)
    );
    assert_eq!(handoff.outstanding(), Some(current));
    assert_eq!(
        report_frame(&mut handoff, current, newer, now),
        Ok(CompletionStatus::Ready)
    );
    assert_eq!(handoff.take_ready().unwrap().0, newer);
}

#[test]
fn dispatch_rejection_and_worker_failure_remain_distinct_and_settle_the_job() {
    let mut handoff: GpuHandoff<()> = active();
    let (now, deadline) = time_window();
    let original = request(1);
    let id = admit(&mut handoff, original, now, deadline);
    assert_eq!(
        handoff.complete(id, JobCompletion::RejectedBeforeSubmission, now),
        Ok(CompletionStatus::RejectedBeforeSubmission)
    );
    assert_eq!(handoff.outstanding(), None);
    assert!(handoff.take_ready().is_none());
    let next = admit(&mut handoff, request(2), now, deadline);
    handoff.interrupt().unwrap();
    assert_eq!(
        handoff.complete(next, JobCompletion::Failed("map failed".into()), now),
        Ok(CompletionStatus::Failed("map failed".into()))
    );
    assert_eq!(handoff.outstanding(), None);
    assert!(handoff.take_ready().is_none());
}

#[test]
fn device_failure_survives_stale_frame_and_preserves_physical_debt() {
    let mut handoff = active();
    let (now, deadline) = time_window();
    let original = request(1);
    let id = admit(&mut handoff, original, now, deadline);
    handoff.interrupt().unwrap();
    handoff.resume().unwrap();
    handoff.device_failed();
    assert_eq!(handoff.outstanding(), Some(id));
    assert_eq!(handoff.resume(), Err(CloseReason::DeviceFailure));
    assert_eq!(
        handoff.schedule(request(2), now, deadline),
        Err(ScheduleRejection::Closed(CloseReason::DeviceFailure))
    );
    assert_eq!(
        report_frame(&mut handoff, id, original, now),
        Ok(CompletionStatus::Obsolete)
    );
    assert_eq!(handoff.outstanding(), None);
    assert!(handoff.take_ready().is_none());
    handoff.shutdown();
    assert_eq!(handoff.resume(), Err(CloseReason::DeviceFailure));
}

#[test]
fn device_failure_discards_a_ready_result() {
    let mut handoff = active();
    let (now, deadline) = time_window();
    let original = request(1);
    let id = admit(&mut handoff, original, now, deadline);
    assert_eq!(
        report_frame(&mut handoff, id, original, now),
        Ok(CompletionStatus::Ready)
    );
    handoff.device_failed();
    assert!(handoff.take_ready().is_none());
    assert_eq!(handoff.resume(), Err(CloseReason::DeviceFailure));
}

#[test]
fn unresolved_report_holds_the_slot_and_revokes_publication_until_settlement() {
    let mut handoff = active();
    let (now, deadline) = time_window();
    let original = request(1);
    let id = admit(&mut handoff, original, now, deadline);
    assert_eq!(
        handoff.complete(id, JobCompletion::Unresolved("poll uncertain".into()), now),
        Ok(CompletionStatus::Unresolved("poll uncertain".into()))
    );
    assert_eq!(handoff.deadline(), None);
    assert_eq!(handoff.outstanding(), Some(id));
    assert!(handoff.take_ready().is_none());
    assert_eq!(
        handoff.schedule(request(2), now, deadline),
        Err(ScheduleRejection::Outstanding)
    );
    assert_eq!(
        report_frame(&mut handoff, id, original, now),
        Ok(CompletionStatus::Obsolete)
    );
    assert_eq!(handoff.outstanding(), None);
    assert!(handoff.schedule(request(2), now, deadline).is_ok());
}

#[test]
fn deadline_expiry_is_reported_once_and_late_completion_cannot_publish() {
    let (now, deadline) = time_window();
    let original = request(1);
    let next_deadline = deadline + DELTA;
    for service_timer_first in [true, false] {
        let mut handoff = active();
        assert_eq!(
            handoff.schedule(original, now, now),
            Err(ScheduleRejection::DeadlineExpired)
        );
        let id = admit(&mut handoff, original, now, deadline);
        assert_eq!(handoff.expire(now), None);
        if service_timer_first {
            assert_eq!(handoff.expire(deadline), Some(id));
            assert_eq!(handoff.expire(next_deadline), None);
            assert_eq!(handoff.deadline(), None);
        }
        assert_eq!(handoff.outstanding(), Some(id));
        assert_eq!(
            handoff.schedule(request(2), deadline, next_deadline),
            Err(ScheduleRejection::Outstanding)
        );
        assert_eq!(
            report_frame(&mut handoff, id, original, deadline),
            Ok(CompletionStatus::Obsolete)
        );
        assert_eq!(handoff.outstanding(), None);
        assert!(handoff.take_ready().is_none());
        assert!(handoff
            .schedule(request(2), deadline, next_deadline)
            .is_ok());
    }
}

#[test]
fn serial_exhaustion_closes_admission_without_wraparound() {
    let mut handoff: GpuHandoff<()> = active();
    handoff.serial = u64::MAX - 1;
    let (now, deadline) = time_window();
    let id = admit(&mut handoff, request(1), now, deadline);
    assert_eq!(id.serial, u64::MAX);
    assert_eq!(
        handoff.schedule(request(2), now, deadline),
        Err(ScheduleRejection::Outstanding)
    );
    handoff
        .complete(id, JobCompletion::RejectedBeforeSubmission, now)
        .unwrap();
    assert_eq!(
        handoff.schedule(request(2), now, deadline),
        Err(ScheduleRejection::Closed(CloseReason::SerialExhausted))
    );
    assert_eq!(handoff.serial, u64::MAX);
    assert_eq!(handoff.resume(), Err(CloseReason::SerialExhausted));
}

#[test]
fn epoch_exhaustion_revokes_eligibility_without_forgetting_the_job() {
    let mut handoff = active();
    handoff.epoch = u64::MAX - 1;
    let (now, deadline) = time_window();
    handoff.interrupt().unwrap();
    handoff.resume().unwrap();
    let original = request(1);
    let id = admit(&mut handoff, original, now, deadline);
    assert_eq!(id.epoch, u64::MAX);
    assert_eq!(handoff.interrupt(), Err(CloseReason::EpochExhausted));
    assert_eq!(handoff.epoch, u64::MAX);
    assert_eq!(handoff.outstanding(), Some(id));
    assert_eq!(handoff.resume(), Err(CloseReason::EpochExhausted));
    assert_eq!(
        report_frame(&mut handoff, id, original, now),
        Ok(CompletionStatus::Obsolete)
    );
    assert!(handoff.take_ready().is_none());
}

#[test]
fn resumption_does_not_queue_paused_or_busy_requests() {
    let mut handoff = active();
    let (now, deadline) = time_window();
    let original = request(1);
    let id = admit(&mut handoff, original, now, deadline);
    assert_eq!(
        handoff.schedule(request(2), now, deadline),
        Err(ScheduleRejection::Outstanding)
    );
    handoff.interrupt().unwrap();
    let resumed = deadline + ELAPSED;
    let next_deadline = resumed + DELTA;
    assert_eq!(
        handoff.schedule(request(3), resumed, next_deadline),
        Err(ScheduleRejection::Paused)
    );
    assert_eq!(
        report_frame(&mut handoff, id, original, resumed),
        Ok(CompletionStatus::Obsolete)
    );
    handoff.resume().unwrap();
    handoff.resume().unwrap();
    assert_eq!(handoff.outstanding(), None);
    assert_eq!(handoff.deadline(), None);
    assert!(handoff.take_ready().is_none());
    let fresh = request(4);
    let job = handoff.schedule(fresh, resumed, next_deadline).unwrap();
    assert_eq!(job.identity().serial, id.serial + 1);
    assert_eq!(job.request(), fresh);
    assert_eq!(
        handoff.schedule(request(5), resumed, next_deadline),
        Err(ScheduleRejection::Outstanding)
    );
}

#[test]
fn shutdown_tracks_pending_work_and_drops_completed_work() {
    let (now, deadline) = time_window();
    let original = request(1);
    for ready in [false, true] {
        let mut handoff = active();
        let id = admit(&mut handoff, original, now, deadline);
        if ready {
            assert_eq!(
                report_frame(&mut handoff, id, original, now),
                Ok(CompletionStatus::Ready)
            );
        }
        handoff.shutdown();
        handoff.shutdown();
        assert_eq!(handoff.resume(), Err(CloseReason::Shutdown));
        assert!(handoff.take_ready().is_none());
        assert_eq!(handoff.outstanding(), (!ready).then_some(id));
        if !ready {
            assert_eq!(handoff.expire(deadline), Some(id));
            assert_eq!(handoff.outstanding(), Some(id));
            assert_eq!(
                report_frame(&mut handoff, id, original, deadline),
                Ok(CompletionStatus::Obsolete)
            );
        }
        assert_eq!(handoff.outstanding(), None);
        assert!(handoff.take_ready().is_none());
    }
}

#[test]
fn gpu_readiness_neither_submits_nor_commits_and_host_still_validates() {
    let mut geometry = FrameGeometry::new(extent());
    let original = geometry.frame_request(INTENT, ELAPSED, DELTA).unwrap();
    let identity = original.identity();
    let mut handoff = active();
    let (now, deadline) = time_window();
    for payload in [vec![0], vec![0; 24]] {
        let valid_size = payload.len() == 24;
        let job = handoff.schedule(original, now, deadline).unwrap();
        let completion = JobCompletion::Frame(frame(original, payload));
        assert_eq!(
            handoff.complete(job.identity(), completion, now),
            Ok(CompletionStatus::Ready)
        );
        let (request, frame) = handoff.take_ready().unwrap();
        assert_eq!(
            geometry.publication_state(identity),
            Err(PublicationError::IdentityMismatch)
        );
        assert!(geometry.committed().is_none());
        let result = geometry.publish_with(
            &request,
            frame,
            |_, _| {
                assert!(valid_size, "invalid frame reached upload");
                Ok(())
            },
            |_| {
                assert!(valid_size, "invalid frame reached blit");
                Ok(())
            },
        );
        if valid_size {
            result.unwrap();
        } else {
            assert_eq!(
                result.unwrap_err().to_string(),
                "BGRA8 payload has an unexpected byte length"
            );
            assert!(geometry.committed().is_none());
        }
    }
    assert_eq!(
        geometry.publication_state(identity),
        Ok(PublicationState::HostCommitted)
    );
    assert_eq!(geometry.committed().unwrap().identity, identity);
}

#[test]
fn ready_retains_exact_admission_until_one_shot_transfer_or_expiry() {
    let (now, deadline) = time_window();
    let original = request(2);
    let mut handoff = active();
    let first = admit(&mut handoff, original, now, deadline);
    assert_eq!(
        report_frame(
            &mut handoff,
            first,
            original,
            deadline - Duration::from_millis(1)
        ),
        Ok(CompletionStatus::Ready)
    );
    assert_eq!(handoff.wait_deadline(), Some(deadline));
    let candidate = handoff.take_ready_frame().unwrap();
    assert_eq!(candidate.identity, first);
    assert_eq!(candidate.deadline, deadline);
    assert_eq!(candidate.request, original);
    assert!(handoff.take_ready().is_none());
    let later = deadline + Duration::from_millis(1);
    let second = admit(&mut handoff, original, now, later);
    assert_ne!(first, second);
    assert_eq!(first.frame, second.frame);
    assert_eq!(
        report_frame(&mut handoff, second, original, deadline),
        Ok(CompletionStatus::Ready)
    );
    assert_eq!(handoff.wait_deadline(), Some(later));
    assert_eq!(handoff.expire(later), Some(second));
    assert_eq!(handoff.expire(later), None);
    assert!(handoff.take_ready_frame().is_none());
    assert_eq!(handoff.wait_deadline(), None);
    assert_eq!(handoff.outstanding(), None);
}

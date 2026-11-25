//! GPU-free owner turns: D1 owns all job state; the real host seam publishes.
use super::*;
use crate::presentation::gpu_handoff::{
    CloseReason, CompletionError, CompletionStatus, GpuHandoff, GpuJob, JobCompletion,
    ScheduleRejection,
};

struct Fixture {
    runtime: ProbeRuntime,
    window: ManagedProbeWindow,
    presentation: BodyPresentation<SyntheticProducer>,
    handoff: GpuHandoff<Vec<u8>>,
    origin: Instant,
}

impl Fixture {
    fn new() -> Self {
        let origin = Instant::now();
        let (runtime, window, presentation, _) = session(origin);
        let mut handoff = GpuHandoff::new(17);
        handoff.resume().unwrap();
        Self {
            runtime,
            window,
            presentation,
            handoff,
            origin,
        }
    }

    fn at(&self, ms: u64) -> Instant {
        self.origin + Duration::from_millis(ms)
    }

    fn request(&mut self, ms: u64) -> FrameRequest {
        let now = self.at(ms);
        self.presentation
            .frame_request(
                FrameKind::Scheduled,
                &self.runtime.frames,
                &mut self.runtime.frame_timing,
                || now,
            )
            .unwrap()
    }

    fn schedule(&mut self, ms: u64, expiry: u64) -> GpuJob {
        let request = self.request(ms);
        self.handoff
            .schedule(request, self.at(ms), self.at(expiry))
            .unwrap()
    }

    fn frame(job: &GpuJob) -> PreparedFrame<Vec<u8>> {
        // This is explicitly a CPU fake completion, not worker/Vulkan evidence.
        SyntheticProducer::new()
            .prepare_frame(&job.request())
            .unwrap()
    }

    fn complete(&mut self, job: &GpuJob, ms: u64) -> CompletionStatus {
        self.handoff
            .complete(
                job.identity(),
                JobCompletion::Frame(Self::frame(job)),
                self.at(ms),
            )
            .unwrap()
    }

    /// No event dispatch or retained consumed candidate between take and publish.
    /// The only persistent job authority in this fixture is the accepted D1.
    fn owner_turn(
        &mut self,
        transport: &CheckedTransport,
        observed: u64,
        expiry: u64,
        validation_end: u64,
        completed: u64,
        owner_busy: bool,
    ) -> Result<bool, HostError> {
        let observed = self.at(observed);
        let deadline = self.at(expiry);
        self.handoff.expire(observed);
        let eligible = self.runtime.frame_eligible(&self.window, owner_busy);
        if !eligible || observed >= deadline {
            self.handoff.interrupt().unwrap();
            return Ok(false);
        }
        let Some((request, frame)) = self.handoff.take_ready() else {
            return Ok(false);
        };
        let clock = Cell::new(observed);
        let validation_end = self.at(validation_end);
        let completed = self.at(completed);
        let body = self.window.window;
        let window = &mut self.window;
        self.runtime.publish_prepared_with(
            request,
            frame,
            eligible,
            || {
                assert!(
                    clock.get() < deadline,
                    "skipped candidates have no completion clock read"
                );
                completed
            },
            |frames, request, frame, input, pointer, _| {
                let mut coverage = AnimatedCoverage {
                    input,
                    pointer,
                    maximum_request_bytes: REQUEST_LIMIT,
                    set_input: |rectangles: Vec<Rectangle>| {
                        let step = if rectangles.is_empty()
                            && transport.trace.borrow().contains(&Step::EmptyShape)
                        {
                            Step::FinalShape
                        } else if rectangles.is_empty() {
                            Step::EmptyShape
                        } else {
                            Step::FinalShape
                        };
                        transport.set_shape(body, step, rectangles)
                    },
                    release: || transport.check(Step::Release),
                };
                // Advance time inside actual host validation, before admission.
                let mut policy = ValidationClock {
                    coverage: RecordBlock(&mut coverage, &transport.trace),
                    clock: &clock,
                    end: validation_end,
                };
                frames.publish_with_admission(
                    request,
                    frame,
                    |dimensions, bytes| {
                        assert_eq!(dimensions, (160, 160));
                        transport.check(Step::Upload)?;
                        *transport.image.borrow_mut() = bytes.to_vec();
                        Ok(())
                    },
                    |_| {
                        checked_blit_with(
                            || transport.check(Step::Blit),
                            || transport.check(Step::Flush),
                        )
                    },
                    Some(&mut policy),
                    |_, _| Ok(clock.get() < deadline),
                )
            },
            |runtime| {
                runtime.fence_visibility_input_with(
                    window,
                    || transport.set_shape(body, Step::SafetyEmpty, vec![]),
                    |_, _| transport.check(Step::SafetyRelease),
                    |_| panic!("no popup in this owner turn"),
                )
            },
        )
    }

    fn assert_usable_initial(&self) {
        assert_eq!(
            self.runtime.frames.require_committed().unwrap().identity,
            INITIAL
        );
        assert!(self.runtime.body_input_ready());
        assert!(!self.runtime.frames.publication_uncertain());
    }
}

struct ValidationClock<'a, C> {
    coverage: C,
    clock: &'a Cell<Instant>,
    end: Instant,
}

impl<C: CoverageCutover> CoverageCutover for ValidationClock<'_, C> {
    fn maximum_request_bytes(&self) -> usize {
        self.clock.set(self.end);
        self.coverage.maximum_request_bytes()
    }
    fn block_input(&mut self) {
        self.coverage.block_input()
    }
    fn empty_and_release(&mut self) -> Result<(), HostError> {
        self.coverage.empty_and_release()
    }
    fn enable(&mut self, rectangles: Vec<Rectangle>) -> Result<u64, HostError> {
        self.coverage.enable(rectangles)
    }
}

#[test]
fn immediate_fake_completion_uses_shared_image_only_publication_and_timing() {
    let mut f = Fixture::new();
    let job = f.schedule(300, 500);
    assert_eq!(f.complete(&job, 300), CompletionStatus::Ready);
    let transport = CheckedTransport::default();
    assert!(f.owner_turn(&transport, 300, 500, 301, 302, false).unwrap());
    assert_eq!(*transport.trace.borrow(), IMAGE_ONLY);
    assert_eq!(
        transport.image.borrow().as_slice(),
        Fixture::frame(&job).payload
    );
    assert_eq!(
        f.runtime.frames.require_committed().unwrap().identity,
        job.request().identity()
    );
    assert_eq!(f.runtime.frame_timing.deadline(), Some(f.at(602)));
    assert!(f.handoff.take_ready().is_none());
    assert!(f.handoff.outstanding().is_none());
}

#[test]
fn delayed_completion_and_expose_use_only_committed_pixels_until_owner_publication() {
    let mut f = Fixture::new();
    let job = f.schedule(600, 900);
    let deadline = f.runtime.frame_timing.deadline();
    let transport = CheckedTransport::default();
    assert!(!f.owner_turn(&transport, 650, 900, 650, 650, false).unwrap());
    let mut repaints = 0;
    paint_committed_with(&f.runtime.frames, |dimensions| {
        assert_eq!(dimensions, (160, 160));
        assert_eq!(
            f.runtime.frames.require_committed().unwrap().identity,
            INITIAL
        );
        repaints += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(repaints, 1);
    assert!(transport.trace.borrow().is_empty());
    assert_eq!(f.runtime.frame_timing.deadline(), deadline);
    f.assert_usable_initial();
    assert_eq!(f.complete(&job, 700), CompletionStatus::Ready);
    // Completion alone is not X11 commitment or successful presentation.
    f.assert_usable_initial();
    assert_eq!(f.runtime.frame_timing.deadline(), deadline);
    assert!(f.owner_turn(&transport, 700, 900, 701, 702, false).unwrap());
    assert_eq!(*transport.trace.borrow(), CUTOVER);
    assert_eq!(f.runtime.input_fence.enabled_after, Some(BOUNDARY));
    assert!(!f.runtime.input_fence.blocked);
}

#[test]
fn repeated_due_opportunities_cannot_admit_backlog_or_duplicate_jobs() {
    let mut f = Fixture::new();
    let job = f.schedule(300, 900);
    for ms in 301..365 {
        let request = f.request(ms);
        assert_eq!(
            f.handoff.schedule(request, f.at(ms), f.at(900)),
            Err(ScheduleRejection::Outstanding)
        );
        assert_eq!(f.handoff.outstanding(), Some(job.identity()));
    }
    assert_eq!(f.complete(&job, 400), CompletionStatus::Ready);
    assert_eq!(
        f.handoff.schedule(job.request(), f.at(400), f.at(900)),
        Err(ScheduleRejection::ResultPending)
    );
    let transport = CheckedTransport::default();
    assert!(f.owner_turn(&transport, 400, 900, 400, 400, false).unwrap());
    assert!(!f.owner_turn(&transport, 401, 900, 401, 401, false).unwrap());
    assert_eq!(*transport.trace.borrow(), IMAGE_ONLY);
}

#[test]
fn gesture_interruption_obsoletes_prospective_frame_without_settling_pending_work() {
    let mut f = Fixture::new();
    let job = f.schedule(600, 900);
    let deadline = f.runtime.frame_timing.deadline();
    f.handoff.interrupt().unwrap();
    assert_eq!(f.handoff.outstanding(), Some(job.identity()));
    f.handoff.resume().unwrap();
    assert_eq!(
        f.handoff.schedule(job.request(), f.at(650), f.at(900)),
        Err(ScheduleRejection::Outstanding)
    );
    assert_eq!(f.complete(&job, 700), CompletionStatus::Obsolete);
    let transport = CheckedTransport::default();
    assert!(!f.owner_turn(&transport, 700, 900, 700, 700, false).unwrap());
    assert!(transport.trace.borrow().is_empty());
    assert_eq!(f.runtime.frame_timing.deadline(), deadline);
    f.assert_usable_initial();
    assert!(f.handoff.outstanding().is_none());
}

#[test]
fn pause_resume_reused_sequence_keeps_original_request_and_rejects_old_job() {
    let mut f = Fixture::new();
    let old = f.schedule(600, 1000);
    f.runtime
        .frame_timing
        .update_eligibility(f.at(650), false)
        .unwrap();
    f.handoff.interrupt().unwrap();
    f.runtime
        .frame_timing
        .update_eligibility(f.at(800), true)
        .unwrap();
    f.handoff.resume().unwrap();
    assert_eq!(f.complete(&old, 850), CompletionStatus::Obsolete);
    let next = f.schedule(1100, 1500);
    assert_eq!(old.request().identity(), next.request().identity());
    assert_ne!(old.identity().epoch, next.identity().epoch);
    assert_ne!(old.identity().serial, next.identity().serial);
    assert_ne!(
        old.request().active_elapsed(),
        next.request().active_elapsed()
    );
    assert_eq!(
        f.handoff.complete(
            old.identity(),
            JobCompletion::Frame(Fixture::frame(&old)),
            f.at(1110)
        ),
        Err(CompletionError::UnexpectedJob)
    );
    // A later construction reuses the unpublished sequence with different time.
    let reconstructed = f.request(1400);
    assert_eq!(reconstructed.identity(), next.request().identity());
    assert_ne!(
        reconstructed.active_elapsed(),
        next.request().active_elapsed()
    );
    assert_eq!(f.complete(&next, 1410), CompletionStatus::Ready);
    let transport = CheckedTransport::default();
    assert!(f
        .owner_turn(&transport, 1410, 1500, 1411, 1412, false)
        .unwrap());
    assert_eq!(
        transport.image.borrow().as_slice(),
        Fixture::frame(&next).payload
    );
    assert_ne!(
        transport.image.borrow().as_slice(),
        SyntheticProducer::new()
            .prepare_frame(&reconstructed)
            .unwrap()
            .payload
    );
    assert_eq!(f.runtime.frame_timing.deadline(), Some(f.at(1712)));
}

#[test]
fn wrong_session_epoch_serial_and_frame_cannot_discharge_or_publish_the_current_job() {
    let mut f = Fixture::new();
    let job = f.schedule(600, 900);
    let identity = job.identity();
    let mut wrong = [identity; 5];
    wrong[0].session += 1;
    wrong[1].epoch += 1;
    wrong[2].serial += 1;
    wrong[3].frame.generation += 1;
    wrong[4].frame.sequence += 1;
    for identity in wrong {
        assert_eq!(
            f.handoff.complete(
                identity,
                JobCompletion::Frame(Fixture::frame(&job)),
                f.at(650)
            ),
            Err(CompletionError::UnexpectedJob)
        );
        assert_eq!(f.handoff.outstanding(), Some(job.identity()));
    }
    f.assert_usable_initial();
    assert!(f.handoff.take_ready().is_none());
}

#[test]
fn expired_or_equal_deadline_completion_settles_but_never_mutates_or_rearms() {
    for observed in [900, 901] {
        let mut f = Fixture::new();
        let job = f.schedule(600, 900);
        let deadline = f.runtime.frame_timing.deadline();
        assert_eq!(f.complete(&job, observed), CompletionStatus::Obsolete);
        let transport = CheckedTransport::default();
        assert!(!f
            .owner_turn(&transport, observed, 900, observed, observed, false)
            .unwrap());
        assert!(transport.trace.borrow().is_empty());
        assert!(f.handoff.outstanding().is_none());
        assert_eq!(f.runtime.frame_timing.deadline(), deadline);
        f.assert_usable_initial();
    }
}

#[test]
fn ready_candidate_expiring_before_consumption_is_discarded_without_publication() {
    let mut f = Fixture::new();
    let job = f.schedule(600, 900);
    assert_eq!(f.complete(&job, 700), CompletionStatus::Ready);
    let deadline = f.runtime.frame_timing.deadline();
    let transport = CheckedTransport::default();
    assert!(!f.owner_turn(&transport, 900, 900, 900, 900, false).unwrap());
    assert!(f.handoff.take_ready().is_none());
    assert!(transport.trace.borrow().is_empty());
    assert_eq!(f.runtime.frame_timing.deadline(), deadline);
    f.assert_usable_initial();
}

#[test]
fn expiry_during_host_validation_is_checked_before_submission_including_equality() {
    for admission in [900, 901] {
        let mut f = Fixture::new();
        let job = f.schedule(600, 900);
        assert_eq!(f.complete(&job, 700), CompletionStatus::Ready);
        let transport = CheckedTransport::default();
        let deadline = f.runtime.frame_timing.deadline();
        assert!(!f
            .owner_turn(&transport, 700, 900, admission, admission, false)
            .unwrap());
        assert!(transport.trace.borrow().is_empty());
        f.assert_usable_initial();
        assert_eq!(f.runtime.frame_timing.deadline(), deadline);
        assert!(f.handoff.take_ready().is_none());
        // Skip neither reserves the sequence nor pauses eligible animation time.
        let next = f.schedule(1000, 1300);
        assert_eq!(next.request().identity(), job.request().identity());
        assert_eq!(next.request().active_elapsed(), Duration::from_millis(1000));
        assert_ne!(next.identity().serial, job.identity().serial);
    }
}

#[test]
fn current_control_or_body_interruption_prevents_ready_consumption_and_native_mutation() {
    for interruption in 0..4 {
        let mut f = Fixture::new();
        let job = f.schedule(600, 900);
        assert_eq!(f.complete(&job, 700), CompletionStatus::Ready);
        match interruption {
            1 => {
                f.runtime.body = BodyAvailability::AwaitingMap;
            }
            2 => {
                assert_eq!(
                    f.runtime.interaction.handle_right_press_on_body(20, true),
                    HostAction::TrackOpening
                );
                f.runtime.pointer.track_automatic_right().unwrap();
            }
            3 => {
                f.runtime.menu = Some(crate::MenuPopup::test_popup(200, 0));
            }
            _ => {}
        }
        let transport = CheckedTransport::default();
        let deadline = f.runtime.frame_timing.deadline();
        assert!(!f
            .owner_turn(&transport, 700, 900, 700, 700, interruption == 0)
            .unwrap());
        assert!(transport.trace.borrow().is_empty());
        assert!(f.handoff.take_ready().is_none());
        assert_eq!(f.runtime.frame_timing.deadline(), deadline);
        assert_eq!(
            f.runtime.frames.require_committed().unwrap().identity,
            INITIAL
        );
    }
}

#[test]
fn fault_observed_before_consumption_discards_ready_but_preserves_settlement_and_commit() {
    let mut f = Fixture::new();
    let job = f.schedule(600, 900);
    assert_eq!(f.complete(&job, 700), CompletionStatus::Ready);
    f.handoff.device_failed();
    let transport = CheckedTransport::default();
    let deadline = f.runtime.frame_timing.deadline();
    assert!(!f.owner_turn(&transport, 700, 900, 700, 700, false).unwrap());
    assert!(transport.trace.borrow().is_empty());
    assert!(f.handoff.outstanding().is_none());
    assert_eq!(f.handoff.resume(), Err(CloseReason::DeviceFailure));
    assert_eq!(f.runtime.frame_timing.deadline(), deadline);
    f.assert_usable_initial();
}

#[test]
fn fault_observed_after_commit_closes_future_admission_without_falsifying_history() {
    let mut f = Fixture::new();
    let job = f.schedule(600, 900);
    assert_eq!(f.complete(&job, 700), CompletionStatus::Ready);
    let transport = CheckedTransport::default();
    assert!(f.owner_turn(&transport, 700, 900, 700, 702, false).unwrap());
    let deadline = f.runtime.frame_timing.deadline();
    f.handoff.device_failed();
    assert_eq!(
        f.runtime.frames.require_committed().unwrap().identity,
        job.request().identity()
    );
    assert_eq!(
        f.runtime.frames.last_committed(),
        Some(job.request().identity())
    );
    assert_eq!(f.runtime.frame_timing.deadline(), deadline);
    assert!(f.runtime.body_input_ready());
    assert!(f.handoff.outstanding().is_none());
    assert_eq!(
        f.handoff.schedule(job.request(), f.at(703), f.at(900)),
        Err(ScheduleRejection::Closed(CloseReason::DeviceFailure))
    );
}

#[test]
fn unresolved_cancelled_work_retains_debt_until_genuine_obsolete_settlement() {
    let mut f = Fixture::new();
    let job = f.schedule(600, 900);
    f.handoff.interrupt().unwrap();
    assert_eq!(
        f.handoff
            .complete(
                job.identity(),
                JobCompletion::Unresolved("unknown".into()),
                f.at(700)
            )
            .unwrap(),
        CompletionStatus::Unresolved("unknown".into())
    );
    assert_eq!(f.handoff.outstanding(), Some(job.identity()));
    f.handoff.resume().unwrap();
    assert_eq!(
        f.handoff.schedule(job.request(), f.at(701), f.at(900)),
        Err(ScheduleRejection::Outstanding)
    );
    assert_eq!(
        f.handoff
            .complete(
                job.identity(),
                JobCompletion::SettledWithoutPayload,
                f.at(702)
            )
            .unwrap(),
        CompletionStatus::Obsolete
    );
    assert!(f.handoff.outstanding().is_none());
    assert!(f.handoff.take_ready().is_none());
    let next = f.schedule(703, 900);
    assert_ne!(next.identity().serial, job.identity().serial);
    f.assert_usable_initial();
}

#[test]
fn rejected_or_failed_fake_preparation_never_reports_success_or_mutates_the_host() {
    for completion in [
        JobCompletion::RejectedBeforeSubmission,
        JobCompletion::Failed("preparation".into()),
    ] {
        let mut f = Fixture::new();
        let job = f.schedule(600, 900);
        let deadline = f.runtime.frame_timing.deadline();
        let status = f
            .handoff
            .complete(job.identity(), completion, f.at(700))
            .unwrap();
        assert!(matches!(
            status,
            CompletionStatus::RejectedBeforeSubmission | CompletionStatus::Failed(_)
        ));
        let transport = CheckedTransport::default();
        assert!(!f.owner_turn(&transport, 700, 900, 700, 700, false).unwrap());
        assert!(transport.trace.borrow().is_empty());
        assert_eq!(f.runtime.frame_timing.deadline(), deadline);
        f.assert_usable_initial();
    }
}

#[test]
fn malformed_ready_payload_coverage_extent_and_identity_never_reach_native_submission() {
    for corruption in 0..5 {
        let mut f = Fixture::new();
        let job = f.schedule(600, 900);
        let mut frame = Fixture::frame(&job);
        match corruption {
            0 => {
                frame.payload.pop();
            }
            1 => {
                frame.payload[0..4].copy_from_slice(&[255, 0, 0, 0]);
            }
            2 => {
                let request = FrameRequest::new(
                    job.request().identity(),
                    job.request().intent(),
                    job.request().extent(),
                    Duration::from_millis(300),
                    Duration::ZERO,
                )
                .unwrap();
                frame.metadata = SyntheticProducer::new()
                    .prepare_frame(&request)
                    .unwrap()
                    .metadata;
            }
            3 => {
                let extent = ClientExtent::new(Size::new(1, 1)).unwrap();
                frame.metadata =
                    FrameMetadata::new(extent, None, None, HitRegion::new(extent, []).unwrap())
                        .unwrap();
            }
            _ => {
                frame.identity.sequence += 1;
            }
        }
        let status = f
            .handoff
            .complete(job.identity(), JobCompletion::Frame(frame), f.at(700));
        if corruption == 4 {
            assert_eq!(status, Err(CompletionError::FrameIdentityMismatch));
        } else {
            assert_eq!(status.unwrap(), CompletionStatus::Ready);
            let transport = CheckedTransport::default();
            assert!(f.owner_turn(&transport, 700, 900, 700, 700, false).is_err());
            assert!(transport.trace.borrow().is_empty());
            f.assert_usable_initial();
        }
    }
}

#[test]
fn invalidated_host_generation_rejects_ready_before_native_mutation() {
    let mut f = Fixture::new();
    let job = f.schedule(600, 900);
    assert_eq!(f.complete(&job, 700), CompletionStatus::Ready);
    f.runtime.frames.invalidate();
    let transport = CheckedTransport::default();
    assert!(!f.owner_turn(&transport, 700, 900, 700, 700, false).unwrap());
    assert!(f.handoff.take_ready().is_none());
    assert!(transport.trace.borrow().is_empty());
    assert_eq!(f.runtime.frames.last_committed(), Some(INITIAL));
    assert!(f.runtime.frames.committed().is_none());
}

#[test]
fn fully_transparent_fake_candidate_commits_exact_empty_input_after_checked_cutover() {
    let mut f = Fixture::new();
    let job = f.schedule(1200, 1500);
    assert_eq!(f.complete(&job, 1300), CompletionStatus::Ready);
    let transport = CheckedTransport::default();
    assert!(f
        .owner_turn(&transport, 1300, 1500, 1300, 1302, false)
        .unwrap());
    assert_eq!(*transport.trace.borrow(), CUTOVER);
    assert!(transport.image.borrow().iter().all(|byte| *byte == 0));
    assert!(transport
        .final_rectangles
        .borrow()
        .as_ref()
        .unwrap()
        .is_empty());
    assert!(f
        .runtime
        .frames
        .require_committed()
        .unwrap()
        .metadata
        .hit_region()
        .spans()
        .is_empty());
    assert!(f
        .runtime
        .frames
        .require_committed()
        .unwrap()
        .metadata
        .visible_bounds()
        .is_none());
    assert_eq!(f.runtime.input_fence.enabled_after, Some(BOUNDARY));
}

#[test]
fn async_native_failure_fences_without_retry_and_keeps_gpu_settlement_independent() {
    for failure in [
        Step::Upload,
        Step::EmptyShape,
        Step::Release,
        Step::Blit,
        Step::Flush,
        Step::FinalShape,
    ] {
        let mut f = Fixture::new();
        let job = f.schedule(600, 900);
        assert_eq!(f.complete(&job, 700), CompletionStatus::Ready);
        let transport = CheckedTransport {
            failure: Some(failure),
            ..Default::default()
        };
        assert_eq!(
            f.owner_turn(&transport, 700, 900, 700, 702, false)
                .unwrap_err()
                .to_string(),
            format!("{failure:?}")
        );
        let through = CUTOVER.iter().position(|step| *step == failure).unwrap();
        let mut expected = CUTOVER[..=through].to_vec();
        expected.push(Step::SafetyEmpty);
        expected.push(Step::SafetyRelease);
        assert_eq!(*transport.trace.borrow(), expected);
        assert!(f.runtime.frames.committed().is_none());
        assert_eq!(f.runtime.frames.last_committed(), Some(INITIAL));
        assert!(f.runtime.input_fence.blocked);
        assert!(f.runtime.frame_timing.deadline().is_none());
        assert!(f.handoff.outstanding().is_none());
        assert!(!f.owner_turn(&transport, 703, 900, 703, 703, false).unwrap());
        assert_eq!(*transport.trace.borrow(), expected);
    }
}

#[test]
fn async_post_commit_timing_error_revokes_usable_authority_and_fences_input() {
    let mut f = Fixture::new();
    let job = f.schedule(600, 900);
    assert_eq!(f.complete(&job, 700), CompletionStatus::Ready);
    let transport = CheckedTransport::default();
    assert!(f
        .owner_turn(&transport, 700, 900, 700, 599, false)
        .unwrap_err()
        .to_string()
        .contains("backwards"));
    let mut expected = CUTOVER.to_vec();
    expected.push(Step::SafetyEmpty);
    expected.push(Step::SafetyRelease);
    assert_eq!(*transport.trace.borrow(), expected);
    assert_eq!(
        f.runtime.frames.last_committed(),
        Some(job.request().identity())
    );
    assert!(f.runtime.frames.committed().is_none());
    assert!(f.runtime.input_fence.blocked);
    assert!(f.runtime.frame_timing.deadline().is_none());
    assert!(f.handoff.outstanding().is_none());
}

//! Opt-in managed adapter. D1 owns desktop work, and the supervisor owns IPC
//! and the child. The owner drives both from the same monotonic observation.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use roomie_worker_protocol::{self as wire, JobKey, Render};
use roomie_worker_supervisor::machine::{JobState, Outcome, Phase, Policy, Report};
use roomie_worker_supervisor::{Progress, Readiness, Supervisor, WaitInterest};

use crate::managed_vulkan_evidence::{Capture, Kind};
use crate::presentation::gpu_handoff::{
    CompletionStatus, GpuHandoff, JobCompletion, JobIdentity, ReadyFrame, ScheduleRejection,
};
use crate::presentation::{
    Action, Facing, FrameMetadata, FrameRequest, HitRegion, Point, PreparedFrame, Rect, RowSpan,
    Size,
};
use crate::HostError;

const EXIT_OBSERVATION: Duration = Duration::from_millis(50);
const QUIESCENT_WAIT_LIMIT: Duration = Duration::from_secs(1);

/// Explicit child configuration. The fixture entry uses an independently built
/// fake-worker; the experimental native entry validates its own configuration.
pub struct Fixture<'a> {
    pub executable: &'a Path,
    pub args: &'a [&'a str],
    pub session: u64,
    pub policy: Policy,
}

/// Neutral name for the same accepted launch contract; fixture API is retained.
pub type Launch<'a> = Fixture<'a>;

pub struct ManagedSupervisor {
    supervisor: Supervisor,
    handoff: GpuHandoff<Vec<u8>>,
    origin: Instant,
    policy: Policy,
    progress: Progress,
    observe_exit_at: Option<Instant>,
    shutting_down: bool,
    pub(crate) evidence: Option<Capture>,
}

impl ManagedSupervisor {
    pub fn spawn(fixture: Fixture<'_>, origin: Instant) -> Result<Self, HostError> {
        let path = fixture.executable;
        if !path.is_absolute() {
            return Err("Managed fixture requires an explicit absolute executable path".into());
        }
        let meta = path
            .metadata()
            .map_err(|error| format!("Managed fixture executable {}: {error}", path.display()))?;
        if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
            return Err(format!("Managed fixture is not executable: {}", path.display()).into());
        }
        let policy = fixture.policy;
        for duration in [
            policy.initialization,
            policy.freshness,
            policy.escalation,
            policy.graceful_shutdown,
            policy.exit_observation,
        ] {
            origin
                .checked_add(duration)
                .ok_or("Fixture deadline exceeds the monotonic clock")?;
        }
        let supervisor =
            Supervisor::spawn(path, fixture.args, fixture.session, policy, Duration::ZERO)?;
        Ok(Self {
            supervisor,
            handoff: GpuHandoff::new(fixture.session),
            origin,
            policy,
            progress: Progress {
                continue_immediately: false,
                drained: false,
            },
            observe_exit_at: None,
            shutting_down: false,
            evidence: None,
        })
    }

    fn relative(&self, now: Instant) -> Result<Duration, HostError> {
        now.checked_duration_since(self.origin)
            .ok_or_else(|| "Managed supervisor observation precedes its origin".into())
    }

    pub fn outcome(&self) -> Outcome {
        self.supervisor.outcome()
    }

    pub fn outstanding(&self) -> Option<JobIdentity> {
        self.handoff.outstanding()
    }

    pub fn unresolved(&self) -> Option<JobIdentity> {
        self.handoff.unresolved()
    }

    pub fn ready(&self) -> Option<&ReadyFrame<Vec<u8>>> {
        self.handoff.ready()
    }

    pub(crate) fn select_evidence(&mut self) {
        if let (Some(capture), Some(ready)) = (&mut self.evidence, self.handoff.ready()) {
            capture.select(ready, ready.deadline.checked_duration_since(self.origin));
        }
    }

    pub fn take_ready(&mut self) -> Option<ReadyFrame<Vec<u8>>> {
        self.handoff.take_ready_frame()
    }

    pub fn immediate(&self) -> bool {
        self.progress.continue_immediately
    }

    pub fn drained(&self) -> bool {
        self.progress.drained && !self.immediate()
    }

    pub fn healthy(&self) -> bool {
        !self.shutting_down && self.outcome().failure.is_none()
    }

    pub(crate) fn can_request(&self) -> bool {
        self.healthy()
            && self.drained()
            && self.outcome().phase == Phase::Ready
            && self.outstanding().is_none()
            && self.ready().is_none()
    }

    pub fn set_eligible(&mut self, eligible: bool) -> Result<(), HostError> {
        self.set_eligible_at(eligible, None)
    }

    pub(crate) fn set_eligible_at(
        &mut self,
        eligible: bool,
        now: Option<Instant>,
    ) -> Result<(), HostError> {
        if self.healthy() {
            if eligible {
                self.handoff
                    .resume()
                    .map_err(|error| format!("D1 resume: {error:?}"))?;
            } else {
                self.record_cancellation(now);
                self.handoff
                    .interrupt()
                    .map_err(|error| format!("D1 interruption: {error:?}"))?;
                self.supervisor.cancel();
            }
        }
        Ok(())
    }

    fn record_cancellation(&mut self, now: Option<Instant>) {
        if self.evidence.is_none() {
            return;
        }
        if let Some(identity) = self.ready().map(|ready| ready.identity) {
            self.record(Kind::CanceledReady, Some(identity), now);
        }
        if let Some(identity) = self.outstanding() {
            let at = now.and_then(|now| self.relative(now).ok());
            if let Some(capture) = &mut self.evidence {
                capture.cancel_pending(identity, at);
            }
        }
    }

    pub(crate) fn record(&mut self, kind: Kind, job: Option<JobIdentity>, now: Option<Instant>) {
        let origin = self.origin;
        if let Some(capture) = &mut self.evidence {
            let at = now.and_then(|now| now.checked_duration_since(origin));
            if now.is_some() && at.is_none() {
                capture.fail("owner observation precedes monotonic origin");
            }
            capture.event(kind, job, at);
        }
    }

    pub(crate) fn evidence_time(&self, now: Instant) -> Option<Duration> {
        self.relative(now).ok()
    }

    pub(crate) fn exit_code(&self) -> Option<i32> {
        self.supervisor.exit_status()?.code()
    }

    pub(crate) fn diagnostic(&self) -> Option<&str> {
        self.supervisor.machine().diagnostic()
    }

    pub fn admit(
        &mut self,
        request: FrameRequest,
        now: Instant,
    ) -> Result<Option<JobKey>, HostError> {
        if !self.can_request() {
            return Ok(None);
        }
        if !supported(&request) {
            return Err("Unsupported managed fixture request extent or intent".into());
        }
        let observed = self.relative(now)?;
        let fresh = observed
            .checked_add(self.policy.freshness)
            .ok_or("Fixture freshness overflow")?;
        let deadline = self
            .origin
            .checked_add(fresh)
            .ok_or("Fixture freshness exceeds the monotonic clock")?;
        now.checked_add(self.policy.escalation)
            .ok_or("Fixture escalation exceeds the monotonic clock")?;
        let job = match self.handoff.schedule(request, now, deadline) {
            Ok(job) => job,
            Err(ScheduleRejection::Closed(reason)) => {
                return Err(format!("D1 admission closed: {reason:?}").into());
            }
            Err(_) => return Ok(None),
        };
        let identity = job.identity();
        let key = job_key(identity);
        let render = Render {
            job: key,
            action: wire::Action::Idle,
            facing: wire::Facing::Neutral,
            elapsed: request.active_elapsed(),
            delta: request.active_delta(),
        };
        if let Some(capture) = &mut self.evidence {
            capture.admitted(identity, Some(observed), request, Some(fresh));
        }
        if self.supervisor.submit(render, observed).is_err() {
            // Every error from the accepted submit interface occurs before
            // the Render acquires a transport slot.
            self.record(Kind::Withdrawn, Some(identity), Some(now));
            self.complete(identity, JobCompletion::RejectedBeforeSubmission, now)?;
            return Ok(None);
        }
        self.progress.drained = false;
        Ok(Some(key))
    }

    pub fn interests(&self) -> Result<(WaitInterest, Option<Instant>), HostError> {
        let interest = self.supervisor.interest();
        let deadline = interest
            .deadline
            .map(|relative| {
                self.origin
                    .checked_add(relative)
                    .ok_or("Supervisor deadline exceeds the monotonic clock")
            })
            .transpose()?;
        Ok((
            interest,
            deadline
                .into_iter()
                .chain(self.handoff.wait_deadline())
                .chain(self.observe_exit_at)
                .min(),
        ))
    }

    /// Wait on the accepted current descriptors, without extending an active
    /// supervisor deadline. The desktop adds X11 to the same poll helper.
    pub fn wait_until(&self, deadline: Instant) -> Result<Readiness, HostError> {
        let (interest, active) = self.interests()?;
        let deadline = if self.immediate() {
            Instant::now()
        } else {
            active.map_or(deadline, |active| active.min(deadline))
        };
        crate::x11::events::wait_managed(None, &interest, Some(deadline))
    }

    pub(crate) fn observe(&mut self, now: impl FnOnce() -> Instant) -> Result<(), HostError> {
        let interest = self.supervisor.interest();
        let readiness = crate::x11::events::poll_managed(None, &interest, Some(Instant::now()))?;
        self.progress(readiness, now())
    }

    pub fn progress(&mut self, ready: Readiness, now: Instant) -> Result<(), HostError> {
        self.progress = self.supervisor.progress(ready, self.relative(now)?);
        let outcome = self.outcome();
        if outcome.failure.is_some() {
            if !self.shutting_down {
                eprintln!(
                    "[MANAGED] Supervisor failure {:?}: {}",
                    outcome.failure,
                    self.supervisor
                        .machine()
                        .diagnostic()
                        .unwrap_or("no worker diagnostic"),
                );
            }
            self.record_cancellation(Some(now));
            self.handoff.device_failed();
            self.begin_shutdown(now)?;
        }
        if let Some(identity) = self.handoff.expire(now) {
            self.record(Kind::Expired, Some(identity), Some(now));
        }
        if let Some(report) = self.supervisor.take_report() {
            self.accept_report(outcome.session, report, now)?;
        }
        if let Some(identity) = self.outstanding() {
            if self.unresolved().is_none()
                && outcome.job.is_some_and(|status| {
                    status.job == job_key(identity)
                        && outcome.session == identity.session
                        && (status.state == JobState::Unresolved
                            || (outcome.reaped && outcome.unresolved == Some(status.job)))
                })
            {
                self.record(Kind::Unresolved, Some(identity), Some(now));
                self.complete(
                    identity,
                    JobCompletion::Unresolved(format!(
                        "Supervisor lacks settlement evidence: {:?}",
                        outcome.failure
                    )),
                    now,
                )?;
            }
        }
        let interest = self.supervisor.interest();
        self.observe_exit_at = if interest.process.is_none() && !outcome.reaped {
            Some(
                now.checked_add(EXIT_OBSERVATION)
                    .ok_or("Process observation deadline overflow")?,
            )
        } else {
            None
        };
        Ok(())
    }

    fn accept_report(
        &mut self,
        session: u64,
        report: Report,
        now: Instant,
    ) -> Result<(), HostError> {
        let key = match &report {
            Report::Frame { job, .. }
            | Report::RejectedBeforeDispatch { job }
            | Report::Rejected { job, .. }
            | Report::Failed { job, .. }
            | Report::Obsolete(job) => *job,
        };
        let Some(identity) = self
            .outstanding()
            .filter(|identity| identity.session == session && job_key(*identity) == key)
        else {
            return Err("Supervisor report does not match D1 physical work".into());
        };
        let kind = match &report {
            Report::Frame { .. } => Kind::Frame,
            Report::Obsolete(_) => Kind::Obsolete,
            Report::RejectedBeforeDispatch { .. } => Kind::Withdrawn,
            Report::Rejected { .. } => Kind::Rejected,
            Report::Failed { .. } => Kind::Failed,
        };
        self.record(kind, Some(identity), Some(now));
        let completion = match report {
            Report::Frame { bgra, .. } => {
                let request = self
                    .handoff
                    .outstanding_request()
                    .expect("correlated D1 request");
                match prepare_returned(&request, bgra) {
                    Some(frame) => JobCompletion::Frame(frame),
                    None => {
                        self.record(Kind::InvalidFrame, Some(identity), Some(now));
                        JobCompletion::SettledWithoutPayload
                    }
                }
            }
            Report::Obsolete(_) => JobCompletion::SettledWithoutPayload,
            Report::RejectedBeforeDispatch { .. } | Report::Rejected { .. } => {
                JobCompletion::RejectedBeforeSubmission
            }
            Report::Failed { diagnostic, .. } => JobCompletion::Failed(diagnostic),
        };
        let status = self.complete(identity, completion, now)?;
        match status {
            CompletionStatus::Ready => self.record(Kind::Ready, Some(identity), Some(now)),
            CompletionStatus::Obsolete if kind == Kind::Frame => {
                self.record(Kind::D1Obsolete, Some(identity), Some(now));
            }
            _ => {}
        }
        Ok(())
    }

    fn complete(
        &mut self,
        job: JobIdentity,
        completion: JobCompletion<Vec<u8>>,
        now: Instant,
    ) -> Result<CompletionStatus, HostError> {
        self.handoff
            .complete(job, completion, now)
            .map_err(|error| format!("D1 settlement mismatch: {error:?}").into())
    }

    pub fn begin_shutdown(&mut self, now: Instant) -> Result<(), HostError> {
        if !self.shutting_down {
            self.record_cancellation(Some(now));
            self.record(Kind::Shutdown, None, Some(now));
            self.handoff.shutdown();
            self.supervisor.shutdown(self.relative(now)?);
            self.shutting_down = true;
        }
        Ok(())
    }

    pub(crate) fn reap_after_host_cleanup(&mut self) -> Result<(), HostError> {
        self.reap_after_host_cleanup_with(Instant::now)
    }

    pub(crate) fn reap_after_host_cleanup_with(
        &mut self,
        mut now: impl FnMut() -> Instant,
    ) -> Result<(), HostError> {
        let mut first_error = self.begin_shutdown(now()).err();
        while !self.outcome().reaped {
            let wait = self.interests().and_then(|(interest, deadline)| {
                let real_now = Instant::now();
                let limit = real_now
                    .checked_add(QUIESCENT_WAIT_LIMIT)
                    .ok_or("Quiescent observation deadline overflow")?;
                let deadline = if self.immediate() {
                    real_now
                } else {
                    deadline.map_or(limit, |deadline| deadline.min(limit))
                };
                crate::x11::events::wait_managed(None, &interest, Some(deadline))
            });
            let readiness = match wait {
                Ok(readiness) => readiness,
                Err(error) => {
                    first_error.get_or_insert(error);
                    std::thread::sleep(EXIT_OBSERVATION);
                    Readiness::default()
                }
            };
            if let Err(error) = self.progress(readiness, now()) {
                first_error.get_or_insert(error);
            }
        }
        let outcome = self.outcome();
        eprintln!(
            "[MANAGED] shutdown={:?}, stopped={}, reaped={}, termination_requested={}, unresolved={:?}, failure={:?}",
            outcome.shutdown, outcome.stopped, outcome.reaped, outcome.termination_requested,
            self.unresolved(), outcome.failure,
        );
        first_error.map_or(Ok(()), Err)
    }
}

fn job_key(identity: JobIdentity) -> JobKey {
    JobKey {
        epoch: identity.epoch,
        serial: identity.serial,
        generation: identity.frame.generation,
        sequence: identity.frame.sequence,
    }
}

fn supported(request: &FrameRequest) -> bool {
    request.extent().size() == Size::new(wire::WIDTH.into(), wire::HEIGHT.into())
        && request.intent().action == Action::Idle
        && request.intent().facing == Facing::Neutral
}

fn prepare_returned(request: &FrameRequest, pixels: Vec<u8>) -> Option<PreparedFrame<Vec<u8>>> {
    if !supported(request) || pixels.len() != wire::FRAME_BYTES {
        return None;
    }
    let extent = request.extent();
    let mut spans = Vec::new();
    let mut left = i32::from(wire::WIDTH);
    let mut right = 0;
    for (y, row) in pixels
        .chunks_exact(4 * usize::from(wire::WIDTH))
        .enumerate()
    {
        let mut start = None;
        for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
            let alpha = pixel[3];
            if pixel[..3].iter().any(|&channel| channel > alpha) {
                return None;
            }
            if alpha != 0 {
                start.get_or_insert(x as i32);
            } else if let Some(start) = start.take() {
                spans.push(RowSpan {
                    y: y as i32,
                    start_x: start,
                    end_x: x as i32,
                });
                left = left.min(start);
                right = right.max(x as i32);
            }
        }
        if let Some(start) = start {
            spans.push(RowSpan {
                y: y as i32,
                start_x: start,
                end_x: wire::WIDTH.into(),
            });
            left = left.min(start);
            right = wire::WIDTH.into();
        }
    }
    let anchor = spans.first().map(|span| Point::new(span.start_x, span.y));
    let bounds = spans.first().zip(spans.last()).map(|(first, last)| {
        Rect::new(
            left,
            first.y,
            (right - left) as u32,
            (last.y - first.y + 1) as u32,
        )
    });
    let region = HitRegion::new(extent, spans).ok()?;
    let metadata = FrameMetadata::new(extent, bounds, anchor, region).ok()?;
    Some(PreparedFrame {
        identity: request.identity(),
        metadata,
        payload: pixels,
    })
}

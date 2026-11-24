use crate::oracle;
use desktop_probe::presentation::{self as p, gpu_handoff::*};
use roomie_worker_protocol::{self as wire, JobKey};
use roomie_worker_supervisor::{
    machine::{Outcome, Phase, Policy, Report},
    Readiness, Supervisor,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Debug, Default, Clone, Copy)]
pub struct Counters {
    pub attempts: u64,
    pub admitted: u64,
    pub dispatched: u64,
    pub busy: u64,
    pub valid: u64,
    pub obsolete: u64,
    pub rejected: u64,
    pub failed: u64,
    pub cancellations: u64,
}
#[derive(Debug)]
pub enum Event {
    Frame {
        request: p::FrameRequest,
        job: JobIdentity,
        ipc: Duration,
        end_to_end: Duration,
    },
    Obsolete(JobIdentity),
    /// Supervisor proves no render byte was dispatched.
    Withdrawn(JobIdentity),
    Rejected(JobIdentity),
    Failed(JobIdentity),
}
struct Pending {
    identity: JobIdentity,
    request: p::FrameRequest,
    admitted: Duration,
    dispatched: Option<Duration>,
}
pub struct Adapter {
    supervisor: Supervisor,
    handoff: GpuHandoff<Vec<u8>>,
    origin: Instant,
    policy: Policy,
    pending: Option<Pending>,
    unresolved_reported: bool,
    counters: Counters,
    initialization_duration: Option<Duration>,
    shutdown_duration: Option<Duration>,
    shutdown_started: Option<Duration>,
    observed_at: Duration,
}

pub fn key(id: JobIdentity) -> JobKey {
    JobKey {
        epoch: id.epoch,
        serial: id.serial,
        generation: id.frame.generation,
        sequence: id.frame.sequence,
    }
}
fn render(id: JobIdentity, request: p::FrameRequest) -> Result<wire::Render, String> {
    if request.extent().size() != p::Size::new(160, 160) {
        return Err("only 160x160 supported".into());
    }
    Ok(wire::Render {
        job: key(id),
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
impl Adapter {
    pub fn spawn(
        executable: &Path,
        args: &[&str],
        session: u64,
        policy: Policy,
    ) -> Result<Self, String> {
        let origin = Instant::now();
        let supervisor = Supervisor::spawn(executable, args, session, policy, Duration::ZERO)
            .map_err(|e| e.to_string())?;
        let mut handoff = GpuHandoff::new(session);
        handoff.resume().map_err(|e| format!("{e:?}"))?;
        Ok(Self {
            supervisor,
            handoff,
            origin,
            policy,
            pending: None,
            unresolved_reported: false,
            counters: Counters::default(),
            initialization_duration: None,
            shutdown_duration: None,
            shutdown_started: None,
            observed_at: Duration::ZERO,
        })
    }
    pub fn now(&self) -> Duration {
        self.origin.elapsed()
    }
    pub fn outcome(&self) -> Outcome {
        self.supervisor.outcome()
    }
    pub fn observed_at(&self) -> Duration {
        self.observed_at
    }
    pub fn diagnostic(&self) -> Option<&str> {
        self.supervisor.machine().diagnostic()
    }
    pub fn initialization_duration(&self) -> Option<Duration> {
        self.initialization_duration
    }
    pub fn shutdown_duration(&self) -> Option<Duration> {
        self.shutdown_duration
    }
    pub fn counters(&self) -> Counters {
        self.counters
    }
    pub fn outstanding(&self) -> Option<JobIdentity> {
        self.handoff.outstanding()
    }
    pub fn exit_code(&self) -> Option<i32> {
        self.supervisor.exit_status().and_then(|s| s.code())
    }
    pub fn submit(
        &mut self,
        request: p::FrameRequest,
        now: Duration,
    ) -> Result<JobIdentity, String> {
        self.counters.attempts += 1;
        let job = match self.handoff.schedule(
            request,
            self.origin + now,
            self.origin + now + self.policy.freshness,
        ) {
            Ok(job) => job,
            Err(e) => {
                self.counters.busy += 1;
                return Err(format!("D1 admission: {e:?}"));
            }
        };
        let id = job.identity();
        let rendered = render(id, job.request());
        let admitted = match rendered {
            Ok(render) => self
                .supervisor
                .submit(render, now)
                .map_err(|e| format!("supervisor admission: {e:?}")),
            Err(e) => Err(e),
        };
        if let Err(e) = admitted {
            // submit Err cannot acquire the supervisor slot or write any byte.
            self.handoff
                .complete(
                    id,
                    JobCompletion::RejectedBeforeSubmission,
                    self.origin + now,
                )
                .map_err(|e| format!("{e:?}"))?;
            self.counters.rejected += 1;
            return Err(e);
        }
        self.counters.admitted += 1;
        self.pending = Some(Pending {
            identity: id,
            request,
            admitted: now,
            dispatched: None,
        });
        Ok(id)
    }
    pub fn cancel(&mut self) {
        self.counters.cancellations += 1;
        let _ = self.handoff.interrupt();
        self.supervisor.cancel();
    }
    pub fn resume(&mut self) -> Result<(), String> {
        self.handoff.resume().map_err(|e| format!("{e:?}"))
    }
    pub fn shutdown(&mut self, now: Duration) {
        self.shutdown_started.get_or_insert(now);
        self.handoff.shutdown();
        self.supervisor.shutdown(now);
    }
    /// One owner time is used by BOTH authorities. Fault observation precedes
    /// report consumption; no telemetry participates in this method.
    pub fn progress(&mut self, ready: Readiness, now: Duration) -> Result<Option<Event>, String> {
        self.observed_at = now;
        self.handoff.expire(self.origin + now);
        self.supervisor.progress(ready, now);
        let outcome = self.outcome();
        if outcome.phase != Phase::Starting && self.initialization_duration.is_none() {
            self.initialization_duration = Some(now);
        }
        if let Some(p) = &mut self.pending {
            if p.dispatched.is_none()
                && outcome.job.is_some_and(|j| {
                    j.job == key(p.identity)
                        && j.transmission
                            == roomie_worker_supervisor::machine::Transmission::Complete
                })
            {
                p.dispatched = Some(now);
                self.counters.dispatched += 1;
            }
        }
        if outcome.failure.is_some() {
            self.handoff.device_failed();
        }
        if let Some(report) = self.supervisor.take_report() {
            let reported = match &report {
                Report::Frame { job, .. }
                | Report::RejectedBeforeDispatch { job }
                | Report::Rejected { job, .. }
                | Report::Failed { job, .. } => *job,
                Report::Obsolete(job) => *job,
            };
            let p = self.pending.as_ref().ok_or("report without D1 job")?;
            if reported != key(p.identity) || outcome.session != p.identity.session {
                self.handoff.device_failed();
                return Err("exact dispatched identity mismatch".into());
            }
            let id = p.identity;
            let mut validation = Ok(());
            let local_withdrawal = matches!(report, Report::RejectedBeforeDispatch { .. });
            let completion = match report {
                Report::Frame { bgra, .. } => {
                    let prepared = oracle::metadata(&bgra).map(|metadata| p::PreparedFrame {
                        identity: id.frame,
                        metadata,
                        payload: bgra,
                    });
                    match prepared {
                        Ok(frame) => {
                            validation = oracle::validate(&p.request, &frame);
                            if validation.is_ok() {
                                JobCompletion::Frame(frame)
                            } else {
                                JobCompletion::Failed("independent CPU oracle mismatch".into())
                            }
                        }
                        Err(e) => {
                            validation = Err(e);
                            JobCompletion::Failed("invalid pixel layout".into())
                        }
                    }
                }
                Report::Obsolete(_) => JobCompletion::SettledWithoutPayload,
                Report::RejectedBeforeDispatch { .. } | Report::Rejected { .. } => {
                    JobCompletion::RejectedBeforeSubmission
                }
                Report::Failed { diagnostic, .. } => JobCompletion::Failed(diagnostic),
            };
            let status = self
                .handoff
                .complete(id, completion, self.origin + now)
                .map_err(|e| format!("D1 completion: {e:?}"))?;
            let p = self.pending.take().unwrap();
            let event = match status {
                CompletionStatus::Ready => {
                    let (original, frame) =
                        self.handoff.take_ready().ok_or("missing ready frame")?;
                    if original != p.request {
                        return Err("authoritative request changed".into());
                    }
                    oracle::validate(&original, &frame)?;
                    self.counters.valid += 1;
                    Event::Frame {
                        request: original,
                        job: id,
                        ipc: now.saturating_sub(p.dispatched.ok_or("frame without dispatch")?),
                        end_to_end: now.saturating_sub(p.admitted),
                    }
                }
                CompletionStatus::Obsolete => {
                    self.counters.obsolete += 1;
                    Event::Obsolete(id)
                }
                CompletionStatus::RejectedBeforeSubmission => {
                    self.counters.rejected += 1;
                    if local_withdrawal {
                        Event::Withdrawn(id)
                    } else {
                        Event::Rejected(id)
                    }
                }
                CompletionStatus::Failed(_) => {
                    self.counters.failed += 1;
                    Event::Failed(id)
                }
                CompletionStatus::Unresolved(_) => unreachable!("not a terminal completion"),
            };
            validation?;
            return Ok(Some(event));
        }
        if let (Some(_), Some(p), false) = (
            outcome.failure,
            self.pending.as_ref(),
            self.unresolved_reported,
        ) {
            self.unresolved_reported = true;
            let id = p.identity;
            // The exact identity stays in D1 after transport abandonment AND
            // reaping. Neither observation is a physical completion report.
            self.handoff
                .complete(
                    id,
                    JobCompletion::Unresolved("supervisor failure; settlement unproven".into()),
                    self.origin + now,
                )
                .map_err(|e| format!("{e:?}"))?;
        }
        Ok(None)
    }
    pub fn poll(&mut self, wait: Duration) -> Result<Option<Event>, String> {
        self.poll_bounded(wait, None)
    }
    pub fn poll_window(
        &mut self,
        wait: Duration,
        cutoff: Duration,
    ) -> Result<Option<Event>, String> {
        self.poll_bounded(wait, Some(cutoff))
    }
    pub fn progress_window(
        &mut self,
        ready: Readiness,
        now: Duration,
        cutoff: Duration,
    ) -> Result<Option<Event>, String> {
        if now >= cutoff {
            self.cancel();
        }
        self.progress(ready, now)
    }
    fn poll_bounded(
        &mut self,
        wait: Duration,
        cutoff: Option<Duration>,
    ) -> Result<Option<Event>, String> {
        let ready = match self.supervisor.poll_ready(wait, self.now()) {
            Ok(ready) => ready,
            Err(error) => {
                // Give the accepted supervisor the transport failure; do not
                // retry an invalid poll set as if it were recovery.
                self.progress(
                    Readiness {
                        error: true,
                        ..Default::default()
                    },
                    self.now(),
                )?;
                return Err(error.to_string());
            }
        };
        let now = self.now();
        match cutoff {
            Some(cutoff) => self.progress_window(ready, now, cutoff),
            None => self.progress(ready, now),
        }
    }
    pub fn wait_ready(&mut self) -> Result<(), String> {
        while self.outcome().phase == Phase::Starting {
            self.poll(Duration::from_millis(100))?;
        }
        if self.outcome().phase == Phase::Ready && self.outcome().failure.is_none() {
            Ok(())
        } else {
            Err(format!("initialization failed: {:?}", self.outcome()))
        }
    }
    pub fn wait_event(&mut self) -> Result<Event, String> {
        loop {
            if let Some(event) = self.poll(Duration::from_millis(100))? {
                return Ok(event);
            }
            if self.outcome().failure.is_some() || self.outcome().reaped {
                return Err(format!("worker failed: {:?}", self.outcome()));
            }
        }
    }
    /// Accepted escalation/exit-observation bounds, no automatic replacement.
    pub fn finish(&mut self) -> Result<Outcome, String> {
        self.finish_events(|_| {})
    }
    pub fn finish_events(&mut self, mut observe: impl FnMut(Event)) -> Result<Outcome, String> {
        self.finish_observations(|event, _| observe(event))
    }
    pub fn finish_observations(
        &mut self,
        mut observe: impl FnMut(Event, Duration),
    ) -> Result<Outcome, String> {
        let started = self.now();
        self.shutdown(started);
        let stop = self.now()
            + self.policy.graceful_shutdown
            + self.policy.exit_observation
            + Duration::from_millis(200);
        let mut error = None;
        while !self.outcome().reaped && self.now() < stop {
            match self.poll(Duration::from_millis(50)) {
                Ok(Some(event)) => observe(event, self.observed_at),
                Ok(None) => {}
                Err(e) => {
                    error.get_or_insert(e);
                }
            }
        }
        self.shutdown_duration
            .get_or_insert(self.now().saturating_sub(self.shutdown_started.unwrap()));
        error.map_or(Ok(self.outcome()), Err)
    }
}

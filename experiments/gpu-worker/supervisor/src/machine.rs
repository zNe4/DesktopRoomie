use std::time::Duration;

use roomie_worker_protocol::{Body, JobKey, Message, Render};

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub initialization: Duration,
    pub freshness: Duration,
    pub escalation: Duration,
    pub graceful_shutdown: Duration,
    pub exit_observation: Duration,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            initialization: Duration::from_secs(5),
            freshness: Duration::from_millis(100),
            escalation: Duration::from_millis(2500),
            graceful_shutdown: Duration::from_millis(500),
            exit_observation: Duration::from_millis(500),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Ready,
    Outstanding,
    Closing,
    TerminationRequested,
    ShutdownUnconfirmed,
    Exited,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    NotReady,
    Outstanding,
    ResultPending,
    Identity,
    Backpressure,
    Clock,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Report {
    Frame {
        job: JobKey,
        bgra: Vec<u8>,
    },
    /// Local proof: the sole transport discarded this job before writing any byte.
    RejectedBeforeDispatch {
        job: JobKey,
    },
    Obsolete(JobKey),
    Rejected {
        job: JobKey,
        diagnostic: String,
    },
    Failed {
        job: JobKey,
        diagnostic: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transmission {
    Unsent,
    Partial,
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    AdmittedUnsent,
    PartiallyTransmitted,
    CompletelyTransmitted,
    PhysicallySettled,
    RejectedBeforeDispatch,
    Unresolved,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobStatus {
    pub job: JobKey,
    pub transmission: Transmission,
    pub state: JobState,
}
#[derive(Clone, Copy)]
struct Pending {
    job: JobKey,
    fresh: Duration,
    escalate: Duration,
    eligible: bool,
    transmission: Transmission,
    uncertain: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    Pending,
    Confirmed,
    Unconfirmed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub session: u64,
    pub initialize_transmitted: bool,
    pub job: Option<JobStatus>,
    pub phase: Phase,
    pub failure: Option<Failure>,
    pub unresolved: Option<JobKey>,
    pub stopped: bool,
    pub reaped: bool,
    pub termination_requested: bool,
    pub shutdown: Shutdown,
}
/// The first causal failure is retained. A correlated backend-wide fault
/// overrides ordinary failures; an invalid owner clock has highest priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    InitializationTimeout,
    Escalation,
    Unresolved,
    UnexpectedExit,
    Disconnected,
    ProcessObservation,
    Transport,
    Correlation,
    Protocol,
    BackendFault,
    Clock,
}

/// Pure policy machine. Times are owner-supplied monotonic durations from one
/// owner-chosen origin; they are never sent over IPC.
pub struct Machine {
    session: u64,
    policy: Policy,
    phase: Phase,
    failure: Option<Failure>,
    now: Duration,
    init: Duration,
    close: Option<Duration>,
    exit: Option<Duration>,
    pending: Option<Pending>,
    last_job: Option<JobStatus>,
    initialize_transmitted: bool,
    report: Option<Report>,
    ready_until: Option<Duration>,
    last_serial: u64,
    stopped: bool,
    reaped: bool,
    termination_requested: bool,
    unconfirmed: bool,
    shutdown_sent: bool,
    diagnostic: Option<String>,
}
impl Machine {
    pub fn new(session: u64, policy: Policy, now: Duration) -> Self {
        Self {
            session,
            policy,
            phase: Phase::Starting,
            failure: None,
            pending: None,
            last_job: None,
            initialize_transmitted: false,
            report: None,
            ready_until: None,
            diagnostic: None,
            now,
            init: now.saturating_add(policy.initialization),
            close: None,
            exit: None,
            last_serial: 0,
            shutdown_sent: false,
            stopped: false,
            reaped: false,
            termination_requested: false,
            unconfirmed: false,
        }
    }
    /// Transport fact, recorded only after the entire Initialize was written.
    pub fn initialize_transmitted(&mut self) {
        self.initialize_transmitted = true;
    }
    /// Transport fact after a positive write of this Render. `complete` means
    /// its final byte was written, not merely queued. No settlement is implied.
    pub fn render_transmitted(&mut self, job: JobKey, complete: bool) {
        if let Some(p) = self
            .pending
            .as_mut()
            .filter(|p| p.job == job && p.transmission != Transmission::Complete)
        {
            p.transmission = if complete {
                Transmission::Complete
            } else {
                Transmission::Partial
            };
        } else {
            self.fail(Failure::Protocol);
            self.begin_close();
        }
    }
    /// Only the sole transport may call this after proving its Render offset is
    /// zero, as part of withdrawing the whole queued message in the same
    /// exclusive transport operation. Partial/full dispatch
    /// must never use this path; worker evidence is required for those jobs.
    pub(crate) fn reject_before_dispatch(&mut self, job: JobKey) -> bool {
        if matches!(
            self.phase,
            Phase::Starting | Phase::Ready | Phase::Outstanding
        ) || self
            .pending
            .is_none_or(|p| p.job != job || p.transmission != Transmission::Unsent)
        {
            return false;
        }
        self.pending = None;
        self.last_job = Some(JobStatus {
            job,
            transmission: Transmission::Unsent,
            state: JobState::RejectedBeforeDispatch,
        });
        self.report = Some(Report::RejectedBeforeDispatch { job });
        true
    }
    pub fn session(&self) -> u64 {
        self.session
    }
    pub fn outcome(&self) -> Outcome {
        Outcome {
            session: self.session,
            initialize_transmitted: self.initialize_transmitted,
            job: self
                .pending
                .map(|p| JobStatus {
                    job: p.job,
                    transmission: p.transmission,
                    state: if p.uncertain {
                        JobState::Unresolved
                    } else {
                        match p.transmission {
                            Transmission::Unsent => JobState::AdmittedUnsent,
                            Transmission::Partial => JobState::PartiallyTransmitted,
                            Transmission::Complete => JobState::CompletelyTransmitted,
                        }
                    },
                })
                .or(self.last_job),
            phase: self.phase,
            failure: self.failure,
            unresolved: self.pending.map(|p| p.job),
            stopped: self.stopped,
            reaped: self.reaped,
            termination_requested: self.termination_requested,
            shutdown: if self.stopped && self.reaped && !self.unconfirmed && self.pending.is_none()
            {
                Shutdown::Confirmed
            } else if self.unconfirmed || self.reaped {
                Shutdown::Unconfirmed
            } else {
                Shutdown::Pending
            },
        }
    }
    /// At most one bounded worker diagnostic; later diagnostics are discarded.
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
    pub fn deadline(&self) -> Option<Duration> {
        if self.reaped {
            return None;
        }
        let mut deadline = match self.phase {
            Phase::Starting => Some(self.init),
            Phase::Closing => self.close,
            Phase::TerminationRequested => self.exit,
            _ => None,
        };
        if matches!(self.report, Some(Report::Frame { .. })) {
            deadline = match (deadline, self.ready_until) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (None, b) => b,
                (a, None) => a,
            };
        }
        if matches!(self.phase, Phase::Outstanding | Phase::Closing) {
            if let Some(p) = self.pending {
                let next = if p.eligible {
                    p.fresh.min(p.escalate)
                } else {
                    p.escalate
                };
                deadline = Some(deadline.map_or(next, |d| d.min(next)));
            }
        }
        deadline
    }
    fn observe_time(&mut self, now: Duration) -> bool {
        if now < self.now {
            self.fail(Failure::Clock);
            self.begin_close();
            false
        } else {
            self.now = now;
            true
        }
    }
    fn fail(&mut self, failure: Failure) {
        if self.failure.is_none()
            || failure == Failure::Clock
            || (failure == Failure::BackendFault && self.failure != Some(Failure::Clock))
        {
            self.failure = Some(failure);
        }
        self.mark_uncertain();
        self.cancel();
    }
    fn mark_uncertain(&mut self) {
        if let Some(p) = &mut self.pending {
            if p.transmission != Transmission::Unsent {
                p.uncertain = true;
            }
        }
    }
    pub fn cancel(&mut self) {
        if let Some(p) = &mut self.pending {
            p.eligible = false;
        }
        if let Some(Report::Frame { job, .. }) = self.report.as_ref() {
            self.report = Some(Report::Obsolete(*job));
        }
    }
    fn begin_close(&mut self) {
        if matches!(
            self.phase,
            Phase::Starting | Phase::Ready | Phase::Outstanding
        ) {
            self.phase = Phase::Closing;
            self.close = Some(self.now.saturating_add(self.policy.graceful_shutdown));
        }
    }
    pub fn close(&mut self, now: Duration) {
        self.observe_time(now);
        self.cancel();
        self.begin_close();
    }
    pub fn transport_failed(&mut self, failure: Failure, now: Duration) {
        self.observe_time(now);
        self.fail(failure);
        self.begin_close();
    }
    pub fn admit(&mut self, render: Render, now: Duration) -> Result<(), Admission> {
        if !self.observe_time(now) {
            return Err(Admission::Clock);
        }
        self.tick(now);
        if self.pending.is_some() {
            return Err(Admission::Outstanding);
        }
        if self.report.is_some() {
            return Err(Admission::ResultPending);
        }
        if self.phase != Phase::Ready || self.failure.is_some() {
            return Err(Admission::NotReady);
        }
        if render.job.serial <= self.last_serial || render.delta > render.elapsed {
            return Err(Admission::Identity);
        }
        self.last_serial = render.job.serial;
        self.last_job = None;
        self.pending = Some(Pending {
            job: render.job,
            fresh: now.saturating_add(self.policy.freshness),
            escalate: now.saturating_add(self.policy.escalation),
            eligible: true,
            transmission: Transmission::Unsent,
            uncertain: false,
        });
        self.phase = Phase::Outstanding;
        Ok(())
    }
    pub fn receive(&mut self, message: Message, now: Duration) {
        if !self.observe_time(now) {
            return;
        }
        if message.session != self.session {
            self.fail(Failure::Correlation);
            self.begin_close();
            return;
        }
        // Still accept matching settlement evidence after closing/termination.
        match message.body {
            Body::Ready if !self.initialize_transmitted => {
                self.fail(Failure::Protocol);
                self.begin_close();
            }
            Body::Ready if self.phase == Phase::Starting && now < self.init => {
                self.phase = Phase::Ready
            }
            Body::Ready if self.phase == Phase::Starting => {
                self.fail(Failure::InitializationTimeout);
                self.begin_close();
            }
            Body::BackendFault(text) => {
                self.record_diagnostic(text);
                self.fail(Failure::BackendFault);
                self.begin_close();
            }
            Body::Stopped
                if matches!(
                    self.phase,
                    Phase::Closing | Phase::TerminationRequested | Phase::ShutdownUnconfirmed
                ) && self.pending.is_none()
                    && !self.stopped =>
            {
                self.stopped = true
            }
            Body::Frame { job, bgra } => self.complete(job, Some(bgra), None),
            Body::RejectedBeforeSubmission { job, diagnostic } => {
                self.complete(job, None, Some(Ok(diagnostic)))
            }
            Body::JobFailed { job, diagnostic } => self.complete(job, None, Some(Err(diagnostic))),
            Body::Unresolved { job, diagnostic } => {
                if self.pending.is_none_or(|p| p.job != job) {
                    self.fail(Failure::Correlation);
                } else if self
                    .pending
                    .is_some_and(|p| p.transmission != Transmission::Complete)
                {
                    self.fail(Failure::Protocol);
                } else {
                    self.record_diagnostic(diagnostic);
                    self.fail(Failure::Unresolved);
                }
                self.begin_close();
            }
            _ => {
                self.fail(Failure::Protocol);
                self.begin_close();
            }
        }
    }
    fn record_diagnostic(&mut self, text: String) {
        // Defense for direct callers too, without slicing through UTF-8.
        if self.diagnostic.is_none() {
            let mut end = text.len().min(roomie_worker_protocol::MAX_DIAGNOSTIC);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.diagnostic = Some(text[..end].to_owned());
        }
    }
    fn complete(
        &mut self,
        job: JobKey,
        bgra: Option<Vec<u8>>,
        failure: Option<Result<String, String>>,
    ) {
        let Some(p) = self.pending.filter(|p| p.job == job) else {
            self.fail(Failure::Correlation);
            self.begin_close();
            return;
        };
        if p.transmission != Transmission::Complete {
            self.fail(Failure::Protocol);
            self.begin_close();
            return;
        }
        if bgra
            .as_ref()
            .is_some_and(|bytes| bytes.len() != roomie_worker_protocol::FRAME_BYTES)
        {
            self.fail(Failure::Protocol);
            self.begin_close();
            return;
        }
        self.pending = None; // These post-dispatch worker assertions discharge debt.
        self.last_job = Some(JobStatus {
            job,
            transmission: p.transmission,
            state: JobState::PhysicallySettled,
        });
        if self.phase == Phase::Outstanding {
            self.phase = Phase::Ready;
        }
        self.ready_until = Some(p.fresh);
        self.report = Some(match failure {
            Some(Ok(diagnostic)) => Report::Rejected { job, diagnostic },
            Some(Err(diagnostic)) => Report::Failed { job, diagnostic },
            None if p.eligible
                && self.now < p.fresh
                && self.failure.is_none()
                && self.phase == Phase::Ready =>
            {
                Report::Frame {
                    job,
                    bgra: bgra.expect("frame completion"),
                }
            }
            None => Report::Obsolete(job),
        });
    }
    pub fn take_report(&mut self) -> Option<Report> {
        self.report.take()
    }
    /// Must be called after the current bounded read batch and before output
    /// consumption. Terminal reports observed this turn precede escalation.
    pub fn tick(&mut self, now: Duration) {
        if !self.observe_time(now) || self.reaped {
            return;
        }
        if self.ready_until.is_some_and(|d| now >= d) {
            if let Some(Report::Frame { job, .. }) = self.report.as_ref() {
                self.report = Some(Report::Obsolete(*job));
            }
        }
        if let Some(p) = &mut self.pending {
            if now >= p.fresh {
                p.eligible = false;
            }
        }
        if self.phase == Phase::Starting && now >= self.init {
            self.fail(Failure::InitializationTimeout);
            self.begin_close();
        }
        if matches!(self.phase, Phase::Outstanding | Phase::Closing)
            && self.pending.is_some_and(|p| now >= p.escalate)
        {
            self.fail(Failure::Escalation);
            self.request_termination();
        }
        if self.phase == Phase::Closing && self.close.is_some_and(|d| now >= d) {
            self.request_termination();
        }
        if self.phase == Phase::TerminationRequested && self.exit.is_some_and(|d| now >= d) {
            self.unconfirmed = true;
            self.phase = Phase::ShutdownUnconfirmed;
        }
    }
    fn request_termination(&mut self) {
        if !self.termination_requested && !self.reaped {
            self.mark_uncertain();
            self.cancel();
            self.termination_requested = true;
            self.unconfirmed = true;
            self.phase = Phase::TerminationRequested;
            self.exit = Some(self.now.saturating_add(self.policy.exit_observation));
        }
    }
    pub(crate) fn needs_shutdown(&self) -> bool {
        self.phase == Phase::Closing && !self.shutdown_sent
    }
    pub(crate) fn sent_shutdown(&mut self) {
        self.shutdown_sent = true;
    }
    pub fn exited(&mut self, success: bool, now: Duration) {
        self.observe_time(now);
        if !self.stopped || !success {
            self.fail(Failure::UnexpectedExit);
            self.unconfirmed = true;
        }
        self.reaped = true;
        self.phase = Phase::Exited;
    }
}

#[cfg(test)]
mod tests;

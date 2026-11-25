//! Pure owner-side scheduling for asynchronous GPU frame preparation.
//!
//! Feed worker reports and owner-observed monotonic time into this state
//! machine. Native scheduling, staging and publication belong to the callers.

use std::time::Instant;

use super::{FrameIdentity, FrameRequest, PreparedFrame};

/// Owner-issued correlation for a GPU job, independent of native submissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobIdentity {
    pub session: u64,
    pub epoch: u64,
    pub serial: u64,
    pub frame: FrameIdentity,
}

/// One dispatch token carrying the original, immutable frame request.
#[derive(Debug, PartialEq, Eq)]
pub struct GpuJob {
    identity: JobIdentity,
    request: FrameRequest,
}

/// One settled candidate with the original admission facts.
pub struct ReadyFrame<P> {
    pub identity: JobIdentity,
    pub request: FrameRequest,
    pub deadline: Instant,
    pub frame: PreparedFrame<P>,
}

impl GpuJob {
    pub fn identity(&self) -> JobIdentity {
        self.identity
    }

    pub fn request(&self) -> FrameRequest {
        self.request
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Shutdown,
    DeviceFailure,
    EpochExhausted,
    SerialExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleRejection {
    Paused,
    Outstanding,
    ResultPending,
    DeadlineExpired,
    Closed(CloseReason),
}

/// A worker report about one job. Terminal reports certify that the worker has
/// settled all associated GPU and staging work. A successful frame owns its
/// payload independently of the staging slot.
#[derive(Debug)]
pub enum JobCompletion<P> {
    Frame(PreparedFrame<P>),
    /// A correlated successful worker result whose payload was discarded by
    /// supervision after invalidation. This certifies settlement only and can
    /// never create a ready frame, even if eligibility has since resumed.
    SettledWithoutPayload,
    RejectedBeforeSubmission,
    Failed(String),
    /// Settlement is unproven. A terminal report is still owed for this job.
    Unresolved(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CompletionStatus {
    Ready,
    Obsolete,
    RejectedBeforeSubmission,
    Failed(String),
    Unresolved(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionError {
    UnexpectedJob,
    FrameIdentityMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Eligibility {
    Paused,
    Active,
    Closed(CloseReason),
}

enum Work<P> {
    Idle,
    Pending {
        identity: JobIdentity,
        request: FrameRequest,
        deadline: Instant,
        // Timeout or uncertainty retires the wait interest, never the debt.
        fresh: bool,
        unresolved: bool,
    },
    Ready(ReadyFrame<P>),
}

/// Owns one job or one GPU-ready frame. The owner supplies requests explicitly
/// and consumes ready frames for checked publication in the same owner turn.
pub struct GpuHandoff<P> {
    session: u64,
    epoch: u64,
    serial: u64,
    eligibility: Eligibility,
    work: Work<P>,
}

impl<P> GpuHandoff<P> {
    /// Starts paused. Use a fresh session ID wherever prior reports can still
    /// arrive, and retain this coordinator until its outstanding work settles.
    pub fn new(session: u64) -> Self {
        Self {
            session,
            epoch: 0,
            serial: 0,
            eligibility: Eligibility::Paused,
            work: Work::Idle,
        }
    }

    pub fn resume(&mut self) -> Result<(), CloseReason> {
        if let Eligibility::Closed(reason) = self.eligibility {
            return Err(reason);
        }
        self.eligibility = Eligibility::Active;
        Ok(())
    }

    /// Cancels eligibility for the current epoch. Pending work still owns its
    /// slot until settlement, even if the owner resumes in the meantime.
    pub fn interrupt(&mut self) -> Result<(), CloseReason> {
        match self.eligibility {
            Eligibility::Paused => return Ok(()),
            Eligibility::Closed(reason) => return Err(reason),
            Eligibility::Active => {}
        }
        self.discard_ready();
        let Some(epoch) = self.epoch.checked_add(1) else {
            self.eligibility = Eligibility::Closed(CloseReason::EpochExhausted);
            return Err(CloseReason::EpochExhausted);
        };
        self.epoch = epoch;
        self.eligibility = Eligibility::Paused;
        Ok(())
    }

    /// Reserves the physical slot for one job. The owner chooses the deadline
    /// and must report a pre-submission rejection if dispatch is refused, or
    /// an unresolved completion if dispatch may have occurred.
    pub fn schedule(
        &mut self,
        request: FrameRequest,
        now: Instant,
        deadline: Instant,
    ) -> Result<GpuJob, ScheduleRejection> {
        match self.eligibility {
            Eligibility::Paused => return Err(ScheduleRejection::Paused),
            Eligibility::Closed(reason) => return Err(ScheduleRejection::Closed(reason)),
            Eligibility::Active => {}
        }
        match self.work {
            Work::Pending { .. } => return Err(ScheduleRejection::Outstanding),
            Work::Ready(..) => return Err(ScheduleRejection::ResultPending),
            Work::Idle => {}
        }
        if now >= deadline {
            return Err(ScheduleRejection::DeadlineExpired);
        }
        let Some(serial) = self.serial.checked_add(1) else {
            self.eligibility = Eligibility::Closed(CloseReason::SerialExhausted);
            return Err(ScheduleRejection::Closed(CloseReason::SerialExhausted));
        };
        self.serial = serial;
        let identity = JobIdentity {
            session: self.session,
            epoch: self.epoch,
            serial,
            frame: request.identity(),
        };
        self.work = Work::Pending {
            identity,
            request,
            deadline,
            fresh: true,
            unresolved: false,
        };
        Ok(GpuJob { identity, request })
    }

    pub fn outstanding(&self) -> Option<JobIdentity> {
        match self.work {
            Work::Pending { identity, .. } => Some(identity),
            _ => None,
        }
    }

    /// The original request for the outstanding physical obligation.
    pub fn outstanding_request(&self) -> Option<FrameRequest> {
        match self.work {
            Work::Pending { request, .. } => Some(request),
            _ => None,
        }
    }

    pub fn unresolved(&self) -> Option<JobIdentity> {
        match self.work {
            Work::Pending {
                identity,
                unresolved: true,
                ..
            } => Some(identity),
            _ => None,
        }
    }

    pub fn ready(&self) -> Option<&ReadyFrame<P>> {
        match &self.work {
            Work::Ready(ready) => Some(ready),
            _ => None,
        }
    }

    /// One pending-work wakeup deadline, including canceled jobs awaiting settlement.
    pub fn deadline(&self) -> Option<Instant> {
        match &self.work {
            Work::Pending {
                deadline,
                fresh: true,
                ..
            } => Some(*deadline),
            _ => None,
        }
    }

    /// Includes the freshness deadline of an unconsumed ready candidate.
    pub fn wait_deadline(&self) -> Option<Instant> {
        self.ready().map(|ready| ready.deadline).or(self.deadline())
    }

    /// Reports expiry once and revokes this job's result eligibility. Physical
    /// settlement is still owed. Equality with the deadline counts as expiry.
    pub fn expire(&mut self, now: Instant) -> Option<JobIdentity> {
        match &mut self.work {
            Work::Pending {
                identity,
                deadline,
                fresh,
                ..
            } if *fresh && now >= *deadline => {
                *fresh = false;
                Some(*identity)
            }
            Work::Ready(ready) if now >= ready.deadline => {
                let identity = ready.identity;
                self.work = Work::Idle;
                Some(identity)
            }
            _ => None,
        }
    }

    /// Accepts a worker report, for example after a future owner wakeup. Use
    /// the time the owner observes the report, so a late frame stays obsolete
    /// even if the owner has yet to service the deadline.
    pub fn complete(
        &mut self,
        identity: JobIdentity,
        completion: JobCompletion<P>,
        now: Instant,
    ) -> Result<CompletionStatus, CompletionError> {
        let Work::Pending {
            identity: pending,
            request,
            deadline,
            fresh,
            unresolved,
        } = &mut self.work
        else {
            return Err(CompletionError::UnexpectedJob);
        };
        if *pending != identity {
            return Err(CompletionError::UnexpectedJob);
        }
        let request = *request;
        let eligible = self.eligibility == Eligibility::Active
            && identity.epoch == self.epoch
            && *fresh
            && now < *deadline;
        let status = match completion {
            JobCompletion::Unresolved(error) => {
                *fresh = false;
                *unresolved = true;
                return Ok(CompletionStatus::Unresolved(error));
            }
            JobCompletion::Frame(_) if !eligible => Ok(CompletionStatus::Obsolete),
            JobCompletion::Frame(frame) if frame.identity != request.identity() => {
                Err(CompletionError::FrameIdentityMismatch)
            }
            JobCompletion::Frame(frame) => {
                self.work = Work::Ready(ReadyFrame {
                    identity,
                    request,
                    deadline: *deadline,
                    frame,
                });
                return Ok(CompletionStatus::Ready);
            }
            JobCompletion::SettledWithoutPayload => Ok(CompletionStatus::Obsolete),
            JobCompletion::RejectedBeforeSubmission => {
                Ok(CompletionStatus::RejectedBeforeSubmission)
            }
            JobCompletion::Failed(error) => Ok(CompletionStatus::Failed(error)),
        };
        self.work = Work::Idle;
        status
    }

    /// Transfers the original request and candidate to the owner once. The
    /// caller must validate and publish it before processing another owner event.
    pub fn take_ready(&mut self) -> Option<(FrameRequest, PreparedFrame<P>)> {
        self.take_ready_frame()
            .map(|ready| (ready.request, ready.frame))
    }

    /// One-shot transfer with the full admitted identity and deadline.
    pub fn take_ready_frame(&mut self) -> Option<ReadyFrame<P>> {
        if !matches!(self.work, Work::Ready(..)) {
            return None;
        }
        let Work::Ready(ready) = std::mem::replace(&mut self.work, Work::Idle) else {
            unreachable!("ready frame checked above");
        };
        Some(ready)
    }

    /// Latches a device-wide failure from this backend, independently of any
    /// frame report. The owner handles its diagnostic and any eventual job
    /// settlement separately, including when the affected frame is obsolete.
    pub fn device_failed(&mut self) {
        self.eligibility = Eligibility::Closed(CloseReason::DeviceFailure);
        self.discard_ready();
    }

    /// Closes admission and drops any ready frame. A pending job is still
    /// observable through `outstanding` until a matching terminal report.
    pub fn shutdown(&mut self) {
        if !matches!(self.eligibility, Eligibility::Closed(_)) {
            self.eligibility = Eligibility::Closed(CloseReason::Shutdown);
        }
        self.discard_ready();
    }

    fn discard_ready(&mut self) {
        if matches!(self.work, Work::Ready(..)) {
            self.work = Work::Idle;
        }
    }
}

#[cfg(test)]
mod tests;

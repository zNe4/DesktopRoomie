//! Shared transfer algorithm, exercised without a GPU through Operations.
use crate::{diagnostic, layout::Layout, settlement::*};
use std::ops::Deref;
use std::sync::{
    mpsc::{self, Receiver, SyncSender},
    Arc,
};
use std::time::{Duration, Instant};

pub const WAIT_BUDGET: Duration = Duration::from_secs(2);

use crate::native_contract::checked_call;
pub use crate::native_contract::Faults;

#[derive(Debug)]
pub struct Completion {
    pub operation: Operation,
    pub result: Result<(), String>,
}
pub struct MapReceiver {
    expected: Operation,
    receiver: Receiver<Completion>,
}
impl MapReceiver {
    pub fn new(expected: Operation) -> (Self, SyncSender<Completion>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        (Self { expected, receiver }, sender)
    }
    pub fn complete(self, deadline: Instant, evidence: &mut Evidence) -> Result<(), String> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("mapping deadline")?;
        let report = self
            .receiver
            .recv_timeout(remaining)
            .map_err(|_| "mapping callback missing or delayed")?;
        if report.operation != self.expected {
            return Err("stale mapping callback".into());
        }
        // A matching result is evidence even if observed at the deadline.
        // It cannot turn into frame success after the budget expires.
        evidence.mapping = if report.result.is_ok() {
            Mapping::Confirmed
        } else {
            Mapping::Failed
        };
        report.result.map_err(|s| diagnostic(&s))?;
        if Instant::now() >= deadline {
            return Err("mapping deadline".into());
        }
        Ok(())
    }
}

/// Implementations own exactly one device/queue, pipeline, target/view and
/// staging buffer. Only command encoding and tagged callback channels recur.
pub trait Operations {
    type Submission;
    type Commands;
    type View<'a>: Deref<Target = [u8]>
    where
        Self: 'a;
    fn encode(&mut self, phase: u32) -> Result<Self::Commands, String>;
    fn submit(&mut self, commands: Self::Commands) -> Result<Self::Submission, String>;
    fn wait(&mut self, submission: &Self::Submission, remaining: Duration) -> Result<(), String>;
    fn map(&mut self, operation: Operation, sender: SyncSender<Completion>) -> Result<(), String>;
    fn view(&self) -> Self::View<'_>;
    fn unmap(&mut self) -> Result<(), String>;
    /// Affirm native teardown preflight/intentional destruction before owning
    /// wrappers may be dropped. Any error must leave the owner retained.
    fn prepare_release(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// Nonblocking native progress for idle fault delivery. An error here
    /// means native state is uncertain, not just a latched device fault.
    fn observe(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Encode,
    SubmissionAttempted,
    SubmissionAccepted,
    SubmissionComplete,
    MapRequested,
    MapComplete,
    ViewLive,
    ViewReleased,
    Unmapped,
    Packed,
    Retired,
}
#[derive(Debug, Default, Clone)]
pub struct Telemetry {
    pub adapter: Option<Adapter>,
    pub initialization: Duration,
    pub initialization_complete: bool,
    pub device_initialization: Duration,
    pub pipeline_initialization: Duration,
    pub submissions: u64,
    pub accepted_submissions: u64,
    pub completed_submissions: u64,
    pub mapping_requests: u64,
    pub mapping_completions: u64,
    pub successful_maps: u64,
    pub explicit_unmaps: u64,
    pub staging_allocations: u64,
    pub staging_reuses: u64,
    pub staging_retirements: u64,
    pub peak_outstanding: u8,
    /// Only the most recent operation, at most ten stages; never a frame log.
    pub stages: Vec<Stage>,
    pub preparation: Duration,
    pub times: StageTimes,
    pub total_times: StageTimes,
    pub timed_operations: u64,
    pub controlled_shutdown: bool,
    pub uncertain_shutdown: bool,
}
/// Fixed-size, last-operation timings, including failed attempts.
#[derive(Debug, Default, Clone, Copy)]
pub struct StageTimes {
    pub encode: Duration,
    pub submission: Duration,
    pub submission_wait: Duration,
    pub mapping: Duration,
    pub repack: Duration,
}
#[derive(Debug)]
pub struct Prepared {
    pub bgra: Vec<u8>,
    pub evidence: Evidence,
}
#[derive(Debug, Clone)]
pub struct Adapter {
    pub name: String,
    pub device_type: String,
    pub backend: String,
    pub driver: String,
}
#[derive(Debug)]
pub struct Failure {
    pub diagnostic: String,
    pub evidence: Evidence,
}
pub trait Backend {
    /// Read-only diagnostics; never an input to settlement or release.
    fn acceptance_telemetry(&self) -> Option<&Telemetry> {
        None
    }
    fn initialize(&mut self) -> Result<(), String>;
    fn render(&mut self, operation: Operation) -> Result<Prepared, Failure>;
    fn fault(&mut self) -> Option<String>;
    fn settled(&self) -> bool;
    fn evidence(&self) -> Option<Evidence>;
    fn reject(&mut self, operation: Operation) -> Option<Evidence>;
    /// True only after all resources have actually been released safely.
    fn release(&mut self) -> bool;
}

pub struct Persistent<O: Operations> {
    ops: Option<O>,
    faults: Arc<Faults>,
    telemetry: Telemetry,
    evidence: Option<Evidence>,
    slot: Slot,
    failed: bool,
    uncertain_native: bool,
    release_started: bool,
    staging_used: bool,
}
impl<O: Operations> Persistent<O> {
    pub fn new(ops: O, faults: Arc<Faults>, mut telemetry: Telemetry) -> Self {
        telemetry.staging_allocations = 1;
        Self {
            ops: Some(ops),
            faults,
            telemetry,
            evidence: None,
            slot: Slot::Available,
            failed: false,
            uncertain_native: false,
            release_started: false,
            staging_used: false,
        }
    }
    pub fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }
    pub fn evidence(&self) -> Option<Evidence> {
        self.evidence
    }
    pub fn slot(&self) -> Slot {
        self.slot
    }
    /// The only authority for both explicit release and defensive Drop.
    pub fn release_eligible(&self) -> bool {
        !self.uncertain_native
            && !self.release_started
            && !self.faults.device_lost()
            && self.slot != Slot::InFlight
            && self.evidence.is_none_or(Evidence::settled)
    }
    fn check(&self) -> Result<(), String> {
        self.faults.get().map_or(Ok(()), Err)
    }
    fn transfer(&mut self, evidence: &mut Evidence) -> Result<Vec<u8>, String> {
        self.check()?;
        let started = Instant::now();
        let commands = self
            .ops
            .as_mut()
            .ok_or("backend released")?
            .encode(evidence.operation.phase);
        self.telemetry.times.encode = started.elapsed();
        let commands = commands?;
        self.telemetry.stages.push(Stage::Encode);
        self.check()?;
        let deadline = Instant::now() + WAIT_BUDGET;
        evidence.submission = Submission::Attempted;
        self.telemetry.stages.push(Stage::SubmissionAttempted);
        self.telemetry.submissions += 1;
        self.telemetry.peak_outstanding = 1;
        let started = Instant::now();
        let submission = checked_call(&self.faults, || self.ops.as_mut().unwrap().submit(commands));
        self.telemetry.times.submission = started.elapsed();
        let submission = submission?;
        evidence.submission = Submission::IndexReturned;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("submission deadline")?;
        let started = Instant::now();
        let wait = checked_call(&self.faults, || {
            self.ops.as_mut().unwrap().wait(&submission, remaining)
        });
        self.telemetry.times.submission_wait = started.elapsed();
        wait?;
        // The pinned core rejects a target beyond last_successful_submission;
        // a quiet successful specific wait proves acceptance AND completion.
        evidence.submission = Submission::Accepted;
        self.telemetry.accepted_submissions = self.telemetry.accepted_submissions.saturating_add(1);
        self.telemetry.stages.push(Stage::SubmissionAccepted);
        evidence.submission = Submission::Complete;
        self.telemetry.completed_submissions =
            self.telemetry.completed_submissions.saturating_add(1);
        self.telemetry.stages.push(Stage::SubmissionComplete);
        self.check()?;
        let started = Instant::now();
        let mapping = self.map(&submission, deadline, evidence);
        self.telemetry.times.mapping = started.elapsed();
        if let Err(error) = mapping {
            self.faults.retain(&error);
            if matches!(evidence.mapping, Mapping::Confirmed | Mapping::Failed) {
                // Cleanup failure must not replace the original diagnostic.
                self.unmap(evidence)?;
            }
            return Err(error);
        }
        if let Err(error) = self.check() {
            self.unmap(evidence)?;
            return Err(error);
        }
        let started = Instant::now();
        let packed = {
            evidence.view = View::Live;
            self.telemetry.stages.push(Stage::ViewLive);
            let view = self.ops.as_ref().unwrap().view();
            Layout::fixture()
                .unwrap()
                .repack(&view)
                .map_err(|e| format!("readback layout: {e:?}"))
        };
        evidence.view = View::Released;
        self.telemetry.times.repack = started.elapsed();
        self.telemetry.stages.push(Stage::ViewReleased);
        if let Err(error) = &packed {
            self.faults.retain(error);
        }
        self.unmap(evidence)?;
        self.check()?;
        let bytes = packed?;
        self.telemetry.stages.push(Stage::Packed);
        Ok(bytes)
    }
    fn map(
        &mut self,
        submission: &O::Submission,
        deadline: Instant,
        evidence: &mut Evidence,
    ) -> Result<(), String> {
        if Instant::now() >= deadline {
            return Err("submission deadline".into());
        }
        let (mapping, sender) = MapReceiver::new(evidence.operation);
        evidence.mapping = Mapping::Requested;
        self.telemetry.mapping_requests += 1;
        self.telemetry.stages.push(Stage::MapRequested);
        checked_call(&self.faults, || {
            self.ops.as_mut().unwrap().map(evidence.operation, sender)
        })?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("mapping deadline")?;
        checked_call(&self.faults, || {
            self.ops.as_mut().unwrap().wait(submission, remaining)
        })?;
        let completion = mapping.complete(deadline, evidence);
        if evidence.mapping == Mapping::Confirmed {
            self.telemetry.successful_maps = self.telemetry.successful_maps.saturating_add(1);
        }
        if matches!(evidence.mapping, Mapping::Confirmed | Mapping::Failed) {
            self.telemetry.mapping_completions += 1;
            self.telemetry.stages.push(Stage::MapComplete);
        }
        completion
    }
    fn unmap(&mut self, evidence: &mut Evidence) -> Result<(), String> {
        // Operations::unmap additionally owes its backend's affirmative
        // cleanup proof. This outer check rejects even a lying normal return
        // that reported an error through the shared native error-handler seam.
        checked_call(&self.faults, || self.ops.as_mut().unwrap().unmap())?;
        evidence.unmapped = true;
        self.telemetry.explicit_unmaps += 1;
        self.telemetry.stages.push(Stage::Unmapped);
        Ok(())
    }
}
impl<O: Operations> Backend for Persistent<O> {
    fn acceptance_telemetry(&self) -> Option<&Telemetry> {
        Some(&self.telemetry)
    }
    fn initialize(&mut self) -> Result<(), String> {
        self.check()
    }
    fn render(&mut self, operation: Operation) -> Result<Prepared, Failure> {
        let mut evidence = Evidence::new(operation);
        if self.failed || self.slot != Slot::Available || self.ops.is_none() {
            evidence.uncertain = true;
            return Err(Failure {
                diagnostic: "backend admission closed".into(),
                evidence,
            });
        }
        self.slot = Slot::InFlight;
        self.telemetry.staging_reuses += u64::from(self.staging_used);
        self.staging_used = true;
        self.telemetry.stages.clear();
        self.telemetry.times = StageTimes::default();
        let started = Instant::now();
        let result = self.transfer(&mut evidence);
        self.telemetry.preparation = started.elapsed();
        self.telemetry.timed_operations = self.telemetry.timed_operations.saturating_add(1);
        let t = self.telemetry.times;
        let total = &mut self.telemetry.total_times;
        total.encode = total.encode.saturating_add(t.encode);
        total.submission = total.submission.saturating_add(t.submission);
        total.submission_wait = total.submission_wait.saturating_add(t.submission_wait);
        total.mapping = total.mapping.saturating_add(t.mapping);
        total.repack = total.repack.saturating_add(t.repack);
        if result.is_err() {
            self.failed = true;
            self.telemetry.uncertain_shutdown = !evidence.settled();
            evidence.retired = true;
            self.slot = Slot::Retired;
            self.telemetry.staging_retirements += 1;
            self.telemetry.stages.push(Stage::Retired);
        } else {
            self.slot = Slot::Available;
        }
        self.evidence = Some(evidence);
        result
            .map(|bgra| Prepared { bgra, evidence })
            .map_err(|text| {
                self.faults.retain(&text);
                Failure {
                    diagnostic: diagnostic(&text),
                    evidence,
                }
            })
    }
    fn fault(&mut self) -> Option<String> {
        // Do not touch an uncertain device again just to observe faults.
        if !self.failed
            && self.faults.get().is_none()
            && !self.uncertain_native
            && self.evidence.is_none_or(Evidence::settled)
        {
            if let Some(ops) = self.ops.as_mut() {
                if let Err(error) = ops.observe() {
                    self.uncertain_native = true;
                    self.telemetry.uncertain_shutdown = true;
                    self.failed = true;
                    self.faults.record(&error);
                }
            }
        }
        self.faults.get()
    }
    fn settled(&self) -> bool {
        self.release_eligible()
    }
    fn evidence(&self) -> Option<Evidence> {
        self.evidence
    }
    fn reject(&mut self, operation: Operation) -> Option<Evidence> {
        if !self.release_eligible() {
            return None;
        }
        let evidence = Evidence::new(operation);
        self.evidence = Some(evidence);
        Some(evidence)
    }
    fn release(&mut self) -> bool {
        self.failed = true;
        if !self.release_eligible() {
            self.telemetry.uncertain_shutdown = true;
            return false;
        }
        if self.telemetry.controlled_shutdown {
            return true;
        }
        // Sticky before any native teardown. If preparation or wrapper Drop
        // unwinds to an external caller, neither release nor Drop may retry.
        self.release_started = true;
        self.telemetry.uncertain_shutdown = true;
        let before = self.faults.revision();
        let preparation = self
            .ops
            .as_mut()
            .map_or(Ok(()), Operations::prepare_release);
        if preparation.is_err() || self.faults.device_lost() || self.faults.revision() != before {
            if let Err(error) = preparation {
                self.faults.retain(&error);
            }
            self.uncertain_native = true;
            self.telemetry.uncertain_shutdown = true;
            return false;
        }
        drop(self.ops.take());
        if self.faults.revision() != before || self.faults.device_lost() {
            self.uncertain_native = true;
            self.telemetry.uncertain_shutdown = true;
            return false;
        }
        self.release_started = false;
        self.telemetry.uncertain_shutdown = false;
        self.telemetry.controlled_shutdown = true;
        true
    }
}
impl<O: Operations> Drop for Persistent<O> {
    fn drop(&mut self) {
        if self.release_eligible() {
            self.release();
        }
        // Either already released, or explicit release/preflight could not
        // prove safety. Never let the remaining Option run native Drop.
        if let Some(ops) = self.ops.take() {
            std::mem::forget(ops);
        }
    }
}

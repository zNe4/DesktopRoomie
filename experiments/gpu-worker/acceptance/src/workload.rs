use crate::{
    adapter::{Adapter, Event},
    oracle,
    policy::LATENCY_REFERENCE,
    scheduler::{self, Samples, Scheduler},
};
use roomie_worker_supervisor::machine::{Outcome, Shutdown};
use std::time::Duration;

#[derive(Debug, Default)]
pub struct Exact {
    pub warmup: u64,
    pub measured: u64,
    pub phases: [u64; 5],
    pub ipc: Samples,
    pub end_to_end: Samples,
    pub warmup_ipc: Samples,
    pub warmup_end_to_end: Samples,
    /// Owner-observed frame latencies at/above the original freshness limit.
    /// Includes obsolete frame settlements; never establishes pixel mismatch.
    pub warmup_reference_violations: u64,
    pub measured_reference_violations: u64,
}
impl Exact {
    pub fn record_reference_latency(&mut self, warmup: bool, latency: Duration) {
        if latency >= LATENCY_REFERENCE {
            if warmup {
                self.warmup_reference_violations += 1;
            } else {
                self.measured_reference_violations += 1;
            }
        }
    }
    pub fn latency_reference_summary(&self) -> String {
        format!("exact latency_reference_ms={} violation=owner_frame_end_to_end_at_or_above_reference warmup_violations={} measured_violations={} total_violations={} reference_includes_obsolete_frame_observations=true latency_violation_is_pixel_mismatch=false real_time_performance_acceptance=false",
            LATENCY_REFERENCE.as_millis(), self.warmup_reference_violations,
            self.measured_reference_violations,
            self.warmup_reference_violations + self.measured_reference_violations)
    }
}
pub fn exact(adapter: &mut Adapter) -> Result<Exact, String> {
    exact_observed(adapter, |_| {})
}
/// Retain partial timing evidence even when exact eligibility/validation fails.
pub fn exact_observed(
    adapter: &mut Adapter,
    observe: impl FnOnce(&Exact),
) -> Result<Exact, String> {
    let mut result = Exact::default();
    let body = (|| {
        adapter.wait_ready()?;
        for sweep in 0..25 {
            for (phase, ms) in oracle::SAMPLE_MS.into_iter().enumerate() {
                let request =
                    oracle::request((sweep * 5 + phase + 1) as u64, Duration::from_millis(ms));
                let admitted = adapter.now();
                let id = adapter.submit(request, admitted)?;
                let event = adapter.wait_event()?;
                if matches!(&event, Event::Frame { job, .. } | Event::Obsolete(job) if *job == id) {
                    result.record_reference_latency(
                        sweep < 5,
                        adapter.observed_at().saturating_sub(admitted),
                    );
                }
                match event {
                    Event::Frame {
                        request: original,
                        job,
                        ipc,
                        end_to_end,
                    } if original == request && id == job => {
                        result.phases[phase] += 1;
                        if sweep < 5 {
                            result.warmup += 1;
                            result.warmup_ipc.push(ipc)?;
                            result.warmup_end_to_end.push(end_to_end)?;
                        } else {
                            result.measured += 1;
                            result.ipc.push(ipc)?;
                            result.end_to_end.push(end_to_end)?;
                        }
                    }
                    event => return Err(format!("exact frame not accepted: {event:?}")),
                }
            }
        }
        Ok(())
    })();
    let finish = adapter.finish();
    observe(&result);
    let outcome = finish?;
    body?;
    controlled(outcome)?;
    let c = adapter.counters();
    if c.attempts != 125
        || c.admitted != 125
        || c.dispatched != 125
        || c.valid != 125
        || c.obsolete != 0
        || c.failed != 0
        || adapter.outstanding().is_some()
    {
        return Err("exact workload accounting mismatch".into());
    }
    Ok(result)
}
pub fn controlled(outcome: Outcome) -> Result<(), String> {
    if outcome.shutdown == Shutdown::Confirmed
        && outcome.reaped
        && outcome.stopped
        && outcome.unresolved.is_none()
        && outcome.failure.is_none()
    {
        Ok(())
    } else {
        Err(format!("shutdown not confirmed clean: {outcome:?}"))
    }
}

#[derive(Debug, Default)]
pub struct Segment {
    pub scheduled: u64,
    pub attempts: u64,
    pub admitted: u64,
    pub dispatched: u64,
    pub busy: u64,
    pub admission_failures: u64,
    pub missed: u64,
    pub skipped: u64,
    pub valid: u64,
    pub obsolete: u64,
    pub withdrawn: u64,
    pub rejected: u64,
    pub settled_failures: u64,
    pub failures: u64,
    pub unresolved: u64,
    pub late_completed: u64,
    pub late: u64,
}
#[derive(Debug, Clone, Copy)]
struct Tracked {
    id: desktop_probe::presentation::gpu_handoff::JobIdentity,
    dispatched: bool,
    terminal: bool,
}
#[derive(Debug)]
pub struct Stress {
    pub warmup: Segment,
    pub measured: Segment,
    pub lateness: Samples,
    pub cadence: Samples,
    pub ipc: Samples,
    pub end_to_end: Samples,
    /// Actual pacing end minus the two-second warmup, including overrun.
    pub measured_elapsed: Duration,
    /// Denominator for the fixed [2s,12s) observation window.
    pub window_elapsed: Duration,
    /// Actual end of pacing, relative to the post-initialization origin.
    pub pacing_elapsed: Duration,
    /// Valid frames observed in [2s,12s), irrespective of request cohort.
    pub window_valid: u64,
    pub window_obsolete: u64,
    pub failure: Option<String>,
    pub shutdown: Option<Outcome>,
    jobs: [Option<Tracked>; scheduler::TOTAL],
}
impl Default for Stress {
    fn default() -> Self {
        Self {
            warmup: Segment {
                scheduled: 120,
                ..Default::default()
            },
            measured: Segment {
                scheduled: 600,
                ..Default::default()
            },
            lateness: Samples::default(),
            cadence: Samples::default(),
            ipc: Samples::default(),
            end_to_end: Samples::default(),
            measured_elapsed: Duration::ZERO,
            window_elapsed: Duration::ZERO,
            pacing_elapsed: Duration::ZERO,
            window_valid: 0,
            window_obsolete: 0,
            failure: None,
            shutdown: None,
            jobs: [None; scheduler::TOTAL],
        }
    }
}
impl Stress {
    pub fn throughput(&self) -> f64 {
        if self.window_elapsed.is_zero() {
            0.0
        } else {
            self.window_valid as f64 / self.window_elapsed.as_secs_f64()
        }
    }
    pub fn settled_frame_throughput(&self) -> f64 {
        if self.window_elapsed.is_zero() {
            0.0
        } else {
            (self.window_valid + self.window_obsolete) as f64 / self.window_elapsed.as_secs_f64()
        }
    }
    pub fn observe_pacing_end(&mut self, elapsed: Duration) {
        self.pacing_elapsed = elapsed;
        self.measured_elapsed = elapsed.saturating_sub(Duration::from_secs(2));
        self.window_elapsed = elapsed
            .min(scheduler::END)
            .saturating_sub(Duration::from_secs(2));
    }
    fn slot(id: desktop_probe::presentation::gpu_handoff::JobIdentity) -> Result<usize, String> {
        let index = usize::try_from(id.frame.sequence).map_err(|_| "invalid stress sequence")?;
        if !(1..=scheduler::TOTAL).contains(&index) {
            return Err("invalid stress sequence".into());
        }
        Ok(index - 1)
    }
    fn segment_mut(&mut self, index: usize) -> &mut Segment {
        if scheduler::segment(index) == 1 {
            &mut self.measured
        } else {
            &mut self.warmup
        }
    }
    pub fn admit(
        &mut self,
        id: desktop_probe::presentation::gpu_handoff::JobIdentity,
    ) -> Result<(), String> {
        let i = Self::slot(id)?;
        if self.jobs[i].is_some() || self.jobs.iter().flatten().any(|j| !j.terminal) {
            return Err("overlapping or duplicate stress admission".into());
        }
        self.jobs[i] = Some(Tracked {
            id,
            dispatched: false,
            terminal: false,
        });
        self.segment_mut(i).admitted += 1;
        Ok(())
    }
    pub fn dispatch(
        &mut self,
        id: desktop_probe::presentation::gpu_handoff::JobIdentity,
    ) -> Result<(), String> {
        let i = Self::slot(id)?;
        let job = self.jobs[i].as_mut().ok_or("dispatch without admission")?;
        if job.id != id {
            return Err("stress dispatch identity mismatch".into());
        }
        if !job.dispatched {
            job.dispatched = true;
            self.segment_mut(i).dispatched += 1;
        }
        Ok(())
    }
    /// Terminal evidence comes only from the adapter's correlated reports.
    /// Observation time classifies throughput; request identity classifies cohorts.
    pub fn record_event(&mut self, event: Event, observed: Duration) -> Result<(), String> {
        let id = match &event {
            Event::Frame { job, .. }
            | Event::Obsolete(job)
            | Event::Withdrawn(job)
            | Event::Rejected(job)
            | Event::Failed(job) => *job,
        };
        if matches!(event, Event::Frame { .. }) && observed >= scheduler::END {
            return Err("current frame observed after stress cutoff".into());
        }
        let in_window = (Duration::from_secs(2)..scheduler::END).contains(&observed)
            && (self.pacing_elapsed.is_zero() || observed <= self.pacing_elapsed);
        let i = Self::slot(id)?;
        let job = self.jobs[i].ok_or("terminal without stress admission")?;
        if job.id != id || job.terminal {
            return Err("duplicate or uncorrelated stress terminal".into());
        }
        if matches!(event, Event::Withdrawn(_)) {
            if job.dispatched {
                return Err("dispatched job claimed locally withdrawn".into());
            }
        } else {
            self.dispatch(id)?;
        }
        self.jobs[i].as_mut().unwrap().terminal = true;
        let segment = self.segment_mut(i);
        match event {
            Event::Frame {
                ipc, end_to_end, ..
            } => {
                segment.valid += 1;
                if in_window {
                    self.window_valid += 1;
                }
                if scheduler::segment(i) == 1 {
                    self.ipc.push(ipc)?;
                    self.end_to_end.push(end_to_end)?;
                }
            }
            Event::Obsolete(_) => {
                segment.obsolete += 1;
                if observed >= scheduler::END {
                    segment.late_completed += 1;
                }
                if in_window {
                    self.window_obsolete += 1;
                }
            }
            Event::Withdrawn(_) => segment.withdrawn += 1,
            Event::Rejected(_) => {
                segment.rejected += 1;
                segment.failures += 1;
            }
            Event::Failed(_) => {
                segment.settled_failures += 1;
                segment.failures += 1;
            }
        }
        Ok(())
    }
    pub fn verify(&self) -> Result<(), String> {
        if self.warmup.scheduled != scheduler::WARMUP as u64
            || self.measured.scheduled != scheduler::MEASURED as u64
        {
            return Err("stress opportunity register changed".into());
        }
        for (group, s) in [&self.warmup, &self.measured].into_iter().enumerate() {
            let jobs = self
                .jobs
                .iter()
                .enumerate()
                .filter(|(i, _)| scheduler::segment(*i) == group)
                .filter_map(|(_, j)| j.as_ref());
            let admitted = jobs.clone().count() as u64;
            let dispatched = jobs.filter(|j| j.dispatched).count() as u64;
            if s.admitted != admitted || s.dispatched != dispatched {
                return Err("stress ledger counter mismatch".into());
            }
        }
        for s in [&self.warmup, &self.measured] {
            if s.scheduled != s.attempts + s.missed + s.skipped
                || s.attempts != s.admitted + s.busy + s.admission_failures
                || s.admitted
                    != s.valid
                        + s.obsolete
                        + s.withdrawn
                        + s.rejected
                        + s.settled_failures
                        + s.unresolved
                || s.dispatched > s.admitted - s.withdrawn
                || s.failures != s.rejected + s.settled_failures
            {
                return Err("stress accounting partition mismatch".into());
            }
            if s.admission_failures != 0 || s.failures != 0 || s.unresolved != 0 {
                return Err("stress supported request rejected, failed or unresolved".into());
            }
        }
        if self.jobs.iter().flatten().any(|j| !j.terminal) {
            return Err("stress admission lacks terminal evidence".into());
        }
        Ok(())
    }
}
fn sync_dispatch(adapter: &Adapter, result: &mut Stress) -> Result<(), String> {
    let known = result.warmup.dispatched + result.measured.dispatched;
    if adapter.counters().dispatched > known {
        let id = adapter
            .outstanding()
            .ok_or("missing dispatched stress identity")?;
        result.dispatch(id)?;
    }
    if adapter.counters().dispatched != result.warmup.dispatched + result.measured.dispatched {
        return Err("stress dispatch counter mismatch".into());
    }
    Ok(())
}
/// Real monotonic 12-second pacing; poll sleeps on socket/process/deadline.
pub fn stress(adapter: &mut Adapter) -> Result<Stress, String> {
    let mut schedule = Scheduler::default();
    let mut result = Stress::default();
    let mut start = adapter.now();
    let mut pacing_started = false;
    let body = (|| {
        adapter.wait_ready()?;
        start = adapter.now();
        pacing_started = true;
        let mut previous = None;
        while adapter.now() - start < scheduler::END {
            let relative = adapter.now() - start;
            if let Some(op) = schedule.poll(relative).map_err(String::from)? {
                let measured = scheduler::segment(op.index) == 1;
                let s = result.segment_mut(op.index);
                s.attempts += 1;
                if !op.lateness.is_zero() {
                    s.late += 1;
                }
                if measured {
                    result.lateness.push(op.lateness)?;
                    if let Some(previous) = previous {
                        result.cadence.push(relative - previous)?;
                    }
                    previous = Some(relative);
                }
                let request = oracle::request(op.index as u64 + 1, relative);
                match adapter.submit(request, adapter.now()) {
                    Ok(id) => result.admit(id)?,
                    Err(_) if adapter.outstanding().is_some() => {
                        result.segment_mut(op.index).busy += 1
                    }
                    Err(e) => {
                        result.segment_mut(op.index).admission_failures += 1;
                        return Err(e);
                    }
                }
            }
            let next = schedule.next_deadline().unwrap_or(scheduler::END);
            let wait = (start + next.min(scheduler::END)).saturating_sub(adapter.now());
            let event = adapter.poll_window(wait, start + scheduler::END)?;
            if let Some(event) = event {
                result.record_event(event, adapter.observed_at().saturating_sub(start))?;
            }
            sync_dispatch(adapter, &mut result)?;
            // Correlated rejection can leave the supervisor healthy. It still
            // fails this supported-fixture workload and stops further requests.
            if result.warmup.failures + result.measured.failures != 0 {
                return Err("stress supported request rejected or failed".into());
            }
            if adapter.outcome().failure.is_some() {
                return Err(format!("stress worker failure: {:?}", adapter.outcome()));
            }
        }
        schedule
            .poll((adapter.now() - start).max(scheduler::END))
            .map_err(String::from)?;
        Ok(())
    })();
    if let Err(error) = body {
        result.failure = Some(error);
        schedule.cancel();
    }
    result.warmup.missed = schedule.missed[0];
    result.measured.missed = schedule.missed[1];
    result.warmup.skipped = schedule.skipped[0];
    result.measured.skipped = schedule.skipped[1];
    result.observe_pacing_end(if pacing_started {
        adapter.now() - start
    } else {
        Duration::ZERO
    });
    if adapter.outstanding().is_some() {
        adapter.cancel();
    }
    let finish = adapter.finish_observations(|event, observed| {
        // Finish observations always follow the fixed window (or cancellation).
        if let Err(error) = result.record_event(event, observed.saturating_sub(start)) {
            result.failure.get_or_insert(error);
        }
    });
    match finish {
        Ok(outcome) => {
            result.shutdown = Some(outcome);
            if let Err(error) = controlled(outcome) {
                result.failure.get_or_insert(error);
            }
        }
        Err(error) => {
            result.failure.get_or_insert(error);
            result.shutdown = Some(adapter.outcome());
        }
    }
    if let Err(error) = sync_dispatch(adapter, &mut result) {
        result.failure.get_or_insert(error);
    }
    if let Some(id) = adapter.outstanding() {
        let i = Stress::slot(id)?;
        if result.jobs[i].is_none_or(|j| j.id != id || j.terminal) {
            return Err("unreconciled exact stress debt".into());
        }
        result.segment_mut(i).unresolved = 1;
    }
    if let Err(error) = result.verify() {
        result.failure.get_or_insert(error);
    }
    if adapter.counters().admitted != result.warmup.admitted + result.measured.admitted {
        result
            .failure
            .get_or_insert("stress admission counter mismatch".into());
    }
    Ok(result)
}

/// N3 proves a correlated initialization rejection, not arbitrary launch failure.
/// Failed shutdown classification is retained; reaping is only containment.
pub fn initialization_rejection(adapter: &Adapter, native: bool) -> Result<(), String> {
    use roomie_worker_supervisor::machine::Failure;
    let o = adapter.outcome();
    let diagnostic = adapter
        .diagnostic()
        .ok_or("missing initialization diagnostic")?;
    let expected = if native {
        diagnostic == "required Vulkan adapter unavailable"
            || diagnostic.starts_with("Vulkan adapter unavailable:")
    } else {
        diagnostic == "injected initialization failure"
    };
    let c = adapter.counters();
    if !expected
        || !o.initialize_transmitted
        || o.failure != Some(Failure::BackendFault)
        || o.job.is_some()
        || !o.reaped
        || o.unresolved.is_some()
        || c.attempts != 0
        || c.admitted != 0
        || c.dispatched != 0
    {
        return Err(format!("not correlated unavailable-adapter initialization rejection: {o:?}; diagnostic={diagnostic:?}"));
    }
    Ok(())
}

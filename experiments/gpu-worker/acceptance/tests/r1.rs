//! Deterministic evidence validation. Native-shaped reports below are fabricated
//! test inputs, never measurements or physical Vulkan execution.
use desktop_probe::presentation::gpu_handoff::JobIdentity;
use roomie_worker_acceptance::{
    adapter::Event,
    oracle,
    scheduler::{self, Scheduler},
    telemetry::Report,
    workload::{self, Stress},
};
use roomie_worker_supervisor::machine::{Machine, Policy};
use std::time::{Duration, Instant};

fn id(sequence: u64) -> JobIdentity {
    use desktop_probe::presentation::gpu_handoff::GpuHandoff;
    let mut handoff: GpuHandoff<Vec<u8>> = GpuHandoff::new(1);
    handoff.resume().unwrap();
    let now = Instant::now();
    handoff
        .schedule(
            oracle::request(sequence, Duration::ZERO),
            now,
            now + Duration::from_secs(1),
        )
        .unwrap()
        .identity()
}
fn frame(job: JobIdentity) -> Event {
    Event::Frame {
        request: oracle::request(job.frame.sequence, Duration::ZERO),
        job,
        ipc: Duration::from_millis(40),
        end_to_end: Duration::from_millis(42),
    }
}
fn clean() -> roomie_worker_supervisor::machine::Outcome {
    use roomie_worker_protocol::{Body, Message};
    let mut m = Machine::new(1, Policy::default(), Duration::ZERO);
    m.initialize_transmitted();
    m.receive(
        Message {
            session: 1,
            body: Body::Ready,
        },
        Duration::ZERO,
    );
    m.close(Duration::ZERO);
    m.receive(
        Message {
            session: 1,
            body: Body::Stopped,
        },
        Duration::ZERO,
    );
    m.exited(true, Duration::ZERO);
    m.outcome()
}
fn cohort() -> Stress {
    let mut s = Stress::default();
    // All opportunities retained; unused ones explicitly missed.
    s.warmup.missed = 119;
    s.warmup.attempts = 1;
    s.measured.missed = 598;
    s.measured.attempts = 2;
    s.admit(id(120)).unwrap();
    s.record_event(frame(id(120)), Duration::from_millis(2010))
        .unwrap();
    s.admit(id(121)).unwrap();
    s.record_event(Event::Obsolete(id(121)), Duration::from_millis(12001))
        .unwrap();
    s.admit(id(122)).unwrap();
    s.record_event(Event::Withdrawn(id(122)), Duration::from_millis(12002))
        .unwrap();
    s.observe_pacing_end(Duration::from_millis(12001));
    s.shutdown = Some(clean());
    s
}
fn report_text(frames: u128, native: bool) -> String {
    let kind = if native { "native-vulkan" } else { "synthetic" };
    let hex = kind
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut text = format!("roomie_acceptance_report=1\ncomplete=0\nkind_hex={hex}\nexit=Controlled\ntelemetry_available=1\n");
    for (key, n) in [
        ("initialization_complete", 1),
        ("initialization_ns", 0),
        ("device_initialization_ns", 0),
        ("pipeline_initialization_ns", 0),
        ("submission_attempts", frames),
        ("accepted_submissions", frames),
        ("confirmed_completions", frames),
        ("native_gpu_completions", if native { frames } else { 0 }),
        ("map_requests", frames),
        ("map_callbacks", frames),
        ("successful_maps", frames),
        ("unmaps", frames),
        ("staging_allocations", 1),
        ("staging_reuses", frames.saturating_sub(1)),
        ("staging_retirements", 0),
        ("peak_outstanding", u128::from(frames > 0)),
        ("timed_operations", frames),
        ("encode_ns", 0),
        ("submit_ns", 0),
        ("wait_ns", 0),
        ("map_ns", 0),
        ("repack_ns", 0),
        ("controlled_release", 1),
        ("uncertain_release", 0),
    ] {
        text.push_str(&format!("{key}={n}\n"));
    }
    for (key, value) in [
        ("adapter_hex", "test-only identity"),
        ("device_type_hex", "Cpu"),
        ("backend_hex", if native { "Vulkan" } else { "Synthetic" }),
        ("driver_hex", "test-only driver"),
    ] {
        let hex = value
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        text.push_str(&format!("{key}={hex}\n"));
    }
    text.push_str("telemetry_error=0\ncomplete=1\n");
    text
}
fn mutated(text: &str, key: &str, value: u128) -> Result<Report, String> {
    Report::parse(
        text.lines()
            .map(|line| {
                if line.starts_with(&format!("{key}=")) {
                    format!("{key}={value}\n")
                } else {
                    format!("{line}\n")
                }
            })
            .collect::<String>()
            .as_bytes(),
    )
}
#[test]
fn supported_rejection_and_settled_failure_fail_even_with_clean_supervisor_shutdown() {
    for event in [Event::Rejected(id(1)), Event::Failed(id(1))] {
        let mut s = Stress::default();
        s.warmup.attempts = 1;
        s.warmup.missed = 119;
        s.measured.missed = 600;
        s.admit(id(1)).unwrap();
        s.record_event(event, Duration::from_millis(1)).unwrap();
        s.shutdown = Some(clean());
        workload::controlled(s.shutdown.unwrap()).unwrap();
        assert!(s.verify().is_err());
        assert_eq!(s.warmup.dispatched, 1);
        assert_eq!(s.warmup.failures, 1);
    }
}
#[test]
fn healthy_slow_execution_obsolete_settlement_and_local_withdrawal_are_distinct() {
    let s = cohort();
    s.verify().unwrap();
    assert_eq!(
        (s.warmup.valid, s.measured.obsolete, s.measured.withdrawn),
        (1, 1, 1)
    );
    assert_eq!((s.warmup.dispatched, s.measured.dispatched), (1, 1));
    assert_eq!(s.measured.late_completed, 1);
    assert_eq!(s.measured.valid, 0);
    assert_eq!(s.window_valid, 1);
    assert_eq!(s.throughput(), 0.1); // warmup request, measured observation
    Report::parse(report_text(2, false).as_bytes())
        .unwrap()
        .stress(&s, false)
        .unwrap();
}
#[test]
fn terminal_reconciliation_rejects_missing_duplicate_wrong_and_overlapping_identities() {
    let mut s = Stress::default();
    s.admit(id(1)).unwrap();
    assert!(s.admit(id(2)).is_err());
    assert!(s.record_event(frame(id(2)), Duration::ZERO).is_err());
    assert!(s.verify().is_err());
    s.record_event(frame(id(1)), Duration::ZERO).unwrap();
    assert!(s.record_event(frame(id(1)), Duration::ZERO).is_err());
    s.admit(id(2)).unwrap();
    s.dispatch(id(2)).unwrap();
    assert!(s
        .record_event(Event::Withdrawn(id(2)), Duration::ZERO)
        .is_err());
}
#[test]
fn stress_reports_reconcile_dispatched_frames_not_opportunities_or_withdrawals() {
    let s = cohort();
    for native in [false, true] {
        Report::parse(report_text(2, native).as_bytes())
            .unwrap()
            .stress(&s, native)
            .unwrap();
        for count in [0, 1, 3, 600, 720] {
            assert!(Report::parse(report_text(count, native).as_bytes())
                .unwrap()
                .stress(&s, native)
                .is_err());
        }
        assert!(Report::parse(report_text(2, !native).as_bytes())
            .unwrap()
            .stress(&s, native)
            .is_err());
    }
}
#[test]
fn stress_reports_reject_fabricated_native_work_reuse_release_and_missing_metrics() {
    let s = cohort();
    let text = report_text(2, false);
    for (key, value) in [
        ("native_gpu_completions", 2),
        ("staging_allocations", 2),
        ("staging_reuses", 2),
        ("staging_retirements", 1),
        ("peak_outstanding", 0),
        ("controlled_release", 0),
        ("uncertain_release", 1),
        ("map_requests", 3),
        ("map_callbacks", 3),
        ("unmaps", 1),
        ("timed_operations", 3),
        ("initialization_complete", 0),
    ] {
        assert!(
            mutated(&text, key, value)
                .and_then(|r| r.stress(&s, false))
                .is_err(),
            "{key}"
        );
    }
    for text in [
        text.replace("telemetry_available=1", "telemetry_available=0"),
        text.replace("exit=Controlled", "exit=Uncertain"),
        text.replace("map_ns=0\n", ""),
    ] {
        assert!(Report::parse(text.as_bytes())
            .and_then(|r| r.stress(&s, false))
            .is_err());
    }
    let missing_identity = text
        .lines()
        .filter(|l| !l.starts_with("adapter_hex="))
        .map(|l| format!("{l}\n"))
        .collect::<String>();
    assert!(Report::parse(missing_identity.as_bytes())
        .unwrap()
        .stress(&s, false)
        .is_err());
    let native_text = report_text(2, true);
    assert!(Report::parse(
        native_text
            .replace("backend_hex=56756c6b616e", "backend_hex=4e6f6e65")
            .as_bytes()
    )
    .unwrap()
    .stress(&s, true)
    .is_err());
    let mut contradictory = cohort();
    contradictory.shutdown.as_mut().unwrap().reaped = false;
    assert!(Report::parse(report_text(2, false).as_bytes())
        .unwrap()
        .stress(&contradictory, false)
        .is_err());
}
#[test]
fn exact_and_cancellation_reports_require_all_mapping_callbacks_and_timed_operations() {
    for frames in [0, 1, 125] {
        let text = report_text(frames, false);
        Report::parse(text.as_bytes())
            .unwrap()
            .completed(frames, false)
            .unwrap();
        for (key, value) in [
            ("map_requests", frames + 1),
            ("map_callbacks", frames + 1),
            ("timed_operations", frames + 1),
        ] {
            assert!(
                mutated(&text, key, value)
                    .and_then(|r| r.completed(frames, false))
                    .is_err(),
                "{frames} {key}"
            );
        }
        if frames > 0 {
            assert!(mutated(&text, "map_callbacks", frames - 1)
                .and_then(|r| r.completed(frames, false))
                .is_err());
        }
    }
}
#[test]
fn measurement_window_excludes_boundaries_and_shutdown_but_retains_actual_overrun() {
    for (observed, expected) in [
        (Duration::from_secs(2) - Duration::from_nanos(1), 0),
        (Duration::from_secs(2), 1),
        (scheduler::END - Duration::from_nanos(1), 1),
        (scheduler::END, 0),
        (scheduler::END + Duration::from_nanos(1), 0),
    ] {
        let mut s = Stress::default();
        s.admit(id(121)).unwrap();
        if observed < scheduler::END {
            s.record_event(frame(id(121)), observed).unwrap();
        } else {
            s.record_event(Event::Obsolete(id(121)), observed).unwrap();
        }
        s.observe_pacing_end(scheduler::END + Duration::from_nanos(1));
        assert_eq!(s.window_valid, expected);
        assert_eq!(s.measured.valid, u64::from(observed < scheduler::END));
        assert_eq!(
            s.measured_elapsed,
            Duration::from_secs(10) + Duration::from_nanos(1)
        );
        assert_eq!(s.window_elapsed, Duration::from_secs(10));
        assert_eq!(s.pacing_elapsed, scheduler::END + Duration::from_nanos(1));
    }
    let mut partial = Stress::default();
    partial.admit(id(121)).unwrap();
    partial.observe_pacing_end(Duration::from_secs(3));
    partial
        .record_event(
            Event::Obsolete(id(121)),
            Duration::from_secs(3) + Duration::from_nanos(1),
        )
        .unwrap();
    assert_eq!(partial.measured.obsolete, 1);
    assert_eq!(partial.window_obsolete, 0);
    let mut s = Stress::default();
    s.observe_pacing_end(Duration::from_millis(2500));
    assert_eq!(s.measured_elapsed, Duration::from_millis(500));
}
#[test]
fn near_boundary_overrun_waits_full_cadence_and_never_queues_catchup() {
    for index in [0, 119, 120, 718] {
        let mut s = Scheduler::default();
        let observed = scheduler::deadline(index + 1) - Duration::from_nanos(1);
        let first = s.poll(observed).unwrap().unwrap();
        assert_eq!(first.index, index);
        assert!(s.poll(scheduler::deadline(index + 1)).unwrap().is_none());
        let earliest = observed + scheduler::deadline(index + 1) - scheduler::deadline(index);
        assert_eq!(s.next_deadline(), Some(earliest));
        assert!(s
            .poll(earliest - Duration::from_nanos(1))
            .unwrap()
            .is_none());
        let next = s.poll(earliest).unwrap().unwrap();
        assert_eq!(
            next.index,
            if earliest >= scheduler::deadline(index + 2) {
                index + 2
            } else {
                index + 1
            }
        );
        assert!(s.poll(earliest).unwrap().is_none());
        s.cancel();
        assert!(s.poll(scheduler::END).unwrap().is_none());
        assert_eq!(
            s.missed.iter().sum::<u64>() + s.skipped.iter().sum::<u64>() + 2,
            720
        );
    }
}

#[test]
fn initialization_reports_require_domain_zero_operations_and_retained_backend_fault() {
    use roomie_worker_protocol::{Body, Message};
    let mut m = Machine::new(1, Policy::default(), Duration::ZERO);
    m.initialize_transmitted();
    m.receive(
        Message {
            session: 1,
            body: Body::BackendFault("required Vulkan adapter unavailable".into()),
        },
        Duration::ZERO,
    );
    m.receive(
        Message {
            session: 1,
            body: Body::Stopped,
        },
        Duration::ZERO,
    );
    m.exited(true, Duration::ZERO);
    let outcome = m.outcome();
    for native in [true, false] {
        let text = report_text(0, native)
            .replace("initialization_complete=1", "initialization_complete=0")
            .replace("staging_allocations=1", "staging_allocations=0");
        Report::parse(text.as_bytes())
            .unwrap()
            .initialization_failure(native, outcome, Some(0))
            .unwrap();
        assert!(Report::parse(text.as_bytes())
            .unwrap()
            .initialization_failure(!native, outcome, Some(0))
            .is_err());
        assert!(Report::parse(text.as_bytes())
            .unwrap()
            .initialization_failure(native, clean(), Some(0))
            .is_err());
        assert!(Report::parse(text.as_bytes())
            .unwrap()
            .initialization_failure(native, outcome, Some(1))
            .is_err());
        for (key, value) in [
            ("initialization_complete", 1),
            ("submission_attempts", 1),
            ("native_gpu_completions", 1),
            ("map_requests", 1),
            ("timed_operations", 1),
            ("staging_allocations", 1),
            ("controlled_release", 0),
            ("uncertain_release", 1),
        ] {
            assert!(
                mutated(&text, key, value)
                    .and_then(|r| r.initialization_failure(native, outcome, Some(0)))
                    .is_err(),
                "{key}"
            );
        }
        assert!(Report::parse(
            text.replace("telemetry_available=1", "telemetry_available=0")
                .as_bytes()
        )
        .and_then(|r| r.initialization_failure(native, outcome, Some(0)))
        .is_err());
    }
}
#[test]
fn healthy_slow_controlled_time_stress_passes_correctness_with_busy_opportunities() {
    let mut result = Stress::default();
    let mut schedule = Scheduler::default();
    let mut pending: Option<(JobIdentity, Duration)> = None;
    while let Some(now) = schedule.next_deadline() {
        if now >= scheduler::END {
            break;
        }
        if pending.is_some_and(|(_, end)| now >= end) {
            let (job, end) = pending.take().unwrap();
            result.record_event(frame(job), end).unwrap();
        }
        if let Some(op) = schedule.poll(now).unwrap() {
            let s = if op.index < 120 {
                &mut result.warmup
            } else {
                &mut result.measured
            };
            s.attempts += 1;
            if pending.is_some() {
                s.busy += 1;
            } else {
                let job = id(op.index as u64 + 1);
                result.admit(job).unwrap();
                result.dispatch(job).unwrap();
                pending = Some((job, now + Duration::from_millis(42)));
            }
        }
    }
    if let Some((job, end)) = pending {
        result
            .record_event(Event::Obsolete(job), end.max(scheduler::END))
            .unwrap();
    }
    result.warmup.missed = schedule.missed[0];
    result.measured.missed = schedule.missed[1];
    result.observe_pacing_end(scheduler::END);
    result.shutdown = Some(clean());
    result.verify().unwrap();
    assert!(result.measured.busy > 0);
    assert!(result.throughput() < 60.0);
    let completed = result.warmup.valid
        + result.measured.valid
        + result.warmup.obsolete
        + result.measured.obsolete;
    Report::parse(report_text(completed as u128, false).as_bytes())
        .unwrap()
        .stress(&result, false)
        .unwrap();
}

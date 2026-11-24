//! GPU-free policy regressions. All boundary observations use supplied monotonic
//! offsets; Instant::now only establishes an origin and no sleeps determine time.
use desktop_probe::presentation::gpu_handoff::{
    CompletionStatus, GpuHandoff, JobCompletion, JobIdentity,
};
use roomie_worker_acceptance::{
    adapter::key,
    oracle,
    policy::{Workload, EXACT_FRESHNESS, LATENCY_REFERENCE},
    workload::Exact,
};
use roomie_worker_protocol::{Action, Body, Facing, Message, Render};
use roomie_worker_supervisor::machine::{Machine, Policy, Report};
use std::time::{Duration, Instant};

const SESSION: u64 = 73;
const START: Duration = Duration::from_secs(9);
struct Pair {
    handoff: GpuHandoff<Vec<u8>>,
    supervisor: Machine,
    job: JobIdentity,
    origin: Instant,
}
impl Pair {
    fn admitted(workload: Workload) -> Self {
        let policy = workload.policy();
        let origin = Instant::now();
        let request = oracle::request(1, Duration::ZERO);
        let mut handoff = GpuHandoff::new(SESSION);
        handoff.resume().unwrap();
        let job = handoff
            .schedule(request, origin + START, origin + START + policy.freshness)
            .unwrap()
            .identity();
        let mut supervisor = Machine::new(SESSION, policy, Duration::ZERO);
        supervisor.initialize_transmitted();
        supervisor.receive(
            Message {
                session: SESSION,
                body: Body::Ready,
            },
            Duration::ZERO,
        );
        supervisor
            .admit(
                Render {
                    job: key(job),
                    action: Action::Idle,
                    facing: Facing::Neutral,
                    elapsed: Duration::ZERO,
                    delta: Duration::ZERO,
                },
                START,
            )
            .unwrap();
        supervisor.render_transmitted(key(job), true);
        assert_eq!(supervisor.deadline(), Some(START + policy.freshness));
        assert_eq!(handoff.deadline(), Some(origin + START + policy.freshness));
        Self {
            handoff,
            supervisor,
            job,
            origin,
        }
    }
    fn frame_at(&mut self, elapsed: Duration) -> CompletionStatus {
        let at = START + elapsed;
        self.handoff.expire(self.origin + at);
        // Fabricated worker terminal input, not physical settlement evidence.
        let request = oracle::request(1, Duration::ZERO);
        let frame = oracle::expected(&request).unwrap();
        self.supervisor.receive(
            Message {
                session: SESSION,
                body: Body::Frame {
                    job: key(self.job),
                    bgra: frame.payload.clone(),
                },
            },
            at,
        );
        self.supervisor.tick(at);
        let completion = match self.supervisor.take_report().unwrap() {
            Report::Frame { job, bgra } => {
                assert_eq!(job, key(self.job));
                let mut observed = frame;
                observed.payload = bgra;
                oracle::validate(&request, &observed).unwrap();
                JobCompletion::Frame(observed)
            }
            Report::Obsolete(job) => {
                assert_eq!(job, key(self.job));
                JobCompletion::SettledWithoutPayload
            }
            report => panic!("unexpected {report:?}"),
        };
        self.handoff
            .complete(self.job, completion, self.origin + at)
            .unwrap()
    }
}

#[test]
fn exact_and_stress_select_fixed_budgets_and_all_other_workloads_keep_defaults() {
    for (name, kind, budget) in [
        ("exact", Workload::Exact, Duration::from_millis(1_000)),
        ("stress", Workload::Stress, Duration::from_millis(100)),
        ("cancel", Workload::Cancel, Duration::from_millis(100)),
        (
            "cancel-before",
            Workload::CancelBefore,
            Duration::from_millis(100),
        ),
        (
            "unsupported",
            Workload::Unsupported,
            Duration::from_millis(100),
        ),
        (
            "init-failure",
            Workload::InitFailure,
            Duration::from_millis(100),
        ),
    ] {
        assert_eq!(Workload::parse(name).unwrap(), kind);
        assert_eq!(kind.policy().freshness, budget);
    }
    assert!(Workload::parse("exact-2000").is_err());
}
#[test]
fn every_nonfreshness_policy_field_is_identical_to_accepted_defaults() {
    let defaults = Policy::default();
    for kind in [
        Workload::Exact,
        Workload::Stress,
        Workload::Cancel,
        Workload::CancelBefore,
        Workload::Unsupported,
        Workload::InitFailure,
    ] {
        // Exhaustive destructuring forces review if Policy gains a field.
        let Policy {
            initialization,
            freshness: _,
            escalation,
            graceful_shutdown,
            exit_observation,
        } = kind.policy();
        assert_eq!(initialization, defaults.initialization);
        assert_eq!(escalation, defaults.escalation);
        assert_eq!(graceful_shutdown, defaults.graceful_shutdown);
        assert_eq!(exit_observation, defaults.exit_observation);
    }
}
#[test]
fn correlated_valid_frame_at_128ms_is_exact_eligible_and_default_obsolete() {
    for (kind, expected) in [
        (Workload::Exact, CompletionStatus::Ready),
        (Workload::Stress, CompletionStatus::Obsolete),
    ] {
        let mut pair = Pair::admitted(kind);
        assert_eq!(pair.frame_at(Duration::from_millis(128)), expected);
        assert!(pair.handoff.outstanding().is_none());
        assert!(pair.supervisor.outcome().unresolved.is_none());
        assert!(pair.supervisor.outcome().failure.is_none());
        if kind == Workload::Exact {
            let (request, frame) = pair.handoff.take_ready().unwrap();
            oracle::validate(&request, &frame).unwrap();
        } else {
            assert!(pair.handoff.take_ready().is_none());
        }
    }
}
#[test]
fn exact_expiry_rejects_equality_and_later_frames_without_accepting_them() {
    for (elapsed, expected) in [
        (
            EXACT_FRESHNESS - Duration::from_nanos(1),
            CompletionStatus::Ready,
        ),
        (EXACT_FRESHNESS, CompletionStatus::Obsolete),
        (
            EXACT_FRESHNESS + Duration::from_nanos(1),
            CompletionStatus::Obsolete,
        ),
    ] {
        let mut pair = Pair::admitted(Workload::Exact);
        assert_eq!(pair.frame_at(elapsed), expected);
        assert_eq!(
            pair.handoff.take_ready().is_some(),
            elapsed < EXACT_FRESHNESS
        );
        assert!(pair.handoff.outstanding().is_none());
        assert!(pair.supervisor.outcome().unresolved.is_none());
    }
}
#[test]
fn explicit_cancel_under_exact_policy_cannot_be_undone_by_resume_or_early_frame() {
    let mut pair = Pair::admitted(Workload::Exact);
    pair.handoff.interrupt().unwrap();
    pair.supervisor.cancel();
    pair.handoff.resume().unwrap();
    assert_eq!(pair.handoff.outstanding(), Some(pair.job));
    assert_eq!(pair.supervisor.outcome().unresolved, Some(key(pair.job)));
    assert!(pair
        .handoff
        .schedule(
            oracle::request(2, Duration::ZERO),
            pair.origin + START,
            pair.origin + START + EXACT_FRESHNESS,
        )
        .is_err());
    assert_eq!(
        pair.frame_at(Duration::from_millis(128)),
        CompletionStatus::Obsolete
    );
    assert!(pair.handoff.take_ready().is_none());
}
#[test]
fn unresolved_exact_work_survives_expiry_escalation_and_reaping_until_matching_terminal() {
    let mut pair = Pair::admitted(Workload::Exact);
    let at = START + Duration::from_millis(128);
    pair.supervisor.receive(
        Message {
            session: SESSION,
            body: Body::Unresolved {
                job: key(pair.job),
                diagnostic: "simulated uncertain submission".into(),
            },
        },
        at,
    );
    pair.handoff.device_failed();
    assert!(matches!(
        pair.handoff.complete(
            pair.job,
            JobCompletion::Unresolved("settlement unproven".into()),
            pair.origin + at,
        ),
        Ok(CompletionStatus::Unresolved(_))
    ));
    for elapsed in [EXACT_FRESHNESS, Duration::from_millis(2_500)] {
        pair.handoff.expire(pair.origin + START + elapsed);
        pair.supervisor.tick(START + elapsed);
        assert_eq!(pair.handoff.outstanding(), Some(pair.job));
        assert_eq!(pair.supervisor.outcome().unresolved, Some(key(pair.job)));
        assert!(pair.supervisor.take_report().is_none());
    }
    pair.supervisor
        .exited(false, START + Duration::from_millis(2_600));
    assert!(pair.supervisor.outcome().reaped);
    assert_eq!(pair.supervisor.outcome().unresolved, Some(key(pair.job)));
    assert_eq!(pair.handoff.outstanding(), Some(pair.job));
    assert!(pair.handoff.take_ready().is_none());
    assert_eq!(
        pair.frame_at(Duration::from_millis(2_601)),
        CompletionStatus::Obsolete
    );
    assert!(pair.handoff.outstanding().is_none());
    assert!(pair.supervisor.outcome().unresolved.is_none());
    assert!(pair.handoff.take_ready().is_none());
}
#[test]
fn latency_reference_violations_include_warmup_without_implying_pixel_mismatch() {
    let mut evidence = Exact::default();
    // Every frame remains valid against the independent oracle. The timing
    // reference is a separate observation, including the equality boundary.
    for (i, latency) in [
        LATENCY_REFERENCE - Duration::from_nanos(1),
        LATENCY_REFERENCE,
        Duration::from_millis(128),
    ]
    .into_iter()
    .enumerate()
    {
        for warmup in [true, false] {
            let request = oracle::request(i as u64 + 1, Duration::ZERO);
            oracle::validate(&request, &oracle::expected(&request).unwrap()).unwrap();
            evidence.record_reference_latency(warmup, latency);
        }
    }
    assert_eq!(evidence.warmup_reference_violations, 2);
    assert_eq!(evidence.measured_reference_violations, 2);
    assert_eq!((evidence.warmup, evidence.measured), (0, 0));
    let summary = evidence.latency_reference_summary();
    assert!(summary.contains("latency_reference_ms=100"));
    assert!(summary.contains("warmup_violations=2 measured_violations=2 total_violations=4"));
    assert!(summary.contains("latency_violation_is_pixel_mismatch=false"));
    assert!(summary.contains("real_time_performance_acceptance=false"));
}

#[test]
fn exact_cli_reports_effective_policy_and_distinct_latency_cohorts_through_real_ipc() {
    let fixture = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("roomie-acceptance-fixture");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_roomie-worker-acceptance"))
        .args([
            "--run",
            "--worker",
            fixture.to_str().unwrap(),
            "--synthetic",
            "--workload",
            "exact",
            "--telemetry",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    for required in [
        "freshness_policy workload=exact budget_ms=1000 authorities=D1_and_supervisor",
        "purpose=bounded_exact_parity_verification_allowance",
        "exact warmup=25 measured=100 phases=[25, 25, 25, 25, 25]",
        "warmup_IPC_quantiles_50_95_99=[Some(",
        "warmup_end_to_end_quantiles_50_95_99=[Some(",
        "measured_IPC_quantiles_50_95_99=[Some(",
        "measured_end_to_end_quantiles_50_95_99=[Some(",
        "latency_reference_ms=100",
        "warmup_violations=",
        "measured_violations=",
        "latency_violation_is_pixel_mismatch=false real_time_performance_acceptance=false",
        "telemetry=complete domain=synthetic",
        "valid: 125, obsolete: 0",
    ] {
        assert!(text.contains(required), "missing {required}: {text}");
    }
}

use roomie_worker_acceptance::{
    adapter::{key, Adapter, Event},
    oracle,
    telemetry::{Report, RunDirectory},
    workload,
};
use roomie_worker_supervisor::{
    machine::{Failure, Policy, Shutdown, Transmission},
    Readiness,
};
use std::{path::PathBuf, time::Duration};

fn fixture() -> PathBuf {
    let path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("roomie-acceptance-fixture");
    assert!(
        path.is_file(),
        "build workspace --all-targets before acceptance tests: {}",
        path.display()
    );
    path
}
fn owner(mode: &str) -> Adapter {
    Adapter::spawn(&fixture(), &[mode], 73, Policy::default()).unwrap()
}
#[test]
fn every_warmup_and_measured_frame_crosses_d1_supervisor_worker_ipc_and_oracle() {
    let dir = RunDirectory::create().unwrap();
    let path = dir.report_path();
    let mut owner = Adapter::spawn(
        &fixture(),
        &["good", "--acceptance-report", path.to_str().unwrap()],
        73,
        Policy::default(),
    )
    .unwrap();
    let exact = workload::exact(&mut owner).unwrap();
    assert_eq!((exact.warmup, exact.measured), (25, 100));
    assert_eq!(exact.phases, [25; 5]);
    assert_eq!(owner.counters().valid, 125);
    assert_eq!(owner.counters().dispatched, 125);
    assert!(owner.outstanding().is_none());
    let report = Report::read(&path).unwrap();
    report.exact125(false).unwrap();
    assert!(report.exact125(true).is_err());
    assert_eq!(owner.finish().unwrap().shutdown, Shutdown::Confirmed);
}
#[test]
fn original_request_and_every_identity_field_survive_real_private_ipc() {
    let mut owner = owner("good");
    owner.wait_ready().unwrap();
    let request = oracle::request(913, Duration::from_millis(600));
    let id = owner.submit(request, owner.now()).unwrap();
    match owner.wait_event().unwrap() {
        Event::Frame {
            request: original,
            job,
            ..
        } => {
            assert_eq!(request, original);
            assert_eq!(job, id);
            assert_eq!(key(job), owner.outcome().job.unwrap().job);
        }
        e => panic!("{e:?}"),
    }
    workload::controlled(owner.finish().unwrap()).unwrap();
}
#[test]
fn unsupported_intent_is_rejected_through_the_actual_worker_before_submission() {
    use desktop_probe::presentation::{Action, ActionIntent, Facing, FrameRequest};
    let mut owner = owner("good");
    owner.wait_ready().unwrap();
    let original = oracle::request(1, Duration::ZERO);
    let request = FrameRequest::new(
        original.identity(),
        ActionIntent {
            action: Action::Walk,
            facing: Facing::BackRight,
        },
        original.extent(),
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    let id = owner.submit(request, owner.now()).unwrap();
    assert!(matches!(owner.wait_event().unwrap(),Event::Rejected(job) if job==id));
    assert!(owner.outstanding().is_none());
    assert_eq!(owner.counters().valid, 0);
    workload::controlled(owner.finish().unwrap()).unwrap();
}
#[test]
fn cancellation_before_first_render_byte_is_proven_local_rejection() {
    let mut owner = owner("good");
    owner.wait_ready().unwrap();
    let id = owner
        .submit(oracle::request(1, Duration::ZERO), owner.now())
        .unwrap();
    owner.shutdown(owner.now());
    assert_eq!(
        owner.outcome().job.unwrap().transmission,
        Transmission::Unsent
    );
    assert!(matches!(owner.wait_event().unwrap(),Event::Withdrawn(job) if job==id));
    workload::controlled(owner.finish().unwrap()).unwrap();
    assert!(owner.outstanding().is_none());
    assert_eq!(owner.counters().dispatched, 0);
}
#[test]
fn after_dispatch_cancel_resume_preserves_debt_until_genuine_obsolete_settlement() {
    let mut owner = owner("good");
    owner.wait_ready().unwrap();
    let id = owner
        .submit(oracle::request(1, Duration::ZERO), owner.now())
        .unwrap();
    owner
        .progress(
            Readiness {
                writable: true,
                ..Default::default()
            },
            owner.now(),
        )
        .unwrap();
    assert_eq!(
        owner.outcome().job.unwrap().transmission,
        Transmission::Complete
    );
    owner.cancel();
    owner.resume().unwrap();
    assert_eq!(owner.outstanding(), Some(id));
    assert!(owner
        .submit(oracle::request(2, Duration::ZERO), owner.now())
        .is_err());
    assert!(matches!(owner.wait_event().unwrap(),Event::Obsolete(job) if job==id));
    assert!(owner.outstanding().is_none());
    assert_eq!(owner.counters().valid, 0);
    owner
        .submit(oracle::request(2, Duration::from_millis(900)), owner.now())
        .unwrap();
    assert!(matches!(owner.wait_event().unwrap(), Event::Frame { .. }));
    workload::controlled(owner.finish().unwrap()).unwrap();
}
#[test]
fn owner_observed_deadline_boundary_makes_success_obsolete() {
    let policy = Policy {
        freshness: Duration::from_nanos(1),
        ..Default::default()
    };
    let mut owner = Adapter::spawn(&fixture(), &["good"], 73, policy).unwrap();
    owner.wait_ready().unwrap();
    let now = owner.now();
    let id = owner
        .submit(oracle::request(1, Duration::ZERO), now)
        .unwrap();
    owner
        .progress(
            Readiness {
                writable: true,
                ..Default::default()
            },
            now,
        )
        .unwrap();
    // Expiry is observed at equality regardless of when the subprocess writes.
    let event = owner
        .progress(Readiness::default(), now + policy.freshness)
        .unwrap()
        .unwrap_or_else(|| owner.wait_event().unwrap());
    assert!(matches!(event,Event::Obsolete(job) if job==id));
    workload::controlled(owner.finish().unwrap()).unwrap();
}
#[test]
fn truncation_abandons_terminal_and_retains_exact_dispatch_identity_after_reaping() {
    let mut owner = owner("partial-terminal-loss");
    owner.wait_ready().unwrap();
    let id = owner
        .submit(
            oracle::request(911, Duration::from_millis(600)),
            owner.now(),
        )
        .unwrap();
    assert!(owner.wait_event().is_err());
    let outcome = owner.finish().unwrap();
    assert!(outcome.reaped);
    assert_eq!(outcome.failure, Some(Failure::Protocol));
    assert_eq!(outcome.unresolved, Some(key(id)));
    assert_eq!(owner.outstanding(), Some(id));
    assert_eq!(owner.counters().valid, 0);
    assert_eq!(owner.counters().obsolete, 0);
    assert_eq!(outcome.shutdown, Shutdown::Unconfirmed);
    assert!(owner.resume().is_err());
    assert_eq!(owner.finish().unwrap().unresolved, Some(key(id)));
}
#[test]
fn native_error_and_teardown_simulations_never_manufacture_completion() {
    for mode in [
        "submit-reports-error",
        "unmap-reports-error",
        "deferred-destroyed-unmap",
        "map-wait",
        "panic-submit",
    ] {
        let mut owner = owner(mode);
        owner.wait_ready().unwrap();
        let id = owner
            .submit(oracle::request(81, Duration::ZERO), owner.now())
            .unwrap();
        assert!(owner.wait_event().is_err(), "{mode}");
        let outcome = owner.finish().unwrap();
        assert!(outcome.reaped, "{mode}");
        assert_eq!(outcome.unresolved, Some(key(id)), "{mode}");
        assert_eq!(owner.outstanding(), Some(id), "{mode}");
        assert_eq!(owner.counters().valid, 0);
        assert!(owner
            .submit(oracle::request(82, Duration::ZERO), owner.now())
            .is_err());
        assert_ne!(outcome.shutdown, Shutdown::Confirmed);
    }
}
#[test]
fn initialization_failure_has_no_admission_and_no_automatic_respawn() {
    let mut owner = owner("init-failure");
    assert!(owner.wait_ready().is_err());
    let outcome = owner.finish().unwrap();
    assert!(outcome.reaped);
    assert_eq!(owner.counters().admitted, 0);
    assert_eq!(owner.counters().dispatched, 0);
    assert!(owner.outstanding().is_none());
    assert!(owner.resume().is_err());
}
#[test]
fn shutdown_during_outstanding_work_never_accepts_a_current_frame() {
    let mut owner = owner("good");
    owner.wait_ready().unwrap();
    let id = owner
        .submit(oracle::request(1, Duration::ZERO), owner.now())
        .unwrap();
    owner
        .progress(
            Readiness {
                writable: true,
                ..Default::default()
            },
            owner.now(),
        )
        .unwrap();
    owner.shutdown(owner.now());
    assert!(matches!(owner.wait_event().unwrap(),Event::Obsolete(job) if job==id));
    workload::controlled(owner.finish().unwrap()).unwrap();
    assert_eq!(owner.counters().valid, 0);
}
#[test]
fn corrupted_ipc_pixels_fail_independent_validation_but_do_not_invent_unresolved_work() {
    let mut owner = owner("corrupt");
    assert!(workload::exact(&mut owner).is_err());
    assert_eq!(owner.counters().failed, 1);
    assert_eq!(owner.counters().valid, 0);
    assert!(owner.outstanding().is_none());
    assert!(owner.outcome().reaped);
}
#[test]
fn disabled_telemetry_is_the_default_and_exact_workload_still_validates_every_frame() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_roomie-worker-acceptance"))
        .args([
            "--run",
            "--worker",
            fixture().to_str().unwrap(),
            "--synthetic",
            "--workload",
            "exact",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("telemetry=disabled"));
    assert!(!text.contains("telemetry_path="));
    assert!(text.contains("zero native GPU work"));
}

#[test]
fn complete_stopped_and_telemetry_cannot_override_unsuccessful_child_exit() {
    let dir = RunDirectory::create().unwrap();
    let path = dir.report_path();
    let mut owner = Adapter::spawn(
        &fixture(),
        &[
            "stopped-exit-failure",
            "--acceptance-report",
            path.to_str().unwrap(),
        ],
        1,
        Policy::default(),
    )
    .unwrap();
    owner.wait_ready().unwrap();
    let outcome = owner.finish().unwrap();
    assert!(outcome.stopped && outcome.reaped);
    assert_eq!(owner.exit_code(), Some(1));
    assert_eq!(outcome.shutdown, Shutdown::Unconfirmed);
    assert_eq!(outcome.failure, Some(Failure::UnexpectedExit));
    let report = Report::read(&path).unwrap();
    assert_eq!(report.value("exit").unwrap(), "Controlled");
    assert_eq!(report.number("controlled_release").unwrap(), 1);
    assert!(workload::controlled(outcome).is_err());
}
#[test]
fn abnormal_worker_termination_leaves_incomplete_telemetry_and_exact_debt() {
    let dir = RunDirectory::create().unwrap();
    let path = dir.report_path();
    let mut owner = Adapter::spawn(
        &fixture(),
        &[
            "partial-terminal-loss",
            "--acceptance-report",
            path.to_str().unwrap(),
        ],
        1,
        Policy::default(),
    )
    .unwrap();
    owner.wait_ready().unwrap();
    let id = owner
        .submit(oracle::request(1, Duration::ZERO), owner.now())
        .unwrap();
    assert!(owner.wait_event().is_err());
    assert!(owner.finish().unwrap().reaped);
    assert_eq!(owner.outstanding(), Some(id));
    assert!(Report::read(&path).unwrap_err().contains("incomplete"));
}
#[test]
fn repeated_controlled_shutdown_sessions_and_repeated_finish_preserve_reaping() {
    for _ in 0..3 {
        let mut owner = owner("good");
        owner.wait_ready().unwrap();
        let first = owner.finish().unwrap();
        let second = owner.finish().unwrap();
        assert_eq!(first, second);
        workload::controlled(second).unwrap();
        assert!(owner.outstanding().is_none());
    }
}
#[test]
fn telemetry_creation_failure_preserves_ipc_correctness_and_unrelated_files() {
    let dir = RunDirectory::create().unwrap();
    let path = dir.report_path();
    std::fs::write(&path, "owner bytes").unwrap();
    let mut owner = Adapter::spawn(
        &fixture(),
        &["good", "--acceptance-report", path.to_str().unwrap()],
        1,
        Policy::default(),
    )
    .unwrap();
    workload::exact(&mut owner).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "owner bytes");
    assert!(Report::read(&path).is_err());
}
#[test]
fn stress_initialization_failure_reports_all_skipped_and_no_native_success() {
    let mut owner = owner("init-failure");
    let stress = workload::stress(&mut owner).unwrap();
    assert!(stress.failure.is_some());
    assert_eq!(stress.warmup.skipped, 120);
    assert_eq!(stress.measured.skipped, 600);
    assert_eq!(stress.measured.attempts, 0);
    assert_eq!(stress.measured.valid, 0);
    assert!(owner.outcome().reaped);
}

#[test]
fn stress_failure_after_dispatch_retains_exact_unresolved_job_and_skips_remaining_opportunities() {
    let mut owner = owner("submit-reports-error");
    let stress = workload::stress(&mut owner).unwrap();
    assert!(stress.failure.is_some());
    assert_eq!(stress.warmup.attempts, 1);
    assert_eq!(stress.warmup.admitted, 1);
    assert_eq!(stress.warmup.dispatched, 1);
    assert_eq!(stress.warmup.unresolved, 1);
    assert_eq!(stress.warmup.skipped, 119);
    assert_eq!(stress.measured.skipped, 600);
    assert_eq!(stress.warmup.valid, 0);
    assert_eq!(stress.measured.valid, 0);
    assert_eq!(owner.outstanding().map(key), owner.outcome().unresolved);
    assert!(owner.outcome().reaped);
}

#[test]
fn stalled_non_native_work_is_contained_reaped_and_never_silently_settled() {
    let policy = Policy {
        graceful_shutdown: Duration::from_millis(30),
        exit_observation: Duration::from_millis(100),
        ..Default::default()
    };
    let mut owner = Adapter::spawn(&fixture(), &["silent-after-render"], 1, policy).unwrap();
    owner.wait_ready().unwrap();
    let id = owner
        .submit(oracle::request(1, Duration::ZERO), owner.now())
        .unwrap();
    owner
        .progress(
            Readiness {
                writable: true,
                ..Default::default()
            },
            owner.now(),
        )
        .unwrap();
    assert_eq!(
        owner.outcome().job.unwrap().transmission,
        Transmission::Complete
    );
    let started = std::time::Instant::now();
    let outcome = owner.finish().unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(outcome.termination_requested && outcome.reaped);
    assert_eq!(outcome.unresolved, Some(key(id)));
    assert_eq!(owner.outstanding(), Some(id));
    assert_eq!(outcome.shutdown, Shutdown::Unconfirmed);
    assert_eq!(owner.counters().valid, 0);
    assert!(owner.resume().is_err());
    assert_eq!(owner.finish().unwrap().unresolved, Some(key(id)));
}

#[test]
fn result_observed_at_measurement_cutoff_is_obsolete_before_report_consumption() {
    let mut owner = owner("good");
    owner.wait_ready().unwrap();
    let id = owner
        .submit(oracle::request(1, Duration::ZERO), owner.now())
        .unwrap();
    owner
        .progress(
            Readiness {
                writable: true,
                ..Default::default()
            },
            owner.now(),
        )
        .unwrap();
    let now = owner.now();
    let event = owner
        .progress_window(Readiness::default(), now, now)
        .unwrap()
        .unwrap_or_else(|| owner.wait_event().unwrap());
    assert!(matches!(event,Event::Obsolete(job) if job==id));
    assert_eq!(owner.counters().valid, 0);
    workload::controlled(owner.finish().unwrap()).unwrap();
}

#[test]
fn cancellation_and_unsupported_cli_commands_verify_their_own_telemetry_counts() {
    for mode in ["cancel", "cancel-before", "unsupported"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_roomie-worker-acceptance"))
            .args([
                "--run",
                "--worker",
                fixture().to_str().unwrap(),
                "--synthetic",
                "--workload",
                mode,
                "--telemetry",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn correlated_supported_rejection_fails_stress_while_supervisor_shuts_down_cleanly() {
    let mut owner = owner("reject-supported");
    let stress = workload::stress(&mut owner).unwrap();
    assert!(stress.failure.as_ref().unwrap().contains("rejected"));
    assert_eq!(
        (
            stress.warmup.admitted,
            stress.warmup.dispatched,
            stress.warmup.rejected
        ),
        (1, 1, 1)
    );
    assert_eq!(stress.warmup.unresolved, 0);
    assert!(owner.outstanding().is_none());
    workload::controlled(owner.outcome()).unwrap();
}
#[test]
fn stress_cli_returns_failure_for_correlated_rejection_without_session_fault() {
    use std::os::unix::fs::PermissionsExt;
    let dir = RunDirectory::create().unwrap();
    let wrapper = dir.0.join("reject-peer");
    let exe = fixture().to_str().unwrap().replace('\'', "'\\''");
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\nexec '{exe}' reject-supported\n"),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_roomie-worker-acceptance"))
        .args([
            "--run",
            "--worker",
            wrapper.to_str().unwrap(),
            "--synthetic",
            "--workload",
            "stress",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("shutdown: Confirmed"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("supported request rejected"));
}
#[test]
fn initialization_rejection_requires_specific_correlated_evidence_and_complete_metrics() {
    for mode in [
        "init-rejection",
        "init-unrelated",
        "startup-exit",
        "startup-protocol",
        "startup-uncorrelated",
        "init-failure",
    ] {
        let dir = RunDirectory::create().unwrap();
        let path = dir.report_path();
        let mut owner = Adapter::spawn(
            &fixture(),
            &[mode, "--acceptance-report", path.to_str().unwrap()],
            1,
            Policy::default(),
        )
        .unwrap();
        assert!(owner.wait_ready().is_err());
        owner.finish().unwrap();
        assert!(owner.outcome().reaped);
        assert_eq!(owner.counters().admitted, 0);
        let proof = workload::initialization_rejection(&owner, false).and_then(|_| {
            Report::read(&path)?.initialization_failure(false, owner.outcome(), owner.exit_code())
        });
        assert_eq!(proof.is_ok(), mode == "init-rejection", "{mode}: {proof:?}");
        // Synthetic rejection must never be relabeled as actual N3.
        assert!(workload::initialization_rejection(&owner, true).is_err());
        if mode == "init-rejection" {
            assert_eq!(owner.outcome().shutdown, Shutdown::Confirmed);
        }
    }
}
#[test]
fn initialization_failure_cli_requires_telemetry_and_retains_observed_shutdown() {
    for telemetry in [false, true] {
        let mut command =
            std::process::Command::new(env!("CARGO_BIN_EXE_roomie-worker-acceptance"));
        command.args([
            "--run",
            "--worker",
            fixture().to_str().unwrap(),
            "--synthetic",
            "--workload",
            "init-failure",
        ]);
        if telemetry {
            command.arg("--telemetry");
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.success(),
            telemetry,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if telemetry {
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text.contains("shutdown: Confirmed"));
            assert!(text.contains("owner_initialization_duration=Some"));
        }
    }
}

#[test]
fn terminal_observation_timestamp_is_the_authority_sample_before_oracle_processing() {
    let mut owner = owner("good");
    owner.wait_ready().unwrap();
    let at = owner.now();
    let id = owner
        .submit(oracle::request(121, Duration::ZERO), at)
        .unwrap();
    owner
        .progress(
            Readiness {
                writable: true,
                ..Default::default()
            },
            at,
        )
        .unwrap();
    let event = owner.wait_event().unwrap();
    assert!(matches!(event,Event::Frame{job,..} if job==id));
    let observed = owner.observed_at();
    assert!(observed >= at);
    assert!(observed <= owner.now());
    // Later processing does not move this report across a measurement boundary.
    let mut stress = workload::Stress::default();
    stress.admit(id).unwrap();
    stress
        .record_event(event, Duration::from_secs(12) - Duration::from_nanos(1))
        .unwrap();
    assert_eq!(stress.window_valid, 1);
    workload::controlled(owner.finish().unwrap()).unwrap();
}

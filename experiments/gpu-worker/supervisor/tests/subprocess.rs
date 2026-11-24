#![cfg(target_os = "linux")]
use roomie_worker_protocol::{Action, Facing, JobKey, Render, FRAME_BYTES};
use roomie_worker_supervisor::{
    machine::{Admission, Failure, Phase, Policy, Report, Shutdown},
    Supervisor,
};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
const WORKER: &str = env!("CARGO_BIN_EXE_fake-worker");
fn render() -> Render {
    Render {
        job: JobKey {
            epoch: 3,
            serial: 1,
            generation: 4,
            sequence: 5,
        },
        action: Action::Idle,
        facing: Facing::Neutral,
        elapsed: Duration::from_millis(300),
        delta: Duration::ZERO,
    }
}
fn policy() -> Policy {
    Policy {
        initialization: Duration::from_secs(2),
        freshness: Duration::from_secs(1),
        escalation: Duration::from_secs(2),
        graceful_shutdown: Duration::from_millis(100),
        exit_observation: Duration::from_millis(200),
    }
}
struct Harness {
    supervisor: Supervisor,
    clock: Instant,
}
impl Harness {
    fn new(mode: &str, policy: Policy) -> Self {
        let clock = Instant::now();
        let supervisor =
            Supervisor::spawn(Path::new(WORKER), &[mode], 91, policy, clock.elapsed()).unwrap();
        Self { supervisor, clock }
    }
    fn step(&mut self) {
        let ready = self
            .supervisor
            .poll_ready(Duration::from_millis(25), self.clock.elapsed())
            .unwrap();
        self.supervisor.progress(ready, self.clock.elapsed());
    }
    fn until(&mut self, predicate: impl Fn(&Supervisor) -> bool) {
        while !predicate(&self.supervisor) {
            assert!(
                self.clock.elapsed() < Duration::from_secs(6),
                "native timeout: {:?}",
                self.supervisor.outcome()
            );
            self.step();
        }
    }
    fn ready(&mut self) {
        self.until(|s| s.outcome().phase == Phase::Ready);
    }
    fn submit(&mut self) {
        self.supervisor
            .submit(render(), self.clock.elapsed())
            .unwrap();
    }
    fn report(&mut self) -> Report {
        loop {
            self.step();
            if let Some(report) = self.supervisor.take_report() {
                return report;
            }
            assert!(
                self.clock.elapsed() < Duration::from_secs(6),
                "report timeout: {:?}",
                self.supervisor.outcome()
            );
        }
    }
    fn shutdown(&mut self) {
        self.supervisor.shutdown(self.clock.elapsed());
        self.until(|s| s.outcome().reaped);
    }
}
#[test]
fn real_socket_normal_and_fragmented_frame_then_graceful_reap() {
    for mode in ["normal", "fragment"] {
        let mut h = Harness::new(mode, policy());
        h.ready();
        h.submit();
        assert_eq!(
            h.supervisor.submit(render(), h.clock.elapsed()),
            Err(Admission::Outstanding)
        );
        let Report::Frame { job, bgra } = h.report() else {
            panic!("expected synthetic frame");
        };
        assert_eq!(job, render().job);
        assert_eq!(bgra.len(), FRAME_BYTES);
        assert!(bgra.as_chunks::<4>().0.iter().all(|p| *p == [1, 0, 0, 255]));
        h.shutdown();
        assert_eq!(h.supervisor.outcome().shutdown, Shutdown::Confirmed);
        assert_eq!(h.supervisor.outcome().failure, None);
        assert_eq!(h.supervisor.interest().socket, None);
        assert_eq!(h.supervisor.interest().process, None);
    }
}
#[test]
fn real_rejection_and_settled_failure_can_release_the_slot() {
    for mode in ["reject", "fail"] {
        let mut h = Harness::new(mode, policy());
        h.ready();
        h.submit();
        assert!(matches!(
            h.report(),
            Report::Rejected { .. } | Report::Failed { .. }
        ));
        assert_eq!(h.supervisor.outcome().unresolved, None);
        let mut next = render();
        next.job.serial += 1;
        h.supervisor.submit(next, h.clock.elapsed()).unwrap();
        h.report();
        h.shutdown();
        assert_eq!(h.supervisor.outcome().shutdown, Shutdown::Confirmed);
    }
}
#[test]
fn real_unresolved_report_closes_admission_and_preserves_final_debt() {
    let mut h = Harness::new("unresolved", policy());
    h.ready();
    h.submit();
    h.until(|s| s.outcome().failure == Some(Failure::Unresolved));
    assert_eq!(
        h.supervisor.submit(render(), h.clock.elapsed()),
        Err(Admission::Outstanding)
    );
    h.until(|s| s.outcome().reaped);
    assert_eq!(h.supervisor.outcome().unresolved, Some(render().job));
    assert_eq!(h.supervisor.outcome().shutdown, Shutdown::Unconfirmed);
    assert!(h.supervisor.outcome().termination_requested);
}
#[test]
fn actual_startup_timeout_and_silent_job_escalation_kill_and_reap() {
    let short = Policy {
        initialization: Duration::from_millis(50),
        freshness: Duration::from_millis(20),
        escalation: Duration::from_millis(80),
        ..policy()
    };
    for mode in ["stall-init", "partial-init"] {
        let mut h = Harness::new(mode, short);
        h.until(|s| s.outcome().reaped);
        assert_eq!(
            h.supervisor.outcome().failure,
            Some(Failure::InitializationTimeout)
        );
        assert!(h.supervisor.outcome().termination_requested);
        assert_eq!(
            h.supervisor.exit_status().unwrap().signal(),
            Some(libc::SIGKILL)
        );
    }
    let mut h = Harness::new(
        "stall-job",
        Policy {
            initialization: Duration::from_secs(2),
            ..short
        },
    );
    h.ready();
    h.submit();
    h.until(|s| s.outcome().reaped);
    assert_eq!(
        h.supervisor.exit_status().unwrap().signal(),
        Some(libc::SIGKILL)
    );
    assert_eq!(h.supervisor.outcome().unresolved, Some(render().job));
    assert!(h.supervisor.outcome().termination_requested);
    assert_eq!(h.supervisor.outcome().shutdown, Shutdown::Unconfirmed);
}
#[test]
fn actual_freshness_timeout_allows_late_physical_settlement() {
    let mut h = Harness::new(
        "late",
        Policy {
            freshness: Duration::from_millis(20),
            ..policy()
        },
    );
    h.ready();
    h.submit();
    assert_eq!(h.report(), Report::Obsolete(render().job));
    assert_eq!(h.supervisor.outcome().unresolved, None);
    assert_eq!(h.supervisor.outcome().failure, None);
    h.shutdown();
    assert_eq!(h.supervisor.outcome().shutdown, Shutdown::Confirmed);
}
#[test]
fn native_wrong_session_and_job_fail_closed_without_settlement() {
    for mode in ["wrong-session", "wrong-job"] {
        let mut h = Harness::new(mode, policy());
        h.ready();
        h.submit();
        h.until(|s| s.outcome().reaped);
        assert_eq!(h.supervisor.outcome().failure, Some(Failure::Correlation));
        assert_eq!(h.supervisor.outcome().unresolved, Some(render().job));
    }
}
#[test]
fn actual_unexpected_exit_and_truncated_hangup_do_not_settle() {
    for mode in ["exit-job", "truncated"] {
        let mut h = Harness::new(mode, policy());
        h.ready();
        h.submit();
        h.until(|s| s.outcome().reaped);
        assert_eq!(h.supervisor.outcome().unresolved, Some(render().job));
        assert_eq!(h.supervisor.outcome().shutdown, Shutdown::Unconfirmed);
        assert_eq!(
            h.supervisor.outcome().failure,
            Some(if mode == "truncated" {
                Failure::Protocol
            } else {
                Failure::Disconnected
            })
        );
    }
}
#[test]
fn stopped_is_read_before_hangup_and_exit_classification() {
    let mut h = Harness::new("normal", policy());
    h.ready();
    h.shutdown();
    assert!(h.supervisor.outcome().stopped);
    assert!(h.supervisor.outcome().reaped);
    assert_eq!(h.supervisor.outcome().failure, None);
}
#[test]
fn absent_stopped_and_stopped_without_exit_are_unconfirmed() {
    for mode in ["no-stop", "stop-hang"] {
        let mut h = Harness::new(mode, policy());
        h.ready();
        h.shutdown();
        assert_eq!(h.supervisor.outcome().shutdown, Shutdown::Unconfirmed);
        assert_eq!(h.supervisor.outcome().stopped, mode == "stop-hang");
        assert_eq!(
            h.supervisor.outcome().termination_requested,
            mode == "stop-hang"
        );
    }
}
#[test]
fn idle_wait_has_no_write_interest_or_recurring_deadline() {
    let mut h = Harness::new("normal", policy());
    h.ready();
    assert_eq!(h.supervisor.interest().deadline, None);
    assert!(!h.supervisor.interest().writable);
    let before = Instant::now();
    let ready = h
        .supervisor
        .poll_ready(Duration::from_millis(40), h.clock.elapsed())
        .unwrap();
    assert!(before.elapsed() >= Duration::from_millis(30));
    assert!(!ready.readable && !ready.writable && !ready.exited && !ready.hangup && !ready.error);
    h.shutdown();
}
#[test]
fn executable_arguments_are_literal_and_unrelated_non_cloexec_fds_are_removed() {
    let (sentinel, _peer) = UnixStream::pair().unwrap();
    // SAFETY: duplicate the live sentinel to a deliberately inheritable high fd.
    let raw = unsafe { libc::fcntl(sentinel.as_raw_fd(), libc::F_DUPFD, 200) };
    assert!(raw >= 200);
    let inherited = unsafe { OwnedFd::from_raw_fd(raw) };
    let value = raw.to_string();
    let clock = Instant::now();
    let supervisor = Supervisor::spawn(
        Path::new(WORKER),
        &["fd-check", &value, "$(exit 33); | literal"],
        91,
        policy(),
        clock.elapsed(),
    )
    .unwrap();
    let mut h = Harness { supervisor, clock };
    h.ready();
    h.shutdown();
    assert_eq!(h.supervisor.outcome().failure, None);
    drop(inherited);
}
#[test]
fn ordinary_parent_exit_and_forced_parent_kill_deliver_pdeathsig() {
    // Become a subreaper only in this disposable test process, so this test can
    // reap each orphan directly rather than relying on the machine's PID 1.
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
    for mode in ["exit", "hold"] {
        let mut parent = Command::new(env!("CARGO_BIN_EXE_parent-fixture"))
            .args([WORKER, mode])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdout = parent.stdout.take().unwrap();
        let flags = unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        let handshake_deadline = Instant::now() + Duration::from_secs(5);
        let mut line = Vec::new();
        while !line.contains(&b'\n') {
            let mut bytes = [0; 32];
            match stdout.read(&mut bytes) {
                Ok(0) => panic!("parent exited before handshake"),
                Ok(count) => {
                    line.extend_from_slice(&bytes[..count]);
                    assert!(line.len() <= 32);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("parent handshake: {error}"),
            }
            assert!(
                Instant::now() < handshake_deadline,
                "parent handshake timeout"
            );
        }
        let worker: libc::pid_t = std::str::from_utf8(&line).unwrap().trim().parse().unwrap();
        if mode == "hold" {
            parent.kill().unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while parent.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut status = 0;
        loop {
            let result = unsafe { libc::waitpid(worker, &mut status, libc::WNOHANG) };
            if result == worker {
                break;
            }
            assert!(
                result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR),
                "waitpid failed: {}",
                std::io::Error::last_os_error()
            );
            assert!(Instant::now() < deadline, "worker outlived parent");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(libc::WIFSIGNALED(status));
        assert_eq!(libc::WTERMSIG(status), libc::SIGKILL);
    }
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 0) }, 0);
}
#[test]
fn coalesced_backend_fault_invalidates_frame_in_either_wire_order() {
    for mode in ["frame-fault", "fault-frame"] {
        let mut h = Harness::new(mode, policy());
        h.ready();
        h.submit();
        // A fault can precede the remainder of its separate terminal Frame.
        // Observe physical settlement independently, then honor the drain barrier.
        h.until(|s| {
            s.outcome().failure == Some(Failure::BackendFault) && s.outcome().unresolved.is_none()
        });
        assert_eq!(h.report(), Report::Obsolete(render().job));
        assert_eq!(h.supervisor.outcome().unresolved, None);
        h.until(|s| s.outcome().reaped);
        assert_eq!(h.supervisor.outcome().failure, Some(Failure::BackendFault));
    }
}
#[test]
fn error_readiness_fails_closed_and_does_not_settle_a_queued_job() {
    let mut h = Harness::new("stall-job", policy());
    h.ready();
    h.submit();
    h.supervisor.progress(
        roomie_worker_supervisor::Readiness {
            writable: true,
            ..Default::default()
        },
        h.clock.elapsed(),
    );
    // Inject readiness failure after full dispatch; this process event cannot
    // settle the job. Entirely unsent work now has a separate proven rejection.
    h.supervisor.progress(
        roomie_worker_supervisor::Readiness {
            error: true,
            ..Default::default()
        },
        h.clock.elapsed(),
    );
    assert_eq!(h.supervisor.outcome().failure, Some(Failure::Transport));
    assert_eq!(h.supervisor.outcome().unresolved, Some(render().job));
    h.until(|s| s.outcome().reaped);
}
#[test]
fn finite_read_budget_withholds_report_until_fault_batch_is_drained() {
    let mut h = Harness::new("flood-faults", policy());
    h.ready();
    h.submit();
    let mut budget_observed = false;
    loop {
        assert!(h.clock.elapsed() < Duration::from_secs(5));
        let ready = h
            .supervisor
            .poll_ready(Duration::from_millis(25), h.clock.elapsed())
            .unwrap();
        let progress = h.supervisor.progress(ready, h.clock.elapsed());
        if progress.continue_immediately {
            budget_observed = true;
            assert!(!progress.drained);
            assert!(h.supervisor.take_report().is_none());
        }
        if progress.drained && h.supervisor.outcome().failure == Some(Failure::BackendFault) {
            break;
        }
    }
    assert!(budget_observed);
    assert_eq!(
        h.supervisor.take_report(),
        Some(Report::Obsolete(render().job))
    );
    h.until(|s| s.outcome().reaped);
}
// Wait only for readable IPC, deliberately excluding writable readiness so the
// parent's Initialize remains entirely unsent. No sleep establishes ordering.
fn wait_for_readable(supervisor: &Supervisor, clock: Instant) {
    let mut fd = libc::pollfd {
        fd: supervisor.interest().socket.unwrap(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        assert!(clock.elapsed() < Duration::from_secs(5));
        let result = unsafe { libc::poll(&mut fd, 1, 25) };
        if result > 0 {
            assert_ne!(fd.revents & libc::POLLIN, 0);
            return;
        }
        if result < 0 {
            assert_eq!(
                io::Error::last_os_error().kind(),
                io::ErrorKind::Interrupted
            );
        }
    }
}
#[test]
fn premature_ready_through_actual_ipc_cannot_initialize_an_unsent_session() {
    let mut h = Harness::new("premature-ready", policy());
    wait_for_readable(&h.supervisor, h.clock);
    h.supervisor.progress(
        roomie_worker_supervisor::Readiness {
            readable: true,
            ..Default::default()
        },
        h.clock.elapsed(),
    );
    assert!(!h.supervisor.outcome().initialize_transmitted);
    assert_eq!(h.supervisor.outcome().phase, Phase::Closing);
    assert_eq!(h.supervisor.outcome().failure, Some(Failure::Protocol));
    assert!(h.supervisor.submit(render(), h.clock.elapsed()).is_err());
    h.until(|s| s.outcome().reaped);
}
#[test]
fn backend_fault_through_actual_ipc_has_authority_before_initialize_dispatch() {
    let mut h = Harness::new("pre-init-fault", policy());
    wait_for_readable(&h.supervisor, h.clock);
    h.supervisor.progress(
        roomie_worker_supervisor::Readiness {
            readable: true,
            ..Default::default()
        },
        h.clock.elapsed(),
    );
    assert!(!h.supervisor.outcome().initialize_transmitted);
    assert_eq!(h.supervisor.outcome().failure, Some(Failure::BackendFault));
    h.until(|s| s.outcome().reaped);
    assert_eq!(h.supervisor.outcome().failure, Some(Failure::BackendFault));
}
#[test]
fn duplicate_completion_through_actual_ipc_closes_after_only_one_settlement() {
    let mut h = Harness::new("duplicate-frame", policy());
    h.ready();
    h.submit();
    h.until(|s| s.outcome().failure == Some(Failure::Correlation));
    h.until(|s| s.outcome().reaped);
    assert_eq!(h.supervisor.outcome().unresolved, None);
    assert_eq!(
        h.supervisor.outcome().job.unwrap().state,
        roomie_worker_supervisor::machine::JobState::PhysicallySettled
    );
    assert_eq!(
        h.supervisor.take_report(),
        Some(Report::Obsolete(render().job))
    );
    assert!(h.supervisor.take_report().is_none());
}
#[test]
fn malformed_header_through_actual_ipc_is_terminal() {
    let mut h = Harness::new("malformed-header", policy());
    h.until(|s| s.outcome().reaped);
    assert_eq!(h.supervisor.outcome().failure, Some(Failure::Protocol));
    assert_eq!(h.supervisor.outcome().unresolved, None);
    assert!(h.supervisor.take_report().is_none());
}
#[test]
fn oversized_body_declaration_through_actual_ipc_is_terminal() {
    let mut h = Harness::new("oversized-header", policy());
    h.until(|s| s.outcome().reaped);
    assert_eq!(h.supervisor.outcome().failure, Some(Failure::Protocol));
    assert_eq!(h.supervisor.outcome().unresolved, None);
    assert!(h.supervisor.take_report().is_none());
}
#[test]
fn graceful_close_of_unsent_render_never_delivers_render_to_the_worker() {
    let mut h = Harness::new("no-render-expected", policy());
    h.ready();
    h.submit();
    h.supervisor.shutdown(h.clock.elapsed());
    assert_eq!(h.supervisor.outcome().unresolved, None);
    assert!(h.supervisor.interest().writable); // Shutdown is queued without a timer wake.
    assert_eq!(
        h.supervisor.outcome().job.unwrap().state,
        roomie_worker_supervisor::machine::JobState::RejectedBeforeDispatch
    );
    assert_eq!(
        h.report(),
        Report::RejectedBeforeDispatch { job: render().job }
    );
    h.until(|s| s.outcome().reaped);
    assert_eq!(h.supervisor.outcome().failure, None);
    assert_eq!(h.supervisor.outcome().shutdown, Shutdown::Confirmed);
}
#[test]
fn escalation_before_first_render_write_rejects_locally_and_reaps_without_dispatch() {
    let mut h = Harness::new("no-render-expected", policy());
    h.ready();
    h.submit();
    // Injectable time makes escalation simultaneous with writable readiness.
    // Maintain this origin consistently during subsequent real exit observation.
    let now = || Duration::from_secs(3) + h.clock.elapsed();
    h.supervisor.progress(
        roomie_worker_supervisor::Readiness {
            writable: true,
            ..Default::default()
        },
        now(),
    );
    assert!(h.supervisor.outcome().termination_requested);
    assert_eq!(h.supervisor.outcome().unresolved, None);
    assert!(!h.supervisor.interest().writable);
    assert_eq!(
        h.supervisor.take_report(),
        Some(Report::RejectedBeforeDispatch { job: render().job })
    );
    while !h.supervisor.outcome().reaped {
        assert!(h.clock.elapsed() < Duration::from_secs(5));
        let ready = h
            .supervisor
            .poll_ready(Duration::from_millis(25), now())
            .unwrap();
        h.supervisor.progress(ready, now());
    }
    assert_eq!(
        h.supervisor.exit_status().unwrap().signal(),
        Some(libc::SIGKILL)
    );
    assert_eq!(h.supervisor.outcome().failure, Some(Failure::Escalation));
}

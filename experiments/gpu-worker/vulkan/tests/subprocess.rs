//! Actual private Unix IPC through the production worker driver, with injected
//! GPU-free operations. No test opens a Vulkan device or X11 connection.
use roomie_worker_protocol::{
    encode, read_step, Action, Body, Decoder, Facing, JobKey, Message, Render, FRAME_BYTES,
};
use roomie_worker_supervisor::{
    machine::{Phase, Policy, Report, Shutdown},
    Supervisor,
};
use std::io::Write;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
const FIXTURE: &str = env!("CARGO_BIN_EXE_vulkan-worker-fixture");
const NATIVE: &str = env!("CARGO_BIN_EXE_roomie-worker-vulkan");
#[test]
fn accepted_supervisor_keeps_exact_dispatched_job_unresolved_after_invalid_terminal_prefix() {
    use roomie_worker_supervisor::machine::{Failure, JobState, Transmission};
    let trace = std::env::temp_dir().join(format!("roomie-r2-terminal-{}.txt", std::process::id()));
    let start = Instant::now();
    let policy = Policy {
        initialization: Duration::from_secs(3),
        freshness: Duration::from_secs(3),
        escalation: Duration::from_secs(4),
        graceful_shutdown: Duration::from_secs(2),
        exit_observation: Duration::from_secs(2),
    };
    let mut s = Supervisor::spawn(
        Path::new(FIXTURE),
        &["partial-terminal-loss", trace.to_str().unwrap()],
        29,
        policy,
        start.elapsed(),
    )
    .unwrap();
    while s.outcome().phase != Phase::Ready {
        assert!(start.elapsed() < Duration::from_secs(5));
        let r = s
            .poll_ready(Duration::from_millis(20), start.elapsed())
            .unwrap();
        s.progress(r, start.elapsed());
    }
    let job = render(1).job;
    s.submit(render(1), start.elapsed()).unwrap();
    while !s.outcome().reaped {
        assert!(start.elapsed() < Duration::from_secs(5));
        let r = s
            .poll_ready(Duration::from_millis(20), start.elapsed())
            .unwrap();
        s.progress(r, start.elapsed());
        assert_eq!(
            s.take_report(),
            None,
            "no ready frame or settling report may escape"
        );
    }
    let outcome = s.outcome();
    assert_eq!(s.exit_status().unwrap().code(), Some(2));
    assert_eq!(outcome.failure, Some(Failure::Protocol)); // actual truncated header at EOF
    assert_eq!(outcome.unresolved, Some(job));
    let status = outcome.job.unwrap();
    assert_eq!(status.job, job);
    assert_eq!(status.transmission, Transmission::Complete);
    assert_eq!(status.state, JobState::Unresolved);
    assert!(!outcome.stopped);
    assert_ne!(outcome.shutdown, Shutdown::Confirmed);
    assert_eq!(std::fs::read_to_string(&trace).unwrap(), "Render completely decoded\nterminal prefix written\nsettlement invalidated\ntransport abandoned\nprivate stream closed by serve\nuncertain exit 2\n");
    std::fs::remove_file(trace).unwrap();
}
fn render(serial: u64) -> Render {
    Render {
        job: JobKey {
            epoch: 17,
            serial,
            generation: 31,
            sequence: 37,
        },
        action: Action::Idle,
        facing: Facing::Neutral,
        elapsed: Duration::from_millis(1200),
        delta: Duration::ZERO,
    }
}
struct ChildSocket {
    child: Child,
    socket: UnixStream,
    decoder: Decoder,
}
impl ChildSocket {
    fn spawn(exe: &str, args: &[&str]) -> Self {
        let (socket, peer) = UnixStream::pair().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let fd: OwnedFd = peer.into();
        let child = Command::new(exe)
            .args(args)
            .env_clear()
            .stdin(Stdio::from(fd))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            child,
            socket,
            decoder: Decoder::default(),
        }
    }
    fn send(&mut self, session: u64, body: Body) {
        self.socket
            .write_all(&encode(&Message { session, body }).unwrap())
            .unwrap();
    }
    fn receive(&mut self) -> Message {
        for _ in 0..64 {
            if let Some(message) = read_step(&mut self.decoder, &mut self.socket).unwrap() {
                return message;
            }
        }
        panic!("response not bounded");
    }
    fn body(&mut self) -> Body {
        let m = self.receive();
        assert_eq!(m.session, 29);
        m.body
    }
    fn exit(&mut self, expected: i32) {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(expected));
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for ChildSocket {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn actual_worker_loop_reuses_resources_rejects_intent_and_stops_after_release() {
    let mut h = ChildSocket::spawn(FIXTURE, &["good"]);
    // Deliberately fragment Initialize to exercise the real bounded decoder.
    for byte in encode(&Message {
        session: 29,
        body: Body::Initialize,
    })
    .unwrap()
    {
        h.socket.write_all(&[byte]).unwrap();
    }
    assert_eq!(h.body(), Body::Ready);
    let mut unsupported = render(1);
    unsupported.action = Action::Walk;
    h.send(29, Body::Render(unsupported));
    assert!(matches!(h.body(),Body::RejectedBeforeSubmission { job,.. } if job == unsupported.job));
    // Coalesced commands remain in the socket until all frame bytes are sent.
    let bytes: Vec<u8> = [
        Body::Render(render(2)),
        Body::Render(render(3)),
        Body::Shutdown,
    ]
    .into_iter()
    .flat_map(|body| encode(&Message { session: 29, body }).unwrap())
    .collect();
    h.socket.write_all(&bytes).unwrap();
    for serial in [2, 3] {
        assert_eq!(
            h.body(),
            Body::Frame {
                job: render(serial).job,
                bgra: vec![4; FRAME_BYTES]
            }
        );
    }
    assert_eq!(h.body(), Body::Stopped);
    h.exit(0);
}
#[test]
fn actual_settled_failure_reports_fault_first_then_stops_without_another_job() {
    let mut h = ChildSocket::spawn(FIXTURE, &["map-failed"]);
    h.send(29, Body::Initialize);
    assert_eq!(h.body(), Body::Ready);
    h.send(29, Body::Render(render(1)));
    assert!(matches!(h.body(), Body::BackendFault(_)));
    assert!(matches!(h.body(),Body::JobFailed { job,.. } if job == render(1).job));
    h.send(29, Body::Shutdown);
    assert_eq!(h.body(), Body::Stopped);
    h.exit(0);
}
#[test]
fn actual_uncertain_mapping_or_submission_reports_fault_then_unresolved_without_stopped() {
    for mode in ["submit-failure", "missing", "map-wait", "stale"] {
        let mut h = ChildSocket::spawn(FIXTURE, &[mode]);
        h.send(29, Body::Initialize);
        assert_eq!(h.body(), Body::Ready);
        h.send(29, Body::Render(render(1)));
        assert!(matches!(h.body(), Body::BackendFault(_)));
        assert!(matches!(h.body(),Body::Unresolved { job,.. } if job == render(1).job));
        h.exit(2);
        assert!(read_step(&mut h.decoder, &mut h.socket).is_err());
    }
}
#[test]
fn actual_initialization_failure_has_no_ready_and_can_release_empty_backend() {
    let mut h = ChildSocket::spawn(FIXTURE, &["init-failure"]);
    h.send(29, Body::Initialize);
    assert!(matches!(h.body(), Body::BackendFault(_)));
    h.send(29, Body::Shutdown);
    assert_eq!(h.body(), Body::Stopped);
    h.exit(0);
}

#[test]
fn native_normal_return_error_contract_preserves_fault_unresolved_order_through_private_ipc() {
    for mode in [
        "submit-reports-error",
        "unmap-reports-error",
        "failed-map-cleanup-error",
        "deferred-destroyed-unmap",
    ] {
        let mut h = ChildSocket::spawn(FIXTURE, &[mode]);
        h.send(29, Body::Initialize);
        assert_eq!(h.body(), Body::Ready);
        h.send(29, Body::Render(render(1)));
        let Body::BackendFault(first) = h.body() else {
            panic!("fault must precede terminal report");
        };
        assert!(
            matches!(h.body(),Body::Unresolved {job,diagnostic} if job==render(1).job && diagnostic==first)
        );
        h.exit(2);
        assert!(read_step(&mut h.decoder, &mut h.socket).is_err());
    }
}

#[test]
fn teardown_fault_over_private_ipc_never_announces_stopped() {
    let mut h = ChildSocket::spawn(FIXTURE, &["unknown-drop"]);
    h.send(29, Body::Initialize);
    assert_eq!(h.body(), Body::Ready);
    h.send(29, Body::Shutdown);
    assert!(matches!(h.body(), Body::BackendFault(_)));
    h.exit(2);
    assert!(read_step(&mut h.decoder, &mut h.socket).is_err());
}

#[test]
fn supervisor_retains_backend_fault_after_settled_failure_stopped_and_exit() {
    use roomie_worker_supervisor::machine::Failure;
    let start = Instant::now();
    let policy = Policy {
        initialization: Duration::from_secs(2),
        freshness: Duration::from_secs(2),
        escalation: Duration::from_secs(3),
        graceful_shutdown: Duration::from_secs(2),
        exit_observation: Duration::from_secs(1),
    };
    let mut s = Supervisor::spawn(
        Path::new(FIXTURE),
        &["short-view"],
        29,
        policy,
        start.elapsed(),
    )
    .unwrap();
    while s.outcome().phase != Phase::Ready {
        assert!(start.elapsed() < Duration::from_secs(5));
        let r = s
            .poll_ready(Duration::from_millis(20), start.elapsed())
            .unwrap();
        s.progress(r, start.elapsed());
    }
    s.submit(render(1), start.elapsed()).unwrap();
    while !s.outcome().reaped {
        assert!(start.elapsed() < Duration::from_secs(5));
        let r = s
            .poll_ready(Duration::from_millis(20), start.elapsed())
            .unwrap();
        s.progress(r, start.elapsed());
    }
    assert_eq!(s.outcome().failure, Some(Failure::BackendFault));
    assert_eq!(s.outcome().unresolved, None);
    assert!(s.outcome().stopped && s.exit_status().unwrap().success());
}

#[test]
fn idle_worker_reports_asynchronous_backend_fault_without_another_command() {
    let mut h = ChildSocket::spawn(FIXTURE, &["idle-fault"]);
    h.send(29, Body::Initialize);
    assert_eq!(h.body(), Body::Ready);
    assert!(matches!(h.body(), Body::BackendFault(_)));
    // This can have been fully dispatched before the owner observed the fault.
    // Reject it explicitly, without re-opening backend GPU admission.
    h.send(29, Body::Render(render(1)));
    assert!(matches!(h.body(),Body::RejectedBeforeSubmission { job,.. } if job == render(1).job));
    h.send(29, Body::Shutdown);
    assert_eq!(h.body(), Body::Stopped);
    h.exit(0);
}

#[test]
fn unexpected_panic_exits_without_any_settlement_or_stopped_assertion() {
    let mut h = ChildSocket::spawn(FIXTURE, &["panic-submit"]);
    h.send(29, Body::Initialize);
    assert_eq!(h.body(), Body::Ready);
    h.send(29, Body::Render(render(1)));
    h.exit(101);
    assert!(read_step(&mut h.decoder, &mut h.socket).is_err());
}
#[test]
fn actual_wrong_session_duplicate_serial_and_duplicate_initialize_fail_closed() {
    for kind in 0..3 {
        let mut h = ChildSocket::spawn(FIXTURE, &["good"]);
        h.send(29, Body::Initialize);
        assert_eq!(h.body(), Body::Ready);
        if kind == 0 {
            h.send(30, Body::Render(render(1)));
        }
        if kind == 1 {
            h.send(29, Body::Render(render(1)));
            assert!(matches!(h.body(), Body::Frame { .. }));
            h.send(29, Body::Render(render(1)));
        }
        if kind == 2 {
            h.send(29, Body::Initialize);
        }
        h.exit(1);
        assert!(read_step(&mut h.decoder, &mut h.socket).is_err());
    }
}
#[test]
fn real_binary_rejects_unavailable_driver_before_native_initialization_in_both_features() {
    // A bad launch configuration cannot reach wgpu, even in native builds.
    let mut h = ChildSocket::spawn(NATIVE, &["--driver", "/unavailable/d2b/driver.json"]);
    h.send(29, Body::Initialize);
    assert!(matches!(h.body(), Body::BackendFault(_)));
    h.send(29, Body::Shutdown);
    assert_eq!(h.body(), Body::Stopped);
    h.exit(0);
}
#[test]
fn accepted_supervisor_drives_new_worker_with_gpu_free_operations() {
    let start = Instant::now();
    let policy = Policy {
        initialization: Duration::from_secs(3),
        freshness: Duration::from_secs(2),
        escalation: Duration::from_secs(3),
        graceful_shutdown: Duration::from_secs(2),
        exit_observation: Duration::from_secs(1),
    };
    let mut supervisor =
        Supervisor::spawn(Path::new(FIXTURE), &["good"], 29, policy, start.elapsed()).unwrap();
    while supervisor.outcome().phase != Phase::Ready {
        assert!(start.elapsed() < Duration::from_secs(5));
        let r = supervisor
            .poll_ready(Duration::from_millis(25), start.elapsed())
            .unwrap();
        supervisor.progress(r, start.elapsed());
    }
    supervisor.submit(render(1), start.elapsed()).unwrap();
    loop {
        assert!(start.elapsed() < Duration::from_secs(5));
        let r = supervisor
            .poll_ready(Duration::from_millis(25), start.elapsed())
            .unwrap();
        supervisor.progress(r, start.elapsed());
        if let Some(report) = supervisor.take_report() {
            assert!(
                matches!(report, Report::Frame { job,bgra } if job == render(1).job && bgra == vec![4;FRAME_BYTES])
            );
            break;
        }
    }
    supervisor.shutdown(start.elapsed());
    while !supervisor.outcome().reaped {
        assert!(start.elapsed() < Duration::from_secs(5));
        let r = supervisor
            .poll_ready(Duration::from_millis(25), start.elapsed())
            .unwrap();
        supervisor.progress(r, start.elapsed());
    }
    assert_eq!(supervisor.outcome().shutdown, Shutdown::Confirmed);
    assert_eq!(supervisor.outcome().failure, None);
}

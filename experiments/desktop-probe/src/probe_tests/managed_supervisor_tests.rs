use super::*;
use crate::managed_supervisor::{Fixture, ManagedSupervisor};
use roomie_worker_supervisor::machine::{Failure, JobState, Phase, Policy, Shutdown, Transmission};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture_path() -> PathBuf {
    std::env::var_os("DESKTOPROOMIE_MANAGED_FIXTURE")
        .expect("DESKTOPROOMIE_MANAGED_FIXTURE must name the independently built fake-worker")
        .into()
}

struct Gate {
    path: PathBuf,
    listener: UnixListener,
}

impl Gate {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "roomie-i1-{}-{}.sock",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        Self { path, listener }
    }

    fn accept(&self) -> UnixStream {
        let limit = Instant::now() + Duration::from_secs(5);
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    return stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < limit, "fixture failed to join rendezvous");
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("fixture rendezvous failed: {error}"),
            }
        }
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).unwrap();
    }
}

struct Host {
    runtime: ProbeRuntime,
    window: ManagedProbeWindow,
    presentation: BodyPresentation<SyntheticProducer>,
    clock: Cell<Instant>,
    origin: Instant,
    buffered: Option<u32>,
    transport: CheckedTransport,
}

impl Host {
    fn with_worker(args: &[&str]) -> Self {
        Self::with_policy(args, Duration::from_millis(200))
    }

    fn with_policy(args: &[&str], freshness: Duration) -> Self {
        let origin = Instant::now();
        let (mut runtime, window, presentation, _) = session(origin);
        runtime.managed = Some(
            ManagedSupervisor::spawn(
                Fixture {
                    executable: &fixture_path(),
                    args,
                    session: 71,
                    policy: Policy {
                        freshness,
                        initialization: Duration::from_secs(3),
                        ..Policy::default()
                    },
                },
                origin,
            )
            .unwrap(),
        );
        let mut host = Self {
            runtime,
            window,
            presentation,
            clock: Cell::new(origin),
            origin,
            buffered: None,
            transport: CheckedTransport::default(),
        };
        host.until(|managed| managed.outcome().phase == Phase::Ready);
        host.step();
        host
    }

    fn scripted() -> (Self, UnixStream) {
        let gate = Gate::new();
        let host = Self::with_worker(&["managed-script", gate.path.to_str().unwrap()]);
        let stream = gate.accept();
        (host, stream)
    }

    fn managed(&self) -> &ManagedSupervisor {
        self.runtime.managed.as_ref().unwrap()
    }

    fn at(&self, millis: u64) -> Instant {
        self.origin + Duration::from_millis(millis)
    }

    fn set(&self, millis: u64) {
        self.clock.set(self.at(millis));
    }

    fn step(&mut self) {
        self.runtime.progress_managed(|| self.clock.get()).unwrap();
    }

    fn until(&mut self, done: impl Fn(&ManagedSupervisor) -> bool) {
        let limit = Instant::now() + Duration::from_secs(5);
        while !done(self.managed()) {
            assert!(
                Instant::now() < limit,
                "fixture timed out: {:?}",
                self.managed().outcome()
            );
            let (interest, _) = self.managed().interests().unwrap();
            let wake = (Instant::now() + Duration::from_millis(20)).min(limit);
            crate::x11::events::poll_managed(None, &interest, Some(wake)).unwrap();
            self.step();
        }
    }

    fn admit(&mut self, millis: u64) {
        self.set(millis);
        assert!(!self.turn(None, || Ok(None), || {}).unwrap());
        assert!(self.managed().outstanding().is_some());
        assert_eq!(self.runtime.frame_timing.deadline(), None);
        self.step();
    }

    fn turn(
        &mut self,
        deadline: Option<Instant>,
        peek: impl FnMut() -> Result<Option<u32>, HostError>,
        validate: impl FnOnce(),
    ) -> Result<bool, HostError> {
        let transport = &self.transport;
        let body = self.window.window;
        let clock = &self.clock;
        self.runtime.managed_presentation_turn(
            &mut self.window,
            &mut self.presentation,
            deadline,
            || false,
            || None,
            || clock.get(),
            &mut self.buffered,
            peek,
            |frames, request, frame, input, pointer, admit| {
                let mut coverage = AnimatedCoverage {
                    input,
                    pointer,
                    maximum_request_bytes: REQUEST_LIMIT,
                    set_input: |rectangles: Vec<Rectangle>| {
                        let step = if transport.trace.borrow().contains(&Step::EmptyShape) {
                            Step::FinalShape
                        } else {
                            Step::EmptyShape
                        };
                        transport.set_shape(body, step, rectangles)
                    },
                    release: || transport.check(Step::Release),
                };
                let mut cutover = DuringValidation {
                    inner: RecordBlock(&mut coverage, &transport.trace),
                    validate: RefCell::new(Some(validate)),
                };
                frames.publish_with_admission(
                    request,
                    frame,
                    |dimensions, pixels| {
                        assert_eq!(dimensions, (160, 160));
                        transport.check(Step::Upload)?;
                        *transport.image.borrow_mut() = pixels.to_vec();
                        Ok(())
                    },
                    |_| {
                        checked_blit_with(
                            || transport.check(Step::Blit),
                            || transport.check(Step::Flush),
                        )
                    },
                    Some(&mut cutover),
                    admit,
                )
            },
            |runtime, window| {
                runtime.fence_visibility_input_with(
                    window,
                    || transport.set_shape(body, Step::SafetyEmpty, vec![]),
                    |_, _| transport.check(Step::SafetyRelease),
                    |_| unreachable!("no popup in checked fixture"),
                )
            },
        )
    }

    fn confirm_no_mutation(&self) {
        assert!(self.transport.trace.borrow().is_empty());
        assert_eq!(self.runtime.frames.last_committed(), Some(INITIAL));
        assert!(self.runtime.body_input_ready());
    }

    fn shutdown(&mut self) -> Shutdown {
        let managed = self.runtime.managed.as_mut().unwrap();
        managed.begin_shutdown(self.clock.get()).unwrap();
        let real = Instant::now();
        let observed = self.clock.get();
        managed
            .reap_after_host_cleanup_with(|| observed + real.elapsed())
            .unwrap();
        managed.outcome().shutdown
    }
}

struct DuringValidation<C, V> {
    inner: C,
    validate: RefCell<Option<V>>,
}

impl<C: CoverageCutover, V: FnOnce()> CoverageCutover for DuringValidation<C, V> {
    fn maximum_request_bytes(&self) -> usize {
        (self.validate.borrow_mut().take().unwrap())();
        self.inner.maximum_request_bytes()
    }
    fn block_input(&mut self) {
        self.inner.block_input();
    }
    fn empty_and_release(&mut self) -> Result<(), HostError> {
        self.inner.empty_and_release()
    }
    fn enable(&mut self, rectangles: Vec<Rectangle>) -> Result<u64, HostError> {
        self.inner.enable(rectangles)
    }
    fn input_ready_for_admission(&self) -> bool {
        self.inner.input_ready_for_admission()
    }
}

fn gate_read(gate: &mut UnixStream, expected: u8) {
    let mut byte = [0];
    gate.read_exact(&mut byte).unwrap();
    assert_eq!(byte, [expected]);
}

fn command(gate: &mut UnixStream, value: u8) {
    gate.write_all(&[value]).unwrap();
    gate_read(gate, b'W');
}

#[test]
fn frames_and_faults_follow_real_supervisor_drainage_and_receive_budget() {
    for script in *b"APG" {
        let (mut host, mut gate) = Host::scripted();
        host.admit(300);
        gate_read(&mut gate, b'R');
        command(&mut gate, script);
        host.step();
        let outcome = host.managed().outcome();
        eprintln!(
            "wire script {}: {:?}, drained={}, immediate={}",
            char::from(script),
            outcome,
            host.managed().drained(),
            host.managed().immediate(),
        );
        assert_eq!(outcome.job.unwrap().state, JobState::PhysicallySettled);
        assert!(host.managed().ready().is_none());
        match script {
            b'P' => {
                assert_eq!(outcome.failure, None);
                assert!(!host.managed().drained());
                assert!(!host.managed().immediate());
                let (interest, deadline) = host.managed().interests().unwrap();
                assert!(interest.socket.is_some());
                assert!(!interest.writable);
                assert_eq!(deadline, Some(host.at(500)));
                command(&mut gate, b'C');
                host.step();
            }
            b'G' => {
                assert!(host.managed().immediate());
                assert!(!host.managed().drained());
                let host_turns = Cell::new(0);
                let mut buffered = None;
                assert!(!host
                    .turn(None, || Ok(None), || panic!("not drained"))
                    .unwrap());
                assert!(host.managed().immediate());
                let events = host
                    .runtime
                    .progress_before_event_batch(
                        || host.clock.get(),
                        &mut buffered,
                        || {
                            host_turns.set(host_turns.get() + 1);
                            Ok(Some(host_turns.get()))
                        },
                    )
                    .unwrap();
                assert_eq!(events.len(), 64);
                assert!(
                    host.managed().immediate(),
                    "host batch ran while IPC still needed another turn"
                );
                host.until(|managed| !managed.immediate());
            }
            _ => {}
        }
        assert_eq!(
            host.managed().outcome().failure,
            Some(Failure::BackendFault)
        );
        assert!(host.managed().outstanding().is_none());
        assert!(host.managed().unresolved().is_none());
        assert!(!host
            .turn(None, || Ok(None), || panic!("fault cannot validate"))
            .unwrap());
        host.confirm_no_mutation();
        host.shutdown();
    }
}

#[test]
fn final_observation_blocks_fault_but_a_fault_after_commit_keeps_history() {
    for before in [true, false] {
        let (mut host, mut gate) = Host::scripted();
        host.admit(300);
        gate_read(&mut gate, b'R');
        command(&mut gate, b'L');
        host.step();
        let ready = host.managed().ready().unwrap();
        let expected = SyntheticProducer::new()
            .prepare_frame(&ready.request)
            .unwrap();
        assert_eq!(ready.frame.payload, expected.payload);
        assert_eq!(ready.frame.metadata, expected.metadata);
        assert_eq!(ready.deadline, host.at(500));
        let commit = host
            .turn(
                None,
                || Ok(None),
                || {
                    if before {
                        command(&mut gate, b'C');
                    }
                },
            )
            .unwrap();
        assert_eq!(commit, !before);
        eprintln!("fault released during validation={before}, committed={commit}");
        if before {
            host.confirm_no_mutation();
        } else {
            assert_eq!(*host.transport.trace.borrow(), IMAGE_ONLY);
            assert_eq!(*host.transport.image.borrow(), expected.payload);
            let historical = host.runtime.frames.last_committed();
            assert_ne!(historical, Some(INITIAL));
            command(&mut gate, b'C');
            host.step();
            assert_eq!(host.runtime.frames.last_committed(), historical);
            assert!(host.runtime.body_input_ready());
        }
        assert_eq!(
            host.managed().outcome().failure,
            Some(Failure::BackendFault)
        );
        assert!(host.managed().outstanding().is_none());
        assert!(host.managed().ready().is_none());
        assert_eq!(host.runtime.frame_timing.deadline(), None);
        host.shutdown();
    }
}

#[test]
fn ready_owner_turn_defers_for_lookahead_and_due_host_deadline() {
    let (mut host, mut gate) = Host::scripted();
    host.admit(300);
    gate_read(&mut gate, b'R');
    command(&mut gate, b'F');
    host.step();
    assert!(host.managed().ready().is_some());
    assert!(!host
        .turn(Some(host.at(300)), || Ok(None), || panic!("host wins tie"))
        .unwrap());
    assert!(host.managed().ready().is_some());
    assert!(!host
        .turn(None, || Ok(Some(33)), || panic!("buffered event wins"))
        .unwrap());
    assert_eq!(host.buffered.take(), Some(33));
    assert!(host.managed().ready().is_some());
    let observations = Cell::new(0);
    assert!(!host
        .turn(
            None,
            || {
                let count = observations.get() + 1;
                observations.set(count);
                Ok((count == 2).then_some(34))
            },
            || {}
        )
        .unwrap());
    assert_eq!(host.buffered.take(), Some(34));
    assert!(host.managed().ready().is_none());
    host.confirm_no_mutation();
    assert_eq!(host.runtime.frame_timing.deadline(), Some(host.at(600)));
    assert_eq!(host.shutdown(), Shutdown::Confirmed);
}

#[test]
fn deadline_equality_before_consumption_and_during_validation_retires_opportunities() {
    for during_validation in [false, true] {
        let (mut host, mut gate) = Host::scripted();
        host.admit(300);
        gate_read(&mut gate, b'R');
        host.set(499);
        command(&mut gate, b'F');
        host.step();
        assert_eq!(host.managed().ready().unwrap().deadline, host.at(500));
        let crossing = if during_validation { 500 } else { 499 };
        let reads = Cell::new(0);
        let end = host.at(500);
        let clock = Cell::new(host.at(crossing));
        if !during_validation {
            host.set(500);
        }
        // The final X lookahead occurs after transport validation. Move the
        // injected clock immediately before the final freshness observation.
        let owner_clock = &host.clock;
        if during_validation {
            owner_clock.set(host.at(499));
        }
        let mut turn = |clock: &Cell<Instant>| {
            let transport = &host.transport;
            let body = host.window.window;
            host.runtime.managed_presentation_turn(
                &mut host.window,
                &mut host.presentation,
                None,
                || false,
                || None,
                || clock.get(),
                &mut host.buffered,
                || Ok(None),
                |frames, req, frame, input, pointer, admit| {
                    reads.set(reads.get() + 1);
                    let mut cutover = DuringValidation {
                        inner: AnimatedCoverage {
                            input,
                            pointer,
                            maximum_request_bytes: REQUEST_LIMIT,
                            set_input: |rects| transport.set_shape(body, Step::FinalShape, rects),
                            release: || transport.check(Step::Release),
                        },
                        validate: RefCell::new(Some(|| clock.set(end))),
                    };
                    frames.publish_with_admission(
                        req,
                        frame,
                        |_, _| panic!("expired upload"),
                        |_| panic!("expired blit"),
                        Some(&mut cutover),
                        admit,
                    )
                },
                |_, _| panic!("expiry has no native fencing"),
            )
        };
        if !during_validation {
            clock.set(end);
        } else {
            clock.set(end - Duration::from_millis(1));
        }
        assert!(!turn(&clock).unwrap());
        assert_eq!(reads.get(), usize::from(during_validation));
        assert!(host.managed().ready().is_none());
        assert_eq!(host.runtime.frame_timing.deadline(), Some(host.at(800)));
        host.confirm_no_mutation();
        host.set(500);
        assert_eq!(host.shutdown(), Shutdown::Confirmed);
    }
}

#[test]
fn interruption_between_events_obsoletes_old_job_even_after_resume() {
    let (mut host, mut gate) = Host::scripted();
    host.admit(300);
    gate_read(&mut gate, b'R');
    let old = host.managed().outstanding().unwrap();
    // Control admission and its terminal service can fall in one handler while
    // the next eligibility observation sees an already-idle control endpoint.
    host.runtime
        .interrupt_frame_eligibility(host.clock.get())
        .unwrap();
    host.runtime
        .refresh_frame_eligibility(host.clock.get(), &host.window, false)
        .unwrap();
    assert!(host.runtime.frame_eligible(&host.window, false));
    assert_eq!(host.managed().outstanding(), Some(old));
    command(&mut gate, b'F');
    host.step();
    assert_eq!(
        host.managed().outcome().job.unwrap().state,
        JobState::PhysicallySettled
    );
    assert!(host.managed().ready().is_none());
    assert!(host.managed().outstanding().is_none());
    host.confirm_no_mutation();
    host.admit(600);
    gate_read(&mut gate, b'R');
    let next = host.managed().outstanding().unwrap();
    assert_ne!(next.epoch, old.epoch);
    assert_ne!(next.serial, old.serial);
    assert_eq!(next.frame, old.frame);
    command(&mut gate, b'F');
    host.step();
    assert_eq!(host.managed().ready().unwrap().deadline, host.at(800));
    assert_eq!(host.shutdown(), Shutdown::Confirmed);
}

#[test]
fn ineligible_host_batches_keep_supervision_live_and_pending_opportunity_retired() {
    let (mut host, mut gate) = Host::scripted();
    host.admit(300);
    gate_read(&mut gate, b'R');
    for now in [400, 501, 700] {
        host.set(now);
        let mut lookahead = None;
        let batch = host
            .runtime
            .progress_before_event_batch(|| host.clock.get(), &mut lookahead, || Ok(Some(())))
            .unwrap();
        assert_eq!(batch.len(), 64);
        assert!(host.managed().outstanding().is_some());
        assert_eq!(host.runtime.frame_timing.deadline(), None);
    }
    host.runtime
        .refresh_frame_eligibility(host.clock.get(), &host.window, true)
        .unwrap();
    command(&mut gate, b'F');
    let mut lookahead = Some(1);
    let batch = host
        .runtime
        .progress_before_event_batch(|| host.clock.get(), &mut lookahead, || Ok(Some(2)))
        .unwrap();
    assert_eq!((batch.len(), batch[0], batch[1]), (64, 1, 2));
    assert!(host.managed().outstanding().is_none());
    assert!(host.managed().ready().is_none());
    assert_eq!(host.runtime.frame_timing.deadline(), None);
    host.runtime
        .refresh_frame_eligibility(host.clock.get(), &host.window, false)
        .unwrap();
    assert_eq!(host.runtime.frame_timing.deadline(), Some(host.at(1000)));
    assert_eq!(host.shutdown(), Shutdown::Confirmed);
}

#[test]
fn invalid_payload_is_settled_and_changed_generation_cannot_submit() {
    for script in *b"MF" {
        let (mut host, mut gate) = Host::scripted();
        host.admit(300);
        gate_read(&mut gate, b'R');
        command(&mut gate, script);
        host.step();
        assert_eq!(
            host.managed().outcome().job.unwrap().state,
            JobState::PhysicallySettled
        );
        assert!(host.managed().unresolved().is_none());
        if script == b'F' {
            assert!(host.managed().ready().is_some());
            host.runtime.frames.invalidate();
        }
        assert!(!host
            .turn(
                None,
                || Ok(None),
                || panic!("invalid candidate cannot validate")
            )
            .unwrap());
        assert!(host.transport.trace.borrow().is_empty());
        assert!(host.managed().outstanding().is_none());
        assert!(host.managed().ready().is_none());
        assert_eq!(host.shutdown(), Shutdown::Confirmed);
    }
}

#[test]
fn report_faults_and_process_exit_do_not_fabricate_settlement() {
    for (mode, settled) in [
        ("wrong-job", false),
        ("wrong-session", false),
        ("duplicate-frame", true),
        ("truncated", false),
        ("exit-job", false),
    ] {
        let mut host = Host::with_worker(&[mode]);
        host.admit(300);
        let admitted = host.managed().outstanding().unwrap();
        host.until(|managed| managed.outcome().failure.is_some());
        assert_eq!(host.managed().outstanding(), (!settled).then_some(admitted));
        assert_eq!(host.managed().unresolved(), (!settled).then_some(admitted));
        assert!(host.managed().ready().is_none());
        host.confirm_no_mutation();
        host.shutdown();
        assert!(host.managed().outcome().reaped);
        assert_eq!(host.managed().outstanding(), (!settled).then_some(admitted));
    }
}

#[test]
fn zero_byte_shutdown_withdrawal_and_host_publication_error_preserve_original_truth() {
    let mut withdrawn = Host::with_worker(&["no-render-expected"]);
    withdrawn.set(300);
    assert!(!withdrawn.turn(None, || Ok(None), || {}).unwrap());
    let key = withdrawn.managed().outcome().job.unwrap().job;
    assert_eq!(
        withdrawn.managed().outcome().job.unwrap().transmission,
        Transmission::Unsent
    );
    assert_eq!(withdrawn.shutdown(), Shutdown::Confirmed);
    let outcome = withdrawn.managed().outcome();
    assert_eq!(outcome.job.unwrap().job, key);
    assert_eq!(outcome.job.unwrap().state, JobState::RejectedBeforeDispatch);
    assert!(withdrawn.managed().outstanding().is_none());

    let (mut host, mut gate) = Host::scripted();
    host.transport.failure = Some(Step::Upload);
    host.transport.fail_safety = true;
    host.admit(300);
    gate_read(&mut gate, b'R');
    command(&mut gate, b'L');
    host.step();
    let error = host.turn(None, || Ok(None), || {}).unwrap_err();
    command(&mut gate, b'C');
    host.step();
    assert_eq!(error.to_string(), "Upload");
    assert_eq!(
        *host.transport.trace.borrow(),
        [Step::Upload, Step::SafetyEmpty, Step::SafetyRelease]
    );
    assert!(host.runtime.frames.committed().is_none());
    assert_eq!(
        host.managed().outcome().failure,
        Some(Failure::BackendFault)
    );
    assert!(host.managed().outstanding().is_none());
    host.shutdown();
}

#[test]
fn worker_and_host_descriptor_failures_stay_separate() {
    use std::os::fd::AsRawFd;
    let (host, host_peer) = UnixStream::pair().unwrap();
    let (worker, worker_peer) = UnixStream::pair().unwrap();
    let interest = roomie_worker_supervisor::WaitInterest {
        socket: Some(worker.as_raw_fd()),
        process: None,
        writable: false,
        deadline: None,
    };
    worker_peer.shutdown(std::net::Shutdown::Both).unwrap();
    let ready =
        crate::x11::events::poll_managed(Some(host.as_raw_fd()), &interest, Some(Instant::now()))
            .unwrap();
    assert!(ready.hangup);
    assert!(!ready.writable);
    drop(host_peer);
    let error =
        crate::x11::events::poll_managed(Some(host.as_raw_fd()), &interest, Some(Instant::now()))
            .unwrap_err();
    assert!(error.to_string().contains("X11 socket unavailable"));
}

#[test]
fn termination_can_be_unconfirmed_before_later_real_exit_and_reaping() {
    let mut host = Host::with_worker(&["stop-hang"]);
    let now = host.clock.get();
    host.runtime
        .managed
        .as_mut()
        .unwrap()
        .begin_shutdown(now)
        .unwrap();
    host.until(|managed| managed.outcome().stopped);
    assert_eq!(host.managed().outcome().shutdown, Shutdown::Pending);
    assert!(!host.managed().outcome().reaped);
    let close = host.managed().interests().unwrap().1.unwrap();
    host.clock.set(close);
    host.step();
    let before_exit = host.managed().outcome();
    eprintln!("observed after termination request: {before_exit:?}");
    assert!(before_exit.termination_requested);
    assert_eq!(before_exit.shutdown, Shutdown::Unconfirmed);
    assert!(!before_exit.reaped, "the kill request itself is not a reap");
    assert_eq!(host.shutdown(), Shutdown::Unconfirmed);
    assert!(host.managed().outcome().reaped);
    eprintln!("observed after later exit: {:?}", host.managed().outcome());
    assert!(host.managed().outcome().unresolved.is_none());
}

#[test]
fn pre_dispatch_refusal_reconciles_d1_and_rearms_without_a_phantom_job() {
    let mut host = Host::with_worker(&["no-render-expected"]);
    let request = host
        .presentation
        .frame_request(
            FrameKind::Scheduled,
            &host.runtime.frames,
            &mut host.runtime.frame_timing,
            || host.origin + Duration::from_millis(300),
        )
        .unwrap();
    let now = host.at(300);
    let managed = host.runtime.managed.as_mut().unwrap();
    managed.set_eligible(true).unwrap();
    managed.observe(|| now + Duration::from_millis(1)).unwrap();
    assert_eq!(managed.admit(request, now).unwrap(), None);
    assert_eq!(managed.outstanding(), None);
    assert_eq!(managed.outcome().unresolved, None);
    assert_eq!(managed.outcome().failure, Some(Failure::Clock));
    host.set(301);
    host.shutdown();
}

#[test]
fn expiration_does_not_pause_elapsed_time_or_retry_the_same_opportunity() {
    let (mut host, mut gate) = Host::scripted();
    host.admit(300);
    gate_read(&mut gate, b'R');
    host.set(700);
    host.step();
    assert_eq!(host.runtime.frame_timing.deadline(), None);
    command(&mut gate, b'F');
    host.step();
    assert_eq!(host.runtime.frame_timing.deadline(), Some(host.at(1000)));
    assert!(!host.turn(None, || Ok(None), || {}).unwrap());
    assert!(host.managed().outstanding().is_none());
    host.admit(1000);
    gate_read(&mut gate, b'R');
    command(&mut gate, b'F');
    host.step();
    let ready = host.managed().ready().unwrap();
    assert_eq!(ready.request.active_elapsed(), Duration::from_millis(1000));
    assert_eq!(ready.request.active_delta(), Duration::from_millis(300));
    assert_eq!(host.runtime.frame_timing.deadline(), None);
    assert_eq!(host.shutdown(), Shutdown::Confirmed);
}

#[test]
fn transient_refusal_consumes_one_opportunity_and_rearms_one_period() {
    let mut host = Host::with_policy(&["no-render-expected"], Duration::ZERO);
    host.set(300);
    assert!(!host.turn(None, || Ok(None), || {}).unwrap());
    assert_eq!(host.managed().outstanding(), None);
    assert_eq!(host.managed().outcome().job, None);
    assert_eq!(host.runtime.frame_timing.deadline(), Some(host.at(600)));
    assert!(!host.turn(None, || Ok(None), || {}).unwrap());
    assert_eq!(host.runtime.frame_timing.deadline(), Some(host.at(600)));
    assert_eq!(host.shutdown(), Shutdown::Confirmed);
}

#[test]
fn expired_exit_observation_preserves_uncertainty_after_later_exit() {
    use roomie_worker_supervisor::machine::Machine;
    let policy = Policy {
        graceful_shutdown: Duration::from_millis(50),
        exit_observation: Duration::from_millis(50),
        ..Policy::default()
    };
    let mut machine = Machine::new(73, policy, Duration::ZERO);
    machine.close(Duration::ZERO);
    machine.tick(policy.graceful_shutdown);
    assert_eq!(machine.outcome().phase, Phase::TerminationRequested);
    machine.tick(Duration::from_millis(100));
    assert_eq!(machine.outcome().phase, Phase::ShutdownUnconfirmed);
    assert!(!machine.outcome().reaped);
    assert_eq!(machine.deadline(), None);
    machine.receive(
        roomie_worker_protocol::Message {
            session: 73,
            body: roomie_worker_protocol::Body::Stopped,
        },
        Duration::from_millis(150),
    );
    assert!(machine.outcome().stopped);
    machine.exited(true, Duration::from_millis(200));
    assert!(machine.outcome().reaped);
    assert_eq!(machine.outcome().shutdown, Shutdown::Unconfirmed);
}

#[test]
fn late_correlated_terminal_reconciles_previously_observed_unresolved_work() {
    let (mut host, mut gate) = Host::scripted();
    host.admit(300);
    gate_read(&mut gate, b'R');
    let identity = host.managed().outstanding().unwrap();
    command(&mut gate, b'Q');
    host.step();
    assert_eq!(host.managed().outstanding(), Some(identity));
    assert_eq!(host.managed().unresolved(), Some(identity));
    assert_eq!(
        host.managed().outcome().failure,
        Some(Failure::BackendFault)
    );
    host.step();
    assert_eq!(host.managed().unresolved(), Some(identity));
    command(&mut gate, b'C');
    host.step();
    assert_eq!(
        host.managed().outcome().job.unwrap().state,
        JobState::PhysicallySettled
    );
    assert_eq!(host.managed().outstanding(), None);
    assert_eq!(host.managed().unresolved(), None);
    assert!(host.managed().ready().is_none());
    host.confirm_no_mutation();
    assert_eq!(host.shutdown(), Shutdown::Confirmed);
}

#[test]
fn all_returned_fixture_phases_match_independent_cpu_reference_and_checked_coverage() {
    let (mut host, mut gate) = Host::scripted();
    let mut prior = SyntheticProducer::new()
        .prepare_frame(
            &FrameRequest::new(
                INITIAL,
                ActionIntent {
                    action: Action::Idle,
                    facing: Facing::Neutral,
                },
                host.runtime.frames.initial_extent(),
                Duration::ZERO,
                Duration::ZERO,
            )
            .unwrap(),
        )
        .unwrap();
    let mut cpu = SyntheticProducer::new();
    for turn in 1..=5 {
        host.admit(turn * 300);
        gate_read(&mut gate, b'R');
        command(&mut gate, b'F');
        host.step();
        let ready = host.managed().ready().unwrap();
        let reference = cpu.prepare_frame(&ready.request).unwrap();
        assert_eq!(ready.frame.payload, reference.payload);
        assert_eq!(ready.frame.metadata, reference.metadata);
        let changed = prior.metadata.hit_region() != reference.metadata.hit_region();
        assert!(host.turn(None, || Ok(None), || {}).unwrap());
        let trace: &[Step] = if changed { &CUTOVER } else { &IMAGE_ONLY };
        assert_eq!(*host.transport.trace.borrow(), trace);
        assert_eq!(*host.transport.image.borrow(), reference.payload);
        assert_eq!(
            host.runtime.frames.require_committed().unwrap().metadata,
            reference.metadata,
        );
        assert_eq!(
            host.runtime.frame_timing.deadline(),
            Some(host.at((turn + 1) * 300))
        );
        host.transport.trace.borrow_mut().clear();
        prior = reference;
    }
    assert_eq!(host.shutdown(), Shutdown::Confirmed);
}

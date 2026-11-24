mod support;
use roomie_worker_protocol::{Action, Body, Decoder, Facing, JobKey, Message, Render, FRAME_BYTES};
use roomie_worker_vulkan::{config::*, diagnostic, engine::*, phase, settlement::*, worker::*};
use std::{
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};
use support::{Fixture, Mode};

fn render(serial: u64) -> Render {
    Render {
        job: JobKey {
            epoch: 7,
            serial,
            generation: 13,
            sequence: 41,
        },
        action: Action::Idle,
        facing: Facing::Neutral,
        elapsed: Duration::from_millis(900),
        delta: Duration::ZERO,
    }
}
fn message(body: Body) -> Message {
    Message { session: 29, body }
}
fn operation() -> Operation {
    Operation {
        session: 29,
        job: render(1).job,
        phase: 3,
    }
}
fn bodies(bytes: &[u8]) -> Vec<Body> {
    let mut decoder = Decoder::default();
    let mut out = vec![];
    let mut offset = 0;
    while offset < bytes.len() {
        offset += decoder.feed(&bytes[offset..]).unwrap();
        if let Some(message) = decoder.take().unwrap() {
            assert_eq!(message.session, 29);
            out.push(message.body);
        }
    }
    decoder.eof().unwrap();
    out
}
fn flush(w: &mut Worker<Fixture>) -> Vec<Body> {
    let mut bytes = Vec::new();
    for _ in 0..8 {
        if !w.pending() {
            return bodies(&bytes);
        }
        w.write(&mut bytes).unwrap();
    }
    panic!("unbounded responses");
}
fn ready(mode: Mode) -> Worker<Fixture> {
    let mut w = Worker::new(Fixture::new(mode));
    w.receive(message(Body::Initialize)).unwrap();
    assert_eq!(flush(&mut w), [Body::Ready]);
    w
}

#[test]
fn initialization_is_once_and_replies_gate_all_request_admission() {
    let mut w = Worker::new(Fixture::new(Mode::Good));
    assert!(w.receive(message(Body::Render(render(1)))).is_err());
    assert!(w.receive(message(Body::Shutdown)).is_err());
    w.receive(message(Body::Initialize)).unwrap();
    assert!(w.receive(message(Body::Render(render(1)))).is_err());
    assert_eq!(flush(&mut w), [Body::Ready]);
    assert!(w.receive(message(Body::Initialize)).is_err());
    assert_eq!(w.backend().trace.lock().unwrap().as_slice(), ["initialize"]);
}
#[test]
fn wrong_session_repeated_decreasing_zero_and_overflow_serials_are_closed() {
    let mut w = ready(Mode::Good);
    assert!(w
        .receive(Message {
            session: 30,
            body: Body::Render(render(1))
        })
        .is_err());
    assert!(w.receive(message(Body::Render(render(0)))).is_err());
    w.receive(message(Body::Render(render(8)))).unwrap();
    flush(&mut w);
    for serial in [8, 7] {
        assert!(w.receive(message(Body::Render(render(serial)))).is_err());
    }
    w.receive(message(Body::Render(render(u64::MAX)))).unwrap();
    flush(&mut w);
    assert!(w.receive(message(Body::Render(render(u64::MAX)))).is_err());
    assert_eq!(w.backend().telemetry().submissions, 2);
}
#[test]
fn every_unsupported_valid_intent_rejects_with_zero_gpu_submissions() {
    let mut w = ready(Mode::Good);
    let mut serial = 1;
    for action in [Action::Idle, Action::Walk] {
        for facing in [
            Facing::Neutral,
            Facing::FrontRight,
            Facing::FrontLeft,
            Facing::BackRight,
            Facing::BackLeft,
        ] {
            if action == Action::Idle && facing == Facing::Neutral {
                continue;
            }
            let r = Render {
                action,
                facing,
                ..render(serial)
            };
            w.receive(message(Body::Render(r))).unwrap();
            assert!(
                matches!(flush(&mut w).as_slice(), [Body::RejectedBeforeSubmission { job, .. }] if *job == r.job)
            );
            serial += 1;
        }
    }
    assert_eq!(w.backend().telemetry().submissions, 0);
    assert_eq!(w.backend().telemetry().mapping_requests, 0);
}
#[test]
fn exact_phase_boundaries_repeats_cycles_and_maximum_duration() {
    for cycle in 0..5u64 {
        for p in 0..5 {
            let start = Duration::from_millis(cycle * 1500 + p * 300);
            assert_eq!(phase(start), p as u32);
            assert_eq!(
                phase(start + Duration::from_millis(299) + Duration::from_nanos(999_999)),
                p as u32
            );
            if start != Duration::ZERO {
                assert_eq!(phase(start - Duration::from_nanos(1)), ((p + 4) % 5) as u32);
            }
        }
    }
    let huge = Duration::new(u64::MAX, 999_999_999);
    assert_eq!(
        phase(huge),
        ((u128::from(u64::MAX) * 1000 + 999) / 300 % 5) as u32
    );
}
#[test]
fn repeated_owner_times_render_the_same_phase_with_full_identity() {
    let mut w = ready(Mode::Good);
    for serial in 1..=3 {
        let r = Render {
            elapsed: Duration::from_millis(600),
            delta: Duration::from_millis(serial),
            ..render(serial)
        };
        w.receive(message(Body::Render(r))).unwrap();
        assert_eq!(
            flush(&mut w),
            [Body::Frame {
                job: r.job,
                bgra: vec![2; FRAME_BYTES]
            }]
        );
        assert_eq!(
            w.backend()
                .engine
                .as_ref()
                .unwrap()
                .evidence()
                .unwrap()
                .operation,
            Operation {
                session: 29,
                job: r.job,
                phase: 2
            }
        );
    }
}
#[test]
fn resources_persist_one_allocation_one_outstanding_and_bounded_telemetry() {
    let mut w = ready(Mode::Good);
    for serial in 1..=20 {
        w.receive(message(Body::Render(render(serial)))).unwrap();
        flush(&mut w);
        assert_eq!(w.backend().telemetry().stages.len(), 10);
    }
    let t = w.backend().telemetry();
    assert_eq!(
        (
            t.staging_allocations,
            t.staging_reuses,
            t.staging_retirements,
            t.peak_outstanding
        ),
        (1, 19, 0, 1)
    );
    assert_eq!(
        (
            t.submissions,
            t.mapping_requests,
            t.mapping_completions,
            t.explicit_unmaps
        ),
        (20, 20, 20, 20)
    );
    assert!(!w
        .backend()
        .trace
        .lock()
        .unwrap()
        .contains(&"release resources"));
}
#[test]
fn successful_readback_releases_actual_view_before_explicit_unmap() {
    let mut w = ready(Mode::Good);
    w.receive(message(Body::Render(render(1)))).unwrap();
    assert_eq!(
        flush(&mut w),
        [Body::Frame {
            job: render(1).job,
            bgra: vec![3; FRAME_BYTES]
        }]
    );
    let e = w.backend().engine.as_ref().unwrap().evidence().unwrap();
    assert_eq!(
        (e.submission, e.mapping, e.view, e.unmapped),
        (
            Submission::Complete,
            Mapping::Confirmed,
            View::Released,
            true
        )
    );
    assert!(e.settled());
    assert_eq!(
        w.backend().trace.lock().unwrap().as_slice(),
        [
            "initialize",
            "encode",
            "submit",
            "wait",
            "map request",
            "wait",
            "open view",
            "release view",
            "unmap"
        ]
    );
}
#[test]
fn delayed_callback_arrives_on_specific_mapping_wait_without_reuse() {
    let mut w = ready(Mode::Delayed);
    w.receive(message(Body::Render(render(1)))).unwrap();
    assert!(matches!(flush(&mut w).as_slice(), [Body::Frame { .. }]));
    assert_eq!(w.backend().telemetry().mapping_completions, 1);
}
#[test]
fn every_stale_callback_identity_component_faults_before_unresolved() {
    for component in 0..6 {
        let mut w = ready(Mode::Stale(component));
        w.receive(message(Body::Render(render(1)))).unwrap();
        let reports = flush(&mut w);
        assert!(
            matches!(reports.as_slice(), [Body::BackendFault(_),Body::Unresolved { job, .. }] if *job == render(1).job)
        );
        let e = w.backend().engine.as_ref().unwrap().evidence().unwrap();
        assert_eq!(e.submission, Submission::Complete);
        assert_eq!(e.mapping, Mapping::Requested);
        assert!(!e.staging_settled());
        assert!(!w.backend().trace.lock().unwrap().contains(&"open view"));
        assert_eq!(w.outcome(), Some(Exit::Uncertain));
    }
}
#[test]
fn missing_mapping_and_failed_map_wait_remain_unresolved_despite_gpu_completion() {
    for mode in [Mode::Missing, Mode::MapWait] {
        let mut w = ready(mode);
        w.receive(message(Body::Render(render(1)))).unwrap();
        assert!(matches!(
            flush(&mut w).as_slice(),
            [Body::BackendFault(_), Body::Unresolved { .. }]
        ));
        let e = w.backend().engine.as_ref().unwrap().evidence().unwrap();
        assert!(e.submission_settled());
        assert!(!e.staging_settled());
        assert_eq!(w.disconnect(), Exit::Uncertain);
    }
}
#[test]
fn expired_and_canceled_callback_channels_cannot_supply_completion_evidence() {
    let (receiver, sender) = MapReceiver::new(operation());
    let mut e = Evidence::new(operation());
    e.mapping = Mapping::Requested;
    assert!(receiver.complete(Instant::now(), &mut e).is_err());
    assert_eq!(e.mapping, Mapping::Requested);
    assert!(sender
        .try_send(Completion {
            operation: operation(),
            result: Ok(())
        })
        .is_err());
    let (receiver, sender) = MapReceiver::new(operation());
    drop(receiver);
    assert!(sender
        .try_send(Completion {
            operation: operation(),
            result: Ok(())
        })
        .is_err());
}
#[test]
fn failed_matching_callback_requires_explicit_unmap_and_can_settle_failure() {
    let mut w = ready(Mode::MapFailed);
    w.receive(message(Body::Render(render(1)))).unwrap();
    assert!(
        matches!(flush(&mut w).as_slice(), [Body::BackendFault(_),Body::JobFailed { job, .. }] if *job == render(1).job)
    );
    let e = w.backend().engine.as_ref().unwrap().evidence().unwrap();
    assert_eq!(e.mapping, Mapping::Failed);
    assert!(e.unmapped && e.safely_retired());
    assert!(!w.backend().trace.lock().unwrap().contains(&"open view"));
    w.receive(message(Body::Render(render(2)))).unwrap();
    assert!(matches!(
        flush(&mut w).as_slice(),
        [Body::RejectedBeforeSubmission { .. }]
    ));
    assert_eq!(w.backend().telemetry().submissions, 1);
}
#[test]
fn uncertain_submission_is_never_a_settled_failure_or_retried() {
    for mode in [Mode::SubmitFailure, Mode::SubmissionWait] {
        let mut w = ready(mode);
        w.receive(message(Body::Render(render(1)))).unwrap();
        assert!(matches!(
            flush(&mut w).as_slice(),
            [Body::BackendFault(_), Body::Unresolved { .. }]
        ));
        let e = w.backend().engine.as_ref().unwrap().evidence().unwrap();
        assert_eq!(
            e.submission,
            if mode == Mode::SubmitFailure {
                Submission::Attempted
            } else {
                Submission::IndexReturned
            }
        );
        assert!(!e.settled());
        assert_eq!(w.backend().telemetry().submissions, 1);
        assert_eq!(w.backend().telemetry().mapping_requests, 0);
        assert!(w.receive(message(Body::Render(render(2)))).is_err());
        let trace = Arc::clone(&w.backend().trace);
        drop(w);
        assert!(!trace.lock().unwrap().contains(&"release resources"));
    }
}
#[test]
fn retirement_does_not_establish_either_submission_or_staging_settlement() {
    let mut e = Evidence::new(operation());
    e.retired = true;
    e.submission = Submission::Attempted;
    assert!(!e.safely_retired());
    e.submission = Submission::Complete;
    e.mapping = Mapping::Requested;
    assert!(!e.safely_retired());
    e.mapping = Mapping::Confirmed;
    e.view = View::Live;
    e.unmapped = true;
    assert!(!e.safely_retired());
    e.view = View::Released;
    assert!(e.safely_retired());
    e.uncertain = true;
    assert!(!e.safely_retired());
}
#[test]
fn repack_failure_drops_view_and_unmaps_while_failed_unmap_stays_uncertain() {
    for mode in [Mode::ShortView, Mode::UnmapFailure] {
        let mut w = ready(mode);
        w.receive(message(Body::Render(render(1)))).unwrap();
        let reports = flush(&mut w);
        if mode == Mode::ShortView {
            assert!(matches!(reports[1], Body::JobFailed { .. }));
        } else {
            assert!(matches!(reports[1], Body::Unresolved { .. }));
        }
        let trace = w.backend().trace.lock().unwrap();
        let released = trace.iter().position(|&s| s == "release view").unwrap();
        assert_eq!(trace[released + 1], "unmap");
    }
}
#[test]
fn faults_before_submit_after_wait_map_and_unmap_have_correct_settled_reports() {
    for mode in [
        Mode::EncodeFailure,
        Mode::FaultBeforeSubmit,
        Mode::FaultAfterWait,
        Mode::FaultAfterMap,
        Mode::FaultDuringUnmap,
    ] {
        let mut w = ready(mode);
        w.receive(message(Body::Render(render(1)))).unwrap();
        let reports = flush(&mut w);
        if matches!(
            mode,
            Mode::FaultAfterWait | Mode::FaultAfterMap | Mode::FaultDuringUnmap
        ) {
            assert!(matches!(
                reports.as_slice(),
                [Body::BackendFault(_), Body::Unresolved { .. }]
            ));
            assert_eq!(w.outcome(), Some(Exit::Uncertain));
            assert_eq!(w.disconnect(), Exit::Uncertain);
            continue;
        }
        assert!(matches!(
            reports.as_slice(),
            [Body::BackendFault(_), Body::JobFailed { .. }]
        ));
        assert_eq!(
            w.backend().telemetry().submissions,
            u64::from(!matches!(
                mode,
                Mode::EncodeFailure | Mode::FaultBeforeSubmit
            ))
        );
        assert_eq!(w.phase(), Phase::Faulted);
        w.receive(message(Body::Render(render(2)))).unwrap();
        assert!(matches!(
            flush(&mut w).as_slice(),
            [Body::RejectedBeforeSubmission { .. }]
        ));
        w.receive(message(Body::Shutdown)).unwrap();
        assert_eq!(flush(&mut w), [Body::Stopped]);
        assert!(w.backend().telemetry().controlled_shutdown);
    }
}
#[test]
fn first_fault_latch_bounds_utf8_without_overwriting_cause() {
    for prefix in ["x".repeat(511), "".into()] {
        let text = prefix + &"界".repeat(400);
        let faults = Faults::default();
        faults.record(&text);
        faults.record("secondary");
        let retained = faults.get().unwrap();
        assert!(retained.len() <= 512);
        assert!(text.starts_with(&retained));
        assert_ne!(retained, "secondary");
        assert_eq!(diagnostic(&text), retained);
    }
}

#[test]
fn observation_uncertainty_before_unsent_frame_never_claims_settled_job_failure() {
    let mut w = ready(Mode::Good);
    w.receive(message(Body::Render(render(1)))).unwrap();
    w.backend()
        .observe_failure
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        flush(&mut w).as_slice(),
        [Body::BackendFault(_), Body::Unresolved { .. }]
    ));
    assert_eq!(w.outcome(), Some(Exit::Uncertain));
    assert_eq!(w.disconnect(), Exit::Uncertain);
}
#[test]
fn fault_barriers_suppress_ready_and_unsent_frame_and_precede_stopped() {
    let mut w = Worker::new(Fixture::new(Mode::Good));
    w.receive(message(Body::Initialize)).unwrap();
    w.backend().faults.record("late initialization fault");
    assert!(matches!(flush(&mut w).as_slice(), [Body::BackendFault(_)]));
    let mut w = ready(Mode::Good);
    w.receive(message(Body::Render(render(1)))).unwrap();
    w.backend().faults.record("late completed-frame fault");
    assert!(matches!(
        flush(&mut w).as_slice(),
        [Body::BackendFault(_), Body::JobFailed { .. }]
    ));
    let mut w = ready(Mode::Good);
    w.receive(message(Body::Shutdown)).unwrap();
    w.backend().faults.record("late release fault");
    assert!(matches!(
        flush(&mut w).as_slice(),
        [Body::BackendFault(_), Body::Stopped]
    ));
}
#[test]
fn controlled_shutdown_releases_before_acknowledgment_and_uncertain_release_never_acks() {
    let mut w = ready(Mode::Good);
    w.receive(message(Body::Shutdown)).unwrap();
    assert!(w
        .backend()
        .trace
        .lock()
        .unwrap()
        .contains(&"release resources"));
    assert_eq!(w.outcome(), None);
    assert_eq!(flush(&mut w), [Body::Stopped]);
    assert_eq!(w.outcome(), Some(Exit::Controlled));
    // Separate observer failure fixture fails before Ready and blocks teardown.
    let mut w = Worker::new(Fixture::new(Mode::ObserveFailure));
    w.receive(message(Body::Initialize)).unwrap();
    assert!(matches!(flush(&mut w).as_slice(), [Body::BackendFault(_)]));
    w.receive(message(Body::Shutdown)).unwrap();
    assert!(flush(&mut w).is_empty());
    assert_eq!(w.outcome(), Some(Exit::Uncertain));
    assert!(w.backend().telemetry().uncertain_shutdown);
}
struct ShortWriter {
    bytes: Vec<u8>,
    block: bool,
}
impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.block {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = bytes.len().min(17);
        self.bytes.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn incomplete_transmission_blocks_next_job_and_partial_frame_cannot_be_retracted() {
    let mut w = ready(Mode::Good);
    w.receive(message(Body::Render(render(1)))).unwrap();
    let mut writer = ShortWriter {
        bytes: vec![],
        block: true,
    };
    w.write(&mut writer).unwrap();
    assert!(w.receive(message(Body::Render(render(2)))).is_err());
    writer.block = false;
    w.write(&mut writer).unwrap();
    w.backend()
        .faults
        .record("asynchronous fault after frame prefix");
    for _ in 0..7000 {
        if !w.pending() {
            break;
        }
        w.write(&mut writer).unwrap();
    }
    assert!(matches!(
        bodies(&writer.bytes).as_slice(),
        [Body::Frame { .. }, Body::BackendFault(_)]
    ));
    w.receive(message(Body::Render(render(2)))).unwrap();
    assert!(matches!(
        flush(&mut w).as_slice(),
        [Body::RejectedBeforeSubmission { .. }]
    ));
    assert_eq!(w.backend().telemetry().submissions, 1);
}
#[test]
fn adapter_policy_never_substitutes_software_for_required_hardware() {
    for class in [
        DeviceClass::Cpu,
        DeviceClass::Integrated,
        DeviceClass::Discrete,
        DeviceClass::Other,
    ] {
        assert!(AdapterPolicy::Default.accepts(class));
        assert_eq!(
            AdapterPolicy::Software.accepts(class),
            class == DeviceClass::Cpu
        );
        assert_eq!(
            AdapterPolicy::Hardware.accepts(class),
            matches!(class, DeviceClass::Integrated | DeviceClass::Discrete)
        );
    }
    assert_eq!(
        LaunchConfig::parse(&[]).unwrap().adapter,
        AdapterPolicy::Default
    );
    assert_eq!(
        LaunchConfig::parse(&["--adapter".into(), "hardware".into()])
            .unwrap()
            .adapter,
        AdapterPolicy::Hardware
    );
}
#[test]
fn launch_configuration_rejects_unbounded_unknown_duplicate_and_missing_driver_paths() {
    for args in [
        vec!["--adapter"],
        vec!["--env", "DISPLAY=:0"],
        vec!["--adapter", "hardware", "--adapter", "software"],
        vec!["--driver", "relative.json"],
        vec!["--driver", "/unavailable/manifest.json"],
        vec!["--driver", "/a.json:/b.json"],
        vec!["--adapter", "unknown"],
    ] {
        assert!(
            LaunchConfig::parse(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err()
        );
    }
    let path = std::env::temp_dir().join(format!("d2b-driver-{}.json", std::process::id()));
    std::fs::write(&path, b"{}").unwrap();
    let config = LaunchConfig::parse(&[
        "--driver".into(),
        path.to_str().unwrap().into(),
        "--adapter".into(),
        "software".into(),
    ])
    .unwrap();
    assert_eq!(
        config.driver(),
        Some(path.canonicalize().unwrap().as_path())
    );
    std::fs::remove_file(path).unwrap();
}

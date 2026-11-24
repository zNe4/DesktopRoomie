mod support;
use roomie_worker_protocol::{Action, Body, Decoder, Facing, JobKey, Message, Render, FRAME_BYTES};
use roomie_worker_vulkan::{
    engine::*,
    native_contract::{checked_call, checked_unmap},
    settlement::*,
    worker::*,
};
use std::{
    io::{self, Write},
    sync::Arc,
    time::Duration,
};
use support::{Fixture, Mode};
fn operation() -> Operation {
    Operation {
        session: 29,
        job: JobKey {
            epoch: 7,
            serial: 1,
            generation: 13,
            sequence: 41,
        },
        phase: 3,
    }
}
fn request() -> Render {
    Render {
        job: operation().job,
        action: Action::Idle,
        facing: Facing::Neutral,
        elapsed: Duration::from_millis(900),
        delta: Duration::ZERO,
    }
}
fn message(body: Body) -> Message {
    Message { session: 29, body }
}
fn decode(bytes: &[u8]) -> Vec<Body> {
    let mut decoder = Decoder::default();
    let mut offset = 0;
    let mut out = vec![];
    while offset < bytes.len() {
        offset += decoder.feed(&bytes[offset..]).unwrap();
        if let Some(m) = decoder.take().unwrap() {
            assert_eq!(m.session, 29);
            out.push(m.body);
        }
    }
    decoder.eof().unwrap();
    out
}
fn flush<B: Backend>(w: &mut Worker<B>) -> Vec<Body> {
    let mut bytes = vec![];
    for _ in 0..4 {
        if !w.pending() {
            return decode(&bytes);
        }
        w.write(&mut bytes).unwrap();
    }
    panic!("response bound");
}
fn ready(mode: Mode) -> Worker<Fixture> {
    let mut w = Worker::new(Fixture::new(mode));
    w.receive(message(Body::Initialize)).unwrap();
    assert_eq!(flush(&mut w), [Body::Ready]);
    w
}
fn unresolved(reports: &[Body]) {
    assert!(
        matches!(reports,[Body::BackendFault(_),Body::Unresolved {job,..}] if *job==operation().job),
        "{reports:?}"
    );
}

#[test]
fn native_submission_index_with_error_handler_failure_is_not_acceptance() {
    let mut w = ready(Mode::SubmitReportsError);
    w.receive(message(Body::Render(request()))).unwrap();
    unresolved(&flush(&mut w));
    let e = w.backend().evidence().unwrap();
    assert_eq!(e.submission, Submission::Attempted);
    assert!(!e.settled());
    let trace = w.backend().trace.lock().unwrap();
    assert!(trace.contains(&"native submit returned index"));
    assert!(!trace.contains(&"wait"));
    assert_eq!(w.backend().faults.revision(), 1);
}
#[test]
fn normal_unmap_return_with_native_error_never_proves_unmap_or_safe_retirement() {
    let mut w = ready(Mode::UnmapReportsError);
    w.receive(message(Body::Render(request()))).unwrap();
    unresolved(&flush(&mut w));
    let e = w.backend().evidence().unwrap();
    assert_eq!(e.mapping, Mapping::Confirmed);
    assert_eq!(e.view, View::Released);
    assert!(!e.unmapped && !e.safely_retired());
    assert_eq!(w.backend().telemetry().explicit_unmaps, 0);
    assert!(w
        .backend()
        .trace
        .lock()
        .unwrap()
        .contains(&"native unmap returned normally"));
    assert_eq!(w.disconnect(), Exit::Uncertain);
}
#[test]
fn failed_map_cleanup_error_retains_first_diagnostic_and_does_not_settle() {
    let mut w = ready(Mode::FailedMapUnmapReportsError);
    w.receive(message(Body::Render(request()))).unwrap();
    let reports = flush(&mut w);
    unresolved(&reports);
    assert_eq!(w.backend().faults.get().as_deref(), Some("map failed"));
    assert_eq!(w.backend().faults.revision(), 1);
    let e = w.backend().evidence().unwrap();
    assert_eq!(e.mapping, Mapping::Failed);
    assert!(!e.unmapped);
    assert!(!w.backend().trace.lock().unwrap().contains(&"open view"));
}
#[test]
fn retained_diagnostic_and_secondary_native_errors_have_independent_authority() {
    let faults = Faults::default();
    faults.retain("first causal diagnostic");
    assert!(checked_unmap(
        &faults,
        || {
            faults.record("cleanup error returning normally");
            Ok(())
        },
        || Ok(true)
    )
    .is_err());
    faults.record("another native failure");
    assert_eq!(faults.revision(), 2);
    assert_eq!(faults.get().as_deref(), Some("first causal diagnostic"));
    assert!(
        checked_call::<()>(&faults, || panic!("must refuse calls after native error")).is_err()
    );
    assert!(checked_unmap(
        &faults,
        || panic!("must refuse cleanup after native error"),
        || Ok(true)
    )
    .is_err());
}
#[test]
fn device_loss_before_unmap_and_deferred_destroyed_block_settlement() {
    for mode in [Mode::LossBeforeUnmap, Mode::DeferredDestroyedOnUnmap] {
        let mut w = ready(mode);
        w.receive(message(Body::Render(request()))).unwrap();
        unresolved(&flush(&mut w));
        assert!(w.backend().faults.device_lost());
        assert!(!w.backend().evidence().unwrap().unmapped);
        assert_eq!(w.disconnect(), Exit::Uncertain);
        if mode == Mode::LossBeforeUnmap {
            assert!(!w.backend().trace.lock().unwrap().contains(&"unmap"));
        }
    }
}
#[test]
fn unmap_requires_queue_empty_even_without_an_error_handler_observation() {
    let mut w = ready(Mode::UnmapQueueNotEmpty);
    w.receive(message(Body::Render(request()))).unwrap();
    unresolved(&flush(&mut w));
    assert_eq!(w.backend().faults.revision(), 0);
    assert!(!w.backend().evidence().unwrap().unmapped);
}
#[test]
fn explicit_release_and_drop_share_interrupted_operation_protection() {
    for mode in [Mode::PanicSubmit, Mode::PanicSecondSubmit] {
        let mut backend = Fixture::new(mode);
        backend.initialize().unwrap();
        let trace = Arc::clone(&backend.trace);
        let engine = backend.engine.as_mut().unwrap();
        let mut op = operation();
        if mode == Mode::PanicSecondSubmit {
            engine.render(op).unwrap();
            op.job.serial += 1;
        }
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.render(op))).is_err()
        );
        assert_eq!(engine.slot(), Slot::InFlight);
        assert!(!engine.release_eligible());
        assert!(!engine.release());
        drop(backend);
        assert!(!trace.lock().unwrap().contains(&"release resources"));
    }
}
#[test]
fn intentional_drop_is_distinct_from_unexpected_loss_during_teardown() {
    let mut w = ready(Mode::DestroyedOnDrop);
    w.receive(message(Body::Shutdown)).unwrap();
    assert_eq!(flush(&mut w), [Body::Stopped]);
    assert_eq!(w.backend().faults.revision(), 0);
    assert_eq!(w.outcome(), Some(Exit::Controlled));
    let mut w = ready(Mode::UnknownOnDrop);
    w.receive(message(Body::Shutdown)).unwrap();
    assert!(matches!(flush(&mut w).as_slice(), [Body::BackendFault(_)]));
    assert_eq!(w.outcome(), Some(Exit::Uncertain));
}
#[test]
fn per_stage_timings_are_fixed_size_and_read_only_with_existing_counters() {
    let mut w = ready(Mode::Good);
    w.receive(message(Body::Render(request()))).unwrap();
    flush(&mut w);
    let t = w.backend().telemetry();
    for time in [
        t.times.encode,
        t.times.submission,
        t.times.submission_wait,
        t.times.mapping,
        t.times.repack,
    ] {
        assert!(time <= t.preparation);
    }
    assert_eq!(
        (
            t.submissions,
            t.mapping_requests,
            t.mapping_completions,
            t.explicit_unmaps
        ),
        (1, 1, 1, 1)
    );
    assert!(t.stages.contains(&Stage::SubmissionAccepted));
}

#[derive(Clone, Copy)]
enum Lie {
    Missing,
    WrongOperation,
    Unsettled,
    Contradictory,
    InvalidUnsettled,
    InvalidSettled,
    WrongFailure,
    RejectedUnsettled,
}
struct Adversary {
    lie: Lie,
    current: Option<Evidence>,
}
fn complete(operation: Operation) -> Evidence {
    Evidence {
        operation,
        submission: Submission::Complete,
        mapping: Mapping::Confirmed,
        view: View::Released,
        unmapped: true,
        retired: false,
        uncertain: false,
    }
}
impl Backend for Adversary {
    fn initialize(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn render(&mut self, op: Operation) -> Result<Prepared, Failure> {
        let mut evidence = complete(op);
        let mut bgra = vec![0; FRAME_BYTES];
        match self.lie {
            Lie::WrongOperation => evidence.operation.job.sequence ^= 1,
            Lie::Unsettled | Lie::InvalidUnsettled => evidence.unmapped = false,
            _ => {}
        }
        self.current = Some(evidence);
        if matches!(self.lie, Lie::Missing) {
            self.current = None;
        }
        if matches!(self.lie, Lie::Contradictory) {
            self.current = Some(Evidence::new(op));
        }
        if matches!(self.lie, Lie::InvalidUnsettled | Lie::InvalidSettled) {
            bgra.truncate(1);
        }
        if matches!(self.lie, Lie::WrongFailure) {
            evidence.operation.session ^= 1;
            return Err(Failure {
                diagnostic: "false settled failure".into(),
                evidence,
            });
        }
        Ok(Prepared { bgra, evidence })
    }
    fn fault(&mut self) -> Option<String> {
        None
    }
    fn settled(&self) -> bool {
        true
    } // deliberately lies: worker must also check correlated evidence
    fn evidence(&self) -> Option<Evidence> {
        self.current
    }
    fn reject(&mut self, op: Operation) -> Option<Evidence> {
        let mut e = Evidence::new(op);
        e.submission = Submission::Attempted;
        self.current = Some(e);
        Some(e)
    }
    fn release(&mut self) -> bool {
        true
    }
}
fn adversary(lie: Lie) -> Worker<Adversary> {
    let mut w = Worker::new(Adversary { lie, current: None });
    w.receive(message(Body::Initialize)).unwrap();
    assert_eq!(flush(&mut w), [Body::Ready]);
    w
}
#[test]
fn apparently_successful_backend_must_supply_current_physical_settlement() {
    for lie in [Lie::Missing, Lie::Unsettled, Lie::Contradictory] {
        let mut w = adversary(lie);
        w.receive(message(Body::Render(request()))).unwrap();
        unresolved(&flush(&mut w));
    }
}
#[test]
fn success_or_failure_evidence_for_another_operation_cannot_settle_owner_job() {
    for lie in [Lie::WrongOperation, Lie::WrongFailure] {
        let mut w = adversary(lie);
        w.receive(message(Body::Render(request()))).unwrap();
        unresolved(&flush(&mut w));
    }
}
#[test]
fn invalid_payload_with_unsettled_backend_cannot_become_job_failed() {
    let mut w = adversary(Lie::InvalidUnsettled);
    w.receive(message(Body::Render(request()))).unwrap();
    unresolved(&flush(&mut w));
    let mut w = adversary(Lie::InvalidSettled);
    w.receive(message(Body::Render(request()))).unwrap();
    assert!(matches!(
        flush(&mut w).as_slice(),
        [Body::BackendFault(_), Body::JobFailed { .. }]
    ));
}
#[test]
fn rejection_requires_correlated_never_submitted_backend_evidence() {
    let mut w = adversary(Lie::RejectedUnsettled);
    let mut r = request();
    r.action = Action::Walk;
    w.receive(message(Body::Render(r))).unwrap();
    unresolved(&flush(&mut w));
    let mut w = ready(Mode::Good);
    let mut r = request();
    r.action = Action::Walk;
    w.receive(message(Body::Render(r))).unwrap();
    w.backend()
        .faults
        .record("late backend fault before rejection");
    assert!(matches!(
        flush(&mut w).as_slice(),
        [Body::BackendFault(_), Body::RejectedBeforeSubmission { .. }]
    ));
}

struct Prefix(Vec<u8>);
impl Write for Prefix {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = bytes.len().min(17);
        self.0.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn faults_before_and_during_ready_frame_and_stopped_preserve_framing_and_authority() {
    for kind in 0..3 {
        for partial in [false, true] {
            let mut w = if kind == 0 {
                let mut w = Worker::new(Fixture::new(Mode::Good));
                w.receive(message(Body::Initialize)).unwrap();
                w
            } else {
                ready(Mode::Good)
            };
            if kind == 1 {
                w.receive(message(Body::Render(request()))).unwrap();
            }
            if kind == 2 {
                w.receive(message(Body::Shutdown)).unwrap();
            }
            let mut writer = Prefix(vec![]);
            if partial {
                w.write(&mut writer).unwrap();
            }
            w.backend().faults.record("observed control barrier fault");
            for _ in 0..7000 {
                if !w.pending() {
                    break;
                }
                w.write(&mut writer).unwrap();
            }
            assert!(!w.pending());
            let reports = decode(&writer.0);
            assert!(w.backend_faulted());
            match (kind, partial) {
                (0, false) => assert!(matches!(reports.as_slice(), [Body::BackendFault(_)])),
                (0, true) => assert!(matches!(
                    reports.as_slice(),
                    [Body::Ready, Body::BackendFault(_)]
                )),
                (1, false) => assert!(matches!(
                    reports.as_slice(),
                    [Body::BackendFault(_), Body::JobFailed { .. }]
                )),
                (1, true) => assert!(matches!(
                    reports.as_slice(),
                    [Body::Frame { .. }, Body::BackendFault(_)]
                )),
                (2, false) => assert!(matches!(
                    reports.as_slice(),
                    [Body::BackendFault(_), Body::Stopped]
                )),
                (2, true) => assert!(matches!(
                    reports.as_slice(),
                    [Body::Stopped, Body::BackendFault(_)]
                )),
                _ => unreachable!(),
            }
            assert_ne!(w.phase(), Phase::Ready);
        }
    }
}
#[test]
fn loss_before_or_during_stopped_never_becomes_successful_shutdown() {
    for partial in [false, true] {
        let mut w = ready(Mode::Good);
        w.receive(message(Body::Shutdown)).unwrap();
        let mut writer = Prefix(vec![]);
        if partial {
            w.write(&mut writer).unwrap();
        }
        w.backend().faults.observe_loss(false, "late loss");
        for _ in 0..8 {
            if !w.pending() {
                break;
            }
            if w.write(&mut writer).is_err() {
                break;
            }
        }
        if partial {
            // R2 strengthens the reviewed R1 expectation: an invalidated
            // Stopped prefix must remain truncated, never completed.
            let mut decoder = Decoder::default();
            assert_eq!(decoder.feed(&writer.0).unwrap(), writer.0.len());
            assert_eq!(decoder.take().unwrap(), None);
            assert_eq!(decoder.eof(), Err(roomie_worker_protocol::Error::Truncated));
            assert!(w.transport_abandoned());
        } else {
            assert!(matches!(
                decode(&writer.0).as_slice(),
                [Body::BackendFault(_)]
            ));
            assert!(w.backend_faulted());
        }
        assert_eq!(w.outcome(), Some(Exit::Uncertain));
    }
}

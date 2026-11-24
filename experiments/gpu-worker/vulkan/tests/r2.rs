//! Deterministic terminal byte boundaries. No Vulkan or X11 execution.
mod support;
use roomie_worker_protocol::{Action, Body, Decoder, Error, Facing, JobKey, Message, Render};
use roomie_worker_vulkan::{engine::*, settlement::*, worker::*};
use std::{
    cell::Cell,
    io::{self, Write},
    sync::atomic::Ordering,
    time::Duration,
};
use support::{Fixture, Mode};

#[derive(Clone, Copy, Debug)]
enum Terminal {
    Frame,
    Failed,
    Rejected,
}
#[derive(Clone, Copy, Debug)]
enum Boundary {
    Unsent,
    Partial,
    Complete,
}
#[derive(Clone, Copy, Debug)]
enum Change {
    Ordinary,
    Loss,
    Observation,
    Evidence,
}
struct Controlled {
    fixture: Fixture,
    terminal: Terminal,
    evidence_change: Cell<u8>,
    stop_uncertain: Cell<bool>,
}
impl Backend for Controlled {
    fn initialize(&mut self) -> Result<(), String> {
        self.fixture.initialize()
    }
    fn render(&mut self, op: Operation) -> Result<Prepared, Failure> {
        let mut prepared = self.fixture.render(op)?;
        if matches!(self.terminal, Terminal::Failed) {
            prepared.bgra.truncate(1);
        }
        Ok(prepared)
    }
    fn fault(&mut self) -> Option<String> {
        self.fixture.fault()
    }
    fn settled(&self) -> bool {
        !self.stop_uncertain.get() && self.fixture.settled()
    }
    fn evidence(&self) -> Option<Evidence> {
        let mut e = self.fixture.evidence()?;
        match self.evidence_change.get() {
            1 => return None,
            2 => e.uncertain = true,
            3 => e.operation.session ^= 1,
            4 => e.operation.job.epoch ^= 1,
            5 => e.operation.job.serial ^= 1,
            6 => e.operation.job.generation ^= 1,
            7 => e.operation.job.sequence ^= 1,
            8 => e.operation.phase ^= 1,
            _ => {}
        }
        Some(e)
    }
    fn reject(&mut self, op: Operation) -> Option<Evidence> {
        self.fixture.reject(op)
    }
    fn release(&mut self) -> bool {
        self.fixture.release()
    }
}
fn request() -> Render {
    Render {
        job: JobKey {
            epoch: 7,
            serial: 1,
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
fn ready(terminal: Terminal) -> Worker<Controlled> {
    let mut w = Worker::new(Controlled {
        fixture: Fixture::new(Mode::Good),
        terminal,
        evidence_change: Cell::new(0),
        stop_uncertain: Cell::new(false),
    });
    w.receive(message(Body::Initialize)).unwrap();
    assert_eq!(decode(&drain(&mut w)), [Body::Ready]);
    w
}
fn drain(w: &mut Worker<Controlled>) -> Vec<u8> {
    let mut bytes = vec![];
    for _ in 0..4 {
        if !w.pending() {
            return bytes;
        }
        w.write(&mut bytes).unwrap();
    }
    panic!("bounded response queue");
}
fn decode(bytes: &[u8]) -> Vec<Body> {
    let mut d = Decoder::default();
    let mut offset = 0;
    let mut out = vec![];
    while offset < bytes.len() {
        offset += d.feed(&bytes[offset..]).unwrap();
        if let Some(m) = d.take().unwrap() {
            out.push(m.body);
        }
    }
    d.eof().unwrap();
    out
}
struct Limited {
    bytes: Vec<u8>,
    limit: usize,
    calls: usize,
}
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.limit == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = self.limit.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn truncated(bytes: &[u8]) {
    let mut d = Decoder::default();
    let mut offset = 0;
    while offset < bytes.len() {
        offset += d.feed(&bytes[offset..]).unwrap();
        assert_eq!(d.take().unwrap(), None, "invalid terminal became decodable");
    }
    assert_eq!(d.eof(), Err(Error::Truncated));
}
fn is_terminal(body: &Body, terminal: Terminal) -> bool {
    match body {
        Body::Frame { job, .. } if matches!(terminal, Terminal::Frame) => *job == request().job,
        Body::JobFailed { job, .. } if matches!(terminal, Terminal::Failed) => {
            *job == request().job
        }
        Body::RejectedBeforeSubmission { job, .. } if matches!(terminal, Terminal::Rejected) => {
            *job == request().job
        }
        _ => false,
    }
}
fn matrix(terminal: Terminal, boundary: Boundary, change: Change) {
    // Missing, unsettled and every mismatched operation field, including phase.
    let variants = if matches!(change, Change::Evidence) {
        8
    } else if matches!(change, Change::Loss) {
        2
    } else {
        1
    };
    for variant in 1..=variants {
        for prefix in [1, 17, 32] {
            let mut w = ready(terminal);
            let mut r = request();
            if matches!(terminal, Terminal::Rejected) {
                r.action = Action::Walk;
            }
            w.receive(message(Body::Render(r))).unwrap();
            if matches!(terminal, Terminal::Failed) {
                let mut first = vec![];
                w.write(&mut first).unwrap();
                assert!(matches!(decode(&first).as_slice(), [Body::BackendFault(_)]));
            }
            let mut writer = Limited {
                bytes: vec![],
                limit: match boundary {
                    Boundary::Unsent => 0,
                    Boundary::Partial => prefix,
                    Boundary::Complete => usize::MAX,
                },
                calls: 0,
            };
            w.write(&mut writer).unwrap();
            let old = writer.bytes.clone();
            match change {
                Change::Ordinary => w.backend().fixture.faults.record("ordinary late fault"),
                Change::Loss => w
                    .backend()
                    .fixture
                    .faults
                    .observe_loss(variant == 2, "unexpected late loss"),
                Change::Observation => w
                    .backend()
                    .fixture
                    .observe_failure
                    .store(true, Ordering::SeqCst),
                Change::Evidence => w.backend().evidence_change.set(variant),
            }
            w.observe();
            if matches!(boundary, Boundary::Partial) && !matches!(change, Change::Ordinary) {
                assert!(w.transport_abandoned());
                assert!(!w.pending());
                assert_eq!(w.outcome(), Some(Exit::Uncertain));
                // Even if a test adapter restores its reported proof, the
                // abandoned stream can never regain write authority.
                w.backend().evidence_change.set(0);
                let calls = writer.calls;
                for _ in 0..3 {
                    assert_eq!(
                        w.write(&mut writer).unwrap_err().kind(),
                        io::ErrorKind::BrokenPipe
                    );
                    w.observe();
                    assert_eq!(w.disconnect(), Exit::Uncertain);
                }
                assert_eq!(
                    writer.calls, calls,
                    "abandonment must prevent even an attempted write"
                );
                assert_eq!(
                    writer.bytes, old,
                    "no completion or spliced fault after prefix"
                );
                truncated(&writer.bytes);
                assert!(w.receive(message(Body::Shutdown)).is_err());
            } else {
                writer.limit = usize::MAX;
                for _ in 0..4 {
                    if w.pending() {
                        w.write(&mut writer).unwrap();
                    }
                }
                assert!(!w.pending());
                let reports = decode(&writer.bytes);
                if matches!(boundary, Boundary::Complete) {
                    assert!(is_terminal(&decode(&old)[0], terminal));
                    assert!(writer.bytes.starts_with(&old)); // full reports cannot be retracted
                    assert_eq!(
                        reports.iter().filter(|b| is_terminal(b, terminal)).count(),
                        1
                    );
                    assert!(!reports.iter().any(|b| matches!(b, Body::Unresolved { .. })));
                } else if matches!(change, Change::Ordinary) {
                    assert!(!w.transport_abandoned());
                    if matches!(boundary, Boundary::Partial)
                        || matches!(terminal, Terminal::Rejected)
                    {
                        assert!(reports.iter().any(|b| is_terminal(b, terminal)));
                    } else {
                        assert!(reports
                            .iter()
                            .any(|b| matches!(b, Body::JobFailed { job, .. } if *job == r.job)));
                    }
                    assert!(!reports.iter().any(|b| matches!(b, Body::Unresolved { .. })));
                } else {
                    assert!(old.is_empty());
                    assert!(
                        matches!(reports.last(), Some(Body::Unresolved { job, .. }) if *job == r.job)
                    );
                    assert!(!reports.iter().any(|b| is_terminal(b, terminal)));
                    assert!(w.backend_faulted());
                    assert_eq!(w.outcome(), Some(Exit::Uncertain));
                }
            }
            if matches!(change, Change::Loss | Change::Observation) {
                assert!(!w
                    .backend()
                    .fixture
                    .engine
                    .as_ref()
                    .unwrap()
                    .release_eligible());
                let trace = w.backend().fixture.trace.clone();
                drop(w);
                assert!(!trace.lock().unwrap().contains(&"release resources"));
            }
        }
    }
}
macro_rules! cases {
    ($terminal:ident, $change:ident, $unsent:ident, $partial:ident, $complete:ident) => {
        #[test]
        fn $unsent() {
            matrix(Terminal::$terminal, Boundary::Unsent, Change::$change);
        }
        #[test]
        fn $partial() {
            matrix(Terminal::$terminal, Boundary::Partial, Change::$change);
        }
        #[test]
        fn $complete() {
            matrix(Terminal::$terminal, Boundary::Complete, Change::$change);
        }
    };
}
cases!(
    Frame,
    Ordinary,
    frame_unsent_fault,
    frame_partial_fault,
    frame_complete_fault
);
cases!(
    Frame,
    Loss,
    frame_unsent_loss,
    frame_partial_loss,
    frame_complete_loss
);
cases!(
    Frame,
    Observation,
    frame_unsent_observation,
    frame_partial_observation,
    frame_complete_observation
);
cases!(
    Frame,
    Evidence,
    frame_unsent_evidence,
    frame_partial_evidence,
    frame_complete_evidence
);
cases!(
    Failed,
    Ordinary,
    failed_unsent_fault,
    failed_partial_fault,
    failed_complete_fault
);
cases!(
    Failed,
    Loss,
    failed_unsent_loss,
    failed_partial_loss,
    failed_complete_loss
);
cases!(
    Failed,
    Observation,
    failed_unsent_observation,
    failed_partial_observation,
    failed_complete_observation
);
cases!(
    Failed,
    Evidence,
    failed_unsent_evidence,
    failed_partial_evidence,
    failed_complete_evidence
);
cases!(
    Rejected,
    Ordinary,
    rejected_unsent_fault,
    rejected_partial_fault,
    rejected_complete_fault
);
cases!(
    Rejected,
    Loss,
    rejected_unsent_loss,
    rejected_partial_loss,
    rejected_complete_loss
);
cases!(
    Rejected,
    Observation,
    rejected_unsent_observation,
    rejected_partial_observation,
    rejected_complete_observation
);
cases!(
    Rejected,
    Evidence,
    rejected_unsent_evidence,
    rejected_partial_evidence,
    rejected_complete_evidence
);

#[test]
fn stopped_invalidated_before_partial_and_after_complete_transmission_fails_closed() {
    for boundary in [Boundary::Unsent, Boundary::Partial, Boundary::Complete] {
        for loss in [false, true] {
            let mut w = ready(Terminal::Frame);
            w.receive(message(Body::Shutdown)).unwrap();
            let mut writer = Limited {
                bytes: vec![],
                limit: match boundary {
                    Boundary::Unsent => 0,
                    Boundary::Partial => 17,
                    Boundary::Complete => usize::MAX,
                },
                calls: 0,
            };
            w.write(&mut writer).unwrap();
            let old = writer.bytes.clone();
            if loss {
                w.backend()
                    .fixture
                    .faults
                    .observe_loss(false, "loss at Stopped");
            } else {
                w.backend().stop_uncertain.set(true);
            }
            w.observe();
            if matches!(boundary, Boundary::Partial) {
                assert!(w.write(&mut writer).is_err());
                assert_eq!(old, writer.bytes);
                truncated(&writer.bytes);
                assert!(w.transport_abandoned());
            } else {
                writer.bytes.extend(drain(&mut w));
                let reports = decode(&writer.bytes);
                assert!(matches!(reports.last(), Some(Body::BackendFault(_))));
                assert_eq!(
                    reports
                        .iter()
                        .filter(|b| matches!(b, Body::Stopped))
                        .count(),
                    usize::from(matches!(boundary, Boundary::Complete))
                );
            }
            assert_eq!(w.outcome(), Some(Exit::Uncertain));
            assert_eq!(w.disconnect(), Exit::Uncertain);
        }
    }
}
#[test]
fn release_preparation_unwind_blocks_explicit_retry_and_defensive_drop() {
    for explicit_retry in [false, true] {
        let mut fixture = Fixture::new(Mode::PanicReleasePreparation);
        fixture.initialize().unwrap();
        let trace = fixture.trace.clone();
        let engine = fixture.engine.as_mut().unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.release())).is_err()
        );
        assert!(!engine.release_eligible());
        assert!(engine.telemetry().uncertain_shutdown);
        assert!(!engine.telemetry().controlled_shutdown);
        if explicit_retry {
            assert!(!engine.release());
            assert!(!engine.release());
        }
        drop(fixture);
        let trace = trace.lock().unwrap();
        assert_eq!(trace.iter().filter(|&&s| s == "prepare release").count(), 1);
        assert!(!trace.contains(&"release resources"));
    }
}

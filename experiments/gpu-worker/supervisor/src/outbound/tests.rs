use super::*;
use crate::machine::{Failure, JobState, Phase, Policy, Report, Transmission};
use roomie_worker_protocol::{encode, Action, Decoder, Facing, FRAME_BYTES};
use std::time::Duration;
fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}
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
        elapsed: at(300),
        delta: at(10),
    }
}
fn msg(body: Body) -> Message {
    Message { session: 91, body }
}
struct Writer {
    bytes: Vec<u8>,
    limit: usize,
    blocked: bool,
    forbidden: bool,
    calls: usize,
}
impl Default for Writer {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            limit: usize::MAX,
            blocked: false,
            forbidden: false,
            calls: 0,
        }
    }
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        assert!(!self.forbidden, "write attempted after termination");
        self.calls += 1;
        if self.blocked {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = bytes.len().min(self.limit);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Harness {
    machine: Machine,
    outbound: Outbound,
    writer: Writer,
}
impl Harness {
    fn new() -> Self {
        Self {
            machine: Machine::new(91, Policy::default(), at(0)),
            outbound: Outbound::new(91),
            writer: Writer::default(),
        }
    }
    fn ready() -> Self {
        let mut h = Self::new();
        h.turn(0);
        assert!(h.machine.outcome().initialize_transmitted);
        h.machine.receive(msg(Body::Ready), at(0));
        assert_eq!(h.machine.outcome().phase, Phase::Ready);
        h.writer.bytes.clear();
        h.writer.calls = 0;
        h
    }
    fn admit(&mut self) {
        self.machine.admit(render(), at(0)).unwrap();
        self.outbound.render(91, render()).unwrap();
        self.assert_job(JobState::AdmittedUnsent, Transmission::Unsent);
    }
    fn turn(&mut self, ms: u64) {
        self.machine.tick(at(ms));
        self.outbound
            .progress(&mut self.machine, true, &mut self.writer)
            .unwrap();
    }
    fn assert_job(&self, state: JobState, transmission: Transmission) {
        let job = self.machine.outcome().job.unwrap();
        assert_eq!(job.job, render().job);
        assert_eq!(job.state, state);
        assert_eq!(job.transmission, transmission);
    }
}
fn messages(bytes: &[u8]) -> Vec<Message> {
    let mut decoder = Decoder::default();
    let mut offset = 0;
    let mut result = Vec::new();
    while offset < bytes.len() {
        offset += decoder.feed(&bytes[offset..]).unwrap();
        if let Some(message) = decoder.take().unwrap() {
            result.push(message);
        }
    }
    decoder.eof().unwrap();
    result
}
#[test]
fn entirely_unsent_render_at_escalation_is_explicitly_rejected_without_writing() {
    let mut h = Harness::ready();
    h.admit();
    h.writer.blocked = true;
    h.turn(1); // Real transmitter sees WouldBlock, offset stays zero.
    h.assert_job(JobState::AdmittedUnsent, Transmission::Unsent);
    h.writer.forbidden = true;
    h.turn(2500);
    assert_eq!(h.machine.outcome().phase, Phase::TerminationRequested);
    assert_eq!(h.machine.outcome().failure, Some(Failure::Escalation));
    assert_eq!(h.machine.outcome().unresolved, None);
    h.assert_job(JobState::RejectedBeforeDispatch, Transmission::Unsent);
    assert_eq!(
        h.machine.take_report(),
        Some(Report::RejectedBeforeDispatch { job: render().job })
    );
    assert_eq!(h.outbound.pending(), 0);
    assert!(!h.outbound.writable(&h.machine));
    assert!(h.writer.bytes.is_empty());
    h.turn(3000);
    assert_eq!(h.writer.calls, 1);
}
#[test]
fn partially_transmitted_render_at_escalation_stays_unresolved_and_never_writes_again() {
    let mut h = Harness::ready();
    h.admit();
    h.writer.limit = 7;
    h.turn(1);
    h.assert_job(JobState::PartiallyTransmitted, Transmission::Partial);
    let prefix = h.writer.bytes.clone();
    h.writer.forbidden = true;
    h.turn(2500);
    h.assert_job(JobState::Unresolved, Transmission::Partial);
    assert_eq!(h.machine.outcome().unresolved, Some(render().job));
    assert!(!h.outbound.writable(&h.machine));
    assert!(!h.machine.reject_before_dispatch(render().job));
    h.machine.receive(
        msg(Body::JobFailed {
            job: render().job,
            diagnostic: "premature assertion".into(),
        }),
        at(2500),
    );
    assert_eq!(h.machine.outcome().unresolved, Some(render().job));
    assert!(h.machine.take_report().is_none());
    h.turn(3000);
    assert_eq!(h.machine.outcome().phase, Phase::ShutdownUnconfirmed);
    h.machine.exited(false, at(3001));
    h.turn(3001);
    h.assert_job(JobState::Unresolved, Transmission::Partial);
    assert_eq!(h.writer.bytes, prefix);
    assert_eq!(h.writer.calls, 1);
}
#[test]
fn termination_and_unconfirmed_shutdown_block_initialize_and_shutdown_writes_too() {
    for initialized in [false, true] {
        let mut h = if initialized {
            Harness::ready()
        } else {
            Harness::new()
        };
        h.machine.close(at(0));
        // Prepare Shutdown behind any Initialize without invoking a writer.
        h.outbound
            .progress(&mut h.machine, false, &mut h.writer)
            .unwrap();
        h.writer.forbidden = true;
        h.turn(500);
        h.turn(1000);
        assert_eq!(h.machine.outcome().phase, Phase::ShutdownUnconfirmed);
        assert!(!h.outbound.writable(&h.machine));
        assert_eq!(h.writer.calls, 0);
    }
}
#[test]
fn graceful_closing_rejects_unsent_render_and_sends_only_shutdown() {
    let mut h = Harness::ready();
    h.admit();
    h.machine.close(at(10));
    h.turn(10);
    assert_eq!(messages(&h.writer.bytes), [msg(Body::Shutdown)]);
    h.assert_job(JobState::RejectedBeforeDispatch, Transmission::Unsent);
    assert_eq!(
        h.machine.take_report(),
        Some(Report::RejectedBeforeDispatch { job: render().job })
    );
    assert_eq!(h.machine.outcome().unresolved, None);
    assert_eq!(h.machine.deadline(), Some(at(510)));
}
#[test]
fn graceful_closing_finishes_partial_render_then_shutdown_without_renewing_deadline() {
    let mut h = Harness::ready();
    h.admit();
    h.writer.limit = 7;
    h.turn(1);
    h.machine.close(at(10));
    h.turn(10);
    h.writer.blocked = true;
    h.turn(11); // Backpressure neither retires nor replaces the prefix.
    h.assert_job(JobState::PartiallyTransmitted, Transmission::Partial);
    assert_eq!(h.machine.deadline(), Some(at(510)));
    h.writer.blocked = false;
    h.writer.limit = usize::MAX;
    h.turn(12);
    h.assert_job(JobState::CompletelyTransmitted, Transmission::Complete);
    assert_eq!(h.machine.outcome().unresolved, Some(render().job));
    assert_eq!(messages(&h.writer.bytes), [msg(Body::Render(render()))]);
    h.turn(13);
    assert_eq!(
        messages(&h.writer.bytes),
        [msg(Body::Render(render())), msg(Body::Shutdown)]
    );
    assert_eq!(h.machine.deadline(), Some(at(510)));
    h.machine.receive(
        msg(Body::JobFailed {
            job: render().job,
            diagnostic: "settled".into(),
        }),
        at(14),
    );
    h.assert_job(JobState::PhysicallySettled, Transmission::Complete);
    assert_eq!(h.machine.outcome().unresolved, None);
    assert!(matches!(
        h.machine.take_report(),
        Some(Report::Failed { .. })
    ));
}
#[test]
fn graceful_deadline_abandons_partial_stream_without_appending_shutdown_or_rejecting_debt() {
    let mut h = Harness::ready();
    h.admit();
    h.writer.limit = 7;
    h.turn(1);
    h.machine.close(at(10));
    h.writer.blocked = true;
    h.turn(100);
    h.machine.close(at(200));
    assert_eq!(h.machine.deadline(), Some(at(510)));
    h.writer.forbidden = true;
    h.turn(510);
    h.assert_job(JobState::Unresolved, Transmission::Partial);
    assert_eq!(h.machine.deadline(), Some(at(1010)));
    assert_eq!(
        h.writer.bytes,
        encode(&msg(Body::Render(render()))).unwrap()[..7]
    );
    assert!(h.machine.take_report().is_none());
    assert!(!h.outbound.writable(&h.machine));
}
#[test]
fn ready_is_premature_until_initialize_is_completely_transmitted() {
    for prefix in [0, 7] {
        let mut h = Harness::new();
        if prefix != 0 {
            h.writer.limit = prefix;
            h.turn(0);
        }
        h.machine.receive(msg(Body::Ready), at(1));
        assert!(!h.machine.outcome().initialize_transmitted);
        assert_eq!(h.machine.outcome().failure, Some(Failure::Protocol));
        assert_eq!(h.machine.outcome().phase, Phase::Closing);
        assert!(h.machine.admit(render(), at(1)).is_err());
    }
}
#[test]
fn premature_matching_job_reports_never_discharge_unsent_or_partial_work() {
    for partial in [false, true] {
        for body in [
            Body::Frame {
                job: render().job,
                bgra: vec![0; FRAME_BYTES],
            },
            Body::RejectedBeforeSubmission {
                job: render().job,
                diagnostic: "premature".into(),
            },
            Body::JobFailed {
                job: render().job,
                diagnostic: "premature".into(),
            },
            Body::Unresolved {
                job: render().job,
                diagnostic: "premature".into(),
            },
        ] {
            let mut h = Harness::ready();
            h.admit();
            if partial {
                h.writer.limit = 7;
                h.turn(0);
            }
            h.machine.receive(msg(body), at(1));
            assert_eq!(h.machine.outcome().failure, Some(Failure::Protocol));
            assert_eq!(h.machine.outcome().unresolved, Some(render().job));
            assert!(h.machine.take_report().is_none());
            if partial {
                assert!(!h.machine.reject_before_dispatch(render().job));
            }
        }
    }
}
#[test]
fn backend_fault_authority_is_independent_of_initialization_and_job_dispatch() {
    for initialized in [false, true] {
        let mut h = if initialized {
            Harness::ready()
        } else {
            Harness::new()
        };
        h.machine
            .receive(msg(Body::BackendFault("current backend".into())), at(0));
        assert_eq!(h.machine.outcome().failure, Some(Failure::BackendFault));
    }
    for partial in [false, true] {
        let mut h = Harness::ready();
        h.admit();
        if partial {
            h.writer.limit = 7;
            h.turn(0);
        }
        h.machine
            .receive(msg(Body::BackendFault("device lost".into())), at(1));
        assert_eq!(h.machine.outcome().failure, Some(Failure::BackendFault));
        h.machine.receive(
            msg(Body::JobFailed {
                job: render().job,
                diagnostic: "premature".into(),
            }),
            at(1),
        );
        assert_eq!(h.machine.outcome().failure, Some(Failure::BackendFault));
        assert_eq!(h.machine.outcome().unresolved, Some(render().job));
        assert!(h.machine.take_report().is_none());
    }
}
#[test]
fn dispatch_facts_follow_positive_writes_and_settlement_requires_complete_dispatch() {
    let mut h = Harness::ready();
    h.admit();
    h.writer.blocked = true;
    h.turn(0);
    h.assert_job(JobState::AdmittedUnsent, Transmission::Unsent);
    h.writer.blocked = false;
    h.writer.limit = 1;
    h.turn(0);
    h.assert_job(JobState::PartiallyTransmitted, Transmission::Partial);
    h.writer.limit = usize::MAX;
    h.turn(0);
    h.assert_job(JobState::CompletelyTransmitted, Transmission::Complete);
    h.machine.receive(
        msg(Body::Frame {
            job: render().job,
            bgra: vec![0; FRAME_BYTES],
        }),
        at(1),
    );
    h.assert_job(JobState::PhysicallySettled, Transmission::Complete);
    assert!(matches!(
        h.machine.take_report(),
        Some(Report::Frame { .. })
    ));
}
#[test]
fn local_predispatch_rejection_cannot_retire_partial_or_fully_transmitted_work() {
    for limit in [1, usize::MAX] {
        let mut h = Harness::ready();
        h.admit();
        h.writer.limit = limit;
        h.turn(0);
        h.machine.close(at(1));
        assert!(!h.machine.reject_before_dispatch(render().job));
        assert_eq!(h.machine.outcome().unresolved, Some(render().job));
        assert!(h.machine.take_report().is_none());
    }
}
#[test]
fn complete_dispatch_can_settle_after_termination_without_resuming_protocol_writes() {
    for body in [
        Body::Frame {
            job: render().job,
            bgra: vec![0; FRAME_BYTES],
        },
        Body::JobFailed {
            job: render().job,
            diagnostic: "late settled failure".into(),
        },
        Body::RejectedBeforeSubmission {
            job: render().job,
            diagnostic: "late rejection".into(),
        },
    ] {
        let mut h = Harness::ready();
        h.admit();
        h.turn(0);
        h.writer.forbidden = true;
        h.turn(2500);
        h.assert_job(JobState::Unresolved, Transmission::Complete);
        assert_eq!(h.machine.outcome().unresolved, Some(render().job));
        h.machine.receive(msg(body), at(2501));
        h.assert_job(JobState::PhysicallySettled, Transmission::Complete);
        assert_eq!(h.machine.outcome().unresolved, None);
        assert!(h.machine.take_report().is_some());
        assert_eq!(h.machine.outcome().phase, Phase::TerminationRequested);
        h.turn(2501);
        h.turn(3000);
        assert_eq!(
            h.writer.bytes,
            encode(&msg(Body::Render(render()))).unwrap()
        );
        assert_eq!(h.writer.calls, 1);
    }
}
#[test]
fn initialization_dispatch_fact_changes_only_after_the_last_short_write() {
    let mut h = Harness::new();
    h.writer.limit = 7;
    for _ in 0..4 {
        h.turn(0);
        assert!(!h.machine.outcome().initialize_transmitted);
    }
    assert_eq!(h.outbound.pending(), 1);
    h.turn(0);
    assert!(h.machine.outcome().initialize_transmitted);
    h.machine.receive(msg(Body::Ready), at(1));
    assert_eq!(h.machine.outcome().phase, Phase::Ready);
    assert_eq!(h.machine.outcome().failure, None);
    assert_eq!(messages(&h.writer.bytes), [msg(Body::Initialize)]);
}

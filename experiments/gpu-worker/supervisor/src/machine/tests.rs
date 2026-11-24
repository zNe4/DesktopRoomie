use super::*;
use roomie_worker_protocol::{Action, Facing, FRAME_BYTES};
fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}
fn job(serial: u64) -> JobKey {
    JobKey {
        epoch: 1,
        serial,
        generation: 8,
        sequence: serial,
    }
}
fn render(serial: u64) -> Render {
    Render {
        job: job(serial),
        action: Action::Idle,
        facing: Facing::Neutral,
        elapsed: at(600),
        delta: at(10),
    }
}
fn msg(body: Body) -> Message {
    Message { session: 7, body }
}
fn dispatched(m: &mut Machine, render: Render, now: Duration) -> Result<(), Admission> {
    m.admit(render, now)?;
    m.render_transmitted(render.job, true);
    Ok(())
}
fn ready() -> Machine {
    let mut m = Machine::new(7, Policy::default(), at(0));
    m.initialize_transmitted();
    m.receive(msg(Body::Ready), at(0));
    m
}
fn frame(serial: u64) -> Body {
    Body::Frame {
        job: job(serial),
        bgra: vec![0; FRAME_BYTES],
    }
}
#[test]
fn defaults_startup_and_idle_have_only_active_deadlines() {
    let mut m = Machine::new(7, Policy::default(), at(9));
    assert_eq!(m.outcome().phase, Phase::Starting);
    assert_eq!(m.deadline(), Some(at(5009)));
    assert_eq!(m.admit(render(1), at(9)), Err(Admission::NotReady));
    m.initialize_transmitted();
    m.receive(msg(Body::Ready), at(10));
    assert_eq!(m.deadline(), None);
    assert_eq!(m.outcome().phase, Phase::Ready);
    for time in [11, 100, 5000, 100000] {
        m.tick(at(time));
        assert_eq!(m.deadline(), None);
    }
}
#[test]
fn one_job_and_one_result_are_exclusive() {
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    assert_eq!(m.admit(render(2), at(1)), Err(Admission::Outstanding));
    m.receive(msg(frame(1)), at(2));
    assert_eq!(m.admit(render(2), at(2)), Err(Admission::ResultPending));
    assert!(matches!(m.take_report(), Some(Report::Frame { job: key, .. }) if key == job(1)));
    assert!(m.take_report().is_none());
    assert_eq!(m.admit(render(1), at(3)), Err(Admission::Identity));
    dispatched(&mut m, render(2), at(3)).unwrap();
}
#[test]
fn all_job_fields_and_session_must_correlate() {
    for field in 0..5 {
        let mut m = ready();
        dispatched(&mut m, render(1), at(0)).unwrap();
        let mut key = job(1);
        let mut session = 7;
        match field {
            0 => session += 1,
            1 => key.epoch += 1,
            2 => key.serial += 1,
            3 => key.generation += 1,
            _ => key.sequence += 1,
        }
        m.receive(
            Message {
                session,
                body: Body::JobFailed {
                    job: key,
                    diagnostic: "fault".into(),
                },
            },
            at(1),
        );
        assert_eq!(m.outcome().failure, Some(Failure::Correlation));
        assert_eq!(m.outcome().unresolved, Some(job(1)));
        assert!(m.take_report().is_none());
    }
}
#[test]
fn duplicate_and_unsolicited_reports_close_the_session() {
    let mut m = ready();
    m.receive(msg(frame(1)), at(0));
    assert_eq!(m.outcome().failure, Some(Failure::Correlation));
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    m.receive(msg(frame(1)), at(1));
    m.receive(msg(frame(1)), at(1));
    assert_eq!(m.outcome().failure, Some(Failure::Correlation));
    assert_eq!(m.take_report(), Some(Report::Obsolete(job(1))));
}
#[test]
fn cancellation_and_freshness_expiry_preserve_debt_until_late_settlement() {
    for cancel in [false, true] {
        let mut m = ready();
        dispatched(&mut m, render(1), at(0)).unwrap();
        if cancel {
            m.cancel();
        } else {
            m.tick(at(100));
        }
        assert_eq!(m.deadline(), Some(at(2500)));
        assert_eq!(m.admit(render(2), at(100)), Err(Admission::Outstanding));
        m.receive(msg(frame(1)), at(101));
        assert_eq!(m.take_report(), Some(Report::Obsolete(job(1))));
        assert_eq!(m.outcome().unresolved, None);
        dispatched(&mut m, render(2), at(102)).unwrap();
    }
}
#[test]
fn frame_observed_at_deadline_is_obsolete_without_a_prior_timer_tick() {
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    m.receive(msg(frame(1)), at(100));
    assert_eq!(m.take_report(), Some(Report::Obsolete(job(1))));
}
#[test]
fn pending_ready_candidate_also_expires_behind_a_partial_next_message() {
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    m.receive(msg(frame(1)), at(1));
    m.tick(at(100));
    assert_eq!(m.take_report(), Some(Report::Obsolete(job(1))));
}
#[test]
fn rejection_and_failed_job_are_settled_but_unresolved_is_terminal() {
    for body in [
        Body::RejectedBeforeSubmission {
            job: job(1),
            diagnostic: "r".into(),
        },
        Body::JobFailed {
            job: job(1),
            diagnostic: "f".into(),
        },
    ] {
        let mut m = ready();
        dispatched(&mut m, render(1), at(0)).unwrap();
        m.receive(msg(body), at(1));
        assert_eq!(m.outcome().unresolved, None);
        assert!(m.take_report().is_some());
        dispatched(&mut m, render(2), at(2)).unwrap();
    }
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    m.receive(
        msg(Body::Unresolved {
            job: job(1),
            diagnostic: "unresolved".into(),
        }),
        at(1),
    );
    assert_eq!(m.outcome().phase, Phase::Closing);
    assert_eq!(m.outcome().failure, Some(Failure::Unresolved));
    assert_eq!(m.outcome().unresolved, Some(job(1)));
    assert_eq!(m.admit(render(2), at(2)), Err(Admission::Outstanding));
    m.receive(
        msg(Body::JobFailed {
            job: job(1),
            diagnostic: "late settlement".into(),
        }),
        at(3),
    );
    assert_eq!(m.outcome().unresolved, None);
    m.take_report();
    assert_eq!(m.admit(render(2), at(3)), Err(Admission::NotReady));
}
#[test]
fn backend_fault_has_priority_over_frame_in_both_orders_even_when_stale() {
    for now in [at(2), at(100)] {
        for fault_first in [false, true] {
            let mut m = ready();
            dispatched(&mut m, render(1), at(0)).unwrap();
            let fault = msg(Body::BackendFault("device lost".into()));
            if fault_first {
                m.receive(fault.clone(), now);
            }
            m.receive(msg(frame(1)), now);
            if !fault_first {
                m.receive(fault, now);
            }
            m.transport_failed(Failure::Disconnected, now);
            m.exited(false, now);
            m.tick(now);
            assert_eq!(m.outcome().failure, Some(Failure::BackendFault));
            assert_eq!(m.outcome().unresolved, None);
            assert_eq!(m.take_report(), Some(Report::Obsolete(job(1))));
        }
    }
}
#[test]
fn wrong_session_backend_fault_cannot_masquerade_as_current_device_failure() {
    let mut m = ready();
    m.receive(
        Message {
            session: 9,
            body: Body::BackendFault("old device".into()),
        },
        at(0),
    );
    assert_eq!(m.outcome().failure, Some(Failure::Correlation));
    assert_eq!(m.diagnostic(), None);
}
#[test]
fn initialization_timeout_and_boundary_ready_do_not_reopen() {
    for with_ready in [true, false] {
        let mut m = Machine::new(7, Policy::default(), at(0));
        if with_ready {
            m.initialize_transmitted();
            m.receive(msg(Body::Ready), at(5000));
        } else {
            m.tick(at(5000));
        }
        assert_eq!(m.outcome().failure, Some(Failure::InitializationTimeout));
        assert_eq!(m.outcome().phase, Phase::Closing);
        assert_eq!(m.deadline(), Some(at(5500)));
        assert_eq!(m.admit(render(1), at(5000)), Err(Admission::NotReady));
    }
}
#[test]
fn escalation_termination_reaping_never_discharge_settlement_debt() {
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    m.tick(at(2500));
    assert_eq!(m.outcome().phase, Phase::TerminationRequested);
    assert_eq!(m.deadline(), Some(at(3000)));
    m.tick(at(3000));
    assert_eq!(m.outcome().phase, Phase::ShutdownUnconfirmed);
    assert_eq!(m.deadline(), None);
    m.exited(false, at(3001));
    assert_eq!(m.outcome().phase, Phase::Exited);
    assert_eq!(m.outcome().session, 7);
    assert!(m.outcome().reaped);
    assert_eq!(m.outcome().shutdown, Shutdown::Unconfirmed);
    assert_eq!(m.outcome().unresolved, Some(job(1)));
}
#[test]
fn graceful_shutdown_requires_stopped_and_reaping() {
    let mut m = ready();
    m.close(at(10));
    assert_eq!(m.outcome().phase, Phase::Closing);
    m.receive(msg(Body::Stopped), at(11));
    assert_eq!(m.outcome().shutdown, Shutdown::Pending);
    m.exited(true, at(12));
    assert_eq!(m.outcome().shutdown, Shutdown::Confirmed);
    assert_eq!(m.outcome().failure, None);
}
#[test]
fn stopped_cannot_settle_an_outstanding_job_or_certify_a_hung_exit() {
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    m.close(at(1));
    m.receive(msg(Body::Stopped), at(2));
    assert_eq!(m.outcome().failure, Some(Failure::Protocol));
    assert!(!m.outcome().stopped);
    assert_eq!(m.outcome().unresolved, Some(job(1)));
    let mut m = ready();
    m.close(at(0));
    m.receive(msg(Body::Stopped), at(1));
    m.tick(at(500));
    m.exited(false, at(501));
    assert_eq!(m.outcome().shutdown, Shutdown::Unconfirmed);
}
#[test]
fn repeated_shutdown_and_activity_do_not_renew_deadlines() {
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    m.close(at(10));
    assert_eq!(m.deadline(), Some(at(510)));
    for now in [20, 100, 400, 509] {
        m.close(at(now));
        assert_eq!(m.deadline(), Some(at(510)));
    }
    m.tick(at(510));
    assert_eq!(m.deadline(), Some(at(1010)));
    m.close(at(1009));
    assert_eq!(m.deadline(), Some(at(1010)));
}
#[test]
fn terminal_report_precedes_escalation_but_never_restores_freshness() {
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    m.receive(msg(frame(1)), at(2500));
    m.tick(at(2500));
    assert_eq!(m.take_report(), Some(Report::Obsolete(job(1))));
    assert!(!m.outcome().termination_requested);
    assert_eq!(m.outcome().failure, None);
}
#[test]
fn unexpected_exit_is_distinct_from_gpu_settlement() {
    let mut m = ready();
    dispatched(&mut m, render(1), at(0)).unwrap();
    m.exited(true, at(1));
    assert_eq!(m.outcome().failure, Some(Failure::UnexpectedExit));
    assert_eq!(m.outcome().unresolved, Some(job(1)));
    assert_eq!(m.outcome().shutdown, Shutdown::Unconfirmed);
}
#[test]
fn policy_is_injectable_and_regressing_clock_fails_closed() {
    let policy = Policy {
        initialization: at(1),
        freshness: at(2),
        escalation: at(3),
        graceful_shutdown: at(4),
        exit_observation: at(5),
    };
    let mut m = Machine::new(7, policy, at(0));
    m.initialize_transmitted();
    m.receive(msg(Body::Ready), at(0));
    dispatched(&mut m, render(1), at(10)).unwrap();
    assert_eq!(m.deadline(), Some(at(12)));
    m.tick(at(12));
    assert_eq!(m.deadline(), Some(at(13)));
    m.tick(at(13));
    assert_eq!(m.deadline(), Some(at(18)));
    m.tick(at(18));
    assert_eq!(m.outcome().phase, Phase::ShutdownUnconfirmed);
    let mut m = ready();
    m.tick(at(20));
    m.tick(at(19));
    assert_eq!(m.outcome().failure, Some(Failure::Clock));
}
#[test]
fn diagnostic_retention_is_utf8_bounded_and_not_an_activity_timer() {
    let mut m = ready();
    m.receive(msg(Body::BackendFault("é".repeat(1000))), at(0));
    assert_eq!(m.diagnostic().unwrap().len(), 512);
    let deadline = m.deadline();
    m.receive(msg(Body::BackendFault("later".into())), at(1));
    assert_eq!(m.deadline(), deadline);
    assert_eq!(m.diagnostic().unwrap().len(), 512);
}

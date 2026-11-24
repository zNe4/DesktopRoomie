use crate::{
    diagnostic,
    engine::Backend,
    phase,
    settlement::{Evidence, Operation},
};
use roomie_worker_protocol::{Action, Body, Facing, Message, Transmitter};
use std::collections::VecDeque;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    AwaitInitialize,
    Ready,
    Faulted,
    Closing,
    Released,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Controlled,
    Failed,
    Uncertain,
}

/// Single protocol writer and admission owner. Two bounded response slots are
/// enough for BackendFault followed by the associated correlated report.
pub struct Worker<B: Backend> {
    backend: B,
    phase: Phase,
    session: Option<u64>,
    serial: u64,
    active: Option<Operation>,
    responses: VecDeque<Body>,
    transmitter: Option<Transmitter>,
    started: bool,
    transport_abandoned: bool,
    fault_sent: bool,
    exit: Option<Exit>,
}
impl<B: Backend> Worker<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            phase: Phase::AwaitInitialize,
            session: None,
            serial: 0,
            active: None,
            responses: VecDeque::with_capacity(2),
            transmitter: None,
            started: false,
            transport_abandoned: false,
            fault_sent: false,
            exit: None,
        }
    }
    pub fn backend(&self) -> &B {
        &self.backend
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn backend_faulted(&self) -> bool {
        self.fault_sent
    }
    fn proof(&self, operation: Operation, supplied: Option<Evidence>) -> Option<Evidence> {
        let evidence = supplied?;
        (evidence.operation == operation
            && self.backend.evidence() == Some(evidence)
            && evidence.settled()
            && self.backend.settled())
        .then_some(evidence)
    }
    fn failure(&mut self, operation: Operation, text: &str, evidence: Option<Evidence>) {
        self.fault_report(text);
        if self.proof(operation, evidence).is_some() {
            self.push(Body::JobFailed {
                job: operation.job,
                diagnostic: diagnostic(text),
            });
        } else {
            self.push(Body::Unresolved {
                job: operation.job,
                diagnostic: diagnostic(text),
            });
            self.exit = Some(Exit::Uncertain);
        }
    }
    fn reject(&mut self, operation: Operation, text: &str) {
        let evidence = self.backend.reject(operation);
        if self
            .proof(operation, evidence)
            .is_some_and(Evidence::rejected_before_submission)
        {
            self.push(Body::RejectedBeforeSubmission {
                job: operation.job,
                diagnostic: diagnostic(text),
            });
        } else {
            self.failure(
                operation,
                "pre-submission rejection lacks correlated settlement evidence",
                None,
            );
        }
    }
    pub fn pending(&self) -> bool {
        !self.transport_abandoned && !self.responses.is_empty()
    }
    pub fn transport_abandoned(&self) -> bool {
        self.transport_abandoned
    }
    pub fn outcome(&self) -> Option<Exit> {
        if self.pending() {
            None
        } else {
            self.exit
        }
    }
    fn push(&mut self, body: Body) {
        assert!(self.responses.len() < 2);
        self.responses.push_back(body);
    }
    fn fault_report(&mut self, text: &str) {
        self.phase = Phase::Faulted;
        if !self.fault_sent {
            self.push(Body::BackendFault(diagnostic(text)));
            self.fault_sent = true;
        }
    }
    /// Used before Ready, admission, Frame, Stopped and each finite idle wait.
    pub fn observe(&mut self) {
        if self.transport_abandoned {
            return;
        }
        let fault = self.backend.fault();
        let invalid = self.active.is_some_and(|op| match self.responses.front() {
            Some(Body::Frame { .. }) => !self
                .proof(op, self.backend.evidence())
                .is_some_and(Evidence::frame_ready),
            Some(Body::JobFailed { .. }) => self.proof(op, self.backend.evidence()).is_none(),
            Some(Body::RejectedBeforeSubmission { .. }) => !self
                .proof(op, self.backend.evidence())
                .is_some_and(Evidence::rejected_before_submission),
            _ => false,
        });
        let invalid_stop = (matches!(self.responses.front(), Some(Body::Stopped))
            || self.exit == Some(Exit::Controlled))
            && !self.backend.settled();
        if self.session.is_none() {
            return;
        }
        if invalid || invalid_stop {
            self.exit = Some(Exit::Uncertain);
            if self.started {
                // A prefix cannot be retracted or replaced. Completing it
                // would certify invalid settlement: abandon ALL wire writes.
                self.transport_abandoned = true;
                self.phase = Phase::Faulted;
                return;
            }
        }
        let text = fault.or_else(|| {
            (invalid || invalid_stop)
                .then(|| "terminal response settlement proof invalidated".into())
        });
        let Some(text) = text else {
            return;
        };
        if self.started {
            if !self.fault_sent {
                self.fault_report(&text);
            }
            // Ordinary faults leave the established physical proof intact.
            // Finish that truthful message before attempting fault delivery.
            return;
        }
        if self.fault_sent && !invalid && !invalid_stop {
            return;
        }
        let old = self.responses.pop_front();
        self.transmitter = None;
        self.fault_report(&text);
        match old {
            Some(body @ Body::RejectedBeforeSubmission { .. }) if !invalid => self.push(body),
            Some(Body::Frame { .. })
            | Some(Body::JobFailed { .. })
            | Some(Body::RejectedBeforeSubmission { .. }) => {
                let operation = self.active.expect("job response owns its operation");
                if invalid {
                    self.push(Body::Unresolved {
                        job: operation.job,
                        diagnostic: diagnostic(&text),
                    });
                } else if self.proof(operation, self.backend.evidence()).is_some() {
                    self.push(Body::JobFailed {
                        job: operation.job,
                        diagnostic: diagnostic(&text),
                    });
                } else {
                    self.push(Body::Unresolved {
                        job: operation.job,
                        diagnostic: diagnostic(&text),
                    });
                    self.exit = Some(Exit::Uncertain);
                }
            }
            Some(Body::Ready) | None => {}
            Some(Body::Stopped) if invalid_stop => {}
            Some(body) => self.push(body),
        }
    }
    /// Native transport does not read another command until every response
    /// byte is transmitted. Direct callers receive a bounded ordering error.
    pub fn receive(&mut self, message: Message) -> Result<(), &'static str> {
        if self.pending() || self.exit.is_some() {
            return Err("response incomplete or worker closed");
        }
        if let Some(session) = self.session {
            if session != message.session {
                return Err("session mismatch");
            }
        }
        match message.body {
            Body::Initialize if self.phase == Phase::AwaitInitialize => {
                self.session = Some(message.session);
                match self.backend.initialize() {
                    Ok(()) => {
                        self.phase = Phase::Ready;
                        self.push(Body::Ready);
                        self.observe();
                    }
                    Err(text) => self.fault_report(&text),
                }
            }
            Body::Render(render) if matches!(self.phase, Phase::Ready | Phase::Faulted) => {
                if render.job.serial == 0
                    || self
                        .serial
                        .checked_add(1)
                        .is_none_or(|next| render.job.serial < next)
                    || render.delta > render.elapsed
                {
                    return Err("invalid job ordering or time");
                }
                self.serial = render.job.serial;
                let operation = Operation {
                    session: message.session,
                    job: render.job,
                    phase: phase(render.elapsed),
                };
                self.active = Some(operation);
                // Backend-wide fault wins even when an intent is unsupported.
                self.observe();
                if self.phase == Phase::Faulted {
                    self.reject(operation, "backend admission closed before submission");
                } else if render.action != Action::Idle || render.facing != Facing::Neutral {
                    self.reject(operation, "only Idle/Neutral is supported");
                } else {
                    match self.backend.render(operation) {
                        Ok(prepared) => {
                            let valid = self.proof(operation, Some(prepared.evidence));
                            if prepared.bgra.len() != roomie_worker_protocol::FRAME_BYTES {
                                self.failure(
                                    operation,
                                    "backend returned invalid frame size",
                                    valid,
                                );
                            } else if !valid.is_some_and(Evidence::frame_ready) {
                                self.failure(
                                    operation,
                                    "successful backend result lacks correlated frame settlement",
                                    None,
                                );
                            } else {
                                self.push(Body::Frame {
                                    job: render.job,
                                    bgra: prepared.bgra,
                                });
                                self.observe();
                            }
                        }
                        Err(failure) => {
                            let text = self.backend.fault().unwrap_or(failure.diagnostic);
                            self.failure(operation, &text, Some(failure.evidence));
                        }
                    }
                }
            }

            Body::Shutdown
                if self.session.is_some()
                    && matches!(self.phase, Phase::Ready | Phase::Faulted) =>
            {
                self.phase = Phase::Closing;
                self.observe();
                if self.backend.release() {
                    self.phase = Phase::Released;
                    self.observe();
                    self.push(Body::Stopped);
                    self.exit = Some(Exit::Controlled);
                } else {
                    if !self.fault_sent {
                        self.fault_report("uncertain backend teardown");
                    }
                    self.exit = Some(Exit::Uncertain);
                }
            }
            _ => return Err("unexpected request"),
        }
        Ok(())
    }
    /// One bounded syscall. An unsent terminal can be replaced. A prefix may
    /// finish only while its settlement remains valid; otherwise close the
    /// transport without splicing another message into it.
    pub fn write(&mut self, writer: &mut impl Write) -> io::Result<()> {
        self.observe();
        if self.transport_abandoned {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let Some(body) = self.responses.front() else {
            return Ok(());
        };
        if self.transmitter.is_none() {
            let mut tx = Transmitter::default();
            tx.queue(&Message {
                session: self.session.expect("correlated response"),
                body: body.clone(),
            })
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
            self.transmitter = Some(tx);
        }
        let tx = self.transmitter.as_mut().unwrap();
        let before = tx.pending();
        tx.write(writer)?;
        self.started |= tx.pending() < before;
        if tx.pending() == 0 {
            self.responses.pop_front();
            self.transmitter = None;
            self.started = false;
            if !self.pending() {
                self.active = None;
            }
        }
        Ok(())
    }
    pub fn disconnect(&mut self) -> Exit {
        self.phase = Phase::Closing;
        self.observe();
        if self.exit == Some(Exit::Uncertain) {
            return Exit::Uncertain;
        }
        if self.backend.release() {
            Exit::Failed
        } else {
            Exit::Uncertain
        }
    }
}

//! The sole outgoing stream slot and its dispatch facts. Kept separate from
//! process ownership so deterministic writers exercise the native write path.
use crate::machine::{Machine, Phase};
use roomie_worker_protocol::{Body, Error, JobKey, Message, Render, Transmitter};
use std::io::{self, Write};

#[derive(Clone, Copy)]
enum Command {
    Initialize,
    Render(JobKey),
    Shutdown,
}
pub(crate) struct Outbound {
    tx: Transmitter,
    command: Option<Command>,
    total: usize,
}
impl Outbound {
    pub(crate) fn new(session: u64) -> Self {
        let mut out = Self {
            tx: Transmitter::default(),
            command: None,
            total: 0,
        };
        out.queue(session, Body::Initialize, Command::Initialize)
            .expect("fixed Initialize");
        out
    }
    fn queue(&mut self, session: u64, body: Body, command: Command) -> Result<(), Error> {
        self.tx.queue(&Message { session, body })?;
        self.total = self.tx.pending();
        self.command = Some(command);
        Ok(())
    }
    pub(crate) fn pending(&self) -> usize {
        self.tx.pending()
    }
    fn written(&self) -> usize {
        self.total - self.pending()
    }
    pub(crate) fn render(&mut self, session: u64, render: Render) -> Result<(), Error> {
        self.queue(session, Body::Render(render), Command::Render(render.job))
    }
    /// Withdraw the whole message only with a zero-byte proof from the sole
    /// transmitter. Never use channel loss, a deadline or process death as proof.
    fn reject_unsent(&mut self, machine: &mut Machine) {
        if !matches!(
            machine.outcome().phase,
            Phase::Starting | Phase::Ready | Phase::Outstanding
        ) {
            if let Some(Command::Render(job)) = self.command {
                if self.written() == 0 && machine.reject_before_dispatch(job) {
                    self.tx = Transmitter::default();
                    self.command = None;
                    self.total = 0;
                }
            }
        }
    }
    pub(crate) fn writable(&self, machine: &Machine) -> bool {
        if self.pending() == 0 {
            return false;
        }
        match machine.outcome().phase {
            Phase::TerminationRequested | Phase::ShutdownUnconfirmed | Phase::Exited => false,
            Phase::Closing if matches!(self.command, Some(Command::Render(_))) => {
                self.written() != 0
            }
            _ => true,
        }
    }
    pub(crate) fn prepare(&mut self, machine: &mut Machine) {
        self.reject_unsent(machine);
        if machine.needs_shutdown() && self.pending() == 0 {
            self.queue(machine.session(), Body::Shutdown, Command::Shutdown)
                .expect("fixed Shutdown");
            machine.sent_shutdown();
        }
    }
    /// At most one write, after the caller has processed messages and deadlines.
    /// All command kinds stop at termination; Closing may finish an existing
    /// Render prefix, but may never begin a fresh Render.
    pub(crate) fn progress(
        &mut self,
        machine: &mut Machine,
        writable: bool,
        writer: &mut impl Write,
    ) -> io::Result<()> {
        self.prepare(machine);
        if writable && self.writable(machine) {
            let before = self.pending();
            self.tx.write(writer)?;
            if self.pending() < before {
                let complete = self.pending() == 0;
                match self.command.expect("pending command") {
                    Command::Initialize if complete => machine.initialize_transmitted(),
                    Command::Render(job) => machine.render_transmitted(job, complete),
                    _ => {}
                }
                if complete {
                    self.command = None;
                    self.total = 0;
                }
            }
        }
        // After finishing a partial Render, queue Shutdown behind its final
        // byte without granting another write or renewing the close deadline.
        self.prepare(machine);
        Ok(())
    }
}

#[cfg(test)]
mod tests;

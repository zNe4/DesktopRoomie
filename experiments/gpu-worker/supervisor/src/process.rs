use std::io::{self, Read};
use std::marker::PhantomData;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::rc::Rc;
use std::time::Duration;

use crate::machine::{Admission, Failure, Machine, Outcome, Policy, Report};
use crate::outbound::Outbound;
use roomie_worker_protocol::{Body, Decoder, Message, Render};

// A turn has a fixed syscall budget even under continuous malicious traffic.
const READ_BUDGET: usize = 64;
#[derive(Debug, Default, Clone, Copy)]
pub struct Readiness {
    pub readable: bool,
    pub writable: bool,
    pub hangup: bool,
    pub error: bool,
    pub exited: bool,
}
#[derive(Debug)]
pub struct WaitInterest {
    pub socket: Option<RawFd>,
    pub process: Option<RawFd>,
    pub writable: bool,
    pub deadline: Option<Duration>,
}
#[derive(Debug)]
pub struct Progress {
    /// The bounded read budget ran out: service another turn without sleeping.
    pub continue_immediately: bool,
    pub drained: bool,
}

/// One session and child. Deliberately !Send/!Sync: Linux PDEATHSIG tracks the
/// spawning thread, whose lifetime must encompass supervision and child exit.
/// Keep this value after unconfirmed shutdown to observe and reap a later exit.
pub struct Supervisor {
    child: Child,
    observed_exit: Option<ExitStatus>,
    final_exit: Option<ExitStatus>,
    pidfd: Option<OwnedFd>,
    socket: UnixStream,
    channel_open: bool,
    decoder: Decoder,
    outbound: Outbound,
    machine: Machine,
    drained: bool,
    kill_attempted: bool,
    kill_error: Option<i32>,
    pending_io_error: bool,
    _spawning_thread: PhantomData<Rc<()>>,
}
impl Supervisor {
    pub fn spawn(
        executable: &Path,
        args: &[&str],
        session: u64,
        policy: Policy,
        now: Duration,
    ) -> io::Result<Self> {
        let executable = executable.canonicalize()?;
        let (socket, child_socket) = UnixStream::pair()?;
        socket.set_nonblocking(true)?;
        // std's socket pair is CLOEXEC. Only the child's endpoint is explicitly
        // transferred as fd 0. stdout/stderr are /dev/null, never protocol pipes.
        let child_fd: OwnedFd = child_socket.into();
        let mut command = Command::new(executable);
        command
            .args(args)
            .env_clear()
            .stdin(Stdio::from(child_fd))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: pre_exec performs only async-signal-safe Linux syscalls,
        // accesses captured integers, and constructs errno-only errors.
        let parent = unsafe { libc::getpid() };
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(io::Error::last_os_error());
                }
                // Close the fork/prctl race, including death before prctl.
                if libc::getppid() != parent {
                    libc::_exit(125);
                }
                // Mark *all* unrelated owner descriptors CLOEXEC, including
                // descriptors whose owner omitted CLOEXEC. Do not close the
                // Rust exec-error pipe before exec has had a chance to fail.
                if libc::syscall(
                    libc::SYS_close_range,
                    3_u32,
                    u32::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                ) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        // SAFETY: pidfd_open takes a live child pid and zero flags; ownership of
        // a successful returned descriptor is transferred once to OwnedFd.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id(), 0_u32) };
        let pidfd = if fd >= 0 {
            Some(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
        } else {
            None
        };
        let mut machine = Machine::new(session, policy, now);
        if pidfd.is_none() {
            machine.transport_failed(Failure::ProcessObservation, now);
        }
        let outbound = Outbound::new(session);
        Ok(Self {
            child,
            observed_exit: None,
            final_exit: None,
            pidfd,
            socket,
            channel_open: true,
            decoder: Decoder::default(),
            outbound,
            machine,
            drained: false,
            kill_attempted: false,
            kill_error: None,
            pending_io_error: false,
            _spawning_thread: PhantomData,
        })
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub fn machine(&self) -> &Machine {
        &self.machine
    }
    pub fn outcome(&self) -> Outcome {
        self.machine.outcome()
    }
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.final_exit
    }
    pub fn termination_errno(&self) -> Option<i32> {
        self.kill_error
    }
    pub fn interest(&self) -> WaitInterest {
        WaitInterest {
            socket: self.channel_open.then(|| self.socket.as_raw_fd()),
            process: if self.outcome().reaped {
                None
            } else {
                self.pidfd.as_ref().map(AsRawFd::as_raw_fd)
            },
            writable: self.channel_open && self.outbound.writable(&self.machine),
            deadline: self.machine.deadline(),
        }
    }
    pub fn submit(&mut self, render: Render, now: Duration) -> Result<(), Admission> {
        if self.outcome().unresolved.is_some() {
            return Err(Admission::Outstanding);
        }
        if !self.channel_open || self.outbound.pending() != 0 {
            return Err(Admission::Backpressure);
        }
        // Validate serialization before acquiring the physical obligation.
        let message = Message {
            session: self.machine.session(),
            body: Body::Render(render),
        };
        roomie_worker_protocol::encode(&message).map_err(|_| Admission::Identity)?;
        self.machine.admit(render, now)?;
        self.outbound
            .render(self.machine.session(), render)
            .expect("empty prevalidated transmit slot");
        self.drained = false;
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.machine.cancel();
    }
    pub fn shutdown(&mut self, now: Duration) {
        self.machine.close(now);
        // Make graceful-close work visible to readiness without waiting for a
        // deadline wakeup. This only reconciles/queues; it never writes.
        self.outbound.prepare(&mut self.machine);
    }
    /// Consume in the owner turn immediately after a drained progression. No
    /// observation barrier can promise absence of a future kernel arrival.
    pub fn take_report(&mut self) -> Option<Report> {
        if self.drained {
            self.machine.take_report()
        } else {
            None
        }
    }
    fn drain(&mut self, now: Duration) -> bool {
        let mut exhausted = false;
        if self.channel_open {
            self.drained = false;
            let mut stopped_reading = false;
            for _ in 0..READ_BUDGET {
                let mut buffer = [0; 8192];
                let limit = buffer.len().min(self.decoder.remaining());
                match self.socket.read(&mut buffer[..limit]) {
                    Ok(0) => {
                        let failure = if self.decoder.eof().is_err() {
                            Some(Failure::Protocol)
                        } else if !self.outcome().stopped {
                            Some(Failure::Disconnected)
                        } else {
                            None
                        };
                        if let Some(failure) = failure {
                            self.machine.transport_failed(failure, now);
                        }
                        self.channel_open = false;
                        self.drained = true;
                        stopped_reading = true;
                        break;
                    }
                    Ok(count) => {
                        let result = self
                            .decoder
                            .feed(&buffer[..count])
                            .and_then(|_| self.decoder.take());
                        match result {
                            Ok(Some(message)) => self.machine.receive(message, now),
                            Ok(None) => {}
                            Err(_) => {
                                self.machine.transport_failed(Failure::Protocol, now);
                                self.channel_open = false;
                                self.drained = true;
                                stopped_reading = true;
                                break;
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        self.drained = self.decoder.buffered() == 0;
                        stopped_reading = true;
                        break;
                    }
                    Err(_) => {
                        self.machine.transport_failed(Failure::Transport, now);
                        self.channel_open = false;
                        self.drained = true;
                        stopped_reading = true;
                        break;
                    }
                }
            }
            exhausted = !stopped_reading;
            // Drain readable bytes first, even when error/hangup is set.
            if self.pending_io_error && stopped_reading {
                self.pending_io_error = false;
                self.machine.transport_failed(Failure::Transport, now);
                self.channel_open = false;
                self.drained = true;
            }
        }
        exhausted
    }
    pub fn progress(&mut self, ready: Readiness, now: Duration) -> Progress {
        self.pending_io_error |= ready.error;
        let mut exhausted = self.drain(now);
        // try_wait can observe an exit that raced the first WouldBlock.
        // Drain again after that observation before classifying the status.
        if !exhausted && !self.outcome().reaped {
            if self.observed_exit.is_none() {
                match self.child.try_wait() {
                    Ok(status) => self.observed_exit = status,
                    Err(_) => self
                        .machine
                        .transport_failed(Failure::ProcessObservation, now),
                }
            }
            if self.observed_exit.is_some() {
                exhausted = self.drain(now);
                if !exhausted {
                    let status = self.observed_exit.take().expect("observed exit");
                    self.final_exit = Some(status);
                    self.machine.exited(status.success(), now);
                    self.channel_open = false;
                    self.drained = true;
                }
            }
        }
        self.machine.tick(now);
        // Reconcile zero-byte rejection even after channel closure/exit. The
        // native writer is callable only while the channel and phase permit it.
        if self
            .outbound
            .progress(
                &mut self.machine,
                self.channel_open && ready.writable,
                &mut self.socket,
            )
            .is_err()
        {
            self.machine.transport_failed(Failure::Transport, now);
            self.channel_open = false;
        }
        if self.outcome().termination_requested && !self.kill_attempted && !self.outcome().reaped {
            self.kill_attempted = true;
            if let Err(error) = self.child.kill() {
                self.kill_error = error.raw_os_error();
            }
        }
        Progress {
            continue_immediately: exhausted,
            drained: self.drained,
        }
    }
    /// A finite convenience poll for headless harnesses. The future owner can
    /// incorporate `interest()` in its own poll set instead. No internal clock.
    pub fn poll_ready(&self, max_wait: Duration, now: Duration) -> io::Result<Readiness> {
        let interest = self.interest();
        let wait = interest
            .deadline
            .map_or(max_wait, |d| max_wait.min(d.saturating_sub(now)));
        let timeout = wait.as_nanos().div_ceil(1_000_000).min(i32::MAX as u128) as i32;
        let mut fds = [
            libc::pollfd {
                fd: interest.socket.unwrap_or(-1),
                events: libc::POLLIN | if interest.writable { libc::POLLOUT } else { 0 },
                revents: 0,
            },
            libc::pollfd {
                fd: interest.process.unwrap_or(-1),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: both entries remain live for the call; poll writes only revents.
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
        Ok(Readiness {
            readable: fds[0].revents & libc::POLLIN != 0,
            writable: fds[0].revents & libc::POLLOUT != 0,
            hangup: fds[0].revents & libc::POLLHUP != 0,
            error: fds[0].revents & (libc::POLLERR | libc::POLLNVAL) != 0,
            exited: fds[1].revents != 0,
        })
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        // Best effort only. Drop cannot certify shutdown or reap a child that
        // has not exited. Explicit progression through exit is required.
        if !self.outcome().reaped {
            if !self.kill_attempted {
                let _ = self.child.kill();
            }
            let _ = self.child.try_wait();
        }
    }
}

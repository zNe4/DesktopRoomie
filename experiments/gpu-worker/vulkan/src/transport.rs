//! Worker-local low-frequency fault observation, including during backpressure.
use crate::{
    engine::Backend,
    worker::{Exit, Worker},
};
use roomie_worker_protocol::{read_step, Decoder};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

pub const FAULT_OBSERVATION_MS: i32 = 100;
pub fn serve<B: Backend>(socket: &mut UnixStream, worker: &mut Worker<B>) -> Exit {
    let exit = drive(socket, worker);
    // Close even when abandonment was detected by the final write/disconnect
    // barrier on an error path. The caller may still own this borrowed stream.
    if worker.transport_abandoned() {
        let _ = socket.shutdown(std::net::Shutdown::Both);
    }
    exit
}
fn drive<B: Backend>(socket: &mut UnixStream, worker: &mut Worker<B>) -> Exit {
    if socket.set_nonblocking(true).is_err() {
        return worker.disconnect();
    }
    let mut decoder = Decoder::default();
    loop {
        worker.observe();
        if let Some(exit) = worker.outcome() {
            return exit;
        }
        let mut fd = libc::pollfd {
            fd: socket.as_raw_fd(),
            events: if worker.pending() {
                libc::POLLOUT
            } else {
                libc::POLLIN
            },
            revents: 0,
        };
        // SAFETY: one live socket, one pollfd, finite timeout. There is no idle
        // write interest or busy loop, even with a partial incoming message.
        let result = unsafe { libc::poll(&mut fd, 1, FAULT_OBSERVATION_MS) };
        if result < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return worker.disconnect();
        }
        worker.observe();
        if worker.transport_abandoned() {
            return Exit::Uncertain;
        }
        if fd.revents & libc::POLLOUT != 0 && worker.pending() && worker.write(socket).is_err() {
            return worker.disconnect();
        }
        if fd.revents & libc::POLLIN != 0 && !worker.pending() {
            match read_step(&mut decoder, socket) {
                Ok(Some(message)) => {
                    if worker.receive(message).is_err() {
                        return worker.disconnect();
                    }
                }
                Ok(None) => {}
                Err(_) => return worker.disconnect(),
            }
        }
        if fd.revents & (libc::POLLERR | libc::POLLNVAL | libc::POLLHUP) != 0 {
            if let Some(exit) = worker.outcome() {
                return exit;
            }
            return worker.disconnect();
        }
    }
}

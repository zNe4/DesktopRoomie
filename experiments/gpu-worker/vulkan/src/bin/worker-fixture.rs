//! GPU-free verification binary. The real worker has no injection switch.
#[path = "../../tests/support.rs"]
mod support;
use roomie_worker_vulkan::{
    transport::serve,
    worker::{Exit, Worker},
};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
fn partial_terminal_loss(path: &str) -> ! {
    use roomie_worker_protocol::{read_step, Body, Decoder};
    use std::io::{self, Write};
    struct Prefix<'a>(&'a mut UnixStream);
    impl Write for Prefix<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.write(&bytes[..bytes.len().min(17)])
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    // SAFETY: the accepted supervisor transferred the private socket at fd 0.
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    let mut decoder = Decoder::default();
    let mut worker = Worker::new(support::Fixture::new(support::Mode::Good));
    for initialize in [true, false] {
        let message = loop {
            if let Some(message) = read_step(&mut decoder, &mut socket).unwrap() {
                break message;
            }
        };
        assert!(if initialize {
            matches!(message.body, Body::Initialize)
        } else {
            matches!(message.body, Body::Render(_))
        });
        worker.receive(message).unwrap();
        if initialize {
            while worker.pending() {
                worker.write(&mut socket).unwrap();
            }
        }
    }
    // Fully decoded Render proves dispatch. One controlled short write proves
    // a terminal prefix exists before loss is injected, without timing races.
    worker.write(&mut Prefix(&mut socket)).unwrap();
    assert!(worker.pending());
    worker
        .backend()
        .faults
        .observe_loss(false, "injected loss after terminal prefix");
    worker.observe();
    assert!(worker.transport_abandoned());
    assert_eq!(worker.outcome(), Some(Exit::Uncertain));
    let outcome = serve(&mut socket, &mut worker);
    assert_eq!(outcome, Exit::Uncertain);
    assert_eq!(
        socket.write(&[0]).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    std::fs::write(path, "Render completely decoded\nterminal prefix written\nsettlement invalidated\ntransport abandoned\nprivate stream closed by serve\nuncertain exit 2\n").unwrap();
    std::process::exit(2);
}
fn main() {
    #[cfg(feature = "native-vulkan")]
    roomie_worker_vulkan::native::install_containment_hook();
    let mode = std::env::args().nth(1).unwrap_or("good".into());
    if mode == "partial-terminal-loss" {
        partial_terminal_loss(&std::env::args().nth(2).expect("test trace path"));
    }
    let Some(mode) = support::Mode::parse(&mode) else {
        std::process::exit(3);
    };
    // SAFETY: same private inherited endpoint contract as the real worker.
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };
    let mut worker = Worker::new(support::Fixture::new(mode));
    let outcome = serve(&mut socket, &mut worker);
    std::process::exit(match outcome {
        Exit::Controlled => 0,
        Exit::Failed => 1,
        Exit::Uncertain => 2,
    });
}

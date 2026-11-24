//! GPU-free process fixture. The harness never links this rendering seam.
use roomie_worker_acceptance::oracle;
use roomie_worker_vulkan::{
    engine::*,
    layout::Layout,
    report::ReportFile,
    settlement::*,
    transport::serve,
    worker::{Exit, Worker},
};
use std::{
    io::{self, Write},
    ops::Deref,
    os::fd::FromRawFd,
    os::unix::net::UnixStream,
    path::Path,
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};
#[path = "../../../vulkan/tests/support.rs"]
mod support;

struct Ops {
    bytes: Vec<u8>,
    callback: Option<(Operation, SyncSender<Completion>)>,
    corrupt: bool,
    stall: bool,
}
struct View<'a>(&'a [u8]);
impl Deref for View<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0
    }
}
impl Operations for Ops {
    type Submission = u32;
    type Commands = u32;
    type View<'a> = View<'a>;
    fn encode(&mut self, phase: u32) -> Result<u32, String> {
        if self.stall {
            loop {
                std::thread::park();
            }
        }
        let frame = oracle::expected(&oracle::request(
            1,
            Duration::from_millis(oracle::SAMPLE_MS[phase as usize]),
        ))?;
        let layout = Layout::fixture().unwrap();
        self.bytes.fill(0xee);
        for (row, packed) in self
            .bytes
            .chunks_mut(layout.pitch)
            .zip(frame.payload.chunks_exact(layout.row))
        {
            row[..layout.row].copy_from_slice(packed);
        }
        if self.corrupt {
            self.bytes[0] = 1;
        }
        Ok(phase)
    }
    fn submit(&mut self, _: u32) -> Result<u32, String> {
        Ok(0xface)
    }
    fn wait(&mut self, submission: &u32, _: Duration) -> Result<(), String> {
        assert_eq!(*submission, 0xface);
        if let Some((operation, sender)) = self.callback.take() {
            sender
                .try_send(Completion {
                    operation,
                    result: Ok(()),
                })
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn map(&mut self, operation: Operation, sender: SyncSender<Completion>) -> Result<(), String> {
        self.callback = Some((operation, sender));
        Ok(())
    }
    fn view(&self) -> View<'_> {
        View(&self.bytes)
    }
    fn unmap(&mut self) -> Result<(), String> {
        Ok(())
    }
}
fn exit(outcome: Exit) -> ! {
    std::process::exit(match outcome {
        Exit::Controlled => 0,
        Exit::Failed => 1,
        Exit::Uncertain => 2,
    })
}
fn run(backend: impl Backend, report: Option<&Path>, fail_exit: bool) -> ! {
    let report = report.and_then(|p| ReportFile::create(p).ok());
    // SAFETY: the actual supervisor transfers one private bidirectional fd 0.
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };
    let mut worker = Worker::new(backend);
    let outcome = serve(&mut socket, &mut worker);
    if let Some(report) = report {
        let _ = report.finish(
            worker.backend().acceptance_telemetry(),
            outcome,
            "synthetic",
        );
    }
    if fail_exit {
        std::process::exit(1);
    }
    exit(outcome)
}
fn partial_loss(report: Option<&Path>) -> ! {
    use roomie_worker_protocol::{read_step, Body, Decoder};
    struct Prefix<'a>(&'a mut UnixStream);
    impl Write for Prefix<'_> {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.write(&b[..b.len().min(17)])
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let _report = report.and_then(|p| ReportFile::create(p).ok());
    // SAFETY: supervisor owns and transfers the private endpoint once.
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut worker = Worker::new(support::Fixture::new(support::Mode::Good));
    let mut decoder = Decoder::default();
    for initial in [true, false] {
        let message = loop {
            if let Some(m) = read_step(&mut decoder, &mut socket).unwrap() {
                break m;
            }
        };
        assert!(if initial {
            matches!(message.body, Body::Initialize)
        } else {
            matches!(message.body, Body::Render(_))
        });
        worker.receive(message).unwrap();
        if initial {
            while worker.pending() {
                worker.write(&mut socket).unwrap();
            }
        }
    }
    worker.write(&mut Prefix(&mut socket)).unwrap();
    assert!(worker.pending());
    worker
        .backend()
        .faults
        .observe_loss(false, "simulated loss after terminal prefix");
    worker.observe();
    assert!(worker.transport_abandoned());
    assert_eq!(serve(&mut socket, &mut worker), Exit::Uncertain);
    exit(Exit::Uncertain)
}
// Synthetic unavailable-adapter evidence. No native API is called.
struct InitRejection(roomie_worker_vulkan::engine::Telemetry, &'static str);
impl Backend for InitRejection {
    fn initialize(&mut self) -> Result<(), String> {
        Err(self.1.into())
    }
    fn render(&mut self, _: Operation) -> Result<Prepared, Failure> {
        unreachable!()
    }
    fn fault(&mut self) -> Option<String> {
        None
    }
    fn settled(&self) -> bool {
        true
    }
    fn evidence(&self) -> Option<Evidence> {
        None
    }
    fn reject(&mut self, _: Operation) -> Option<Evidence> {
        None
    }
    fn release(&mut self) -> bool {
        self.0.controlled_shutdown = true;
        true
    }
    fn acceptance_telemetry(&self) -> Option<&Telemetry> {
        Some(&self.0)
    }
}
// Adversarial protocol peer: correlated rejection does NOT fault supervisor.
// This mode tests harness correctness; it is not native worker behavior.
fn reject_supported(mode: &str) -> ! {
    use roomie_worker_protocol::{read_step, Body, Decoder, Message, Transmitter};
    // SAFETY: the supervisor transfers the private socket at fd 0 once.
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut decoder = Decoder::default();
    loop {
        let m = loop {
            if let Some(m) = read_step(&mut decoder, &mut socket).unwrap() {
                break m;
            }
        };
        let startup_failure = mode != "reject-supported";
        let stopped = matches!(m.body, Body::Shutdown) || startup_failure;
        let body = match m.body {
            Body::Initialize if mode == "startup-protocol" => Body::Stopped,
            Body::Initialize if mode == "startup-uncorrelated" => {
                Body::BackendFault("injected initialization failure".into())
            }
            Body::Initialize => Body::Ready,
            Body::Render(r) => Body::RejectedBeforeSubmission {
                job: r.job,
                diagnostic: "injected supported-request rejection".into(),
            },
            Body::Shutdown => Body::Stopped,
            _ => std::process::exit(3),
        };
        let mut tx = Transmitter::default();
        tx.queue(&Message {
            session: m.session + u64::from(mode == "startup-uncorrelated"),
            body,
        })
        .unwrap();
        while tx.pending() != 0 {
            tx.write(&mut socket).unwrap();
        }
        if stopped {
            std::process::exit(0);
        }
    }
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).take(5).collect();
    let mode = args.first().map(String::as_str).unwrap_or("good");
    let report = if args.get(1).is_some_and(|a| a == "--acceptance-report") {
        args.get(2).map(Path::new)
    } else {
        None
    };
    if [
        "reject-supported",
        "startup-protocol",
        "startup-uncorrelated",
    ]
    .contains(&mode)
    {
        reject_supported(mode);
    }
    if mode == "startup-exit" {
        std::process::exit(1);
    }
    if mode == "init-rejection" {
        run(
            InitRejection(Telemetry::default(), "injected initialization failure"),
            report,
            false,
        );
    }
    if mode == "init-unrelated" {
        run(
            InitRejection(Telemetry::default(), "unrelated startup fault"),
            report,
            false,
        );
    }
    if mode == "partial-terminal-loss" {
        partial_loss(report);
    }
    if [
        "good",
        "corrupt",
        "stopped-exit-failure",
        "silent-after-render",
    ]
    .contains(&mode)
    {
        let ops = Ops {
            bytes: vec![0; Layout::fixture().unwrap().staging],
            callback: None,
            corrupt: mode == "corrupt",
            stall: mode == "silent-after-render",
        };
        run(
            Persistent::new(
                ops,
                Arc::default(),
                Telemetry {
                    initialization_complete: true,
                    adapter: Some(roomie_worker_vulkan::engine::Adapter {
                        name: "accepted CPU fixture Operations".into(),
                        device_type: "Synthetic".into(),
                        backend: "None (GPU-free)".into(),
                        driver: "None (GPU-free)".into(),
                    }),
                    ..Default::default()
                },
            ),
            report,
            mode == "stopped-exit-failure",
        );
    }
    if let Some(mode) = support::Mode::parse(mode) {
        run(support::Fixture::new(mode), report, false);
    }
    std::process::exit(3)
}

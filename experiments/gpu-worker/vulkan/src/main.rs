use roomie_worker_vulkan::{
    config::LaunchConfig,
    engine::{Backend, Failure, Prepared},
    settlement::{Evidence, Operation},
    transport::serve,
    worker::{Exit, Worker},
};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;

struct Unavailable(String, Option<Evidence>);
impl Backend for Unavailable {
    fn initialize(&mut self) -> Result<(), String> {
        Err(self.0.clone())
    }
    fn render(&mut self, operation: Operation) -> Result<Prepared, Failure> {
        Err(Failure {
            diagnostic: self.0.clone(),
            evidence: Evidence::new(operation),
        })
    }
    fn fault(&mut self) -> Option<String> {
        None
    }
    fn settled(&self) -> bool {
        true
    }
    fn evidence(&self) -> Option<Evidence> {
        self.1
    }
    fn reject(&mut self, operation: Operation) -> Option<Evidence> {
        let evidence = Evidence::new(operation);
        self.1 = Some(evidence);
        Some(evidence)
    }
    fn release(&mut self) -> bool {
        true
    }
}
fn run(backend: impl Backend, report: Option<&std::path::Path>) -> ! {
    // Failure to create/write diagnostics cannot alter the backend's execution
    // or settlement. The owner treats a missing/incomplete report as an error.
    let report = report.and_then(|p| roomie_worker_vulkan::report::ReportFile::create(p).ok());
    // SAFETY: this executable is only launched with the private inherited
    // bidirectional socket at fd 0, transferred exactly once to UnixStream.
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };
    let mut worker = Worker::new(backend);
    let outcome = serve(&mut socket, &mut worker);
    if let Some(report) = report {
        let _ = report.finish(
            worker.backend().acceptance_telemetry(),
            outcome,
            if cfg!(feature = "native-vulkan") {
                "native-vulkan"
            } else {
                "unavailable-feature-disabled"
            },
        );
    }
    // Explicit release happened before Controlled. Uncertain deliberately
    // bypasses ALL native destructors, including Queue::drop's implicit wait.
    std::process::exit(match outcome {
        Exit::Controlled => 0,
        Exit::Failed => 1,
        Exit::Uncertain => 2,
    });
}
fn main() {
    #[cfg(feature = "native-vulkan")]
    roomie_worker_vulkan::native::install_containment_hook();
    let args: Vec<_> = std::env::args().skip(1).take(8).collect();
    let config = match LaunchConfig::parse(&args) {
        Ok(config) => config,
        Err(text) => run(Unavailable(text, None), None),
    };
    config.configure_driver();
    #[cfg(feature = "native-vulkan")]
    run(
        roomie_worker_vulkan::native::NativeBackend::new(config.adapter),
        config.report(),
    );
    #[cfg(not(feature = "native-vulkan"))]
    run(
        Unavailable("native-vulkan feature is disabled".into(), None),
        config.report(),
    );
}

//! Disposable parent-death test fixture, never an application entry point.
use roomie_worker_supervisor::{
    machine::{Phase, Policy},
    Supervisor,
};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let clock = Instant::now();
    let mut supervisor = Supervisor::spawn(
        Path::new(&args[1]),
        &["parent-death"],
        73,
        Policy::default(),
        clock.elapsed(),
    )
    .unwrap();
    while supervisor.outcome().phase != Phase::Ready {
        assert!(clock.elapsed() < Duration::from_secs(5));
        let readiness = supervisor
            .poll_ready(Duration::from_millis(50), clock.elapsed())
            .unwrap();
        supervisor.progress(readiness, clock.elapsed());
    }
    println!("{}", supervisor.pid());
    io::stdout().flush().unwrap();
    if args[2] == "exit" {
        // Deliberately bypass Supervisor::drop to exercise PDEATHSIG itself.
        std::process::exit(0);
    }
    // A pipe controlled by the outer disposable subprocess test. SIGKILL must
    // also kill the child even though Drop and channel cleanup never run.
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
    std::process::exit(0);
}

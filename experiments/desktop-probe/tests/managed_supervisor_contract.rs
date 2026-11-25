#![cfg(feature = "managed-supervisor-fixture")]

use desktop_probe::managed_supervisor::{Fixture, ManagedSupervisor};
use desktop_probe::presentation::{
    Action, ActionIntent, ClientExtent, Facing, FrameIdentity, FrameRequest, Size,
};
use roomie_worker_protocol::JobKey;
use roomie_worker_supervisor::machine::{Outcome, Phase, Policy, Shutdown};
use roomie_worker_supervisor::{Readiness, WaitInterest};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const FIXTURE_ENV: &str = "DESKTOPROOMIE_MANAGED_FIXTURE";

fn executable() -> PathBuf {
    std::env::var_os(FIXTURE_ENV)
        .unwrap_or_else(|| {
            panic!(
                "{FIXTURE_ENV} must name the absolute path to the independently built fake-worker"
            )
        })
        .into()
}

#[test]
fn production_adapter_accepts_concrete_protocol_and_supervisor_types() {
    let executable = executable();
    let origin = Instant::now();
    let mut managed = ManagedSupervisor::spawn(
        Fixture {
            executable: &executable,
            args: &["normal"],
            session: 41,
            policy: Policy {
                freshness: Duration::from_secs(2),
                ..Policy::default()
            },
        },
        origin,
    )
    .unwrap();
    let end = origin + Duration::from_secs(5);
    let step = |managed: &mut ManagedSupervisor| {
        let now = Instant::now();
        assert!(now < end, "timeout: {:?}", managed.outcome());
        let (interest, _): (WaitInterest, _) = managed.interests().unwrap();
        let readiness: Readiness = managed.wait_until(end).unwrap();
        assert!(interest.socket.is_some() || !readiness.writable);
        managed.progress(readiness, Instant::now()).unwrap();
    };
    while managed.outcome().phase == Phase::Starting {
        step(&mut managed);
    }
    managed.set_eligible(true).unwrap();
    let request = FrameRequest::new(
        FrameIdentity {
            generation: 3,
            sequence: 21,
        },
        ActionIntent {
            action: Action::Idle,
            facing: Facing::Neutral,
        },
        ClientExtent::new(Size::new(160, 160)).unwrap(),
        Duration::from_millis(300),
        Duration::from_millis(200),
    )
    .unwrap();
    let admitted = Instant::now();
    let key: JobKey = managed.admit(request, admitted).unwrap().unwrap();
    while managed.ready().is_none() {
        step(&mut managed);
    }
    assert!(managed.admit(request, Instant::now()).unwrap().is_none());
    let ready = managed.take_ready().unwrap();
    assert_eq!(ready.request, request);
    assert_eq!(ready.deadline, admitted + Duration::from_secs(2));
    assert_eq!((key.generation, key.sequence), (3, 21));
    assert_eq!(
        (key.epoch, key.serial),
        (ready.identity.epoch, ready.identity.serial)
    );
    assert_eq!(ready.frame.payload, [1, 0, 0, 255].repeat(160 * 160));
    assert!(managed.take_ready().is_none());
    managed.begin_shutdown(Instant::now()).unwrap();
    while !managed.outcome().reaped {
        step(&mut managed);
    }
    let outcome: Outcome = managed.outcome();
    assert_eq!(outcome.shutdown, Shutdown::Confirmed);
    assert!(outcome.stopped);
    assert_eq!(outcome.unresolved, None);
}

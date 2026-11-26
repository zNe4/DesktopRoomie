use super::*;
use crate::managed_vulkan::{AdapterClass, OwnerObservation};
use roomie_worker_supervisor::machine::{Phase, Policy, Shutdown};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;

#[test]
fn duplicate_owner_and_acquisition_failure_skip_the_actual_spawn_boundary() {
    for duplicate in [true, false] {
        let spawned = Cell::new(false);
        let result = owner_presentation_startup_with(
            &mut (),
            |_| {
                if duplicate {
                    Ok(Acquisition::Duplicate)
                } else {
                    Err("acquire failed".into())
                }
            },
            |_| panic!("not owned"),
            || Ok(SyntheticProducer::new()),
            |_, _| {
                spawned.set(true);
                ManagedSupervisor::spawn(
                    Fixture {
                        executable: std::path::Path::new("/nonexistent/must-not-spawn"),
                        args: &[],
                        session: 7,
                        policy: Policy::default(),
                    },
                    Instant::now(),
                )?;
                Ok(())
            },
        );
        assert!(!spawned.get());
        if duplicate {
            assert_eq!(result.unwrap(), Acquisition::Duplicate);
        } else {
            assert_eq!(result.unwrap_err().to_string(), "acquire failed");
        }
    }
}
#[test]
fn shared_finish_destroys_connection_before_reaping_and_retains_host_error() {
    let order = RefCell::new(Vec::new());
    let result = finish_managed_owner_with(
        Err("original host failure".into()),
        || order.borrow_mut().push("destroy connection"),
        || {
            order.borrow_mut().push("reap");
            (
                Err("later reap failure".into()),
                OwnerObservation::default(),
            )
        },
    );
    assert_eq!(*order.borrow(), ["destroy connection", "reap"]);
    let summary = managed_vulkan::classify_native(
        AdapterClass::Software,
        7,
        result.host.err().map(|e| e.to_string()),
        result.reaping.err().map(|e| e.to_string()),
        result.observation,
        Err("missing report".into()),
    );
    assert_eq!(summary.failures[0], "host: original host failure");
    assert!(summary.failures[1].contains("later reap failure"));
}
#[test]
fn owned_startup_and_shared_cleanup_reuse_actual_gpu_free_subprocess() {
    let worker = PathBuf::from(
        std::env::var_os("DESKTOPROOMIE_MANAGED_FIXTURE").expect("explicit fake-worker required"),
    );
    let order = RefCell::new(Vec::new());
    let mut managed = None;
    let acquisition = owner_presentation_startup_with(
        &mut (),
        |_| {
            order.borrow_mut().push("acquire");
            Ok(Acquisition::Owned)
        },
        |_| {
            order.borrow_mut().push("verify");
            Ok(())
        },
        || Ok(SyntheticProducer::new()),
        |_, presentation| {
            assert_eq!(
                presentation.initial_extent().size(),
                presentation::Size::new(160, 160)
            );
            order.borrow_mut().push("spawn");
            managed = Some(ManagedSupervisor::spawn(
                Fixture {
                    executable: &worker,
                    args: &["normal"],
                    session: 7,
                    policy: Policy::default(),
                },
                Instant::now(),
            )?);
            // Actual start_body's initial checked CPU publication is covered by the
            // preserved checked-transport tests and inspected production call site.
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(acquisition, Acquisition::Owned);
    let mut managed = managed.unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while managed.outcome().phase == Phase::Starting {
        assert!(Instant::now() < end);
        let ready = managed.wait_until(end).unwrap();
        managed.progress(ready, Instant::now()).unwrap();
    }
    assert_eq!(managed.outcome().phase, Phase::Ready);
    let result = finish_managed_owner_with(
        Ok(()),
        || order.borrow_mut().push("destroy connection"),
        || {
            let reaping = managed.reap_after_host_cleanup();
            order.borrow_mut().push("reaped");
            (reaping, OwnerObservation::of(&managed))
        },
    );
    assert!(result.host.is_ok());
    assert!(result.reaping.is_ok());
    assert_eq!(
        *order.borrow(),
        ["acquire", "verify", "spawn", "destroy connection", "reaped"]
    );
    let o = result.observation.supervisor.unwrap();
    assert!(o.reaped && o.stopped);
    assert_eq!(o.shutdown, Shutdown::Confirmed);
}
#[test]
fn native_and_fixture_entries_share_owner_and_preserve_checked_startup_order() {
    let source = include_str!("../lib.rs");
    let fixture = &source[source.find("pub fn run_managed_fixture(").unwrap()
        ..source
            .find("/// Explicit experimental native entry")
            .unwrap()];
    assert!(fixture.contains("run_managed_owner(fixture, duration_secs)"));
    let native = &source[source.find("pub fn run_managed_vulkan(").unwrap()
        ..source.find("struct ManagedOwnerResult").unwrap()];
    assert!(native.contains("run_managed_owner("));
    let owner = &source
        [source.find("fn run_owner(").unwrap()..source.find("/// Only a confirmed owner").unwrap()];
    assert!(
        owner.find("owner_presentation_startup_with(").unwrap()
            < owner.find("ManagedSupervisor::spawn(").unwrap()
    );
    assert!(owner.find("ManagedSupervisor::spawn(").unwrap() < owner.find("start_body(").unwrap());
    let body = &source
        [source.find("fn start_body(").unwrap()..source.find("fn finish_probe_with").unwrap()];
    assert!(body.find("present_frame(").unwrap() < body.find("run_probe(").unwrap());
}

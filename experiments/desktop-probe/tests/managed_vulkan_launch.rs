#![cfg(feature = "managed-supervisor-fixture")]
use desktop_probe::managed_vulkan::*;
use roomie_worker_protocol::JobKey;
use roomie_worker_supervisor::machine::{
    JobState, JobStatus, Outcome, Phase, Shutdown, Transmission,
};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{symlink, DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Files {
    root: PathBuf,
    worker: PathBuf,
    driver: PathBuf,
}
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "roomie-native-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new().mode(0o700).create(&root).unwrap();
        let worker = root.join("worker with spaces");
        let driver = root.join("driver with spaces.json");
        fs::write(&worker, "synthetic configuration fixture, never executed").unwrap();
        fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&driver, "{}").unwrap();
        Self {
            root,
            worker,
            driver,
        }
    }
    fn args(&self) -> Vec<String> {
        [
            "--worker",
            self.worker.to_str().unwrap(),
            "--adapter",
            "software",
            "--driver",
            self.driver.to_str().unwrap(),
            "--report-directory",
            self.root.join("fresh report").to_str().unwrap(),
            "--session",
            "0",
            "--duration",
            "2",
        ]
        .map(String::from)
        .to_vec()
    }
    fn launch(&self) -> NativeVulkanLaunch {
        NativeVulkanLaunch::parse(self.args()).unwrap()
    }
}
fn replace(args: &mut [String], key: &str, value: &str) {
    let index = args.iter().position(|arg| arg == key).unwrap();
    args[index + 1] = value.into();
}
fn report_text() -> String {
    let mut text = "roomie_acceptance_report=1\ncomplete=0\nkind_hex=6e61746976652d76756c6b616e\nexit=Controlled\ntelemetry_available=1\n".to_owned();
    for (key, value) in [
        ("adapter", "Injected adapter"),
        ("device_type", "Cpu"),
        ("backend", "Vulkan"),
        ("driver", "Injected driver"),
    ] {
        let hex: String = value.bytes().map(|b| format!("{b:02x}")).collect();
        text.push_str(&format!("{key}_hex={hex}\n"));
    }
    for (key, value) in [
        ("initialization_complete", 1),
        ("initialization_ns", 10),
        ("device_initialization_ns", 3),
        ("pipeline_initialization_ns", 4),
        ("submission_attempts", 2),
        ("accepted_submissions", 2),
        ("confirmed_completions", 2),
        ("native_gpu_completions", 2),
        ("map_requests", 2),
        ("map_callbacks", 2),
        ("successful_maps", 2),
        ("unmaps", 2),
        ("staging_allocations", 1),
        ("staging_reuses", 1),
        ("staging_retirements", 0),
        ("peak_outstanding", 1),
        ("timed_operations", 2),
        ("encode_ns", 2),
        ("submit_ns", 2),
        ("wait_ns", 2),
        ("map_ns", 2),
        ("repack_ns", 2),
        ("controlled_release", 1),
        ("uncertain_release", 0),
    ] {
        text.push_str(&format!("{key}={value}\n"));
    }
    text.push_str("telemetry_error=0\ncomplete=1\n");
    text
}
fn observation() -> OwnerObservation {
    OwnerObservation {
        supervisor: Some(Outcome {
            session: 0,
            initialize_transmitted: true,
            job: Some(JobStatus {
                job: JobKey {
                    epoch: 1,
                    serial: 2,
                    generation: 1,
                    sequence: 2,
                },
                transmission: Transmission::Complete,
                state: JobState::PhysicallySettled,
            }),
            phase: Phase::Exited,
            failure: None,
            unresolved: None,
            stopped: true,
            reaped: true,
            termination_requested: false,
            shutdown: Shutdown::Confirmed,
        }),
        ..OwnerObservation::default()
    }
}
fn classify(text: &str, adapter: AdapterClass, observed: OwnerObservation) -> NativeSummary {
    classify_native(
        adapter,
        0,
        None,
        None,
        observed,
        NativeTelemetry::parse(text.as_bytes()),
    )
}
#[test]
fn valid_configuration_has_exact_literal_worker_pairs_including_spaces() {
    let f = Files::new();
    let launch = f.launch();
    assert_eq!(launch.worker(), f.worker);
    assert_eq!(launch.driver(), f.driver);
    assert_eq!(launch.adapter(), AdapterClass::Software);
    assert_eq!(launch.session(), 0);
    assert_eq!(launch.duration_seconds(), 2);
    assert_eq!(
        launch.worker_args(),
        [
            String::from("--adapter"),
            "software".into(),
            "--driver".into(),
            f.driver.to_str().unwrap().into(),
            "--acceptance-report".into(),
            launch
                .report_directory()
                .join("worker.telemetry")
                .to_str()
                .unwrap()
                .into()
        ]
    );
}
#[test]
fn missing_duplicate_unknown_and_unpaired_arguments_are_rejected_before_mkdir() {
    let f = Files::new();
    let args = f.args();
    for i in (0..args.len()).step_by(2) {
        let mut missing = args.clone();
        missing.drain(i..i + 2);
        assert!(NativeVulkanLaunch::parse(missing).is_err());
        let mut duplicate = args.clone();
        duplicate.extend_from_slice(&args[i..i + 2]);
        assert!(NativeVulkanLaunch::parse(duplicate).is_err());
    }
    let mut unknown = args.clone();
    unknown.extend(["--env".into(), "X=Y".into()]);
    assert!(NativeVulkanLaunch::parse(unknown).is_err());
    let mut unpaired = args;
    unpaired.pop();
    assert!(NativeVulkanLaunch::parse(unpaired).is_err());
    assert!(!f.root.join("fresh report").exists());
}
#[test]
fn relative_missing_directory_and_nonexecutable_workers_are_rejected() {
    for invalid in ["relative", "/nonexistent/roomie-worker", "", "/tmp"] {
        let f = Files::new();
        let mut args = f.args();
        replace(&mut args, "--worker", invalid);
        assert!(NativeVulkanLaunch::parse(args).is_err());
    }
    let f = Files::new();
    fs::set_permissions(&f.worker, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(NativeVulkanLaunch::parse(f.args()).is_err());
}
#[test]
fn invalid_adapter_and_driver_manifest_restrictions_are_enforced() {
    let f = Files::new();
    for invalid in ["default", "cpu", "gpu", "Software", ""] {
        let mut a = f.args();
        replace(&mut a, "--adapter", invalid);
        assert!(NativeVulkanLaunch::parse(a).is_err());
    }
    for invalid in [
        "relative.json",
        "/unavailable/driver.json",
        "/tmp",
        "/tmp/a:b.json",
        "/tmp/a\0.json",
    ] {
        let mut a = f.args();
        replace(&mut a, "--driver", invalid);
        assert!(NativeVulkanLaunch::parse(a).is_err());
    }
    let bad = f.root.join("manifest.txt");
    fs::write(&bad, "{}").unwrap();
    let mut a = f.args();
    replace(&mut a, "--driver", bad.to_str().unwrap());
    assert!(NativeVulkanLaunch::parse(a).is_err());
}
#[test]
fn duration_session_bounds_and_owner_deadline_overflow_are_rejected() {
    let f = Files::new();
    for invalid in [
        "0",
        "-1",
        "NaN",
        "inf",
        "1.5",
        "18446744073709551615",
        "18446744073709551616",
        "+1",
        "",
    ] {
        let mut a = f.args();
        replace(&mut a, "--duration", invalid);
        assert!(NativeVulkanLaunch::parse(a).is_err(), "{invalid}");
    }
    for invalid in ["-1", "1.5", "18446744073709551616", "+1", ""] {
        let mut a = f.args();
        replace(&mut a, "--session", invalid);
        assert!(NativeVulkanLaunch::parse(a).is_err());
    }
    let mut a = f.args();
    replace(&mut a, "--session", "18446744073709551615");
    assert_eq!(NativeVulkanLaunch::parse(a).unwrap().session(), u64::MAX);
}
#[test]
fn report_directory_is_fresh_canonical_and_private_from_creation() {
    let f = Files::new();
    let l = f.launch();
    let m = fs::metadata(l.report_directory()).unwrap();
    assert_eq!(m.permissions().mode() & 0o777, 0o700);
    assert_eq!(
        l.report_directory().canonicalize().unwrap(),
        l.report_directory()
    );
    assert!(!l.report_directory().join("worker.telemetry").exists());
    assert!(l.read_telemetry().is_err());
}
#[test]
fn existing_directory_file_symlink_relative_and_unsafe_parents_are_rejected() {
    for kind in 0..6 {
        let f = Files::new();
        let dest = f.root.join("fresh report");
        let mut args = f.args();
        match kind {
            0 => fs::create_dir(&dest).unwrap(),
            1 => fs::write(&dest, "old").unwrap(),
            2 => symlink(&f.driver, &dest).unwrap(),
            3 => replace(&mut args, "--report-directory", "relative"),
            4 => fs::set_permissions(&f.root, fs::Permissions::from_mode(0o777)).unwrap(),
            _ => {
                let link = f.root.join("redirect");
                symlink(&f.root, &link).unwrap();
                replace(
                    &mut args,
                    "--report-directory",
                    link.join("new").to_str().unwrap(),
                );
            }
        }
        assert!(NativeVulkanLaunch::parse(args).is_err(), "kind {kind}");
    }
}
#[test]
fn report_reader_rejects_symlinks_hardlinks_public_files_and_directory_replacement() {
    let f = Files::new();
    let l = f.launch();
    let path = l.report_directory().join("worker.telemetry");
    symlink(&f.driver, &path).unwrap();
    assert!(l.read_telemetry().is_err());
    fs::remove_file(&path).unwrap();
    fs::write(&path, report_text()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(l.read_telemetry().is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(l.read_telemetry().is_ok());
    fs::hard_link(&path, f.root.join("alias")).unwrap();
    assert!(l.read_telemetry().is_err());
    fs::rename(l.report_directory(), f.root.join("moved")).unwrap();
    DirBuilder::new()
        .mode(0o700)
        .create(l.report_directory())
        .unwrap();
    assert!(l.read_telemetry().is_err());
}
#[test]
fn bounded_reader_accepts_complete_private_report_and_rejects_oversize() {
    let f = Files::new();
    let l = f.launch();
    let p = l.report_directory().join("worker.telemetry");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&p)
        .unwrap();
    file.write_all(report_text().as_bytes()).unwrap();
    assert_eq!(
        l.read_telemetry()
            .unwrap()
            .number("native_gpu_completions")
            .unwrap(),
        2
    );
    file.write_all(&vec![b'x'; MAX_REPORT_BYTES]).unwrap();
    assert!(l.read_telemetry().is_err());
}

#[test]
fn preexisting_report_fails_the_native_entry_before_owner_connection() {
    let files = Files::new();
    let launch = files.launch();
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(launch.report_directory().join("worker.telemetry"))
        .unwrap();
    file.write_all(report_text().as_bytes()).unwrap();
    let failure = desktop_probe::run_managed_vulkan(launch).unwrap_err();
    assert!(failure.0.failures[0].contains("worker telemetry already exists"));
    assert!(failure.0.observation.supervisor.is_none());
    assert!(failure.0.telemetry.is_ok());
}
#[test]
fn malformed_incomplete_truncated_duplicate_and_oversized_reports_fail() {
    let good = report_text();
    for bad in [
        Vec::new(),
        b"roomie_acceptance_report=1\ncomplete=0\n".to_vec(),
        good.replace("complete=1\n", "").into_bytes(),
        good.replace("roomie_acceptance_report=1", "roomie_acceptance_report=2")
            .into_bytes(),
        good.replace("telemetry_error=0", "complete=0\ntelemetry_error=0")
            .into_bytes(),
        good.replace("telemetry_error=0", "telemetry_error=0\ntelemetry_error=0")
            .into_bytes(),
        good.replace("wait_ns=2", "wait_ns=-1").into_bytes(),
        vec![b'x'; MAX_REPORT_BYTES + 1],
        vec![255],
    ] {
        assert!(NativeTelemetry::parse(&bad).is_err());
    }
}
#[test]
fn every_required_native_field_is_checked_and_counter_contradictions_fail() {
    let good = report_text();
    for (field, invalid) in [
        ("initialization_ns=10", "initialization_ns=6"),
        (
            "submission_attempts=2",
            "submission_attempts=18446744073709551616",
        ),
        (
            "wait_ns=2",
            "wait_ns=340282366920938463463374607431768211455",
        ),
    ] {
        assert!(NativeTelemetry::parse(good.replace(field, invalid).as_bytes()).is_err());
    }
    for line in good.lines().filter(|line| !line.starts_with("complete=")) {
        let missing = good.replace(&format!("{line}\n"), "");
        assert!(
            !classify(&missing, AdapterClass::Software, observation()).successful(),
            "missing {line}"
        );
    }
    for (a, b) in [
        ("confirmed_completions=2", "confirmed_completions=3"),
        ("accepted_submissions=2", "accepted_submissions=3"),
        ("native_gpu_completions=2", "native_gpu_completions=1"),
        ("unmaps=2", "unmaps=1"),
        ("staging_reuses=1", "staging_reuses=0"),
        ("controlled_release=1", "controlled_release=2"),
        ("uncertain_release=0", "uncertain_release=1"),
        ("telemetry_available=1", "telemetry_available=0"),
    ] {
        assert!(
            !classify(&good.replace(a, b), AdapterClass::Software, observation()).successful(),
            "{a}"
        );
    }
}
#[test]
fn requested_class_does_not_substitute_for_actual_native_identity() {
    let good = report_text();
    assert!(!classify(&good, AdapterClass::Hardware, observation()).successful());
    let hardware = good.replace(
        "device_type_hex=437075",
        "device_type_hex=4469736372657465477075",
    );
    assert!(classify(&hardware, AdapterClass::Hardware, observation()).successful());
    for (a, b) in [
        (
            "kind_hex=6e61746976652d76756c6b616e",
            "kind_hex=73796e746865746963",
        ),
        ("backend_hex=56756c6b616e", "backend_hex=4f70656e474c"),
        (
            "adapter_hex=496e6a65637465642061646170746572",
            "adapter_hex=",
        ),
        ("device_type_hex=437075", "device_type_hex=zz"),
    ] {
        assert!(!classify(&good.replace(a, b), AdapterClass::Software, observation()).successful());
    }
}
#[test]
fn clean_host_initialization_failure_and_zero_completions_are_never_native_success() {
    let good = report_text();
    assert!(!classify(
        &good.replace("initialization_complete=1", "initialization_complete=0"),
        AdapterClass::Software,
        observation()
    )
    .successful());
    let mut zero = good;
    for key in [
        "submission_attempts",
        "accepted_submissions",
        "confirmed_completions",
        "native_gpu_completions",
        "map_requests",
        "map_callbacks",
        "successful_maps",
        "unmaps",
        "timed_operations",
    ] {
        zero = zero.replace(&format!("{key}=2"), &format!("{key}=0"));
    }
    assert!(!classify(&zero, AdapterClass::Software, observation()).successful());
}
#[test]
fn unresolved_shutdown_reaping_and_supervisor_faults_prevent_success() {
    use roomie_worker_supervisor::machine::Failure;
    for which in 0..11 {
        let mut o = observation();
        let s = o.supervisor.as_mut().unwrap();
        match which {
            0 => s.reaped = false,
            1 => s.stopped = false,
            2 => s.shutdown = Shutdown::Unconfirmed,
            3 => s.failure = Some(Failure::BackendFault),
            4 => s.unresolved = s.job.map(|j| j.job),
            5 => s.termination_requested = true,
            6 => s.initialize_transmitted = false,
            7 => s.session = 1,
            8 => s.job = None,
            9 => s.job.as_mut().unwrap().transmission = Transmission::Partial,
            _ => s.job.as_mut().unwrap().state = JobState::RejectedBeforeDispatch,
        }
        assert!(!classify(&report_text(), AdapterClass::Software, o).successful());
    }
    let mut o = observation();
    o.outstanding = Some(desktop_probe::presentation::gpu_handoff::JobIdentity {
        session: 0,
        epoch: 1,
        serial: 1,
        frame: desktop_probe::presentation::FrameIdentity {
            generation: 1,
            sequence: 1,
        },
    });
    o.unresolved = o.outstanding;
    assert!(!classify(&report_text(), AdapterClass::Software, o).successful());
}
#[test]
fn host_failure_is_primary_and_secondary_evidence_survives() {
    let s = classify_native(
        AdapterClass::Software,
        0,
        Some("original upload failure".into()),
        Some("later reap failure".into()),
        OwnerObservation::default(),
        Err("missing report".into()),
    );
    let failure = s.into_result().unwrap_err();
    assert_eq!(failure.0.failures.len(), 4);
    let text = failure.to_string();
    assert!(text.starts_with("native cohort failed: host: original upload failure"));
    assert!(text.contains("later reap failure"));
    assert!(text.contains("missing report"));
}
#[test]
fn complete_consistent_injected_evidence_is_classified_as_success() {
    let s = classify(&report_text(), AdapterClass::Software, observation())
        .into_result()
        .unwrap();
    assert!(s.successful());
    assert_eq!(
        s.telemetry.unwrap().decoded("adapter_hex").unwrap(),
        "Injected adapter"
    );
}
#[test]
fn invalid_launcher_arguments_exit_before_x11_or_child_spawn() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_managed-vulkan-probe"))
        .args(["--unknown", "x"])
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unknown native launch option"));
    assert!(!stderr.contains("X11"));
}
#[test]
fn ordinary_help_and_manifest_default_run_remain_compatible() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_desktop-probe"))
        .arg("--help")
        .env_clear()
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("--synthetic-animation"));
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("default-run = \"desktop-probe\""));
    assert!(manifest.contains("required-features = [\"managed-supervisor-fixture\"]"));
}

// Each report remains internally consistent; only its relationship to the
// otherwise valid supervisor observation varies in these F01 regressions.
fn cross_authority_report(completions: u64) -> NativeTelemetry {
    let mut text = String::new();
    for line in report_text().lines() {
        let (key, value) = line.split_once('=').unwrap();
        let count = match key {
            "submission_attempts"
            | "accepted_submissions"
            | "confirmed_completions"
            | "native_gpu_completions"
            | "map_requests"
            | "map_callbacks"
            | "successful_maps"
            | "unmaps"
            | "timed_operations" => Some(completions),
            "staging_reuses" => Some(completions.saturating_sub(1)),
            "peak_outstanding" => Some(u64::from(completions != 0)),
            _ => None,
        };
        text.push_str(&format!(
            "{key}={}\n",
            count.map_or_else(|| value.to_owned(), |n| n.to_string())
        ));
    }
    NativeTelemetry::parse(text.as_bytes()).unwrap()
}
fn cross_authority_observation(serial: u64, unsent: bool) -> OwnerObservation {
    let mut observed = observation();
    let job = observed.supervisor.as_mut().unwrap().job.as_mut().unwrap();
    job.job.serial = serial;
    if unsent {
        job.state = JobState::RejectedBeforeDispatch;
        job.transmission = Transmission::Unsent;
    }
    observed
}
fn cross_authority_summary(completions: u64, serial: u64, unsent: bool) -> NativeSummary {
    classify_native(
        AdapterClass::Software,
        0,
        None,
        None,
        cross_authority_observation(serial, unsent),
        Ok(cross_authority_report(completions)),
    )
}

#[test]
fn cross_authority_settled_completions_above_final_serial_are_rejected() {
    let summary = cross_authority_summary(2, 1, false);
    assert!(!summary.successful());
    assert_eq!(summary.failures.len(), 1);
    assert!(summary.failures[0].contains("native completion count contradicts final job serial"));
}
#[test]
fn cross_authority_settled_completions_equal_to_final_serial_are_accepted() {
    assert!(cross_authority_summary(2, 2, false).successful());
}
#[test]
fn cross_authority_settled_completions_below_final_serial_are_accepted() {
    assert!(cross_authority_summary(2, 5, false).successful());
}
#[test]
fn cross_authority_unsent_completions_equal_to_final_serial_are_rejected() {
    assert!(!cross_authority_summary(2, 2, true).successful());
}
#[test]
fn cross_authority_unsent_completions_below_final_serial_are_accepted() {
    assert!(cross_authority_summary(2, 5, true).successful());
}
#[test]
fn cross_authority_zero_and_boundary_serials_are_checked_without_subtraction() {
    for unsent in [false, true] {
        assert!(!cross_authority_summary(2, 0, unsent).successful());
        let zero = cross_authority_summary(0, u64::MAX, unsent);
        assert!(!zero.successful());
        assert!(zero
            .failures
            .iter()
            .any(|error| error == "zero native GPU completions"));
        assert!(cross_authority_summary(2, u64::MAX, unsent).successful());
    }
    assert!(cross_authority_summary(u64::MAX, u64::MAX, false).successful());
    assert!(!cross_authority_summary(u64::MAX, u64::MAX, true).successful());
    assert!(cross_authority_summary(u64::MAX.saturating_sub(1), u64::MAX, true).successful());
}
#[test]
fn cross_authority_failure_retains_host_priority_and_secondary_evidence() {
    let summary = classify_native(
        AdapterClass::Software,
        0,
        Some("original host failure".into()),
        Some("secondary reaping failure".into()),
        cross_authority_observation(1, false),
        Ok(cross_authority_report(2)),
    );
    assert_eq!(summary.failures.len(), 3);
    assert_eq!(summary.failures[0], "host: original host failure");
    assert_eq!(summary.failures[1], "reaping: secondary reaping failure");
    assert!(summary.failures[2].contains("native completion count contradicts final job serial"));
    assert!(summary.telemetry.is_ok());
    assert_eq!(
        summary
            .observation
            .supervisor
            .unwrap()
            .job
            .unwrap()
            .job
            .serial,
        1
    );
}

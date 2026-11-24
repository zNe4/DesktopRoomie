use roomie_worker_acceptance::telemetry::{self, Report, RunDirectory};
use roomie_worker_protocol::JobKey;
use roomie_worker_vulkan::{
    engine::{Adapter, Backend, Telemetry},
    report::{write_summary, ReportFile, HEADER, MAX_REPORT_BYTES},
    settlement::Operation,
    worker::Exit,
};
use std::{
    fs,
    io::{self, Write},
    os::unix::{fs::symlink, fs::PermissionsExt},
    time::Duration,
};
#[path = "../../../vulkan/tests/support.rs"]
mod support;
fn operation() -> Operation {
    Operation {
        session: 3,
        job: JobKey {
            epoch: 7,
            serial: 11,
            generation: 13,
            sequence: 17,
        },
        phase: 2,
    }
}
#[test]
fn attempt_does_not_increment_confirmed_acceptance_or_completion() {
    for mode in [
        support::Mode::SubmitFailure,
        support::Mode::SubmitReportsError,
        support::Mode::SubmissionWait,
    ] {
        let mut backend = support::Fixture::new(mode);
        backend.initialize().unwrap();
        assert!(backend.render(operation()).is_err());
        let t = backend.telemetry();
        assert_eq!(t.submissions, 1);
        assert_eq!(t.accepted_submissions, 0);
        assert_eq!(t.completed_submissions, 0);
        assert_eq!(t.successful_maps, 0);
        assert_eq!(t.explicit_unmaps, 0);
        assert_eq!(t.staging_retirements, 1);
        assert!(!backend.release());
    }
}
#[test]
fn successful_wait_failed_map_and_unmap_have_separate_increment_sites() {
    for (mode, maps, unmaps) in [
        (support::Mode::Good, 1, 1),
        (support::Mode::MapFailed, 0, 1),
        (support::Mode::UnmapReportsError, 1, 0),
    ] {
        let mut backend = support::Fixture::new(mode);
        backend.initialize().unwrap();
        let _ = backend.render(operation());
        let t = backend.telemetry();
        assert_eq!(t.submissions, 1);
        assert_eq!(t.accepted_submissions, 1);
        assert_eq!(t.completed_submissions, 1);
        assert_eq!(t.successful_maps, maps);
        assert_eq!(t.explicit_unmaps, unmaps);
        assert_eq!(t.timed_operations, 1);
    }
}
#[test]
fn writer_is_bounded_even_for_long_identity_strings_and_reports_domain() {
    let t = Telemetry {
        adapter: Some(Adapter {
            name: "a".repeat(100000),
            device_type: "b".repeat(100000),
            backend: "Vulkan".into(),
            driver: "d".repeat(100000),
        }),
        ..Default::default()
    };
    let mut bytes = HEADER.as_bytes().to_vec();
    write_summary(&mut bytes, Some(&t), Exit::Controlled, "synthetic").unwrap();
    assert!(bytes.len() <= MAX_REPORT_BYTES);
    assert_eq!(MAX_REPORT_BYTES, telemetry::MAX_REPORT_BYTES);
    let report = Report::parse(&bytes).unwrap();
    assert_eq!(report.kind().unwrap(), "synthetic");
    assert_eq!(report.decoded("adapter_hex").unwrap().len(), 512);
    assert!(report.exact125(true).is_err());
}
#[test]
fn exclusive_creation_rejects_existing_files_symlinks_unsafe_names_and_public_directories() {
    let directory = RunDirectory::create().unwrap();
    let path = directory.report_path();
    fs::write(&path, "owner file").unwrap();
    assert!(ReportFile::create(&path).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "owner file");
    fs::remove_file(&path).unwrap();
    let other = directory.0.join("owner");
    fs::write(&other, "preserve").unwrap();
    symlink(&other, &path).unwrap();
    assert!(ReportFile::create(&path).is_err());
    assert_eq!(fs::read_to_string(&other).unwrap(), "preserve");
    fs::remove_file(&path).unwrap();
    assert!(ReportFile::create(&directory.0.join("unrelated")).is_err());
    assert!(ReportFile::create(std::path::Path::new("worker.telemetry")).is_err());
    assert!(ReportFile::create(&directory.0.join("../worker.telemetry")).is_err());
    fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(ReportFile::create(&path).is_err());
}
#[test]
fn report_is_private_and_interruption_remains_incomplete() {
    let directory = RunDirectory::create().unwrap();
    let path = directory.report_path();
    drop(ReportFile::create(&path).unwrap());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(Report::read(&path).unwrap_err().contains("incomplete"));
    assert!(Report::read(&directory.0.join("missing"))
        .unwrap_err()
        .contains("unavailable"));
}
#[test]
fn parser_rejects_oversized_truncated_duplicate_missing_and_non_ascii_fields() {
    let mut good = HEADER.as_bytes().to_vec();
    write_summary(
        &mut good,
        Some(&Telemetry::default()),
        Exit::Controlled,
        "synthetic",
    )
    .unwrap();
    for end in [0, 1, HEADER.len(), good.len() - 1, good.len() - 5] {
        assert!(Report::parse(&good[..end]).is_err());
    }
    assert!(Report::parse(&vec![b'x'; MAX_REPORT_BYTES + 1]).is_err());
    let text = String::from_utf8(good).unwrap();
    assert!(Report::parse(
        text.replace("telemetry_error=0", "telemetry_error=0\ntelemetry_error=0")
            .as_bytes()
    )
    .is_err());
    assert!(Report::parse(text.replace("telemetry_error=0\n", "").as_bytes()).is_err());
    let report = Report::parse(
        text.replace("kind_hex=73796e746865746963", "kind_hex=ࠀࠀ")
            .as_bytes(),
    )
    .unwrap_err();
    assert!(report.contains("hex"));
    assert!(Report::parse(
        text.replace("telemetry_error=0", "telemetry_error=1")
            .as_bytes()
    )
    .is_err());
}
struct FailAfter {
    remaining: usize,
    bytes: Vec<u8>,
}
impl Write for FailAfter {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::ErrorKind::StorageFull.into());
        }
        let n = self.remaining.min(b.len());
        self.bytes.extend_from_slice(&b[..n]);
        self.remaining -= n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn io_failure_cannot_write_complete_marker_or_change_backend_settlement() {
    let mut backend = support::Fixture::new(support::Mode::Good);
    backend.initialize().unwrap();
    backend.render(operation()).unwrap();
    let evidence = backend.evidence();
    for remaining in [0, 1, 40, 200] {
        let mut writer = FailAfter {
            remaining,
            bytes: HEADER.as_bytes().to_vec(),
        };
        assert!(write_summary(
            &mut writer,
            Some(backend.telemetry()),
            Exit::Controlled,
            "synthetic"
        )
        .is_err());
        assert!(Report::parse(&writer.bytes).is_err());
        assert_eq!(backend.evidence(), evidence);
        assert!(backend.settled());
    }
    assert!(backend.release());
}
#[test]
fn missing_backend_metrics_are_unknown_not_fabricated_zero_operations() {
    let mut bytes = HEADER.as_bytes().to_vec();
    write_summary(&mut bytes, None, Exit::Uncertain, "synthetic").unwrap();
    let report = Report::parse(&bytes).unwrap();
    assert_eq!(report.value("telemetry_available").unwrap(), "0");
    assert!(report.number("submission_attempts").is_err());
    assert!(report.exact125(false).is_err());
}
#[test]
fn zero_duration_samples_are_distinct_from_missing_metrics() {
    let t = Telemetry {
        initialization: Duration::ZERO,
        ..Default::default()
    };
    let mut bytes = HEADER.as_bytes().to_vec();
    write_summary(&mut bytes, Some(&t), Exit::Controlled, "synthetic").unwrap();
    let report = Report::parse(&bytes).unwrap();
    assert_eq!(report.number("initialization_ns").unwrap(), 0);
    assert!(report.number("invented").is_err());
}

#[test]
fn flush_failure_is_reported_before_the_complete_marker() {
    struct FlushFailure(Vec<u8>);
    impl Write for FlushFailure {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::Other.into())
        }
    }
    let mut writer = FlushFailure(HEADER.as_bytes().to_vec());
    assert!(write_summary(
        &mut writer,
        Some(&Telemetry::default()),
        Exit::Controlled,
        "synthetic"
    )
    .is_err());
    assert!(Report::parse(&writer.0).is_err());
}

#![cfg(feature = "managed-supervisor-fixture")]

use desktop_probe::managed_vulkan::NativeVulkanLaunch;
use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "roomie-evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new().mode(0o700).create(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn explicit_evidence_literals_preserve_existing_worker_arguments() {
    let test = Directory::new();
    let worker = test.0.join("worker");
    let driver = test.0.join("driver.json");
    fs::write(&worker, "fixture for parse only").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&driver, b"{}").unwrap();
    let args = |report: &str| {
        vec![
            "--worker".to_owned(),
            worker.display().to_string(),
            "--driver".to_owned(),
            driver.display().to_string(),
            "--adapter".to_owned(),
            "software".to_owned(),
            "--session".to_owned(),
            "7".to_owned(),
            "--duration".to_owned(),
            "15".to_owned(),
            "--report-directory".to_owned(),
            test.0.join(report).display().to_string(),
        ]
    };
    let normal = NativeVulkanLaunch::parse(args("normal")).unwrap();
    let mut correct = args("correct");
    correct.extend(["--evidence".to_owned(), "correctness".to_owned()]);
    let correct = NativeVulkanLaunch::parse(correct).unwrap();
    assert_eq!(normal.worker_args()[..5], correct.worker_args()[..5]);

    for literal in ["Correctness", "true", "expected-initialization-rejection"] {
        let mut invalid = args("invalid");
        invalid.extend(["--evidence".to_owned(), literal.to_owned()]);
        assert!(NativeVulkanLaunch::parse(invalid).is_err());
        assert!(!test.0.join("invalid").exists());
    }
    fs::write(&driver, r#"{"file_format_version":"1.0.0","ICD":{"library_path":"/nonexistent/desktoproomie-unavailable-vulkan.so","api_version":"1.3.0"}}"#).unwrap();
    let mut negative = args("reject");
    negative.extend([
        "--evidence".to_owned(),
        "expected-initialization-rejection".to_owned(),
    ]);
    assert!(NativeVulkanLaunch::parse(negative).is_ok());
}

#[test]
fn offline_binary_handles_missing_and_unsafe_reports_without_native_environment() {
    let test = Directory::new();
    let executable = env!("CARGO_BIN_EXE_managed-vulkan-verify");
    let no_args = Command::new(executable).env_clear().output().unwrap();
    assert_eq!(no_args.status.code(), Some(2));
    let missing = Command::new(executable)
        .env_clear()
        .arg(&test.0)
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8(missing.stderr)
        .unwrap()
        .contains("report unavailable"));
    std::os::unix::fs::symlink("/dev/zero", test.0.join("worker.telemetry")).unwrap();
    assert_eq!(
        Command::new(executable)
            .env_clear()
            .arg(&test.0)
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
}

//! Optional experimental report. This module has no settlement authority.
use crate::{engine::Telemetry, worker::Exit};
use std::{
    ffi::CString,
    fmt::Write as _,
    fs::{File, OpenOptions},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, Path},
};

/// Implementation parameter, absent numerically from approved D2-C-P.
pub const MAX_REPORT_BYTES: usize = 16_384;
pub const HEADER: &str = "roomie_acceptance_report=1\ncomplete=0\n";
pub struct ReportFile(File);
impl ReportFile {
    pub fn create(path: &Path) -> io::Result<Self> {
        let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
            || path.file_name().is_none_or(|n| n != "worker.telemetry")
            || parent.canonicalize()? != parent
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent)?;
        let metadata = directory.metadata()?;
        // SAFETY: geteuid is a side-effect-free identity query.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let name = CString::new(path.file_name().unwrap().as_bytes())
            .map_err(|_| io::ErrorKind::InvalidInput)?;
        // SAFETY: directory fd and NUL-terminated name live across openat; the
        // returned descriptor is owned once. O_EXCL never follows an old file.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY
                    | libc::O_CREAT
                    | libc::O_EXCL
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(HEADER.as_bytes())?;
        Ok(Self(file))
    }
    pub fn finish(
        mut self,
        telemetry: Option<&Telemetry>,
        exit: Exit,
        kind: &str,
    ) -> io::Result<()> {
        write_summary(&mut self.0, telemetry, exit, kind)
    }
}
fn hex(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .take(512)
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn write_summary(
    writer: &mut impl Write,
    telemetry: Option<&Telemetry>,
    exit: Exit,
    kind: &str,
) -> io::Result<()> {
    let mut text = String::with_capacity(8192);
    writeln!(text, "kind_hex={}", hex(kind)).unwrap();
    writeln!(
        text,
        "exit={exit:?}\ntelemetry_available={}",
        u8::from(telemetry.is_some())
    )
    .unwrap();
    if let Some(t) = telemetry {
        if let Some(adapter) = &t.adapter {
            for (k, v) in [
                ("adapter", &adapter.name),
                ("device_type", &adapter.device_type),
                ("backend", &adapter.backend),
                ("driver", &adapter.driver),
            ] {
                writeln!(text, "{k}_hex={}", hex(v)).unwrap();
            }
        }
        for (k, v) in [
            (
                "initialization_complete",
                u128::from(t.initialization_complete),
            ),
            ("initialization_ns", t.initialization.as_nanos()),
            (
                "device_initialization_ns",
                t.device_initialization.as_nanos(),
            ),
            (
                "pipeline_initialization_ns",
                t.pipeline_initialization.as_nanos(),
            ),
            ("submission_attempts", t.submissions as u128),
            ("accepted_submissions", t.accepted_submissions as u128),
            ("confirmed_completions", t.completed_submissions as u128),
            (
                "native_gpu_completions",
                if kind == "native-vulkan" {
                    t.completed_submissions as u128
                } else {
                    0
                },
            ),
            ("map_requests", t.mapping_requests as u128),
            ("map_callbacks", t.mapping_completions as u128),
            ("successful_maps", t.successful_maps as u128),
            ("unmaps", t.explicit_unmaps as u128),
            ("staging_allocations", t.staging_allocations as u128),
            ("staging_reuses", t.staging_reuses as u128),
            ("staging_retirements", t.staging_retirements as u128),
            ("peak_outstanding", t.peak_outstanding as u128),
            ("timed_operations", t.timed_operations as u128),
            ("encode_ns", t.total_times.encode.as_nanos()),
            ("submit_ns", t.total_times.submission.as_nanos()),
            ("wait_ns", t.total_times.submission_wait.as_nanos()),
            ("map_ns", t.total_times.mapping.as_nanos()),
            ("repack_ns", t.total_times.repack.as_nanos()),
            ("controlled_release", u128::from(t.controlled_shutdown)),
            ("uncertain_release", u128::from(t.uncertain_shutdown)),
        ] {
            writeln!(text, "{k}={v}").unwrap();
        }
    }
    text.push_str("telemetry_error=0\n");
    if HEADER.len() + text.len() + "complete=1\n".len() > MAX_REPORT_BYTES {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    writer.write_all(text.as_bytes())?;
    // A short/failed earlier write never reaches the completeness marker.
    writer.flush()?;
    writer.write_all(b"complete=1\n")
}

//! Bounded experimental native launch and read-only cohort diagnostics.
//! This module has no publication, physical-settlement or lifecycle authority.
use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Instant;

use roomie_worker_supervisor::machine::{
    Failure, JobState, Outcome, Phase, Shutdown, Transmission,
};

use crate::managed_vulkan_evidence::{self as evidence, Capture, Cohort, FileIdentity};
use crate::presentation::gpu_handoff::JobIdentity;

const EVIDENCE_FILE: &str = "owner.evidence";
const UNAVAILABLE_ICD: &[u8] = br#"{"file_format_version":"1.0.0","ICD":{"library_path":"/nonexistent/desktoproomie-unavailable-vulkan.so","api_version":"1.3.0"}}"#;
const UNAVAILABLE_LIBRARY: &str = "/nonexistent/desktoproomie-unavailable-vulkan.so";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvidenceMode {
    Correctness,
    ExpectedInitializationRejection,
}

fn validate_unavailable_icd(path: &Path) -> Result<(), String> {
    let mut bytes = [0; UNAVAILABLE_ICD.len() + 3];
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(NOFOLLOW | NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("unavailable ICD manifest must be a regular file".into());
    }
    let mut size = 0;
    while size < bytes.len() {
        let count = file.read(&mut bytes[size..]).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        size += count;
    }
    let manifest = &bytes[..size];
    if manifest.strip_suffix(b"\n").unwrap_or(manifest) != UNAVAILABLE_ICD
        || !matches!(fs::symlink_metadata(UNAVAILABLE_LIBRARY), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
    {
        return Err(
            "expected rejection requires the accepted unavailable-ICD manifest and absent library"
                .into(),
        );
    }
    Ok(())
}

// Linux flags, used through std only; the accepted managed host is Linux/X11.
const NOFOLLOW: i32 = 0o400000;
const NONBLOCK: i32 = 0o4000;
const DIRECTORY: i32 = 0o200000;
pub const MAX_REPORT_BYTES: usize = 16_384;
const HEADER: &str = "roomie_acceptance_report=1\ncomplete=0\n";
const NUMBERS: [&str; 26] = [
    "initialization_complete",
    "initialization_ns",
    "device_initialization_ns",
    "pipeline_initialization_ns",
    "submission_attempts",
    "accepted_submissions",
    "confirmed_completions",
    "native_gpu_completions",
    "map_requests",
    "map_callbacks",
    "successful_maps",
    "unmaps",
    "staging_allocations",
    "staging_reuses",
    "staging_retirements",
    "peak_outstanding",
    "timed_operations",
    "encode_ns",
    "submit_ns",
    "wait_ns",
    "map_ns",
    "repack_ns",
    "controlled_release",
    "uncertain_release",
    "telemetry_available",
    "telemetry_error",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterClass {
    Software,
    Hardware,
}
impl AdapterClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Hardware => "hardware",
        }
    }
}

/// Constructed only after complete CLI validation and exclusive private mkdir.
/// Paths remain literal; the supervisor continues to clear the child's env.
#[derive(Debug)]
pub struct NativeVulkanLaunch {
    worker: PathBuf,
    driver: PathBuf,
    adapter: AdapterClass,
    session: u64,
    duration: u64,
    report: PrivateReport,
    evidence: Option<EvidenceMode>,
    worker_identity: Option<FileIdentity>,
    driver_identity: Option<FileIdentity>,
}
impl NativeVulkanLaunch {
    /// Integer seconds match the existing owner CLI/deadline representation.
    /// Every protocol u64 session value, including zero, is valid.
    /// Optional literals: `--evidence correctness` or
    /// `--evidence expected-initialization-rejection`. Omission uses I1-R1 mode.
    pub fn parse(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Self, String> {
        let mut options = BTreeMap::new();
        let mut args = args.into_iter();
        while let Some(key) = args.next() {
            let key = key.as_ref();
            if ![
                "--worker",
                "--adapter",
                "--driver",
                "--report-directory",
                "--session",
                "--duration",
                "--evidence",
            ]
            .contains(&key)
            {
                return Err(format!("unknown native launch option {key}"));
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {key}"))?;
            if options
                .insert(key.to_owned(), value.as_ref().to_owned())
                .is_some()
            {
                return Err(format!("duplicate native launch option {key}"));
            }
        }
        let value = |key: &str| {
            options
                .get(key)
                .map(String::as_str)
                .ok_or_else(|| format!("missing required option {key}"))
        };
        let worker = absolute_file(Path::new(value("--worker")?), true)?;
        let adapter = match value("--adapter")? {
            "software" => AdapterClass::Software,
            "hardware" => AdapterClass::Hardware,
            _ => return Err("adapter must be software or hardware".into()),
        };
        let requested_driver = Path::new(value("--driver")?);
        driver_text(requested_driver)?;
        let driver = absolute_file(requested_driver, false)?;
        driver_text(&driver)?;
        if driver
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            return Err("driver must be a regular JSON manifest".into());
        }
        File::open(&driver).map_err(|e| format!("unreadable driver manifest: {e}"))?;
        let session = decimal(value("--session")?, "session")?;
        let duration = decimal(value("--duration")?, "duration")?;
        if duration == 0 {
            return Err("duration must be positive integer seconds".into());
        }
        crate::probe_duration_deadline(Instant::now(), Some(duration))
            .map_err(|e| e.to_string())?;
        let evidence = match options.get("--evidence").map(String::as_str) {
            None => None,
            Some("correctness") => Some(EvidenceMode::Correctness),
            Some("expected-initialization-rejection") => {
                validate_unavailable_icd(&driver)?;
                Some(EvidenceMode::ExpectedInitializationRejection)
            }
            _ => {
                return Err(
                    "evidence must be correctness or expected-initialization-rejection".into(),
                )
            }
        };
        let (worker_identity, driver_identity) = if evidence.is_some() {
            (Some(file_identity(&worker)?), Some(file_identity(&driver)?))
        } else {
            (None, None)
        };
        // Last: invalid arguments must never create a report directory.
        let report = PrivateReport::create(Path::new(value("--report-directory")?))?;
        Ok(Self {
            worker,
            driver,
            adapter,
            session,
            duration,
            report,
            evidence,
            worker_identity,
            driver_identity,
        })
    }
    pub fn worker(&self) -> &Path {
        &self.worker
    }
    pub fn driver(&self) -> &Path {
        &self.driver
    }
    pub fn adapter(&self) -> AdapterClass {
        self.adapter
    }
    pub fn session(&self) -> u64 {
        self.session
    }
    pub fn duration_seconds(&self) -> u64 {
        self.duration
    }
    pub fn report_directory(&self) -> &Path {
        &self.report.path
    }
    pub fn worker_args(&self) -> [String; 6] {
        [
            "--adapter".into(),
            self.adapter.as_str().into(),
            "--driver".into(),
            self.driver.to_str().expect("validated UTF-8").into(),
            "--acceptance-report".into(),
            self.report
                .path
                .join("worker.telemetry")
                .to_str()
                .expect("validated UTF-8")
                .into(),
        ]
    }
    pub(crate) fn validate_before_owner(&self) -> Result<(), String> {
        self.report.validate()?;
        self.validate_launch_identities()?;
        if self.evidence == Some(EvidenceMode::ExpectedInitializationRejection) {
            validate_unavailable_icd(&self.driver)?;
        }
        crate::probe_duration_deadline(Instant::now(), Some(self.duration))
            .map_err(|e| e.to_string())?;
        // A worker report cannot predate this owned run.
        match fs::symlink_metadata(self.report.path.join("worker.telemetry")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            _ => Err("worker telemetry already exists or cannot be checked".into()),
        }
    }
    fn validate_launch_identities(&self) -> Result<(), String> {
        if let (Some(worker), Some(driver)) = (self.worker_identity, self.driver_identity) {
            if file_identity(&self.worker)? != worker || file_identity(&self.driver)? != driver {
                return Err("worker executable or ICD manifest identity changed".into());
            }
        }
        Ok(())
    }

    pub(crate) fn evidence_mode(&self) -> Option<EvidenceMode> {
        self.evidence
    }

    pub(crate) fn classify(
        &self,
        capture: Option<Capture>,
        rejection: (bool, Option<i32>),
        host_error: Option<String>,
        reaping_error: Option<String>,
        observation: OwnerObservation,
    ) -> NativeSummary {
        let telemetry = self.read_telemetry();
        let negative = self.evidence == Some(EvidenceMode::ExpectedInitializationRejection);
        let proof = owner_proof(&observation, rejection);
        let mut summary = if negative {
            classify_rejection(
                self.session,
                host_error,
                reaping_error,
                observation,
                telemetry,
                rejection,
            )
        } else {
            classify_native(
                self.adapter,
                self.session,
                host_error,
                reaping_error,
                observation,
                telemetry,
            )
        };
        if let Some(capture) = capture {
            if let Err(error) = self.validate_launch_identities() {
                summary
                    .failures
                    .push(format!("evidence provenance: {error}"));
            }
            let cohort = Cohort {
                negative,
                hardware: self.adapter == AdapterClass::Hardware,
                session: self.session,
                completions: summary
                    .telemetry
                    .as_ref()
                    .ok()
                    .and_then(|report| {
                        u64::try_from(report.number("native_gpu_completions").ok()?).ok()
                    })
                    .unwrap_or(0),
                owner_proof: proof,
                worker: self.worker_identity,
                driver: self.driver_identity,
                last_job: summary
                    .observation
                    .supervisor
                    .and_then(|owner| owner.job)
                    .map(|status| JobIdentity {
                        session: self.session,
                        epoch: status.job.epoch,
                        serial: status.job.serial,
                        frame: crate::presentation::FrameIdentity {
                            generation: status.job.generation,
                            sequence: status.job.sequence,
                        },
                    }),
            };
            if let Some(error) = capture.failure() {
                summary.failures.push(format!("evidence: {error}"));
            }
            let result = self
                .report
                .write_evidence(&capture, cohort, summary.successful());
            match result {
                Ok(()) if summary.successful() => match self.report.verify_evidence() {
                    Ok(classification) => eprintln!("[MANAGED EVIDENCE] {classification}"),
                    Err(error) => summary.failures.push(format!("evidence: {error}")),
                },
                Ok(()) => {}
                Err(error) => summary.failures.push(format!("evidence write: {error}")),
            }
        }
        summary
    }

    pub fn read_telemetry(&self) -> Result<NativeTelemetry, String> {
        self.report.read()
    }
}
fn file_identity(path: &Path) -> Result<FileIdentity, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() == 0 {
        return Err("launch input is no longer a nonempty regular file".into());
    }
    Ok(FileIdentity {
        device: meta.dev(),
        inode: meta.ino(),
        bytes: meta.len(),
        changed_seconds: meta.ctime(),
        changed_nanos: u32::try_from(meta.ctime_nsec()).map_err(|e| e.to_string())?,
    })
}
fn decimal(text: &str, label: &str) -> Result<u64, String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("invalid {label}: expected decimal u64"));
    }
    text.parse().map_err(|_| format!("{label} exceeds u64"))
}
fn absolute_file(path: &Path, executable: bool) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.to_str().is_none_or(|s| s.contains('\0')) {
        return Err("expected an explicit absolute UTF-8 file path".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("file unavailable: {e}"))?;
    let meta = canonical.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file()
        || (executable && meta.permissions().mode() & 0o111 == 0)
        || canonical.to_str().is_none()
    {
        return Err("expected a regular file with required executable access".into());
    }
    Ok(canonical)
}
fn driver_text(path: &Path) -> Result<(), String> {
    let text = path.to_str().ok_or("driver path must be UTF-8")?;
    if !path.is_absolute() || text.len() > 4096 || text.contains([':', '\0']) {
        return Err("driver must be one absolute manifest path".into());
    }
    Ok(())
}

#[derive(Debug)]
struct PrivateReport {
    path: PathBuf,
    directory: File,
    uid: u32,
}
impl PrivateReport {
    fn create(path: &Path) -> Result<Self, String> {
        let parent = path.parent().ok_or("report directory needs a parent")?;
        let name = path
            .file_name()
            .ok_or("report directory needs a fresh name")?;
        if !path.is_absolute()
            || path.to_str().is_none_or(|s| s.contains('\0'))
            || parent
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_os_str()
                != parent.as_os_str()
            || parent.join(name).as_os_str() != path.as_os_str()
        {
            return Err("report directory must have an absolute canonical destination".into());
        }
        // /proc/self follows the calling process and records its effective uid.
        let uid = fs::metadata("/proc/self").map_err(|e| e.to_string())?.uid();
        for ancestor in parent.ancestors() {
            let m = fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
            let sticky_root = m.uid() == 0 && m.mode() & 0o1000 != 0;
            if !m.is_dir()
                || (m.uid() != 0 && m.uid() != uid)
                || (m.mode() & 0o022 != 0 && !sticky_root)
            {
                return Err("unsafe report parent ownership or permissions".into());
            }
        }
        let parent_fd = OpenOptions::new()
            .read(true)
            .custom_flags(DIRECTORY | NOFOLLOW)
            .open(parent)
            .map_err(|e| e.to_string())?;
        // Pin creation to the checked parent inode even if its pathname moves.
        // mkdir is exclusive and applies 0700 at creation (umask can only narrow).
        let pinned = PathBuf::from(format!("/proc/self/fd/{}", parent_fd.as_raw_fd())).join(name);
        DirBuilder::new()
            .mode(0o700)
            .create(&pinned)
            .map_err(|e| format!("exclusive private report mkdir: {e}"))?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(DIRECTORY | NOFOLLOW)
            .open(&pinned)
            .map_err(|e| e.to_string())?;
        let result = Self {
            path: path.into(),
            directory,
            uid,
        };
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<(), String> {
        let held = self.directory.metadata().map_err(|e| e.to_string())?;
        let live = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !live.is_dir()
            || live.uid() != self.uid
            || live.mode() & 0o077 != 0
            || live.dev() != held.dev()
            || live.ino() != held.ino()
            || self
                .path
                .canonicalize()
                .map_err(|e| e.to_string())?
                .as_os_str()
                != self.path.as_os_str()
        {
            return Err("private report directory identity or permissions changed".into());
        }
        Ok(())
    }
    fn pinned(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd())).join(name)
    }

    fn open_private(&self, name: &str, maximum: u64) -> Result<File, String> {
        self.validate()?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW | NONBLOCK)
            .open(self.pinned(name))
            .map_err(|e| format!("report unavailable: {e}"))?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file()
            || meta.uid() != self.uid
            || meta.mode() & 0o077 != 0
            || meta.nlink() != 1
            || meta.len() > maximum
        {
            return Err("unsafe or oversized report file".into());
        }
        Ok(file)
    }

    fn read(&self) -> Result<NativeTelemetry, String> {
        let file = self.open_private("worker.telemetry", MAX_REPORT_BYTES as u64)?;
        let mut bytes = Vec::new();
        file.take(MAX_REPORT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        self.validate()?;
        NativeTelemetry::parse(&bytes)
    }

    fn write_evidence(
        &self,
        capture: &Capture,
        cohort: Cohort,
        complete: bool,
    ) -> Result<(), String> {
        self.validate()?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(NOFOLLOW | NONBLOCK)
            .open(self.pinned(EVIDENCE_FILE))
            .map_err(|e| e.to_string())?;
        let mut writer = BufWriter::new(file);
        capture
            .write(&mut writer, cohort, complete)
            .map_err(|e| e.to_string())?;
        writer.get_ref().sync_all().map_err(|e| e.to_string())?;
        self.validate()
    }

    fn verify_evidence(&self) -> Result<String, String> {
        let report = self.read()?;
        let file = self.open_private(EVIDENCE_FILE, evidence::MAX_BYTES as u64)?;
        let result = evidence::verify(
            BufReader::new(file.take(evidence::MAX_BYTES as u64 + 1)),
            |cohort| {
                if cohort.negative {
                    report.unavailable_native()?;
                } else {
                    report.normal_native(if cohort.hardware {
                        AdapterClass::Hardware
                    } else {
                        AdapterClass::Software
                    })?;
                }
                if report.number("native_gpu_completions")? != u128::from(cohort.completions) {
                    return Err("retained evidence contradicts worker telemetry".into());
                }
                Ok(())
            },
        )?;
        self.validate()?;
        Ok(result)
    }
}

/// Accepted worker report, independently bounded/parsed without linking worker
/// or harness crates. It remains diagnostic evidence only.
#[derive(Debug)]
pub struct NativeTelemetry {
    fields: BTreeMap<String, String>,
}
impl NativeTelemetry {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_REPORT_BYTES {
            return Err("telemetry size bound exceeded".into());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "telemetry is not UTF-8")?;
        let body = text
            .strip_prefix(HEADER)
            .and_then(|s| s.strip_suffix("complete=1\n"))
            .ok_or("incomplete telemetry/version marker")?;
        if !body.ends_with('\n') {
            return Err("truncated telemetry field".into());
        }
        let mut fields = BTreeMap::new();
        for line in body.split_terminator('\n') {
            let (key, value) = line.split_once('=').ok_or("malformed telemetry field")?;
            if fields.len() >= 34
                || value.len() > 1024
                || key.is_empty()
                || !(NUMBERS.contains(&key)
                    || [
                        "kind_hex",
                        "exit",
                        "adapter_hex",
                        "device_type_hex",
                        "backend_hex",
                        "driver_hex",
                    ]
                    .contains(&key))
                || fields.insert(key.to_owned(), value.to_owned()).is_some()
            {
                return Err("unknown, duplicate or unbounded telemetry field".into());
            }
        }
        let report = Self { fields };
        report.decoded("kind_hex")?;
        if !["Controlled", "Failed", "Uncertain"].contains(&report.value("exit")?) {
            return Err("invalid worker exit".into());
        }
        for key in ["telemetry_available", "telemetry_error"] {
            if report.number(key)? > 1 {
                return Err(format!("invalid boolean {key}"));
            }
        }
        if report.number("telemetry_error")? != 0 {
            return Err("worker telemetry error".into());
        }
        if report.number("telemetry_available")? == 1 {
            for key in NUMBERS {
                let number = report.number(key)?;
                let maximum = if key.ends_with("_ns") {
                    std::time::Duration::MAX.as_nanos()
                } else {
                    u128::from(u64::MAX)
                };
                if number > maximum {
                    return Err(format!("telemetry exceeds worker field range: {key}"));
                }
            }
            for key in [
                "initialization_complete",
                "controlled_release",
                "uncertain_release",
            ] {
                if report.number(key)? > 1 {
                    return Err(format!("invalid boolean {key}"));
                }
            }
            let n = |key| report.number(key);
            let initialization = n("initialization_ns")?;
            if n("initialization_complete")? == 1
                && n("device_initialization_ns")?
                    .checked_add(n("pipeline_initialization_ns")?)
                    .is_none_or(|parts| parts > initialization)
            {
                return Err("contradictory initialization timings".into());
            }
            if n("confirmed_completions")? > n("accepted_submissions")?
                || n("accepted_submissions")? > n("submission_attempts")?
                || n("successful_maps")? > n("map_callbacks")?
                || n("map_callbacks")? > n("map_requests")?
                || n("peak_outstanding")? > 1
                || n("native_gpu_completions")? > n("confirmed_completions")?
                || n("controlled_release")? + n("uncertain_release")? > 1
            {
                return Err("contradictory telemetry counters".into());
            }
            for key in [
                "adapter_hex",
                "device_type_hex",
                "backend_hex",
                "driver_hex",
            ] {
                if report.fields.contains_key(key) {
                    report.decoded(key)?;
                }
            }
        } else if report.fields.len() != 4 {
            return Err("unavailable telemetry contains contradictory metrics".into());
        }
        Ok(report)
    }
    fn value(&self, key: &str) -> Result<&str, String> {
        self.fields
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing telemetry {key}"))
    }
    pub fn number(&self, key: &str) -> Result<u128, String> {
        let text = self.value(key)?;
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("invalid numeric telemetry {key}"));
        }
        text.parse()
            .map_err(|_| format!("overflowed telemetry {key}"))
    }
    pub fn decoded(&self, key: &str) -> Result<String, String> {
        let text = self.value(key)?;
        if text.len() > 1024
            || !text.len().is_multiple_of(2)
            || !text.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(format!("invalid hex telemetry {key}"));
        }
        let bytes = text
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        String::from_utf8(bytes).map_err(|_| format!("invalid decoded UTF-8 {key}"))
    }
    fn unavailable_native(&self) -> Result<(), String> {
        if self.decoded("kind_hex")? != "native-vulkan"
            || self.value("exit")? != "Controlled"
            || self.number("telemetry_available")? != 1
            || self.number("controlled_release")? != 1
            || self.number("uncertain_release")? != 0
        {
            return Err("missing controlled native initialization rejection telemetry".into());
        }
        for key in NUMBERS[..NUMBERS.len() - 4]
            .iter()
            .copied()
            .filter(|key| *key != "initialization_ns")
        {
            if self.number(key)? != 0 {
                return Err(format!("initialization rejection has nonzero {key}"));
            }
        }
        Ok(())
    }

    fn normal_native(&self, adapter: AdapterClass) -> Result<(), String> {
        if self.decoded("kind_hex")? != "native-vulkan"
            || self.value("exit")? != "Controlled"
            || self.number("telemetry_available")? != 1
            || self.number("initialization_complete")? != 1
            || self.number("controlled_release")? != 1
            || self.number("uncertain_release")? != 0
        {
            return Err("native initialization or controlled release not demonstrated".into());
        }
        if self.decoded("backend_hex")? != "Vulkan" {
            return Err("observed backend is not Vulkan".into());
        }
        let class = self.decoded("device_type_hex")?;
        if !match adapter {
            AdapterClass::Software => class == "Cpu",
            AdapterClass::Hardware => ["DiscreteGpu", "IntegratedGpu"].contains(&class.as_str()),
        } {
            return Err("observed adapter class mismatch".into());
        }
        if self.decoded("adapter_hex")?.is_empty() || self.decoded("driver_hex")?.is_empty() {
            return Err("missing actual native adapter/driver identity".into());
        }
        let completed = self.number("native_gpu_completions")?;
        if completed == 0 {
            return Err("zero native GPU completions".into());
        }
        for key in [
            "submission_attempts",
            "accepted_submissions",
            "confirmed_completions",
            "map_requests",
            "map_callbacks",
            "successful_maps",
            "unmaps",
            "timed_operations",
        ] {
            if self.number(key)? != completed {
                return Err(format!("inconsistent completed native operations: {key}"));
            }
        }
        if self.number("staging_allocations")? != 1
            || self.number("staging_reuses")? != completed - 1
            || self.number("staging_retirements")? != 0
            || self.number("peak_outstanding")? != 1
        {
            return Err("inconsistent native staging evidence".into());
        }
        Ok(())
    }
}

/// Snapshots of accepted authorities after host destruction and reaping.
#[derive(Debug, Default)]
pub struct OwnerObservation {
    pub supervisor: Option<Outcome>,
    pub outstanding: Option<JobIdentity>,
    pub unresolved: Option<JobIdentity>,
}
impl OwnerObservation {
    pub(crate) fn of(managed: &crate::managed_supervisor::ManagedSupervisor) -> Self {
        Self {
            supervisor: Some(managed.outcome()),
            outstanding: managed.outstanding(),
            unresolved: managed.unresolved(),
        }
    }
}
#[derive(Debug)]
pub struct NativeSummary {
    pub observation: OwnerObservation,
    pub telemetry: Result<NativeTelemetry, String>,
    /// Ordered diagnostics: host first, then cleanup/reaping, supervisor and report.
    pub failures: Vec<String>,
}
impl NativeSummary {
    pub fn successful(&self) -> bool {
        self.failures.is_empty()
    }
    pub fn into_result(self) -> Result<Self, NativeFailure> {
        if self.successful() {
            Ok(self)
        } else {
            Err(NativeFailure(Box::new(self)))
        }
    }
}
#[derive(Debug)]
pub struct NativeFailure(pub Box<NativeSummary>);
impl std::fmt::Display for NativeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "native cohort failed: {}",
            self.0.failures.join("; secondary: ")
        )
    }
}
impl std::error::Error for NativeFailure {}

/// Pure diagnostic classification, shared by the real native entry and tests.
/// No report can discharge D1 debt, prove an X11 commitment or grant admission.
pub fn classify_native(
    adapter: AdapterClass,
    session: u64,
    host_error: Option<String>,
    reaping_error: Option<String>,
    observation: OwnerObservation,
    telemetry: Result<NativeTelemetry, String>,
) -> NativeSummary {
    let mut failures = Vec::new();
    if let Some(error) = host_error {
        failures.push(format!("host: {error}"));
    }
    if let Some(error) = reaping_error {
        failures.push(format!("reaping: {error}"));
    }
    match observation.supervisor {
        Some(o)
            if o.session == session
                && o.initialize_transmitted
                && o.phase == Phase::Exited
                && o.failure.is_none()
                && o.unresolved.is_none()
                && o.stopped
                && o.reaped
                && !o.termination_requested
                && o.shutdown == Shutdown::Confirmed
                && o.job.is_some_and(|j| {
                    matches!(
                        (j.state, j.transmission),
                        (JobState::PhysicallySettled, Transmission::Complete)
                            | (JobState::RejectedBeforeDispatch, Transmission::Unsent)
                    )
                }) => {}
        other => failures.push(format!(
            "missing or unsuccessful supervisor outcome: {other:?}"
        )),
    }
    if observation.outstanding.is_some() || observation.unresolved.is_some() {
        failures.push("D1 physical work remains outstanding or unresolved".into());
    }
    match &telemetry {
        Ok(report) => {
            if let Err(error) = report.normal_native(adapter) {
                failures.push(error);
            }
            if let (Some(job), Ok(completions)) = (
                observation.supervisor.and_then(|outcome| outcome.job),
                report.number("native_gpu_completions"),
            ) {
                // Necessary consistency bound only: this is no per-job native
                // completion, X11 commitment or report-authenticity proof.
                let serial = u128::from(job.job.serial);
                let contradictory = match (job.state, job.transmission) {
                    (JobState::PhysicallySettled, Transmission::Complete) => completions > serial,
                    (JobState::RejectedBeforeDispatch, Transmission::Unsent) => {
                        completions >= serial
                    }
                    _ => false, // Other dispositions already fail the supervisor checks.
                };
                if contradictory {
                    failures.push(format!(
                        "native completion count contradicts final job serial: completions={completions}, serial={serial}, state={:?}, transmission={:?}",
                        job.state, job.transmission,
                    ));
                }
            }
        }
        Err(error) => failures.push(format!("report: {error}")),
    }
    NativeSummary {
        observation,
        telemetry,
        failures,
    }
}

pub(crate) fn unavailable_adapter(diagnostic: Option<&str>) -> bool {
    diagnostic.is_some_and(|text| {
        text == "required Vulkan adapter unavailable"
            || text.starts_with("Vulkan adapter unavailable:")
    })
}

// The bits preserve the independent accepted shutdown facts for offline checking.
fn owner_proof(owner: &OwnerObservation, rejection: (bool, Option<i32>)) -> u8 {
    let Some(o) = owner.supervisor else {
        return 0;
    };
    let mut proof = 0u8;
    for (bit, value) in [
        o.initialize_transmitted,
        o.phase == Phase::Exited,
        o.stopped,
        o.reaped,
        !o.termination_requested,
        o.shutdown == Shutdown::Confirmed,
        rejection.1 == Some(0),
        rejection.0 && o.failure == Some(Failure::BackendFault),
    ]
    .into_iter()
    .enumerate()
    {
        proof |= u8::from(value) << bit;
    }
    proof
}

fn classify_rejection(
    session: u64,
    host: Option<String>,
    reaping: Option<String>,
    observation: OwnerObservation,
    telemetry: Result<NativeTelemetry, String>,
    rejection: (bool, Option<i32>),
) -> NativeSummary {
    let mut failures = Vec::new();
    if let Some(error) = host {
        failures.push(format!("host: {error}"));
    }
    if let Some(error) = reaping {
        failures.push(format!("reaping: {error}"));
    }
    if owner_proof(&observation, rejection) != u8::MAX
        || !observation
            .supervisor
            .is_some_and(|o| o.session == session && o.job.is_none() && o.unresolved.is_none())
        || observation.outstanding.is_some()
        || observation.unresolved.is_some()
    {
        failures.push("expected correlated unavailable-adapter rejection and controlled reaping were not observed".into());
    }
    match &telemetry {
        Ok(report) => {
            if let Err(error) = report.unavailable_native() {
                failures.push(error);
            }
        }
        Err(error) => failures.push(format!("report: {error}")),
    }
    NativeSummary {
        observation,
        telemetry,
        failures,
    }
}

/// Verify a private report directory without connecting to X11 or running a worker.
pub fn verify_evidence(directory: &Path) -> Result<String, String> {
    let uid = fs::metadata("/proc/self").map_err(|e| e.to_string())?.uid();
    let report = PrivateReport {
        path: directory.to_owned(),
        directory: OpenOptions::new()
            .read(true)
            .custom_flags(DIRECTORY | NOFOLLOW)
            .open(directory)
            .map_err(|e| e.to_string())?,
        uid,
    };
    report.verify_evidence()
}

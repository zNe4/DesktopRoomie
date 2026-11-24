//! Independently parsed bounded reports; never used to settle a coordinator.
use std::{
    collections::BTreeMap,
    fs::{DirBuilder, File, OpenOptions},
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
pub const MAX_REPORT_BYTES: usize = 16_384;

#[derive(Debug)]
pub struct RunDirectory(pub PathBuf);
impl RunDirectory {
    pub fn create() -> Result<Self, String> {
        for _ in 0..32 {
            let mut nonce = [0_u8; 16];
            File::open("/dev/urandom")
                .and_then(|mut file| file.read_exact(&mut nonce))
                .map_err(|e| e.to_string())?;
            let nonce = u128::from_ne_bytes(nonce);
            let path = std::env::temp_dir().join(format!(
                "roomie-acceptance-{}-{nonce:032x}",
                std::process::id(),
            ));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path.canonicalize().map_err(|e| e.to_string())?)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("private directory name attempts exhausted".into())
    }
    pub fn report_path(&self) -> PathBuf {
        self.0.join("worker.telemetry")
    }
}
#[derive(Debug)]
pub struct Report {
    fields: BTreeMap<String, String>,
}
impl Report {
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| format!("telemetry unavailable: {e}"))?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        // SAFETY: geteuid is a side-effect-free identity query.
        if !metadata.is_file()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.len() > MAX_REPORT_BYTES as u64
        {
            return Err("unsafe or oversized telemetry file".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_REPORT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::parse(&bytes)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_REPORT_BYTES {
            return Err("telemetry size bound exceeded".into());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "invalid telemetry UTF-8")?;
        if !text.starts_with("roomie_acceptance_report=1\ncomplete=0\n")
            || !text.ends_with("\ncomplete=1\n")
        {
            return Err("incomplete telemetry report; counts are unknown".into());
        }
        let body = &text
            ["roomie_acceptance_report=1\ncomplete=0\n".len()..text.len() - "complete=1\n".len()];
        let mut fields = BTreeMap::new();
        for line in body.lines() {
            let (key, value) = line.split_once('=').ok_or("malformed telemetry field")?;
            if fields.len() >= 40
                || key.len() > 64
                || value.len() > 1024
                || fields.insert(key.to_owned(), value.to_owned()).is_some()
                || key == "complete"
            {
                return Err("duplicate or unbounded telemetry fields".into());
            }
        }
        let result = Self { fields };
        if result.value("telemetry_error")? != "0" {
            return Err("worker telemetry I/O error".into());
        }
        result.value("exit")?;
        match result.value("telemetry_available")? {
            "0" => {}
            "1" => {
                for key in [
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
                ] {
                    result.number(key)?;
                }
                if result.number("confirmed_completions")?
                    > result.number("accepted_submissions")?
                    || result.number("accepted_submissions")?
                        > result.number("submission_attempts")?
                    || result.number("successful_maps")? > result.number("map_callbacks")?
                    || result.number("map_callbacks")? > result.number("map_requests")?
                    || result.number("peak_outstanding")? > 1
                {
                    return Err("inconsistent telemetry counters".into());
                }
            }
            _ => return Err("invalid telemetry availability".into()),
        }
        result.kind()?;
        Ok(result)
    }
    pub fn value(&self, key: &str) -> Result<&str, String> {
        self.fields
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing telemetry field {key}"))
    }
    pub fn number(&self, key: &str) -> Result<u128, String> {
        self.value(key)?
            .parse()
            .map_err(|_| format!("invalid numeric telemetry {key}"))
    }
    pub fn decoded(&self, key: &str) -> Result<String, String> {
        let text = self.value(key)?;
        if !text.is_ascii() || !text.len().is_multiple_of(2) || text.len() > 1024 {
            return Err("invalid bounded hex field".into());
        }
        let bytes: Result<Vec<_>, _> = (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16))
            .collect();
        String::from_utf8(bytes.map_err(|_| "invalid hex")?)
            .map_err(|_| "invalid decoded UTF-8".into())
    }
    pub fn kind(&self) -> Result<String, String> {
        self.decoded("kind_hex")
    }
    pub fn exact125(&self, native: bool) -> Result<(), String> {
        self.completed(125, native)
    }
    pub fn completed(&self, frames: u128, native: bool) -> Result<(), String> {
        if self.kind()? != if native { "native-vulkan" } else { "synthetic" } {
            return Err("telemetry backend domain mismatch".into());
        }
        if self.value("telemetry_available")? != "1" || self.value("exit")? != "Controlled" {
            return Err("incomplete backend lifecycle metrics".into());
        }
        if self.number("native_gpu_completions")? != if native { frames } else { 0 } {
            return Err("native/synthetic completion accounting mismatch".into());
        }
        for (key, expected) in [
            ("submission_attempts", frames),
            ("accepted_submissions", frames),
            ("confirmed_completions", frames),
            ("map_requests", frames),
            ("map_callbacks", frames),
            ("timed_operations", frames),
            ("successful_maps", frames),
            ("unmaps", frames),
            ("staging_allocations", 1),
            ("staging_reuses", frames.saturating_sub(1)),
            ("staging_retirements", 0),
            ("peak_outstanding", u128::from(frames != 0)),
            ("controlled_release", 1),
            ("uncertain_release", 0),
        ] {
            if self.number(key)? != expected {
                return Err(format!("unexpected {key}"));
            }
        }
        Ok(())
    }
    pub fn stress(&self, stress: &crate::workload::Stress, native: bool) -> Result<(), String> {
        stress.verify()?;
        for key in [
            "adapter_hex",
            "device_type_hex",
            "backend_hex",
            "driver_hex",
        ] {
            if self.decoded(key)?.is_empty() {
                return Err(format!("missing stress identity {key}"));
            }
        }
        if native && self.decoded("backend_hex")? != "Vulkan" {
            return Err("stress native backend is not Vulkan".into());
        }
        if stress.failure.is_some() {
            return Err("stress correctness failed".into());
        }
        crate::workload::controlled(stress.shutdown.ok_or("missing stress lifecycle")?)?;
        let completed = stress.warmup.valid
            + stress.measured.valid
            + stress.warmup.obsolete
            + stress.measured.obsolete;
        if stress.warmup.dispatched + stress.measured.dispatched != completed {
            return Err("dispatched work lacks genuine frame settlement".into());
        }
        // Locally withdrawn admissions perform no backend operation. Late and
        // obsolete settled frames still require the full operation counters.
        self.completed(completed as u128, native)?;
        if self.number("initialization_complete")? != 1 {
            return Err("stress initialization metrics incomplete".into());
        }
        Ok(())
    }
    pub fn initialization_failure(
        &self,
        native: bool,
        outcome: roomie_worker_supervisor::machine::Outcome,
        exit: Option<i32>,
    ) -> Result<(), String> {
        if self.kind()? != if native { "native-vulkan" } else { "synthetic" }
            || self.value("telemetry_available")? != "1"
            || self.number("initialization_complete")? != 0
        {
            return Err("missing or mismatched unsuccessful initialization telemetry".into());
        }
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
            "staging_allocations",
            "staging_reuses",
            "staging_retirements",
            "peak_outstanding",
            "encode_ns",
            "submit_ns",
            "wait_ns",
            "map_ns",
            "repack_ns",
            "device_initialization_ns",
            "pipeline_initialization_ns",
        ] {
            if self.number(key)? != 0 {
                return Err(format!("initialization rejection has nonzero {key}"));
            }
        }
        // Worker-controlled release is distinct from supervisor acceptance.
        // Preserve the supervisor's independently observed shutdown classification.
        use roomie_worker_supervisor::machine::{Failure, Shutdown};
        if self.value("exit")? != "Controlled"
            || self.number("controlled_release")? != 1
            || self.number("uncertain_release")? != 0
            || !outcome.stopped
            || !outcome.reaped
            || exit != Some(0)
            || outcome.failure != Some(Failure::BackendFault)
            || outcome.shutdown != Shutdown::Confirmed
            || outcome.unresolved.is_some()
        {
            return Err("initialization release evidence inconsistent or uncertain".into());
        }
        Ok(())
    }
    pub fn adapter(&self, policy: &str) -> Result<(), String> {
        if self.number("initialization_complete")? != 1 {
            return Err("native initialization timings incomplete".into());
        }
        if self.decoded("backend_hex")? != "Vulkan" {
            return Err("backend is not Vulkan".into());
        }
        let class = self.decoded("device_type_hex")?;
        if (policy == "software" && class != "Cpu")
            || (policy == "hardware" && !["DiscreteGpu", "IntegratedGpu"].contains(&class.as_str()))
        {
            return Err("required adapter class mismatch".into());
        }
        self.decoded("adapter_hex")?;
        self.decoded("driver_hex")?;
        Ok(())
    }
}

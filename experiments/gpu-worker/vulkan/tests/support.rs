//! GPU-free implementation of the same operation seam, never a Vulkan fallback.
#![allow(dead_code)]
use roomie_worker_vulkan::native_contract::{checked_call, checked_release, checked_unmap};
use roomie_worker_vulkan::{engine::*, layout::Layout, settlement::*};
use std::ops::Deref;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
    Arc, Mutex,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Good,
    InitFailure,
    EncodeFailure,
    SubmitFailure,
    SubmissionWait,
    MapWait,
    MapFailed,
    Missing,
    Delayed,
    Stale(usize),
    UnmapFailure,
    ShortView,
    FaultBeforeSubmit,
    FaultAfterWait,
    FaultAfterMap,
    FaultDuringUnmap,
    ObserveFailure,
    IdleFault,
    PanicSubmit,
    PanicSecondSubmit,
    SubmitReportsError,
    UnmapReportsError,
    FailedMapUnmapReportsError,
    LossBeforeUnmap,
    DeferredDestroyedOnUnmap,
    DestroyedOnDrop,
    UnknownOnDrop,
    UnmapQueueNotEmpty,
    PanicReleasePreparation,
}
impl Mode {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "good" => Self::Good,
            "init-failure" => Self::InitFailure,
            "submit-failure" => Self::SubmitFailure,
            "map-failed" => Self::MapFailed,
            "missing" => Self::Missing,
            "map-wait" => Self::MapWait,
            "stale" => Self::Stale(0),
            "delayed" => Self::Delayed,
            "idle-fault" => Self::IdleFault,
            "panic-submit" => Self::PanicSubmit,
            "submit-reports-error" => Self::SubmitReportsError,
            "unmap-reports-error" => Self::UnmapReportsError,
            "failed-map-cleanup-error" => Self::FailedMapUnmapReportsError,
            "deferred-destroyed-unmap" => Self::DeferredDestroyedOnUnmap,
            "short-view" => Self::ShortView,
            "unknown-drop" => Self::UnknownOnDrop,
            _ => return None,
        })
    }
}
pub type Trace = Arc<Mutex<Vec<&'static str>>>;
pub struct FakeOps {
    pub mode: Mode,
    trace: Trace,
    faults: Arc<Faults>,
    bytes: Vec<u8>,
    waits: usize,
    pending: Option<(Operation, SyncSender<Completion>)>,
    live: Arc<AtomicBool>,
    created: Instant,
    observe_failure: Arc<AtomicBool>,
    submit_calls: usize,
}
impl Drop for FakeOps {
    fn drop(&mut self) {
        self.trace.lock().unwrap().push("release resources");
        if self.mode == Mode::UnknownOnDrop {
            self.faults.observe_loss(
                self.mode == Mode::DestroyedOnDrop,
                "resource Drop device loss",
            );
        }
    }
}
pub struct FakeView<'a> {
    ops: &'a FakeOps,
}
impl Deref for FakeView<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.ops.bytes
    }
}
impl Drop for FakeView<'_> {
    fn drop(&mut self) {
        self.ops.live.store(false, Ordering::SeqCst);
        self.ops.trace.lock().unwrap().push("release view");
        if self.ops.mode == Mode::LossBeforeUnmap {
            self.ops
                .faults
                .observe_loss(false, "device fault before unmap");
        }
    }
}
impl Operations for FakeOps {
    // Deliberately unrelated to owner serial. The transfer must carry the
    // opaque value unchanged into both submission-specific waits.
    type Submission = u32;
    type Commands = u32;
    type View<'a> = FakeView<'a>;
    fn encode(&mut self, phase: u32) -> Result<u32, String> {
        self.trace.lock().unwrap().push("encode");
        if self.mode == Mode::EncodeFailure {
            return Err("encode failure".into());
        }
        if self.mode == Mode::FaultBeforeSubmit {
            self.faults.record("before submission");
        }
        let layout = Layout::fixture().unwrap();
        self.bytes = vec![0xee; layout.staging - usize::from(self.mode == Mode::ShortView)];
        for row in self.bytes.chunks_mut(layout.pitch) {
            for byte in row.iter_mut().take(layout.row) {
                *byte = phase as u8;
            }
        }
        Ok(phase)
    }
    fn submit(&mut self, _: u32) -> Result<u32, String> {
        self.trace.lock().unwrap().push("submit");
        self.waits = 0;
        self.submit_calls += 1;
        if self.mode == Mode::PanicSubmit
            || self.mode == Mode::PanicSecondSubmit && self.submit_calls == 2
        {
            panic!("unexpected submit panic");
        }
        if self.mode == Mode::SubmitFailure {
            return Err("uncertain submit".into());
        }
        if self.mode == Mode::SubmitReportsError {
            return checked_call(&self.faults, || {
                self.trace
                    .lock()
                    .unwrap()
                    .push("native submit returned index");
                self.faults.record("underlying native submission error");
                Ok(0x1234)
            });
        }
        Ok(0x1234)
    }
    fn wait(&mut self, submission: &u32, remaining: Duration) -> Result<(), String> {
        assert_eq!(*submission, 0x1234);
        assert!(remaining <= WAIT_BUDGET);
        self.waits += 1;
        self.trace.lock().unwrap().push("wait");
        if let Some((operation, sender)) = self.pending.take() {
            sender
                .try_send(Completion {
                    operation,
                    result: Ok(()),
                })
                .unwrap();
        }
        if self.mode == Mode::SubmissionWait || self.mode == Mode::MapWait && self.waits == 2 {
            return Err("wait unproven".into());
        }
        if self.mode == Mode::FaultAfterWait {
            self.faults.record("after submission wait");
        }
        Ok(())
    }
    fn map(
        &mut self,
        mut operation: Operation,
        sender: SyncSender<Completion>,
    ) -> Result<(), String> {
        self.trace.lock().unwrap().push("map request");
        if self.mode == Mode::Missing {
            return Ok(());
        }
        if self.mode == Mode::Delayed {
            self.pending = Some((operation, sender));
            return Ok(());
        }
        if let Mode::Stale(field) = self.mode {
            match field {
                0 => operation.session ^= 1,
                1 => operation.job.epoch ^= 1,
                2 => operation.job.serial ^= 1,
                3 => operation.job.generation ^= 1,
                4 => operation.job.sequence ^= 1,
                _ => operation.phase ^= 1,
            }
        }
        sender
            .try_send(Completion {
                operation,
                result: if matches!(
                    self.mode,
                    Mode::MapFailed | Mode::FailedMapUnmapReportsError
                ) {
                    Err("map failed".into())
                } else {
                    Ok(())
                },
            })
            .unwrap();
        if self.mode == Mode::FaultAfterMap {
            self.faults.record("after map request");
        }
        Ok(())
    }
    fn view(&self) -> FakeView<'_> {
        assert!(!self.live.swap(true, Ordering::SeqCst));
        self.trace.lock().unwrap().push("open view");
        FakeView { ops: self }
    }
    fn unmap(&mut self) -> Result<(), String> {
        assert!(!self.live.load(Ordering::SeqCst));
        self.trace.lock().unwrap().push("unmap");
        if matches!(
            self.mode,
            Mode::UnmapReportsError
                | Mode::FailedMapUnmapReportsError
                | Mode::DeferredDestroyedOnUnmap
                | Mode::UnmapQueueNotEmpty
        ) {
            return checked_unmap(
                &self.faults,
                || {
                    self.trace
                        .lock()
                        .unwrap()
                        .push("native unmap returned normally");
                    if matches!(
                        self.mode,
                        Mode::UnmapReportsError | Mode::FailedMapUnmapReportsError
                    ) {
                        self.faults.record("underlying native unmap error");
                    }
                    Ok(())
                },
                || {
                    if self.mode == Mode::DeferredDestroyedOnUnmap {
                        self.faults
                            .observe_loss(true, "deferred invalid-device Destroyed");
                    }
                    Ok(self.mode != Mode::UnmapQueueNotEmpty)
                },
            );
        }
        if self.mode == Mode::UnmapFailure {
            return Err("unmap unproven".into());
        }
        if self.mode == Mode::FaultDuringUnmap {
            self.faults.record("during unmap");
        }
        Ok(())
    }
    fn observe(&mut self) -> Result<(), String> {
        if self.mode == Mode::ObserveFailure || self.observe_failure.load(Ordering::SeqCst) {
            return Err("native observation uncertain".into());
        }
        if self.mode == Mode::IdleFault && self.created.elapsed() >= Duration::from_millis(200) {
            self.faults.record("asynchronous idle backend fault");
        }
        Ok(())
    }
    fn prepare_release(&mut self) -> Result<(), String> {
        self.trace.lock().unwrap().push("prepare release");
        if self.mode == Mode::PanicReleasePreparation {
            panic!("injected release preparation panic");
        }
        if self.mode == Mode::DestroyedOnDrop {
            checked_release(
                &self.faults,
                || Ok(true),
                || {
                    self.faults
                        .observe_loss(true, "intentional Device::destroy");
                    Ok(())
                },
                || Ok(true),
            )
        } else {
            Ok(())
        }
    }
}
pub struct Fixture {
    pub engine: Option<Persistent<FakeOps>>,
    pub faults: Arc<Faults>,
    pub trace: Trace,
    pub mode: Mode,
    initialized: bool,
    pub observe_failure: Arc<AtomicBool>,
}
impl Fixture {
    pub fn new(mode: Mode) -> Self {
        Self {
            engine: None,
            faults: Arc::default(),
            trace: Arc::default(),
            mode,
            initialized: false,
            observe_failure: Arc::default(),
        }
    }
    pub fn telemetry(&self) -> &Telemetry {
        self.engine.as_ref().unwrap().telemetry()
    }
}
impl Backend for Fixture {
    fn initialize(&mut self) -> Result<(), String> {
        assert!(!self.initialized);
        self.initialized = true;
        self.trace.lock().unwrap().push("initialize");
        if self.mode == Mode::InitFailure {
            return Err("injected initialization failure".into());
        }
        let ops = FakeOps {
            mode: self.mode,
            trace: Arc::clone(&self.trace),
            faults: Arc::clone(&self.faults),
            bytes: vec![],
            waits: 0,
            pending: None,
            live: Arc::default(),
            created: Instant::now(),
            observe_failure: Arc::clone(&self.observe_failure),
            submit_calls: 0,
        };
        self.engine = Some(Persistent::new(
            ops,
            Arc::clone(&self.faults),
            Telemetry::default(),
        ));
        Ok(())
    }
    fn render(&mut self, operation: Operation) -> Result<Prepared, Failure> {
        self.engine.as_mut().unwrap().render(operation)
    }
    fn fault(&mut self) -> Option<String> {
        self.engine
            .as_mut()
            .and_then(Backend::fault)
            .or_else(|| self.faults.get())
    }
    fn settled(&self) -> bool {
        self.engine.as_ref().is_none_or(Backend::settled)
    }
    fn evidence(&self) -> Option<Evidence> {
        self.engine.as_ref().and_then(Persistent::evidence)
    }
    fn reject(&mut self, operation: Operation) -> Option<Evidence> {
        self.engine.as_mut()?.reject(operation)
    }
    fn release(&mut self) -> bool {
        self.engine.as_mut().is_none_or(Backend::release)
    }
}

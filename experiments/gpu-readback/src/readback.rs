use std::cell::Cell;
use std::ops::Deref;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::layout::{Layout, LayoutError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub generation: u64,
    pub phase: usize,
    // Experiment-local serial paired with the opaque wgpu submission index.
    pub submission: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Submission,
    Mapping,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    DeviceLost(String),
    Validation(String),
    OtherDeviceError(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReadbackError {
    Timeout(Step),
    #[cfg(feature = "native-vulkan")]
    Poll(String),
    NativePollFailure(String),
    Map(String),
    MissingCompletion,
    Stale {
        expected: Identity,
        actual: Identity,
    },
    Device(Fault),
    Layout(LayoutError),
    StagingUnavailable,
}

#[derive(Default)]
pub struct Faults(OnceLock<Fault>);

impl Faults {
    pub fn record(&self, fault: Fault) {
        let _ = self.0.set(fault);
    }

    pub fn check(&self) -> Result<(), ReadbackError> {
        self.0
            .get()
            .map_or(Ok(()), |fault| Err(ReadbackError::Device(fault.clone())))
    }
}

thread_local! {
    static IN_NATIVE_POLL: Cell<bool> = const { Cell::new(false) };
}

#[cfg(feature = "native-vulkan")]
pub fn install_native_poll_hook() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if !IN_NATIVE_POLL.get() {
            default(info);
        }
    }));
}

// wgpu 29 reports HAL device loss during Device::poll as a panic after it
// invokes the device-lost callback. Keep it from unwinding through the queue.
pub fn guard_native_poll<T>(faults: &Faults, poll: impl FnOnce() -> T) -> Result<T, ReadbackError> {
    let result = IN_NATIVE_POLL.with(|in_poll| {
        in_poll.set(true);
        let result = panic::catch_unwind(AssertUnwindSafe(poll));
        in_poll.set(false);
        result
    });
    match result {
        Ok(result) => Ok(result),
        Err(payload) => {
            faults.check()?;
            let details = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            Err(ReadbackError::NativePollFailure(details.into()))
        }
    }
}

#[derive(Clone, Copy)]
pub struct Deadline(pub Instant);

impl Deadline {
    pub fn remaining(self, now: Instant, step: Step) -> Result<Duration, ReadbackError> {
        if now >= self.0 {
            return Err(ReadbackError::Timeout(step));
        }
        Ok(self.0 - now)
    }
}

#[derive(Debug)]
pub struct Completion {
    pub identity: Identity,
    pub result: Result<(), String>,
}

pub struct Mapping {
    expected: Identity,
    receiver: Receiver<Completion>,
}

impl Mapping {
    pub fn new(expected: Identity) -> (Self, Sender<Completion>) {
        let (sender, receiver) = mpsc::channel();
        (Self { expected, receiver }, sender)
    }

    pub fn complete(self, deadline: Deadline, faults: &Faults) -> Result<(), ReadbackError> {
        faults.check()?;
        let remaining = deadline.remaining(Instant::now(), Step::Mapping)?;
        let report = self.receiver.recv_timeout(remaining);
        faults.check()?;
        deadline.remaining(Instant::now(), Step::Mapping)?;
        let completion = report.map_err(|error| match error {
            RecvTimeoutError::Timeout => ReadbackError::Timeout(Step::Mapping),
            RecvTimeoutError::Disconnected => ReadbackError::MissingCompletion,
        })?;
        if completion.identity != self.expected {
            return Err(ReadbackError::Stale {
                expected: self.expected,
                actual: completion.identity,
            });
        }
        completion.result.map_err(ReadbackError::Map)
    }
}

// Only the current operation owns the buffer while it is out of the slot.
// Leaving the slot in flight or retiring it both make reacquisition fail.
pub enum Staging<B> {
    Available { buffer: B, reused: bool },
    InFlight,
    Retired,
}

impl<B> Staging<B> {
    pub fn acquire(&mut self) -> Result<(B, bool), ReadbackError> {
        if !matches!(self, Self::Available { .. }) {
            return Err(ReadbackError::StagingUnavailable);
        }
        let Self::Available { buffer, reused } = std::mem::replace(self, Self::InFlight) else {
            unreachable!()
        };
        Ok((buffer, reused))
    }

    pub fn settle(
        &mut self,
        buffer: B,
        result: Result<Vec<u8>, ReadbackError>,
        retire: impl FnOnce(B),
    ) -> Result<Vec<u8>, ReadbackError> {
        match result {
            Ok(bytes) => {
                *self = Self::Available {
                    buffer,
                    reused: true,
                };
                Ok(bytes)
            }
            Err(error) => {
                *self = Self::Retired;
                retire(buffer);
                Err(error)
            }
        }
    }
}

pub trait ReadMap {
    type View<'a>: Deref<Target = [u8]>
    where
        Self: 'a;

    fn view(&self) -> Self::View<'_>;
    fn unmap(&self);
}

pub fn copy_mapped(buffer: &impl ReadMap, layout: Layout) -> Result<Vec<u8>, ReadbackError> {
    let packed = {
        let view = buffer.view();
        layout.repack(&view)
    };
    buffer.unmap();
    packed.map_err(ReadbackError::Layout)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    const KEY: Identity = Identity {
        generation: 7,
        phase: 3,
        submission: 11,
    };

    fn deadline() -> Deadline {
        Deadline(Instant::now() + Duration::from_secs(10))
    }

    #[test]
    fn rejects_each_wrong_identity_component() {
        for actual in [
            Identity {
                generation: 6,
                ..KEY
            },
            Identity { phase: 2, ..KEY },
            Identity {
                submission: 10,
                ..KEY
            },
        ] {
            let (map, sender) = Mapping::new(KEY);
            sender
                .send(Completion {
                    identity: actual,
                    result: Ok(()),
                })
                .unwrap();
            assert_eq!(
                map.complete(deadline(), &Faults::default()),
                Err(ReadbackError::Stale {
                    expected: KEY,
                    actual
                })
            );
        }
    }

    #[test]
    fn failed_map_retires_staging_and_cannot_be_reused() {
        let (map, sender) = Mapping::new(KEY);
        let mut staging = Staging::Available {
            buffer: 42,
            reused: false,
        };
        let (buffer, reused) = staging.acquire().unwrap();
        assert!(!reused);
        assert_eq!(staging.acquire(), Err(ReadbackError::StagingUnavailable));
        sender
            .send(Completion {
                identity: KEY,
                result: Err("injected map error".into()),
            })
            .unwrap();
        let result = map
            .complete(deadline(), &Faults::default())
            .map(|()| vec![0]);
        assert_eq!(
            staging.settle(buffer, result, |buffer| assert_eq!(buffer, 42)),
            Err(ReadbackError::Map("injected map error".into()))
        );
        assert_eq!(staging.acquire(), Err(ReadbackError::StagingUnavailable));
    }

    #[test]
    fn delayed_map_keeps_staging_unavailable_until_matching_completion() {
        let (map, sender) = Mapping::new(KEY);
        let mut staging = Staging::Available {
            buffer: 42,
            reused: false,
        };
        let (buffer, _) = staging.acquire().unwrap();
        assert_eq!(
            map.receiver.try_recv().unwrap_err(),
            mpsc::TryRecvError::Empty
        );
        assert_eq!(staging.acquire(), Err(ReadbackError::StagingUnavailable));
        sender
            .send(Completion {
                identity: KEY,
                result: Ok(()),
            })
            .unwrap();
        let result = map
            .complete(deadline(), &Faults::default())
            .map(|()| vec![1]);
        assert_eq!(
            staging
                .settle(buffer, result, |_| panic!("successful staging retired"))
                .unwrap(),
            [1]
        );
        assert_eq!(staging.acquire(), Ok((42, true)));
    }

    #[test]
    fn timeout_retires_staging_and_late_map_cannot_affect_new_generation() {
        let now = Instant::now();
        let expired = Deadline(now);
        assert_eq!(
            expired.remaining(now, Step::Submission),
            Err(ReadbackError::Timeout(Step::Submission))
        );
        let (old, old_sender) = Mapping::new(KEY);
        let mut staging = Staging::Available {
            buffer: 42,
            reused: false,
        };
        let (buffer, _) = staging.acquire().unwrap();
        let result = old.complete(expired, &Faults::default()).map(|()| vec![1]);
        assert_eq!(
            staging.settle(buffer, result, |_| ()),
            Err(ReadbackError::Timeout(Step::Mapping))
        );
        let next = Identity {
            generation: 8,
            submission: 1,
            ..KEY
        };
        let (new, new_sender) = Mapping::new(next);
        assert!(old_sender
            .send(Completion {
                identity: KEY,
                result: Ok(())
            })
            .is_err());
        assert!(matches!(
            new.receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        new_sender
            .send(Completion {
                identity: next,
                result: Ok(()),
            })
            .unwrap();
        assert_eq!(new.complete(deadline(), &Faults::default()), Ok(()));
        assert_eq!(staging.acquire(), Err(ReadbackError::StagingUnavailable));
    }

    #[test]
    fn callback_after_success_or_cancellation_has_no_receiver() {
        let (canceled, canceled_sender) = Mapping::new(KEY);
        drop(canceled);
        assert!(canceled_sender
            .send(Completion {
                identity: KEY,
                result: Ok(())
            })
            .is_err());
        let (map, sender) = Mapping::new(KEY);
        sender
            .send(Completion {
                identity: KEY,
                result: Ok(()),
            })
            .unwrap();
        assert_eq!(map.complete(deadline(), &Faults::default()), Ok(()));
        assert!(sender
            .send(Completion {
                identity: KEY,
                result: Ok(())
            })
            .is_err());
    }

    #[test]
    fn lost_device_and_validation_override_completed_mapping() {
        for fault in [
            Fault::DeviceLost("injected loss".into()),
            Fault::Validation("injected validation".into()),
            Fault::OtherDeviceError("injected internal error".into()),
        ] {
            let (map, sender) = Mapping::new(KEY);
            sender
                .send(Completion {
                    identity: KEY,
                    result: Ok(()),
                })
                .unwrap();
            let faults = Faults::default();
            faults.record(fault.clone());
            faults.record(Fault::Validation("secondary".into()));
            assert_eq!(
                map.complete(deadline(), &faults),
                Err(ReadbackError::Device(fault))
            );
        }
    }

    #[test]
    fn pinned_native_poll_panic_becomes_explicit_failure() {
        let faults = Faults::default();
        assert_eq!(
            guard_native_poll(&faults, || panic!("injected wgpu poll failure")),
            Err::<(), _>(ReadbackError::NativePollFailure(
                "injected wgpu poll failure".into()
            ))
        );
        let loss = Fault::DeviceLost("injected native loss".into());
        assert_eq!(
            guard_native_poll(&faults, || {
                faults.record(loss.clone());
                panic!("injected wgpu panic after device-loss callback")
            }),
            Err::<(), _>(ReadbackError::Device(loss))
        );
    }

    struct TestBuffer {
        calls: RefCell<Vec<&'static str>>,
        bytes: Vec<u8>,
    }

    struct View<'a>(&'a TestBuffer);

    impl Deref for View<'_> {
        type Target = [u8];

        fn deref(&self) -> &[u8] {
            &self.0.bytes
        }
    }

    impl Drop for View<'_> {
        fn drop(&mut self) {
            self.0.calls.borrow_mut().push("drop view");
        }
    }

    impl ReadMap for TestBuffer {
        type View<'a> = View<'a>;

        fn view(&self) -> Self::View<'_> {
            self.calls.borrow_mut().push("view");
            View(self)
        }

        fn unmap(&self) {
            self.calls.borrow_mut().push("unmap");
        }
    }

    #[test]
    fn view_drops_before_unmap_even_if_repacking_fails() {
        let layout = Layout::fixture().unwrap();
        for len in [layout.staging, layout.staging - 1] {
            let buffer = TestBuffer {
                calls: RefCell::default(),
                bytes: vec![7; len],
            };
            let result = copy_mapped(&buffer, layout);
            assert_eq!(result.is_ok(), len == layout.staging);
            assert_eq!(buffer.calls.into_inner(), ["view", "drop view", "unmap"]);
            if let Ok(packed) = result {
                assert_eq!(packed, vec![7; layout.packed]);
            }
        }
    }
}

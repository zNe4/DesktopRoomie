//! Checked native error contract, shared by Vulkan and GPU-free injected calls.
//! See NATIVE-CONTRACT.md for the exact pinned upstream paths and proof limits.
use crate::diagnostic;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    OnceLock,
};

#[derive(Default)]
pub struct Faults {
    first: OnceLock<String>,
    revision: AtomicU64,
    lost: AtomicBool,
    releasing: AtomicBool,
}
impl Faults {
    /// Diagnostic retention alone is not a native-operation error observation.
    pub fn retain(&self, text: &str) {
        let _ = self.first.set(diagnostic(text));
    }
    /// Every native error advances operational evidence, including secondary
    /// errors after the first diagnostic. Saturation cannot restore quietness.
    pub fn record(&self, text: &str) {
        self.retain(text);
        let mut before = self.revision();
        while let Some(after) = before.checked_add(1) {
            match self.revision.compare_exchange_weak(
                before,
                after,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => before = current,
            }
        }
    }
    pub fn get(&self) -> Option<String> {
        self.first.get().cloned()
    }
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub fn device_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }
    /// `destroyed` is not intrinsically intentional: core's invalid-device
    /// maintenance can also deliver Destroyed when its queue becomes empty.
    pub fn observe_loss(&self, destroyed: bool, text: &str) {
        if destroyed && self.releasing.load(Ordering::Acquire) {
            return;
        }
        self.lost.store(true, Ordering::Release);
        self.record(text);
    }
    pub(crate) fn intentional_release(&self, release: impl FnOnce()) -> bool {
        struct Restore<'a>(&'a AtomicBool, bool);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.0.store(self.1, Ordering::Release);
            }
        }
        let before = self.revision();
        let guard = Restore(&self.releasing, self.releasing.swap(true, Ordering::AcqRel));
        release();
        drop(guard);
        self.revision() == before && !self.device_lost()
    }
    fn begin(&self) -> Result<u64, String> {
        let revision = self.revision();
        // Conservative after ANY prior native error: an already invalid
        // device can make core suppress a DeviceLost error at the API handler.
        if revision != 0 || self.device_lost() {
            return Err(self.get().unwrap_or("native state uncertain".into()));
        }
        Ok(revision)
    }
    fn finish(&self, before: u64) -> Result<(), String> {
        if self.revision() != before || self.device_lost() {
            Err(self
                .get()
                .unwrap_or("native operation reported failure".into()))
        } else {
            Ok(())
        }
    }
}

/// A normal returning call is only a candidate. In particular, a submit index
/// returned here is NOT acceptance; a valid submission-specific wait is owed.
pub fn checked_call<T>(
    faults: &Faults,
    call: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let before = faults.begin()?;
    let result = call();
    faults.finish(before)?;
    result
}

/// Affirm unmap only with quiet native error/loss observations and an actual
/// post-call queue-empty poll. This also delivers deferred Destroyed callbacks
/// for an invalid device; an empty queue alone is never a loss/success proof.
pub fn checked_unmap(
    faults: &Faults,
    unmap: impl FnOnce() -> Result<(), String>,
    queue_empty: impl FnOnce() -> Result<bool, String>,
) -> Result<(), String> {
    let before = faults.begin()?;
    let result = unmap();
    faults.finish(before)?;
    result?;
    let empty = queue_empty();
    faults.finish(before)?;
    if !empty? {
        return Err("unmap settlement requires a quiet empty queue".into());
    }
    Ok(())
}

/// Native teardown preflight is OUTSIDE the intentional-loss context. Only
/// the explicit Device::destroy call and its following callback drain may
/// classify Destroyed as expected. Unknown loss is never suppressed.
pub fn checked_release(
    faults: &Faults,
    before_poll: impl FnOnce() -> Result<bool, String>,
    destroy: impl FnOnce() -> Result<(), String>,
    after_poll: impl FnOnce() -> Result<bool, String>,
) -> Result<(), String> {
    let before = faults.revision();
    let empty = before_poll()?;
    if !empty || faults.revision() != before || faults.device_lost() {
        return Err("native release preflight uncertain".into());
    }
    let mut result = Ok(());
    let quiet = faults.intentional_release(|| {
        result = destroy().and_then(|()| after_poll()).and_then(|empty| {
            if empty {
                Ok(())
            } else {
                Err("native destruction queue not empty".into())
            }
        });
    });
    if !quiet {
        return Err(faults
            .get()
            .unwrap_or("native destruction uncertain".into()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intentional_release_context_is_restored_after_unwind() {
        for unknown in [false, true] {
            let faults = Faults::default();
            assert!(std::panic::catch_unwind(|| faults.intentional_release(|| {
                faults.observe_loss(true, "expected Destroyed");
                assert!(!faults.device_lost());
                if unknown {
                    faults.observe_loss(false, "Unknown during release");
                }
                panic!("injected release unwind");
            }))
            .is_err());
            faults.observe_loss(true, "unexpected Destroyed after unwind");
            assert!(faults.device_lost());
            assert_eq!(faults.revision(), if unknown { 2 } else { 1 });
            assert_eq!(
                faults.get().as_deref(),
                Some(if unknown {
                    "Unknown during release"
                } else {
                    "unexpected Destroyed after unwind"
                })
            );
        }
    }
    #[test]
    fn destroyed_requires_intentional_release_context() {
        let faults = Faults::default();
        assert!(faults.intentional_release(|| faults.observe_loss(true, "expected destruction")));
        assert_eq!(faults.revision(), 0);
        faults.observe_loss(true, "unexpected Destroyed");
        assert!(faults.device_lost());
        assert_eq!(faults.get().as_deref(), Some("unexpected Destroyed"));
    }
    #[test]
    fn unknown_loss_is_never_ignored_during_intentional_release() {
        let faults = Faults::default();
        assert!(!faults.intentional_release(|| faults.observe_loss(false, "unexpected loss")));
        assert!(faults.device_lost());
    }
    #[test]
    fn release_preflight_cannot_relabel_unexpected_destroyed_as_intentional() {
        let faults = Faults::default();
        assert!(checked_release(
            &faults,
            || {
                faults.observe_loss(true, "unexpected preflight Destroyed");
                Ok(true)
            },
            || panic!("must not destroy an unexpectedly lost device"),
            || Ok(true)
        )
        .is_err());
        assert!(faults.device_lost());
        assert_eq!(
            faults.get().as_deref(),
            Some("unexpected preflight Destroyed")
        );
    }
    #[test]
    fn revision_saturation_cannot_make_a_native_error_quiet() {
        let faults = Faults::default();
        faults.revision.store(u64::MAX, Ordering::Release);
        faults.record("saturated error");
        assert!(checked_call(&faults, || Ok(())).is_err());
        assert_eq!(faults.revision(), u64::MAX);
    }
}

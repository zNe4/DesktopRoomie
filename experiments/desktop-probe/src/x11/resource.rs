use std::cell::Cell;

type HostResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Owned,
    Released,
    Unconfirmed,
}

/// One locally owned resource. A failed acknowledgement stays explicit; never repeat
/// a destroy whose server-side outcome is unknown.
#[derive(Debug)]
pub struct OwnedResource(Cell<State>);

impl Default for OwnedResource {
    fn default() -> Self {
        Self(Cell::new(State::Owned))
    }
}

impl OwnedResource {
    pub fn externally_destroyed(&self) {
        self.0.set(State::Released);
    }

    pub fn release_with(&self, release: impl FnOnce() -> HostResult) -> HostResult {
        match self.0.get() {
            State::Released => Ok(()),
            State::Unconfirmed => Err("Previous resource cleanup was not acknowledged".into()),
            State::Owned => {
                self.0.set(State::Unconfirmed);
                release()?;
                self.0.set(State::Released);
                Ok(())
            }
        }
    }
}

/// Run every cleanup obligation in order, retaining the first failure.
pub fn cleanup_all<const N: usize>(steps: [Box<dyn FnOnce() -> HostResult + '_>; N]) -> HostResult {
    let mut first = None;
    for step in steps {
        if let Err(error) = step() {
            eprintln!("[ERROR] Cleanup acknowledgement failed: {error}");
            if first.is_none() {
                first = Some(error);
            }
        }
    }
    first.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_is_idempotent_and_does_not_claim_unacknowledged_success() {
        let resource = OwnedResource::default();
        let calls = Cell::new(0);
        assert!(resource
            .release_with(|| {
                calls.set(calls.get() + 1);
                Err("lost connection".into())
            })
            .is_err());
        assert!(resource
            .release_with(|| {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .is_err());
        assert_eq!(calls.get(), 1);
        let external = OwnedResource::default();
        external.externally_destroyed();
        external
            .release_with(|| panic!("external destruction must not be repeated"))
            .unwrap();
    }

    #[test]
    fn failure_does_not_skip_remaining_cleanup_or_replace_first_error() {
        let calls = Cell::new(0);
        let error = cleanup_all([
            Box::new(|| {
                calls.set(1);
                Err("release failed".into())
            }) as Box<dyn FnOnce() -> HostResult>,
            Box::new(|| {
                assert_eq!(calls.get(), 1);
                calls.set(2);
                Err("destroy failed".into())
            }),
            Box::new(|| {
                assert_eq!(calls.get(), 2);
                calls.set(3);
                Ok(())
            }),
        ])
        .unwrap_err();
        assert_eq!(error.to_string(), "release failed");
        assert_eq!(calls.get(), 3);
    }
    #[test]
    fn acknowledged_cleanup_is_idempotent() {
        let resource = OwnedResource::default();
        resource.release_with(|| Ok(())).unwrap();
        resource
            .release_with(|| panic!("confirmed cleanup must not be repeated"))
            .unwrap();
    }
}

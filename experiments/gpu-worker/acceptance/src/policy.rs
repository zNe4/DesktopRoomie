//! Fixed workload policy; exact parity gets a bounded verification allowance.
use roomie_worker_supervisor::machine::Policy;
use std::time::Duration;

pub const EXACT_FRESHNESS: Duration = Duration::from_millis(1_000);
pub const LATENCY_REFERENCE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    Exact,
    Stress,
    Cancel,
    CancelBefore,
    Unsupported,
    InitFailure,
}
impl Workload {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "exact" => Ok(Self::Exact),
            "stress" => Ok(Self::Stress),
            "cancel" => Ok(Self::Cancel),
            "cancel-before" => Ok(Self::CancelBefore),
            "unsupported" => Ok(Self::Unsupported),
            "init-failure" => Ok(Self::InitFailure),
            _ => Err("invalid workload".into()),
        }
    }
    pub fn policy(self) -> Policy {
        let defaults = Policy::default();
        match self {
            Self::Exact => Policy {
                freshness: EXACT_FRESHNESS,
                ..defaults
            },
            _ => defaults,
        }
    }
}

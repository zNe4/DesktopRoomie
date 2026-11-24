//! Linux-only experimental worker owner. No GPU or X11 integration.
#![cfg(target_os = "linux")]
pub mod machine;
mod outbound;
mod process;
pub use process::{Progress, Readiness, Supervisor, WaitInterest};

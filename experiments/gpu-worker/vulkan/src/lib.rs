//! D2-B: persistent synthetic headless worker. No X11 or supervisor dependency.
pub mod config;
pub mod engine;
pub mod layout;
#[cfg(feature = "native-vulkan")]
pub mod native;
pub mod native_contract;
pub mod report;
pub mod settlement;
pub mod transport;
pub mod worker;

pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 160;

pub fn phase(elapsed: std::time::Duration) -> u32 {
    ((elapsed.as_millis() / 300) % 5) as u32
}

/// Bound before retention, preserving a UTF-8 boundary and the first cause.
pub fn diagnostic(value: &str) -> String {
    let mut end = value.len().min(roomie_worker_protocol::MAX_DIAGNOSTIC);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

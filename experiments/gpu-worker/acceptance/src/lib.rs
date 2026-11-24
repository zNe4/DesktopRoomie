//! Experimental headless acceptance. Frames enter only through Supervisor IPC.
pub mod adapter;
pub mod oracle;
pub mod policy;
pub mod scheduler;
pub mod telemetry;
pub mod workload;
pub type HostError = Box<dyn std::error::Error>;

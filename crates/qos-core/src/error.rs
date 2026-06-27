//! Core error type (no `thiserror`, so it works in `no_std`).

use alloc::string::String;
use qos_abi::JobHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// No job exists with this handle.
    JobNotFound(JobHandle),
    /// An invalid job state transition was attempted.
    InvalidState(JobHandle),
    /// No result is stored for this job.
    ResultNotFound(JobHandle),
    /// I/O error (only produced by the `std` journal path).
    Io(String),
    /// (De)serialization error (only produced by the `std` journal path).
    Serde(String),
}

impl core::fmt::Display for CoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CoreError::JobNotFound(h) => write!(f, "job not found: {}", h.0),
            CoreError::InvalidState(h) => write!(f, "invalid state transition for job: {}", h.0),
            CoreError::ResultNotFound(h) => write!(f, "result not found for job: {}", h.0),
            CoreError::Io(e) => write!(f, "io error: {}", e),
            CoreError::Serde(e) => write!(f, "serde error: {}", e),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for CoreError {}

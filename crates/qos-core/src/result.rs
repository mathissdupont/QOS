//! Job result type carried by the core runtime.

use alloc::collections::BTreeMap;
use alloc::string::String;
use qos_abi::ResultStatus;

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

/// Result of executing a quantum job: measurement counts plus status/metadata.
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct QResult {
    /// OK / ERROR.
    pub status: ResultStatus,
    /// Measurement counts: bitstring -> number of shots. May be empty on error.
    pub counts: BTreeMap<String, u64>,
    /// Free-form metadata (backend name, version, etc.).
    pub meta: String,
    /// Populated when `status == Error`.
    pub error: Option<String>,
}

impl Default for QResult {
    fn default() -> Self {
        Self {
            status: ResultStatus::Ok,
            counts: BTreeMap::new(),
            meta: String::new(),
            error: None,
        }
    }
}

impl QResult {
    pub fn ok(counts: BTreeMap<String, u64>, meta: impl Into<String>) -> Self {
        Self {
            status: ResultStatus::Ok,
            counts,
            meta: meta.into(),
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            status: ResultStatus::Error,
            counts: BTreeMap::new(),
            meta: String::new(),
            error: Some(error.into()),
        }
    }
}

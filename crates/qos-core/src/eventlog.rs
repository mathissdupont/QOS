//! Event log / journal.
//!
//! Always keeps an in-memory record of events (works in `no_std`). With the `std` feature it
//! can additionally append to and replay from a JSONL file, enabling crash recovery on the
//! host daemon. See ADR-0003.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use crate::error::CoreError;
use crate::result::QResult;
use qos_abi::{JobState, ProcSpec};

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub enum Event {
    Submitted { job_id: u64, proc: ProcSpec },
    Dispatched { job_id: u64 },
    Cancelled { job_id: u64 },
    FinishedOk { job_id: u64, result: QResult },
    FinishedErr { job_id: u64, error: String },
    State { job_id: u64, state: JobState },
}

pub struct EventLog {
    mem: Mutex<Vec<Event>>,
    #[cfg(feature = "std")]
    path: Option<String>,
}

impl EventLog {
    /// In-memory-only log (the default for the bare-metal embodiment).
    pub fn in_memory() -> Self {
        Self {
            mem: Mutex::new(Vec::new()),
            #[cfg(feature = "std")]
            path: None,
        }
    }

    /// In-memory log that also persists to a JSONL journal file (`std` only).
    #[cfg(feature = "std")]
    pub fn with_journal(path: impl Into<String>) -> Self {
        Self {
            mem: Mutex::new(Vec::new()),
            path: Some(path.into()),
        }
    }

    pub fn append(&self, ev: &Event) -> Result<(), CoreError> {
        self.mem.lock().push(ev.clone());
        #[cfg(feature = "std")]
        {
            if let Some(path) = self.path.clone() {
                self.append_file(&path, ev)?;
            }
        }
        Ok(())
    }

    /// Snapshot of all events recorded in this process.
    pub fn events(&self) -> Vec<Event> {
        self.mem.lock().clone()
    }

    #[cfg(feature = "std")]
    fn append_file(&self, path: &str, ev: &Event) -> Result<(), CoreError> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| CoreError::Io(e.to_string()))?;
        let line = serde_json::to_string(ev).map_err(|e| CoreError::Serde(e.to_string()))?;
        f.write_all(line.as_bytes())
            .map_err(|e| CoreError::Io(e.to_string()))?;
        f.write_all(b"\n").map_err(|e| CoreError::Io(e.to_string()))?;
        Ok(())
    }

    /// Replay a journal file into a list of events (`std` only).
    #[cfg(feature = "std")]
    pub fn replay(path: &str) -> Result<Vec<Event>, CoreError> {
        use std::io::BufRead;
        if !std::path::Path::new(path).exists() {
            return Ok(Vec::new());
        }
        let f = std::fs::File::open(path).map_err(|e| CoreError::Io(e.to_string()))?;
        let reader = std::io::BufReader::new(f);
        let mut events = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| CoreError::Io(e.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let ev: Event =
                serde_json::from_str(&line).map_err(|e| CoreError::Serde(e.to_string()))?;
            events.push(ev);
        }
        Ok(events)
    }
}

impl Default for EventLog {
    fn default() -> Self {
        Self::in_memory()
    }
}

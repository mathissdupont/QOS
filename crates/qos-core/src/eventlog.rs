use crate::{CoreError, JobState, QProc, QResult};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Submitted { job_id: Uuid, proc: QProc },
    Dispatched { job_id: Uuid },
    Cancelled { job_id: Uuid },
    FinishedOk { job_id: Uuid, result: QResult },
    FinishedErr { job_id: Uuid, error: String },
    State { job_id: Uuid, state: JobState },
}

pub struct EventLog {
    path: String,
}

impl EventLog {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }

    pub fn append(&self, ev: &Event) -> Result<(), CoreError> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| CoreError::Io(e.to_string()))?;

        let line = serde_json::to_string(ev).map_err(|e| CoreError::Serde(e.to_string()))?;
        f.write_all(line.as_bytes()).map_err(|e| CoreError::Io(e.to_string()))?;
        f.write_all(b"\n").map_err(|e| CoreError::Io(e.to_string()))?;
        Ok(())
    }

    pub fn replay(&self) -> Result<Vec<Event>, CoreError> {
        if !Path::new(&self.path).exists() {
            return Ok(vec![]);
        }

        let f = File::open(&self.path).map_err(|e| CoreError::Io(e.to_string()))?;
        let reader = BufReader::new(f);

        let mut events = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| CoreError::Io(e.to_string()))?;
            if line.trim().is_empty() { continue; }
            let ev: Event = serde_json::from_str(&line).map_err(|e| CoreError::Serde(e.to_string()))?;
            events.push(ev);
        }
        Ok(events)
    }
}

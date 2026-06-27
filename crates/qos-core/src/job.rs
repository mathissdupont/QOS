//! Job record and its state machine.

use crate::error::CoreError;
use qos_abi::{JobHandle, JobState, ProcSpec};

/// A quantum job tracked by the control plane.
///
/// OS analogy: this is an entry in the "process table". `ProcSpec` is the program image,
/// `JobState` is the scheduling state, and the timestamps are accounting.
#[derive(Debug, Clone)]
pub struct QJob {
    pub id: JobHandle,
    pub proc: ProcSpec,
    pub state: JobState,
    pub created_us: u64,
    pub started_us: Option<u64>,
    pub finished_us: Option<u64>,
}

impl QJob {
    pub fn new(id: JobHandle, proc: ProcSpec, now_us: u64) -> Self {
        Self {
            id,
            proc,
            state: JobState::Queued,
            created_us: now_us,
            started_us: None,
            finished_us: None,
        }
    }

    /// Apply a legal state transition, stamping the appropriate timestamp.
    pub fn transition(&mut self, next: JobState, now_us: u64) -> Result<(), CoreError> {
        use JobState::*;

        let legal = matches!(
            (self.state, next),
            (Queued, Running)
                | (Queued, Cancelled)
                | (Running, Done)
                | (Running, Failed)
                | (Running, Cancelled)
        );

        if !legal {
            return Err(CoreError::InvalidState(self.id));
        }

        match next {
            Running => self.started_us = Some(now_us),
            Done | Failed | Cancelled => self.finished_us = Some(now_us),
            Queued => {}
        }

        self.state = next;
        Ok(())
    }
}

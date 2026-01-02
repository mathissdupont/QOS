use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;


#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IrFormat {
    OpenQasm2,
    OpenQasm3,
    JsonIrV1,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QProc {
    pub name: String,
    pub ir_format: IrFormat,
    pub ir_bytes: Vec<u8>,
    pub n_qubits: u32,
    pub shots: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QJob {
    pub id: Uuid,
    pub proc: QProc,
    pub state: JobState,
    pub created_at: SystemTime,
    pub started_at: Option<SystemTime>,
    pub finished_at: Option<SystemTime>,
}

impl QJob {
    pub fn new(proc: QProc) -> Self {
        Self {
            id: Uuid::new_v4(),
            proc,
            state: JobState::Queued,
            created_at: SystemTime::now(),
            started_at: None,
            finished_at: None,
        }
    }

    pub fn transition(&mut self, next: JobState) -> Result<(), String> {
        use JobState::*;

        let ok = matches!(
            (self.state, next),
            (Queued, Running)
                | (Queued, Cancelled)
                | (Running, Done)
                | (Running, Failed)
                | (Running, Cancelled)
        );

        if !ok {
            return Err(format!("Invalid transition: {:?} -> {:?}", self.state, next));
        }

        // Zaman damgalarını doğru yerde basıyoruz (kernel disiplini)
        match next {
            Running => self.started_at = Some(SystemTime::now()),
            Done | Failed | Cancelled => self.finished_at = Some(SystemTime::now()),
            Queued => {}
        }

        self.state = next;
        Ok(())
    }
}

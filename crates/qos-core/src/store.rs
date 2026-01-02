use crate::job::QJob;
use crate::result::QResult;
use std::collections::HashMap;
use uuid::Uuid;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("job not found: {0}")]
    JobNotFound(Uuid),

    #[error("invalid state transition for job: {0}")]
    InvalidState(Uuid),

    #[error("result not found for job: {0}")]
    ResultNotFound(Uuid),

    #[error("io error: {0}")]
    Io(String),

    #[error("serde error: {0}")]
    Serde(String),
}

pub struct JobStore {
    jobs: HashMap<Uuid, QJob>,
    results: HashMap<Uuid, QResult>,
}

impl JobStore {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            results: HashMap::new(),
        }
    }

    pub fn insert(&mut self, job: QJob) {
        self.jobs.insert(job.id, job);
    }

    pub fn get(&self, id: Uuid) -> Result<&QJob, CoreError> {
        self.jobs.get(&id).ok_or(CoreError::JobNotFound(id))
    }

    pub fn get_mut(&mut self, id: Uuid) -> Result<&mut QJob, CoreError> {
        self.jobs.get_mut(&id).ok_or(CoreError::JobNotFound(id))
    }

    pub fn put_result(&mut self, id: Uuid, res: QResult) {
        self.results.insert(id, res);
    }

    pub fn get_result(&self, id: Uuid) -> Result<&QResult, CoreError> {
        self.results.get(&id).ok_or(CoreError::ResultNotFound(id))
    }

    pub fn list_ids(&self) -> Vec<Uuid> {
        self.jobs.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }
    
    pub fn clear(&mut self) {
        self.jobs.clear();
        self.results.clear();
    }

    pub fn upsert_job(&mut self, job: QJob) {
        self.jobs.insert(job.id, job);
    }
}

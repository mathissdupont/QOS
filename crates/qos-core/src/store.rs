//! In-memory job + result store (the "process table").

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::error::CoreError;
use crate::job::QJob;
use crate::result::QResult;
use qos_abi::JobHandle;

#[derive(Default)]
pub struct JobStore {
    jobs: BTreeMap<u64, QJob>,
    results: BTreeMap<u64, QResult>,
}

impl JobStore {
    pub fn new() -> Self {
        Self {
            jobs: BTreeMap::new(),
            results: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, job: QJob) {
        self.jobs.insert(job.id.0, job);
    }

    /// Insert or replace a job (used by journal recovery).
    pub fn upsert_job(&mut self, job: QJob) {
        self.jobs.insert(job.id.0, job);
    }

    pub fn get(&self, id: JobHandle) -> Result<&QJob, CoreError> {
        self.jobs.get(&id.0).ok_or(CoreError::JobNotFound(id))
    }

    pub fn get_mut(&mut self, id: JobHandle) -> Result<&mut QJob, CoreError> {
        self.jobs.get_mut(&id.0).ok_or(CoreError::JobNotFound(id))
    }

    pub fn put_result(&mut self, id: JobHandle, res: QResult) {
        self.results.insert(id.0, res);
    }

    pub fn get_result(&self, id: JobHandle) -> Result<&QResult, CoreError> {
        self.results.get(&id.0).ok_or(CoreError::ResultNotFound(id))
    }

    pub fn list_ids(&self) -> Vec<JobHandle> {
        self.jobs.keys().map(|&k| JobHandle(k)).collect()
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn clear(&mut self) {
        self.jobs.clear();
        self.results.clear();
    }
}

use crate::{
    CoreError, Event, EventLog, FifoScheduler, JobState, JobStore, QJob, QProc, QResult, Scheduler,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use crate::result::ResultStatus;

/// OS benzetmesi:
/// - JobStore = process table
/// - Scheduler = run queue
/// - JobManager = kernel service (syscall handler gibi)
pub struct JobManager {
    store: Arc<Mutex<JobStore>>,
    scheduler: Arc<Mutex<Box<dyn Scheduler>>>,
    eventlog: EventLog,
}

impl JobManager {
    /// Varsayılan manager: FIFO scheduler ile gelir (Strategy).
    pub fn new_fifo() -> Self {
        Self {
            store: Arc::new(Mutex::new(JobStore::new())),
            scheduler: Arc::new(Mutex::new(Box::new(FifoScheduler::new()))),
            eventlog: EventLog::new("qos.journal.jsonl"),
        }
    }

    pub fn new_fifo_with_journal(journal_path: &str) -> Self {
        Self {
            store: Arc::new(Mutex::new(JobStore::new())),
            scheduler: Arc::new(Mutex::new(Box::new(FifoScheduler::new()))),
            eventlog: EventLog::new(journal_path),
        }
    }
    
    pub fn get_job(&self, job_id: Uuid) -> Result<QJob, CoreError> {
        let store = self.store.lock().unwrap();
        Ok(store.get(job_id)?.clone())
    }



    /// İleride başka scheduler stratejileri takabilmek için constructor.
    pub fn new_with_scheduler(s: Box<dyn Scheduler>) -> Self {
        Self {
            store: Arc::new(Mutex::new(JobStore::new())),
            scheduler: Arc::new(Mutex::new(s)),
            eventlog: EventLog::new("qos.journal.jsonl"),
        }
    }

    pub fn new_with_scheduler_and_journal(s: Box<dyn Scheduler>, journal_path: &str) -> Self {
        Self {
            store: Arc::new(Mutex::new(JobStore::new())),
            scheduler: Arc::new(Mutex::new(s)),
            eventlog: EventLog::new(journal_path),
        }
    }


    /// Command: Submit
    pub fn submit(&self, proc: QProc) -> Uuid {
        let job = QJob::new(proc.clone());
        let id = job.id;

        {
            let mut store = self.store.lock().unwrap();
            store.insert(job);
        }
        {
            let mut sched = self.scheduler.lock().unwrap();
            sched.enqueue(id);
        }

        let _ = self.eventlog.append(&Event::Submitted { job_id: id, proc });
        let _ = self
            .eventlog
            .append(&Event::State { job_id: id, state: JobState::Queued });

        id
    }

    /// Command: Cancel
    pub fn cancel(&self, job_id: Uuid) -> Result<(), CoreError> {
        {
            let mut store = self.store.lock().unwrap();
            let job = store.get_mut(job_id)?;
            job.transition(JobState::Cancelled)
                .map_err(|_| CoreError::InvalidState(job_id))?;
        }

        let _ = self.eventlog.append(&Event::Cancelled { job_id });
        let _ = self
            .eventlog
            .append(&Event::State { job_id, state: JobState::Cancelled });

        Ok(())
    }

    /// Command: Dispatch
    pub fn dispatch_next(&self) -> Result<Option<Uuid>, CoreError> {
        let next = {
            let mut sched = self.scheduler.lock().unwrap();
            sched.select_next()
        };

        let Some(job_id) = next else { return Ok(None); };

        {
            let mut store = self.store.lock().unwrap();
            let job = store.get_mut(job_id)?;
            job.transition(JobState::Running)
                .map_err(|_| CoreError::InvalidState(job_id))?;
        }

        let _ = self.eventlog.append(&Event::Dispatched { job_id });
        let _ = self
            .eventlog
            .append(&Event::State { job_id, state: JobState::Running });

        Ok(Some(job_id))
    }

    /// Command: Finish OK (Result ile)
    pub fn finish_ok(&self, job_id: Uuid, mut result: QResult) -> Result<(), CoreError> {
        result.status = ResultStatus::Ok;
        result.error = None;

        {
            let mut store = self.store.lock().unwrap();
            store.put_result(job_id, result.clone());

            let job = store.get_mut(job_id)?;
            job.transition(JobState::Done)
                .map_err(|_| CoreError::InvalidState(job_id))?;
        }

        let _ = self.eventlog.append(&Event::FinishedOk { job_id, result });
        let _ = self.eventlog.append(&Event::State {
            job_id,
            state: JobState::Done,
        });

        Ok(())
    }


    /// Command: Finish ERR
    pub fn finish_err(&self, job_id: Uuid, error: String) -> Result<(), CoreError> {
        {
            let mut store = self.store.lock().unwrap();

            let result = QResult {
                status: ResultStatus::Error,
                counts: Default::default(),
                meta: "py-inproc-v0; error".to_string(),
                error: Some(error.clone()),
            };

            // 🔒 RESULT ÖNCE YAZILIR
            store.put_result(job_id, result);

            let job = store.get_mut(job_id)?;
            job.transition(JobState::Failed)
                .map_err(|_| CoreError::InvalidState(job_id))?;
        }

        let _ = self.eventlog.append(&Event::FinishedErr { job_id, error });
        let _ = self.eventlog.append(&Event::State {
            job_id,
            state: JobState::Failed,
        });

        Ok(())
    }

    /// Debug helpers
    pub fn status(&self, job_id: Uuid) -> Result<JobState, CoreError> {
        let store = self.store.lock().unwrap();
        Ok(store.get(job_id)?.state)
    }

    pub fn queued_len(&self) -> usize {
        let sched = self.scheduler.lock().unwrap();
        sched.len()
    }

    pub fn jobs_len(&self) -> usize {
        let store = self.store.lock().unwrap();
        store.len()
    }

    pub fn get_result(&self, job_id: Uuid) -> Result<QResult, CoreError> {
        let store = self.store.lock().unwrap();
        Ok(store.get_result(job_id)?.clone())
    }

    /// UI/debug convenience: list known job IDs.
    pub fn list_ids(&self) -> Vec<Uuid> {
        let store = self.store.lock().unwrap();
        store.list_ids()
    }

    /// UI/debug convenience: list known jobs and their current state.
    pub fn list(&self) -> Vec<(Uuid, JobState)> {
        let ids = self.list_ids();
        ids.into_iter()
            .filter_map(|id| self.status(id).ok().map(|st| (id, st)))
            .collect()
    }

    pub fn new_recovered(journal_path: &str) -> Result<Self, CoreError> {
        let eventlog = EventLog::new(journal_path);
        let events = eventlog.replay()?;

        let store = Arc::new(Mutex::new(JobStore::new()));
        let scheduler: Arc<Mutex<Box<dyn Scheduler>>> =
            Arc::new(Mutex::new(Box::new(FifoScheduler::new())));

        {
            let mut st = store.lock().unwrap();

            for ev in events {
                match ev {
                    Event::Submitted { job_id, proc } => {
                        // job created
                        let mut job = QJob::new(proc);
                        // QJob::new() yeni uuid üretir; biz replay’de ID’yi sabitlemeliyiz
                        job.id = job_id;
                        st.upsert_job(job);
                    }
                    Event::State { job_id, state } => {
                        if let Ok(job) = st.get_mut(job_id) {
                            // state transition kurallarını bypass etmiyoruz,
                            // ama replay için “final state” yazmak istiyoruz.
                            job.state = state;
                        }
                    }
                    Event::FinishedOk { job_id, result } => {
                        st.put_result(job_id, result);
                    }
                    Event::FinishedErr { job_id, .. } => {
                        if let Ok(job) = st.get_mut(job_id) {
                            job.state = JobState::Failed;
                        }
                    }
                    Event::Cancelled { job_id } => {
                        if let Ok(job) = st.get_mut(job_id) {
                            job.state = JobState::Cancelled;
                        }
                    }
                    Event::Dispatched { .. } => {
                        // v0.1’de State event’i zaten Running yazıyor; burada ekstra yok.
                    }
                }
            }
        }

        Ok(Self {
            store,
            scheduler,
            eventlog: EventLog::new(journal_path),
        })
    }

}

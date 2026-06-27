//! Job manager — the control-plane "kernel service" that ties the store, scheduler, and
//! event log together behind a small command surface (submit / dispatch / finish / cancel).
//!
//! Concurrency uses `spin::Mutex` so the same code runs in the bare-metal kernel and on the
//! host. Timestamps come from an injected [`Clock`].

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

use crate::clock::{Clock, ZeroClock};
use crate::error::CoreError;
use crate::eventlog::{Event, EventLog};
use crate::job::QJob;
use crate::result::QResult;
use crate::scheduler::{FifoScheduler, Scheduler};
use crate::store::JobStore;
use qos_abi::{JobHandle, JobState, ProcSpec, ResultStatus};

pub struct JobManager {
    store: Mutex<JobStore>,
    scheduler: Mutex<Box<dyn Scheduler>>,
    eventlog: EventLog,
    clock: Arc<dyn Clock>,
    next_id: AtomicU64,
}

impl JobManager {
    /// Default manager: FIFO scheduler, in-memory log, zero clock.
    pub fn new_fifo() -> Self {
        Self::new(
            Box::new(FifoScheduler::new()),
            EventLog::in_memory(),
            Arc::new(ZeroClock),
        )
    }

    /// FIFO manager that persists to a JSONL journal (`std` only).
    #[cfg(feature = "std")]
    pub fn new_fifo_with_journal(journal_path: &str) -> Self {
        Self::new(
            Box::new(FifoScheduler::new()),
            EventLog::with_journal(journal_path),
            Arc::new(crate::clock::StdClock),
        )
    }

    /// Fully explicit constructor.
    pub fn new(
        scheduler: Box<dyn Scheduler>,
        eventlog: EventLog,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store: Mutex::new(JobStore::new()),
            scheduler: Mutex::new(scheduler),
            eventlog,
            clock,
            next_id: AtomicU64::new(1),
        }
    }

    fn fresh_id(&self) -> JobHandle {
        JobHandle(self.next_id.fetch_add(1, Ordering::SeqCst))
    }

    // ── Commands ────────────────────────────────────────────────────────────────────────

    /// Submit a job; it enters the `Queued` state and the run queue.
    pub fn submit(&self, proc: ProcSpec) -> JobHandle {
        let id = self.fresh_id();
        let now = self.clock.now_micros();
        let job = QJob::new(id, proc.clone(), now);

        self.store.lock().insert(job);
        self.scheduler.lock().enqueue(id);

        let _ = self.eventlog.append(&Event::Submitted { job_id: id.0, proc });
        let _ = self.eventlog.append(&Event::State {
            job_id: id.0,
            state: JobState::Queued,
        });
        id
    }

    /// Pull the next queued job and move it to `Running`.
    pub fn dispatch_next(&self) -> Result<Option<JobHandle>, CoreError> {
        let next = self.scheduler.lock().select_next();
        let Some(id) = next else {
            return Ok(None);
        };
        let now = self.clock.now_micros();
        self.store.lock().get_mut(id)?.transition(JobState::Running, now)?;

        let _ = self.eventlog.append(&Event::Dispatched { job_id: id.0 });
        let _ = self.eventlog.append(&Event::State {
            job_id: id.0,
            state: JobState::Running,
        });
        Ok(Some(id))
    }

    /// Complete a running job successfully with a result.
    pub fn finish_ok(&self, id: JobHandle, mut result: QResult) -> Result<(), CoreError> {
        result.status = ResultStatus::Ok;
        result.error = None;
        let now = self.clock.now_micros();
        {
            let mut store = self.store.lock();
            store.put_result(id, result.clone());
            store.get_mut(id)?.transition(JobState::Done, now)?;
        }
        let _ = self.eventlog.append(&Event::FinishedOk {
            job_id: id.0,
            result,
        });
        let _ = self.eventlog.append(&Event::State {
            job_id: id.0,
            state: JobState::Done,
        });
        Ok(())
    }

    /// Fail a running job with an error message (result stored before the transition).
    pub fn finish_err(&self, id: JobHandle, error: String) -> Result<(), CoreError> {
        let now = self.clock.now_micros();
        {
            let mut store = self.store.lock();
            store.put_result(id, QResult::err(error.clone()));
            store.get_mut(id)?.transition(JobState::Failed, now)?;
        }
        let _ = self.eventlog.append(&Event::FinishedErr { job_id: id.0, error });
        let _ = self.eventlog.append(&Event::State {
            job_id: id.0,
            state: JobState::Failed,
        });
        Ok(())
    }

    /// Cancel a job (legal from `Queued` or `Running`).
    pub fn cancel(&self, id: JobHandle) -> Result<(), CoreError> {
        let now = self.clock.now_micros();
        self.store.lock().get_mut(id)?.transition(JobState::Cancelled, now)?;
        let _ = self.eventlog.append(&Event::Cancelled { job_id: id.0 });
        let _ = self.eventlog.append(&Event::State {
            job_id: id.0,
            state: JobState::Cancelled,
        });
        Ok(())
    }

    // ── Queries ─────────────────────────────────────────────────────────────────────────

    pub fn status(&self, id: JobHandle) -> Result<JobState, CoreError> {
        Ok(self.store.lock().get(id)?.state)
    }

    pub fn get_job(&self, id: JobHandle) -> Result<QJob, CoreError> {
        Ok(self.store.lock().get(id)?.clone())
    }

    pub fn get_result(&self, id: JobHandle) -> Result<QResult, CoreError> {
        Ok(self.store.lock().get_result(id)?.clone())
    }

    pub fn jobs_len(&self) -> usize {
        self.store.lock().len()
    }

    pub fn queued_len(&self) -> usize {
        self.scheduler.lock().len()
    }

    pub fn list_ids(&self) -> Vec<JobHandle> {
        self.store.lock().list_ids()
    }

    pub fn list(&self) -> Vec<(JobHandle, JobState)> {
        let store = self.store.lock();
        store
            .list_ids()
            .into_iter()
            .filter_map(|id| store.get(id).ok().map(|j| (id, j.state)))
            .collect()
    }

    /// Snapshot of the event log.
    pub fn events(&self) -> Vec<Event> {
        self.eventlog.events()
    }

    // ── Recovery (std only) ───────────────────────────────────────────────────────────────

    /// Rebuild a manager by replaying a JSONL journal file.
    #[cfg(feature = "std")]
    pub fn new_recovered(journal_path: &str) -> Result<Self, CoreError> {
        let events = EventLog::replay(journal_path)?;

        let mut store = JobStore::new();
        let mut max_id = 0u64;

        for ev in &events {
            match ev {
                Event::Submitted { job_id, proc } => {
                    max_id = max_id.max(*job_id);
                    store.upsert_job(QJob::new(JobHandle(*job_id), proc.clone(), 0));
                }
                Event::State { job_id, state } => {
                    if let Ok(job) = store.get_mut(JobHandle(*job_id)) {
                        job.state = *state;
                    }
                }
                Event::FinishedOk { job_id, result } => {
                    store.put_result(JobHandle(*job_id), result.clone());
                }
                Event::FinishedErr { job_id, error } => {
                    store.put_result(JobHandle(*job_id), QResult::err(error.clone()));
                    if let Ok(job) = store.get_mut(JobHandle(*job_id)) {
                        job.state = JobState::Failed;
                    }
                }
                Event::Cancelled { job_id } => {
                    if let Ok(job) = store.get_mut(JobHandle(*job_id)) {
                        job.state = JobState::Cancelled;
                    }
                }
                Event::Dispatched { .. } => {}
            }
        }

        Ok(Self {
            store: Mutex::new(store),
            scheduler: Mutex::new(Box::new(FifoScheduler::new())),
            eventlog: EventLog::with_journal(journal_path),
            clock: Arc::new(ZeroClock),
            next_id: AtomicU64::new(max_id + 1),
        })
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new_fifo()
    }
}

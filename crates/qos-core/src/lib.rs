pub mod job;
pub mod scheduler;
pub mod store;
pub mod manager;
pub mod result;
pub mod eventlog;


pub use job::{IrFormat, JobState, QJob, QProc};
pub use scheduler::{FifoScheduler, Scheduler};
pub use store::{CoreError, JobStore};
pub use manager::JobManager;
pub use result::{QResult, ResultStatus};
pub use eventlog::{Event, EventLog};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_enqueue_and_select() {
        let proc = QProc {
            name: "bell".into(),
            ir_format: IrFormat::OpenQasm3,
            ir_bytes: b"OPENQASM 3;".to_vec(),
            n_qubits: 2,
            shots: 1024,
        };

        let job = QJob::new(proc);
        let id = job.id;

        let mut store = JobStore::new();
        store.insert(job);
        assert_eq!(store.len(), 1);

        let mut sched = FifoScheduler::new();
        sched.enqueue(id);
        assert_eq!(sched.len(), 1);

        let next = sched.select_next().unwrap();
        assert_eq!(next, id);

        let j = store.get(id).unwrap();
        assert_eq!(j.state, JobState::Queued);
    }
}

#[test]
fn manager_lifecycle_flow() {
    let m = JobManager::new_fifo();

    let proc = QProc {
        name: "bell".into(),
        ir_format: IrFormat::OpenQasm3,
        ir_bytes: b"OPENQASM 3;".to_vec(),
        n_qubits: 2,
        shots: 100,
    };

    let id = m.submit(proc);
    assert_eq!(m.jobs_len(), 1);
    assert_eq!(m.queued_len(), 1);
    assert_eq!(m.status(id).unwrap(), JobState::Queued);

    let dispatched = m.dispatch_next().unwrap().unwrap();
    assert_eq!(dispatched, id);
    assert_eq!(m.status(id).unwrap(), JobState::Running);

    use std::collections::BTreeMap;

    let mut counts = BTreeMap::new();
    counts.insert("00".to_string(), 50);
    counts.insert("11".to_string(), 50);

    let res = QResult {
        status: ResultStatus::Ok,
        counts,
        meta: "simulated".to_string(),
        error: None,
    };

    m.finish_ok(id, res).unwrap();

    assert_eq!(m.status(id).unwrap(), JobState::Done);
}

#[test]
fn recovery_replays_journal() {
    use std::collections::BTreeMap;

    let journal_path = std::env::temp_dir().join(format!(
        "qos.recovery.test.{}.jsonl",
        std::process::id()
    ));
    let journal = journal_path.to_string_lossy();
    let _ = std::fs::remove_file(journal.as_ref());

    // 1) İlk run: job üret, dispatch, result yaz
    let m1 = JobManager::new_fifo_with_journal(journal.as_ref());

    let proc = QProc {
        name: "bell".into(),
        ir_format: IrFormat::OpenQasm3,
        ir_bytes: b"OPENQASM 3;".to_vec(),
        n_qubits: 2,
        shots: 10,
    };

    let id = m1.submit(proc);
    let _ = m1.dispatch_next().unwrap().unwrap();

    let mut counts = BTreeMap::new();
    counts.insert("00".to_string(), 5);
    counts.insert("11".to_string(), 5);

    let res = QResult {
        status: ResultStatus::Ok,
        counts,
        meta: "simulated".to_string(),
        error: None,
    };

    m1.finish_ok(id, res.clone()).unwrap();

    // 2) Recovery run: journal replay et
    let m2 = JobManager::new_recovered(journal.as_ref()).unwrap();

    assert_eq!(m2.status(id).unwrap(), JobState::Done);
    let got = m2.get_result(id).unwrap();
    assert_eq!(got.meta, res.meta);
    assert_eq!(got.counts, res.counts);

    let _ = std::fs::remove_file(journal.as_ref());
}



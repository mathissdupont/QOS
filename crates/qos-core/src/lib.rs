//! # qos-core
//!
//! The portable quantum control-plane core runtime for QOS.
//!
//! Per [ADR-0003](../../../docs/adr/0003-layered-architecture-single-core.md) this crate is
//! `no_std + alloc` by default and is embedded by both embodiments (the bare-metal kernel
//! and the host daemon). The `std` feature adds an on-disk journal and JSON serialization.
//!
//! Layers it provides:
//! - **L1 (core runtime):** job model, scheduler, store, event log, manager.
//! - **L0 (QHAL):** the [`backend`] module — the device/backend boundary
//!   ([ADR-0004](../../../docs/adr/0004-qhal-quantum-hardware-abstraction.md)).
//!
//! Shared vocabulary (job state, IR format, proc spec, handles) is re-exported from
//! [`qos_abi`] so the kernel, the daemon, and userland all speak the same types.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod backend;
pub mod clock;
pub mod error;
pub mod eventlog;
pub mod job;
pub mod manager;
pub mod result;
pub mod scheduler;
pub mod sim;
pub mod store;

pub use backend::{
    BackendCapabilities, BackendError, BackendJobId, BackendManager, BackendResult,
    BackendStatus, CalibrationData, GateType, LocalSimulatorBackend, QuantumBackend, Topology,
};
pub use sim::{bell_circuit, ghz_circuit, Circuit, Complex, Gate, QuantumState, SimulationResult};
pub use clock::{Clock, ZeroClock};
pub use error::CoreError;
pub use eventlog::{Event, EventLog};
pub use job::QJob;
pub use manager::JobManager;
pub use result::QResult;
pub use scheduler::{FifoScheduler, Scheduler};
pub use store::JobStore;

// Re-export the shared ABI vocabulary so downstream crates use one set of types.
pub use qos_abi::{IrFormat, JobHandle, JobState, ProcSpec, ResultStatus};

// Tests run on the host and therefore require the `std` feature (the test harness needs std).
#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use alloc::collections::BTreeMap;
    use alloc::string::ToString;

    fn bell_proc() -> ProcSpec {
        ProcSpec {
            name: "bell".to_string(),
            ir_format: IrFormat::OpenQasm2,
            ir_bytes: b"OPENQASM 2.0;".to_vec(),
            n_qubits: 2,
            shots: 100,
        }
    }

    #[test]
    fn job_lifecycle() {
        let m = JobManager::new_fifo();
        let id = m.submit(bell_proc());
        assert_eq!(m.jobs_len(), 1);
        assert_eq!(m.queued_len(), 1);
        assert_eq!(m.status(id).unwrap(), JobState::Queued);

        let dispatched = m.dispatch_next().unwrap().unwrap();
        assert_eq!(dispatched, id);
        assert_eq!(m.status(id).unwrap(), JobState::Running);

        let mut counts = BTreeMap::new();
        counts.insert("00".to_string(), 50);
        counts.insert("11".to_string(), 50);
        m.finish_ok(id, QResult::ok(counts, "sim")).unwrap();

        assert_eq!(m.status(id).unwrap(), JobState::Done);
        assert_eq!(m.get_result(id).unwrap().counts.get("11"), Some(&50));
    }

    #[test]
    fn illegal_transition_is_rejected() {
        let m = JobManager::new_fifo();
        let id = m.submit(bell_proc());
        // Cannot finish a job that was never dispatched (Queued -> Done is illegal).
        assert!(m.finish_ok(id, QResult::default()).is_err());
    }

    #[test]
    fn local_backend_runs_bell() {
        let mut backend = LocalSimulatorBackend::new(8);
        let jid = backend.submit(&bell_circuit(), 1000).unwrap();
        assert_eq!(backend.poll(jid), Some(JobState::Done));
        let res = backend.result(jid).unwrap();
        assert_eq!(res.shots, 1000);
        // A Bell state only ever collapses to |00> or |11>, never |01>/|10>.
        let total: u64 = res.counts.values().sum();
        assert_eq!(total, 1000);
        for k in res.counts.keys() {
            assert!(k == "00" || k == "11", "unexpected outcome {k}");
        }
        // Both outcomes should actually occur (entanglement, not a stuck state).
        assert!(res.counts.get("00").copied().unwrap_or(0) > 0);
        assert!(res.counts.get("11").copied().unwrap_or(0) > 0);
    }

    #[test]
    fn backend_rejects_oversized_circuit() {
        let backend = LocalSimulatorBackend::new(1);
        assert!(backend.validate(&bell_circuit()).is_err()); // needs 2 qubits, has 1
    }

    #[test]
    fn topology_connectivity() {
        let lin = Topology::linear(3);
        assert!(lin.is_connected(0, 1));
        assert!(!lin.is_connected(0, 2));
        let full = Topology::all_to_all(3);
        assert!(full.is_connected(0, 2));
    }

    #[test]
    fn journal_recovery_replays_state() {
        let path = std::env::temp_dir().join(format!(
            "qos-core.recovery.{}.jsonl",
            std::process::id()
        ));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);

        let id = {
            let m = JobManager::new_fifo_with_journal(&p);
            let id = m.submit(bell_proc());
            m.dispatch_next().unwrap();
            let mut counts = BTreeMap::new();
            counts.insert("11".to_string(), 100);
            m.finish_ok(id, QResult::ok(counts, "sim")).unwrap();
            id
        };

        let recovered = JobManager::new_recovered(&p).unwrap();
        assert_eq!(recovered.status(id).unwrap(), JobState::Done);
        assert_eq!(recovered.get_result(id).unwrap().counts.get("11"), Some(&100));

        // A fresh submit after recovery must not collide with recovered ids.
        let id2 = recovered.submit(bell_proc());
        assert_ne!(id2, id);

        let _ = std::fs::remove_file(&p);
    }

    // Silence unused-import warnings for `Box` in configs where it is not otherwise used.
    #[allow(dead_code)]
    fn _uses_box() -> Box<dyn Scheduler> {
        Box::new(FifoScheduler::new())
    }
}

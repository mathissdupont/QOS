//! QHAL — Quantum Hardware Abstraction Layer (L0).
//!
//! The single boundary every quantum executor implements: the in-kernel simulator, a cloud
//! QPU (via the ADR-0011 proxy), or a future local QPU. Polling-based and `no_std`-friendly.
//! See ADR-0004. Topology/calibration types here also serve ADR-0006 and ADR-0007.
//!
//! NOTE: the first cut submits a [`ProcSpec`] (carrying the IR bytes). Once the typed
//! `Circuit` IR is moved into `qos-core` (ADR-0005), `submit`/`validate` will take a
//! `&Circuit` instead. The trait shape (lifecycle + polling) is stable.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::sim::Circuit;
use qos_abi::JobState;

/// Logical gate kinds, used to describe a backend's native gate set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateType {
    H, X, Y, Z, S, Sdg, T, Tdg, Rx, Ry, Rz, U3, Id,
    Cx, Cz, Swap, Ccx,
    Measure, Reset, Barrier,
}

impl GateType {
    /// A universal-ish set good enough for the simulator's capabilities advertisement.
    pub fn universal_set() -> Vec<GateType> {
        use GateType::*;
        alloc::vec![
            H, X, Y, Z, S, Sdg, T, Tdg, Rx, Ry, Rz, U3, Id, Cx, Cz, Swap, Ccx, Measure, Reset,
            Barrier
        ]
    }
}

/// Physical qubit connectivity graph (ADR-0006).
#[derive(Clone, Debug)]
pub struct Topology {
    pub n_qubits: usize,
    /// Adjacency list: `connections[i]` are the qubits directly coupled to qubit `i`.
    pub connections: Vec<Vec<usize>>,
}

impl Topology {
    pub fn all_to_all(n: usize) -> Self {
        let connections = (0..n)
            .map(|i| (0..n).filter(|&j| j != i).collect())
            .collect();
        Self { n_qubits: n, connections }
    }

    pub fn linear(n: usize) -> Self {
        let connections = (0..n)
            .map(|i| {
                let mut c = Vec::new();
                if i > 0 {
                    c.push(i - 1);
                }
                if i + 1 < n {
                    c.push(i + 1);
                }
                c
            })
            .collect();
        Self { n_qubits: n, connections }
    }

    pub fn grid(rows: usize, cols: usize) -> Self {
        let n = rows * cols;
        let connections = (0..n)
            .map(|i| {
                let (r, c) = (i / cols, i % cols);
                let mut adj = Vec::new();
                if r > 0 {
                    adj.push(i - cols);
                }
                if r + 1 < rows {
                    adj.push(i + cols);
                }
                if c > 0 {
                    adj.push(i - 1);
                }
                if c + 1 < cols {
                    adj.push(i + 1);
                }
                adj
            })
            .collect();
        Self { n_qubits: n, connections }
    }

    pub fn is_connected(&self, a: usize, b: usize) -> bool {
        a < self.n_qubits && b < self.n_qubits && self.connections[a].contains(&b)
    }
}

/// Characterization / calibration data for a backend (ADR-0007).
#[derive(Clone, Debug, Default)]
pub struct CalibrationData {
    pub single_qubit_error: Vec<f64>,
    pub two_qubit_error: BTreeMap<(usize, usize), f64>,
    /// Per-qubit readout error as `(p(0|1), p(1|0))`.
    pub readout_error: Vec<(f64, f64)>,
    pub t1_us: Vec<f64>,
    pub t2_us: Vec<f64>,
    /// Timestamp (microseconds) when this calibration was measured.
    pub measured_at_us: u64,
}

impl CalibrationData {
    /// Ideal (error-free) calibration for the simulator.
    pub fn ideal(n_qubits: usize) -> Self {
        Self {
            single_qubit_error: alloc::vec![0.0; n_qubits],
            two_qubit_error: BTreeMap::new(),
            readout_error: alloc::vec![(0.0, 0.0); n_qubits],
            t1_us: alloc::vec![f64::INFINITY; n_qubits],
            t2_us: alloc::vec![f64::INFINITY; n_qubits],
            measured_at_us: 0,
        }
    }

    /// Age of the calibration relative to `now_us`.
    pub fn age_us(&self, now_us: u64) -> u64 {
        now_us.saturating_sub(self.measured_at_us)
    }
}

#[derive(Clone, Debug)]
pub struct BackendCapabilities {
    pub max_qubits: usize,
    pub native_gates: Vec<GateType>,
    pub topology: Option<Topology>,
    pub mid_circuit_measurement: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendStatus {
    Available,
    Busy,
    Offline,
    NeedsCalibration,
    Maintenance,
}

#[derive(Clone, Debug)]
pub struct BackendResult {
    pub counts: BTreeMap<String, u64>,
    pub shots: u64,
    pub execution_time_us: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    NotAvailable,
    UnsupportedCircuit(String),
    Communication(String),
    Config(String),
}

/// Backend-local job identifier.
pub type BackendJobId = u64;

/// The device/backend boundary. Polling-based; no async required.
pub trait QuantumBackend: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> &BackendCapabilities;
    fn status(&self) -> BackendStatus;

    /// Latest calibration, if known (ADR-0007).
    fn calibration(&self) -> Option<&CalibrationData> {
        None
    }

    /// Refresh calibration from the device/provider. Default: no-op.
    fn fetch_calibration(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    /// Validate a circuit against capabilities before submission.
    fn validate(&self, circuit: &Circuit) -> Result<(), BackendError> {
        let caps = self.capabilities();
        if circuit.n_qubits > caps.max_qubits {
            return Err(BackendError::UnsupportedCircuit(
                "circuit needs more qubits than the backend supports".to_string(),
            ));
        }
        if let Some(topo) = &caps.topology {
            if circuit.n_qubits > topo.n_qubits {
                return Err(BackendError::UnsupportedCircuit(
                    "circuit exceeds backend topology size".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Submit a circuit for `shots` repetitions. Per ADR-0004/0005 the QHAL operates on the
    /// typed `Circuit` IR; parsing source (QASM/JSON) into a `Circuit` happens at the edge.
    fn submit(&mut self, circuit: &Circuit, shots: u64) -> Result<BackendJobId, BackendError>;
    fn poll(&self, id: BackendJobId) -> Option<JobState>;
    fn result(&self, id: BackendJobId) -> Option<BackendResult>;

    fn cancel(&mut self, id: BackendJobId) -> bool {
        let _ = id;
        false
    }
}

/// The reference QHAL backend: the in-process statevector simulator (ADR-0004).
///
/// Runs a `Circuit` for the requested number of shots and stores the measurement counts.
/// Synchronous, so the result is ready immediately after `submit`.
pub struct LocalSimulatorBackend {
    name: String,
    caps: BackendCapabilities,
    calibration: CalibrationData,
    results: BTreeMap<BackendJobId, BackendResult>,
    next_id: BackendJobId,
}

impl LocalSimulatorBackend {
    pub fn new(max_qubits: usize) -> Self {
        Self {
            name: "local-sim".to_string(),
            caps: BackendCapabilities {
                max_qubits,
                native_gates: GateType::universal_set(),
                topology: Some(Topology::all_to_all(max_qubits)),
                mid_circuit_measurement: true,
            },
            calibration: CalibrationData::ideal(max_qubits),
            results: BTreeMap::new(),
            next_id: 1,
        }
    }
}

impl QuantumBackend for LocalSimulatorBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> &BackendCapabilities {
        &self.caps
    }

    fn status(&self) -> BackendStatus {
        BackendStatus::Available
    }

    fn calibration(&self) -> Option<&CalibrationData> {
        Some(&self.calibration)
    }

    fn submit(&mut self, circuit: &Circuit, shots: u64) -> Result<BackendJobId, BackendError> {
        self.validate(circuit)?;
        let id = self.next_id;
        self.next_id += 1;

        let shots = shots.max(1);
        let sim = circuit.run_shots(shots);

        self.results.insert(
            id,
            BackendResult {
                counts: sim.counts,
                shots,
                execution_time_us: 0,
            },
        );
        Ok(id)
    }

    fn poll(&self, id: BackendJobId) -> Option<JobState> {
        if self.results.contains_key(&id) {
            Some(JobState::Done)
        } else {
            None
        }
    }

    fn result(&self, id: BackendJobId) -> Option<BackendResult> {
        self.results.get(&id).cloned()
    }
}

/// Registry of available backends with a selectable default (ADR-0004).
pub struct BackendManager {
    backends: Vec<Box<dyn QuantumBackend>>,
    default_index: usize,
}

impl BackendManager {
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
            default_index: 0,
        }
    }

    /// A manager preloaded with the reference local statevector simulator.
    pub fn with_local_simulator(max_qubits: usize) -> Self {
        let mut m = Self::new();
        m.register(Box::new(LocalSimulatorBackend::new(max_qubits)));
        m
    }

    pub fn register(&mut self, backend: Box<dyn QuantumBackend>) {
        self.backends.push(backend);
    }

    pub fn len(&self) -> usize {
        self.backends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    pub fn set_default(&mut self, index: usize) -> bool {
        if index < self.backends.len() {
            self.default_index = index;
            true
        } else {
            false
        }
    }

    pub fn default_backend_mut(&mut self) -> Option<&mut Box<dyn QuantumBackend>> {
        self.backends.get_mut(self.default_index)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Box<dyn QuantumBackend>> {
        self.backends.iter_mut().find(|b| b.name() == name)
    }

    pub fn list(&self) -> Vec<(&str, BackendStatus)> {
        self.backends.iter().map(|b| (b.name(), b.status())).collect()
    }
}

impl Default for BackendManager {
    fn default() -> Self {
        Self::new()
    }
}

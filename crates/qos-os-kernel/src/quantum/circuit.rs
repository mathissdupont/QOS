//! Circuit - Quantum Circuit representation and execution
//!
//! Provides a Circuit struct that holds a sequence of gates to be executed.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;

use super::state::QuantumState;

/// A quantum gate operation
#[derive(Clone, Debug, PartialEq)]
pub enum Gate {
    /// Hadamard gate
    H(usize),
    /// Pauli-X gate
    X(usize),
    /// Pauli-Y gate  
    Y(usize),
    /// Pauli-Z gate
    Z(usize),
    /// S gate (√Z)
    S(usize),
    /// S† (S-dagger) gate
    Sdg(usize),
    /// T gate (√S)
    T(usize),
    /// T† (T-dagger) gate
    Tdg(usize),
    /// Rx rotation
    Rx(usize, f64),
    /// Ry rotation
    Ry(usize, f64),
    /// Rz rotation
    Rz(usize, f64),
    /// CNOT gate (control, target)
    Cx(usize, usize),
    /// CZ gate (control, target)
    Cz(usize, usize),
    /// SWAP gate
    Swap(usize, usize),
    /// Toffoli gate (control1, control2, target)
    Ccx(usize, usize, usize),
    /// Identity gate
    Id(usize),
    /// U3 gate (general single qubit rotation)
    U3(usize, f64, f64, f64),
    /// Measure qubit to classical bit
    Measure(usize, usize),
    /// Reset qubit to |0⟩
    Reset(usize),
    /// Barrier (no-op, for synchronization)
    Barrier(Vec<usize>),
}

/// A quantum circuit - sequence of gates
#[derive(Clone, Debug)]
pub struct Circuit {
    /// Number of qubits
    pub n_qubits: usize,
    /// Number of classical bits
    pub n_cbits: usize,
    /// Sequence of gates to execute
    pub gates: Vec<Gate>,
    /// Current instruction pointer (for step execution)
    pub pc: usize,
}

impl Circuit {
    /// Create a new empty circuit
    pub fn new(n_qubits: usize, n_cbits: usize) -> Self {
        Self {
            n_qubits,
            n_cbits,
            gates: Vec::new(),
            pc: 0,
        }
    }

    /// Add a gate to the circuit
    pub fn add(&mut self, gate: Gate) {
        self.gates.push(gate);
    }

    /// Reset execution to the beginning
    pub fn reset_pc(&mut self) {
        self.pc = 0;
    }

    /// Check if circuit execution is complete
    pub fn is_done(&self) -> bool {
        self.pc >= self.gates.len()
    }

    /// Get the number of gates
    pub fn len(&self) -> usize {
        self.gates.len()
    }

    /// Check if circuit is empty
    pub fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }

    /// Get remaining gates to execute
    pub fn remaining(&self) -> usize {
        self.gates.len().saturating_sub(self.pc)
    }

    /// Execute a single gate (step execution)
    /// Returns true if a gate was executed, false if circuit is done
    pub fn step(&mut self, state: &mut QuantumState) -> bool {
        if self.pc >= self.gates.len() {
            return false;
        }

        let gate = &self.gates[self.pc];
        self.execute_gate(state, gate.clone());
        self.pc += 1;
        true
    }

    /// Execute N gates at once (batch execution)
    /// Returns number of gates actually executed
    pub fn step_n(&mut self, state: &mut QuantumState, n: usize) -> usize {
        let mut executed = 0;
        for _ in 0..n {
            if !self.step(state) {
                break;
            }
            executed += 1;
        }
        executed
    }

    /// Execute a single gate on the state
    fn execute_gate(&self, state: &mut QuantumState, gate: Gate) {
        match gate {
            Gate::H(q) => state.apply_h(q),
            Gate::X(q) => state.apply_x(q),
            Gate::Y(q) => state.apply_y(q),
            Gate::Z(q) => state.apply_z(q),
            Gate::S(q) => state.apply_s(q),
            Gate::T(q) => state.apply_t(q),
            Gate::Sdg(q) => state.apply_sdg(q),
            Gate::Tdg(q) => state.apply_tdg(q),
            Gate::Rx(q, theta) => state.apply_rx(q, theta),
            Gate::Ry(q, theta) => state.apply_ry(q, theta),
            Gate::Rz(q, theta) => state.apply_rz(q, theta),
            Gate::Cx(c, t) => state.apply_cx(c, t),
            Gate::Cz(c, t) => state.apply_cz(c, t),
            Gate::Swap(a, b) => state.apply_swap(a, b),
            Gate::Ccx(c1, c2, t) => state.apply_ccx(c1, c2, t),
            Gate::Id(_q) => {
                // Identity - no operation
            }
            Gate::U3(q, theta, phi, lambda) => state.apply_u3(q, theta, phi, lambda),
            Gate::Measure(q, _c) => {
                state.measure_qubit(q);
            }
            Gate::Reset(q) => {
                // Measure then flip if needed
                let result = state.measure_qubit(q);
                if result == 1 {
                    state.apply_x(q);
                    state.classical[q] = 0;
                }
            }
            Gate::Barrier(_) => {
                // No-op
            }
        }
    }

    /// Execute the entire circuit at once
    pub fn run(&mut self, state: &mut QuantumState) {
        while self.step(state) {}
    }

    /// Run multiple shots and collect measurement statistics
    pub fn run_shots(&self, shots: u64) -> SimulationResult {
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();
        let mut state = QuantumState::new(self.n_qubits);
        
        for _ in 0..shots {
            // Reset state and circuit for each shot
            state.reset();
            let mut circuit = self.clone();
            circuit.reset_pc();
            
            // Run circuit
            circuit.run(&mut state);
            
            // Sample outcome
            let outcome = state.sample_outcome();
            *counts.entry(outcome).or_insert(0) += 1;
        }

        SimulationResult {
            counts,
            n_qubits: self.n_qubits,
            shots,
        }
    }
}

/// Result of a simulation run
#[derive(Clone, Debug)]
pub struct SimulationResult {
    /// Measurement counts: bitstring -> count
    pub counts: BTreeMap<String, u64>,
    /// Number of qubits
    pub n_qubits: usize,
    /// Number of shots
    pub shots: u64,
}

impl SimulationResult {
    /// Get count for a specific bitstring
    pub fn get(&self, key: &str) -> u64 {
        *self.counts.get(key).unwrap_or(&0)
    }

    /// Get |00...0⟩ count
    pub fn count_zeros(&self) -> u64 {
        self.get(&"0".repeat(self.n_qubits))
    }

    /// Get |11...1⟩ count
    pub fn count_ones(&self) -> u64 {
        self.get(&"1".repeat(self.n_qubits))
    }

    /// For Bell state: (|00⟩, |11⟩) counts
    pub fn bell_counts(&self) -> (u64, u64) {
        (self.get("00"), self.get("11"))
    }

    /// Get most frequent outcome
    pub fn most_frequent(&self) -> Option<(&String, u64)> {
        self.counts.iter().max_by_key(|(_, &c)| c).map(|(k, &v)| (k, v))
    }
}

// Convenience methods for building circuits
impl Circuit {
    pub fn h(&mut self, q: usize) -> &mut Self { self.add(Gate::H(q)); self }
    pub fn x(&mut self, q: usize) -> &mut Self { self.add(Gate::X(q)); self }
    pub fn y(&mut self, q: usize) -> &mut Self { self.add(Gate::Y(q)); self }
    pub fn z(&mut self, q: usize) -> &mut Self { self.add(Gate::Z(q)); self }
    pub fn s(&mut self, q: usize) -> &mut Self { self.add(Gate::S(q)); self }
    pub fn t(&mut self, q: usize) -> &mut Self { self.add(Gate::T(q)); self }
    pub fn cx(&mut self, c: usize, t: usize) -> &mut Self { self.add(Gate::Cx(c, t)); self }
    pub fn cz(&mut self, c: usize, t: usize) -> &mut Self { self.add(Gate::Cz(c, t)); self }
    pub fn swap(&mut self, a: usize, b: usize) -> &mut Self { self.add(Gate::Swap(a, b)); self }
    pub fn measure(&mut self, q: usize, c: usize) -> &mut Self { self.add(Gate::Measure(q, c)); self }
    pub fn reset(&mut self, q: usize) -> &mut Self { self.add(Gate::Reset(q)); self }
}

/// Create a standard Bell state circuit
pub fn bell_circuit() -> Circuit {
    let mut c = Circuit::new(2, 2);
    c.h(0);
    c.cx(0, 1);
    c.measure(0, 0);
    c.measure(1, 1);
    c
}

/// Create a GHZ state circuit for n qubits
pub fn ghz_circuit(n: usize) -> Circuit {
    let mut c = Circuit::new(n, n);
    c.h(0);
    for i in 0..n-1 {
        c.cx(i, i + 1);
    }
    for i in 0..n {
        c.measure(i, i);
    }
    c
}

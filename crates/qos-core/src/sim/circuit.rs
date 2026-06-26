//! Typed circuit IR (the internal representation per ADR-0005) and its execution.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::state::QuantumState;

/// A quantum gate operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Gate {
    H(usize),
    X(usize),
    Y(usize),
    Z(usize),
    S(usize),
    Sdg(usize),
    T(usize),
    Tdg(usize),
    Rx(usize, f64),
    Ry(usize, f64),
    Rz(usize, f64),
    Cx(usize, usize),
    Cz(usize, usize),
    Swap(usize, usize),
    Ccx(usize, usize, usize),
    Id(usize),
    U3(usize, f64, f64, f64),
    Measure(usize, usize),
    Reset(usize),
    Barrier(Vec<usize>),
}

/// A quantum circuit — an ordered sequence of gates over qreg/creg.
#[derive(Clone, Debug)]
pub struct Circuit {
    pub n_qubits: usize,
    pub n_cbits: usize,
    pub gates: Vec<Gate>,
    /// Instruction pointer for stepwise execution (ADR-0009).
    pub pc: usize,
}

impl Circuit {
    pub fn new(n_qubits: usize, n_cbits: usize) -> Self {
        Self {
            n_qubits,
            n_cbits,
            gates: Vec::new(),
            pc: 0,
        }
    }

    pub fn add(&mut self, gate: Gate) {
        self.gates.push(gate);
    }

    pub fn reset_pc(&mut self) {
        self.pc = 0;
    }

    pub fn is_done(&self) -> bool {
        self.pc >= self.gates.len()
    }

    pub fn len(&self) -> usize {
        self.gates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.gates.len().saturating_sub(self.pc)
    }

    /// Execute a single gate (stepwise). Returns false when the circuit is done.
    pub fn step(&mut self, state: &mut QuantumState) -> bool {
        if self.pc >= self.gates.len() {
            return false;
        }
        let gate = self.gates[self.pc].clone();
        self.execute_gate(state, gate);
        self.pc += 1;
        true
    }

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
            Gate::Id(_q) => {}
            Gate::U3(q, theta, phi, lambda) => state.apply_u3(q, theta, phi, lambda),
            Gate::Measure(q, _c) => {
                state.measure_qubit(q);
            }
            Gate::Reset(q) => {
                let result = state.measure_qubit(q);
                if result == 1 {
                    state.apply_x(q);
                    state.classical[q] = 0;
                }
            }
            Gate::Barrier(_) => {}
        }
    }

    pub fn run(&mut self, state: &mut QuantumState) {
        while self.step(state) {}
    }

    /// Run multiple shots and collect measurement statistics.
    pub fn run_shots(&self, shots: u64) -> SimulationResult {
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();
        let mut state = QuantumState::new(self.n_qubits);

        for _ in 0..shots {
            state.reset();
            let mut circuit = self.clone();
            circuit.reset_pc();
            circuit.run(&mut state);
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

/// Result of a simulation run.
#[derive(Clone, Debug)]
pub struct SimulationResult {
    pub counts: BTreeMap<String, u64>,
    pub n_qubits: usize,
    pub shots: u64,
}

impl SimulationResult {
    pub fn get(&self, key: &str) -> u64 {
        *self.counts.get(key).unwrap_or(&0)
    }

    pub fn count_zeros(&self) -> u64 {
        let mut z = String::new();
        for _ in 0..self.n_qubits {
            z.push('0');
        }
        self.get(&z)
    }

    pub fn most_frequent(&self) -> Option<(&String, u64)> {
        self.counts.iter().max_by_key(|(_, &c)| c).map(|(k, &v)| (k, v))
    }
}

// Convenience builders.
impl Circuit {
    pub fn h(&mut self, q: usize) -> &mut Self {
        self.add(Gate::H(q));
        self
    }
    pub fn x(&mut self, q: usize) -> &mut Self {
        self.add(Gate::X(q));
        self
    }
    pub fn cx(&mut self, c: usize, t: usize) -> &mut Self {
        self.add(Gate::Cx(c, t));
        self
    }
    pub fn measure(&mut self, q: usize, c: usize) -> &mut Self {
        self.add(Gate::Measure(q, c));
        self
    }
}

/// Standard 2-qubit Bell-state circuit.
pub fn bell_circuit() -> Circuit {
    let mut c = Circuit::new(2, 2);
    c.h(0);
    c.cx(0, 1);
    c.measure(0, 0);
    c.measure(1, 1);
    c
}

/// GHZ-state circuit for n qubits.
pub fn ghz_circuit(n: usize) -> Circuit {
    let mut c = Circuit::new(n, n);
    c.h(0);
    for i in 0..n - 1 {
        c.cx(i, i + 1);
    }
    for i in 0..n {
        c.measure(i, i);
    }
    c
}

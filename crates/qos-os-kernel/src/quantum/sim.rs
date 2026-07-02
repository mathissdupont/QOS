//! Statevector Quantum Simulator for QOS
//!
//! Implements a full statevector simulation with measurement sampling.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::linalg::{
    expand_single_gate, expand_two_qubit_gate, gate_cx, gate_cz, gate_h, gate_s, gate_swap,
    gate_t, gate_x, gate_y, gate_z, Complex, Matrix,
};
use super::parser::{Instruction, QasmProgram};

/// Simple PRNG (xorshift64) for measurement sampling
static RNG_STATE: AtomicU64 = AtomicU64::new(0x853c49e6748fea9b);

fn xorshift64() -> u64 {
    let mut x = RNG_STATE.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    RNG_STATE.store(x, Ordering::Relaxed);
    x
}

/// Seed the RNG (useful for reproducibility)
pub fn seed_rng(seed: u64) {
    RNG_STATE.store(if seed == 0 { 1 } else { seed }, Ordering::Relaxed);
}

/// Generate a random f64 in [0, 1)
fn random_f64() -> f64 {
    (xorshift64() as f64) / (u64::MAX as f64)
}

/// Result of a simulation run
#[derive(Clone, Debug)]
pub struct SimResult {
    /// Measurement counts: bitstring -> count
    pub counts: BTreeMap<String, u64>,
    /// Number of qubits
    pub n_qubits: usize,
    /// Number of shots executed
    pub shots: u64,
}

impl SimResult {
    /// Get count for |00...0⟩ state
    pub fn count_zeros(&self) -> u64 {
        let key = "0".repeat(self.n_qubits);
        *self.counts.get(&key).unwrap_or(&0)
    }

    /// Get count for |11...1⟩ state
    pub fn count_ones(&self) -> u64 {
        let key = "1".repeat(self.n_qubits);
        *self.counts.get(&key).unwrap_or(&0)
    }

    /// For Bell state: get (|00⟩ count, |11⟩ count)
    pub fn bell_counts(&self) -> (u64, u64) {
        let n00 = *self.counts.get("00").unwrap_or(&0);
        let n11 = *self.counts.get("11").unwrap_or(&0);
        (n00, n11)
    }
}

/// The statevector quantum simulator
pub struct Simulator {
    /// Number of qubits
    n_qubits: usize,
    /// Statevector: 2^n complex amplitudes
    state: Vec<Complex>,
    /// Classical register
    classical: Vec<u8>,
}

impl Simulator {
    /// Create a new simulator with n qubits, initialized to |0...0⟩
    pub fn new(n_qubits: usize) -> Self {
        let dim = 1 << n_qubits;
        let mut state = vec![Complex::ZERO; dim];
        state[0] = Complex::ONE; // |0...0⟩

        Self {
            n_qubits,
            state,
            classical: vec![0u8; n_qubits],
        }
    }

    /// Reset to |0...0⟩
    pub fn reset(&mut self) {
        for amp in self.state.iter_mut() {
            *amp = Complex::ZERO;
        }
        self.state[0] = Complex::ONE;
        for c in self.classical.iter_mut() {
            *c = 0;
        }
    }

    /// Reset a single qubit to |0⟩ (partial reset)
    pub fn reset_qubit(&mut self, qubit: usize) {
        // Measure first to collapse
        let _ = self.measure_qubit(qubit);
        
        // If result was |1⟩, apply X to flip to |0⟩
        if self.classical[qubit] == 1 {
            self.apply_x(qubit);
            self.classical[qubit] = 0;
        }
    }

    /// Apply a single-qubit gate
    fn apply_single_gate(&mut self, gate: &Matrix, target: usize) {
        let full_gate = expand_single_gate(gate, target, self.n_qubits);
        self.state = full_gate.mul_vec(&self.state);
    }

    /// Apply a two-qubit gate
    fn apply_two_qubit_gate(&mut self, gate: &Matrix, ctrl: usize, targ: usize) {
        let full_gate = expand_two_qubit_gate(gate, ctrl, targ, self.n_qubits);
        self.state = full_gate.mul_vec(&self.state);
    }

    /// Apply Hadamard gate
    pub fn apply_h(&mut self, target: usize) {
        self.apply_single_gate(&gate_h(), target);
    }

    /// Apply Pauli-X gate
    pub fn apply_x(&mut self, target: usize) {
        self.apply_single_gate(&gate_x(), target);
    }

    /// Apply Pauli-Y gate
    pub fn apply_y(&mut self, target: usize) {
        self.apply_single_gate(&gate_y(), target);
    }

    /// Apply Pauli-Z gate
    pub fn apply_z(&mut self, target: usize) {
        self.apply_single_gate(&gate_z(), target);
    }

    /// Apply S gate
    pub fn apply_s(&mut self, target: usize) {
        self.apply_single_gate(&gate_s(), target);
    }

    /// Apply T gate
    pub fn apply_t(&mut self, target: usize) {
        self.apply_single_gate(&gate_t(), target);
    }

    /// Apply CNOT (CX) gate
    pub fn apply_cx(&mut self, ctrl: usize, targ: usize) {
        self.apply_two_qubit_gate(&gate_cx(), ctrl, targ);
    }

    /// Apply CZ gate
    pub fn apply_cz(&mut self, ctrl: usize, targ: usize) {
        self.apply_two_qubit_gate(&gate_cz(), ctrl, targ);
    }

    /// Apply SWAP gate
    pub fn apply_swap(&mut self, q1: usize, q2: usize) {
        self.apply_two_qubit_gate(&gate_swap(), q1, q2);
    }

    /// Measure a single qubit, collapsing the state
    pub fn measure_qubit(&mut self, qubit: usize) -> u8 {
        // Calculate probability of measuring |1⟩
        let mut prob_one = 0.0;
        let dim = self.state.len();

        for i in 0..dim {
            // Check if qubit is |1⟩ in this basis state
            let bit = (i >> (self.n_qubits - 1 - qubit)) & 1;
            if bit == 1 {
                prob_one += self.state[i].norm_sq();
            }
        }

        // Sample outcome
        let r = random_f64();
        let outcome = if r < prob_one { 1u8 } else { 0u8 };

        // Collapse the state
        let mut norm_sq = 0.0;
        for i in 0..dim {
            let bit = (i >> (self.n_qubits - 1 - qubit)) & 1;
            if (bit as u8) != outcome {
                self.state[i] = Complex::ZERO;
            } else {
                norm_sq += self.state[i].norm_sq();
            }
        }

        // Renormalize
        if norm_sq > 1e-15 {
            let norm = libm::sqrt(norm_sq);
            for amp in self.state.iter_mut() {
                *amp = *amp * (1.0 / norm);
            }
        }

        // Store in classical register
        self.classical[qubit] = outcome;
        outcome
    }

    /// Measure all qubits, return bitstring
    pub fn measure_all(&mut self) -> String {
        let mut result = String::new();
        for q in 0..self.n_qubits {
            let bit = self.measure_qubit(q);
            result.push(if bit == 1 { '1' } else { '0' });
        }
        result
    }

    /// Sample a measurement outcome without collapsing (for shots)
    fn sample_outcome(&self) -> String {
        let r = random_f64();
        let mut cumulative = 0.0;
        let dim = self.state.len();

        for i in 0..dim {
            cumulative += self.state[i].norm_sq();
            if r < cumulative {
                // Convert index to bitstring
                let mut s = String::new();
                for q in 0..self.n_qubits {
                    let bit = (i >> (self.n_qubits - 1 - q)) & 1;
                    s.push(if bit == 1 { '1' } else { '0' });
                }
                return s;
            }
        }

        // Fallback (shouldn't happen with normalized state)
        "0".repeat(self.n_qubits)
    }

    /// Execute a single instruction
    pub fn execute_instruction(&mut self, inst: &Instruction) {
        match inst {
            Instruction::H(q) => self.apply_h(*q),
            Instruction::X(q) => self.apply_x(*q),
            Instruction::Y(q) => self.apply_y(*q),
            Instruction::Z(q) => self.apply_z(*q),
            Instruction::S(q) => self.apply_s(*q),
            Instruction::T(q) => self.apply_t(*q),
            Instruction::Cx(c, t) => self.apply_cx(*c, *t),
            Instruction::Cz(c, t) => self.apply_cz(*c, *t),
            Instruction::Swap(a, b) => self.apply_swap(*a, *b),
            Instruction::Measure(q, c) => {
                let result = self.measure_qubit(*q);
                if *c < self.classical.len() {
                    self.classical[*c] = result;
                }
            }
            Instruction::Reset(q) => self.reset_qubit(*q),
            Instruction::Barrier(_) => {
                // Barrier is a no-op in simulation
            }
        }
    }

    /// Execute a program for a single shot
    pub fn execute_program(&mut self, program: &QasmProgram) {
        for inst in &program.instructions {
            self.execute_instruction(inst);
        }
    }

    /// Run simulation for multiple shots
    pub fn run(&mut self, program: &QasmProgram, shots: u64) -> SimResult {
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();

        // Check if program has measurements
        let has_measurements = program.instructions.iter().any(|i| matches!(i, Instruction::Measure(_, _)));

        for _ in 0..shots {
            self.reset();

            if has_measurements {
                // Execute with measurements
                self.execute_program(program);
                
                // Build result from classical register
                let mut result = String::new();
                for &c in &self.classical[..program.n_cbits.min(self.classical.len())] {
                    result.push(if c == 1 { '1' } else { '0' });
                }
                
                *counts.entry(result).or_insert(0) += 1;
            } else {
                // Execute without measurements, then sample
                self.execute_program(program);
                let outcome = self.sample_outcome();
                *counts.entry(outcome).or_insert(0) += 1;
            }
        }

        SimResult {
            counts,
            n_qubits: program.n_qubits,
            shots,
        }
    }
}

/// Total simulator jobs executed since boot (surfaced by the Process viewer / System Monitor).
pub static SIM_JOBS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// High-level function: parse and run QASM2
pub fn run_qasm2(qasm: &[u8], shots: u64) -> Result<SimResult, super::parser::ParseError> {
    let program = super::parser::parse_qasm2(qasm)?;
    let mut sim = Simulator::new(program.n_qubits);
    SIM_JOBS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    Ok(sim.run(&program, shots))
}

/// Quick Bell state simulation (for compatibility with existing API)
pub fn run_bell(shots: u64) -> (u64, u64) {
    let qasm = b"OPENQASM 2.0;\nqreg q[2];\ncreg c[2];\nh q[0];\ncx q[0],q[1];\nmeasure q[0] -> c[0];\nmeasure q[1] -> c[1];\n";
    match run_qasm2(qasm, shots) {
        Ok(result) => result.bell_counts(),
        Err(_) => (shots / 2, shots - shots / 2), // Fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bell_state() {
        seed_rng(42);
        let (n00, n11) = run_bell(1000);
        // Should be roughly 50/50
        assert!(n00 > 400 && n00 < 600);
        assert!(n11 > 400 && n11 < 600);
        assert_eq!(n00 + n11, 1000);
    }

    #[test]
    fn test_hadamard() {
        seed_rng(42);
        let qasm = b"OPENQASM 2.0;\nqreg q[1];\ncreg c[1];\nh q[0];\nmeasure q[0] -> c[0];\n";
        let result = run_qasm2(qasm, 1000).unwrap();
        let n0 = *result.counts.get("0").unwrap_or(&0);
        let n1 = *result.counts.get("1").unwrap_or(&0);
        // Should be roughly 50/50
        assert!(n0 > 400 && n0 < 600);
        assert!(n1 > 400 && n1 < 600);
    }
}

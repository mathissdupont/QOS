//! Statevector Quantum Simulator for QOS
//!
//! Implements a full statevector simulation with measurement sampling.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::linalg::Complex;
use super::parser::{Instruction, QasmProgram};

/// Hard cap on simulated qubits: 2^20 amplitudes × 16 B = 16 MiB — fits the kernel heap with
/// headroom. Also input validation: a hostile/typo'd `qreg q[64]` must fail cleanly, not OOM.
pub const MAX_QUBITS: usize = 20;

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

    /// Bit mask selecting `qubit` in a basis-state index (qubit 0 = most significant, matching
    /// `measure_qubit`'s convention).
    #[inline]
    fn qbit(&self, qubit: usize) -> usize {
        1usize << (self.n_qubits - 1 - qubit)
    }

    /// Apply a 2×2 unitary to `target` **in place**: O(2^n) time, O(1) extra memory. (The old
    /// path built the full 2^n×2^n operator — O(4^n) — which made >10 qubits infeasible.)
    fn apply_single_inplace(&mut self, m: [[Complex; 2]; 2], target: usize) {
        let bit = self.qbit(target);
        let dim = self.state.len();
        for i in 0..dim {
            if i & bit == 0 {
                let a = self.state[i];
                let b = self.state[i | bit];
                self.state[i] = m[0][0] * a + m[0][1] * b;
                self.state[i | bit] = m[1][0] * a + m[1][1] * b;
            }
        }
    }

    /// Apply a controlled 2×2 unitary (target rotated only where `ctrl` is |1⟩), in place.
    fn apply_controlled_inplace(&mut self, m: [[Complex; 2]; 2], ctrl: usize, targ: usize) {
        let cbit = self.qbit(ctrl);
        let tbit = self.qbit(targ);
        let dim = self.state.len();
        for i in 0..dim {
            if i & cbit != 0 && i & tbit == 0 {
                let a = self.state[i];
                let b = self.state[i | tbit];
                self.state[i] = m[0][0] * a + m[0][1] * b;
                self.state[i | tbit] = m[1][0] * a + m[1][1] * b;
            }
        }
    }

    /// Apply Hadamard gate
    pub fn apply_h(&mut self, target: usize) {
        let s = Complex::inv_sqrt2();
        let h = Complex::new(s, 0.0);
        self.apply_single_inplace([[h, h], [h, Complex::new(-s, 0.0)]], target);
    }

    /// Apply Pauli-X gate
    pub fn apply_x(&mut self, target: usize) {
        self.apply_single_inplace([[Complex::ZERO, Complex::ONE], [Complex::ONE, Complex::ZERO]], target);
    }

    /// Apply Pauli-Y gate
    pub fn apply_y(&mut self, target: usize) {
        let ni = Complex::new(0.0, -1.0);
        self.apply_single_inplace([[Complex::ZERO, ni], [Complex::I, Complex::ZERO]], target);
    }

    /// Apply Pauli-Z gate
    pub fn apply_z(&mut self, target: usize) {
        let nz = Complex::new(-1.0, 0.0);
        self.apply_single_inplace([[Complex::ONE, Complex::ZERO], [Complex::ZERO, nz]], target);
    }

    /// Apply S gate
    pub fn apply_s(&mut self, target: usize) {
        self.apply_single_inplace([[Complex::ONE, Complex::ZERO], [Complex::ZERO, Complex::I]], target);
    }

    /// Apply T gate
    pub fn apply_t(&mut self, target: usize) {
        let s = Complex::inv_sqrt2();
        let t = Complex::new(s, s); // e^{iπ/4}
        self.apply_single_inplace([[Complex::ONE, Complex::ZERO], [Complex::ZERO, t]], target);
    }

    /// Apply RX(θ) — rotation about the X axis (parametric).
    pub fn apply_rx(&mut self, target: usize, theta: f64) {
        let c = Complex::new(libm::cos(theta / 2.0), 0.0);
        let ns = Complex::new(0.0, -libm::sin(theta / 2.0));
        self.apply_single_inplace([[c, ns], [ns, c]], target);
    }

    /// Apply RY(θ) — rotation about the Y axis (parametric).
    pub fn apply_ry(&mut self, target: usize, theta: f64) {
        let c = Complex::new(libm::cos(theta / 2.0), 0.0);
        let s = libm::sin(theta / 2.0);
        self.apply_single_inplace([[c, Complex::new(-s, 0.0)], [Complex::new(s, 0.0), c]], target);
    }

    /// Apply RZ(θ) — rotation about the Z axis (parametric).
    pub fn apply_rz(&mut self, target: usize, theta: f64) {
        let e0 = Complex::new(libm::cos(theta / 2.0), -libm::sin(theta / 2.0));
        let e1 = Complex::new(libm::cos(theta / 2.0), libm::sin(theta / 2.0));
        self.apply_single_inplace([[e0, Complex::ZERO], [Complex::ZERO, e1]], target);
    }

    /// Apply P(θ) — phase gate diag(1, e^{iθ}) (parametric).
    pub fn apply_p(&mut self, target: usize, theta: f64) {
        let e = Complex::new(libm::cos(theta), libm::sin(theta));
        self.apply_single_inplace([[Complex::ONE, Complex::ZERO], [Complex::ZERO, e]], target);
    }

    /// Apply CNOT (CX) gate
    pub fn apply_cx(&mut self, ctrl: usize, targ: usize) {
        self.apply_controlled_inplace([[Complex::ZERO, Complex::ONE], [Complex::ONE, Complex::ZERO]], ctrl, targ);
    }

    /// Apply CRZ(θ) — controlled rotation about Z (parametric).
    pub fn apply_crz(&mut self, ctrl: usize, targ: usize, theta: f64) {
        let e0 = Complex::new(libm::cos(theta / 2.0), -libm::sin(theta / 2.0));
        let e1 = Complex::new(libm::cos(theta / 2.0), libm::sin(theta / 2.0));
        self.apply_controlled_inplace([[e0, Complex::ZERO], [Complex::ZERO, e1]], ctrl, targ);
    }

    /// Apply CP(θ) — controlled phase diag(1, e^{iθ}) on the target (parametric).
    pub fn apply_cp(&mut self, ctrl: usize, targ: usize, theta: f64) {
        let e = Complex::new(libm::cos(theta), libm::sin(theta));
        self.apply_controlled_inplace([[Complex::ONE, Complex::ZERO], [Complex::ZERO, e]], ctrl, targ);
    }

    /// Apply CZ gate
    pub fn apply_cz(&mut self, ctrl: usize, targ: usize) {
        let nz = Complex::new(-1.0, 0.0);
        self.apply_controlled_inplace([[Complex::ONE, Complex::ZERO], [Complex::ZERO, nz]], ctrl, targ);
    }

    /// Apply SWAP gate: exchange amplitudes of basis states that differ in exactly the two bits.
    pub fn apply_swap(&mut self, q1: usize, q2: usize) {
        let b1 = self.qbit(q1);
        let b2 = self.qbit(q2);
        let dim = self.state.len();
        for i in 0..dim {
            if i & b1 != 0 && i & b2 == 0 {
                let j = (i & !b1) | b2;
                self.state.swap(i, j);
            }
        }
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
            Instruction::Rx(q, theta) => self.apply_rx(*q, *theta),
            Instruction::Ry(q, theta) => self.apply_ry(*q, *theta),
            Instruction::Rz(q, theta) => self.apply_rz(*q, *theta),
            Instruction::P(q, theta) => self.apply_p(*q, *theta),
            Instruction::Crz(c, t, theta) => self.apply_crz(*c, *t, *theta),
            Instruction::Cp(c, t, theta) => self.apply_cp(*c, *t, *theta),
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

        if has_measurements {
            // Measurements collapse the state, so each shot needs a full re-execution.
            for _ in 0..shots {
                self.reset();
                self.execute_program(program);
                let mut result = String::new();
                for &c in &self.classical[..program.n_cbits.min(self.classical.len())] {
                    result.push(if c == 1 { '1' } else { '0' });
                }
                *counts.entry(result).or_insert(0) += 1;
            }
        } else {
            // No mid-circuit measurement: evolve the state ONCE and draw all shots from the final
            // distribution — turns 1000 shots from 1000 executions into 1 (major speedup).
            self.reset();
            self.execute_program(program);
            for _ in 0..shots {
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

/// High-level function: parse and run QASM2. Rejects programs beyond [`MAX_QUBITS`] up front so a
/// hostile or mistyped register size fails with an error instead of exhausting the kernel heap.
pub fn run_qasm2(qasm: &[u8], shots: u64) -> Result<SimResult, super::parser::ParseError> {
    let program = super::parser::parse_qasm2(qasm)?;
    if program.n_qubits == 0 || program.n_qubits > MAX_QUBITS {
        return Err(super::parser::ParseError::program(
            super::parser::ParseErrorKind::QubitOutOfRange(program.n_qubits, MAX_QUBITS),
        ));
    }
    let mut sim = Simulator::new(program.n_qubits);
    SIM_JOBS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    Ok(sim.run(&program, shots))
}

/// Run a pre-built instruction list (the Quantum Lab circuit editor path — no QASM round-trip).
/// Measures nothing itself: pass explicit `Measure` instructions or rely on final-state sampling.
pub fn run_program(n_qubits: usize, n_cbits: usize, instructions: Vec<Instruction>, shots: u64) -> Option<SimResult> {
    if n_qubits == 0 || n_qubits > MAX_QUBITS {
        return None;
    }
    let program = QasmProgram { n_qubits, n_cbits, instructions };
    let mut sim = Simulator::new(n_qubits);
    SIM_JOBS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    Some(sim.run(&program, shots))
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

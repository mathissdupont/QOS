//! Quantum State - QuantumState struct with efficient gate operations
//!
//! Implements statevector simulation without creating full 2^n x 2^n matrices.
//! Uses index-based updates for O(2^n) gate application instead of O(4^n).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::complex::Complex;

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

/// Seed the PRNG
pub fn seed_rng(seed: u64) {
    RNG_STATE.store(if seed == 0 { 1 } else { seed }, Ordering::Relaxed);
}

/// Generate random f64 in [0, 1)
fn random_f64() -> f64 {
    (xorshift64() as f64) / (u64::MAX as f64)
}

/// Quantum state holding a statevector
#[derive(Clone, Debug)]
pub struct QuantumState {
    /// Number of qubits
    pub n_qubits: usize,
    /// Statevector: 2^n complex amplitudes
    pub amplitudes: Vec<Complex>,
    /// Classical register for measurement results
    pub classical: Vec<u8>,
}

impl QuantumState {
    /// Create a new quantum state initialized to |0...0⟩
    pub fn new(n_qubits: usize) -> Self {
        let dim = 1 << n_qubits;
        let mut amplitudes = vec![Complex::ZERO; dim];
        amplitudes[0] = Complex::ONE; // |0...0⟩

        Self {
            n_qubits,
            amplitudes,
            classical: vec![0u8; n_qubits],
        }
    }

    /// Reset to |0...0⟩
    pub fn reset(&mut self) {
        for amp in self.amplitudes.iter_mut() {
            *amp = Complex::ZERO;
        }
        self.amplitudes[0] = Complex::ONE;
        for c in self.classical.iter_mut() {
            *c = 0;
        }
    }

    /// Get dimension of state space (2^n)
    #[inline]
    pub fn dim(&self) -> usize {
        self.amplitudes.len()
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPTIMIZED SINGLE-QUBIT GATE APPLICATION
    // Instead of creating 2^n x 2^n matrices, we update indices directly.
    // For a single-qubit gate G on qubit q, we process pairs of amplitudes.
    // ═══════════════════════════════════════════════════════════════════════════

    /// Apply a 2x2 unitary gate to a single qubit
    /// gate = [[a, b], [c, d]]
    fn apply_single_qubit(&mut self, target: usize, gate: [[Complex; 2]; 2]) {
        let n = self.n_qubits;
        let dim = self.dim();
        
        // Qubit index from MSB (qubit 0 is most significant)
        let bit_pos = n - 1 - target;
        let step = 1 << bit_pos;

        // Process pairs of amplitudes where only the target qubit differs
        let mut i = 0;
        while i < dim {
            for j in i..(i + step) {
                let idx0 = j;           // target qubit = 0
                let idx1 = j + step;    // target qubit = 1

                let a0 = self.amplitudes[idx0];
                let a1 = self.amplitudes[idx1];

                // Apply 2x2 gate: [new0, new1] = gate * [a0, a1]
                self.amplitudes[idx0] = gate[0][0] * a0 + gate[0][1] * a1;
                self.amplitudes[idx1] = gate[1][0] * a0 + gate[1][1] * a1;
            }
            i += step << 1;
        }
    }

    /// Apply Hadamard gate: H = 1/√2 * [[1, 1], [1, -1]]
    pub fn apply_h(&mut self, target: usize) {
        let s = Complex::INV_SQRT2;
        let gate = [
            [s, s],
            [s, Complex::new(-s.re, 0.0)],
        ];
        self.apply_single_qubit(target, gate);
    }

    /// Apply Pauli-X gate: X = [[0, 1], [1, 0]]
    pub fn apply_x(&mut self, target: usize) {
        let gate = [
            [Complex::ZERO, Complex::ONE],
            [Complex::ONE, Complex::ZERO],
        ];
        self.apply_single_qubit(target, gate);
    }

    /// Apply Pauli-Y gate: Y = [[0, -i], [i, 0]]
    pub fn apply_y(&mut self, target: usize) {
        let gate = [
            [Complex::ZERO, Complex::new(0.0, -1.0)],
            [Complex::I, Complex::ZERO],
        ];
        self.apply_single_qubit(target, gate);
    }

    /// Apply Pauli-Z gate: Z = [[1, 0], [0, -1]]
    pub fn apply_z(&mut self, target: usize) {
        let gate = [
            [Complex::ONE, Complex::ZERO],
            [Complex::ZERO, Complex::new(-1.0, 0.0)],
        ];
        self.apply_single_qubit(target, gate);
    }

    /// Apply S gate (√Z): S = [[1, 0], [0, i]]
    pub fn apply_s(&mut self, target: usize) {
        let gate = [
            [Complex::ONE, Complex::ZERO],
            [Complex::ZERO, Complex::I],
        ];
        self.apply_single_qubit(target, gate);
    }

    /// Apply T gate (√S): T = [[1, 0], [0, e^(iπ/4)]]
    pub fn apply_t(&mut self, target: usize) {
        let s = core::f64::consts::FRAC_1_SQRT_2;
        let gate = [
            [Complex::ONE, Complex::ZERO],
            [Complex::ZERO, Complex::new(s, s)], // e^(iπ/4) = (1+i)/√2
        ];
        self.apply_single_qubit(target, gate);
    }

    /// Apply Rz(θ) gate: Rz(θ) = [[e^(-iθ/2), 0], [0, e^(iθ/2)]]
    pub fn apply_rz(&mut self, target: usize, theta: f64) {
        let half = theta / 2.0;
        let cos_h = libm::cos(half);
        let sin_h = libm::sin(half);
        let gate = [
            [Complex::new(cos_h, -sin_h), Complex::ZERO],
            [Complex::ZERO, Complex::new(cos_h, sin_h)],
        ];
        self.apply_single_qubit(target, gate);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // OPTIMIZED TWO-QUBIT GATE APPLICATION
    // For CNOT, we only flip target amplitudes where control is |1⟩
    // ═══════════════════════════════════════════════════════════════════════════

    /// Apply CNOT (CX) gate: flip target if control is |1⟩
    pub fn apply_cx(&mut self, control: usize, target: usize) {
        let n = self.n_qubits;
        let dim = self.dim();

        let ctrl_bit = n - 1 - control;
        let targ_bit = n - 1 - target;
        let ctrl_mask = 1 << ctrl_bit;
        let targ_mask = 1 << targ_bit;

        // For each pair where control=1, swap amplitudes where target differs
        for i in 0..dim {
            // Only process if control bit is 1 AND target bit is 0
            // (to avoid swapping twice)
            if (i & ctrl_mask) != 0 && (i & targ_mask) == 0 {
                let j = i | targ_mask; // same but with target=1
                self.amplitudes.swap(i, j);
            }
        }
    }

    /// Apply CZ gate: apply Z to target if control is |1⟩
    pub fn apply_cz(&mut self, control: usize, target: usize) {
        let n = self.n_qubits;
        let dim = self.dim();

        let ctrl_bit = n - 1 - control;
        let targ_bit = n - 1 - target;
        let ctrl_mask = 1 << ctrl_bit;
        let targ_mask = 1 << targ_bit;

        // Phase flip when both control and target are |1⟩
        for i in 0..dim {
            if (i & ctrl_mask) != 0 && (i & targ_mask) != 0 {
                self.amplitudes[i] = self.amplitudes[i] * Complex::new(-1.0, 0.0);
            }
        }
    }

    /// Apply SWAP gate
    pub fn apply_swap(&mut self, q1: usize, q2: usize) {
        let n = self.n_qubits;
        let dim = self.dim();

        let bit1 = n - 1 - q1;
        let bit2 = n - 1 - q2;
        let mask1 = 1 << bit1;
        let mask2 = 1 << bit2;

        for i in 0..dim {
            let b1 = (i & mask1) != 0;
            let b2 = (i & mask2) != 0;
            
            // Only swap when bits differ (01 <-> 10)
            if b1 && !b2 {
                let j = (i & !mask1) | mask2; // swap the bits
                self.amplitudes.swap(i, j);
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // MEASUREMENT
    // ═══════════════════════════════════════════════════════════════════════════

    /// Measure a single qubit, collapsing the state
    pub fn measure_qubit(&mut self, qubit: usize) -> u8 {
        let n = self.n_qubits;
        let dim = self.dim();
        let bit_pos = n - 1 - qubit;
        let mask = 1 << bit_pos;

        // Calculate probability of measuring |1⟩
        let mut prob_one = 0.0;
        for i in 0..dim {
            if (i & mask) != 0 {
                prob_one += self.amplitudes[i].norm_sq();
            }
        }

        // Sample outcome
        let r = random_f64();
        let outcome = if r < prob_one { 1u8 } else { 0u8 };

        // Collapse the state
        let mut norm_sq = 0.0;
        for i in 0..dim {
            let is_one = (i & mask) != 0;
            if is_one != (outcome == 1) {
                self.amplitudes[i] = Complex::ZERO;
            } else {
                norm_sq += self.amplitudes[i].norm_sq();
            }
        }

        // Renormalize
        if norm_sq > 1e-15 {
            let norm_inv = 1.0 / libm::sqrt(norm_sq);
            for amp in self.amplitudes.iter_mut() {
                *amp = *amp * norm_inv;
            }
        }

        self.classical[qubit] = outcome;
        outcome
    }

    /// Measure all qubits and return bitstring
    pub fn measure_all(&mut self) -> String {
        let mut result = String::new();
        for q in 0..self.n_qubits {
            let bit = self.measure_qubit(q);
            result.push(if bit == 1 { '1' } else { '0' });
        }
        result
    }

    /// Sample a measurement outcome without collapse (for multiple shots)
    pub fn sample_outcome(&self) -> String {
        let r = random_f64();
        let mut cumulative = 0.0;

        for (i, amp) in self.amplitudes.iter().enumerate() {
            cumulative += amp.norm_sq();
            if r < cumulative {
                let mut s = String::new();
                for q in 0..self.n_qubits {
                    let bit = (i >> (self.n_qubits - 1 - q)) & 1;
                    s.push(if bit == 1 { '1' } else { '0' });
                }
                return s;
            }
        }

        // Fallback
        "0".repeat(self.n_qubits)
    }

    /// Sample a measurement outcome as an index (usize) without collapse
    /// Returns the computational basis state index
    pub fn sample_outcome_index(&self) -> usize {
        let r = random_f64();
        let mut cumulative = 0.0;

        for (i, amp) in self.amplitudes.iter().enumerate() {
            cumulative += amp.norm_sq();
            if r < cumulative {
                return i;
            }
        }

        // Fallback: return |0...0⟩
        0
    }

    /// Get the probability of measuring a specific computational basis state
    pub fn probability(&self, basis_state: usize) -> f64 {
        if basis_state < self.dim() {
            self.amplitudes[basis_state].norm_sq()
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let state = QuantumState::new(2);
        assert_eq!(state.n_qubits, 2);
        assert_eq!(state.dim(), 4);
        // Should be |00⟩
        assert!((state.amplitudes[0].re - 1.0).abs() < 1e-10);
        assert!((state.amplitudes[1].norm_sq()).abs() < 1e-10);
    }

    #[test]
    fn test_hadamard() {
        let mut state = QuantumState::new(1);
        state.apply_h(0);
        // Should be (|0⟩ + |1⟩)/√2
        let s = core::f64::consts::FRAC_1_SQRT_2;
        assert!((state.amplitudes[0].re - s).abs() < 1e-10);
        assert!((state.amplitudes[1].re - s).abs() < 1e-10);
    }

    #[test]
    fn test_bell_state() {
        let mut state = QuantumState::new(2);
        state.apply_h(0);
        state.apply_cx(0, 1);
        // Should be (|00⟩ + |11⟩)/√2
        let s = core::f64::consts::FRAC_1_SQRT_2;
        assert!((state.amplitudes[0].re - s).abs() < 1e-10); // |00⟩
        assert!((state.amplitudes[1].norm_sq()).abs() < 1e-10); // |01⟩
        assert!((state.amplitudes[2].norm_sq()).abs() < 1e-10); // |10⟩
        assert!((state.amplitudes[3].re - s).abs() < 1e-10); // |11⟩
    }
}

//! Quantum statevector with efficient (index-based) gate application.
//!
//! O(2^n) gate application without building 2^n × 2^n matrices.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::complex::Complex;

/// Simple PRNG (xorshift64) for measurement sampling.
static RNG_STATE: AtomicU64 = AtomicU64::new(0x853c49e6748fea9b);

fn xorshift64() -> u64 {
    let mut x = RNG_STATE.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    RNG_STATE.store(x, Ordering::Relaxed);
    x
}

/// Seed the PRNG (embodiments should call this with real entropy at boot).
pub fn seed_rng(seed: u64) {
    RNG_STATE.store(if seed == 0 { 1 } else { seed }, Ordering::Relaxed);
}

fn random_f64() -> f64 {
    (xorshift64() as f64) / (u64::MAX as f64)
}

/// Quantum state holding a statevector.
#[derive(Clone, Debug)]
pub struct QuantumState {
    pub n_qubits: usize,
    pub amplitudes: Vec<Complex>,
    pub classical: Vec<u8>,
}

impl QuantumState {
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

    pub fn reset(&mut self) {
        for amp in self.amplitudes.iter_mut() {
            *amp = Complex::ZERO;
        }
        self.amplitudes[0] = Complex::ONE;
        for c in self.classical.iter_mut() {
            *c = 0;
        }
    }

    #[inline]
    pub fn dim(&self) -> usize {
        self.amplitudes.len()
    }

    fn apply_single_qubit(&mut self, target: usize, gate: [[Complex; 2]; 2]) {
        let n = self.n_qubits;
        let dim = self.dim();
        let bit_pos = n - 1 - target;
        let step = 1 << bit_pos;

        let mut i = 0;
        while i < dim {
            for j in i..(i + step) {
                let idx0 = j;
                let idx1 = j + step;
                let a0 = self.amplitudes[idx0];
                let a1 = self.amplitudes[idx1];
                self.amplitudes[idx0] = gate[0][0] * a0 + gate[0][1] * a1;
                self.amplitudes[idx1] = gate[1][0] * a0 + gate[1][1] * a1;
            }
            i += step << 1;
        }
    }

    pub fn apply_h(&mut self, target: usize) {
        let s = Complex::INV_SQRT2;
        let gate = [[s, s], [s, Complex::new(-s.re, 0.0)]];
        self.apply_single_qubit(target, gate);
    }

    pub fn apply_x(&mut self, target: usize) {
        let gate = [
            [Complex::ZERO, Complex::ONE],
            [Complex::ONE, Complex::ZERO],
        ];
        self.apply_single_qubit(target, gate);
    }

    pub fn apply_y(&mut self, target: usize) {
        let gate = [
            [Complex::ZERO, Complex::new(0.0, -1.0)],
            [Complex::I, Complex::ZERO],
        ];
        self.apply_single_qubit(target, gate);
    }

    pub fn apply_z(&mut self, target: usize) {
        let gate = [
            [Complex::ONE, Complex::ZERO],
            [Complex::ZERO, Complex::new(-1.0, 0.0)],
        ];
        self.apply_single_qubit(target, gate);
    }

    pub fn apply_s(&mut self, target: usize) {
        let gate = [
            [Complex::ONE, Complex::ZERO],
            [Complex::ZERO, Complex::I],
        ];
        self.apply_single_qubit(target, gate);
    }

    pub fn apply_t(&mut self, target: usize) {
        let s = core::f64::consts::FRAC_1_SQRT_2;
        let gate = [
            [Complex::ONE, Complex::ZERO],
            [Complex::ZERO, Complex::new(s, s)], // e^(iπ/4)
        ];
        self.apply_single_qubit(target, gate);
    }

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

    pub fn apply_cx(&mut self, control: usize, target: usize) {
        let n = self.n_qubits;
        let dim = self.dim();
        let ctrl_mask = 1 << (n - 1 - control);
        let targ_mask = 1 << (n - 1 - target);
        for i in 0..dim {
            if (i & ctrl_mask) != 0 && (i & targ_mask) == 0 {
                let j = i | targ_mask;
                self.amplitudes.swap(i, j);
            }
        }
    }

    pub fn apply_cz(&mut self, control: usize, target: usize) {
        let n = self.n_qubits;
        let dim = self.dim();
        let ctrl_mask = 1 << (n - 1 - control);
        let targ_mask = 1 << (n - 1 - target);
        for i in 0..dim {
            if (i & ctrl_mask) != 0 && (i & targ_mask) != 0 {
                self.amplitudes[i] = self.amplitudes[i] * Complex::new(-1.0, 0.0);
            }
        }
    }

    pub fn apply_swap(&mut self, q1: usize, q2: usize) {
        let n = self.n_qubits;
        let dim = self.dim();
        let mask1 = 1 << (n - 1 - q1);
        let mask2 = 1 << (n - 1 - q2);
        for i in 0..dim {
            let b1 = (i & mask1) != 0;
            let b2 = (i & mask2) != 0;
            if b1 && !b2 {
                let j = (i & !mask1) | mask2;
                self.amplitudes.swap(i, j);
            }
        }
    }

    pub fn apply_sdg(&mut self, target: usize) {
        let gate = [
            [Complex::ONE, Complex::ZERO],
            [Complex::ZERO, Complex::new(0.0, -1.0)],
        ];
        self.apply_single_qubit(target, gate);
    }

    pub fn apply_tdg(&mut self, target: usize) {
        let s = core::f64::consts::FRAC_1_SQRT_2;
        let gate = [
            [Complex::ONE, Complex::ZERO],
            [Complex::ZERO, Complex::new(s, -s)],
        ];
        self.apply_single_qubit(target, gate);
    }

    pub fn apply_rx(&mut self, target: usize, theta: f64) {
        let half = theta / 2.0;
        let cos_h = libm::cos(half);
        let sin_h = libm::sin(half);
        let gate = [
            [Complex::new(cos_h, 0.0), Complex::new(0.0, -sin_h)],
            [Complex::new(0.0, -sin_h), Complex::new(cos_h, 0.0)],
        ];
        self.apply_single_qubit(target, gate);
    }

    pub fn apply_ry(&mut self, target: usize, theta: f64) {
        let half = theta / 2.0;
        let cos_h = libm::cos(half);
        let sin_h = libm::sin(half);
        let gate = [
            [Complex::new(cos_h, 0.0), Complex::new(-sin_h, 0.0)],
            [Complex::new(sin_h, 0.0), Complex::new(cos_h, 0.0)],
        ];
        self.apply_single_qubit(target, gate);
    }

    pub fn apply_ccx(&mut self, control1: usize, control2: usize, target: usize) {
        let n = self.n_qubits;
        let dim = self.dim();
        let ctrl1_mask = 1 << (n - 1 - control1);
        let ctrl2_mask = 1 << (n - 1 - control2);
        let targ_mask = 1 << (n - 1 - target);
        for i in 0..dim {
            if (i & ctrl1_mask) != 0 && (i & ctrl2_mask) != 0 && (i & targ_mask) == 0 {
                let j = i | targ_mask;
                self.amplitudes.swap(i, j);
            }
        }
    }

    pub fn apply_u3(&mut self, target: usize, theta: f64, phi: f64, lambda: f64) {
        let cos_h = libm::cos(theta / 2.0);
        let sin_h = libm::sin(theta / 2.0);
        let gate = [
            [
                Complex::new(cos_h, 0.0),
                Complex::new(-libm::cos(lambda) * sin_h, -libm::sin(lambda) * sin_h),
            ],
            [
                Complex::new(libm::cos(phi) * sin_h, libm::sin(phi) * sin_h),
                Complex::new(libm::cos(phi + lambda) * cos_h, libm::sin(phi + lambda) * cos_h),
            ],
        ];
        self.apply_single_qubit(target, gate);
    }

    /// Measure a single qubit, collapsing the state.
    pub fn measure_qubit(&mut self, qubit: usize) -> u8 {
        let n = self.n_qubits;
        let dim = self.dim();
        let mask = 1 << (n - 1 - qubit);

        let mut prob_one = 0.0;
        for i in 0..dim {
            if (i & mask) != 0 {
                prob_one += self.amplitudes[i].norm_sq();
            }
        }

        let r = random_f64();
        let outcome = if r < prob_one { 1u8 } else { 0u8 };

        let mut norm_sq = 0.0;
        for i in 0..dim {
            let is_one = (i & mask) != 0;
            if is_one != (outcome == 1) {
                self.amplitudes[i] = Complex::ZERO;
            } else {
                norm_sq += self.amplitudes[i].norm_sq();
            }
        }

        if norm_sq > 1e-15 {
            let norm_inv = 1.0 / libm::sqrt(norm_sq);
            for amp in self.amplitudes.iter_mut() {
                *amp = *amp * norm_inv;
            }
        }

        self.classical[qubit] = outcome;
        outcome
    }

    pub fn measure_all(&mut self) -> String {
        let mut result = String::new();
        for q in 0..self.n_qubits {
            let bit = self.measure_qubit(q);
            result.push(if bit == 1 { '1' } else { '0' });
        }
        result
    }

    /// Sample a measurement outcome without collapse (for multi-shot sampling).
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
        let mut s = String::new();
        for _ in 0..self.n_qubits {
            s.push('0');
        }
        s
    }

    pub fn sample_outcome_index(&self) -> usize {
        let r = random_f64();
        let mut cumulative = 0.0;
        for (i, amp) in self.amplitudes.iter().enumerate() {
            cumulative += amp.norm_sq();
            if r < cumulative {
                return i;
            }
        }
        0
    }

    pub fn probability(&self, basis_state: usize) -> f64 {
        if basis_state < self.dim() {
            self.amplitudes[basis_state].norm_sq()
        } else {
            0.0
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_zero() {
        let state = QuantumState::new(2);
        assert_eq!(state.dim(), 4);
        assert!((state.amplitudes[0].re - 1.0).abs() < 1e-10);
    }

    #[test]
    fn hadamard_superposes() {
        let mut state = QuantumState::new(1);
        state.apply_h(0);
        let s = core::f64::consts::FRAC_1_SQRT_2;
        assert!((state.amplitudes[0].re - s).abs() < 1e-10);
        assert!((state.amplitudes[1].re - s).abs() < 1e-10);
    }

    #[test]
    fn bell_state_entangles() {
        let mut state = QuantumState::new(2);
        state.apply_h(0);
        state.apply_cx(0, 1);
        let s = core::f64::consts::FRAC_1_SQRT_2;
        assert!((state.amplitudes[0].re - s).abs() < 1e-10); // |00⟩
        assert!(state.amplitudes[1].norm_sq() < 1e-10); // |01⟩
        assert!(state.amplitudes[2].norm_sq() < 1e-10); // |10⟩
        assert!((state.amplitudes[3].re - s).abs() < 1e-10); // |11⟩
    }
}

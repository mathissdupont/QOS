//! Linear Algebra primitives for quantum simulation.
//!
//! Provides Complex numbers and Matrix operations using only `alloc`.

use alloc::vec;
use alloc::vec::Vec;
use core::ops::{Add, AddAssign, Mul, MulAssign, Sub};

/// A complex number with f64 real and imaginary parts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };
    pub const ONE: Self = Self { re: 1.0, im: 0.0 };
    pub const I: Self = Self { re: 0.0, im: 1.0 };

    #[inline]
    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    /// Squared magnitude |z|^2 = re^2 + im^2
    #[inline]
    pub fn norm_sq(self) -> f64 {
        self.re * self.re + self.im * self.im
    }

    /// Magnitude |z|
    #[inline]
    pub fn norm(self) -> f64 {
        libm::sqrt(self.norm_sq())
    }

    /// Complex conjugate
    #[inline]
    pub fn conj(self) -> Self {
        Self {
            re: self.re,
            im: -self.im,
        }
    }

    /// 1/sqrt(2) - commonly used in quantum gates
    #[inline]
    pub fn inv_sqrt2() -> f64 {
        core::f64::consts::FRAC_1_SQRT_2
    }
}

impl Default for Complex {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Add for Complex {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            re: self.re + rhs.re,
            im: self.im + rhs.im,
        }
    }
}

impl AddAssign for Complex {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.re += rhs.re;
        self.im += rhs.im;
    }
}

impl Sub for Complex {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            re: self.re - rhs.re,
            im: self.im - rhs.im,
        }
    }
}

impl Mul for Complex {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        // (a+bi)(c+di) = (ac-bd) + (ad+bc)i
        Self {
            re: self.re * rhs.re - self.im * rhs.im,
            im: self.re * rhs.im + self.im * rhs.re,
        }
    }
}

impl MulAssign for Complex {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl Mul<f64> for Complex {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f64) -> Self {
        Self {
            re: self.re * rhs,
            im: self.im * rhs,
        }
    }
}

/// A dense matrix stored in row-major order.
#[derive(Clone, Debug)]
pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<Complex>,
}

impl Matrix {
    /// Create a zero matrix
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            data: vec![Complex::ZERO; rows * cols],
        }
    }

    /// Create an identity matrix
    pub fn identity(n: usize) -> Self {
        let mut m = Self::zeros(n, n);
        for i in 0..n {
            m.set(i, i, Complex::ONE);
        }
        m
    }

    /// Get element at (row, col)
    #[inline]
    pub fn get(&self, row: usize, col: usize) -> Complex {
        self.data[row * self.cols + col]
    }

    /// Set element at (row, col)
    #[inline]
    pub fn set(&mut self, row: usize, col: usize, val: Complex) {
        self.data[row * self.cols + col] = val;
    }

    /// Matrix-vector multiplication: result = self * vec
    pub fn mul_vec(&self, vec: &[Complex]) -> Vec<Complex> {
        assert_eq!(self.cols, vec.len());
        let mut result = vec![Complex::ZERO; self.rows];
        for i in 0..self.rows {
            let mut sum = Complex::ZERO;
            for j in 0..self.cols {
                sum += self.get(i, j) * vec[j];
            }
            result[i] = sum;
        }
        result
    }

    /// Tensor product (Kronecker product): self ⊗ other
    pub fn tensor(&self, other: &Matrix) -> Matrix {
        let new_rows = self.rows * other.rows;
        let new_cols = self.cols * other.cols;
        let mut result = Matrix::zeros(new_rows, new_cols);

        for i in 0..self.rows {
            for j in 0..self.cols {
                let a = self.get(i, j);
                for k in 0..other.rows {
                    for l in 0..other.cols {
                        let b = other.get(k, l);
                        let row = i * other.rows + k;
                        let col = j * other.cols + l;
                        result.set(row, col, a * b);
                    }
                }
            }
        }
        result
    }

    /// Matrix multiplication: self * other
    pub fn mul_mat(&self, other: &Matrix) -> Matrix {
        assert_eq!(self.cols, other.rows);
        let mut result = Matrix::zeros(self.rows, other.cols);
        for i in 0..self.rows {
            for j in 0..other.cols {
                let mut sum = Complex::ZERO;
                for k in 0..self.cols {
                    sum += self.get(i, k) * other.get(k, j);
                }
                result.set(i, j, sum);
            }
        }
        result
    }
}

// ============ Standard Quantum Gates ============

/// Hadamard gate: H = 1/sqrt(2) * [[1,1],[1,-1]]
pub fn gate_h() -> Matrix {
    let s = Complex::inv_sqrt2();
    let mut m = Matrix::zeros(2, 2);
    m.set(0, 0, Complex::new(s, 0.0));
    m.set(0, 1, Complex::new(s, 0.0));
    m.set(1, 0, Complex::new(s, 0.0));
    m.set(1, 1, Complex::new(-s, 0.0));
    m
}

/// Pauli-X gate (NOT): X = [[0,1],[1,0]]
pub fn gate_x() -> Matrix {
    let mut m = Matrix::zeros(2, 2);
    m.set(0, 1, Complex::ONE);
    m.set(1, 0, Complex::ONE);
    m
}

/// Pauli-Y gate: Y = [[0,-i],[i,0]]
pub fn gate_y() -> Matrix {
    let mut m = Matrix::zeros(2, 2);
    m.set(0, 1, Complex::new(0.0, -1.0));
    m.set(1, 0, Complex::new(0.0, 1.0));
    m
}

/// Pauli-Z gate: Z = [[1,0],[0,-1]]
pub fn gate_z() -> Matrix {
    let mut m = Matrix::zeros(2, 2);
    m.set(0, 0, Complex::ONE);
    m.set(1, 1, Complex::new(-1.0, 0.0));
    m
}

/// S gate (Phase gate): S = [[1,0],[0,i]]
pub fn gate_s() -> Matrix {
    let mut m = Matrix::zeros(2, 2);
    m.set(0, 0, Complex::ONE);
    m.set(1, 1, Complex::I);
    m
}

/// T gate: T = [[1,0],[0,e^(i*pi/4)]]
pub fn gate_t() -> Matrix {
    let angle = core::f64::consts::FRAC_PI_4;
    let mut m = Matrix::zeros(2, 2);
    m.set(0, 0, Complex::ONE);
    m.set(1, 1, Complex::new(libm::cos(angle), libm::sin(angle)));
    m
}

/// Identity gate for 1 qubit
pub fn gate_id() -> Matrix {
    Matrix::identity(2)
}

/// CNOT (CX) gate for 2 qubits (control=0, target=1)
/// |00⟩ → |00⟩, |01⟩ → |01⟩, |10⟩ → |11⟩, |11⟩ → |10⟩
pub fn gate_cx() -> Matrix {
    let mut m = Matrix::zeros(4, 4);
    m.set(0, 0, Complex::ONE); // |00⟩ → |00⟩
    m.set(1, 1, Complex::ONE); // |01⟩ → |01⟩
    m.set(2, 3, Complex::ONE); // |10⟩ → |11⟩
    m.set(3, 2, Complex::ONE); // |11⟩ → |10⟩
    m
}

/// CZ gate for 2 qubits
pub fn gate_cz() -> Matrix {
    let mut m = Matrix::zeros(4, 4);
    m.set(0, 0, Complex::ONE);
    m.set(1, 1, Complex::ONE);
    m.set(2, 2, Complex::ONE);
    m.set(3, 3, Complex::new(-1.0, 0.0));
    m
}

/// SWAP gate for 2 qubits
pub fn gate_swap() -> Matrix {
    let mut m = Matrix::zeros(4, 4);
    m.set(0, 0, Complex::ONE); // |00⟩ → |00⟩
    m.set(1, 2, Complex::ONE); // |01⟩ → |10⟩
    m.set(2, 1, Complex::ONE); // |10⟩ → |01⟩
    m.set(3, 3, Complex::ONE); // |11⟩ → |11⟩
    m
}

/// Build a single-qubit gate expanded to n qubits, applied to qubit `target`.
/// Uses: I ⊗ I ⊗ ... ⊗ G ⊗ ... ⊗ I
pub fn expand_single_gate(gate: &Matrix, target: usize, n_qubits: usize) -> Matrix {
    assert!(target < n_qubits);
    
    let mut result = if target == 0 {
        gate.clone()
    } else {
        Matrix::identity(2)
    };

    for i in 1..n_qubits {
        let next = if i == target { gate.clone() } else { Matrix::identity(2) };
        result = result.tensor(&next);
    }

    result
}

/// Build a two-qubit gate (like CNOT) for arbitrary control/target positions.
/// This handles the more complex case of non-adjacent qubits.
pub fn expand_two_qubit_gate(
    gate_2q: &Matrix,
    ctrl: usize,
    targ: usize,
    n_qubits: usize,
) -> Matrix {
    assert!(ctrl < n_qubits && targ < n_qubits && ctrl != targ);

    let dim = 1 << n_qubits;
    let mut result = Matrix::zeros(dim, dim);

    // For each basis state, compute the output
    for i in 0..dim {
        let ctrl_bit = (i >> (n_qubits - 1 - ctrl)) & 1;
        let targ_bit = (i >> (n_qubits - 1 - targ)) & 1;

        // 2-qubit index: ctrl is high bit, targ is low bit
        let idx_2q = (ctrl_bit << 1) | targ_bit;

        // Find which output states this maps to
        for j_2q in 0..4 {
            let amp = gate_2q.get(j_2q, idx_2q);
            if amp.norm_sq() < 1e-15 {
                continue;
            }

            let new_ctrl_bit = (j_2q >> 1) & 1;
            let new_targ_bit = j_2q & 1;

            // Build output state index
            let mut out = i;
            // Clear and set ctrl bit
            out = out & !(1 << (n_qubits - 1 - ctrl));
            out = out | (new_ctrl_bit << (n_qubits - 1 - ctrl));
            // Clear and set targ bit
            out = out & !(1 << (n_qubits - 1 - targ));
            out = out | (new_targ_bit << (n_qubits - 1 - targ));

            let current = result.get(out, i);
            result.set(out, i, current + amp);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complex_mul() {
        let a = Complex::new(1.0, 2.0);
        let b = Complex::new(3.0, 4.0);
        let c = a * b;
        // (1+2i)(3+4i) = 3+4i+6i+8i² = 3+10i-8 = -5+10i
        assert!((c.re - (-5.0)).abs() < 1e-10);
        assert!((c.im - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_hadamard_twice() {
        let h = gate_h();
        let hh = h.mul_mat(&h);
        // H*H = I
        assert!((hh.get(0, 0).re - 1.0).abs() < 1e-10);
        assert!((hh.get(1, 1).re - 1.0).abs() < 1e-10);
        assert!(hh.get(0, 1).norm_sq() < 1e-10);
        assert!(hh.get(1, 0).norm_sq() < 1e-10);
    }
}

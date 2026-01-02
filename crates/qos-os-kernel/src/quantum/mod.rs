//! Quantum Computing Module for QOS
//!
//! This module provides a real statevector quantum simulator
//! that runs inside the kernel. It supports a subset of OpenQASM 2.0.
//!
//! ## New Architecture (v2)
//! - `complex`: Complex number arithmetic
//! - `state`: QuantumState with optimized gate operations (no full matrices)
//! - `circuit`: Circuit representation with step-by-step execution
//! - `qasm`: QASM parser producing Circuit
//!
//! ## Legacy modules (kept for compatibility)
//! - `linalg`, `parser`, `sim`: Original matrix-based implementation

// New modules (v2)
pub mod complex;
pub mod state;
pub mod circuit;
pub mod qasm;

// Legacy modules for backward compatibility
pub mod linalg;
pub mod parser;
pub mod sim;

// Re-export new types
pub use complex::Complex;
pub use state::QuantumState;
pub use circuit::{Circuit, Gate, SimulationResult, bell_circuit, ghz_circuit};
pub use qasm::{parse_qasm, ParseError};

// Legacy re-exports (aliased to avoid conflicts)
pub use linalg::Complex as MatrixComplex;
pub use parser::{parse_qasm2, Instruction, ParseError as ParserError, QasmProgram};
pub use sim::{Simulator, SimResult, run_qasm2, run_bell};

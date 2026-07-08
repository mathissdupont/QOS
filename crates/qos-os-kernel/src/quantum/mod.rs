//! Quantum Computing Module for QOS
//!
//! This module provides a real statevector quantum simulator
//! that runs inside the kernel. It supports a subset of OpenQASM 2.0.
//!
//! ## Architecture
//! - `complex`: Complex number arithmetic
//! - `state`: QuantumState with optimized gate operations (no full matrices)
//! - `circuit`: Circuit representation with step-by-step execution
//! - `qasm`: QASM parser producing Circuit
//! - `backend`: Backend abstraction for QPU integration
//!
//! ## Backend Support
//! - Local simulator (default, up to 32 qubits)
//! - Remote QPU framework (IBM, Google, IonQ - requires networking)
//!
//! ## Legacy modules (kept for compatibility)
//! - `linalg`, `parser`, `sim`: Original matrix-based implementation

// Core modules
pub mod complex;
pub mod state;
pub mod circuit;
pub mod qasm;

// Backend abstraction layer for QPU support
pub mod backend;

// Legacy modules for backward compatibility
pub mod linalg;
pub mod parser;
pub mod sim;

// Transpile passes: gate cancellation + depth analysis (WP-06, E-80)
pub mod transpile;

// Re-export core types
#[allow(unused_imports)]
pub use complex::Complex;
pub use state::QuantumState;
#[allow(unused_imports)]
pub use circuit::{Circuit, Gate, SimulationResult, bell_circuit, ghz_circuit};
#[allow(unused_imports)]
pub use qasm::{parse_qasm, ParseError};

// Re-export backend types
#[allow(unused_imports)]
pub use backend::{
    QuantumBackend, BackendCapabilities, BackendStatus, BackendResult,
    BackendError, BackendManager, JobPriority, JobOptions, JobSubmission,
    LocalSimulatorBackend, RemoteQpuBackend, RemoteQpuConfig,
    ConnectivityMap, ErrorRates, GateType,
    CircuitTranspiler, ErrorMitigator, MitigationStrategy,
};

// Legacy re-exports (aliased to avoid conflicts)
#[allow(unused_imports)]
pub use linalg::Complex as MatrixComplex;
pub use parser::{parse_qasm2, Instruction, ParseError as ParserError, QasmProgram};
pub use sim::{Simulator, SimResult, run_qasm2, run_bell};

/// Extract qubit count from QASM string without full parsing
pub fn count_qubits_from_qasm(qasm: &str) -> u32 {
    // Quick regex-style search for "qreg q[N];"
    let bytes = qasm.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for "qreg"
        if i + 4 < bytes.len() && &bytes[i..i+4] == b"qreg" {
            // Skip whitespace
            i += 4;
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            // Should be 'q' or 'Q'
            if i < bytes.len() && (bytes[i] == b'q' || bytes[i] == b'Q') {
                i += 1;
            }
            // Should be '['
            while i < bytes.len() && bytes[i] != b'[' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                // Parse number
                let mut num_str = alloc::string::String::new();
                while i < bytes.len() && bytes[i] >= b'0' && bytes[i] <= b'9' {
                    num_str.push(bytes[i] as char);
                    i += 1;
                }
                if let Ok(n) = num_str.parse::<u32>() {
                    return n;
                }
            }
        }
        i += 1;
    }
    0 // Default if not found
}

//! In-kernel statevector simulator — the reference backend behind the QHAL (ADR-0004).
//!
//! Moved into `qos-core` from the kernel so both embodiments share one simulator and the
//! typed `Circuit` IR (ADR-0005). `no_std + alloc`, using `libm` for transcendental math.

pub mod circuit;
pub mod complex;
pub mod state;

pub use circuit::{bell_circuit, ghz_circuit, Circuit, Gate, SimulationResult};
pub use complex::Complex;
pub use state::QuantumState;

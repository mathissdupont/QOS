//! Quantum Backend Abstraction Layer
//!
//! This module provides the infrastructure needed for real QPU integration.
//! It defines traits and structures that allow QaOS to work with different
//! quantum backends: local simulators, remote cloud QPUs, or future hardware.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::circuit::Circuit;
#[allow(unused_imports)]
use super::state::QuantumState;

// ============================================================================
// Backend Provider Trait
// ============================================================================

/// Quantum backend capabilities
#[derive(Clone, Debug, Default)]
pub struct BackendCapabilities {
    /// Maximum number of qubits supported
    pub max_qubits: usize,
    /// Maximum circuit depth (gate count)
    pub max_depth: usize,
    /// Supported gate set
    pub supported_gates: Vec<GateType>,
    /// Whether mid-circuit measurement is supported
    pub mid_circuit_measurement: bool,
    /// Whether reset is supported
    pub reset_supported: bool,
    /// Whether conditional operations are supported
    pub conditional_supported: bool,
    /// Native gate set (for transpilation)
    pub native_gates: Vec<GateType>,
    /// Connectivity map (which qubits can interact)
    pub connectivity: Option<ConnectivityMap>,
    /// Error rates (for error mitigation)
    pub error_rates: Option<ErrorRates>,
}

/// Gate types for capability checking
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateType {
    // Single qubit gates
    H,
    X,
    Y,
    Z,
    S,
    Sdg,
    T,
    Tdg,
    Rx,
    Ry,
    Rz,
    U1,
    U2,
    U3,
    Id,
    // Two qubit gates
    Cx,
    Cz,
    Swap,
    // Three qubit gates
    Ccx,  // Toffoli
    Cswap, // Fredkin
    // Measurement
    Measure,
    Reset,
    Barrier,
}

/// Qubit connectivity map for hardware backends
#[derive(Clone, Debug)]
pub struct ConnectivityMap {
    /// Number of physical qubits
    pub n_qubits: usize,
    /// Adjacency list: connections[i] = qubits connected to qubit i
    pub connections: Vec<Vec<usize>>,
}

impl ConnectivityMap {
    /// Create a fully connected topology (all-to-all)
    pub fn fully_connected(n_qubits: usize) -> Self {
        let connections = (0..n_qubits)
            .map(|i| (0..n_qubits).filter(|&j| i != j).collect())
            .collect();
        Self { n_qubits, connections }
    }
    
    /// Create a linear topology (1D chain)
    pub fn linear(n_qubits: usize) -> Self {
        let connections = (0..n_qubits)
            .map(|i| {
                let mut c = Vec::new();
                if i > 0 { c.push(i - 1); }
                if i < n_qubits - 1 { c.push(i + 1); }
                c
            })
            .collect();
        Self { n_qubits, connections }
    }
    
    /// Create a grid topology (2D lattice)
    pub fn grid(rows: usize, cols: usize) -> Self {
        let n_qubits = rows * cols;
        let connections = (0..n_qubits)
            .map(|i| {
                let row = i / cols;
                let col = i % cols;
                let mut c = Vec::new();
                if row > 0 { c.push(i - cols); }
                if row < rows - 1 { c.push(i + cols); }
                if col > 0 { c.push(i - 1); }
                if col < cols - 1 { c.push(i + 1); }
                c
            })
            .collect();
        Self { n_qubits, connections }
    }
    
    /// Check if two qubits are directly connected
    pub fn is_connected(&self, q1: usize, q2: usize) -> bool {
        if q1 >= self.n_qubits || q2 >= self.n_qubits {
            return false;
        }
        self.connections[q1].contains(&q2)
    }
}

/// Error rates for error mitigation
#[derive(Clone, Debug, Default)]
pub struct ErrorRates {
    /// Single qubit gate error rates per qubit
    pub single_qubit_errors: Vec<f64>,
    /// Two qubit gate error rates per qubit pair (q1, q2) -> error
    pub two_qubit_errors: BTreeMap<(usize, usize), f64>,
    /// Readout (measurement) error rates per qubit [p(0|1), p(1|0)]
    pub readout_errors: Vec<(f64, f64)>,
    /// T1 decoherence times (microseconds)
    pub t1_times: Vec<f64>,
    /// T2 decoherence times (microseconds)
    pub t2_times: Vec<f64>,
}

// ============================================================================
// Backend Status & Results
// ============================================================================

/// Status of a backend
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendStatus {
    /// Backend is available and ready
    Available,
    /// Backend is busy processing jobs
    Busy,
    /// Backend is offline or unreachable
    Offline,
    /// Backend requires calibration
    NeedsCalibration,
    /// Backend in maintenance mode
    Maintenance,
}

/// Result from backend execution
#[derive(Clone, Debug)]
pub struct BackendResult {
    /// Job identifier
    pub job_id: u64,
    /// Measurement counts: bitstring -> count
    pub counts: BTreeMap<String, u64>,
    /// Number of shots executed
    pub shots: u64,
    /// Execution time in microseconds
    pub execution_time_us: u64,
    /// Backend-specific metadata
    pub metadata: Option<BackendMetadata>,
}

/// Backend-specific metadata
#[derive(Clone, Debug)]
pub struct BackendMetadata {
    /// Backend name
    pub backend_name: String,
    /// Backend version
    pub backend_version: String,
    /// Calibration timestamp
    pub calibration_time: Option<u64>,
    /// Additional info
    pub extra: BTreeMap<String, String>,
}

// ============================================================================
// Backend Provider Trait
// ============================================================================

/// Job submission handle
#[derive(Clone, Debug)]
pub struct JobSubmission {
    pub circuit: Circuit,
    pub shots: u32,
    pub priority: JobPriority,
    pub options: JobOptions,
}

/// Job priority levels
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Default for JobPriority {
    fn default() -> Self {
        JobPriority::Normal
    }
}

/// Job execution options
#[derive(Clone, Debug, Default)]
pub struct JobOptions {
    /// Enable error mitigation
    pub error_mitigation: bool,
    /// Optimization level (0-3)
    pub optimization_level: u8,
    /// Timeout in milliseconds (0 = no timeout)
    pub timeout_ms: u64,
    /// Custom backend options
    pub custom: BTreeMap<String, String>,
}

/// Backend error types
#[derive(Clone, Debug)]
pub enum BackendError {
    /// Backend is not available
    NotAvailable,
    /// Circuit exceeds backend capabilities
    CircuitTooLarge { max_qubits: usize, requested: usize },
    /// Unsupported gate
    UnsupportedGate(GateType),
    /// Connectivity constraint violation
    ConnectivityViolation { q1: usize, q2: usize },
    /// Job timed out
    Timeout,
    /// Backend communication error
    CommunicationError(String),
    /// Calibration required
    CalibrationRequired,
    /// Configuration error
    ConfigError(String),
    /// Generic error
    Other(String),
}

/// Trait for quantum backend providers
pub trait QuantumBackend: Send + Sync {
    /// Get backend name
    fn name(&self) -> &str;
    
    /// Get backend capabilities
    fn capabilities(&self) -> &BackendCapabilities;
    
    /// Get current backend status
    fn status(&self) -> BackendStatus;
    
    /// Submit a job for execution
    fn submit(&self, job: JobSubmission) -> Result<u64, BackendError>;
    
    /// Check job status
    fn job_status(&self, job_id: u64) -> Option<JobState>;
    
    /// Get job result (blocking)
    fn get_result(&self, job_id: u64) -> Option<BackendResult>;
    
    /// Cancel a job
    fn cancel(&self, job_id: u64) -> bool;
    
    /// Validate a circuit against backend capabilities
    fn validate_circuit(&self, circuit: &Circuit) -> Result<(), BackendError> {
        let caps = self.capabilities();
        
        // Check qubit count
        if circuit.n_qubits > caps.max_qubits {
            return Err(BackendError::CircuitTooLarge {
                max_qubits: caps.max_qubits,
                requested: circuit.n_qubits,
            });
        }
        
        // Check circuit depth
        if circuit.gates.len() > caps.max_depth && caps.max_depth > 0 {
            return Err(BackendError::CircuitTooLarge {
                max_qubits: caps.max_depth,
                requested: circuit.gates.len(),
            });
        }
        
        Ok(())
    }
}

/// Job state (backend-agnostic)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Pending,
    Running,
    Completed,
    Succeeded,
    Failed,
    Cancelled,
}

// ============================================================================
// Local Simulator Backend
// ============================================================================

/// Local statevector simulator backend
pub struct LocalSimulatorBackend {
    name: String,
    capabilities: BackendCapabilities,
    next_job_id: AtomicU64,
}

impl LocalSimulatorBackend {
    pub fn new(max_qubits: usize) -> Self {
        let capabilities = BackendCapabilities {
            max_qubits,
            max_depth: 100000, // Virtually unlimited for simulator
            supported_gates: vec![
                GateType::H, GateType::X, GateType::Y, GateType::Z,
                GateType::S, GateType::T, GateType::Rx, GateType::Ry, GateType::Rz,
                GateType::Cx, GateType::Cz, GateType::Swap,
                GateType::Measure, GateType::Reset, GateType::Barrier,
            ],
            mid_circuit_measurement: true,
            reset_supported: true,
            conditional_supported: true,
            native_gates: vec![
                GateType::H, GateType::X, GateType::Y, GateType::Z,
                GateType::S, GateType::T, GateType::Rz,
                GateType::Cx, GateType::Cz,
            ],
            connectivity: Some(ConnectivityMap::fully_connected(max_qubits)),
            error_rates: None, // Perfect simulator
        };
        
        Self {
            name: String::from("QaOS Local Simulator"),
            capabilities,
            next_job_id: AtomicU64::new(1),
        }
    }
}

impl QuantumBackend for LocalSimulatorBackend {
    fn name(&self) -> &str {
        &self.name
    }
    
    fn capabilities(&self) -> &BackendCapabilities {
        &self.capabilities
    }
    
    fn status(&self) -> BackendStatus {
        BackendStatus::Available
    }
    
    fn submit(&self, job: JobSubmission) -> Result<u64, BackendError> {
        self.validate_circuit(&job.circuit)?;
        
        let job_id = self.next_job_id.fetch_add(1, Ordering::Relaxed);
        // In a real implementation, this would queue the job
        // For now, we just return the ID - actual execution happens via syscall layer
        Ok(job_id)
    }
    
    fn job_status(&self, _job_id: u64) -> Option<JobState> {
        // Delegate to syscall layer
        None
    }
    
    fn get_result(&self, _job_id: u64) -> Option<BackendResult> {
        // Delegate to syscall layer
        None
    }
    
    fn cancel(&self, _job_id: u64) -> bool {
        // Delegate to syscall layer
        false
    }
}

// ============================================================================
// Remote QPU Backend (Framework)
// ============================================================================

/// Configuration for remote QPU connection
#[derive(Clone, Debug)]
pub struct RemoteQpuConfig {
    /// Provider name (ibm, google, ionq, etc.)
    pub provider: String,
    /// API endpoint URL
    pub endpoint: String,
    /// API key/token
    pub api_key: String,
    /// Backend/device name
    pub backend_name: String,
    /// Connection timeout in ms
    pub timeout_ms: u64,
}

/// Remote QPU backend using HTTP client
pub struct RemoteQpuBackend {
    config: RemoteQpuConfig,
    capabilities: BackendCapabilities,
    status: BackendStatus,
    /// Cached jobs (job_id -> state)
    jobs: BTreeMap<u64, JobState>,
    /// Job results (job_id -> result)
    results: BTreeMap<u64, BackendResult>,
    /// Next job ID (local tracking)
    next_job_id: AtomicU64,
}

impl RemoteQpuBackend {
    /// Create a new remote QPU backend
    pub fn new(config: RemoteQpuConfig) -> Self {
        // Default capabilities - should be fetched from the remote API
        let capabilities = BackendCapabilities {
            max_qubits: 0,  // Unknown until connected
            max_depth: 0,
            supported_gates: Vec::new(),
            mid_circuit_measurement: false,
            reset_supported: false,
            conditional_supported: false,
            native_gates: Vec::new(),
            connectivity: None,
            error_rates: None,
        };
        
        Self {
            config,
            capabilities,
            status: BackendStatus::Offline,
            jobs: BTreeMap::new(),
            results: BTreeMap::new(),
            next_job_id: AtomicU64::new(1),
        }
    }
    
    /// Connect to the remote QPU and fetch capabilities
    pub fn connect(&mut self) -> Result<(), BackendError> {
        use crate::http::{Request, HttpError};
        
        // Build capability endpoint URL
        let url = match self.config.provider.as_str() {
            "ibm" => format!("{}/backends/{}", self.config.endpoint, self.config.backend_name),
            "ionq" => format!("{}/backends/{}", self.config.endpoint, self.config.backend_name),
            "google" => format!("{}/projects/-/locations/-/processors/{}", 
                               self.config.endpoint, self.config.backend_name),
            _ => return Err(BackendError::ConfigError(String::from("Unknown provider"))),
        };
        
        // Fetch backend info
        let response = Request::get(&url)
            .map_err(|_| BackendError::CommunicationError(String::from("Invalid URL")))?
            .bearer_auth(&self.config.api_key)
            .timeout(self.config.timeout_ms)
            .send();
        
        match response {
            Ok(resp) => {
                if resp.is_success() {
                    // Parse capabilities from response
                    if let Ok(body) = resp.text() {
                        self.parse_capabilities(&body)?;
                    }
                    self.status = BackendStatus::Available;
                    Ok(())
                } else {
                    Err(BackendError::CommunicationError(
                        format!("API error: HTTP {}", resp.status.0)
                    ))
                }
            }
            Err(e) => {
                Err(BackendError::CommunicationError(format!("{:?}", e)))
            }
        }
    }
    
    /// Parse capabilities from JSON response
    fn parse_capabilities(&mut self, json: &str) -> Result<(), BackendError> {
        // Simple JSON parsing for key fields
        // In production, use a proper JSON parser
        
        // Extract max qubits
        if let Some(start) = json.find("\"n_qubits\"") {
            if let Some(colon) = json[start..].find(':') {
                let num_start = start + colon + 1;
                let num_str: String = json[num_start..]
                    .chars()
                    .skip_while(|c| c.is_whitespace())
                    .take_while(|c| c.is_numeric())
                    .collect();
                if let Ok(n) = num_str.parse::<usize>() {
                    self.capabilities.max_qubits = n;
                }
            }
        }
        
        // Extract max_shots or similar fields based on provider
        // This is simplified - real implementation needs proper JSON parsing
        
        Ok(())
    }
    
    /// Fetch latest calibration data
    pub fn fetch_calibration(&mut self) -> Result<(), BackendError> {
        use crate::http::Request;
        
        if self.status != BackendStatus::Available {
            return Err(BackendError::NotAvailable);
        }
        
        let url = match self.config.provider.as_str() {
            "ibm" => format!("{}/backends/{}/properties", 
                            self.config.endpoint, self.config.backend_name),
            "ionq" => format!("{}/characterizations/current", self.config.endpoint),
            _ => return Ok(()), // No calibration endpoint
        };
        
        let response = Request::get(&url)
            .map_err(|_| BackendError::CommunicationError(String::from("Invalid URL")))?
            .bearer_auth(&self.config.api_key)
            .timeout(self.config.timeout_ms)
            .send();
        
        match response {
            Ok(resp) if resp.is_success() => {
                if let Ok(body) = resp.text() {
                    self.parse_calibration(&body)?;
                }
                Ok(())
            }
            Ok(resp) => Err(BackendError::CommunicationError(
                format!("Calibration fetch failed: HTTP {}", resp.status.0)
            )),
            Err(e) => Err(BackendError::CommunicationError(format!("{:?}", e))),
        }
    }
    
    /// Parse calibration data from JSON
    fn parse_calibration(&mut self, json: &str) -> Result<(), BackendError> {
        // Extract T1, T2, gate errors from calibration JSON
        // This is provider-specific parsing
        
        let mut error_rates = ErrorRates::default();
        
        // Simple parsing for T1/T2 (IBM format)
        if let Some(start) = json.find("\"T1\"") {
            if let Some(colon) = json[start..].find(':') {
                let num_start = start + colon + 1;
                let num_str: String = json[num_start..]
                    .chars()
                    .skip_while(|c| c.is_whitespace() || *c == '[')
                    .take_while(|c| c.is_numeric() || *c == '.' || *c == 'e' || *c == '-')
                    .collect();
                if let Ok(t1) = num_str.parse::<f64>() {
                    error_rates.t1_times.push(t1);
                }
            }
        }
        
        self.capabilities.error_rates = Some(error_rates);
        Ok(())
    }
    
    /// Submit job to remote QPU
    fn submit_remote(&self, circuit: &Circuit, shots: u32) -> Result<String, BackendError> {
        use crate::http::Request;
        
        // Convert circuit to QASM
        let qasm = self.circuit_to_qasm(circuit);
        
        // Build submission JSON based on provider
        let (url, body) = match self.config.provider.as_str() {
            "ibm" => {
                let url = format!("{}/jobs", self.config.endpoint);
                let body = format!(
                    r#"{{"backend":"{}","shots":{},"qasm":"{}"}}"#,
                    self.config.backend_name,
                    shots,
                    qasm.replace('"', "\\\"").replace('\n', "\\n")
                );
                (url, body)
            }
            "ionq" => {
                let url = format!("{}/jobs", self.config.endpoint);
                let body = format!(
                    r#"{{"target":"{}","shots":{},"input":{{"format":"qasm","data":"{}"}}}}"#,
                    self.config.backend_name,
                    shots,
                    qasm.replace('"', "\\\"").replace('\n', "\\n")
                );
                (url, body)
            }
            "google" => {
                let url = format!("{}/projects/-/programs", self.config.endpoint);
                let body = format!(
                    r#"{{"code":{{"qasm":"{}"}}}}"#,
                    qasm.replace('"', "\\\"").replace('\n', "\\n")
                );
                (url, body)
            }
            _ => return Err(BackendError::ConfigError(String::from("Unknown provider"))),
        };
        
        let response = Request::post(&url)
            .map_err(|_| BackendError::CommunicationError(String::from("Invalid URL")))?
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .timeout(self.config.timeout_ms)
            .send();
        
        match response {
            Ok(resp) if resp.is_success() => {
                // Extract job ID from response
                if let Ok(body) = resp.text() {
                    self.extract_job_id(&body)
                } else {
                    Err(BackendError::CommunicationError(String::from("Invalid response")))
                }
            }
            Ok(resp) => Err(BackendError::CommunicationError(
                format!("Job submission failed: HTTP {}", resp.status.0)
            )),
            Err(e) => Err(BackendError::CommunicationError(format!("{:?}", e))),
        }
    }
    
    /// Extract job ID from JSON response
    fn extract_job_id(&self, json: &str) -> Result<String, BackendError> {
        // Look for "id" or "job_id" field
        for key in &["\"id\"", "\"job_id\"", "\"jobId\""] {
            if let Some(start) = json.find(key) {
                let after_key = &json[start + key.len()..];
                if let Some(colon) = after_key.find(':') {
                    let value_part = &after_key[colon + 1..];
                    // Skip whitespace and find string value
                    let trimmed = value_part.trim_start();
                    if trimmed.starts_with('"') {
                        let id: String = trimmed[1..]
                            .chars()
                            .take_while(|c| *c != '"')
                            .collect();
                        return Ok(id);
                    }
                }
            }
        }
        Err(BackendError::CommunicationError(String::from("No job ID in response")))
    }
    
    /// Convert circuit to OpenQASM 2.0
    fn circuit_to_qasm(&self, circuit: &Circuit) -> String {
        use super::circuit::Gate;
        
        let mut qasm = String::from("OPENQASM 2.0;\ninclude \"qelib1.inc\";\n");
        qasm.push_str(&format!("qreg q[{}];\n", circuit.n_qubits));
        qasm.push_str(&format!("creg c[{}];\n", circuit.n_cbits));
        
        for gate in &circuit.gates {
            match gate {
                Gate::H(q) => qasm.push_str(&format!("h q[{}];\n", q)),
                Gate::X(q) => qasm.push_str(&format!("x q[{}];\n", q)),
                Gate::Y(q) => qasm.push_str(&format!("y q[{}];\n", q)),
                Gate::Z(q) => qasm.push_str(&format!("z q[{}];\n", q)),
                Gate::S(q) => qasm.push_str(&format!("s q[{}];\n", q)),
                Gate::Sdg(q) => qasm.push_str(&format!("sdg q[{}];\n", q)),
                Gate::T(q) => qasm.push_str(&format!("t q[{}];\n", q)),
                Gate::Tdg(q) => qasm.push_str(&format!("tdg q[{}];\n", q)),
                Gate::Rx(q, theta) => qasm.push_str(&format!("rx({}) q[{}];\n", theta, q)),
                Gate::Ry(q, theta) => qasm.push_str(&format!("ry({}) q[{}];\n", theta, q)),
                Gate::Rz(q, phi) => qasm.push_str(&format!("rz({}) q[{}];\n", phi, q)),
                Gate::Cx(c, t) => qasm.push_str(&format!("cx q[{}], q[{}];\n", c, t)),
                Gate::Cz(c, t) => qasm.push_str(&format!("cz q[{}], q[{}];\n", c, t)),
                Gate::Swap(a, b) => qasm.push_str(&format!("swap q[{}], q[{}];\n", a, b)),
                Gate::Ccx(c1, c2, t) => qasm.push_str(&format!("ccx q[{}], q[{}], q[{}];\n", c1, c2, t)),
                Gate::Measure(q, c) => qasm.push_str(&format!("measure q[{}] -> c[{}];\n", q, c)),
                Gate::Barrier(qubits) => {
                    if qubits.is_empty() {
                        qasm.push_str("barrier q;\n");
                    } else {
                        let qs: Vec<String> = qubits.iter().map(|q| format!("q[{}]", q)).collect();
                        qasm.push_str(&format!("barrier {};\n", qs.join(", ")));
                    }
                }
                Gate::Reset(q) => qasm.push_str(&format!("reset q[{}];\n", q)),
                Gate::Id(q) => qasm.push_str(&format!("id q[{}];\n", q)),
                Gate::U3(q, theta, phi, lambda) => {
                    qasm.push_str(&format!("u3({}, {}, {}) q[{}];\n", theta, phi, lambda, q));
                }
            }
        }
        
        qasm
    }
    
    /// Poll job status from remote API
    fn poll_status(&self, remote_job_id: &str) -> Result<(JobState, Option<BackendResult>), BackendError> {
        use crate::http::Request;
        
        let url = match self.config.provider.as_str() {
            "ibm" => format!("{}/jobs/{}", self.config.endpoint, remote_job_id),
            "ionq" => format!("{}/jobs/{}", self.config.endpoint, remote_job_id),
            "google" => format!("{}/projects/-/programs/-/jobs/{}", self.config.endpoint, remote_job_id),
            _ => return Err(BackendError::ConfigError(String::from("Unknown provider"))),
        };
        
        let response = Request::get(&url)
            .map_err(|_| BackendError::CommunicationError(String::from("Invalid URL")))?
            .bearer_auth(&self.config.api_key)
            .timeout(self.config.timeout_ms)
            .send();
        
        match response {
            Ok(resp) if resp.is_success() => {
                if let Ok(body) = resp.text() {
                    self.parse_job_status(&body)
                } else {
                    Err(BackendError::CommunicationError(String::from("Invalid response")))
                }
            }
            Ok(resp) => Err(BackendError::CommunicationError(
                format!("Status query failed: HTTP {}", resp.status.0)
            )),
            Err(e) => Err(BackendError::CommunicationError(format!("{:?}", e))),
        }
    }
    
    /// Parse job status from JSON
    fn parse_job_status(&self, json: &str) -> Result<(JobState, Option<BackendResult>), BackendError> {
        // Look for "status" field
        let status_str = if let Some(start) = json.find("\"status\"") {
            let after_key = &json[start + 8..];
            if let Some(colon) = after_key.find(':') {
                let value_part = &after_key[colon + 1..].trim_start();
                if value_part.starts_with('"') {
                    value_part[1..].chars().take_while(|c| *c != '"').collect::<String>()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        
        let state = match status_str.to_lowercase().as_str() {
            "queued" | "pending" | "created" => JobState::Pending,
            "running" | "executing" => JobState::Running,
            "completed" | "done" | "finished" => JobState::Succeeded,
            "failed" | "error" => JobState::Failed,
            "cancelled" | "canceled" => JobState::Cancelled,
            _ => JobState::Pending,
        };
        
        // If completed, extract results
        let result = if state == JobState::Succeeded {
            self.extract_results(json).ok()
        } else {
            None
        };
        
        Ok((state, result))
    }
    
    /// Extract measurement results from JSON
    fn extract_results(&self, json: &str) -> Result<BackendResult, BackendError> {
        // Look for "counts" or "results" field
        let mut counts = BTreeMap::new();
        
        // Simple extraction - real implementation needs proper JSON parsing
        if let Some(start) = json.find("\"counts\"") {
            // Extract counts object
            if let Some(brace_start) = json[start..].find('{') {
                let counts_start = start + brace_start + 1;
                if let Some(brace_end) = json[counts_start..].find('}') {
                    let counts_str = &json[counts_start..counts_start + brace_end];
                    // Parse "0x0": 500, "0x3": 500 format
                    for pair in counts_str.split(',') {
                        let parts: Vec<&str> = pair.split(':').collect();
                        if parts.len() == 2 {
                            let key = parts[0].trim().trim_matches('"');
                            if let Ok(count) = parts[1].trim().parse::<u64>() {
                                counts.insert(String::from(key), count);
                            }
                        }
                    }
                }
            }
        }
        
        Ok(BackendResult {
            job_id: 0, // Will be set by caller
            counts,
            shots: 0, // Unknown from result
            execution_time_us: 0,
            metadata: None,
        })
    }
}

impl QuantumBackend for RemoteQpuBackend {
    fn name(&self) -> &str {
        &self.config.backend_name
    }
    
    fn capabilities(&self) -> &BackendCapabilities {
        &self.capabilities
    }
    
    fn status(&self) -> BackendStatus {
        self.status.clone()
    }
    
    fn submit(&self, job: JobSubmission) -> Result<u64, BackendError> {
        if self.status != BackendStatus::Available {
            return Err(BackendError::NotAvailable);
        }
        
        // Submit to remote API
        let _remote_id = self.submit_remote(&job.circuit, job.shots)?;
        
        // Generate local job ID
        let local_id = self.next_job_id.fetch_add(1, Ordering::SeqCst);
        
        // Note: In production, store mapping of local_id -> remote_id
        // for status polling
        
        Ok(local_id)
    }
    
    fn job_status(&self, _job_id: u64) -> Option<JobState> {
        // TODO: Query job status from remote API
        None
    }
    
    fn get_result(&self, _job_id: u64) -> Option<BackendResult> {
        // TODO: Fetch result from remote API
        None
    }
    
    fn cancel(&self, _job_id: u64) -> bool {
        // TODO: Send cancel request to remote API
        false
    }
}

// ============================================================================
// Backend Manager
// ============================================================================

/// Manages multiple quantum backends
pub struct BackendManager {
    /// Available backends
    backends: Vec<Box<dyn QuantumBackend>>,
    /// Default backend index
    default_backend: usize,
}

impl BackendManager {
    /// Create a new backend manager with local simulator
    pub fn new() -> Self {
        let local = Box::new(LocalSimulatorBackend::new(32));
        Self {
            backends: vec![local],
            default_backend: 0,
        }
    }
    
    /// Add a backend
    pub fn add_backend(&mut self, backend: Box<dyn QuantumBackend>) {
        self.backends.push(backend);
    }
    
    /// Get default backend
    pub fn default_backend(&self) -> Option<&dyn QuantumBackend> {
        self.backends.get(self.default_backend).map(|b| b.as_ref())
    }
    
    /// Set default backend by index
    pub fn set_default(&mut self, index: usize) -> bool {
        if index < self.backends.len() {
            self.default_backend = index;
            true
        } else {
            false
        }
    }
    
    /// List all backends
    pub fn list_backends(&self) -> Vec<(&str, BackendStatus)> {
        self.backends
            .iter()
            .map(|b| (b.name(), b.status()))
            .collect()
    }
    
    /// Get backend by name
    pub fn get_backend(&self, name: &str) -> Option<&dyn QuantumBackend> {
        self.backends
            .iter()
            .find(|b| b.name() == name)
            .map(|b| b.as_ref())
    }
    
    /// Find best available backend for a circuit
    pub fn find_backend_for_circuit(&self, circuit: &Circuit) -> Option<&dyn QuantumBackend> {
        // Find first available backend that can run this circuit
        for backend in &self.backends {
            if backend.status() == BackendStatus::Available {
                if backend.validate_circuit(circuit).is_ok() {
                    return Some(backend.as_ref());
                }
            }
        }
        None
    }
}

impl Default for BackendManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Circuit Transpilation (for hardware backends)
// ============================================================================

/// Transpiler for converting circuits to native gate sets
pub struct CircuitTranspiler {
    target_gates: Vec<GateType>,
    connectivity: Option<ConnectivityMap>,
    optimization_level: u8,
}

impl CircuitTranspiler {
    pub fn new(target_gates: Vec<GateType>) -> Self {
        Self {
            target_gates,
            connectivity: None,
            optimization_level: 1,
        }
    }
    
    pub fn with_connectivity(mut self, connectivity: ConnectivityMap) -> Self {
        self.connectivity = Some(connectivity);
        self
    }
    
    pub fn with_optimization(mut self, level: u8) -> Self {
        self.optimization_level = level.min(3);
        self
    }
    
    /// Transpile a circuit to the target backend
    pub fn transpile(&self, circuit: &Circuit) -> Result<Circuit, BackendError> {
        let mut result = circuit.clone();
        
        // Step 1: Decompose unsupported gates to native gates
        // (For now, we assume the circuit uses native gates)
        
        // Step 2: Route qubits to satisfy connectivity constraints
        if let Some(ref connectivity) = self.connectivity {
            result = self.route_qubits(result, connectivity)?;
        }
        
        // Step 3: Optimize (if requested)
        if self.optimization_level > 0 {
            result = self.optimize(result);
        }
        
        Ok(result)
    }
    
    fn route_qubits(&self, circuit: Circuit, _connectivity: &ConnectivityMap) -> Result<Circuit, BackendError> {
        // TODO: Implement qubit routing (SWAP insertion)
        // This is a complex problem - for now, just return the original
        Ok(circuit)
    }
    
    fn optimize(&self, circuit: Circuit) -> Circuit {
        // TODO: Implement circuit optimization
        // - Cancel adjacent inverse gates (H-H, X-X, etc.)
        // - Commute gates to reduce depth
        // - Merge rotation gates
        circuit
    }
}

// ============================================================================
// Error Mitigation
// ============================================================================

/// Error mitigation strategies
#[derive(Clone, Debug)]
pub enum MitigationStrategy {
    /// No mitigation
    None,
    /// Readout error mitigation using calibration matrix
    ReadoutMitigation,
    /// Zero noise extrapolation
    ZeroNoiseExtrapolation,
    /// Probabilistic error cancellation
    ProbabilisticErrorCancellation,
}

/// Error mitigator
pub struct ErrorMitigator {
    strategy: MitigationStrategy,
    calibration: Option<ErrorRates>,
}

impl ErrorMitigator {
    pub fn new(strategy: MitigationStrategy) -> Self {
        Self {
            strategy,
            calibration: None,
        }
    }
    
    pub fn with_calibration(mut self, calibration: ErrorRates) -> Self {
        self.calibration = Some(calibration);
        self
    }
    
    /// Apply error mitigation to measurement results
    pub fn mitigate(&self, counts: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
        match self.strategy {
            MitigationStrategy::None => counts.clone(),
            MitigationStrategy::ReadoutMitigation => {
                self.apply_readout_mitigation(counts)
            }
            _ => counts.clone(), // Other strategies not yet implemented
        }
    }
    
    fn apply_readout_mitigation(&self, counts: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
        // TODO: Implement readout error mitigation
        // This requires the calibration matrix and solving a linear system
        counts.clone()
    }
}

// ============================================================================
// Provider Configurations (for future use)
// ============================================================================

/// IBM Quantum configuration
pub mod ibm {
    use super::*;
    
    pub const API_BASE: &str = "https://api.quantum-computing.ibm.com";
    
    pub fn create_config(api_key: &str, backend: &str) -> RemoteQpuConfig {
        RemoteQpuConfig {
            provider: String::from("ibm"),
            endpoint: String::from(API_BASE),
            api_key: String::from(api_key),
            backend_name: String::from(backend),
            timeout_ms: 30000,
        }
    }
}

/// Google Quantum (Cirq) configuration
pub mod google {
    use super::*;
    
    pub const API_BASE: &str = "https://quantum.googleapis.com";
    
    pub fn create_config(api_key: &str, processor: &str) -> RemoteQpuConfig {
        RemoteQpuConfig {
            provider: String::from("google"),
            endpoint: String::from(API_BASE),
            api_key: String::from(api_key),
            backend_name: String::from(processor),
            timeout_ms: 30000,
        }
    }
}

/// IonQ configuration
pub mod ionq {
    use super::*;
    
    pub const API_BASE: &str = "https://api.ionq.co/v0.3";
    
    pub fn create_config(api_key: &str) -> RemoteQpuConfig {
        RemoteQpuConfig {
            provider: String::from("ionq"),
            endpoint: String::from(API_BASE),
            api_key: String::from(api_key),
            backend_name: String::from("ionq_qpu"),
            timeout_ms: 60000,
        }
    }
}

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
use pyo3::types::PyDict;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PyRunOut {
    pub counts: BTreeMap<String, u64>,
    pub runtime_ms: u64,
    pub meta: String,
}

/// python/qossim/bridge.py içindeki run_qasm fonksiyonunu çağırır.
#[cfg(feature = "python")]
pub fn run_qasm_inproc(qasm: &str, shots: u32) -> Result<PyRunOut, String> {
    Python::with_gil(|py| {
        let m = py.import("python.qossim.bridge").map_err(|e| {
            e.print(py);
            format!("import error: {e}")
        })?;

        let func = m.getattr("run_qasm").map_err(|e| {
            e.print(py);
            format!("missing run_qasm: {e}")
        })?;

        let out = func.call1((qasm, shots)).map_err(|e| {
            e.print(py); // <<< Python traceback burada görünecek
            format!("call error: {e}")
        })?;

        // Artık dict bekliyoruz: {"counts": {...}, "runtime_ms": 12, "meta": "..."}
        let d: &PyDict = out.downcast::<PyDict>().map_err(|e| {
            format!("extract error: run_qasm did not return dict: {e}")
        })?;

        let counts_obj = d
        .get_item("counts")
        .map_err(|e| format!("extract error: get_item(counts): {e}"))?
        .ok_or_else(|| "extract error: missing key 'counts'".to_string())?;

    let runtime_obj = d
        .get_item("runtime_ms")
        .map_err(|e| format!("extract error: get_item(runtime_ms): {e}"))?
        .ok_or_else(|| "extract error: missing key 'runtime_ms'".to_string())?;

    let meta_obj = d
        .get_item("meta")
        .map_err(|e| format!("extract error: get_item(meta): {e}"))?
        .ok_or_else(|| "extract error: missing key 'meta'".to_string())?;

        let counts_py: std::collections::HashMap<String, u64> = counts_obj.extract().map_err(|e| {
            format!("extract error: counts: {e}")
        })?;
        let runtime_ms: u64 = runtime_obj.extract().map_err(|e| {
            format!("extract error: runtime_ms: {e}")
        })?;
        let meta: String = meta_obj.extract().map_err(|e| {
            format!("extract error: meta: {e}")
        })?;

        let mut counts = BTreeMap::new();
        for (k, v) in counts_py {
            counts.insert(k, v);
        }

        Ok(PyRunOut {
            counts,
            runtime_ms,
            meta,
        })
    })
}

/// Stub backend (pure Rust) so the project runs without Python.
///
/// This is intentionally minimal: it only recognizes a few common patterns and produces
/// deterministic counts for UI/demo purposes.
#[cfg(not(feature = "python"))]
pub fn run_qasm_inproc(qasm: &str, shots: u32) -> Result<PyRunOut, String> {
    let mut counts = BTreeMap::new();

    // Very small heuristic: if it looks like a Bell circuit, return ~50/50 00 and 11.
    let q = qasm.to_ascii_lowercase();
    let is_bell = q.contains("openqasm") && q.contains("h") && q.contains("cx") && q.contains("measure");

    if is_bell {
        let a = (shots as u64) / 2;
        let b = (shots as u64) - a;
        counts.insert("00".to_string(), a);
        counts.insert("11".to_string(), b);
    } else {
        counts.insert("0".repeat(2), shots as u64);
    }

    Ok(PyRunOut {
        counts,
        runtime_ms: 0,
        meta: "stub-sim-v0".to_string(),
    })
}

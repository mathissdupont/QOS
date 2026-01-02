from __future__ import annotations

from dataclasses import dataclass
import time
from typing import Any, Dict, Tuple, Optional

# Qiskit imports
import qiskit
import qiskit.qasm2
import qiskit.qasm3
from qiskit import transpile
from qiskit_aer import AerSimulator

BRIDGE_VERSION = "7.1-aer-meta-test-1"


@dataclass
class RunResult:
    counts: Dict[str, int]
    runtime_ms: int
    meta: str


def _detect_qasm_version(qasm: str) -> str:
    # very simple header detection
    head = (qasm or "").lstrip().upper()
    if head.startswith("OPENQASM 3"):
        return "3"
    if head.startswith("OPENQASM 2"):
        return "2"
    # fallback: if it contains "OPENQASM 3" anywhere early
    if "OPENQASM 3" in head[:100]:
        return "3"
    if "OPENQASM 2" in head[:100]:
        return "2"
    return "unknown"


def _parse_qasm_to_circuit(qasm: str, qasm_ver: str):
    if qasm_ver == "3":
        return qiskit.qasm3.loads(qasm)

    if qasm_ver == "2":
        q = (qasm or "").strip()

        # include var mı?
        q_lower = q.lower().replace("'", '"')
        has_include = 'include "qelib1.inc"' in q_lower

        # OPENQASM 2.x header'ını yakala (tek satır gelse bile)
        # include yoksa header'dan hemen sonra enjekte et
        if not has_include:
            # En yaygın: "OPENQASM 2.0;" veya "OPENQASM 2;"
            if "OPENQASM 2.0;" in q.upper():
                q = q.replace("OPENQASM 2.0;", 'OPENQASM 2.0;\ninclude "qelib1.inc";\n', 1)
            elif "OPENQASM 2;" in q.upper():
                q = q.replace("OPENQASM 2;", 'OPENQASM 2;\ninclude "qelib1.inc";\n', 1)
            else:
                # son çare: başa ekle
                q = 'OPENQASM 2.0;\ninclude "qelib1.inc";\n' + q

        return qiskit.qasm2.loads(q)

    # unknown: best-effort
    try:
        return qiskit.qasm3.loads(qasm)
    except Exception:
        return qiskit.qasm2.loads(qasm)


def _ensure_measurements(circuit) -> Tuple[Any, bool]:
    """
    Ensure circuit has measurements so we can return counts.
    Returns: (circuit, did_add_measurements)
    """
    # Heuristic: if no classical bits or no measure instructions -> measure_all
    did_add = False
    has_measure = any(inst.operation.name == "measure" for inst in circuit.data)
    if (circuit.num_clbits == 0) or (not has_measure):
        circuit = circuit.copy()
        circuit.measure_all()
        did_add = True
    return circuit, did_add


def run_qasm(qasm: str, shots: int = 1024):
    t0 = time.perf_counter()

    try:
        qasm_ver = _detect_qasm_version(qasm)
        circuit = _parse_qasm_to_circuit(qasm, qasm_ver)
        circuit, did_add_meas = _ensure_measurements(circuit)

        backend = AerSimulator()
        tcirc = transpile(circuit, backend=backend, optimization_level=1)

        job = backend.run(tcirc, shots=int(shots))
        result = job.result()
        counts = result.get_counts()

        counts_out = {str(k): int(v) for k, v in counts.items()}

        runtime_ms = int((time.perf_counter() - t0) * 1000)

        meta = "; ".join([
            "py-inproc-v0",
            "aer",
            "bridge=7.2",
            f"backend={backend.name}",
            f"qasm={qasm_ver}",
            f"shots={shots}",
            f"runtime_ms={runtime_ms}",
        ])

        return {
            "counts": counts_out,
            "runtime_ms": runtime_ms,   # <<< ZORUNLU
            "meta": meta,
        }

    except Exception as e:
        runtime_ms = int((time.perf_counter() - t0) * 1000)

        return {
            "counts": {},               # <<< ZORUNLU
            "runtime_ms": runtime_ms,   # <<< ZORUNLU (ŞU AN EKSİK OLAN BU)
            "meta": f"py-inproc-v0; error; {type(e).__name__}: {e}",
        }


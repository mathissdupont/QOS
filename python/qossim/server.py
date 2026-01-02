from fastapi import FastAPI
from pydantic import BaseModel
from typing import Dict
import time

app = FastAPI()

class RunReq(BaseModel):
    qasm: str
    shots: int

class RunResp(BaseModel):
    counts: Dict[str, int]
    runtime_ms: int
    meta: str

@app.post("/run", response_model=RunResp)
def run(req: RunReq):
    t0 = time.time()

    # v0.1: gerçek Aer yerine şimdilik deterministik fake (hemen çalışsın diye)
    # Step 6B'de burayı Qiskit Aer'a bağlayacağız.
    # Basit: Bell gibi davranalım
    counts = {"00": req.shots // 2, "11": req.shots - (req.shots // 2)}

    rt = int((time.time() - t0) * 1000)
    return RunResp(counts=counts, runtime_ms=rt, meta="py-sim-v0")

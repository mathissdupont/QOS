use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use qos_abi::{
    JobHandle, JobResult as AbiJobResult, JobState as AbiJobState, IrFormat as AbiIrFormat,
    QosRequest, QosResponse, ResultStatus as AbiResultStatus,
};
use qos_core::{IrFormat, JobManager, JobState, QProc};
use qos_core::{QResult, ResultStatus};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    jm: Arc<JobManager>,
    next_handle: Arc<AtomicU64>,
    handle_to_uuid: Arc<Mutex<HashMap<JobHandle, Uuid>>>,
    uuid_to_handle: Arc<Mutex<HashMap<Uuid, JobHandle>>>,
}

impl AppState {
    fn uuid_for_handle(&self, h: JobHandle) -> Option<Uuid> {
        self.handle_to_uuid.lock().ok()?.get(&h).copied()
    }

    fn handle_for_uuid(&self, id: Uuid) -> Option<JobHandle> {
        self.uuid_to_handle.lock().ok()?.get(&id).copied()
    }

    fn get_or_create_handle_for_uuid(&self, id: Uuid) -> JobHandle {
        if let Some(h) = self.handle_for_uuid(id) {
            return h;
        }
        let raw = self.next_handle.fetch_add(1, Ordering::Relaxed);
        let h = JobHandle(raw);
        {
            let mut a = self.handle_to_uuid.lock().unwrap();
            a.insert(h, id);
        }
        {
            let mut b = self.uuid_to_handle.lock().unwrap();
            b.insert(id, h);
        }
        h
    }
}

fn map_ir_format_from_abi(f: AbiIrFormat) -> IrFormat {
    match f {
        AbiIrFormat::OpenQasm2 => IrFormat::OpenQasm2,
        AbiIrFormat::OpenQasm3 => IrFormat::OpenQasm3,
        AbiIrFormat::JsonIrV1 => IrFormat::JsonIrV1,
    }
}

fn map_state_to_abi(s: JobState) -> AbiJobState {
    match s {
        JobState::Queued => AbiJobState::Queued,
        JobState::Running => AbiJobState::Running,
        JobState::Done => AbiJobState::Done,
        JobState::Failed => AbiJobState::Failed,
        JobState::Cancelled => AbiJobState::Cancelled,
    }
}

fn map_result_to_abi(r: QResult) -> AbiJobResult {
    let counts_json = serde_json::to_string(&r.counts).unwrap_or_else(|_| "{}".to_string());
    let status = match r.status {
        ResultStatus::Ok => AbiResultStatus::Ok,
        ResultStatus::Error => AbiResultStatus::Error,
    };
    AbiJobResult {
        status,
        counts_json,
        meta: r.meta,
        error: r.error,
    }
}

fn map_result_from_abi(r: AbiJobResult) -> QResult {
    let counts = serde_json::from_str(&r.counts_json).unwrap_or_default();
    let status = match r.status {
        AbiResultStatus::Ok => ResultStatus::Ok,
        AbiResultStatus::Error => ResultStatus::Error,
    };
    QResult {
        status,
        counts,
        meta: r.meta,
        error: r.error,
    }
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang=\"en\">
    <head>
        <meta charset=\"utf-8\" />
        <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />
        <title>QOS UI</title>
        <style>
            body { font-family: system-ui, sans-serif; margin: 16px; }
            textarea { width: 100%; min-height: 140px; }
            input, select, button, textarea { font: inherit; }
            .row { display: flex; gap: 12px; flex-wrap: wrap; }
            .row > div { flex: 1 1 220px; }
            pre { white-space: pre-wrap; }
            .box { border: 1px solid currentColor; padding: 12px; margin: 12px 0; }
        </style>
    </head>
    <body>
        <h1>QOS (Hosted UI)</h1>
        <p>Submit a circuit (QASM/IR), then watch status/result update.</p>

        <div class=\"box\">
            <h2>Submit</h2>
            <div class=\"row\">
                <div>
                    <label>Name<br/><input id=\"name\" value=\"bell\" /></label>
                </div>
                <div>
                    <label>IR Format<br/>
                        <select id=\"ir_format\">
                            <option>OpenQasm3</option>
                            <option>OpenQasm2</option>
                            <option>JsonIrV1</option>
                        </select>
                    </label>
                </div>
                <div>
                    <label>Qubits<br/><input id=\"n_qubits\" type=\"number\" value=\"2\" min=\"1\" /></label>
                </div>
                <div>
                    <label>Shots<br/><input id=\"shots\" type=\"number\" value=\"1024\" min=\"1\" /></label>
                </div>
            </div>
            <p><label>IR / QASM<br/>
                <textarea id=\"ir\">OPENQASM 3;
include \"stdgates.inc\";
qubit[2] q;
bit[2] c;
h q[0];
cx q[0], q[1];
measure q -> c;
</textarea>
            </label></p>
            <button id=\"submit\">Submit</button>
            <button id=\"dispatch\" title=\"Only needed in manual-dispatch mode\">Dispatch next</button>
            <pre id=\"submit_out\"></pre>
        </div>

        <div class=\"box\">
            <h2>Jobs</h2>
            <button id=\"refresh\">Refresh list</button>
            <ul id=\"jobs\"></ul>
        </div>

        <div class=\"box\">
            <h2>Selected Job</h2>
            <p>
                <label>Job ID<br/><input id=\"job_id\" size=\"40\" placeholder=\"uuid...\" /></label>
                <button id=\"poll\">Poll now</button>
            </p>
            <pre id=\"job_out\"></pre>
        </div>

        <script>
            const $ = (id) => document.getElementById(id);

            async function api(path, opts) {
                const res = await fetch(path, opts);
                const text = await res.text();
                let json;
                try { json = text ? JSON.parse(text) : null; } catch { json = null; }
                if (!res.ok) {
                    const msg = json?.message || json?.error || text || (res.status + " " + res.statusText);
                    throw new Error(msg);
                }
                return json;
            }

            async function refreshJobs() {
                const list = await api('/jobs');
                const ul = $('jobs');
                ul.innerHTML = '';
                for (const j of list) {
                    const li = document.createElement('li');
                    const a = document.createElement('a');
                    a.href = '#';
                    a.textContent = j.job_id + '  (' + j.state + ')';
                    a.onclick = (e) => {
                        e.preventDefault();
                        $('job_id').value = j.job_id;
                        pollJob();
                    };
                    li.appendChild(a);
                    ul.appendChild(li);
                }
            }

            async function pollJob() {
                const id = $('job_id').value.trim();
                if (!id) return;
                let out = '';
                try {
                    const st = await api('/jobs/' + id);
                    out += 'status: ' + JSON.stringify(st, null, 2) + '\n\n';
                } catch (e) {
                    out += 'status error: ' + e.message + '\n\n';
                }
                try {
                    const r = await api('/jobs/' + id + '/result');
                    out += 'result: ' + JSON.stringify(r, null, 2) + '\n';
                } catch (e) {
                    out += 'result error: ' + e.message + '\n';
                }
                $('job_out').textContent = out;
            }

            $('submit').onclick = async () => {
                $('submit_out').textContent = '';
                const body = {
                    name: $('name').value,
                    ir_format: $('ir_format').value,
                    ir: $('ir').value,
                    n_qubits: Number($('n_qubits').value),
                    shots: Number($('shots').value),
                };
                try {
                    const resp = await api('/jobs', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify(body),
                    });
                    $('submit_out').textContent = JSON.stringify(resp, null, 2);
                    $('job_id').value = resp.job_id;
                    await refreshJobs();
                    await pollJob();
                } catch (e) {
                    $('submit_out').textContent = 'submit error: ' + e.message;
                }
            };

            $('dispatch').onclick = async () => {
                $('submit_out').textContent = '';
                try {
                    const resp = await api('/jobs/dispatch', { method: 'POST' });
                    $('submit_out').textContent = JSON.stringify(resp, null, 2);
                    await refreshJobs();
                } catch (e) {
                    $('submit_out').textContent = 'dispatch error: ' + e.message;
                }
            };

            $('refresh').onclick = refreshJobs;
            $('poll').onclick = pollJob;

            refreshJobs().catch(() => {});
            setInterval(() => pollJob().catch(() => {}), 1000);
        </script>
    </body>
</html>
"#;

// -------- Requests / Responses --------

#[derive(Debug, Deserialize)]
struct SubmitReq {
    name: String,
    ir_format: String, // "OpenQasm2" / "OpenQasm3" / "JsonIrV1"
    ir: String,
    n_qubits: u32,
    shots: u32,
}

#[derive(Debug, Serialize)]
struct SubmitResp {
    job_id: Uuid,
    state: JobState,
}

#[derive(Debug, Serialize)]
struct StatusResp {
    job_id: Uuid,
    state: JobState,
}

#[derive(Debug, Serialize)]
struct DispatchResp {
    dispatched: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct JobListItem {
    job_id: Uuid,
    state: JobState,
}

#[derive(Debug, Deserialize)]
struct FinishOkReq {
    result: QResult,
}

// -------- Job runner (Python in-proc) --------

fn run_job_blocking(jm: Arc<JobManager>, job_id: Uuid) -> Result<(), String> {
    let job = jm.get_job(job_id).map_err(|e| e.to_string())?;
    let qasm =
        String::from_utf8(job.proc.ir_bytes).map_err(|_| "ir_bytes is not valid UTF-8".to_string())?;

    let out = qos_pybridge::run_qasm_inproc(&qasm, job.proc.shots)?;
    let is_err = out.meta.contains("; error");
    let result = if is_err {
        QResult {
            status: ResultStatus::Error,
            counts: Default::default(),
            meta: out.meta,
            error: Some("python: parse/run error".to_string()),
        }
    } else {
        QResult {
            status: ResultStatus::Ok,
            counts: out.counts,
            meta: out.meta,
            error: None,
        }
    };

    jm.finish_ok(job_id, result).map_err(|e| e.to_string())?;
    Ok(())
}

// -------- Handlers --------

async fn submit(
    State(st): State<AppState>,
    Json(req): Json<SubmitReq>,
) -> Result<Json<SubmitResp>, (StatusCode, String)> {
    let ir_format = match req.ir_format.as_str() {
        "OpenQasm2" => IrFormat::OpenQasm2,
        "OpenQasm3" => IrFormat::OpenQasm3,
        "JsonIrV1" => IrFormat::JsonIrV1,
        _ => return Err((StatusCode::BAD_REQUEST, "invalid ir_format".into())),
    };

    let proc = QProc {
        name: req.name,
        ir_format,
        ir_bytes: req.ir.into_bytes(),
        n_qubits: req.n_qubits,
        shots: req.shots,
    };

    let id = st.jm.submit(proc);
    let state = st
        .jm
        .status(id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(SubmitResp { job_id: id, state }))
}

async fn list_jobs(State(st): State<AppState>) -> Json<Vec<JobListItem>> {
    let list = st
        .jm
        .list()
        .into_iter()
        .map(|(job_id, state)| JobListItem { job_id, state })
        .collect();
    Json(list)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn abi_rpc(State(st): State<AppState>, Json(req): Json<QosRequest>) -> Json<QosResponse> {
    let resp = match req {
        QosRequest::Submit { proc } => {
            let core_proc = QProc {
                name: proc.name,
                ir_format: map_ir_format_from_abi(proc.ir_format),
                ir_bytes: proc.ir_bytes,
                n_qubits: proc.n_qubits,
                shots: proc.shots,
            };
            let id = st.jm.submit(core_proc);
            let h = st.get_or_create_handle_for_uuid(id);
            match st.jm.status(id) {
                Ok(state) => QosResponse::SubmitOk {
                    handle: h,
                    state: map_state_to_abi(state),
                },
                Err(e) => QosResponse::Err {
                    message: e.to_string(),
                },
            }
        }

        QosRequest::Status { handle } => {
            let Some(id) = st.uuid_for_handle(handle) else {
                return Json(QosResponse::Err {
                    message: "unknown handle".to_string(),
                });
            };
            match st.jm.status(id) {
                Ok(state) => QosResponse::StatusOk {
                    handle,
                    state: map_state_to_abi(state),
                },
                Err(e) => QosResponse::Err {
                    message: e.to_string(),
                },
            }
        }

        QosRequest::GetResult { handle } => {
            let Some(id) = st.uuid_for_handle(handle) else {
                return Json(QosResponse::Err {
                    message: "unknown handle".to_string(),
                });
            };
            match st.jm.get_result(id) {
                Ok(r) => QosResponse::ResultOk {
                    handle,
                    result: map_result_to_abi(r),
                },
                Err(e) => QosResponse::Err {
                    message: e.to_string(),
                },
            }
        }

        QosRequest::DispatchNext => match st.jm.dispatch_next() {
            Ok(Some(id)) => {
                let h = st.get_or_create_handle_for_uuid(id);
                QosResponse::DispatchOk {
                    dispatched: Some(h),
                }
            }
            Ok(None) => QosResponse::DispatchOk { dispatched: None },
            Err(e) => QosResponse::Err {
                message: e.to_string(),
            },
        },

        QosRequest::Cancel { handle } => {
            let Some(id) = st.uuid_for_handle(handle) else {
                return Json(QosResponse::Err {
                    message: "unknown handle".to_string(),
                });
            };
            match st.jm.cancel(id) {
                Ok(()) => QosResponse::Ok,
                Err(e) => QosResponse::Err {
                    message: e.to_string(),
                },
            }
        }

        QosRequest::FinishOk { handle, result } => {
            let Some(id) = st.uuid_for_handle(handle) else {
                return Json(QosResponse::Err {
                    message: "unknown handle".to_string(),
                });
            };
            match st.jm.finish_ok(id, map_result_from_abi(result)) {
                Ok(()) => QosResponse::Ok,
                Err(e) => QosResponse::Err {
                    message: e.to_string(),
                },
            }
        }

        QosRequest::FinishErr { handle, error } => {
            let Some(id) = st.uuid_for_handle(handle) else {
                return Json(QosResponse::Err {
                    message: "unknown handle".to_string(),
                });
            };
            match st.jm.finish_err(id, error) {
                Ok(()) => QosResponse::Ok,
                Err(e) => QosResponse::Err {
                    message: e.to_string(),
                },
            }
        }
    };

    Json(resp)
}

async fn status(
    State(st): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<StatusResp>, (StatusCode, String)> {
    let state = st
        .jm
        .status(job_id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(StatusResp { job_id, state }))
}

async fn get_result(
    State(st): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<QResult>, (StatusCode, String)> {
    let res = st
        .jm
        .get_result(job_id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(res))
}

async fn finish_ok(
    State(st): State<AppState>,
    Path(job_id): Path<Uuid>,
    Json(req): Json<FinishOkReq>,
) -> Result<StatusCode, (StatusCode, String)> {
    st.jm
        .finish_ok(job_id, req.result)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// Manuel dispatch endpoint (istersen açık)
async fn dispatch(
    State(st): State<AppState>,
) -> Result<Json<DispatchResp>, (StatusCode, String)> {
    let id = st
        .jm
        .dispatch_next()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let Some(job_id) = id else {
        return Ok(Json(DispatchResp { dispatched: None }));
    };

    let jm = st.jm.clone();
    tokio::spawn(async move {
        let jm2 = jm.clone();
        let job_id2 = job_id;
        let r = tokio::task::spawn_blocking(move || run_job_blocking(jm2, job_id2)).await;

        if let Err(err) = r {
            let _ = jm.finish_err(job_id, format!("join error: {err}"));
            return;
        }
        if let Ok(Err(e)) = r {
            let _ = jm.finish_err(job_id, format!("python: {e}"));
        }
    });

    Ok(Json(DispatchResp { dispatched: Some(job_id) }))
}

// -------- Worker loop (auto-dispatch) --------

async fn worker_loop(jm: Arc<JobManager>, poll_ms: u64) {
    loop {
        let next = jm.dispatch_next();
        match next {
            Ok(Some(job_id)) => {
                let jm2 = jm.clone();
                tokio::spawn(async move {
                    let jm3 = jm2.clone();
                    let job_id2 = job_id;
                    let r = tokio::task::spawn_blocking(move || run_job_blocking(jm3, job_id2)).await;

                    if let Err(err) = r {
                        let _ = jm2.finish_err(job_id, format!("join error: {err}"));
                        return;
                    }
                    if let Ok(Err(e)) = r {
                        let _ = jm2.finish_err(job_id, format!("python: {e}"));
                    }
                });
            }
            Ok(None) => {
                sleep(Duration::from_millis(poll_ms)).await;
            }
            Err(_) => {
                // dispatch_next hata verirse de çok kısa bekle, tight loop olmasın
                sleep(Duration::from_millis(poll_ms)).await;
            }
        }
    }
}

// -------- main --------

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn env_string(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() {
    let workers = env_u64("QOS_WORKERS", 0) as usize;          // 0 => worker yok
    let poll_ms = env_u64("QOS_POLL_MS", 100);
    let manual_dispatch = std::env::var("QOS_MANUAL_DISPATCH")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let jm = JobManager::new_fifo_with_journal("qos.journal.jsonl");
    let st = AppState {
        jm: Arc::new(jm),
        next_handle: Arc::new(AtomicU64::new(1)),
        handle_to_uuid: Arc::new(Mutex::new(HashMap::new())),
        uuid_to_handle: Arc::new(Mutex::new(HashMap::new())),
    };

    // Router önce "eksik state" olarak kurulur (Router<AppState>)
    // with_state(st) ile state verilince Router<()> olur ve serve bunu kabul eder. :contentReference[oaicite:2]{index=2}
    let mut app = Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/abi", post(abi_rpc))
        .route("/jobs", post(submit))
        .route("/jobs", get(list_jobs))
        .route("/jobs/:id", get(status))
        .route("/jobs/:id/result", get(get_result))
        .route("/jobs/:id/finish_ok", post(finish_ok));

    if manual_dispatch {
        app = app.route("/jobs/dispatch", post(dispatch));
    }

    let app = app.with_state(st.clone());

    // Auto worker
    if !manual_dispatch {
        for _ in 0..workers.max(1) {
            let jm_bg = st.jm.clone();
            tokio::spawn(worker_loop(jm_bg, poll_ms));
        }
        println!("auto-dispatch ON | workers={} poll_ms={}", workers.max(1), poll_ms);
    } else {
        println!("manual-dispatch ON | use POST /jobs/dispatch");
    }

    let addr_s = env_string("QOS_ADDR", "127.0.0.1:8080");
    let addr: SocketAddr = addr_s.parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    println!("qosd listening on http://{}", addr);

    axum::serve(listener, app).await.unwrap();
}

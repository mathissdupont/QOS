function Submit-Job($name, $qasm, $nq, $shots) {
  $body = @{
    name      = $name
    ir_format = "OpenQasm2"
    ir        = $qasm
    n_qubits  = $nq
    shots     = $shots
  } | ConvertTo-Json

  Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8080/jobs -Body $body -ContentType "application/json"
}

function Wait-Result($jobId, $timeoutSec = 15) {
  $deadline = (Get-Date).AddSeconds($timeoutSec)
  $res = $null

  while ((Get-Date) -lt $deadline) {
    try {
      $res = Invoke-RestMethod -Method Get -Uri "http://127.0.0.1:8080/jobs/$jobId/result" -ErrorAction Stop
      return $res
    } catch {
      Start-Sleep -Milliseconds 200
    }
  }

  throw "TIMEOUT: result gelmedi (jobId=$jobId)"
}

# ---- TEST 1: Bell ----
$qasm_bell = @"
OPENQASM 2.0;
include "qelib1.inc";
qreg q[2];
creg c[2];
h q[0];
cx q[0],q[1];
measure q->c;
"@

$r1 = Submit-Job "bell" $qasm_bell 2 1000
try { Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8080/jobs/dispatch | Out-Null } catch {}

$job1 = $r1.job_id
$res1 = Wait-Result $job1
"=== TEST1 Bell ==="
$job1
$res1

# ---- TEST 2: GHZ3 ----
$qasm_ghz = @"
OPENQASM 2.0;
include "qelib1.inc";
qreg q[3];
creg c[3];
h q[0];
cx q[0],q[1];
cx q[1],q[2];
measure q->c;
"@

$r2 = Submit-Job "ghz3" $qasm_ghz 3 1000
try { Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8080/jobs/dispatch | Out-Null } catch {}

$job2 = $r2.job_id
$res2 = Wait-Result $job2
"=== TEST2 GHZ3 ==="
$job2
$res2

# ---- TEST 3: Bad QASM ----
$qasm_bad = @"
OPENQASM 2.0;
include "qelib1.inc";
qreg q[1];
this_is_not_a_gate q[0];
"@

$r3 = Submit-Job "bad" $qasm_bad 1 10
try { Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8080/jobs/dispatch | Out-Null } catch {}

$job3 = $r3.job_id
$res3 = Wait-Result $job3
"=== TEST3 Bad QASM ==="
$job3
$res3

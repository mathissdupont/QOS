$ErrorActionPreference = "Stop"

function Submit-Job($name, $ir_format, $ir, $n_qubits, $shots) {
  $body = @{
    name      = $name
    ir_format = $ir_format
    ir        = $ir
    n_qubits  = $n_qubits
    shots     = $shots
  } | ConvertTo-Json

  return Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8080/jobs -Body $body -ContentType "application/json"
}

function Dispatch-Once() {
  Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8080/jobs/dispatch | Out-Null
}

function Get-State($jobId) {
  Invoke-RestMethod -Method Get -Uri ("http://127.0.0.1:8080/jobs/{0}" -f $jobId)
}

function Poll-Result($jobId, $timeoutSec = 15) {
  $deadline = (Get-Date).AddSeconds($timeoutSec)
  while ((Get-Date) -lt $deadline) {
    try {
      return Invoke-RestMethod -Method Get -Uri ("http://127.0.0.1:8080/jobs/{0}/result" -f $jobId)
    } catch {
      Start-Sleep -Milliseconds 200
    }
  }
  throw "Timeout: result gelmedi (jobId=$jobId)"
}
function Print-Result($title, $jobId, $res) {
  Write-Host ""
  Write-Host "==== $title ===="
  Write-Host "jobId: $jobId"

  if ($res.PSObject.Properties.Name -contains "status") {
    Write-Host "status: $($res.status)"
  }

  Write-Host "meta:  $($res.meta)"

  if ($res.PSObject.Properties.Name -contains "error" -and $null -ne $res.error) {
    Write-Host "error: $($res.error)"
  }

  Write-Host "counts:"
  $res.counts
}

# -------------------------
# TEST 1: Bell (OpenQasm2)
# -------------------------
$bellIr = @"
OPENQASM 2.0;
include "qelib1.inc";
qreg q[2];
creg c[2];
h q[0];
cx q[0], q[1];
measure q -> c;
"@

$r1 = Submit-Job "bell-qasm2" "OpenQasm2" $bellIr 2 1000
$job1 = $r1.job_id
Dispatch-Once
$st1 = Get-State $job1
Write-Host "TEST1 state:" $st1.state
$res1 = Poll-Result $job1 20
Print-Result "TEST1 Bell OpenQasm2" $job1 $res1

# -------------------------
# TEST 2: GHZ3 (OpenQasm2)
# -------------------------
$ghzIr = @"
OPENQASM 2.0;
include "qelib1.inc";
qreg q[3];
creg c[3];
h q[0];
cx q[0], q[1];
cx q[1], q[2];
measure q -> c;
"@

$r2 = Submit-Job "ghz3-qasm2" "OpenQasm2" $ghzIr 3 1000
$job2 = $r2.job_id
Dispatch-Once
$st2 = Get-State $job2
Write-Host "TEST2 state:" $st2.state
$res2 = Poll-Result $job2 20
Print-Result "TEST2 GHZ3 OpenQasm2" $job2 $res2

# -------------------------------------------------
# TEST 3: Invalid QASM (Failed ama result VAR mı?)
# -------------------------------------------------
$badIr = @"
OPENQASM 2.0;
include "qelib1.inc";
qreg q[2];
creg c[2];
this_is_not_a_gate q[0];
measure q -> c;
"@

try {
  $r3 = Submit-Job "bad-qasm2" "OpenQasm2" $badIr 2 100
  $job3 = $r3.job_id
  Dispatch-Once
  $st3 = Get-State $job3
  Write-Host "TEST3 state:" $st3.state
  $res3 = Poll-Result $job3 20
  Print-Result "TEST3 Invalid QASM - Expect Error Result" $job3 $res3

  Write-Host ""
  Write-Host "NOTE: TEST3'te counts boş olabilir; meta/error alanını kontrol et."
} catch {
  Write-Host ""
  Write-Host "TEST3 FAIL (bu kötü): Failed job result üretmemiş olabilir."
  Write-Host $_.Exception.Message
}

Write-Host ""
Write-Host "ALL TESTS DONE."

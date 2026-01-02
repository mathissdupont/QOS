$repo = (Resolve-Path ".").Path
$site = (Resolve-Path ".\.venv\Lib\site-packages").Path
$activate = Join-Path $repo ".venv\Scripts\Activate.ps1"
$python = Join-Path $repo ".venv\Scripts\python.exe"

if (Test-Path $activate) {
    . $activate
}

if (Test-Path $python) {
    # Help PyO3 reliably locate Python on Windows.
    $env:PYO3_PYTHON = $python
}

$env:PYTHONPATH = "$repo;$site"

if (-not $env:QOS_WORKERS) {
    $env:QOS_WORKERS = "2"
}

if (-not $env:QOS_POLL_MS) {
    $env:QOS_POLL_MS = "50"
}

if (-not $env:QOS_MANUAL_DISPATCH) {
    $env:QOS_MANUAL_DISPATCH = "0"
}

cargo run -p qosd

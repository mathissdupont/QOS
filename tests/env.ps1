$repo = (Resolve-Path ".").Path
$site = (Resolve-Path ".\.venv\Lib\site-packages").Path

$env:PYTHONPATH = "$repo;$site"

Write-Host "PYTHONPATH set:"
Write-Host $env:PYTHONPATH

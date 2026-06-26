#requires -Version 5.1
<#
.SYNOPSIS
  Build (optional) and run QOS in QEMU on Windows.

.DESCRIPTION
  Handles two Windows-specific gotchas automatically:
    1. QEMU may be installed but not on PATH (this script also checks
       "C:\Program Files\qemu").
    2. Non-ASCII characters in the repository path (e.g. "Masaüstü") corrupt
       native-argument passing to qemu-system-x86_64.exe, so the boot image is
       copied to an ASCII-only temp path before launching.

.PARAMETER Build
  Rebuild the boot image first via `cargo os-bootimage` (needs the pinned nightly
  toolchain). Without this switch the existing prebuilt image is used.

.PARAMETER Serial
  Mirror the guest serial output into this console (in addition to the window).

.EXAMPLE
  ./run-qos.ps1
  Boot the existing image in a QEMU window.

.EXAMPLE
  ./run-qos.ps1 -Build -Serial
  Rebuild, then boot with serial output shown in the terminal.
#>
[CmdletBinding()]
param(
    [switch]$Build,
    [switch]$Serial
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot

function Find-Qemu {
    $c = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
    if ($c) { return $c.Source }
    $p = Join-Path $env:ProgramFiles 'qemu\qemu-system-x86_64.exe'
    if (Test-Path $p) { return $p }
    throw "QEMU not found. Install it: 'winget install qemu' or https://qemu.weilnetz.de/w64/"
}

if ($Build) {
    Write-Host 'Building boot image (cargo os-bootimage)...' -ForegroundColor Cyan
    Push-Location $root
    try { cargo os-bootimage } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw "Build failed (exit $LASTEXITCODE)." }
}

$img = Join-Path $root 'target\x86_64-unknown-none\debug\bootimage-os.bin'
if (-not (Test-Path $img)) {
    throw "Boot image not found:`n  $img`nRun with -Build first (or build via Docker)."
}

# Copy to an ASCII-only working dir to dodge non-ASCII argv corruption.
$work = Join-Path $env:TEMP 'qos-run'
New-Item -ItemType Directory -Force -Path $work | Out-Null
$boot = Join-Path $work 'qos-boot.bin'
Copy-Item $img $boot -Force
$log = Join-Path $work 'qos-serial.log'
Remove-Item $log -ErrorAction SilentlyContinue

$qemu  = Find-Qemu
$qargs = @('-drive', "format=raw,file=$boot", '-m', '256M')

Write-Host ''
Write-Host 'QOS is booting in QEMU.' -ForegroundColor Green
Write-Host "  Type 'gdesk' for the graphical desktop, 'help' for all commands."
Write-Host '  MOUSE: click inside the QEMU window to CAPTURE the mouse' -ForegroundColor Yellow
Write-Host '         (relative PS/2 mouse only moves once captured; Ctrl+Alt+G releases).' -ForegroundColor Yellow
Write-Host ''

if ($Serial) {
    $qargs += @('-serial', 'stdio')
    Write-Host '(serial mirrored below; close the QEMU window to stop)'
    & $qemu @qargs
} else {
    $qargs += @('-serial', "file:$log")
    Start-Process -FilePath $qemu -ArgumentList $qargs
    Start-Sleep -Seconds 3
    Write-Host "Serial log: $log"
}

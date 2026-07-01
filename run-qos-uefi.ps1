#requires -Version 5.1
<#
.SYNOPSIS
  Build (optional) and run the QOS UEFI image in QEMU with OVMF firmware (Windows).

.DESCRIPTION
  QOS now boots via UEFI on a linear framebuffer (bootloader 0.11, ADR-0014). This is the
  successor to run-qos.ps1 (which drove the retired legacy-BIOS bootimage path).

  Handles the same Windows gotchas as run-qos.ps1:
    1. QEMU may be installed but not on PATH (also checks "C:\Program Files\qemu").
    2. A non-ASCII repo path (e.g. "Masaüstü") corrupts native argument passing to
       qemu-system-x86_64.exe, so the image and firmware are copied to an ASCII-only temp
       path before launching.
  OVMF requires a writable variable store, so a private copy of the vars template is used.

.PARAMETER Build
  Rebuild the UEFI image first via `cargo image` (needs the pinned nightly toolchain).

.PARAMETER Serial
  Mirror the guest serial output into this console (in addition to the QEMU window).

.EXAMPLE
  ./run-qos-uefi.ps1 -Build -Serial
  Rebuild the UEFI image, boot it, and show the serial log in the terminal.
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

function Find-QemuShare {
    $qemu = Find-Qemu
    $share = Join-Path (Split-Path $qemu -Parent) 'share'
    if (-not (Test-Path $share)) { throw "QEMU 'share' dir not found next to $qemu" }
    return $share
}

if ($Build) {
    Write-Host 'Building UEFI image (cargo image)...' -ForegroundColor Cyan
    Push-Location $root
    try { cargo image } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw "Build failed (exit $LASTEXITCODE)." }
}

$img = Join-Path $root 'dist\qos-uefi.img'
if (-not (Test-Path $img)) {
    throw "UEFI image not found:`n  $img`nRun with -Build first (or `cargo image`)."
}

$share = Find-QemuShare
$ovmfCode = Join-Path $share 'edk2-x86_64-code.fd'
$ovmfVars = Join-Path $share 'edk2-i386-vars.fd'   # the vars store is arch-independent format
if (-not (Test-Path $ovmfCode)) { throw "OVMF firmware not found: $ovmfCode" }
if (-not (Test-Path $ovmfVars)) { throw "OVMF vars template not found: $ovmfVars" }

# Copy everything to an ASCII-only working dir to dodge non-ASCII argv corruption.
$work = Join-Path $env:TEMP 'qos-run-uefi'
New-Item -ItemType Directory -Force -Path $work | Out-Null
$bootImg  = Join-Path $work 'qos-uefi.img'
$codeCopy = Join-Path $work 'OVMF_CODE.fd'
$varsCopy = Join-Path $work 'OVMF_VARS.fd'   # writable per-run copy
Copy-Item $img $bootImg -Force
Copy-Item $ovmfCode $codeCopy -Force
Copy-Item $ovmfVars $varsCopy -Force
Set-ItemProperty -Path $varsCopy -Name IsReadOnly -Value $false

$qemu  = Find-Qemu
$qargs = @(
    '-machine', 'q35',
    '-drive', "if=pflash,unit=0,format=raw,readonly=on,file=$codeCopy",
    '-drive', "if=pflash,unit=1,format=raw,file=$varsCopy",
    '-drive', "format=raw,file=$bootImg",
    '-m', '512M'
)

Write-Host ''
Write-Host 'QOS (UEFI) is booting in QEMU.' -ForegroundColor Green
Write-Host "  Type 'gdesk' for the graphical desktop, 'help' for all commands."
Write-Host '  MOUSE: click inside the QEMU window to CAPTURE the mouse' -ForegroundColor Yellow
Write-Host '         (relative PS/2 mouse only moves once captured; Ctrl+Alt+G releases).' -ForegroundColor Yellow
Write-Host ''

if ($Serial) {
    $qargs += @('-serial', 'stdio')
    Write-Host '(serial mirrored below; close the QEMU window to stop)'
    & $qemu @qargs
} else {
    $log = Join-Path $work 'qos-serial.log'
    Remove-Item $log -ErrorAction SilentlyContinue
    $qargs += @('-serial', "file:$log")
    Start-Process -FilePath $qemu -ArgumentList $qargs
    Start-Sleep -Seconds 3
    Write-Host "Serial log: $log"
}

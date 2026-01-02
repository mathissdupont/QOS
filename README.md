
# QOS

Usage guide: see [docs/USAGE.md](docs/USAGE.md).

This repo contains a Rust bare-metal kernel (`crates/qos-os-kernel`, package name: `os`) plus supporting crates.

## Kernel (QEMU)

The kernel is `no_std`, so you must build it for the bare-metal target:

- Build bootable image:
	- `cargo os-bootimage`
	- (equivalent) `cargo bootimage -p os --target x86_64-unknown-none`

- Run in QEMU:
	- `cargo os-run`
	- (equivalent) `cargo run -p os --target x86_64-unknown-none`

### Ring3 Quantum Demo (syscalls)

The kernel includes a minimal Ring3 demo that uses `int 0x80` + a shared-memory ABI to:

- Submit a tiny “Bell” job
- Query status
- Fetch deterministic result counts (512/512)
- Exit QEMU via `isa-debug-exit`

Run it:

- `cargo os-run --features "userdemo,verify"`

## Verification

- `cargo os-verify`

`os-verify` builds/runs the kernel with `--features verify,userdemo` and asserts log markers for the Ring3 syscall quantum demo.

## Hosted UI (recommended for day-to-day development)

Run the server and open the web UI:

- PowerShell: `./run-qosd.ps1`
- Then open: http://127.0.0.1:8080/

By default, `qosd` uses a pure-Rust **stub simulator backend** so it builds/runs on a fresh Windows machine without needing Python.

To use the Python simulator backend (requires Python/venv):

- `cargo run -p qosd --features python`

This path lets you iterate on `qos-core`/`qosd` quickly on Windows, then later port the same API/logic into the kernel/userland.

## ABI RPC (kernel/userland wire model on host)

`qosd` also exposes a single RPC endpoint that accepts/returns `qos-abi` JSON:

- POST `http://127.0.0.1:8080/abi`

Example (PowerShell):

- Submit:
	- `$req = @{ Submit = @{ proc = @{ name='bell'; ir_format='OpenQasm3'; ir_bytes=@(79,80,69,78,81,65,83,77,32,51,59); n_qubits=2; shots=100 } } } | ConvertTo-Json -Depth 8`
	- `Invoke-RestMethod http://127.0.0.1:8080/abi -Method Post -ContentType 'application/json' -Body $req`

## Notes

- `cargo bootimage -p os` **without** `--target x86_64-unknown-none` will try to build for the host target and fail.


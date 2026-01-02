# QOS Usage Guide

This repo contains a Rust x86_64 `no_std` kernel ("QOS") plus hosted components.

## Quick Commands

- Verify (headless QEMU, deterministic):
  - `cargo os-verify`

- Run interactively in QEMU:
  - `cargo run -p os --target x86_64-unknown-none`

- Run interactively with a second "FS disk" attached (safe, stored in workspace):
  - `cargo xtask run-fs`

## Shell: Scheduled User Processes (Job Control)

The kernel shell supports running ELF64 user programs as **scheduled processes**.

- `spawn <path>`: start an ELF64 program as a background scheduled process.
- `exec <path>`: start an ELF64 program as a foreground scheduled process and wait for exit.
- `udemo`: run the built-in Ring3 demo as a scheduled foreground process.
- `udemo-bg`: run the built-in Ring3 demo as a scheduled background process.
- `procs`: list scheduled processes (foreground marked with `*`).
- `fg <pid>`: set the foreground scheduled PID (Ctrl+C targets it) and wait for exit.
- `bg [pid]`: clear the foreground PID (or background a specific pid).
- `killp <pid>`: terminate a scheduled process.
- `waitp <pid>`: wait for a scheduled process to exit.
- `ui [on|off]`: toggle the embedded UI overlay.

### Shell Quality-of-Life

- Command history: use `↑` / `↓` to cycle through previous commands.
  - Press `↓` past the newest entry to restore what you were typing.
- Keyboard layout: `kbd us` / `kbd tr`.
  - Note: the VGA console is ASCII-only, so `kbd tr` uses ASCII transliteration for TR-specific letters.

## Try These Scenarios (recommended)

These are short, repeatable "smoke tests" you can run interactively to confirm things work.

1) UI + history sanity
  - Boot: `cargo run -p os --target x86_64-unknown-none`
  - In QOS: type `help`, then press `↑` to recall it, `↓` to return.
  - Toggle UI: `ui off` then `ui on`.

2) Scheduled processes + Ctrl+C job control
  - Start a background Ring3 process: `udemo-bg`
  - See it in the dashboard and with: `procs`
  - Bring it foreground: `fg <pid>`
  - Hit `Ctrl+C` to terminate the foreground process.
  - Confirm exit: `waitp <pid>`

3) Kernel jobs (quantum submit pipeline)
  - Submit a job: `submit-ir-bell 1024`
  - List: `jobs`
  - Watch it progress: `status <handle>`
  - When `Done`: `result <handle>`
  - Try cancel: `submit-ir-bell 1024` then `cancel <handle>`

4) VFS and disk (requires extra disk)
  - Boot with disk: `cargo xtask run-fs`
  - Format once: `mkfs`
  - Create a program: `mkbell /ram/bell.qasm`
  - Copy to disk: `dput bell.qasm` then `dls`
  - Submit from disk: `dsubmit bell.qasm 1024`

- Build bootable raw disk image:
  - `cargo bootimage -p os --target x86_64-unknown-none`

## What `os-verify` Exercises (Ring3 + Quantum Submit)

`os-verify` builds the kernel with `--features verify,userdemo` and runs QEMU headless.
The kernel:

1. Boots (paging + heap)
2. Sets up GDT/IDT + PIT timer (100Hz)
3. Enters Ring3 (user mode)
4. Ring3 submits a real QASM2 payload to the kernel via a shared-memory syscall ABI
5. Kernel schedules the job using timer ticks (preemptive RR) until it reaches `Done`
6. Ring3 polls `GetStatus` until `Done`, then calls `GetResult`
7. Ring3 calls `Exit`, and the kernel exits QEMU via `isa-debug-exit`

## Shared-Memory Syscall ABI (current)

(You can think of this as the **syscall protocol/contract** between userland and kernel.)

The user/kernel boundary uses a fixed `repr(C)` call frame at `ABI_CALL_ADDR = 0x4001_0000`.

- User writes fields in the call frame and triggers `int 0x80`
- Kernel reads `op`, writes `status` and `ret*`

Key ops (see `qos_abi::shm`):

- `OP_SUBMIT_IR`:
  - `arg0 = user_ptr` (virtual address in user space)
  - `arg1 = total_len` (bytes)
  - Buffer format is **header + payload**:
    - `ShmSubmitIrHeader` (versioned) followed immediately by `payload_len` bytes of IR
    - Header fields include `ir_format`, `n_qubits`, `shots`, `payload_len`
  - Kernel copies up to 4096 bytes, validates header + QASM2 (`OPENQASM` substring)
  - Returns `ret0 = handle`

- `OP_GET_STATUS`:
  - `arg0 = handle`
  - Returns `ret0 = state` (as `qos_abi::JobState` discriminant)

- `OP_GET_RESULT`:
  - `arg0 = handle`
  - Only succeeds when job state is `Done`
  - Returns `ret0=n00`, `ret1=n11`

- `OP_CANCEL`:
  - `arg0 = handle`
  - Marks job cancelled

- `OP_VFS_IO`:
  - `arg0 = user_ptr` (virtual address in user space)
  - `arg1 = total_len` (bytes)
  - Buffer format is **header + path + data region**:
    - `ShmVfsIoHeader` (versioned)
    - `path_len` bytes of path (e.g. `/ram/bell.qasm`, `/disk/bell.qasm`)
    - `data_cap` bytes of data region
  - `vfs_op`:
    - `VFS_OP_READ`: kernel writes bytes into data region, returns `ret0=bytes_written`
    - `VFS_OP_WRITE`: user provides `data_len` bytes in data region
    - `VFS_OP_REMOVE`: deletes file
    - `VFS_OP_LIST_DIR`: minimal listing (`/` -> `/ram` + `/disk`)

## Notes

- The current demo payload is a QASM2 Bell circuit string that the kernel preloads into a user-mapped page.
- Result counts are deterministic (derived from content patterns) so CI/verify stays stable.

## Kernel Shell (interactive)

When you run the kernel interactively (without `userdemo`), a minimal VGA "CMD-like" shell is available.

Commands:

- `help` — show help
- `clear` — clear VGA screen
- `ticks` — show PIT tick counter
- `pwd` — print current dir
- `cd <dir>` — change dir (supported: `/`, `/ram`, `/disk`)
- `ls [dir]` — list files (defaults to current dir)
- `cat <path>` — print file contents (accepts `/ram/...`, `/disk/...`, and relative paths)
- `rm <path>` — delete file (accepts VFS paths)
- `mkbell <path>` — create a built-in `bell.qasm` program (accepts VFS paths)
- `submit <path> [shots]` — submit a QASM2 file as a job (accepts VFS paths)
- `disk-id` — identify attached FS disk (IDE index=1)
- `disk-read <lba>` — read one sector from FS disk
- `mkfs` — format the persistent disk filesystem (IDE index=1)
- `dls` — list files on disk filesystem
- `dcat <file>` — print disk file
- `drm <file>` — delete disk file
- `dput <file>` — copy RAM file -> disk filesystem
- `dget <file>` — copy disk filesystem -> RAM file
- `dsubmit <file> [shots]` — submit disk QASM2 file as job
- `jobs` — list current kernel job table
- `submit-bell` — submit a built-in Bell-ish job (kernel-side)
- `submit-ir-bell [shots]` — submit a built-in QASM2 Bell IR job
- `status <handle>` — show job status
- `result <handle>` — get job result (frees slot when `Done`)
- `cancel <handle>` — cancel a job

Scheduled user processes (job control):

- `procs` — list scheduled processes
- `spawn <path>` — spawn ELF64 background process
- `exec <path>` — run ELF64 as foreground process and wait
- `udemo` / `udemo-bg` — run built-in Ring3 demo (scheduled)
- `fg <pid>` / `bg [pid]` — manage foreground PID (Ctrl+C targets foreground)
- `killp <pid>` / `waitp <pid>` — terminate or wait for a pid
- `ui [on|off]` — toggle the embedded UI overlay

### VFS (path-based, mounts)

VFS provides a unified namespace with two mounts:

- `/ram` - RAM filesystem
- `/disk` - persistent disk filesystem (only available if you boot with the extra disk image)

VFS commands:

- `vls [dir]` - list a VFS directory (defaults to current dir). Supported dirs: `/`, `/ram`, `/disk`
- `vcat <path>` - print file contents by path (accepts relative paths too)
- `vrm <path>` - remove file by path
- `vcp <src> <dst>` - copy between mounts (e.g. `/ram/x` -> `/disk/x`)
- `vsubmit <path> [shots]` - submit a QASM2 file from a VFS path as a job

User-mode / ELF:

- `userdemo` — enter the built-in Ring3 demo (returns to shell on OP_EXIT)
- `exec <path>` — run an ELF64 (x86_64) from VFS as a scheduled foreground process (returns after exit)
### Notes about the filesystem

This is currently a minimal **RAM-backed** filesystem:

- Max files: 32
- Max filename length: 32 bytes
- Max file size: 65536 bytes (64 KiB)
- No persistence yet (files reset on reboot)

### Notes about the persistent disk filesystem

This is a minimal **disk-backed** filesystem stored in `target/qos-fs.img` when using `cargo xtask run-fs`.

- Only uses the second QEMU IDE disk (index=1)
- MVP format: superblock + fixed directory + append-only data area
- No crash safety / free space reuse yet (we will stabilize/optimize later)

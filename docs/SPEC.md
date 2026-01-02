
# QOS Spec (Roadmap)

Goal: A "Quantum Operating System" that feels like Windows/Linux, but treats quantum circuits as first-class workloads (like processes/jobs), with scheduling, resource management, UI, and device backends (simulator now, real QPU later).

Important reality check:
- The CPU remains classical; quantum execution happens on a *device/backend* (simulator or QPU).
- "Quantum OS" means the OS provides: job/process model, scheduling, isolation, accounting, drivers/adapters, UI, and developer tooling for quantum workloads.

## Layered Architecture

1) **ABI (shared language)**
- `qos-abi` (no_std): request/response + versioning for the syscall boundary.
- Stable types: Job handle, proc spec, result spec.

2) **Core logic (OS-agnostic)**
- `qos-core` (hosted today): job store, scheduler strategies, journaling, recovery.
- Long term: compile in `no_std + alloc` mode (or split a `qos-core-nostd`).

3) **Userland service**
- `qosd` (hosted today): the "quantum daemon".
- Long term: runs as a userland process in QOS and talks to kernel via syscalls/IPC.

4) **Kernel**
- `qos-os-kernel`: boot, memory, interrupts, syscall entry, process model.
- Long term: exposes syscalls defined in `qos-abi`.

5) **UI**
- Hosted UI (today) validates end-to-end workflow.
- Long term: OS GUI or shell uses the same API model.

## Milestones (Step-by-step)

### Milestone A — Product loop (hosted, fast)
**Objective:** You can submit circuits, see status, see results, and trust persistence.

Deliverables:
- Web UI in `qosd` (submit + jobs + poll status/result).
- `qos-core` tests passing and journaling/recovery deterministic.

### Milestone B — Stable syscall ABI
**Objective:** Define the boundary between kernel/userland like Linux syscalls.

Deliverables:
- `qos-abi` with `QosRequest/QosResponse` and `ABI_VERSION`.
- A host-side adapter that maps `JobHandle` <-> internal IDs.

### Milestone C — Userland-on-host (simulate OS services)
**Objective:** Run `qosd` as if it's a userland daemon, using the ABI message model.

Deliverables:
- An IPC transport (host): initially in-proc function call, then TCP/Unix-like, later shared memory.
- `qosd` can serve both HTTP UI and ABI RPC.

### Milestone D — Kernel process + syscalls
**Objective:** Minimal user mode + `int 0x80` syscalls that carry ABI messages.

Deliverables:
- Kernel syscall handler reads a request buffer, writes a response buffer.
- A tiny userland demo issues `Submit` then `Status`.

### Milestone E — Boot/verify reliability
**Objective:** QEMU boots deterministically; `xtask verify` reliably captures logs and exits.

Deliverables:
- Boot reaches kernel prints.
- QEMU exits via `isa-debug-exit` in verify mode.

## Non-goals (for now)
- Full GUI desktop environment like Windows Explorer.
- Real QPU drivers (we keep simulator backend first).


# WP-08: Kernel foundations — preemption, user-mode processes, memory protection

- Status: 🟡 in progress (slice 1 done)
- Epic: E-11 (SMP later), E-30 (processes), E-31 (syscalls)
- ADRs: ADR-0020 (CPU hardening baseline); new ADRs per slice below
- Commits: 43d10e0 (slice 1: preemptive scheduler armed under the desktop + background quantum
  jobs), 5dbe995 (slice 2: W^X enforced + page-table audit)

## Goal

Close the biggest "real OS" gaps in the kernel core. Today the desktop runs as a single
cooperative kernel loop: no preemption while an app computes, no isolation between "apps" (all
share kernel memory), and kernel mappings are not W^X-audited. A real OS needs real processes.

## Steps (planned slices)

- [x] **Slice 1 — preemption in production (43d10e0).** The modern desktop runs with the
  `kthread` timer-driven scheduler **armed**: the desktop is the main context and a background
  **quantum-job worker** thread runs beside it (`qjob.rs`: atomic state machine, spin-mutex
  payload slots held only for the enqueue/dequeue instant — spin locks never disable interrupts,
  so lock contention resolves via the timer, no deadlock). Lab Run / IDE F5 submit jobs; the
  desktop polls completion and delivers results asynchronously; Processes shows the worker state.
  **Verified in QEMU:** a 16-qubit 1000-shot GHZ ran ~1 min on the worker while the UI kept
  accepting input and redrawing; result arrived with the exact physical split (0¹⁶ 529 / 1¹⁶ 471).
  *(Original "compositor becomes a thread itself" reworded: the desktop stays the main context —
  full compositor-as-thread lands with slice 3's process model if needed.)*
- [x] **Slice 2 — W^X enforced + audited (5dbe995).** Found and closed the real hole: the 64 MiB
  kernel **heap was W+X** (mapped without NX). Heap pages now carry NO_EXECUTE, with EFER.NXE
  enabled before the mapping (NX is a reserved PTE bit while NXE is off — ordering documented).
  `security::wx_audit()` walks the live page table (1G/2M/4K leaves) and reports W+X pages at
  boot + in the Settings Security row. **Result: 0 W+X pages of ~268 M mapped** (bootloader
  mappings were already NX). Regression-proof: the preemptive quantum worker (stack + statevector
  on the NX heap) runs to completion. *(A dedicated ADR is deferred until kernel sections get a
  custom layout; today's layout is bootloader-ELF + our NX heap.)*
- [ ] **Slice 3 — user-mode processes.** Build on `user.rs`/`syscall.rs`: load a flat/ELF binary
  into ring 3 with its own address space, syscall surface for fs/console, clean exit + reaping.
  Target: a userland `hello` + a userland QASM runner as the first real apps.
- [ ] **Slice 4 — per-process resources.** Handles/quotas (open files, memory), so a runaway
  process can't exhaust the kernel (extends the "validate before allocate" principle).
- [ ] **Slice 5 — SMP bring-up (E-11).** Start APs via APIC INIT/SIPI, per-CPU scheduler queues.

## Acceptance criteria

Per slice: a QEMU demonstration (e.g. a spinning task no longer freezes the cursor; a user
process crashes without taking the kernel down), 0-fault boot, ADR + WP updated.

## Notes & gaps

- Ordering matters: preemption (1) before user processes (3); W^X (2) is independent.
- The cooperative `scheduler::Scheduler` remains for lightweight kernel services after slice 1.

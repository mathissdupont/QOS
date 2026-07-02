# WP-08: Kernel foundations — preemption, user-mode processes, memory protection

- Status: 🔴 not started
- Epic: E-11 (SMP later), E-30 (processes), E-31 (syscalls)
- ADRs: ADR-0020 (CPU hardening baseline); new ADRs per slice below
- Commits: (appended as delivered)

## Goal

Close the biggest "real OS" gaps in the kernel core. Today the desktop runs as a single
cooperative kernel loop: no preemption while an app computes, no isolation between "apps" (all
share kernel memory), and kernel mappings are not W^X-audited. A real OS needs real processes.

## Steps (planned slices)

- [ ] **Slice 1 — preemptive kernel threads.** Timer-driven preemption for kernel threads
  (`kthread` exists; arm it under the APIC timer), so a busy task can't freeze input/UI. The
  compositor loop becomes a thread instead of owning the CPU.
- [ ] **Slice 2 — W^X audit.** Walk kernel mappings; make code RX, data RW+NX, rodata RO+NX
  (NX/WP already enforced by ADR-0020). New ADR documenting the layout.
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

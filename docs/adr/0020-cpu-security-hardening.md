# ADR-0020: CPU security hardening baseline (NX, WP, SMEP, SMAP)

- **Status:** Accepted
- **Date:** 2026-07-02
- **Deciders:** QOS core
- **Related ADRs:** ADR-0014 (UEFI boot), ADR-0015 (modern hardware)

## Context

QOS aims to be a real, modern OS; the user set security as a standing requirement. Modern x86-64
CPUs provide hardware memory-protection features that mainstream OSes (Linux/Windows/macOS) enable
unconditionally, and their absence is considered a vulnerability:

- **NX / EFER.NXE** — data pages can be marked non-executable (blocks classic stack/heap shellcode).
- **CR0.WP** — ring 0 honors read-only mappings (kernel can't silently scribble on RO pages;
  prerequisite for W^X kernels).
- **CR4.SMEP** — kernel cannot execute user-mode pages (kills "jump to user shellcode" exploits).
- **CR4.SMAP** — kernel cannot read/write user pages unless explicitly permitted (blocks a large
  class of confused-deputy bugs).

SMEP/SMAP exist only on newer CPUs and must be gated on CPUID leaf 7 (universal detection — never
assume a model, per the project's no-hardcoding rule).

## Decision

Add a `security` kernel module that, at boot (right after paging/PCI are up):

1. sets EFER.NXE and CR0.WP unconditionally,
2. sets CR4.SMEP / CR4.SMAP **iff** CPUID.(7,0).EBX reports them (bits 7 / 20),
3. records the active set and logs `[SEC] hardening: NX .. WP .. SMEP .. SMAP ..`,
4. surfaces the status in the UI (a **Security** row in Settings via `security::status_line()`).

Missing features are reported, never fatal (fallback-first).

Related input-hardening in the same change: the quantum engine bounds attacker-controllable
allocations (`MAX_QUBITS`, ADR-0019) — the same "validate before you allocate" principle applied
at the subsystem level.

## Rationale

- These are the lowest-cost, highest-value hardening bits on x86-64 — single register writes.
- Surfacing them in Settings makes the security posture *visible*, aligning with the project's
  "real OS" bar (users can see what protections are active on their machine).

## Consequences

### Positive

- Immediate hardware-enforced protections wherever the CPU supports them; on QEMU TCG (qemu64)
  NX + WP are active, SMEP/SMAP correctly detected absent; on real hardware all four engage.

### Negative / Trade-offs

- SMAP requires `stac`/`clac` around intentional user-memory access once user mode matures — the
  user-copy paths must be audited then (follow-up).
- WP means writing genuinely RO kernel data now faults — desirable, but any legacy self-patching
  code would break (none known).

### Neutral / Follow-ups

- Next hardening steps (future ADRs): W^X audit of kernel mappings, guard pages for kernel
  stacks, UMIP, KASLR-style layout randomization, syscall argument validation audit.

## Alternatives considered

1. **Do nothing (rely on the bootloader's defaults)** — NXE happens to be set by the bootloader,
   but WP/SMEP/SMAP are not guaranteed; implicit inheritance is not a security posture. Rejected.
2. **Full W^X remap now** — requires walking/adjusting all kernel mappings; planned as its own
   work package rather than blocking the baseline bits.

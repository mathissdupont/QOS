# WP-03: ACPI + modern APIC interrupts

- Status: ✅ done
- Epic: E-10
- ADRs: ADR-0015
- Commits: b47c59b, 5d08347, 610e2f0, a0b13be

## Goal

Move off the legacy 8259 PIC / 8254 PIT to the modern APIC interrupt architecture — the
prerequisite for USB interrupts and SMP.

## What was delivered (slices)

1. **Discovery** — `qos-acpi` crate (SDT/MADT parsing, 8 host tests); kernel walks RSDP (from
   `boot_info.rsdp_addr`) → XSDT → MADT and logs the APIC topology.
2. **Local APIC enable** — IA32_APIC_BASE global-enable, SVR software-enable + spurious vector
   (0xFF, with an IDT handler), TPR=0.
3a. **APIC timer** — calibrated against a PIT channel-2 busy-wait; scheduler tick moved to the
   local-APIC timer; PIT IRQ0 masked; `timer_dispatch` EOIs the APIC.
3b. **IO-APIC** — keyboard (GSI1) and mouse (GSI12) routed to the local APIC; external-IRQ EOI
   switched to the APIC; **8259 PIC fully masked**.

## Acceptance criteria

✅ QEMU+OVMF (q35): boots to `QaOS ready`, 0 faults; APIC-timer self-test shows ticks advancing
with the PIT masked; with the PIC fully masked, `gdesk` typed on the keyboard opens the desktop
and the `d` key opens the Display app (screenshot) — tick + input both on the pure APIC path.

## Notes & gaps

- Legacy `acpi.rs` RSDP scan still broken under UEFI; only its shutdown ports are wanted (**G-01**).
- Single BSP; no APs started yet (**G-05**, blocks SMP E-11).

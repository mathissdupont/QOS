# WP-01: UEFI boot repair + framebuffer desktop

- Status: ✅ done
- Epic: E-70 (UI seed), platform boot
- ADRs: ADR-0014
- Commits: a6adaf4, a8f0e06

## Goal

Make QOS actually boot under UEFI and render its desktop on the linear framebuffer.

## What was delivered

- Fixed three stacked boot-chain bugs that triple/double-faulted before the OS could run:
  RELRO-read-only `.data` (→ `relocation-model=static` + `-z norelro`), a missing kernel-data
  GDT segment (→ add it + reload SS/DS/ES), and a 256 KiB stack too small for the E1000 struct
  (→ 1 MiB).
- `qos-gfx` crate (palette + integer-scale `ScaleMap`, 10 host tests); kernel `draw` facade;
  ported `gfxui` to render on the framebuffer; added the **Display** app.
- `run-qos-uefi.ps1` launcher (QEMU + OVMF).

## Acceptance criteria

✅ Boots to `QaOS ready` under QEMU+OVMF (q35 and pc), 0 faults; `gdesk` renders the desktop
fullscreen at 1280×800 (4× scale) — verified by screenshot.

# ADR-0014: Migrate to bootloader 0.11 for UEFI boot and a high-resolution framebuffer

- Status: Accepted
- Date: 2026-06-27
- Supersedes parts of: ADR-0013 (graphics path)

## Context

QOS currently boots via `bootloader` 0.9.29, which is **BIOS-only** and gives the kernel a
legacy environment. Two user-facing goals require more than this:

1. **Run on modern hardware** — most current machines are UEFI; many are UEFI-only. Legacy BIOS
   boot is increasingly unavailable (no CSM).
2. **High-resolution graphics** — the desktop currently uses VGA Mode 13h (320×200×256), set by
   programming VGA registers from the kernel. Arbitrary high-resolution modes require real-mode
   VBE/GOP calls that can only run *before* the kernel (in the bootloader), so the kernel cannot
   set them itself in long mode.

The `bootloader` 0.11 line solves both: it boots on **both BIOS and UEFI**, and it hands the
kernel a **linear framebuffer** (GOP on UEFI, VBE on BIOS) at a chosen resolution, plus a richer
`BootInfo` (memory regions, physical-memory offset, RSDP, etc.).

## Decision

Migrate the boot path from `bootloader` 0.9.29 to **`bootloader` 0.11.x** (`bootloader_api` in
the kernel, `bootloader` as the image builder). Produce **both** a BIOS disk image and a UEFI
disk image from the same kernel ELF.

The kernel keeps the existing VGA Mode 13h graphics initially (it still works under the new
bootloader because it is direct hardware programming); the high-resolution framebuffer path is
added in a later stage so UEFI boot and hi-res graphics can be de-risked independently.

## Build model

- The kernel (`crates/qos-os-kernel`, package `os`) depends on **`bootloader_api`** and exposes
  `bootloader_api::entry_point!(kernel_main, config = &CONFIG)` with
  `fn kernel_main(boot_info: &'static mut bootloader_api::BootInfo) -> !`.
- `CONFIG` requests a dynamic physical-memory mapping (`mappings.physical_memory =
  Some(Mapping::Dynamic)`) so `boot_info.physical_memory_offset` is populated, and a framebuffer.
- A new builder crate (`crates/qos-image`) takes the compiled kernel ELF as an **artifact
  dependency** (`-Z bindeps`) and uses `bootloader::UefiBoot` / `bootloader::BiosBoot` to create
  `qos-uefi.img` and `qos-bios.img`.
- `.cargo/config.toml` enables `[unstable] bindeps = true`; `rust-toolchain.toml` adds the
  `x86_64-unknown-uefi` target.

## Staged plan (each stage independently verifiable)

1. **Build foundation + BIOS boot.** Kernel → `bootloader_api`; rewrite `memory.rs` frame
   allocator for `MemoryRegions`; builder crate produces a BIOS image; verify it boots to
   `QaOS ready` in QEMU. (Graphics stay Mode 13h.)
2. **UEFI boot.** Produce the UEFI image; verify it boots under QEMU + OVMF firmware.
3. **High-resolution framebuffer (Phase 3.2).** Add a linear-framebuffer drawing backend; port
   `vga13h`/`gfxui` primitives to it; pick a default mode (e.g. 640×480 / 800×600 truecolor)
   with Mode 13h as a fallback when no framebuffer is provided.
4. **Real-hardware validation (Phase 4.1).** Write the UEFI image to USB; test on real machines
   and the major hypervisors; document results.

## Consequences

- **Pros:** boots on BIOS *and* UEFI (real-hardware reach); high-resolution truecolor graphics;
  a modern, maintained bootloader with a richer `BootInfo`.
- **Cons / risks:** invasive change to boot entry, memory init, build tooling, and (later) the
  graphics stack; UEFI testing needs OVMF; the build now uses unstable `bindeps`. Mitigated by
  doing it on a branch, staging the work, and keeping `main` (BIOS, Mode 13h) fully working until
  each stage is verified.
- `cargo-bootimage` (0.9-specific) is retired in favor of the builder crate.

## Status of execution

Tracked in `docs/PLAN.md` (Phase 3.2 + Phase 4). Stage 1 is in progress on branch
`feat/uefi-framebuffer`.

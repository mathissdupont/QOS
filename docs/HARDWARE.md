# Running QOS on real hardware and VMs

This document describes how to run QOS outside the development QEMU setup, and what hardware is
currently supported. It is kept honest: items are marked **verified**, **should work**, or
**not yet supported**.

## Boot model

QOS currently boots via **legacy BIOS** (it uses `bootloader` 0.9.29, which produces a raw,
MBR-bootable disk image — `bootimage-os.bin`). It is **not yet UEFI-native**: on modern
UEFI-only machines you must enable **CSM / Legacy Boot**, or wait for the UEFI build (tracked as
Phase 3.2, a `bootloader` 0.11+ migration — see [PLAN.md](PLAN.md)).

The build produces a single bootable image. The helper `dist/` images below are the same image
in different container formats:

| File | Format | Use |
|---|---|---|
| `dist/qos.img` | raw | `dd` to a USB stick; QEMU `-drive format=raw` |
| `dist/qos.vdi` | VirtualBox | attach as a VirtualBox disk |
| `dist/qos.vmdk` | VMware | attach as a VMware disk |

Regenerate them from a fresh build with:

```sh
cargo os-bootimage
qemu-img convert -f raw -O vdi  target/x86_64-unknown-none/debug/bootimage-os.bin dist/qos.vdi
qemu-img convert -f raw -O vmdk target/x86_64-unknown-none/debug/bootimage-os.bin dist/qos.vmdk
cp                              target/x86_64-unknown-none/debug/bootimage-os.bin dist/qos.img
```

## How to run

### QEMU — verified

```powershell
./run-qos.ps1 -Build
```

Verified machine types: the default `pc` (i440fx) and `q35`. Boot reaches the shell and the
graphical desktop (`gdesk`) on both, with or without a network device.

### VirtualBox — should work (legacy BIOS)

1. New VM → Type: *Other*, Version: *Other/Unknown (64-bit)*.
2. **System → Motherboard → disable EFI** (use legacy BIOS).
3. Attach `dist/qos.vdi` as the (IDE/SATA) hard disk.
4. Start. (PS/2 mouse: click into the VM to capture; host key releases it.)

### VMware — should work (legacy BIOS)

Create a VM with **BIOS** firmware (not UEFI) and attach `dist/qos.vmdk` as the disk.

### Real PC via USB — should work where Legacy/CSM is available

Write the raw image to a USB stick, then boot the target machine with **Legacy/CSM** enabled:

- Linux/macOS: `sudo dd if=dist/qos.img of=/dev/sdX bs=4M conv=fsync` (replace `/dev/sdX` with
  your USB device — **this erases it**).
- Windows: use a raw-image writer such as Rufus in *DD image* mode, or `Win32 Disk Imager`.

> ⚠️ Real-hardware boot has not been independently verified by the maintainers across many
> machines. It depends on the firmware supporting legacy boot. Reports (success or failure, with
> the machine model) via an issue are very welcome.

## Hardware probing & graceful fallback

QOS probes for devices at boot and continues if they are absent — a missing device must not hang
or panic the kernel:

| Device | Behavior |
|---|---|
| PS/2 keyboard | Driven via IRQ1; assumed present. |
| PS/2 mouse | Probed (Intellimouse); the desktop works without it (keyboard shortcuts). |
| PIT timer (IRQ0) | Used for preemptive scheduling and the clock. |
| PCI bus | Enumerated; devices logged. |
| Intel E1000 NIC | Optional — if not found, networking is skipped (verified: boots `-net none`). |
| RTC | Read for the wall clock. |

## Known limitations

- **UEFI** is not supported yet (legacy BIOS only) — Phase 3.2.
- **Graphics** use VGA Mode 13h (320×200×256) for the desktop; higher-resolution VESA is Phase 3.2.
- **APIC/HPET**: QOS currently assumes the legacy PIC/PIT; APIC/HPET support is planned (Phase 4.3).
- SMP (multi-core) is not used; QOS runs on a single core.

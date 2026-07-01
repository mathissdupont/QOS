# QOS build & test recipe (Windows)

The exact workflow used to build and verify QOS. Kept so any session can reproduce it.

## Build

```sh
# Bootable UEFI image → dist/qos-uefi.img (release, uses -Z bindeps to embed the kernel ELF)
cargo image

# Bare-metal kernel only (faster compile check)
cargo os-build            # = build -p os --target x86_64-unknown-none -Zbuild-std=core,alloc

# Host unit tests for the portable crates
cargo test -p qos-gfx -p qos-driver -p qos-acpi -p qos-core --features qos-core/std
```

Toolchain is pinned (nightly-2024-12-01) in `rust-toolchain.toml`. `.cargo/config.toml` sets
`relocation-model=static` + `-z norelro` (required for UEFI — see ADR-0014) and `bindeps`.

## Run / test in QEMU + OVMF (UEFI)

QEMU is at `C:\Program Files\qemu`; OVMF firmware ships with it:
`share/edk2-x86_64-code.fd` (code) and `share/edk2-i386-vars.fd` (vars template — arch-neutral
format). **Copy the image + firmware to an ASCII-only path** first: the repo path contains
`Masaüstü`, and non-ASCII chars corrupt QEMU's native argv on Windows.

Interactive (a window): `./run-qos-uefi.ps1 -Build` (or `-Serial`). It already attaches
`qemu-xhci + usb-kbd + usb-mouse` and handles the OVMF vars copy + ASCII path.

Headless serial-log check (used for automated verification), from an ASCII working dir with
`OVMF_CODE.fd` (from edk2-x86_64-code.fd), a writable `OVMF_VARS.fd` (copy of edk2-i386-vars.fd),
and `qos-uefi.img`:

```sh
qemu-system-x86_64 -machine q35 \
  -drive if=pflash,unit=0,format=raw,readonly=on,file=OVMF_CODE.fd \
  -drive if=pflash,unit=1,format=raw,file=OVMF_VARS.fd \
  -drive format=raw,file=qos-uefi.img -m 512M \
  -device qemu-xhci -device usb-kbd -device usb-mouse \
  -serial file:serial.log -display none
# then grep serial.log for "QaOS ready", "[XHCI]", "[APIC]", "EXCEPTION|HALTED|PANIC"
```

### GUI screenshot test (important gotcha)

To capture the desktop, drive the guest via the QEMU monitor and `screendump`:

```sh
( sleep 10; printf 'sendkey spc\n'; sleep 2; for k in g d e s k; do printf 'sendkey %s\n' "$k"; sleep 0.4; done;
  printf 'sendkey ret\n'; sleep 3.5; printf 'sendkey d\n'; sleep 2; printf 'screendump ui.ppm\n'; sleep 2; printf 'quit\n' ) \
  | qemu-system-x86_64 -machine q35 <pflash+drive as above> -m 512M \
    -serial file:serial.log -monitor stdio -vnc 127.0.0.1:3
```

- **Do NOT attach `usb-kbd` for the screenshot test.** QEMU routes `sendkey` to the USB keyboard
  when present, and USB HID input isn't finished yet (WP-04), so `gdesk` never gets typed. Use the
  PS/2 keyboard (omit `-device usb-kbd`) so `sendkey` drives PS/2 and the desktop launches.
- Convert PPM→PNG with a small pure-Python (zlib) snippet (no PIL needed); `Read` the PNG.
- Confirm the desktop rendered by grepping serial for `[GFXUI] entering interactive desktop`.

## Symbolicating a fault

The image embeds a **release** kernel at
`target/x86_64-unknown-none/release/deps/artifact/os-*/bin/os-*`. Use the LLVM tools under
`~/.rustup/toolchains/*/lib/rustlib/x86_64-pc-windows-msvc/bin/` (`llvm-nm -nC`, `llvm-objdump -dC`)
to map a fault RIP to a symbol. Segments: `.text` at `0xffffffff80000000`.

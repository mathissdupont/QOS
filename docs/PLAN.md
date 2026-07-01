# QOS — Gap Analysis & Delivery Plan

Status snapshot (2026-06-26). This is the working roadmap from "a kernel that boots + a shell"
to "a real, usable, graphical OS". It interleaves the **interactive GUI** track (visible,
motivating) with the **OS-foundation** track (what actually makes it an operating system).
Decisions behind it live in `docs/adr/`.

Legend: ✅ works (verified) · 🟡 partial / code exists but unverified · 🔴 missing or broken.

---

## A. Gaps that stop this from being a *real OS* (core)

These are the load-bearing pillars. Without them it is a kernel demo, not an OS.

1. **User space (Ring 3) with real isolation** — 🔴/🟡
   Today everything (shell, quantum, "apps") runs in Ring 0 as cooperative kernel tasks.
   `user.rs` + `asm_iretq_to_user` exist but are not a reliable, general path. A real OS runs
   untrusted programs in Ring 3.
2. **Per-process virtual address spaces & memory protection** — 🔴
   No per-process page tables in use, no copy-on-write, no demand paging. One bad pointer can
   corrupt everything. This is what *isolation* actually means.
3. **Preemptive scheduling + context switch** — 🟡
   The run model is cooperative (`step()`-based). A real OS preempts on the timer and
   saves/restores full register state across tasks.
4. **Stable syscall ABI** — 🟡
   `int 0x80` + the `qos-abi` shared-memory frame exist, but the surface must be solid and
   complete for Ring 3 programs (process, file, IPC, gfx).
5. **Fault handling that doesn't kill the machine** — 🔴
   Page fault / general-protection fault should report and kill the offending *process*, not
   panic-halt the whole kernel. Required for stability once programs run.
6. **Process lifecycle & IPC** — 🔴
   spawn / exit / wait, signals, and at least one IPC primitive (pipe or shared memory) so
   programs can cooperate.

## B. Gaps that stop it from being *usable like Windows* (desktop)

7. **Interactive pixel GUI** — 🟡 (only a static Mode 13h demo today)
   Needs: a mouse cursor in graphics mode, a unified input event loop, a window manager
   (move/close/focus), and basic widgets (button, menu, textbox).
8. **A persistent, structured filesystem** — 🟡
   `/ram` works; disk/`/disk`/FAT16 code exists but persistence is unverified; no real
   directories/metadata/permissions story end-to-end.
9. **Working networking + secure egress** — 🟡/🔴
   E1000 "link up" is seen; actual TCP/DNS round-trips are unverified; TLS is a non-functional
   stub, so no real cloud reachability (see ADR-0011: terminate TLS in `qosd`).
10. **Apps & shell-of-the-GUI** — 🔴
    A file manager, a text editor in graphics, and our differentiator: a quantum circuit
    editor / job monitor.
11. **Comfort features** — 🔴
    Clipboard, settings, multi-user/login, audio, USB. Long tail.

---

## Delivery plan (phased; each phase ships something testable)

> Verification note: `qos-core` work is host-verifiable with `cargo test`. Kernel work is
> verified by building locally (`./run-qos.ps1 -Build`) and observing QEMU + serial.

### Phase 0 — Foundations the GUI itself needs (do first)

- **0.1 Unified input event queue.** Keyboard + mouse interrupts push into one event queue;
  consumers (shell, GUI) pull events. (Today keyboard drives the shell directly.)
- **0.2 Persistent graphics mode.** Make Mode 13h a real mode the system lives in, with a
  clean return to text (save/restore the VGA font in plane 2). Removes the "reboot to exit".
- **0.3 Fault handlers.** Page-fault / GPF handlers that print diagnostics instead of a bare
  panic-halt. Stability groundwork.

*Ship:* graphics mode you can enter and leave cleanly; input events flowing.

### Phase 1 — Interactive graphical desktop (visible milestone)

- **1.1 Mouse cursor in Mode 13h** (draw/erase a sprite, track movement from the event queue).
- **1.2 Event loop** (draw → wait event → update) replacing the static demo.
- **1.3 Window manager**: one or more windows with a title bar; move by drag; a clickable
  close button; focus.
- **1.4 Widgets**: button + menu, enough to click around.

*Ship:* a desktop you can actually point-and-click — the first "Windows-like" feel.

### Phase 2 — Real-OS core ✅ DONE (verified in QEMU)

- **2.1 Preemptive scheduler + context switch** on the timer interrupt. ✅
  `kthread.rs` + `asm_timer_isr` (global_asm, sidesteps the LLVM naked-asm bug). `threadtest`.
- **2.2 Reliable Ring 3 + solid syscall ABI**. ✅
  New register-based ABI on `int 0x81` (`asm_syscall_isr` + `syscall_dispatch`: rax=number,
  rdi/rsi args, return in rax, with user-pointer validation), coexisting with the `int 0x80`
  shared-memory ABI. `regabitest`.
- **2.3 Per-process page tables + memory protection**. ✅
  Per-process CR3 (lock-free switch), W^X (code demoted to read-execute after load), NX data,
  implicit stack guard page. `proctest`, `wxtest`.
- **2.4 Process lifecycle + IPC**. ✅
  spawn / exit (clean via syscall) / fault-kill (only the offending process dies) / wait;
  in-kernel pipe IPC (`ipc.rs`). `exittest`, `faulttest`, `ipctest`.

*Shipped:* a user-space program runs isolated in Ring 3, is preempted, and a crash/runaway/W^X
violation kills only it — the kernel and other processes survive.

*Optional polish (deferred, no new capability):* migrate the legacy `exec`/`udemo` commands to
launch through the `kthread` engine (they currently use the original direct-`iretq` path).

### Phase 3 — Real, visible GUI (current focus)

- **3.1 Live, multi-window desktop**: surface Phase-2 preemption in the GUI — a background task
  indicator (live counter) + clock in the taskbar; multiple windows/apps open at once; window
  focus/z-order. The "Windows-like" feel.
- **3.2 Higher resolution (VESA)** ✅: kernel migrated to bootloader 0.11 (BIOS+UEFI); the desktop
  now renders through a resolution-agnostic facade (`draw.rs`) onto the bootloader's linear
  framebuffer, integer-scaled and centered from the 320×200 logical canvas to any resolution in
  true color, with VGA Mode 13h kept as a 1:1 fallback. Palette/scale math is host-tested in the
  `qos-gfx` crate; a **Display** app shows the live backend/resolution/scale. See ADR-0014.
- **3.3 More apps**: a file manager (on the existing FS), a process/task monitor (shows the
  Phase-2 processes), a terminal window, alongside the Quantum Lab (circuit editor / job
  monitor — our quantum hook).
- **3.4 Filesystem & networking verify/harden**; wire the **`qosd` TLS proxy** for real cloud
  QPU; kernel cutover to `qos-core` `JobManager` + QHAL.

*Ship:* a usable, good-looking desktop with several real apps.

### Phase 4 — Runs on all computers (real-hardware portability)

Goal: boot and run on real PCs and common VMs, not just QEMU.

- **4.1 Boot portability**: produce a bootable USB/ISO image; verify on UEFI (via CSM/legacy and
  a UEFI path) and BIOS; test under VirtualBox/VMware/Hyper-V and at least one physical machine.
- **4.2 Hardware probing & graceful fallback**: detect RAM, CPU features, storage (AHCI/IDE),
  NIC, and input; degrade gracefully when a device is absent (e.g. no PS/2 mouse, USB-only).
- **4.3 Timing & APIC**: don't assume a fixed PIT/PIC; support APIC/HPET where present, calibrate
  the timer, and avoid QEMU-specific assumptions.
- **4.4 Robust display init**: query available VESA/EDID modes; pick a safe default; fall back to
  Mode 13h/text when needed.

*Ship:* a USB/ISO that boots to the desktop on common machines and VMs.

### Phase 5 — Open-source / GitHub contribution readiness

Prepare the repo so others can understand, build, and contribute.

- **5.1 Docs**: a strong `README.md` (what QOS is, screenshots, quick start, the `run-qos.ps1`
  flow), architecture overview linking the ADRs, and a build matrix (Windows local, Docker).
- **5.2 Contribution scaffolding**: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, a `LICENSE`
  (review third-party/dependency licenses — bootloader, x86_64, etc.), issue/PR templates,
  and `CODEOWNERS`.
- **5.3 CI**: GitHub Actions that build the kernel + boot image and run `cargo test` for
  `qos-core`; a headless QEMU smoke test (boot → run a verify command → exit code).
- **5.4 Project hygiene**: `.gitignore` review, label scheme, a "good first issue" set, a
  roadmap/ROADMAP sync, and a tagged release with the bootable image attached.

*Ship:* a newcomer can clone, read, build, run, and open a meaningful PR.

### Phase 6 — Long tail

Multi-user/login, permissions enforcement, clipboard, settings, audio, USB stack, package
manager.

---

## Status

Phases 0, 1, and 2 are complete and verified in QEMU. Current focus: **Phase 3** (visible GUI),
then **Phase 4** (real-hardware portability), then **Phase 5** (open-source readiness).

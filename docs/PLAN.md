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

### Phase 2 — Real-OS core (the hard, essential part; parallelizable)

- **2.1 Preemptive scheduler + context switch** on the timer interrupt.
- **2.2 Reliable Ring 3** + a solid syscall ABI (build on `user.rs`/`asm_iretq_to_user`).
- **2.3 Per-process page tables** + basic memory protection.
- **2.4 Process lifecycle** (spawn/exit/wait) + one IPC primitive.

*Ship:* a user-space program runs isolated in Ring 3, preempted, and a crash kills only it.

### Phase 3 — Make it useful

- **3.1 Verify & harden the filesystem** (disk persistence, directories, metadata).
- **3.2 Verify networking** (ping/DNS round-trips); wire **`qosd` TLS proxy** for real cloud QPU.
- **3.3 GUI apps**: file manager + the **quantum circuit editor / job monitor** (our hook).
- **3.4 Kernel cutover**: kernel uses `qos-core` `JobManager` + QHAL (retire the 16-slot array).

### Phase 4 — Long tail

Multi-user/login, permissions enforcement, clipboard, settings, audio, USB, package manager.

---

## Suggested starting point

**Phase 0.1 (unified input event queue).** It is the shared prerequisite for the interactive
GUI (Phase 1) *and* clean program input later, and it is small and verifiable. From there,
Phase 1 makes the desktop clickable while Phase 2 (the real-OS core) proceeds in parallel.

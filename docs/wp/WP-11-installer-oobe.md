# WP-11: Installer & first-boot setup (OOBE) — language, user, disk, settings

- Status: 🟡 in progress
- Epic: E-90 (product/installation experience)
- ADRs: (new ADR when the on-disk config/install format stabilizes)
- Commits: (appended as delivered)

## Goal

Give QOS a real **installation / first-boot experience** like Windows or Linux: on an
unconfigured machine the system boots into a **setup wizard** — language (Türkçe/English),
user creation, disk selection/formatting, appearance — persists the choices to the system disk,
and on every later boot skips setup, applies the settings and greets the user. Longer term:
a genuine installer that copies QOS onto a target disk so the USB/ISO medium can be removed.

## Steps (verified slices)

- [ ] **Slice 1 — first-boot wizard + persisted system config.** Full-screen OOBE after the
  splash when no config exists: ① language (TR/EN — the wizard itself is bilingual),
  ② user name, ③ system disk (detected SATA disk shown with capacity; formats QOSFS if needed;
  falls back to RAM-only with a clear warning when no disk is attached), ④ theme (Dark/Light),
  ⑤ summary → install. Config stored as `system.cfg` (key=value) on the persistent disk via a
  new `sysconfig` module. Later boots: config loads, wizard skipped, theme applied, the top bar
  shows the user. Keyboard + mouse parity throughout.
- [ ] **Slice 2 — login & accounts.** Password at setup (salted hash on disk), a login screen
  after boot, lock (Win+L-style), multiple users.
- [ ] **Slice 3 — full i18n.** A string table for the whole desktop (apps, menus, hints) driven
  by the configured language; TR + EN complete; runtime switch in Settings.
- [ ] **Slice 4 — real installer.** Partition the target disk (GPT), create an ESP, copy the
  QOS boot image from the live medium onto it, register the boot entry — boot without the
  installation medium. Disk partitioning UI (choose disk, wipe/alongside).
- [ ] **Slice 5 — recovery & updates.** Re-run setup from Settings; reset to defaults; staged
  system image updates.

## Acceptance criteria

Per slice: QEMU two-boot proof (fresh disk → wizard runs; same disk again → wizard skipped and
settings applied), 0-fault boot, keyboard-only completion possible, WP/ADR updated. Slice 4:
boot from the installed disk with the install medium detached.

## Notes & gaps

- Slice 1 stores config on the QOSFS data disk (ADR-0018); no partitioning yet — that arrives
  with the real installer (slice 4).
- Passwordless in slice 1 (auth needs hashing + a login surface — slice 2); do not fake it.
- Settings currently shows config values; editing them in Settings lands with slice 3.

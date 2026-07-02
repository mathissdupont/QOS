# WP-09: VFS unification — one file tree over RAM fs, QOSFS and FAT

- Status: 🔴 not started
- Epic: E-40/E-41 (storage & filesystems)
- ADRs: ADR-0018 (AHCI storage); new ADR for the mount model
- Commits: (appended as delivered)

## Goal

Today three filesystems coexist with separate APIs and separate UI paths: the RAM `fs` (the
default tree), `diskfs`/QOSFS on AHCI (persistent, flat), and a read-only FAT16 module. Files
shows the disk as a special location and apps hardcode `disk:` prefixes. A real OS has **one
tree**: mount points, one path namespace, one API.

## Steps (planned slices)

- [ ] **Slice 1 — trait + mount table.** A `FileSystem` trait (read/write/list/create/remove/
  rename/metadata) implemented by RAM fs and diskfs; a mount table mapping path prefixes
  (`/`, `/disk`, later `/fat`) to filesystems; `vfs::` façade the whole kernel calls.
- [ ] **Slice 2 — adopt everywhere.** Files, Text Editor, QASM Studio, Terminal commands and the
  `qasm` runner drop `disk:` special-casing and use plain paths (`/disk/notes.txt`).
- [ ] **Slice 3 — QOSFS directories.** Give QOSFS real subdirectories (it is flat today) or
  bring FAT16 write support to parity, so the persistent volume matches the RAM tree.
- [ ] **Slice 4 — mount UX.** `mount` shell command; Files shows mounts as one tree with badges;
  System Monitor storage row lists mounts.

## Acceptance criteria

Per slice: the same file operations work through one API in QEMU; existing persistence tests
(two-boot survival) keep passing; UI paths show the unified namespace; 0-fault boot.

## Notes & gaps

- The existing `vfs.rs` is a stub to grow into slice 1 rather than a new module.
- Keep fallback-first: a missing disk must leave the RAM tree fully functional.

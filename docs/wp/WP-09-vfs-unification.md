# WP-09: VFS unification — one file tree over RAM fs, QOSFS and FAT

- Status: 🟡 in progress (slices 1–2 done)
- Epic: E-40/E-41 (storage & filesystems)
- ADRs: ADR-0018 (AHCI storage); ADR-0022 (VFS mount model)
- Issue: [#6](https://github.com/Heptapus-Open-Code-Organization/QOS/issues/6)
- Commits: 10f0427 (slice 1: FileSystem trait + mount table + facade, ADR-0022)

## Goal

Today three filesystems coexist with separate APIs and separate UI paths: the RAM `fs` (the
default tree), `diskfs`/QOSFS on AHCI (persistent, flat), and a read-only FAT16 module. Files
shows the disk as a special location and apps hardcode `disk:` prefixes. A real OS has **one
tree**: mount points, one path namespace, one API.

## Steps (planned slices)

- [x] **Slice 1 — trait + mount table (ADR-0022).** `FileSystem` trait (name/label/ready/
  supports_dirs/read/write/remove/mkdir/rename/exists/is_dir/entries/usage) implemented by
  RAM fs, QOSFS and (feature-gated) FAT16; static mount table with longest-prefix resolution
  (`/` → RAM tree, `/disk` → QOSFS, `/ram` compat alias); path normalization rejects invalid
  traversal; new `vfs::entries()` structured-listing API for UIs; cross-mount `rename` via
  copy+remove. **Verified in QEMU (3 boots, 0 PANIC):** unified `ls /` shows mounts + RAM
  tree; `cat readme.txt` reads the root=RAM tree; `mkfs` → `write /disk/wp09.txt` → `ls
  /disk` → `cat /disk/wp09.txt` all through the facade; `mkdir /docs` at the root; boot 2
  shows `/disk/wp09.txt  15 B` surviving reboot; desktop + Files unregressed.
- [x] **Slice 2 — GUI apps adopt the facade.** The desktop apps drop the app-level `disk:` prefix
  convention and route **all** file I/O through `crate::vfs` on plain paths. Files lists via
  `vfs::entries` (`/disk` for the persistent volume, `/ram[/cwd]` for the RAM tree — the explicit
  `/ram` mount avoids the virtual root's synthetic mount entries), and its create/rename/delete/
  preview/edit and the Text Editor + QASM Studio open/save all call `vfs::read/write/remove/mkdir/
  rename` with `/disk/<name>` or RAM paths (new `files_vfs_path` helper). The Files path label now
  reads `/disk` (was `disk:/`). **Verified in QEMU (2 boots, 0 PANIC, W^X clean):** Terminal
  `dformat`+`dsave readme.txt` seeds the disk; Files enters the Disk location and lists `readme.txt`
  via `vfs::entries(/disk)`, previews it via `vfs::read(/disk/readme.txt)`, and `n` creates
  `vfsproof.txt` via `vfs::write(/disk/vfsproof.txt)`; boot 2 (same data image) shows both
  `readme.txt 51 B` and `vfsproof.txt 0 B` surviving reboot — the GUI VFS-write persisted to QOSFS.
- [ ] **Slice 2b — Terminal path unification.** The compositor Terminal's generic commands
  (`ls`/`cat`/`cd`/`mkdir`/`touch`/`rm`/`write`) still resolve to RAM-only relative paths and the
  disk lives behind separate `d*` commands (`dls`/`dcat`/`dsave`/`dload`/`dformat`). Rework
  `resolve()` to preserve absolute VFS paths and route through `crate::vfs` so `ls /disk`,
  `cat /disk/notes.txt`, etc. work uniformly; keep `dformat` (a filesystem-admin op with no path
  form) and retire the other `d*` aliases. (Admin ops `mkfs`/`is_formatted` stay on `diskfs` until
  slice 4's mount API.)
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

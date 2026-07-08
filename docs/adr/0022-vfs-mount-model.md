# ADR-0022: VFS mount model — one path namespace over a `FileSystem` trait

- **Status:** Accepted
- **Date:** 2026-07-08
- **Deciders:** Samet Unsal, Claude
- **Related ADRs:** ADR-0018 (AHCI/QOSFS storage)

## Context

QOS grew three filesystems with three APIs: the RAM `fs` tree (default), `diskfs`/QOSFS on
AHCI (persistent, flat), and an optional read-only FAT16 module. The old `vfs.rs` dispatched
over a hard-coded `Mount` enum with per-filesystem `match` arms in every operation, and apps
carried `disk:` prefixes (Files, Text Editor, QASM Studio, Terminal). Every new filesystem
(FAT write parity, USB mass storage, future NVMe volumes) would have meant touching every
operation in the facade and every app. A real OS has one tree: mount points, one path
namespace, one kernel-facing API (WP-09, issue #6).

Constraints: `no_std + alloc`; fallback-first (a missing/unformatted disk must leave the RAM
tree fully usable); existing callers (shell, syscalls, desktop apps) use the facade functions
`read/write/remove/mkdir/list_dir/copy` and must keep working unchanged.

## Decision

Introduce a `FileSystem` trait (`name/label/ready/supports_dirs/read/write/remove/mkdir/
rename/exists/is_dir/entries/usage`) and a static mount table of `(prefix, &'static dyn
FileSystem)` entries resolved by longest-prefix match. The facade normalizes the path
(`.`/`..`), resolves it to `(filesystem, mount-relative path)`, and calls the trait — no
per-filesystem knowledge remains in the facade or in callers.

Mount layout: `/` → RAM fs (the root tree), `/disk` → QOSFS, `/fat` → FAT16 (feature-gated);
`/ram` is kept as a compatibility alias of `/`. Relative paths keep addressing the RAM fs
directly (historical shell behaviour). The virtual root lists the RAM tree plus one synthetic
directory per non-root mount.

## Rationale

- **Open for extension:** USB storage (#22), NVMe volumes, and FAT write parity become one
  trait impl + one mount-table row, not edits across the kernel.
- **Root = RAM tree** (instead of a mounts-only virtual root) makes `/notes.txt`,
  `mkdir /docs`, and future `/home` semantics natural — one tree like Linux/Windows, and it
  is strictly more permissive than the old behaviour (which rejected such paths), so nothing
  breaks.
- **Longest-prefix match** is the standard, predictable resolution rule and costs one linear
  scan over a tiny table.
- **`ready()` on the trait** keeps fallback-first: an unformatted disk yields
  `VfsError::NotFormatted` from that mount only; `/` stays fully functional.

## Consequences

### Positive

- One API for the whole kernel; `vfs::entries()` gives UIs structured listings (Files can
  drop `disk:` special-casing in WP-09 slice 2).
- Cross-mount `rename` falls back to copy+delete transparently.
- Verified in QEMU (three boots, no PANIC): unified `ls /` (mounts + RAM tree), root-relative
  RAM reads, `mkfs` → `write`/`ls`/`cat` on `/disk` through the facade, `mkdir /docs` at the
  root, and two-boot persistence of `/disk/wp09.txt`.

### Negative / Trade-offs

- `resolve()` allocates a small `Vec` per call for the relative path (bounded by the 128-byte
  normalize buffer) — acceptable; can become borrow-based if profiling ever cares.
- QOSFS `rename` is copy+remove (not atomic) until the on-disk format gains directories/rename
  (WP-09 slice 3, crash-consistency in issue #27).
- FAT16 `entries()` returns `NotSupported` (the module only exposes a printing `list()`); FAT
  stays feature-gated and read-mostly until slice 3.

### Neutral / Follow-ups

- WP-09 slice 2: move Files/Text Editor/QASM Studio/Terminal `disk:` prefixes to plain
  `/disk/...` paths via the facade.
- The mount table is static today; a dynamic `mount`/`umount` command is WP-09 slice 4 and
  will require interior mutability (spin-locked table) when it lands.
- Permission enforcement (issue #24) gets a single choke point: the facade.

## Alternatives considered

1. **Keep the enum dispatch, just add arms** — every new filesystem touches every operation;
   no structured listing API; apps keep prefix hacks. Rejected as accumulating debt.
2. **Full inode/dentry VFS (Linux-style)** — overkill for three filesystems and would stall
   WP-09; the trait facade preserves the option to grow toward it later.
3. **Mounts-only virtual root (no RAM tree at `/`)** — keeps `/` artificial, blocks natural
   root paths, and matches no mainstream OS user model. Rejected.

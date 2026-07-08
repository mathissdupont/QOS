//! Virtual File System — one path namespace over multiple filesystems.
//!
//! WP-09 slice 1 (ADR-0022): a `FileSystem` trait implemented by each backing store and a
//! mount table mapping path prefixes to filesystems. The whole kernel talks to the facade
//! functions at the bottom of this file; no caller needs to know which filesystem serves a
//! path.
//!
//! Mount layout:
//! - `/`      → RAM filesystem (the root tree; also aliased at `/ram` for compatibility)
//! - `/disk`  → persistent QOSFS on AHCI (flat namespace today)
//! - `/fat`   → FAT16 (optional, behind the `fat` cargo feature)
//!
//! Paths are absolute (`/...`); `.` and `..` are resolved by [`normalize_path`]. Relative
//! paths (no leading `/`) address the RAM filesystem directly — the historical shell
//! behaviour, kept so existing callers don't break. A missing/unformatted disk leaves the
//! RAM tree fully functional (fallback-first).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// VFS error types with descriptive messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsError {
    /// Path is empty, malformed, or invalid
    BadPath,
    /// File or directory not found
    NotFound,
    /// I/O error during read/write
    Io,
    /// Filesystem not formatted (disk)
    NotFormatted,
    /// File too large for filesystem
    TooLarge,
    /// Path is a directory when file expected (or vice versa)
    IsDirectory,
    /// Directory is not empty
    NotEmpty,
    /// No free slots available
    NoSpace,
    /// Operation not supported
    NotSupported,
}

impl VfsError {
    /// Get a human-readable error message
    pub fn message(&self) -> &'static str {
        match self {
            VfsError::BadPath => "invalid or malformed path",
            VfsError::NotFound => "file or directory not found",
            VfsError::Io => "I/O error",
            VfsError::NotFormatted => "filesystem not formatted (run mkfs)",
            VfsError::TooLarge => "file too large",
            VfsError::IsDirectory => "path is a directory",
            VfsError::NotEmpty => "directory is not empty",
            VfsError::NoSpace => "no space available",
            VfsError::NotSupported => "operation not supported",
        }
    }
}

/// One directory entry as reported through the VFS.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: usize,
}

/// The contract every mounted filesystem implements. Paths handed to these methods are
/// *relative to the mount point* with no leading slash (`""` = the mount root).
pub trait FileSystem: Sync {
    /// Short identifier for logs/UI ("ramfs", "qosfs", "fat16").
    fn name(&self) -> &'static str;
    /// Human-readable description for mount listings.
    fn label(&self) -> &'static str;
    /// Is the backing store present and usable right now?
    fn ready(&self) -> bool;
    /// Does this filesystem support subdirectories?
    fn supports_dirs(&self) -> bool;
    fn read(&self, rel: &[u8]) -> Result<Vec<u8>, VfsError>;
    fn write(&self, rel: &[u8], data: &[u8]) -> Result<(), VfsError>;
    fn remove(&self, rel: &[u8]) -> Result<(), VfsError>;
    fn mkdir(&self, rel: &[u8]) -> Result<(), VfsError>;
    fn rename(&self, from: &[u8], to: &[u8]) -> Result<(), VfsError>;
    fn exists(&self, rel: &[u8]) -> bool;
    fn is_dir(&self, rel: &[u8]) -> bool;
    fn entries(&self, rel: &[u8]) -> Result<Vec<Entry>, VfsError>;
    /// (used, capacity) in bytes, when the filesystem can report it.
    fn usage(&self) -> Option<(usize, usize)> {
        None
    }
}

// ---------------------------------------------------------------------------
// RAM filesystem adapter (crate::fs)
// ---------------------------------------------------------------------------

struct RamFs;

fn map_ramfs_err(e: &'static str) -> VfsError {
    match e {
        "not found" | "parent not found" => VfsError::NotFound,
        "too large" => VfsError::TooLarge,
        "bad name" | "already exists" | "parent is not a directory" => VfsError::BadPath,
        "no slots" => VfsError::NoSpace,
        "is a directory" => VfsError::IsDirectory,
        "directory not empty" => VfsError::NotEmpty,
        _ => VfsError::Io,
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &'static str {
        "ramfs"
    }
    fn label(&self) -> &'static str {
        "RAM filesystem"
    }
    fn ready(&self) -> bool {
        true
    }
    fn supports_dirs(&self) -> bool {
        true
    }
    fn read(&self, rel: &[u8]) -> Result<Vec<u8>, VfsError> {
        crate::fs::read(rel).ok_or(VfsError::NotFound)
    }
    fn write(&self, rel: &[u8], data: &[u8]) -> Result<(), VfsError> {
        crate::fs::write(rel, data).map_err(map_ramfs_err)
    }
    fn remove(&self, rel: &[u8]) -> Result<(), VfsError> {
        crate::fs::remove(rel).map_err(map_ramfs_err)
    }
    fn mkdir(&self, rel: &[u8]) -> Result<(), VfsError> {
        crate::fs::mkdir(rel).map_err(map_ramfs_err)
    }
    fn rename(&self, from: &[u8], to: &[u8]) -> Result<(), VfsError> {
        crate::fs::rename(from, to).map_err(map_ramfs_err)
    }
    fn exists(&self, rel: &[u8]) -> bool {
        crate::fs::exists(rel)
    }
    fn is_dir(&self, rel: &[u8]) -> bool {
        crate::fs::is_dir(rel)
    }
    fn entries(&self, rel: &[u8]) -> Result<Vec<Entry>, VfsError> {
        if !rel.is_empty() && !crate::fs::is_dir(rel) {
            return if crate::fs::exists(rel) {
                Err(VfsError::BadPath)
            } else {
                Err(VfsError::NotFound)
            };
        }
        Ok(crate::fs::get_entries(rel)
            .into_iter()
            .map(|(name, is_dir, size)| Entry { name, is_dir, size })
            .collect())
    }
    fn usage(&self) -> Option<(usize, usize)> {
        Some((crate::fs::used_space(), crate::fs::total_capacity()))
    }
}

// ---------------------------------------------------------------------------
// Persistent QOSFS adapter (crate::diskfs over AHCI) — flat namespace today
// ---------------------------------------------------------------------------

struct QosFs;

impl FileSystem for QosFs {
    fn name(&self) -> &'static str {
        "qosfs"
    }
    fn label(&self) -> &'static str {
        "persistent disk (QOSFS/SATA)"
    }
    fn ready(&self) -> bool {
        crate::diskfs::is_formatted()
    }
    fn supports_dirs(&self) -> bool {
        false
    }
    fn read(&self, rel: &[u8]) -> Result<Vec<u8>, VfsError> {
        if !self.ready() {
            return Err(VfsError::NotFormatted);
        }
        crate::diskfs::read(rel).ok_or(VfsError::NotFound)
    }
    fn write(&self, rel: &[u8], data: &[u8]) -> Result<(), VfsError> {
        crate::diskfs::write(rel, data).map_err(|e| match e {
            "not formatted" => VfsError::NotFormatted,
            "too large" => VfsError::TooLarge,
            "bad name" => VfsError::BadPath,
            "no slots" => VfsError::NoSpace,
            _ => VfsError::Io,
        })
    }
    fn remove(&self, rel: &[u8]) -> Result<(), VfsError> {
        if !self.ready() {
            return Err(VfsError::NotFormatted);
        }
        if crate::diskfs::remove(rel) {
            Ok(())
        } else {
            Err(VfsError::NotFound)
        }
    }
    fn mkdir(&self, _rel: &[u8]) -> Result<(), VfsError> {
        Err(VfsError::NotSupported)
    }
    fn rename(&self, from: &[u8], to: &[u8]) -> Result<(), VfsError> {
        // Flat namespace: copy + remove is atomic enough for slice 1 (WP-09 slice 3 brings
        // real directories/rename to the on-disk format).
        let data = self.read(from)?;
        self.write(to, &data)?;
        self.remove(from)
    }
    fn exists(&self, rel: &[u8]) -> bool {
        self.ready() && crate::diskfs::read(rel).is_some()
    }
    fn is_dir(&self, _rel: &[u8]) -> bool {
        false
    }
    fn entries(&self, rel: &[u8]) -> Result<Vec<Entry>, VfsError> {
        if !self.ready() {
            return Err(VfsError::NotFormatted);
        }
        if !rel.is_empty() {
            return Err(VfsError::NotSupported); // flat: no subdirectories yet
        }
        Ok(crate::diskfs::get_entries(b"")
            .into_iter()
            .map(|(name, is_dir, size)| Entry { name, is_dir, size })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// FAT16 adapter (optional)
// ---------------------------------------------------------------------------

#[cfg(feature = "fat")]
struct Fat16Fs;

#[cfg(feature = "fat")]
impl FileSystem for Fat16Fs {
    fn name(&self) -> &'static str {
        "fat16"
    }
    fn label(&self) -> &'static str {
        "FAT16 filesystem"
    }
    fn ready(&self) -> bool {
        crate::fat16::is_fat16()
    }
    fn supports_dirs(&self) -> bool {
        false // read paths only address the root directory today
    }
    fn read(&self, rel: &[u8]) -> Result<Vec<u8>, VfsError> {
        if !self.ready() {
            return Err(VfsError::NotFormatted);
        }
        crate::fat16::read(rel).ok_or(VfsError::NotFound)
    }
    fn write(&self, rel: &[u8], data: &[u8]) -> Result<(), VfsError> {
        if !self.ready() {
            return Err(VfsError::NotFormatted);
        }
        crate::fat16::write(rel, data).map_err(|e| match e {
            "FAT16 not mounted" => VfsError::NotFormatted,
            _ => VfsError::Io,
        })
    }
    fn remove(&self, rel: &[u8]) -> Result<(), VfsError> {
        if !self.ready() {
            return Err(VfsError::NotFormatted);
        }
        if crate::fat16::remove(rel) {
            Ok(())
        } else {
            Err(VfsError::NotFound)
        }
    }
    fn mkdir(&self, _rel: &[u8]) -> Result<(), VfsError> {
        Err(VfsError::NotSupported)
    }
    fn rename(&self, _from: &[u8], _to: &[u8]) -> Result<(), VfsError> {
        Err(VfsError::NotSupported)
    }
    fn exists(&self, rel: &[u8]) -> bool {
        if !self.ready() {
            return false;
        }
        let name = core::str::from_utf8(rel).unwrap_or("");
        crate::fat16::Fat16::new().and_then(|fs| fs.stat(name)).is_some()
    }
    fn is_dir(&self, rel: &[u8]) -> bool {
        if !self.ready() {
            return false;
        }
        let name = core::str::from_utf8(rel).unwrap_or("");
        crate::fat16::Fat16::new()
            .and_then(|fs| fs.stat(name))
            .map(|info| info.is_dir)
            .unwrap_or(false)
    }
    fn entries(&self, _rel: &[u8]) -> Result<Vec<Entry>, VfsError> {
        Err(VfsError::NotSupported) // fat16 exposes only a printing list() today
    }
}

// ---------------------------------------------------------------------------
// Mount table
// ---------------------------------------------------------------------------

/// One mount: a path prefix and the filesystem serving it.
pub struct MountPoint {
    /// Absolute prefix without a trailing slash (`/` for the root mount).
    pub prefix: &'static [u8],
    pub fs: &'static dyn FileSystem,
}

static RAMFS: RamFs = RamFs;
static QOSFS: QosFs = QosFs;
#[cfg(feature = "fat")]
static FATFS: Fat16Fs = Fat16Fs;

#[cfg(not(feature = "fat"))]
static MOUNTS: [MountPoint; 3] = [
    MountPoint { prefix: b"/ram", fs: &RAMFS },
    MountPoint { prefix: b"/disk", fs: &QOSFS },
    MountPoint { prefix: b"/", fs: &RAMFS },
];
#[cfg(feature = "fat")]
static MOUNTS: [MountPoint; 4] = [
    MountPoint { prefix: b"/ram", fs: &RAMFS },
    MountPoint { prefix: b"/disk", fs: &QOSFS },
    MountPoint { prefix: b"/fat", fs: &FATFS },
    MountPoint { prefix: b"/", fs: &RAMFS },
];

/// The active mount table (longest-prefix match decides which mount serves a path).
pub fn mounts() -> &'static [MountPoint] {
    &MOUNTS
}

/// Does normalized absolute path `p` live under mount `prefix`?
fn prefix_matches(p: &[u8], prefix: &[u8]) -> bool {
    if prefix == b"/" {
        return true; // root mount catches everything absolute
    }
    p == prefix || (p.len() > prefix.len() && p.starts_with(prefix) && p[prefix.len()] == b'/')
}

/// Resolve a path to (filesystem, mount-relative path). Relative paths address the RAM fs
/// directly (historical shell behaviour); absolute paths are normalized then matched against
/// the mount table by longest prefix.
fn resolve(path: &[u8]) -> Result<(&'static dyn FileSystem, Vec<u8>), VfsError> {
    if path.is_empty() {
        return Err(VfsError::BadPath);
    }
    if path[0] != b'/' {
        return Ok((&RAMFS, path.to_vec()));
    }
    let mut norm = [0u8; 128];
    let len = normalize_path(path, &mut norm).ok_or(VfsError::BadPath)?;
    let p = &norm[..len];

    let mut best: Option<&MountPoint> = None;
    for m in mounts().iter() {
        if prefix_matches(p, m.prefix)
            && best.map_or(true, |b| m.prefix.len() > b.prefix.len())
        {
            best = Some(m);
        }
    }
    let m = best.ok_or(VfsError::BadPath)?;
    let cut = if m.prefix == b"/" { 1 } else { m.prefix.len() };
    let rel = if p.len() > cut {
        let r = &p[cut..];
        if r[0] == b'/' { &r[1..] } else { r }
    } else {
        b"" as &[u8]
    };
    Ok((m.fs, rel.to_vec()))
}

/// Normalize path by resolving `.` and `..` components
/// Returns normalized path in provided buffer, or None if invalid
pub fn normalize_path(path: &[u8], out: &mut [u8; 128]) -> Option<usize> {
    if path.is_empty() || path[0] != b'/' {
        return None;
    }

    let mut components: Vec<&[u8]> = Vec::new();

    let mut i = 1;
    let mut start = 1;

    while i <= path.len() {
        if i == path.len() || path[i] == b'/' {
            let component = &path[start..i];
            if !component.is_empty() {
                if component == b"." {
                    // Current dir - skip
                } else if component == b".." {
                    // Parent dir - pop if possible
                    components.pop();
                } else {
                    components.push(component);
                }
            }
            start = i + 1;
        }
        i += 1;
    }

    // Build normalized path
    let mut len = 0;
    out[len] = b'/';
    len += 1;

    for (idx, comp) in components.iter().enumerate() {
        if idx > 0 {
            if len >= out.len() {
                return None;
            }
            out[len] = b'/';
            len += 1;
        }
        if len + comp.len() > out.len() {
            return None;
        }
        out[len..len + comp.len()].copy_from_slice(comp);
        len += comp.len();
    }

    Some(len)
}

// ---------------------------------------------------------------------------
// Facade — the API the rest of the kernel calls
// ---------------------------------------------------------------------------

/// Is `path` exactly the virtual root (`/`, empty, or `.`)?
fn is_root(path: &[u8]) -> bool {
    if path.is_empty() || path == b"." || path == b"/" {
        return true;
    }
    if path[0] == b'/' {
        let mut norm = [0u8; 128];
        if let Some(len) = normalize_path(path, &mut norm) {
            return &norm[..len] == b"/";
        }
    }
    false
}

/// List a directory as structured entries. The virtual root shows the RAM tree plus one
/// synthetic directory per non-root mount (`disk`, `fat`).
pub fn entries(path: &[u8]) -> Result<Vec<Entry>, VfsError> {
    if is_root(path) {
        let mut list = RAMFS.entries(b"")?;
        for m in mounts().iter() {
            if m.prefix == b"/" || m.prefix == b"/ram" {
                continue; // the root *is* the RAM tree; /ram is only a compat alias
            }
            list.push(Entry {
                name: String::from_utf8_lossy(&m.prefix[1..]).into_owned(),
                is_dir: true,
                size: 0,
            });
        }
        return Ok(list);
    }
    let (fs, rel) = resolve(path)?;
    fs.entries(&rel)
}

/// List directory contents to the console.
pub fn list_dir(path: &[u8]) -> Result<(), VfsError> {
    if is_root(path) {
        for m in mounts().iter() {
            if m.prefix == b"/" {
                continue;
            }
            let status = if m.fs.ready() { "" } else { "  (not ready)" };
            crate::println!(
                "  {}/  [{}: {}]{}",
                core::str::from_utf8(&m.prefix[1..]).unwrap_or("?"),
                m.fs.name(),
                m.fs.label(),
                status
            );
        }
        for e in RAMFS.entries(b"")? {
            print_entry(&e);
        }
        return Ok(());
    }
    let ents = entries(path)?;
    if ents.is_empty() {
        crate::println!("  (empty)");
    }
    for e in ents {
        print_entry(&e);
    }
    Ok(())
}

fn print_entry(e: &Entry) {
    if e.is_dir {
        crate::println!("  {}/", e.name);
    } else {
        crate::println!("  {}  {} B", e.name, e.size);
    }
}

/// Create a directory
pub fn mkdir(path: &[u8]) -> Result<(), VfsError> {
    let (fs, rel) = resolve(path)?;
    if rel.is_empty() {
        return Err(VfsError::BadPath);
    }
    fs.mkdir(&rel)
}

/// Check if path exists
pub fn exists(path: &[u8]) -> bool {
    if is_root(path) {
        return true;
    }
    match resolve(path) {
        Ok((fs, rel)) => rel.is_empty() || fs.exists(&rel),
        Err(_) => false,
    }
}

/// Check if path is a directory
pub fn is_dir(path: &[u8]) -> bool {
    if is_root(path) {
        return true;
    }
    match resolve(path) {
        Ok((fs, rel)) => rel.is_empty() || fs.is_dir(&rel),
        Err(_) => false,
    }
}

/// Read file contents
pub fn read(path: &[u8]) -> Result<Vec<u8>, VfsError> {
    let (fs, rel) = resolve(path)?;
    if rel.is_empty() {
        return Err(VfsError::IsDirectory);
    }
    fs.read(&rel)
}

/// Write file contents
pub fn write(path: &[u8], data: &[u8]) -> Result<(), VfsError> {
    let (fs, rel) = resolve(path)?;
    if rel.is_empty() {
        return Err(VfsError::IsDirectory);
    }
    fs.write(&rel, data)
}

/// Remove file or empty directory
pub fn remove(path: &[u8]) -> Result<(), VfsError> {
    let (fs, rel) = resolve(path)?;
    if rel.is_empty() {
        return Err(VfsError::NotSupported); // cannot remove a mount point
    }
    fs.remove(&rel)
}

/// Rename/move within one filesystem; falls back to copy+remove across mounts.
pub fn rename(old: &[u8], new: &[u8]) -> Result<(), VfsError> {
    let (fs_a, rel_a) = resolve(old)?;
    let (fs_b, rel_b) = resolve(new)?;
    if rel_a.is_empty() || rel_b.is_empty() {
        return Err(VfsError::BadPath);
    }
    if core::ptr::eq(fs_a as *const dyn FileSystem, fs_b as *const dyn FileSystem) {
        fs_a.rename(&rel_a, &rel_b)
    } else {
        let data = fs_a.read(&rel_a)?;
        fs_b.write(&rel_b, &data)?;
        fs_a.remove(&rel_a)
    }
}

/// Copy file from source to destination
pub fn copy(src: &[u8], dst: &[u8]) -> Result<(), VfsError> {
    let bytes = read(src)?;
    write(dst, &bytes)?;
    Ok(())
}

/// Get file size (returns None if not found or is directory)
pub fn file_size(path: &[u8]) -> Option<usize> {
    read(path).ok().map(|v| v.len())
}

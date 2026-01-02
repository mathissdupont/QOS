//! Virtual File System layer for QOS
//!
//! Provides unified path-based access to multiple mount points:
//! - `/ram` - RAM-based temporary filesystem
//! - `/disk` - Persistent disk filesystem (QOS-FS format)
//!
//! All paths must be absolute (starting with `/`) and can include
//! `.` (current dir) and `..` (parent dir) components.

extern crate alloc;

use alloc::vec::Vec;

/// Mount point identifiers
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mount {
    Ram,
    Disk,
    #[cfg(feature = "fat")]
    Fat,
}

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

/// Normalize path by resolving `.` and `..` components
/// Returns normalized path in provided buffer, or None if invalid
pub fn normalize_path(path: &[u8], out: &mut [u8; 128]) -> Option<usize> {
    if path.is_empty() || path[0] != b'/' {
        return None;
    }

    let mut components: Vec<&[u8]> = Vec::new();
    
    // Skip leading slash and split by '/'
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
            if len >= out.len() { return None; }
            out[len] = b'/';
            len += 1;
        }
        if len + comp.len() > out.len() { return None; }
        out[len..len + comp.len()].copy_from_slice(comp);
        len += comp.len();
    }
    
    // Root case
    if len == 0 {
        out[0] = b'/';
        len = 1;
    }
    
    Some(len)
}

/// Split path into mount point and relative path within mount
fn split_mount(path: &[u8]) -> Result<(Mount, &[u8]), VfsError> {
    if path.is_empty() {
        return Err(VfsError::BadPath);
    }

    // Non-absolute paths are treated as relative to /ram
    if path[0] != b'/' {
        return Ok((Mount::Ram, path));
    }

    // Root directory - special case
    if path == b"/" {
        return Err(VfsError::BadPath);
    }

    // Check mount points
    if path.starts_with(b"/ram/") {
        let name = &path[5..];
        if name.is_empty() {
            return Err(VfsError::BadPath);
        }
        return Ok((Mount::Ram, name));
    }

    if path.starts_with(b"/disk/") {
        let name = &path[6..];
        if name.is_empty() {
            return Err(VfsError::BadPath);
        }
        return Ok((Mount::Disk, name));
    }

    #[cfg(feature = "fat")]
    if path.starts_with(b"/fat/") {
        let name = &path[5..];
        if name.is_empty() {
            return Err(VfsError::BadPath);
        }
        return Ok((Mount::Fat, name));
    }

    // Check if path IS a mount point
    if path == b"/ram" || path == b"/disk" {
        return Err(VfsError::IsDirectory);
    }
    
    #[cfg(feature = "fat")]
    if path == b"/fat" {
        return Err(VfsError::IsDirectory);
    }

    Err(VfsError::BadPath)
}

/// List directory contents
pub fn list_dir(path: &[u8]) -> Result<(), VfsError> {
    // Normalize the path first
    let mut norm = [0u8; 128];
    let path = if path.is_empty() || path == b"." {
        b"/" as &[u8]
    } else if path[0] != b'/' {
        // For relative paths, just use as-is for now
        path
    } else if let Some(len) = normalize_path(path, &mut norm) {
        &norm[..len]
    } else {
        return Err(VfsError::BadPath);
    };

    match path {
        b"/" => {
            crate::println!("mounts:");
            crate::println!("  /ram     (RAM filesystem)");
            crate::println!("  /disk    (persistent disk)");
            #[cfg(feature = "fat")]
            {
                if crate::fat16::is_fat16() {
                    crate::println!("  /fat     (FAT16 filesystem)");
                }
            }
            Ok(())
        }
        b"/ram" => {
            crate::fs::list_dir(b"");
            Ok(())
        }
        b"/disk" => {
            if !crate::diskfs::is_formatted() {
                crate::println!("error: disk not formatted (run mkfs)");
                return Err(VfsError::NotFormatted);
            }
            if crate::diskfs::list() {
                Ok(())
            } else {
                Err(VfsError::Io)
            }
        }
        #[cfg(feature = "fat")]
        b"/fat" => {
            if !crate::fat16::is_fat16() {
                crate::println!("error: FAT16 filesystem not available");
                return Err(VfsError::NotFormatted);
            }
            crate::fat16::list();
            Ok(())
        }
        _ if path.starts_with(b"/ram/") => {
            let subpath = &path[5..];
            crate::fs::list_dir(subpath);
            Ok(())
        }
        _ if path.starts_with(b"/disk/") => {
            // Disk FS doesn't support subdirectories yet
            crate::println!("error: disk filesystem does not support subdirectories");
            Err(VfsError::NotSupported)
        }
        #[cfg(feature = "fat")]
        _ if path.starts_with(b"/fat/") => {
            // FAT16 subdirectory listing not implemented
            crate::println!("error: FAT16 subdirectory listing not yet implemented");
            Err(VfsError::NotSupported)
        }
        _ => Err(VfsError::BadPath),
    }
}

/// Create a directory
pub fn mkdir(path: &[u8]) -> Result<(), VfsError> {
    let (m, name) = split_mount(path)?;
    match m {
        Mount::Ram => crate::fs::mkdir(name).map_err(|e| match e {
            "parent not found" => VfsError::NotFound,
            "already exists" => VfsError::BadPath,
            "bad name" => VfsError::BadPath,
            "no slots" => VfsError::NoSpace,
            _ => VfsError::Io,
        }),
        Mount::Disk => {
            crate::println!("error: mkdir not supported on disk filesystem");
            Err(VfsError::NotSupported)
        }
        #[cfg(feature = "fat")]
        Mount::Fat => {
            crate::println!("error: mkdir not supported on FAT16 filesystem (read-only for now)");
            Err(VfsError::NotSupported)
        }
    }
}

/// Check if path exists
pub fn exists(path: &[u8]) -> bool {
    // Mount points always exist
    if path == b"/" || path == b"/ram" || path == b"/disk" {
        return true;
    }
    
    #[cfg(feature = "fat")]
    if path == b"/fat" {
        return crate::fat16::is_fat16();
    }
    
    if let Ok((m, name)) = split_mount(path) {
        match m {
            Mount::Ram => crate::fs::exists(name),
            Mount::Disk => {
                if !crate::diskfs::is_formatted() {
                    return false;
                }
                crate::diskfs::read(name).is_some()
            }
            #[cfg(feature = "fat")]
            Mount::Fat => {
                let name_str = core::str::from_utf8(name).unwrap_or("");
                if let Some(fs) = crate::fat16::Fat16::new() {
                    fs.stat(name_str).is_some()
                } else {
                    false
                }
            }
        }
    } else {
        false
    }
}

/// Check if path is a directory
pub fn is_dir(path: &[u8]) -> bool {
    // Mount points are directories
    if path == b"/" || path == b"/ram" || path == b"/disk" {
        return true;
    }
    
    #[cfg(feature = "fat")]
    if path == b"/fat" {
        return crate::fat16::is_fat16();
    }
    
    if let Ok((m, name)) = split_mount(path) {
        match m {
            Mount::Ram => crate::fs::is_dir(name),
            Mount::Disk => false, // Disk FS is flat
            #[cfg(feature = "fat")]
            Mount::Fat => {
                let name_str = core::str::from_utf8(name).unwrap_or("");
                if let Some(fs) = crate::fat16::Fat16::new() {
                    if let Some(info) = fs.stat(name_str) {
                        info.is_dir
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        }
    } else {
        false
    }
}

/// Read file contents
pub fn read(path: &[u8]) -> Result<Vec<u8>, VfsError> {
    let (m, name) = split_mount(path)?;
    match m {
        Mount::Ram => crate::fs::read(name).ok_or(VfsError::NotFound),
        Mount::Disk => {
            if !crate::diskfs::is_formatted() {
                return Err(VfsError::NotFormatted);
            }
            crate::diskfs::read(name).ok_or(VfsError::NotFound)
        }
        #[cfg(feature = "fat")]
        Mount::Fat => {
            if !crate::fat16::is_fat16() {
                return Err(VfsError::NotFormatted);
            }
            crate::fat16::read(name).ok_or(VfsError::NotFound)
        }
    }
}

/// Write file contents
pub fn write(path: &[u8], data: &[u8]) -> Result<(), VfsError> {
    let (m, name) = split_mount(path)?;
    match m {
        Mount::Ram => crate::fs::write(name, data).map_err(|e| match e {
            "too large" => VfsError::TooLarge,
            "bad name" => VfsError::BadPath,
            "no slots" => VfsError::NoSpace,
            "is a directory" => VfsError::IsDirectory,
            "parent not found" => VfsError::NotFound,
            "parent is not a directory" => VfsError::BadPath,
            _ => VfsError::Io,
        }),
        Mount::Disk => {
            crate::diskfs::write(name, data).map_err(|e| match e {
                "not formatted" => VfsError::NotFormatted,
                "too large" => VfsError::TooLarge,
                "bad name" => VfsError::BadPath,
                "no slots" => VfsError::NoSpace,
                _ => VfsError::Io,
            })
        }
        #[cfg(feature = "fat")]
        Mount::Fat => {
            if !crate::fat16::is_fat16() {
                return Err(VfsError::NotFormatted);
            }
            crate::fat16::write(name, data).map_err(|e| match e {
                "write failed" => VfsError::Io,
                "FAT16 not mounted" => VfsError::NotFormatted,
                _ => VfsError::Io,
            })
        }
    }
}

/// Remove file or empty directory
pub fn remove(path: &[u8]) -> Result<(), VfsError> {
    let (m, name) = split_mount(path)?;
    match m {
        Mount::Ram => {
            crate::fs::remove(name).map_err(|e| match e {
                "not found" => VfsError::NotFound,
                "directory not empty" => VfsError::NotEmpty,
                _ => VfsError::Io,
            })
        }
        Mount::Disk => {
            if !crate::diskfs::is_formatted() {
                return Err(VfsError::NotFormatted);
            }
            if crate::diskfs::remove(name) {
                Ok(())
            } else {
                Err(VfsError::NotFound)
            }
        }
        #[cfg(feature = "fat")]
        Mount::Fat => {
            if !crate::fat16::is_fat16() {
                return Err(VfsError::NotFormatted);
            }
            if crate::fat16::remove(name) {
                Ok(())
            } else {
                Err(VfsError::NotFound)
            }
        }
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

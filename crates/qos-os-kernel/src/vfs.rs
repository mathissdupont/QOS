extern crate alloc;

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mount {
    Ram,
    Disk,
}

#[derive(Debug)]
pub enum VfsError {
    BadPath,
    NotFound,
    Io,
    NotFormatted,
    TooLarge,
}

fn split_mount(path: &[u8]) -> Result<(Mount, &[u8]), VfsError> {
    // Supported:
    // - /ram/<name>
    // - /disk/<name>
    // Also allow bare names -> treated as ram.<name> for convenience.
    if path.is_empty() {
        return Err(VfsError::BadPath);
    }

    if path[0] != b'/' {
        return Ok((Mount::Ram, path));
    }

    // "/ram" or "/ram/.."
    if path == b"/" {
        return Err(VfsError::BadPath);
    }

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

    Err(VfsError::BadPath)
}

pub fn list_dir(path: &[u8]) -> Result<(), VfsError> {
    // Supported dirs:
    // - /          => show mounts
    // - /ram       => list ram files
    // - /ram/...   => list subdir in ram
    // - /disk      => list disk files
    match path {
        b"/" => {
            crate::println!("mounts:");
            crate::println!("  /ram");
            crate::println!("  /disk");
            Ok(())
        }
        b"/ram" => {
            crate::fs::list_dir(b"");
            Ok(())
        }
        b"/disk" => {
            if !crate::diskfs::is_formatted() {
                return Err(VfsError::NotFormatted);
            }
            if crate::diskfs::list() {
                Ok(())
            } else {
                Err(VfsError::Io)
            }
        }
        _ if path.starts_with(b"/ram/") => {
            let subpath = &path[5..];
            crate::fs::list_dir(subpath);
            Ok(())
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
            _ => VfsError::Io,
        }),
        Mount::Disk => Err(VfsError::Io), // Not supported on disk yet
    }
}

/// Check if path exists
pub fn exists(path: &[u8]) -> bool {
    if path == b"/" || path == b"/ram" || path == b"/disk" {
        return true;
    }
    if let Ok((m, name)) = split_mount(path) {
        match m {
            Mount::Ram => crate::fs::exists(name),
            Mount::Disk => false, // TODO
        }
    } else {
        false
    }
}

/// Check if path is a directory
pub fn is_dir(path: &[u8]) -> bool {
    if path == b"/" || path == b"/ram" || path == b"/disk" {
        return true;
    }
    if let Ok((m, name)) = split_mount(path) {
        match m {
            Mount::Ram => crate::fs::is_dir(name),
            Mount::Disk => false,
        }
    } else {
        false
    }
}

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
    }
}

pub fn write(path: &[u8], data: &[u8]) -> Result<(), VfsError> {
    let (m, name) = split_mount(path)?;
    match m {
        Mount::Ram => crate::fs::write(name, data).map_err(|_| VfsError::TooLarge),
        Mount::Disk => {
            crate::diskfs::write(name, data).map_err(|e| match e {
                "not formatted" => VfsError::NotFormatted,
                "too large" => VfsError::TooLarge,
                _ => VfsError::Io,
            })
        }
    }
}

pub fn remove(path: &[u8]) -> Result<(), VfsError> {
    let (m, name) = split_mount(path)?;
    match m {
        Mount::Ram => {
            crate::fs::remove(name).map_err(|e| match e {
                "not found" => VfsError::NotFound,
                "directory not empty" => VfsError::Io,
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
    }
}

pub fn copy(src: &[u8], dst: &[u8]) -> Result<(), VfsError> {
    let bytes = read(src)?;
    write(dst, &bytes)?;
    Ok(())
}

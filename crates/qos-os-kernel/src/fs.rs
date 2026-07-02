extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use lazy_static::lazy_static;
use spin::Mutex;

const MAX_ENTRIES: usize = 64;
const MAX_NAME: usize = 32;
const MAX_PATH: usize = 128;
const MAX_FILE_BYTES: usize = 64 * 1024;

/// File metadata - timestamps and attributes
#[derive(Clone, Copy, Debug)]
pub struct FileMetadata {
    pub created: u64,      // Unix timestamp (from RTC)
    pub modified: u64,     // Last modification time
    pub accessed: u64,     // Last access time
    pub permissions: u16,  // rwxrwxrwx (owner/group/other) - simplified
    pub hidden: bool,      // Hidden file flag
}

impl FileMetadata {
    pub fn new() -> Self {
        let now = crate::rtc::unix_timestamp();
        Self {
            created: now,
            modified: now,
            accessed: now,
            permissions: 0o644, // rw-r--r-- default for files
            hidden: false,
        }
    }
    
    pub fn new_dir() -> Self {
        let now = crate::rtc::unix_timestamp();
        Self {
            created: now,
            modified: now,
            accessed: now,
            permissions: 0o755, // rwxr-xr-x default for directories
            hidden: false,
        }
    }
    
    pub fn touch(&mut self) {
        self.modified = crate::rtc::unix_timestamp();
        self.accessed = self.modified;
    }
    
    pub fn access(&mut self) {
        self.accessed = crate::rtc::unix_timestamp();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryType {
    File,
    Directory,
}

#[derive(Clone)]
struct Entry {
    used: bool,
    entry_type: EntryType,
    name: [u8; MAX_NAME],
    name_len: usize,
    parent_path: [u8; MAX_PATH],
    parent_len: usize,
    data: Vec<u8>,
    metadata: FileMetadata,
}

impl Entry {
    fn empty() -> Self {
        Self {
            used: false,
            entry_type: EntryType::File,
            name: [0; MAX_NAME],
            name_len: 0,
            parent_path: [0; MAX_PATH],
            parent_len: 0,
            data: Vec::new(),
            metadata: FileMetadata::new(),
        }
    }

    fn name_bytes(&self) -> &[u8] {
        &self.name[..self.name_len]
    }

    fn parent_bytes(&self) -> &[u8] {
        &self.parent_path[..self.parent_len]
    }

    fn full_path(&self, buf: &mut [u8]) -> usize {
        let mut len = 0;
        if self.parent_len > 0 {
            buf[..self.parent_len].copy_from_slice(self.parent_bytes());
            len = self.parent_len;
            if len > 0 && buf[len - 1] != b'/' {
                buf[len] = b'/';
                len += 1;
            }
        }
        buf[len..len + self.name_len].copy_from_slice(self.name_bytes());
        len + self.name_len
    }
}

pub struct RamFs {
    entries: [Entry; MAX_ENTRIES],
}

impl RamFs {
    pub fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| Entry::empty()),
        }
    }

    /// Find entry by full path
    fn find_by_path(&self, path: &[u8]) -> Option<usize> {
        let mut path_buf = [0u8; MAX_PATH];
        for (i, e) in self.entries.iter().enumerate() {
            if !e.used {
                continue;
            }
            let len = e.full_path(&mut path_buf);
            if &path_buf[..len] == path {
                return Some(i);
            }
        }
        None
    }

    /// Find entry by name in a specific parent directory
    fn find_in_dir(&self, parent: &[u8], name: &[u8]) -> Option<usize> {
        for (i, e) in self.entries.iter().enumerate() {
            if e.used && e.parent_bytes() == parent && e.name_bytes() == name {
                return Some(i);
            }
        }
        None
    }

    fn find_free(&self) -> Option<usize> {
        self.entries.iter().position(|e| !e.used)
    }

    /// List entries in a directory
    pub fn list_dir(&self, dir_path: &[u8]) {
        let normalized = if dir_path.is_empty() { b"" as &[u8] } else { dir_path };
        
        crate::println!("contents of {}:", 
            if normalized.is_empty() { "/" } else { core::str::from_utf8(normalized).unwrap_or("?") });
        
        let mut count = 0;
        for e in self.entries.iter() {
            if !e.used {
                continue;
            }
            if e.parent_bytes() == normalized {
                let name = core::str::from_utf8(e.name_bytes()).unwrap_or("?");
                match e.entry_type {
                    EntryType::Directory => crate::println!("  [DIR]  {}/", name),
                    EntryType::File => crate::println!("  [FILE] {} ({} bytes)", name, e.data.len()),
                }
                count += 1;
            }
        }
        if count == 0 {
            crate::println!("  (empty)");
        }
    }

    /// Create a directory
    pub fn mkdir(&mut self, path: &[u8]) -> Result<(), &'static str> {
        let (parent, name) = self.split_path(path)?;
        
        if name.is_empty() || name.len() > MAX_NAME {
            return Err("bad name");
        }

        // Check parent exists (if not root)
        if !parent.is_empty() && self.find_by_path(parent).is_none() {
            return Err("parent not found");
        }

        // Check if already exists
        if self.find_in_dir(parent, name).is_some() {
            return Err("already exists");
        }

        let idx = self.find_free().ok_or("no slots")?;
        
        let mut entry = Entry::empty();
        entry.used = true;
        entry.entry_type = EntryType::Directory;
        entry.name_len = name.len();
        entry.name[..name.len()].copy_from_slice(name);
        entry.parent_len = parent.len();
        entry.parent_path[..parent.len()].copy_from_slice(parent);
        entry.metadata = FileMetadata::new_dir();
        self.entries[idx] = entry;
        Ok(())
    }
    
    /// Create directory with all parent directories (mkdir -p)
    pub fn mkdir_p(&mut self, path: &[u8]) -> Result<(), &'static str> {
        if path.is_empty() {
            return Ok(());
        }
        
        // Build path incrementally
        let mut current_path = Vec::new();
        let mut component = Vec::new();
        
        for &b in path.iter() {
            if b == b'/' {
                if !component.is_empty() {
                    if !current_path.is_empty() {
                        current_path.push(b'/');
                    }
                    current_path.extend_from_slice(&component);
                    
                    // Create this directory if it doesn't exist
                    if self.find_by_path(&current_path).is_none() {
                        let _ = self.mkdir(&current_path);
                    }
                    component.clear();
                }
            } else {
                component.push(b);
            }
        }
        
        // Handle final component
        if !component.is_empty() {
            if !current_path.is_empty() {
                current_path.push(b'/');
            }
            current_path.extend_from_slice(&component);
            
            if self.find_by_path(&current_path).is_none() {
                self.mkdir(&current_path)?;
            }
        }
        
        Ok(())
    }

    /// Remove a file or empty directory
    pub fn remove(&mut self, path: &[u8]) -> Result<(), &'static str> {
        let Some(i) = self.find_by_path(path) else {
            return Err("not found");
        };

        // If directory, check it's empty
        if self.entries[i].entry_type == EntryType::Directory {
            for e in self.entries.iter() {
                if e.used && e.parent_bytes() == path {
                    return Err("directory not empty");
                }
            }
        }

        self.entries[i] = Entry::empty();
        Ok(())
    }

    /// Write a file
    pub fn write(&mut self, path: &[u8], data: &[u8]) -> Result<(), &'static str> {
        let (parent, name) = self.split_path(path)?;
        
        if name.is_empty() || name.len() > MAX_NAME {
            return Err("bad name");
        }
        if data.len() > MAX_FILE_BYTES {
            return Err("too large");
        }

        // Check parent exists (if not root)
        if !parent.is_empty() {
            let parent_idx = self.find_by_path(parent).ok_or("parent not found")?;
            if self.entries[parent_idx].entry_type != EntryType::Directory {
                return Err("parent is not a directory");
            }
        }

        let idx = if let Some(i) = self.find_in_dir(parent, name) {
            if self.entries[i].entry_type == EntryType::Directory {
                return Err("is a directory");
            }
            i
        } else {
            self.find_free().ok_or("no slots")?
        };

        let mut entry = Entry::empty();
        entry.used = true;
        entry.entry_type = EntryType::File;
        entry.name_len = name.len();
        entry.name[..name.len()].copy_from_slice(name);
        entry.parent_len = parent.len();
        entry.parent_path[..parent.len()].copy_from_slice(parent);
        entry.data = data.to_vec();
        entry.metadata = FileMetadata::new();
        self.entries[idx] = entry;
        Ok(())
    }
    
    /// Append data to a file
    pub fn append(&mut self, path: &[u8], data: &[u8]) -> Result<(), &'static str> {
        let Some(i) = self.find_by_path(path) else {
            // If file doesn't exist, create it
            return self.write(path, data);
        };
        
        if self.entries[i].entry_type != EntryType::File {
            return Err("is a directory");
        }
        
        if self.entries[i].data.len() + data.len() > MAX_FILE_BYTES {
            return Err("too large");
        }
        
        self.entries[i].data.extend_from_slice(data);
        self.entries[i].metadata.touch();
        Ok(())
    }
    
    /// Get file/directory metadata
    pub fn get_metadata(&self, path: &[u8]) -> Option<(EntryType, usize, FileMetadata)> {
        let i = self.find_by_path(path)?;
        let e = &self.entries[i];
        Some((e.entry_type, e.data.len(), e.metadata))
    }
    
    /// Set file permissions
    pub fn chmod(&mut self, path: &[u8], permissions: u16) -> Result<(), &'static str> {
        let i = self.find_by_path(path).ok_or("not found")?;
        self.entries[i].metadata.permissions = permissions;
        Ok(())
    }
    
    /// Touch a file (update modification time, create if not exists)
    pub fn touch(&mut self, path: &[u8]) -> Result<(), &'static str> {
        if let Some(i) = self.find_by_path(path) {
            self.entries[i].metadata.touch();
            Ok(())
        } else {
            // Create empty file
            self.write(path, b"")
        }
    }
    
    /// Get total used space in bytes
    pub fn used_space(&self) -> usize {
        self.entries.iter()
            .filter(|e| e.used)
            .map(|e| e.data.len())
            .sum()
    }
    
    /// Get number of used entries
    pub fn used_entries(&self) -> usize {
        self.entries.iter().filter(|e| e.used).count()
    }
    
    /// Get total capacity
    pub fn total_capacity(&self) -> usize {
        MAX_ENTRIES * MAX_FILE_BYTES
    }
    
    /// Get directory size (recursive)
    pub fn dir_size(&self, path: &[u8]) -> usize {
        let prefix = if path.is_empty() {
            Vec::new()
        } else {
            let mut p = path.to_vec();
            if !p.ends_with(b"/") {
                p.push(b'/');
            }
            p
        };
        
        let mut total = 0;
        for e in self.entries.iter() {
            if !e.used {
                continue;
            }
            
            // Check if entry is in this directory or subdirectory
            let parent = e.parent_bytes();
            if path.is_empty() || parent == path || 
               (parent.len() > prefix.len() && parent.starts_with(&prefix)) ||
               e.parent_bytes() == path {
                total += e.data.len();
            }
        }
        total
    }
    
    /// Rename/move a file or directory
    pub fn rename(&mut self, old_path: &[u8], new_path: &[u8]) -> Result<(), &'static str> {
        let i = self.find_by_path(old_path).ok_or("not found")?;
        let (new_parent, new_name) = self.split_path(new_path)?;
        
        if new_name.is_empty() || new_name.len() > MAX_NAME {
            return Err("bad name");
        }
        
        // Check new parent exists
        if !new_parent.is_empty() && self.find_by_path(new_parent).is_none() {
            return Err("destination parent not found");
        }
        
        // Check destination doesn't exist
        if self.find_in_dir(new_parent, new_name).is_some() {
            return Err("destination exists");
        }
        
        // Update entry
        self.entries[i].name_len = new_name.len();
        self.entries[i].name[..new_name.len()].copy_from_slice(new_name);
        self.entries[i].parent_len = new_parent.len();
        self.entries[i].parent_path[..new_parent.len()].copy_from_slice(new_parent);
        self.entries[i].metadata.touch();
        
        Ok(())
    }
    
    /// Copy a file
    pub fn copy(&mut self, src: &[u8], dst: &[u8]) -> Result<(), &'static str> {
        let src_i = self.find_by_path(src).ok_or("source not found")?;
        
        if self.entries[src_i].entry_type != EntryType::File {
            return Err("can only copy files");
        }
        
        let data = self.entries[src_i].data.clone();
        self.write(dst, &data)
    }

    /// Read a file
    pub fn read(&self, path: &[u8]) -> Option<&[u8]> {
        let i = self.find_by_path(path)?;
        if self.entries[i].entry_type != EntryType::File {
            return None;
        }
        Some(self.entries[i].data.as_slice())
    }

    /// Check if path exists
    pub fn exists(&self, path: &[u8]) -> bool {
        self.find_by_path(path).is_some()
    }

    /// Check if path is a directory
    pub fn is_dir(&self, path: &[u8]) -> bool {
        if let Some(i) = self.find_by_path(path) {
            self.entries[i].entry_type == EntryType::Directory
        } else {
            false
        }
    }

    /// Split path into (parent, name)
    fn split_path<'a>(&self, path: &'a [u8]) -> Result<(&'a [u8], &'a [u8]), &'static str> {
        if path.is_empty() {
            return Err("empty path");
        }

        // Find last '/'
        let mut last_slash = None;
        for (i, &b) in path.iter().enumerate() {
            if b == b'/' {
                last_slash = Some(i);
            }
        }

        match last_slash {
            Some(i) => {
                let parent = &path[..i];
                let name = &path[i + 1..];
                Ok((parent, name))
            }
            None => Ok((b"", path)), // Root level
        }
    }

    pub fn read_string_lossy(&self, path: &[u8]) -> Option<String> {
        let bytes = self.read(path)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

lazy_static! {
    static ref FS: Mutex<RamFs> = Mutex::new(RamFs::new());
}

/// Seed a small demo tree so the shell (`ls`) and the GUI File Manager show real content on a
/// fresh boot. Idempotent-ish: safe to call once at startup.
pub fn seed_demo() {
    let mut fs = FS.lock();
    let _ = fs.mkdir(b"quantum");
    let _ = fs.mkdir(b"bin");
    let _ = fs.write(b"readme.txt", b"Welcome to QOS - a quantum-ready operating system.\n");
    let _ = fs.write(b"quantum/bell.qasm", b"OPENQASM 2.0;\nqreg q[2];\nh q[0];\ncx q[0],q[1];\n");
    let _ = fs.write(b"quantum/ghz.qasm", b"OPENQASM 2.0;\nqreg q[3];\nh q[0];\ncx q[0],q[1];\ncx q[1],q[2];\n");
    let _ = fs.write(
        b"quantum/rotate.qasm",
        b"OPENQASM 2.0;\nqreg q[2];\ncreg c[2];\nry(pi/3) q[0];\ncx q[0],q[1];\nrx(pi/2) q[1];\nmeasure q[0] -> c[0];\nmeasure q[1] -> c[1];\n",
    );
}

pub fn list() {
    FS.lock().list_dir(b"");
}

pub fn list_dir(path: &[u8]) {
    FS.lock().list_dir(path);
}

pub fn mkdir(path: &[u8]) -> Result<(), &'static str> {
    FS.lock().mkdir(path)
}

pub fn remove(path: &[u8]) -> Result<(), &'static str> {
    FS.lock().remove(path)
}

pub fn write(path: &[u8], data: &[u8]) -> Result<(), &'static str> {
    FS.lock().write(path, data)
}

pub fn read(path: &[u8]) -> Option<Vec<u8>> {
    let fs = FS.lock();
    let bytes = fs.read(path)?;
    Some(bytes.to_vec())
}

pub fn exists(path: &[u8]) -> bool {
    FS.lock().exists(path)
}

pub fn is_dir(path: &[u8]) -> bool {
    FS.lock().is_dir(path)
}

/// Get list of entries in a directory as (name, is_dir, size) tuples
pub fn get_entries(dir_path: &[u8]) -> Vec<(String, bool, usize)> {
    let fs = FS.lock();
    let normalized = if dir_path.is_empty() { b"" as &[u8] } else { dir_path };
    let mut result = Vec::new();
    
    for e in fs.entries.iter() {
        if !e.used {
            continue;
        }
        if e.parent_bytes() == normalized {
            let name = core::str::from_utf8(e.name_bytes()).unwrap_or("?").to_string();
            let is_dir = e.entry_type == EntryType::Directory;
            let size = e.data.len();
            result.push((name, is_dir, size));
        }
    }
    
    result
}

/// Create directories recursively (mkdir -p)
pub fn mkdir_p(path: &[u8]) -> Result<(), &'static str> {
    FS.lock().mkdir_p(path)
}

/// Append data to a file
pub fn append(path: &[u8], data: &[u8]) -> Result<(), &'static str> {
    FS.lock().append(path, data)
}

/// Get file metadata
pub fn get_metadata(path: &[u8]) -> Option<(EntryType, usize, FileMetadata)> {
    FS.lock().get_metadata(path)
}

/// Set file permissions
pub fn chmod(path: &[u8], permissions: u16) -> Result<(), &'static str> {
    FS.lock().chmod(path, permissions)
}

/// Touch file (update timestamp or create empty)
pub fn touch(path: &[u8]) -> Result<(), &'static str> {
    FS.lock().touch(path)
}

/// Get total used space
pub fn used_space() -> usize {
    FS.lock().used_space()
}

/// Get number of used entries  
pub fn used_entries() -> usize {
    FS.lock().used_entries()
}

/// Get total capacity
pub fn total_capacity() -> usize {
    FS.lock().total_capacity()
}

/// Get free entries count
pub fn free_entries() -> usize {
    MAX_ENTRIES - used_entries()
}

/// Get directory size
pub fn dir_size(path: &[u8]) -> usize {
    FS.lock().dir_size(path)
}

/// Rename/move file or directory
pub fn rename(old_path: &[u8], new_path: &[u8]) -> Result<(), &'static str> {
    FS.lock().rename(old_path, new_path)
}

/// Copy a file
pub fn copy(src: &[u8], dst: &[u8]) -> Result<(), &'static str> {
    FS.lock().copy(src, dst)
}

/// Get detailed entries with metadata
pub fn get_entries_detailed(dir_path: &[u8]) -> Vec<(String, EntryType, usize, FileMetadata)> {
    let fs = FS.lock();
    let normalized = if dir_path.is_empty() { b"" as &[u8] } else { dir_path };
    let mut result = Vec::new();
    
    for e in fs.entries.iter() {
        if !e.used {
            continue;
        }
        if e.parent_bytes() == normalized {
            let name = core::str::from_utf8(e.name_bytes()).unwrap_or("?").to_string();
            result.push((name, e.entry_type, e.data.len(), e.metadata));
        }
    }
    
    result
}

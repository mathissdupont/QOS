extern crate alloc;

use alloc::{string::String, vec::Vec};
use lazy_static::lazy_static;
use spin::Mutex;

const MAX_ENTRIES: usize = 64;
const MAX_NAME: usize = 32;
const MAX_PATH: usize = 128;
const MAX_FILE_BYTES: usize = 64 * 1024;

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
        self.entries[idx] = entry;
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
        self.entries[idx] = entry;
        Ok(())
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

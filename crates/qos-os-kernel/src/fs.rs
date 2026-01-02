extern crate alloc;

use alloc::{string::String, vec::Vec};
use lazy_static::lazy_static;
use spin::Mutex;

const MAX_FILES: usize = 32;
const MAX_NAME: usize = 32;
const MAX_FILE_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct Entry {
    used: bool,
    name: [u8; MAX_NAME],
    name_len: usize,
    data: Vec<u8>,
}

impl Entry {
    fn empty() -> Self {
        Self {
            used: false,
            name: [0; MAX_NAME],
            name_len: 0,
            data: Vec::new(),
        }
    }

    fn name_bytes(&self) -> &[u8] {
        &self.name[..self.name_len]
    }
}

pub struct RamFs {
    entries: [Entry; MAX_FILES],
}

impl RamFs {
    pub fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| Entry::empty()),
        }
    }

    fn find(&self, name: &[u8]) -> Option<usize> {
        for (i, e) in self.entries.iter().enumerate() {
            if e.used && e.name_bytes() == name {
                return Some(i);
            }
        }
        None
    }

    fn find_free(&self) -> Option<usize> {
        self.entries.iter().position(|e| !e.used)
    }

    pub fn list(&self) {
        crate::println!("files:");
        for e in self.entries.iter() {
            if !e.used {
                continue;
            }
            let name = core::str::from_utf8(e.name_bytes()).unwrap_or("?");
            crate::println!("  {} ({} bytes)", name, e.data.len());
        }
    }

    pub fn remove(&mut self, name: &[u8]) -> bool {
        let Some(i) = self.find(name) else {
            return false;
        };
        self.entries[i] = Entry::empty();
        true
    }

    pub fn write(&mut self, name: &[u8], data: &[u8]) -> Result<(), &'static str> {
        if name.is_empty() || name.len() > MAX_NAME {
            return Err("bad name");
        }
        if data.len() > MAX_FILE_BYTES {
            return Err("too large");
        }

        let idx = if let Some(i) = self.find(name) {
            i
        } else {
            self.find_free().ok_or("no slots")?
        };

        let mut entry = Entry::empty();
        entry.used = true;
        entry.name_len = name.len();
        entry.name[..name.len()].copy_from_slice(name);
        entry.data = data.to_vec();
        self.entries[idx] = entry;
        Ok(())
    }

    pub fn read(&self, name: &[u8]) -> Option<&[u8]> {
        let i = self.find(name)?;
        Some(self.entries[i].data.as_slice())
    }

    pub fn read_string_lossy(&self, name: &[u8]) -> Option<String> {
        let bytes = self.read(name)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

lazy_static! {
    static ref FS: Mutex<RamFs> = Mutex::new(RamFs::new());
}

pub fn list() {
    FS.lock().list();
}

pub fn remove(name: &[u8]) -> bool {
    FS.lock().remove(name)
}

pub fn write(name: &[u8], data: &[u8]) -> Result<(), &'static str> {
    FS.lock().write(name, data)
}

pub fn read(name: &[u8]) -> Option<Vec<u8>> {
    // Return an owned copy to avoid holding the FS lock while callers process bytes.
    let fs = FS.lock();
    let bytes = fs.read(name)?;
    Some(bytes.to_vec())
}

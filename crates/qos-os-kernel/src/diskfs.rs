extern crate alloc;

use alloc::vec::Vec;
use alloc::string::ToString;

use crate::ata::{AtaPio, DriveSelect};

// Very small, safe-on-disk format for a dedicated workspace image (target/qos-fs.img).
// Only intended for QEMU IDE index=1 (primary slave).
//
// Layout (LBA28):
// - LBA 0: Superblock
// - LBA 1..=DIR_LAST: directory table (fixed entries)
// - LBA DATA_START..: data blocks (append-only allocator)
//
// Notes:
// - No crash safety yet (MVP).
// - No free list yet; delete only clears directory entry.

const MAGIC: &[u8; 8] = b"QOSFS1\0\0";
const VERSION: u32 = 1;

const DIR_SECTORS: u32 = 8; // 8 * 512 = 4096 bytes
const DIR_START: u32 = 1;
const DATA_START: u32 = DIR_START + DIR_SECTORS;

const MAX_FILES: usize = 32;
const NAME_MAX: usize = 32;
const MAX_FILE_BYTES: usize = 64 * 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct Superblock {
    magic: [u8; 8],
    version: u32,
    dir_start: u32,
    dir_sectors: u32,
    data_start: u32,
    next_free_lba: u32,
    _pad: [u8; 512 - 8 - 5 * 4],
}

impl Superblock {
    fn new() -> Self {
        Self {
            magic: *MAGIC,
            version: VERSION,
            dir_start: DIR_START,
            dir_sectors: DIR_SECTORS,
            data_start: DATA_START,
            next_free_lba: DATA_START,
            _pad: [0; 512 - 8 - 5 * 4],
        }
    }

    fn is_valid(&self) -> bool {
        self.magic == *MAGIC && self.version == VERSION && self.dir_start == DIR_START && self.dir_sectors == DIR_SECTORS && self.data_start == DATA_START
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DirEntry {
    used: u8,
    name_len: u8,
    _rsv: [u8; 2],
    name: [u8; NAME_MAX],
    start_lba: u32,
    size: u32,
    _pad: [u8; 64 - 1 - 1 - 2 - NAME_MAX - 4 - 4],
}

impl DirEntry {
    const fn empty() -> Self {
        Self {
            used: 0,
            name_len: 0,
            _rsv: [0; 2],
            name: [0; NAME_MAX],
            start_lba: 0,
            size: 0,
            _pad: [0; 64 - 1 - 1 - 2 - NAME_MAX - 4 - 4],
        }
    }

    fn name_bytes(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }
}

fn disk() -> AtaPio {
    AtaPio::primary(DriveSelect::Slave)
}

fn read_sector(lba: u32, out: &mut [u8; 512]) -> bool {
    disk().read_sector28(lba, out)
}

fn write_sector(lba: u32, data: &[u8; 512]) -> bool {
    disk().write_sector28(lba, data)
}

fn read_superblock() -> Option<Superblock> {
    let mut sec = [0u8; 512];
    if !read_sector(0, &mut sec) {
        return None;
    }
    let sb = unsafe { core::ptr::read_unaligned(sec.as_ptr() as *const Superblock) };
    if sb.is_valid() {
        Some(sb)
    } else {
        None
    }
}

fn write_superblock(sb: &Superblock) -> bool {
    let mut sec = [0u8; 512];
    unsafe {
        core::ptr::write_unaligned(sec.as_mut_ptr() as *mut Superblock, *sb);
    }
    write_sector(0, &sec)
}

fn read_dir_table(sb: &Superblock, out: &mut [DirEntry; MAX_FILES]) -> bool {
    // 32 entries * 64 bytes = 2048 bytes -> fits in 4 sectors, but we allocate 8.
    let mut buf = [0u8; (DIR_SECTORS as usize) * 512];
    for i in 0..(DIR_SECTORS as usize) {
        let mut sec = [0u8; 512];
        if !read_sector(sb.dir_start + i as u32, &mut sec) {
            return false;
        }
        buf[i * 512..(i + 1) * 512].copy_from_slice(&sec);
    }

    for i in 0..MAX_FILES {
        let off = i * 64;
        out[i] = unsafe { core::ptr::read_unaligned(buf[off..].as_ptr() as *const DirEntry) };
    }
    true
}

fn write_dir_table(sb: &Superblock, table: &[DirEntry; MAX_FILES]) -> bool {
    let mut buf = [0u8; (DIR_SECTORS as usize) * 512];
    for i in 0..MAX_FILES {
        let off = i * 64;
        unsafe {
            core::ptr::write_unaligned(buf[off..].as_mut_ptr() as *mut DirEntry, table[i]);
        }
    }

    for i in 0..(DIR_SECTORS as usize) {
        let mut sec = [0u8; 512];
        sec.copy_from_slice(&buf[i * 512..(i + 1) * 512]);
        if !write_sector(sb.dir_start + i as u32, &sec) {
            return false;
        }
    }
    true
}

fn find_entry(table: &[DirEntry; MAX_FILES], name: &[u8]) -> Option<usize> {
    table.iter().position(|e| e.used != 0 && e.name_bytes() == name)
}

fn find_free(table: &[DirEntry; MAX_FILES]) -> Option<usize> {
    table.iter().position(|e| e.used == 0)
}

pub fn mkfs() -> bool {
    // Write superblock + clear directory sectors.
    let mut sb = Superblock::new();

    // Clear directory
    let empty = DirEntry::empty();
    let table = [empty; MAX_FILES];

    if !write_superblock(&sb) {
        return false;
    }
    if !write_dir_table(&sb, &table) {
        return false;
    }

    // Reset allocator pointer.
    sb.next_free_lba = DATA_START;
    write_superblock(&sb)
}

pub fn is_formatted() -> bool {
    read_superblock().is_some()
}

pub fn list() -> bool {
    let Some(sb) = read_superblock() else {
        return false;
    };
    let mut table = [DirEntry::empty(); MAX_FILES];
    if !read_dir_table(&sb, &mut table) {
        return false;
    }

    crate::println!("disk files:");
    for e in table.iter() {
        if e.used == 0 {
            continue;
        }
        let name = core::str::from_utf8(e.name_bytes()).unwrap_or("?");
        crate::println!("  {} ({} bytes @ lba {})", name, e.size, e.start_lba);
    }
    true
}

pub fn read(name: &[u8]) -> Option<Vec<u8>> {
    let sb = read_superblock()?;
    let mut table = [DirEntry::empty(); MAX_FILES];
    if !read_dir_table(&sb, &mut table) {
        return None;
    }
    let idx = find_entry(&table, name)?;
    let e = table[idx];
    let size = e.size as usize;
    if size > MAX_FILE_BYTES {
        return None;
    }
    let start = e.start_lba;
    let sectors = ((size + 511) / 512) as u32;

    let mut out = Vec::with_capacity(size);
    for i in 0..sectors {
        let mut sec = [0u8; 512];
        if !read_sector(start + i, &mut sec) {
            return None;
        }
        let take = core::cmp::min(512, size.saturating_sub(out.len()));
        out.extend_from_slice(&sec[..take]);
    }
    Some(out)
}

pub fn remove(name: &[u8]) -> bool {
    let Some(sb) = read_superblock() else {
        return false;
    };
    let mut table = [DirEntry::empty(); MAX_FILES];
    if !read_dir_table(&sb, &mut table) {
        return false;
    }
    let Some(idx) = find_entry(&table, name) else {
        return false;
    };
    table[idx] = DirEntry::empty();
    write_dir_table(&sb, &table)
}

pub fn write(name: &[u8], data: &[u8]) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > NAME_MAX {
        return Err("bad name");
    }
    if data.len() > MAX_FILE_BYTES {
        return Err("too large");
    }

    let mut sb = read_superblock().ok_or("not formatted")?;

    let mut table = [DirEntry::empty(); MAX_FILES];
    if !read_dir_table(&sb, &mut table) {
        return Err("io");
    }

    let idx = if let Some(i) = find_entry(&table, name) {
        i
    } else {
        find_free(&table).ok_or("no slots")?
    };

    let start = sb.next_free_lba;
    let sectors = ((data.len() + 511) / 512) as u32;

    // Write payload sectors.
    for i in 0..sectors {
        let mut sec = [0u8; 512];
        let off = (i as usize) * 512;
        let end = core::cmp::min(off + 512, data.len());
        sec[..end - off].copy_from_slice(&data[off..end]);
        if !write_sector(start + i, &sec) {
            return Err("io");
        }
    }

    // Update directory entry.
    let mut e = DirEntry::empty();
    e.used = 1;
    e.name_len = name.len() as u8;
    e.name[..name.len()].copy_from_slice(name);
    e.start_lba = start;
    e.size = data.len() as u32;
    table[idx] = e;

    // Advance allocator.
    sb.next_free_lba = start + sectors;

    if !write_dir_table(&sb, &table) {
        return Err("io");
    }
    if !write_superblock(&sb) {
        return Err("io");
    }
    Ok(())
}

/// Get list of entries in disk filesystem as (name, is_dir, size) tuples
/// Note: diskfs is flat (no subdirectories), so dir_path is ignored
pub fn get_entries(_dir_path: &[u8]) -> alloc::vec::Vec<(alloc::string::String, bool, usize)> {
    let mut result = alloc::vec::Vec::new();
    
    let Some(sb) = read_superblock() else {
        return result;
    };
    
    let mut table = [DirEntry::empty(); MAX_FILES];
    if !read_dir_table(&sb, &mut table) {
        return result;
    }
    
    for e in table.iter() {
        if e.used == 0 {
            continue;
        }
        let name = core::str::from_utf8(e.name_bytes()).unwrap_or("?").to_string();
        // diskfs is flat, all entries are files
        result.push((name, false, e.size as usize));
    }
    
    result
}

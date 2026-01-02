//! FAT16 File System Implementation for QOS
//!
//! Provides read/write support for FAT16 formatted disks.

use crate::ata::{AtaPio, DriveSelect};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// FAT16 Boot Sector (BIOS Parameter Block)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Fat16Bpb {
    pub jump: [u8; 3],
    pub oem_name: [u8; 8],
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub num_fats: u8,
    pub root_entry_count: u16,
    pub total_sectors_16: u16,
    pub media_type: u8,
    pub fat_size_16: u16,
    pub sectors_per_track: u16,
    pub num_heads: u16,
    pub hidden_sectors: u32,
    pub total_sectors_32: u32,
    pub drive_number: u8,
    pub reserved1: u8,
    pub boot_signature: u8,
    pub volume_id: u32,
    pub volume_label: [u8; 11],
    pub fs_type: [u8; 8],
}

/// FAT16 Directory Entry
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Fat16DirEntry {
    pub name: [u8; 8],
    pub ext: [u8; 3],
    pub attributes: u8,
    pub reserved: u8,
    pub create_time_tenth: u8,
    pub create_time: u16,
    pub create_date: u16,
    pub access_date: u16,
    pub first_cluster_high: u16,
    pub modify_time: u16,
    pub modify_date: u16,
    pub first_cluster_low: u16,
    pub file_size: u32,
}

/// File attributes
pub const ATTR_READ_ONLY: u8 = 0x01;
pub const ATTR_HIDDEN: u8 = 0x02;
pub const ATTR_SYSTEM: u8 = 0x04;
pub const ATTR_VOLUME_ID: u8 = 0x08;
pub const ATTR_DIRECTORY: u8 = 0x10;
pub const ATTR_ARCHIVE: u8 = 0x20;

/// FAT16 special cluster values
const FAT16_FREE: u16 = 0x0000;
const FAT16_END: u16 = 0xFFFF;

/// FAT16 file system state
pub struct Fat16 {
    ata: AtaPio,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,
    fat_size: u16,
    root_dir_sectors: u32,
    first_data_sector: u32,
    total_clusters: u32,
    fat_start: u32,
    root_start: u32,
}

/// File info for listings
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub size: u32,
    pub is_dir: bool,
    pub cluster: u16,
}

impl Fat16 {
    /// Create FAT16 instance by reading BPB from disk
    pub fn new() -> Option<Self> {
        let ata = AtaPio::primary(DriveSelect::Master);
        
        let mut sector = [0u8; 512];
        if !ata.read_sector28(0, &mut sector) {
            return None;
        }
        
        // Check boot signature
        if sector[510] != 0x55 || sector[511] != 0xAA {
            crate::serial_println!("[FAT16] Invalid boot signature");
            return None;
        }
        
        // Parse BPB
        let bpb = unsafe { &*(sector.as_ptr() as *const Fat16Bpb) };
        
        if bpb.bytes_per_sector != 512 {
            crate::serial_println!("[FAT16] Unsupported sector size");
            return None;
        }
        
        let sectors_per_cluster = bpb.sectors_per_cluster;
        let reserved_sectors = bpb.reserved_sectors;
        let num_fats = bpb.num_fats;
        let root_entry_count = bpb.root_entry_count;
        let fat_size = bpb.fat_size_16;
        
        let root_dir_sectors = ((root_entry_count as u32 * 32) + 511) / 512;
        let fat_start = reserved_sectors as u32;
        let root_start = fat_start + (num_fats as u32 * fat_size as u32);
        let first_data_sector = root_start + root_dir_sectors;
        
        let total_sectors = if bpb.total_sectors_16 != 0 {
            bpb.total_sectors_16 as u32
        } else {
            bpb.total_sectors_32
        };
        
        let data_sectors = total_sectors.saturating_sub(first_data_sector);
        let total_clusters = data_sectors / sectors_per_cluster as u32;
        
        crate::serial_println!("[FAT16] Mounted - {} clusters", total_clusters);
        
        Some(Self {
            ata,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            root_entry_count,
            fat_size,
            root_dir_sectors,
            first_data_sector,
            total_clusters,
            fat_start,
            root_start,
        })
    }
    
    /// Read a sector
    fn read_sector(&self, lba: u32, buf: &mut [u8; 512]) -> bool {
        self.ata.read_sector28(lba, buf)
    }
    
    /// Write a sector
    fn write_sector(&self, lba: u32, buf: &[u8; 512]) -> bool {
        self.ata.write_sector28(lba, buf)
    }
    
    /// Get FAT entry value
    fn get_fat_entry(&self, cluster: u16) -> Option<u16> {
        let fat_offset = (cluster as u32) * 2;
        let sector = self.fat_start + (fat_offset / 512);
        let offset = (fat_offset % 512) as usize;
        
        let mut buf = [0u8; 512];
        if !self.read_sector(sector, &mut buf) {
            return None;
        }
        
        let value = u16::from_le_bytes([buf[offset], buf[offset + 1]]);
        Some(value)
    }
    
    /// Set FAT entry value
    fn set_fat_entry(&self, cluster: u16, value: u16) -> bool {
        let fat_offset = (cluster as u32) * 2;
        let sector = self.fat_start + (fat_offset / 512);
        let offset = (fat_offset % 512) as usize;
        
        let mut buf = [0u8; 512];
        if !self.read_sector(sector, &mut buf) {
            return false;
        }
        
        let bytes = value.to_le_bytes();
        buf[offset] = bytes[0];
        buf[offset + 1] = bytes[1];
        
        // Write to both FATs
        for fat_num in 0..self.num_fats {
            let fat_sector = sector + (fat_num as u32 * self.fat_size as u32);
            if !self.write_sector(fat_sector, &buf) {
                return false;
            }
        }
        true
    }
    
    /// Convert cluster to sector
    fn cluster_to_sector(&self, cluster: u16) -> u32 {
        self.first_data_sector + ((cluster as u32 - 2) * self.sectors_per_cluster as u32)
    }
    
    /// Parse 8.3 filename
    fn parse_83_name(entry: &Fat16DirEntry) -> String {
        let mut name = String::new();
        
        for &b in &entry.name {
            if b == 0x20 || b == 0 {
                break;
            }
            name.push(b as char);
        }
        
        let mut has_ext = false;
        for &b in &entry.ext {
            if b != 0x20 && b != 0 {
                has_ext = true;
                break;
            }
        }
        
        if has_ext {
            name.push('.');
            for &b in &entry.ext {
                if b == 0x20 || b == 0 {
                    break;
                }
                name.push(b as char);
            }
        }
        
        name
    }
    
    /// Create 8.3 filename
    fn make_83_name(filename: &str) -> ([u8; 8], [u8; 3]) {
        let mut name = [0x20u8; 8];
        let mut ext = [0x20u8; 3];
        
        let parts: Vec<&str> = filename.splitn(2, '.').collect();
        let base = parts[0].to_uppercase();
        let extension = parts.get(1).map(|s| s.to_uppercase()).unwrap_or_default();
        
        for (i, c) in base.bytes().take(8).enumerate() {
            name[i] = c;
        }
        
        for (i, c) in extension.bytes().take(3).enumerate() {
            ext[i] = c;
        }
        
        (name, ext)
    }
    
    /// List root directory
    pub fn list_root(&self) -> Vec<FileInfo> {
        let mut files = Vec::new();
        let entries_per_sector = 512 / 32;
        let root_sectors = (self.root_entry_count as u32 + entries_per_sector - 1) / entries_per_sector;
        
        for sector_idx in 0..root_sectors {
            let mut buf = [0u8; 512];
            if !self.read_sector(self.root_start + sector_idx, &mut buf) {
                break;
            }
            
            for entry_idx in 0..entries_per_sector {
                let offset = (entry_idx * 32) as usize;
                let entry = unsafe { &*(buf.as_ptr().add(offset) as *const Fat16DirEntry) };
                
                // End of directory
                if entry.name[0] == 0x00 {
                    return files;
                }
                
                // Deleted entry
                if entry.name[0] == 0xE5 {
                    continue;
                }
                
                // Skip volume label and long names
                if entry.attributes & (ATTR_VOLUME_ID | 0x0F) != 0 {
                    continue;
                }
                
                files.push(FileInfo {
                    name: Self::parse_83_name(entry),
                    size: entry.file_size,
                    is_dir: entry.attributes & ATTR_DIRECTORY != 0,
                    cluster: entry.first_cluster_low,
                });
            }
        }
        
        files
    }
    
    /// Find file in root directory
    fn find_file(&self, filename: &str) -> Option<(Fat16DirEntry, u32, usize)> {
        let (target_name, target_ext) = Self::make_83_name(filename);
        let entries_per_sector = 512 / 32;
        let root_sectors = (self.root_entry_count as u32 + entries_per_sector - 1) / entries_per_sector;
        
        for sector_idx in 0..root_sectors {
            let mut buf = [0u8; 512];
            if !self.read_sector(self.root_start + sector_idx, &mut buf) {
                break;
            }
            
            for entry_idx in 0..entries_per_sector {
                let offset = (entry_idx * 32) as usize;
                let entry = unsafe { *(buf.as_ptr().add(offset) as *const Fat16DirEntry) };
                
                if entry.name[0] == 0x00 {
                    return None;
                }
                
                if entry.name[0] == 0xE5 {
                    continue;
                }
                
                if entry.attributes & (ATTR_VOLUME_ID | 0x0F) != 0 {
                    continue;
                }
                
                if entry.name == target_name && entry.ext == target_ext {
                    return Some((entry, self.root_start + sector_idx, offset));
                }
            }
        }
        
        None
    }
    
    /// Read file contents
    pub fn read_file(&self, filename: &str) -> Option<Vec<u8>> {
        let (entry, _, _) = self.find_file(filename)?;
        
        if entry.attributes & ATTR_DIRECTORY != 0 {
            return None; // Can't read directories this way
        }
        
        let mut data = Vec::with_capacity(entry.file_size as usize);
        let mut cluster = entry.first_cluster_low;
        let bytes_per_cluster = self.sectors_per_cluster as usize * 512;
        let mut remaining = entry.file_size as usize;
        
        while cluster >= 2 && cluster < 0xFFF0 && remaining > 0 {
            let sector = self.cluster_to_sector(cluster);
            
            for i in 0..self.sectors_per_cluster {
                if remaining == 0 {
                    break;
                }
                
                let mut buf = [0u8; 512];
                if !self.read_sector(sector + i as u32, &mut buf) {
                    return None;
                }
                
                let to_copy = core::cmp::min(512, remaining);
                data.extend_from_slice(&buf[..to_copy]);
                remaining -= to_copy;
            }
            
            cluster = self.get_fat_entry(cluster).unwrap_or(FAT16_END);
        }
        
        Some(data)
    }
    
    /// Allocate a free cluster
    fn alloc_cluster(&self) -> Option<u16> {
        for cluster in 2..self.total_clusters as u16 {
            if let Some(FAT16_FREE) = self.get_fat_entry(cluster) {
                if self.set_fat_entry(cluster, FAT16_END) {
                    return Some(cluster);
                }
            }
        }
        None
    }
    
    /// Find empty directory entry
    fn find_empty_entry(&self) -> Option<(u32, usize)> {
        let entries_per_sector = 512 / 32;
        let root_sectors = (self.root_entry_count as u32 + entries_per_sector - 1) / entries_per_sector;
        
        for sector_idx in 0..root_sectors {
            let mut buf = [0u8; 512];
            if !self.read_sector(self.root_start + sector_idx, &mut buf) {
                continue;
            }
            
            for entry_idx in 0..entries_per_sector {
                let offset = (entry_idx * 32) as usize;
                let first_byte = buf[offset];
                
                if first_byte == 0x00 || first_byte == 0xE5 {
                    return Some((self.root_start + sector_idx, offset));
                }
            }
        }
        None
    }
    
    /// Write file
    pub fn write_file(&self, filename: &str, data: &[u8]) -> bool {
        // Delete existing file first
        self.delete_file(filename);
        
        // Find empty directory entry
        let (entry_sector, entry_offset) = match self.find_empty_entry() {
            Some(e) => e,
            None => return false,
        };
        
        // Allocate clusters
        let bytes_per_cluster = self.sectors_per_cluster as usize * 512;
        let clusters_needed = (data.len() + bytes_per_cluster - 1) / bytes_per_cluster;
        let clusters_needed = core::cmp::max(1, clusters_needed);
        
        let mut clusters = Vec::new();
        for _ in 0..clusters_needed {
            match self.alloc_cluster() {
                Some(c) => clusters.push(c),
                None => {
                    // Free already allocated clusters
                    for &c in &clusters {
                        let _ = self.set_fat_entry(c, FAT16_FREE);
                    }
                    return false;
                }
            }
        }
        
        // Link clusters in FAT
        for i in 0..clusters.len() - 1 {
            self.set_fat_entry(clusters[i], clusters[i + 1]);
        }
        if let Some(&last) = clusters.last() {
            self.set_fat_entry(last, FAT16_END);
        }
        
        // Write data
        let mut data_offset = 0;
        for &cluster in &clusters {
            let sector = self.cluster_to_sector(cluster);
            
            for i in 0..self.sectors_per_cluster {
                let mut buf = [0u8; 512];
                let to_copy = core::cmp::min(512, data.len().saturating_sub(data_offset));
                
                if to_copy > 0 {
                    buf[..to_copy].copy_from_slice(&data[data_offset..data_offset + to_copy]);
                }
                
                if !self.write_sector(sector + i as u32, &buf) {
                    return false;
                }
                
                data_offset += 512;
            }
        }
        
        // Create directory entry
        let (name, ext) = Self::make_83_name(filename);
        let mut entry = Fat16DirEntry {
            name,
            ext,
            attributes: ATTR_ARCHIVE,
            reserved: 0,
            create_time_tenth: 0,
            create_time: 0,
            create_date: 0,
            access_date: 0,
            first_cluster_high: 0,
            modify_time: 0,
            modify_date: 0,
            first_cluster_low: clusters.first().copied().unwrap_or(0),
            file_size: data.len() as u32,
        };
        
        // Write directory entry
        let mut buf = [0u8; 512];
        if !self.read_sector(entry_sector, &mut buf) {
            return false;
        }
        
        let entry_bytes = unsafe {
            core::slice::from_raw_parts(&entry as *const _ as *const u8, 32)
        };
        buf[entry_offset..entry_offset + 32].copy_from_slice(entry_bytes);
        
        self.write_sector(entry_sector, &buf)
    }
    
    /// Delete file
    pub fn delete_file(&self, filename: &str) -> bool {
        let (entry, entry_sector, entry_offset) = match self.find_file(filename) {
            Some(e) => e,
            None => return false,
        };
        
        // Free cluster chain
        let mut cluster = entry.first_cluster_low;
        while cluster >= 2 && cluster < 0xFFF0 {
            let next = self.get_fat_entry(cluster).unwrap_or(FAT16_END);
            self.set_fat_entry(cluster, FAT16_FREE);
            cluster = next;
        }
        
        // Mark directory entry as deleted
        let mut buf = [0u8; 512];
        if !self.read_sector(entry_sector, &mut buf) {
            return false;
        }
        
        buf[entry_offset] = 0xE5; // Deleted marker
        self.write_sector(entry_sector, &buf)
    }
    
    /// Get file info
    pub fn stat(&self, filename: &str) -> Option<FileInfo> {
        let (entry, _, _) = self.find_file(filename)?;
        Some(FileInfo {
            name: Self::parse_83_name(&entry),
            size: entry.file_size,
            is_dir: entry.attributes & ATTR_DIRECTORY != 0,
            cluster: entry.first_cluster_low,
        })
    }
}

/// Check if disk has FAT16
pub fn is_fat16() -> bool {
    Fat16::new().is_some()
}

/// List files
pub fn list() {
    if let Some(fs) = Fat16::new() {
        for file in fs.list_root() {
            if file.is_dir {
                crate::println!("  <DIR>  {}", file.name);
            } else {
                crate::println!("  {:>6} {}", file.size, file.name);
            }
        }
    }
}

/// Read file
pub fn read(name: &[u8]) -> Option<Vec<u8>> {
    let filename = core::str::from_utf8(name).ok()?;
    Fat16::new()?.read_file(filename)
}

/// Write file
pub fn write(name: &[u8], data: &[u8]) -> Result<(), &'static str> {
    let filename = core::str::from_utf8(name).map_err(|_| "invalid filename")?;
    let fs = Fat16::new().ok_or("FAT16 not mounted")?;
    if fs.write_file(filename, data) {
        Ok(())
    } else {
        Err("write failed")
    }
}

/// Delete file
pub fn remove(name: &[u8]) -> bool {
    if let Ok(filename) = core::str::from_utf8(name) {
        if let Some(fs) = Fat16::new() {
            return fs.delete_file(filename);
        }
    }
    false
}

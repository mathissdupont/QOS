//! AHCI (SATA) block driver — modern persistent storage for QOS (ADR-0018 / WP-05).
//!
//! Replaces the legacy ATA-PIO path (ISA ports `0x1F0`, absent on q35 / real UEFI machines) with
//! the AHCI HBA that q35 actually exposes (`8086:2922`, PCI class `01:06:01`). Bring-up:
//!
//!   1. find the HBA via PCI (class 0x01 / subclass 0x06 / prog-IF 0x01),
//!   2. enable memory-space + bus-master in the PCI command register, map the ABAR (BAR5) through
//!      the bootloader's physical-memory offset,
//!   3. set GHC.AE, scan PxSSTS/PxSIG for attached SATA disks,
//!   4. per port: stop, install a DMA command list + received-FIS area, restart, then issue
//!      IDENTIFY / READ DMA EXT / WRITE DMA EXT via a command table (Register-H2D FIS + PRDT).
//!
//! All controller-visible structures live in DMA-allocated physical frames (known phys addr,
//! accessed through `phys_offset()`), the same model the xHCI driver (WP-04) uses.

extern crate alloc;

use spin::Mutex;

// ---- HBA global registers (offsets into the ABAR) ----
const HBA_GHC: u64 = 0x04;
const HBA_PI: u64 = 0x0C;
const GHC_AE: u32 = 1 << 31;

// ---- per-port registers (base = 0x100 + port*0x80) ----
const PORT_BASE: u64 = 0x100;
const PORT_STRIDE: u64 = 0x80;
const PX_CLB: u64 = 0x00;
const PX_CLBU: u64 = 0x04;
const PX_FB: u64 = 0x08;
const PX_FBU: u64 = 0x0C;
const PX_IS: u64 = 0x10;
const PX_CMD: u64 = 0x18;
const PX_TFD: u64 = 0x20;
const PX_SIG: u64 = 0x24;
const PX_SSTS: u64 = 0x28;
const PX_SERR: u64 = 0x30;
const PX_CI: u64 = 0x38;

const CMD_ST: u32 = 1 << 0; // start
const CMD_FRE: u32 = 1 << 4; // FIS receive enable
const CMD_FR: u32 = 1 << 14; // FIS receive running
const CMD_CR: u32 = 1 << 15; // command list running

const TFD_BSY: u32 = 1 << 7;
const TFD_DRQ: u32 = 1 << 3;
const TFD_ERR: u32 = 1 << 0;

const SIG_SATA: u32 = 0x0000_0101; // non-packet SATA disk

// ATA commands issued via the H2D FIS.
const ATA_IDENTIFY: u8 = 0xEC;
const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;

pub const SECTOR: usize = 512;

/// A discovered, ready SATA disk: the HBA's mapped ABAR (virt) + the port index + its DMA area.
#[derive(Clone, Copy)]
pub struct Disk {
    abar: u64,        // virtual base of the ABAR (phys_offset + BAR5)
    port: u32,        // port index
    clb_phys: u64,    // command list base (phys)
    clb_virt: u64,    // command list base (virt)
    ctba_phys: u64,   // command table base (phys)
    ctba_virt: u64,   // command table base (virt)
    fb_phys: u64,     // received-FIS base (phys)
    pub sectors: u64, // capacity in 512-byte sectors (from IDENTIFY)
}

/// The first ready SATA disk found at boot (the persistent data disk). `None` if none attached.
static DISK: Mutex<Option<Disk>> = Mutex::new(None);

#[inline]
fn r32(virt: u64) -> u32 {
    unsafe { core::ptr::read_volatile(virt as *const u32) }
}
#[inline]
fn w32(virt: u64, v: u32) {
    unsafe { core::ptr::write_volatile(virt as *mut u32, v) }
}

impl Disk {
    #[inline]
    fn pr(&self, off: u64) -> u32 {
        r32(self.abar + PORT_BASE + self.port as u64 * PORT_STRIDE + off)
    }
    #[inline]
    fn pw(&self, off: u64, v: u32) {
        w32(self.abar + PORT_BASE + self.port as u64 * PORT_STRIDE + off, v)
    }

    /// Stop the port's command engine (clear ST + FRE and wait for CR + FR to clear).
    fn stop(&self) {
        let mut cmd = self.pr(PX_CMD);
        cmd &= !(CMD_ST | CMD_FRE);
        self.pw(PX_CMD, cmd);
        for _ in 0..1_000_000 {
            if self.pr(PX_CMD) & (CMD_CR | CMD_FR) == 0 {
                break;
            }
        }
    }

    /// Start the port: install CLB/FB, then set FRE + ST.
    fn start(&self) {
        for _ in 0..1_000_000 {
            if self.pr(PX_CMD) & CMD_CR == 0 {
                break;
            }
        }
        self.pw(PX_CLB, self.clb_phys as u32);
        self.pw(PX_CLBU, (self.clb_phys >> 32) as u32);
        self.pw(PX_FB, self.fb_phys as u32);
        self.pw(PX_FBU, (self.fb_phys >> 32) as u32);
        let mut cmd = self.pr(PX_CMD);
        cmd |= CMD_FRE;
        self.pw(PX_CMD, cmd);
        cmd |= CMD_ST;
        self.pw(PX_CMD, cmd);
    }

    fn wait_not_busy(&self) -> bool {
        for _ in 0..2_000_000 {
            if self.pr(PX_TFD) & (TFD_BSY | TFD_DRQ) == 0 {
                return true;
            }
        }
        false
    }

    /// Build a command (slot 0) that transfers `count` sectors for ATA `cmd` at `lba`, with the DMA
    /// data buffer at `buf_phys` (`write` sets the H2D FIS write direction), then issue it and wait.
    fn issue(&self, cmd: u8, lba: u64, count: u16, buf_phys: u64, write: bool) -> bool {
        if !self.wait_not_busy() {
            return false;
        }
        self.pw(PX_IS, !0); // clear pending interrupts
        self.pw(PX_SERR, !0);

        let bytes = count as u32 * SECTOR as u32;

        // Command header (slot 0), 32 bytes at clb_virt.
        // dw0: bits0-4 CFL (FIS len in dwords), bit6 W (write), bits16-31 PRDTL.
        let cfl = 5u32; // Register H2D FIS = 5 dwords (20 bytes)
        let w_bit = if write { 1u32 << 6 } else { 0 };
        let prdtl = 1u32; // one PRDT entry
        unsafe {
            let h = self.clb_virt as *mut u32;
            core::ptr::write_volatile(h, cfl | w_bit | (prdtl << 16));
            core::ptr::write_volatile(h.add(1), 0); // PRDBC (bytes transferred) = 0
            core::ptr::write_volatile(h.add(2), self.ctba_phys as u32); // CTBA
            core::ptr::write_volatile(h.add(3), (self.ctba_phys >> 32) as u32); // CTBAU
            core::ptr::write_volatile(h.add(4), 0);
            core::ptr::write_volatile(h.add(5), 0);
            core::ptr::write_volatile(h.add(6), 0);
            core::ptr::write_volatile(h.add(7), 0);
        }

        // Command table: CFIS (64 bytes) + ACMD (16) + reserved (48) + PRDT entries (16 each).
        unsafe {
            core::ptr::write_bytes(self.ctba_virt as *mut u8, 0, 128);
            let fis = self.ctba_virt as *mut u8;
            core::ptr::write_volatile(fis.add(0), 0x27); // FIS type: Register H2D
            core::ptr::write_volatile(fis.add(1), 0x80); // C=1 (command)
            core::ptr::write_volatile(fis.add(2), cmd); // command
            core::ptr::write_volatile(fis.add(3), 0); // featurel
            core::ptr::write_volatile(fis.add(4), (lba & 0xFF) as u8); // lba0
            core::ptr::write_volatile(fis.add(5), ((lba >> 8) & 0xFF) as u8); // lba1
            core::ptr::write_volatile(fis.add(6), ((lba >> 16) & 0xFF) as u8); // lba2
            core::ptr::write_volatile(fis.add(7), 0x40); // device: LBA mode
            core::ptr::write_volatile(fis.add(8), ((lba >> 24) & 0xFF) as u8); // lba3
            core::ptr::write_volatile(fis.add(9), ((lba >> 32) & 0xFF) as u8); // lba4
            core::ptr::write_volatile(fis.add(10), ((lba >> 40) & 0xFF) as u8); // lba5
            core::ptr::write_volatile(fis.add(11), 0); // featureh
            core::ptr::write_volatile(fis.add(12), (count & 0xFF) as u8); // countl
            core::ptr::write_volatile(fis.add(13), ((count >> 8) & 0xFF) as u8); // counth

            // PRDT entry 0 at offset 0x80: DBA, DBAU, rsvd, (DBC | I<<31).
            let prd = (self.ctba_virt + 0x80) as *mut u32;
            core::ptr::write_volatile(prd, buf_phys as u32);
            core::ptr::write_volatile(prd.add(1), (buf_phys >> 32) as u32);
            core::ptr::write_volatile(prd.add(2), 0);
            // Data byte count is (bytes - 1); bit31 = interrupt on completion.
            core::ptr::write_volatile(prd.add(3), (bytes - 1) | (1 << 31));
        }

        // Issue on slot 0 and wait for CI bit to clear (or an error).
        self.pw(PX_CI, 1);
        for _ in 0..5_000_000 {
            if self.pr(PX_CI) & 1 == 0 {
                return self.pr(PX_TFD) & TFD_ERR == 0;
            }
            if self.pr(PX_IS) & (1 << 30) != 0 {
                return false; // Task File Error Status
            }
        }
        false
    }

    /// Read `count` (1..) sectors starting at `lba` into `out` (>= count*512 bytes).
    pub fn read(&self, lba: u64, count: u16, out: &mut [u8]) -> bool {
        if count == 0 || out.len() < count as usize * SECTOR {
            return false;
        }
        let Some((buf_phys, buf_virt)) = alloc_dma_frame() else {
            return false;
        };
        let ok = self.issue(ATA_READ_DMA_EXT, lba, count, buf_phys, false);
        if ok {
            unsafe {
                core::ptr::copy_nonoverlapping(buf_virt as *const u8, out.as_mut_ptr(), count as usize * SECTOR);
            }
        }
        ok
    }

    /// Write `count` (1..) sectors starting at `lba` from `data` (>= count*512 bytes).
    pub fn write(&self, lba: u64, count: u16, data: &[u8]) -> bool {
        if count == 0 || data.len() < count as usize * SECTOR {
            return false;
        }
        let Some((buf_phys, buf_virt)) = alloc_dma_frame() else {
            return false;
        };
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), buf_virt as *mut u8, count as usize * SECTOR);
        }
        self.issue(ATA_WRITE_DMA_EXT, lba, count, buf_phys, true)
    }
}

/// Allocate a zeroed 4 KiB physical frame for DMA; returns `(phys, virt)`.
fn alloc_dma_frame() -> Option<(u64, u64)> {
    use x86_64::structures::paging::FrameAllocator;
    let frame = crate::memory::with_ctx(|_, fa| fa.allocate_frame())?;
    let phys = frame.start_address().as_u64();
    let virt = crate::memory::phys_offset().as_u64() + phys;
    unsafe { core::ptr::write_bytes(virt as *mut u8, 0, 4096) };
    Some((phys, virt))
}

/// Discover the AHCI HBA(s) and the first attached SATA disk; run IDENTIFY and cache it in `DISK`.
/// Safe to call once at boot after PCI enumeration. Logs to serial. Never panics on a bad/absent
/// controller — persistent storage is opt-in (fallback-first, ADR-0015).
pub fn init() {
    for dev in crate::pci::devices() {
        if !(dev.class_code == 0x01 && dev.subclass == 0x06 && dev.prog_if == 0x01) {
            continue;
        }
        // BAR5 (offset 0x24) holds the ABAR (memory-mapped, 32-bit on q35).
        let bar5 = crate::pci::config_read32(&dev, 0x24);
        let abar_phys = (bar5 & 0xFFFF_FFF0) as u64;
        if abar_phys == 0 {
            continue;
        }
        // Enable memory space (bit1) + bus master (bit2) in the PCI command register (0x04).
        let cmd = crate::pci::config_read16(&dev, 0x04);
        crate::pci::config_write16(&dev, 0x04, cmd | 0x0006);

        let abar = crate::memory::phys_offset().as_u64() + abar_phys;
        // Enable AHCI mode.
        w32(abar + HBA_GHC, r32(abar + HBA_GHC) | GHC_AE);

        let pi = r32(abar + HBA_PI);
        crate::serial_println!(
            "[AHCI] HBA {:04x}:{:04x} ABAR {:#x} PI {:#010x}",
            dev.vendor_id,
            dev.device_id,
            abar_phys,
            pi
        );

        for port in 0..32u32 {
            if pi & (1 << port) == 0 {
                continue;
            }
            let pbase = abar + PORT_BASE + port as u64 * PORT_STRIDE;
            let ssts = r32(pbase + PX_SSTS);
            let det = ssts & 0xF;
            let ipm = (ssts >> 8) & 0xF;
            if det != 3 || ipm != 1 {
                continue; // no device / not active
            }
            let sig = r32(pbase + PX_SIG);
            if sig != SIG_SATA {
                continue; // not a plain SATA disk (e.g. ATAPI)
            }

            // One frame holds the command list (1 KiB) + received FIS (256 B) + command table.
            let Some((frame_phys, frame_virt)) = alloc_dma_frame() else {
                continue;
            };
            let disk = Disk {
                abar,
                port,
                clb_phys: frame_phys,
                clb_virt: frame_virt,
                fb_phys: frame_phys + 0x400, // received FIS at +1 KiB (256-aligned)
                ctba_phys: frame_phys + 0x500, // command table at +1280 (128-aligned)
                ctba_virt: frame_virt + 0x500,
                sectors: 0,
            };
            disk.stop();
            disk.start();

            let Some(sectors) = identify(&disk) else {
                crate::serial_println!("[AHCI] port {}: IDENTIFY failed", port);
                continue;
            };
            let mut d = disk;
            d.sectors = sectors;
            let mib = sectors * SECTOR as u64 / (1024 * 1024);

            // SAFETY: never adopt the boot volume as the data disk (writing to it would corrupt
            // boot). Classify by LBA0: our QOSFS magic → definitely data; an MBR/GPT boot
            // signature (0x55AA at offset 510) → skip; otherwise a blank disk → data candidate.
            let mut lba0 = [0u8; SECTOR];
            let read_ok = d.read(0, 1, &mut lba0);
            let is_qosfs = read_ok && &lba0[0..6] == b"QOSFS1";
            let is_boot = read_ok && lba0[510] == 0x55 && lba0[511] == 0xAA && !is_qosfs;
            if is_boot {
                crate::serial_println!("[AHCI] port {}: boot volume ({} MiB) — skipping", port, mib);
                continue;
            }
            crate::serial_println!(
                "[AHCI] port {}: SATA data disk, {} sectors (~{} MiB){}",
                port,
                sectors,
                mib,
                if is_qosfs { " [QOSFS present]" } else { " [blank]" }
            );
            *DISK.lock() = Some(d);
            return; // first data disk is enough
        }
    }
    if DISK.lock().is_none() {
        crate::serial_println!("[AHCI] no data disk found (persistent storage unavailable)");
    }
}

/// Run IDENTIFY DEVICE on `disk`; return the LBA sector count (words 100..103, or the 28-bit count
/// from words 60..61 as a fallback).
fn identify(disk: &Disk) -> Option<u64> {
    let (buf_phys, buf_virt) = alloc_dma_frame()?;
    if !disk.issue(ATA_IDENTIFY, 0, 1, buf_phys, false) {
        return None;
    }
    let words = buf_virt as *const u16;
    let rd = |i: usize| unsafe { core::ptr::read_volatile(words.add(i)) };
    let lba48 = (rd(100) as u64)
        | ((rd(101) as u64) << 16)
        | ((rd(102) as u64) << 32)
        | ((rd(103) as u64) << 48);
    if lba48 > 0 {
        return Some(lba48);
    }
    let lba28 = (rd(60) as u64) | ((rd(61) as u64) << 16);
    if lba28 > 0 {
        Some(lba28)
    } else {
        None
    }
}

/// Is a persistent SATA disk present?
pub fn present() -> bool {
    DISK.lock().is_some()
}

/// Capacity in 512-byte sectors of the data disk (0 if none).
pub fn capacity_sectors() -> u64 {
    DISK.lock().map(|d| d.sectors).unwrap_or(0)
}

/// Read one 512-byte sector. Returns false if no disk / on error.
pub fn read_sector(lba: u64, out: &mut [u8; SECTOR]) -> bool {
    match *DISK.lock() {
        Some(d) => d.read(lba, 1, out),
        None => false,
    }
}

/// Write one 512-byte sector. Returns false if no disk / on error.
pub fn write_sector(lba: u64, data: &[u8; SECTOR]) -> bool {
    match *DISK.lock() {
        Some(d) => d.write(lba, 1, data),
        None => false,
    }
}

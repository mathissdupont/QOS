use crate::arch;

const PRI_IO: u16 = 0x1F0;
const PRI_CTRL: u16 = 0x3F6;

const REG_DATA: u16 = 0x00;
const REG_ERROR: u16 = 0x01;
const REG_SECCNT: u16 = 0x02;
const REG_LBA0: u16 = 0x03;
const REG_LBA1: u16 = 0x04;
const REG_LBA2: u16 = 0x05;
const REG_HDDEVSEL: u16 = 0x06;
const REG_STATUS: u16 = 0x07;
const REG_COMMAND: u16 = 0x07;

const CMD_IDENTIFY: u8 = 0xEC;
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;

const SR_BSY: u8 = 1 << 7;
const SR_DRDY: u8 = 1 << 6;
const SR_DF: u8 = 1 << 5;
const SR_DREQ: u8 = 1 << 3;
const SR_ERR: u8 = 1 << 0;

#[derive(Clone, Copy)]
pub enum DriveSelect {
    Master,
    Slave,
}

pub struct AtaPio {
    io: u16,
    ctrl: u16,
    drive: DriveSelect,
}

impl AtaPio {
    pub const fn primary(drive: DriveSelect) -> Self {
        Self {
            io: PRI_IO,
            ctrl: PRI_CTRL,
            drive,
        }
    }

    fn reg(&self, r: u16) -> u16 {
        self.io + r
    }

    fn select_drive(&self) {
        // 0xA0 = master, 0xB0 = slave, plus LBA bit (0x40).
        let v = match self.drive {
            DriveSelect::Master => 0xE0,
            DriveSelect::Slave => 0xF0,
        };
        unsafe {
            arch::outb(self.reg(REG_HDDEVSEL), v);
            // 400ns delay: read alternate status 4x.
            let _ = arch::inb(self.ctrl);
            let _ = arch::inb(self.ctrl);
            let _ = arch::inb(self.ctrl);
            let _ = arch::inb(self.ctrl);
        }
    }

    fn status(&self) -> u8 {
        unsafe { arch::inb(self.reg(REG_STATUS)) }
    }

    fn wait_not_busy(&self, mut spins: u32) -> bool {
        while spins > 0 {
            let s = self.status();
            if (s & SR_BSY) == 0 {
                return true;
            }
            spins -= 1;
        }
        false
    }

    fn wait_drq(&self, mut spins: u32) -> bool {
        while spins > 0 {
            let s = self.status();
            if (s & SR_ERR) != 0 || (s & SR_DF) != 0 {
                return false;
            }
            if (s & SR_BSY) == 0 && (s & SR_DREQ) != 0 {
                return true;
            }
            spins -= 1;
        }
        false
    }

    pub fn identify(&self, out_words: &mut [u16; 256]) -> bool {
        self.select_drive();

        unsafe {
            // Clear registers
            arch::outb(self.reg(REG_SECCNT), 0);
            arch::outb(self.reg(REG_LBA0), 0);
            arch::outb(self.reg(REG_LBA1), 0);
            arch::outb(self.reg(REG_LBA2), 0);
            arch::outb(self.reg(REG_COMMAND), CMD_IDENTIFY);
        }

        // If no device, status is 0.
        let s = self.status();
        if s == 0 {
            return false;
        }

        if !self.wait_not_busy(200_000) {
            return false;
        }
        if !self.wait_drq(200_000) {
            return false;
        }

        for w in out_words.iter_mut() {
            *w = unsafe { arch::inw(self.reg(REG_DATA)) };
        }
        true
    }

    pub fn read_sector28(&self, lba: u32, out_512: &mut [u8; 512]) -> bool {
        self.select_drive();

        unsafe {
            arch::outb(self.reg(REG_SECCNT), 1);
            arch::outb(self.reg(REG_LBA0), (lba & 0xFF) as u8);
            arch::outb(self.reg(REG_LBA1), ((lba >> 8) & 0xFF) as u8);
            arch::outb(self.reg(REG_LBA2), ((lba >> 16) & 0xFF) as u8);

            let head = match self.drive {
                DriveSelect::Master => 0xE0,
                DriveSelect::Slave => 0xF0,
            };
            arch::outb(self.reg(REG_HDDEVSEL), head | (((lba >> 24) & 0x0F) as u8));

            arch::outb(self.reg(REG_COMMAND), CMD_READ_SECTORS);
        }

        if !self.wait_not_busy(200_000) {
            return false;
        }
        if !self.wait_drq(200_000) {
            return false;
        }

        for i in 0..256 {
            let w = unsafe { arch::inw(self.reg(REG_DATA)) };
            out_512[i * 2] = (w & 0xFF) as u8;
            out_512[i * 2 + 1] = (w >> 8) as u8;
        }
        true
    }

    pub fn write_sector28(&self, lba: u32, data_512: &[u8; 512]) -> bool {
        self.select_drive();

        unsafe {
            arch::outb(self.reg(REG_SECCNT), 1);
            arch::outb(self.reg(REG_LBA0), (lba & 0xFF) as u8);
            arch::outb(self.reg(REG_LBA1), ((lba >> 8) & 0xFF) as u8);
            arch::outb(self.reg(REG_LBA2), ((lba >> 16) & 0xFF) as u8);

            let head = match self.drive {
                DriveSelect::Master => 0xE0,
                DriveSelect::Slave => 0xF0,
            };
            arch::outb(self.reg(REG_HDDEVSEL), head | (((lba >> 24) & 0x0F) as u8));

            arch::outb(self.reg(REG_COMMAND), CMD_WRITE_SECTORS);
        }

        if !self.wait_not_busy(200_000) {
            return false;
        }
        if !self.wait_drq(200_000) {
            return false;
        }

        for i in 0..256 {
            let lo = data_512[i * 2] as u16;
            let hi = (data_512[i * 2 + 1] as u16) << 8;
            unsafe { arch::outw(self.reg(REG_DATA), lo | hi) };
        }

        // Wait for write to complete.
        self.wait_not_busy(500_000)
    }
}

pub fn parse_model(words: &[u16; 256]) -> [u8; 40] {
    // ATA identify model is words 27..46 (20 words), big-endian pairs.
    let mut out = [b' '; 40];
    let mut o = 0;
    for w in &words[27..47] {
        let hi = (w >> 8) as u8;
        let lo = (*w & 0xFF) as u8;
        out[o] = hi;
        out[o + 1] = lo;
        o += 2;
    }
    out
}

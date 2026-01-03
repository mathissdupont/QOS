//! CMOS Real-Time Clock (RTC) driver for QOS
//!
//! Reads date/time from the MC146818 RTC chip via CMOS ports 0x70/0x71.
//! Provides system time for timestamps, job scheduling, and user display.

use crate::arch;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

/// CMOS I/O ports
const CMOS_ADDRESS: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

/// CMOS registers
const RTC_SECONDS: u8 = 0x00;
const RTC_MINUTES: u8 = 0x02;
const RTC_HOURS: u8 = 0x04;
const RTC_DAY: u8 = 0x07;
const RTC_MONTH: u8 = 0x08;
const RTC_YEAR: u8 = 0x09;
const RTC_CENTURY: u8 = 0x32; // May not exist on all systems
const RTC_STATUS_A: u8 = 0x0A;
const RTC_STATUS_B: u8 = 0x0B;

/// Cached boot time as Unix timestamp (seconds since 1970-01-01)
static BOOT_TIME: AtomicU64 = AtomicU64::new(0);

/// Mutex for RTC access
static RTC_LOCK: Mutex<()> = Mutex::new(());

/// Date/Time structure
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DateTime {
    /// Format as ISO-8601 string (YYYY-MM-DD HH:MM:SS)
    pub fn format(&self, buf: &mut [u8]) -> usize {
        if buf.len() < 19 {
            return 0;
        }
        
        // YYYY-MM-DD HH:MM:SS
        buf[0] = b'0' + ((self.year / 1000) % 10) as u8;
        buf[1] = b'0' + ((self.year / 100) % 10) as u8;
        buf[2] = b'0' + ((self.year / 10) % 10) as u8;
        buf[3] = b'0' + (self.year % 10) as u8;
        buf[4] = b'-';
        buf[5] = b'0' + (self.month / 10);
        buf[6] = b'0' + (self.month % 10);
        buf[7] = b'-';
        buf[8] = b'0' + (self.day / 10);
        buf[9] = b'0' + (self.day % 10);
        buf[10] = b' ';
        buf[11] = b'0' + (self.hour / 10);
        buf[12] = b'0' + (self.hour % 10);
        buf[13] = b':';
        buf[14] = b'0' + (self.minute / 10);
        buf[15] = b'0' + (self.minute % 10);
        buf[16] = b':';
        buf[17] = b'0' + (self.second / 10);
        buf[18] = b'0' + (self.second % 10);
        19
    }

    /// Format time only (HH:MM:SS)
    pub fn format_time(&self, buf: &mut [u8]) -> usize {
        if buf.len() < 8 {
            return 0;
        }
        buf[0] = b'0' + (self.hour / 10);
        buf[1] = b'0' + (self.hour % 10);
        buf[2] = b':';
        buf[3] = b'0' + (self.minute / 10);
        buf[4] = b'0' + (self.minute % 10);
        buf[5] = b':';
        buf[6] = b'0' + (self.second / 10);
        buf[7] = b'0' + (self.second % 10);
        8
    }

    /// Convert to Unix timestamp (seconds since 1970-01-01 00:00:00 UTC)
    pub fn to_unix(&self) -> u64 {
        // Days from year 1970 to this year
        let mut days: u64 = 0;
        for y in 1970..self.year {
            days += if is_leap_year(y) { 366 } else { 365 };
        }
        
        // Days from months in current year
        const DAYS_IN_MONTH: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for m in 1..self.month {
            days += DAYS_IN_MONTH[(m - 1) as usize] as u64;
            if m == 2 && is_leap_year(self.year) {
                days += 1;
            }
        }
        
        // Add days in current month
        days += (self.day - 1) as u64;
        
        // Convert to seconds
        days * 86400 + (self.hour as u64) * 3600 + (self.minute as u64) * 60 + self.second as u64
    }
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Read a CMOS register
fn read_cmos(reg: u8) -> u8 {
    unsafe {
        // Disable NMI (bit 7) while accessing CMOS
        arch::outb(CMOS_ADDRESS, (1 << 7) | reg);
        arch::inb(CMOS_DATA)
    }
}

/// Check if RTC update is in progress
fn is_update_in_progress() -> bool {
    read_cmos(RTC_STATUS_A) & 0x80 != 0
}

/// Convert BCD to binary
fn bcd_to_bin(bcd: u8) -> u8 {
    ((bcd >> 4) * 10) + (bcd & 0x0F)
}

/// Read current date/time from RTC
pub fn read_datetime() -> DateTime {
    let _guard = RTC_LOCK.lock();
    
    // Wait for update to complete
    while is_update_in_progress() {}
    
    // Read status register B to check format
    let status_b = read_cmos(RTC_STATUS_B);
    let is_binary = (status_b & 0x04) != 0;
    let is_24hour = (status_b & 0x02) != 0;
    
    // Read values
    let mut second = read_cmos(RTC_SECONDS);
    let mut minute = read_cmos(RTC_MINUTES);
    let mut hour = read_cmos(RTC_HOURS);
    let mut day = read_cmos(RTC_DAY);
    let mut month = read_cmos(RTC_MONTH);
    let mut year = read_cmos(RTC_YEAR);
    
    // Try to read century (may not exist)
    let century = read_cmos(RTC_CENTURY);
    
    // Read again to ensure consistency
    while is_update_in_progress() {}
    let second2 = read_cmos(RTC_SECONDS);
    let minute2 = read_cmos(RTC_MINUTES);
    
    // If values changed, read again
    if second != second2 || minute != minute2 {
        while is_update_in_progress() {}
        second = read_cmos(RTC_SECONDS);
        minute = read_cmos(RTC_MINUTES);
        hour = read_cmos(RTC_HOURS);
        day = read_cmos(RTC_DAY);
        month = read_cmos(RTC_MONTH);
        year = read_cmos(RTC_YEAR);
    }
    
    // Convert from BCD if needed
    if !is_binary {
        second = bcd_to_bin(second);
        minute = bcd_to_bin(minute);
        hour = bcd_to_bin(hour & 0x7F) | (hour & 0x80); // Preserve PM bit
        day = bcd_to_bin(day);
        month = bcd_to_bin(month);
        year = bcd_to_bin(year);
    }
    
    // Convert 12-hour to 24-hour if needed
    if !is_24hour && (hour & 0x80) != 0 {
        hour = ((hour & 0x7F) + 12) % 24;
    }
    
    // Calculate full year
    let full_year = if century != 0 && century != 0xFF {
        let c = if is_binary { century } else { bcd_to_bin(century) };
        (c as u16) * 100 + (year as u16)
    } else {
        // Assume 20xx for years 00-99
        2000 + (year as u16)
    };
    
    DateTime {
        year: full_year,
        month,
        day,
        hour,
        minute,
        second,
    }
}

/// Initialize RTC - cache boot time
pub fn init() {
    let dt = read_datetime();
    let unix = dt.to_unix();
    BOOT_TIME.store(unix, Ordering::SeqCst);
    
    crate::serial_println!("[RTC] Initialized: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second);
}

/// Get boot time as Unix timestamp
pub fn boot_time() -> u64 {
    BOOT_TIME.load(Ordering::Relaxed)
}

/// Get current Unix timestamp
pub fn unix_time() -> u64 {
    read_datetime().to_unix()
}

/// Alias for unix_time - used by fs module
pub fn unix_timestamp() -> u64 {
    unix_time()
}

/// Get system uptime in seconds (using PIT ticks)
pub fn uptime_seconds() -> u64 {
    let ticks = crate::interrupts::TICKS.load(Ordering::Relaxed);
    // PIT is configured for ~100 Hz (actually 1193182/11931 ≈ 100.006 Hz)
    ticks / 100
}

/// Get formatted current time string
pub fn time_string() -> alloc::string::String {
    let dt = read_datetime();
    alloc::format!("{:02}:{:02}:{:02}", dt.hour, dt.minute, dt.second)
}

/// Get formatted current date/time string
pub fn datetime_string() -> alloc::string::String {
    let dt = read_datetime();
    alloc::format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second)
}

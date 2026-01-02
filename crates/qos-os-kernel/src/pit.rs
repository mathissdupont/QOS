use crate::arch;

pub fn init_timer(hz: u32) {
    // PIT base frequency: 1_193_182 Hz
    let divisor = (1_193_182u32 / hz.max(1)).clamp(1, 0xFFFF) as u16;

    unsafe {
        // Channel 0, lobyte/hibyte, mode 3 (square wave), binary
        arch::outb(0x43, 0x36);
        arch::outb(0x40, (divisor & 0xFF) as u8);
        arch::outb(0x40, (divisor >> 8) as u8);
    }
}

use crate::arch;

/// Busy-wait roughly `us` microseconds using PIT channel 2 in one-shot mode (mode 0), polling the
/// output on port 0x61 bit 5. Uses no interrupts and is independent of channel 0 (the scheduler
/// tick), so it works even after IRQ0 is masked — used to calibrate the local-APIC timer (E-10).
/// Max ~54 ms per call (16-bit counter); callers loop for longer delays.
pub fn busy_wait_us(us: u32) {
    let count = ((1_193_182u64 * us as u64) / 1_000_000).clamp(1, 0xFFFF) as u16;
    unsafe {
        // Enable the channel-2 gate, keep the speaker off: (0x61 & 0xFC) | 0x01.
        let v = arch::inb(0x61);
        arch::outb(0x61, (v & 0xFC) | 0x01);
        // Channel 2, lobyte/hibyte, mode 0 (interrupt on terminal count), binary.
        arch::outb(0x43, 0xB0);
        arch::outb(0x42, (count & 0xFF) as u8);
        arch::outb(0x42, (count >> 8) as u8);
        // Mode 0: OUT goes high at terminal count — wait for 0x61 bit 5.
        while arch::inb(0x61) & 0x20 == 0 {
            core::hint::spin_loop();
        }
    }
}

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

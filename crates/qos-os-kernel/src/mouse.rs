//! PS/2 Mouse driver for QaOS
//! Supports scroll wheel for VGA buffer scrolling

use core::sync::atomic::{AtomicI8, AtomicU8, Ordering};
use spin::Mutex;

use crate::arch;

/// Mouse packet state
static PACKET_BYTE: AtomicU8 = AtomicU8::new(0);
static PACKET: Mutex<[u8; 4]> = Mutex::new([0; 4]);

/// Scroll delta (positive = up, negative = down)
static SCROLL_DELTA: AtomicI8 = AtomicI8::new(0);

/// Mouse has scroll wheel
static HAS_WHEEL: AtomicU8 = AtomicU8::new(0);

/// Wait for PS/2 controller input buffer to be ready
fn wait_write() {
    for _ in 0..100_000 {
        if (unsafe { arch::inb(0x64) } & 0x02) == 0 {
            return;
        }
    }
}

/// Wait for PS/2 controller output buffer to have data
fn wait_read() {
    for _ in 0..100_000 {
        if (unsafe { arch::inb(0x64) } & 0x01) != 0 {
            return;
        }
    }
}

/// Send command to PS/2 controller
fn controller_cmd(cmd: u8) {
    wait_write();
    unsafe { arch::outb(0x64, cmd); }
}

/// Send command to mouse (through controller)
fn mouse_cmd(cmd: u8) {
    controller_cmd(0xD4); // Tell controller next byte goes to mouse
    wait_write();
    unsafe { arch::outb(0x60, cmd); }
    
    // Wait for ACK
    wait_read();
    let _ = unsafe { arch::inb(0x60) };
}

/// Read from PS/2 data port
fn read_data() -> u8 {
    wait_read();
    unsafe { arch::inb(0x60) }
}

/// Initialize PS/2 mouse with scroll wheel detection
pub fn init() {
    // Enable auxiliary device (mouse)
    controller_cmd(0xA8);
    
    // Get current controller config
    controller_cmd(0x20);
    let config = read_data();
    
    // Enable IRQ12 (mouse interrupt) in controller config
    controller_cmd(0x60);
    wait_write();
    unsafe { arch::outb(0x60, config | 0x02); }
    
    // Reset mouse
    mouse_cmd(0xFF);
    wait_read();
    let _ = unsafe { arch::inb(0x60) }; // Discard reset response
    wait_read();
    let _ = unsafe { arch::inb(0x60) }; // Discard device ID
    
    // Try to enable scroll wheel (Intellimouse protocol)
    // Magic sequence: set sample rate 200, 100, 80
    mouse_cmd(0xF3); mouse_cmd(200);
    mouse_cmd(0xF3); mouse_cmd(100);
    mouse_cmd(0xF3); mouse_cmd(80);
    
    // Get device ID
    mouse_cmd(0xF2);
    wait_read();
    let device_id = unsafe { arch::inb(0x60) };
    
    if device_id == 3 || device_id == 4 {
        HAS_WHEEL.store(1, Ordering::Relaxed);
        crate::serial_println!("[Mouse] Scroll wheel detected (ID={})", device_id);
    } else {
        crate::serial_println!("[Mouse] Standard mouse (ID={})", device_id);
    }
    
    // Set sample rate to 100
    mouse_cmd(0xF3);
    mouse_cmd(100);
    
    // Enable mouse data reporting
    mouse_cmd(0xF4);
    
    crate::serial_println!("[Mouse] Initialized");
}

/// Called from IRQ12 handler
pub fn handle_interrupt() {
    let data = unsafe { arch::inb(0x60) };
    
    let byte_num = PACKET_BYTE.load(Ordering::Relaxed);
    let has_wheel = HAS_WHEEL.load(Ordering::Relaxed) != 0;
    let packet_size = if has_wheel { 4 } else { 3 };
    
    {
        let mut packet = PACKET.lock();
        packet[byte_num as usize] = data;
    }
    
    let next_byte = (byte_num + 1) % packet_size;
    PACKET_BYTE.store(next_byte, Ordering::Relaxed);
    
    // Process complete packet
    if next_byte == 0 {
        let packet = PACKET.lock();
        
        // Check if this looks like a valid packet (bit 3 should be set in byte 0)
        if (packet[0] & 0x08) == 0 {
            // Invalid packet, resync
            return;
        }
        
        // Extract scroll wheel delta from byte 3
        if has_wheel {
            let scroll = packet[3] as i8;
            if scroll != 0 {
                // Scroll: positive = scroll up, negative = scroll down
                let delta = -scroll; // Invert for natural scrolling
                SCROLL_DELTA.store(delta, Ordering::Relaxed);
                
                // Directly scroll VGA buffer
                if delta > 0 {
                    crate::vga::scroll_up_lines(delta as usize);
                } else if delta < 0 {
                    crate::vga::scroll_down_lines((-delta) as usize);
                }
            }
        }
    }
}

/// Get and clear scroll delta
pub fn take_scroll_delta() -> i8 {
    SCROLL_DELTA.swap(0, Ordering::Relaxed)
}

/// Check if mouse has scroll wheel
pub fn has_scroll_wheel() -> bool {
    HAS_WHEEL.load(Ordering::Relaxed) != 0
}

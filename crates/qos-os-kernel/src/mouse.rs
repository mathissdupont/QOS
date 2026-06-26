//! PS/2 Mouse driver for QaOS
//! Supports scroll wheel for VGA buffer scrolling

use core::sync::atomic::{AtomicI8, AtomicI16, AtomicU8, Ordering};
use spin::Mutex;

use crate::arch;

/// Mouse packet state
static PACKET_BYTE: AtomicU8 = AtomicU8::new(0);
static PACKET: Mutex<[u8; 4]> = Mutex::new([0; 4]);

/// Scroll delta (positive = up, negative = down)
static SCROLL_DELTA: AtomicI8 = AtomicI8::new(0);

/// Mouse has scroll wheel
static HAS_WHEEL: AtomicU8 = AtomicU8::new(0);

/// Mouse position (in text columns/rows for VGA 80x25)
static MOUSE_X: AtomicI16 = AtomicI16::new(40);  // Start center
static MOUSE_Y: AtomicI16 = AtomicI16::new(12);

/// Mouse button state
static MOUSE_BUTTONS: AtomicU8 = AtomicU8::new(0);

/// Click event queue
static CLICK_EVENT: Mutex<Option<MouseClick>> = Mutex::new(None);

/// Screen dimensions (text mode)
const SCREEN_WIDTH: i16 = 80;
const SCREEN_HEIGHT: i16 = 25;

/// Mouse sensitivity divisor (pixels per text cell)
const SENSITIVITY: i16 = 8;

/// Mouse click event
#[derive(Clone, Copy, Debug)]
pub struct MouseClick {
    pub x: usize,      // Column (0-79)
    pub y: usize,      // Row (0-24)
    pub button: MouseButton,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

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

    // PS/2 packet sync: byte 0 of every packet has bit 3 (0x08) set. If we are expecting the
    // first byte and it does NOT have bit 3 set, we are desynchronised (a byte was dropped) —
    // discard this byte to realign instead of building a garbage packet. Without this, motion
    // looks erratic or dead once sync is lost.
    if byte_num == 0 && (data & 0x08) == 0 {
        return;
    }

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
        
        let buttons = packet[0] & 0x07;
        let old_buttons = MOUSE_BUTTONS.swap(buttons, Ordering::Relaxed);
        
        // Extract movement delta
        let dx = packet[1] as i8;
        let dy = packet[2] as i8;
        
        // Update position
        let mut x = MOUSE_X.load(Ordering::Relaxed);
        let mut y = MOUSE_Y.load(Ordering::Relaxed);
        
        // Scale movement (divide by sensitivity for text mode)
        x += (dx as i16) / SENSITIVITY;
        y -= (dy as i16) / SENSITIVITY;  // Y is inverted in PS/2
        
        // Clamp to screen bounds
        x = x.clamp(0, SCREEN_WIDTH - 1);
        y = y.clamp(0, SCREEN_HEIGHT - 1);
        
        MOUSE_X.store(x, Ordering::Relaxed);
        MOUSE_Y.store(y, Ordering::Relaxed);
        
        // Detect button clicks (transition from not pressed to pressed)
        if (buttons & 0x01) != 0 && (old_buttons & 0x01) == 0 {
            // Left click
            *CLICK_EVENT.lock() = Some(MouseClick {
                x: x as usize,
                y: y as usize,
                button: MouseButton::Left,
            });
        }
        if (buttons & 0x02) != 0 && (old_buttons & 0x02) == 0 {
            // Right click
            *CLICK_EVENT.lock() = Some(MouseClick {
                x: x as usize,
                y: y as usize,
                button: MouseButton::Right,
            });
        }
        if (buttons & 0x04) != 0 && (old_buttons & 0x04) == 0 {
            // Middle click
            *CLICK_EVENT.lock() = Some(MouseClick {
                x: x as usize,
                y: y as usize,
                button: MouseButton::Middle,
            });
        }

        // Feed the unified input event queue (Phase 0.1) with raw movement + button edges.
        if dx != 0 || dy != 0 {
            crate::input::push(crate::input::InputEvent::MouseMove {
                dx: dx as i16,
                dy: dy as i16,
            });
        }
        let changed = buttons ^ old_buttons;
        if changed & 0x01 != 0 {
            crate::input::push(crate::input::InputEvent::MouseButton {
                button: MouseButton::Left,
                pressed: buttons & 0x01 != 0,
            });
        }
        if changed & 0x02 != 0 {
            crate::input::push(crate::input::InputEvent::MouseButton {
                button: MouseButton::Right,
                pressed: buttons & 0x02 != 0,
            });
        }
        if changed & 0x04 != 0 {
            crate::input::push(crate::input::InputEvent::MouseButton {
                button: MouseButton::Middle,
                pressed: buttons & 0x04 != 0,
            });
        }

        // Extract scroll wheel delta from byte 3
        if has_wheel {
            let scroll = packet[3] as i8;
            if scroll != 0 {
                crate::input::push(crate::input::InputEvent::MouseScroll { delta: scroll });
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

/// Get current mouse position (column, row)
pub fn position() -> (usize, usize) {
    let x = MOUSE_X.load(Ordering::Relaxed) as usize;
    let y = MOUSE_Y.load(Ordering::Relaxed) as usize;
    (x, y)
}

/// Get and clear click event
pub fn take_click() -> Option<MouseClick> {
    CLICK_EVENT.lock().take()
}

/// Check if left button is currently held
pub fn left_held() -> bool {
    (MOUSE_BUTTONS.load(Ordering::Relaxed) & 0x01) != 0
}

/// Check if right button is currently held  
pub fn right_held() -> bool {
    (MOUSE_BUTTONS.load(Ordering::Relaxed) & 0x02) != 0
}

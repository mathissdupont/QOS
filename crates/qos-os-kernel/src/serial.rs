use core::fmt;

use spin::Mutex;

use crate::arch;

const COM1: u16 = 0x3F8;

static SERIAL_LOCK: Mutex<()> = Mutex::new(());

fn write_byte(byte: u8) {
    unsafe {
        // If the UART is absent/misconfigured, an infinite wait here would freeze the kernel
        // before we even get a VGA message. Keep it bounded and still log to debugcon.
        let mut spins: u32 = 0;
        while (arch::inb(COM1 + 5) & 0x20) == 0 {
            spins = spins.wrapping_add(1);
            if spins > 1_000_000 {
                break;
            }
        }

        if (arch::inb(COM1 + 5) & 0x20) != 0 {
            arch::outb(COM1, byte);
        }
        // QEMU debug console is commonly wired to either I/O port 0x402 or 0xE9.
        // We write to both so our headless verification can capture logs reliably.
        arch::outb(0x402, byte);
        arch::outb(0xE9, byte);
    }
}

pub fn init() {
    let _guard = SERIAL_LOCK.lock();
    unsafe {
        arch::outb(COM1 + 1, 0x00);
        arch::outb(COM1 + 3, 0x80);
        arch::outb(COM1 + 0, 0x03);
        arch::outb(COM1 + 1, 0x00);
        arch::outb(COM1 + 3, 0x03);
        arch::outb(COM1 + 2, 0xC7);
        arch::outb(COM1 + 4, 0x0B);
    }
}

struct SerialWriter;

impl fmt::Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            match b {
                b'\n' => {
                    write_byte(b'\r');
                    write_byte(b'\n');
                }
                _ => write_byte(b),
            }
        }
        Ok(())
    }
}

pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    let _guard = SERIAL_LOCK.lock();
    SerialWriter.write_fmt(args).ok();
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_print(core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! serial_println {
    () => {
        $crate::serial_print!("\n")
    };
    ($fmt:expr) => {
        $crate::serial_print!(concat!($fmt, "\n"))
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::serial_print!(concat!($fmt, "\n"), $($arg)*)
    };
}

pub use crate::serial_println as println;

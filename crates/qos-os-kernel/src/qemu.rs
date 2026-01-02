use crate::arch;

/// Exit QEMU via the `isa-debug-exit` device.
///
/// Requires QEMU args: `-device isa-debug-exit,iobase=0xf4,iosize=0x04`.
///
/// QEMU encodes the exit status as `(code << 1) | 1`.
pub fn exit(code: u32) -> ! {
    unsafe {
        arch::outl(0xF4, code);
    }
    loop {
        arch::hlt();
    }
}

use x86_64::instructions::port::{Port, PortGeneric, ReadWriteAccess};

#[inline(always)]
pub unsafe fn outb(port: u16, value: u8) {
    let mut p = Port::new(port);
    p.write(value);
}

#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let mut p = Port::new(port);
    p.read()
}

#[inline(always)]
pub unsafe fn outw(port: u16, value: u16) {
    let mut p: PortGeneric<u16, ReadWriteAccess> = PortGeneric::new(port);
    p.write(value);
}

#[inline(always)]
pub unsafe fn inw(port: u16) -> u16 {
    let mut p: PortGeneric<u16, ReadWriteAccess> = PortGeneric::new(port);
    p.read()
}

#[inline(always)]
pub unsafe fn outl(port: u16, value: u32) {
    let mut p: PortGeneric<u32, ReadWriteAccess> = PortGeneric::new(port);
    p.write(value);
}

#[inline(always)]
pub unsafe fn inl(port: u16) -> u32 {
    let mut p: PortGeneric<u32, ReadWriteAccess> = PortGeneric::new(port);
    p.read()
}

#[inline(always)]
pub fn hlt() {
    x86_64::instructions::hlt();
}

#[inline(always)]
pub fn enable_interrupts() {
    x86_64::instructions::interrupts::enable();
}

#[inline(always)]
pub fn disable_interrupts() {
    x86_64::instructions::interrupts::disable();
}

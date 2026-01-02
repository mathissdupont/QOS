#![no_std]
#![no_main]

use core::arch::asm;

#[repr(C)]
struct ShmCall {
    abi_version: u32,
    op: u32,
    status: u32,
    _reserved: u32,
    ret0: u64,
    ret1: u64,
    arg0: u64,
    arg1: u64,
}

const ABI_CALL_ADDR: u64 = 0x0000_0000_4001_0000;
const ABI_VERSION: u32 = 1;
const OP_EXIT: u32 = 4;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        let call = &mut *(ABI_CALL_ADDR as *mut ShmCall);
        call.abi_version = ABI_VERSION;
        call.arg0 = 0;
        call.arg1 = 0;

        // Exit immediately. This proves ELF loading + Ring3 -> int 0x80 works.
        call.op = OP_EXIT;
        asm!("int 0x80", options(nostack));
    }

    loop {
        unsafe { asm!("pause", options(nostack)); }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        unsafe { asm!("pause", options(nostack)); }
    }
}

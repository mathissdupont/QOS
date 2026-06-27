use x86_64::{
    PhysAddr,
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{FrameAllocator, Mapper, Page, PageTableFlags, PhysFrame, Size4KiB},
};

use alloc::vec::Vec;
use spin::Mutex;

use crate::{elf, gdt, memory, serial, vga};

extern "C" {
    fn asm_iretq_to_user(rip: u64, rsp: u64, cs: u64, ss: u64, rflags: u64) -> !;
}

static USER_MAPPED_PAGES: Mutex<Vec<Page<Size4KiB>>> = Mutex::new(Vec::new());

pub struct SpawnedUserProcess {
    pub user_cr3: PhysFrame<Size4KiB>,
    pub entry: VirtAddr,
    pub user_stack_top: VirtAddr,
    pub mapped_pages: Vec<Page<Size4KiB>>,
}

fn track_user_page(page: Page<Size4KiB>) {
    let mut pages = USER_MAPPED_PAGES.lock();
    if pages.iter().any(|p| p.start_address() == page.start_address()) {
        return;
    }
    pages.push(page);
}

fn track_page_into(pages: &mut Vec<Page<Size4KiB>>, page: Page<Size4KiB>) {
    if pages.iter().any(|p| p.start_address() == page.start_address()) {
        return;
    }
    pages.push(page);
}

fn map_vga_identity(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) {
    // The VGA text buffer is accessed via identity-mapped low memory (see `vga.rs`).
    // When running under a per-process CR3, that identity mapping must exist too,
    // otherwise any vga output (including from exception handlers) will page fault.
    let vga_virt = VirtAddr::new(0xb8000);
    let page = Page::containing_address(vga_virt);
    let frame = PhysFrame::containing_address(PhysAddr::new(0xb8000));
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;

    if mapper.translate_page(page).is_ok() {
        return;
    }

    // SAFETY: We intentionally map the known VGA physical frame at 0xb8000.
    unsafe {
        mapper
            .map_to(page, frame, flags, frame_allocator)
            .expect("map vga identity")
            .flush();
    }
}

pub fn cleanup_user_mappings() {
    let pages = {
        let mut guard = USER_MAPPED_PAGES.lock();
        if guard.is_empty() {
            return;
        }
        core::mem::take(&mut *guard)
    };

    // Unmap from the currently active address space (typically the user process CR3).
    let mut mapper = unsafe { memory::init(memory::phys_offset()) };
    for page in pages {
        if mapper.translate_page(page).is_err() {
            continue;
        }
        // SAFETY: We only unmap pages we previously mapped for user mode.
        unsafe {
            if let Ok((frame, flush)) = mapper.unmap(page) {
                flush.flush();
                memory::with_ctx(|_, frame_allocator| {
                    frame_allocator.deallocate_frame(frame);
                });
            }
        }
    }
}

pub fn cleanup_user_addrspace_and_get_cr3() -> PhysFrame<Size4KiB> {
    // Capture current user CR3 before we modify anything.
    let (user_cr3, _) = Cr3::read();

    // Free all tracked user-mapped pages (and recycle their frames).
    cleanup_user_mappings();

    // Free page-table frames for the user subtree (P4[0]) while still on the user CR3.
    // We reserved P4[0] for user virtual addresses (0x4000_0000..), so we can safely
    // tear down only that subtree without touching kernel mappings.
    let p4 = unsafe {
        let virt = memory::phys_offset() + user_cr3.start_address().as_u64();
        &mut *(virt.as_mut_ptr::<x86_64::structures::paging::PageTable>())
    };

    let entry0 = p4[0].clone();
    p4[0].set_unused();

    // Recursively free page tables for P4[0] (P3 -> P2 -> P1). Do not free leaf frames here;
    // leaf frames are recycled by `cleanup_user_mappings()`.
    fn free_table_level(
        table_frame: PhysFrame<Size4KiB>,
        level: u8,
    ) {
        let table = unsafe {
            let virt = memory::phys_offset() + table_frame.start_address().as_u64();
            &mut *(virt.as_mut_ptr::<x86_64::structures::paging::PageTable>())
        };

        if level > 1 {
            for entry in table.iter_mut() {
                if !entry.flags().contains(PageTableFlags::PRESENT) {
                    continue;
                }
                if entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                    // MVP: we don't create huge pages for user space; just clear.
                    entry.set_unused();
                    continue;
                }
                if let Ok(child) = entry.frame() {
                    entry.set_unused();
                    free_table_level(child, level - 1);
                } else {
                    entry.set_unused();
                }
            }
        } else {
            // Level 1 contains leaf mappings; just clear entries.
            for entry in table.iter_mut() {
                entry.set_unused();
            }
        }

        memory::with_ctx(|_, frame_allocator| {
            frame_allocator.deallocate_frame(table_frame);
        });
    }

    if entry0.flags().contains(PageTableFlags::PRESENT) {
        if entry0.flags().contains(PageTableFlags::HUGE_PAGE) {
            // Unexpected; clear only.
        } else if let Ok(p3) = entry0.frame() {
            free_table_level(p3, 3);
        }
    }

    user_cr3
}

const USER_CODE_START: u64 = 0x0000_0000_4000_0000;
const USER_STACK_START: u64 = 0x0000_0000_4000_8000;
const USER_ABI_CALL_START: u64 = crate::syscall::ABI_CALL_ADDR;
const USER_ABI_SUBMITBUF_START: u64 = USER_ABI_CALL_START + 0x100;
const USER_ELF_STACK_START: u64 = 0x0000_0000_4010_0000;

const USER_QASM2_BELL: &[u8] = b"OPENQASM 2.0;\ninclude \"qelib1.inc\";\nqreg q[2];\ncreg c[2];\nh q[0];\ncx q[0],q[1];\nmeasure q -> c;\n";
const USER_SHOTS: u32 = 1024;
const USER_N_QUBITS: u32 = 2;

const SUBMIT_HDR_LEN: usize = core::mem::size_of::<qos_abi::shm::ShmSubmitIrHeader>();
const SUBMIT_TOTAL_LEN: usize = SUBMIT_HDR_LEN + USER_QASM2_BELL.len();

// MINIMAL TEST: Set ABI version and call exit syscall
const USER_PROG_SIMPLE: &[u8] = &[
    // mov dword ptr [0x40010000], 1  ; abi_version = 1
    0xC7, 0x04, 0x25, 0x00, 0x00, 0x01, 0x40, 0x01, 0x00, 0x00, 0x00,
    // mov dword ptr [0x40010004], 4  ; op=Exit (syscall 4)
    0xC7, 0x04, 0x25, 0x04, 0x00, 0x01, 0x40, 0x04, 0x00, 0x00, 0x00,
    // int 0x80
    0xCD, 0x80,
    // hlt (should never reach)
    0xF4,
];

// x86_64 user program (FULL VERSION - DISABLED FOR NOW):
//   ; SubmitIr(QASM2)
//   mov qword ptr [CALL.arg0], SUBMITBUF_PTR
//   mov qword ptr [CALL.arg1], SUBMITBUF_TOTAL
//   mov dword ptr [CALL.op], 7
//   int 0x80
//   mov rbx, qword ptr [CALL.ret0]   ; handle1
//
//   ; SubmitIr(QASM2) #2
//   mov qword ptr [CALL.arg0], SUBMITBUF_PTR
//   mov qword ptr [CALL.arg1], SUBMITBUF_TOTAL
//   mov dword ptr [CALL.op], 7
//   int 0x80
//   mov rcx, qword ptr [CALL.ret0]   ; handle2
//
//   ; Poll status(handle1) until Done
// .L1:
//   mov qword ptr [CALL.arg0], rbx
//   mov dword ptr [CALL.op], 2
//   int 0x80
//   mov rax, qword ptr [CALL.ret0]
//   cmp eax, 3                             ; Done=3?
//   jne .L1                                ; loop until Done
//
//   ; GetResult(handle1)
//   mov qword ptr [CALL.arg0], rbx
//   mov dword ptr [CALL.op], 3
//   int 0x80
//
//   ; Poll status(handle2) until Done
// .L2:
//   mov qword ptr [CALL.arg0], rcx
//   mov dword ptr [CALL.op], 2
//   int 0x80
//   mov rax, qword ptr [CALL.ret0]
//   cmp eax, 3                             ; Done=3?
//   jne .L2                                ; loop until Done
//
//   ; GetResult(handle2)
//   mov qword ptr [CALL.arg0], rcx
//   mov dword ptr [CALL.op], 3
//   int 0x80
//
//   ; Exit
//   mov dword ptr [CALL.op], 4
//   int 0x80
//   jmp .
//
// Encoding note: uses absolute disp32 addressing (sign-extended) which works because
// USER_ABI_CALL_START fits in 32 bits (0x4001_0000).
const USER_PROG: &[u8] = &[
    // mov dword ptr [0x40010000], 1        ; abi_version
    0xC7, 0x04, 0x25, 0x00, 0x00, 0x01, 0x40, 0x01, 0x00, 0x00, 0x00,

    // mov qword ptr [0x40010020], 0x40010100 ; arg0=submitbuf ptr
    0x48, 0xC7, 0x04, 0x25, 0x20, 0x00, 0x01, 0x40, 0x00, 0x01, 0x01, 0x40,
    // mov qword ptr [0x40010028], SUBMIT_TOTAL ; arg1=total bytes
    0x48, 0xC7, 0x04, 0x25, 0x28, 0x00, 0x01, 0x40, (SUBMIT_TOTAL_LEN as u8), 0x00, 0x00, 0x00,
    // mov dword ptr [0x40010004], 7         ; op=SubmitIr
    0xC7, 0x04, 0x25, 0x04, 0x00, 0x01, 0x40, 0x07, 0x00, 0x00, 0x00,
    // int 0x80
    0xCD, 0x80,
    // mov rbx, qword ptr [0x40010010]      ; ret0=handle
    0x48, 0x8B, 0x1C, 0x25, 0x10, 0x00, 0x01, 0x40,

    // mov qword ptr [0x40010020], 0x40010100 ; arg0=submitbuf ptr
    0x48, 0xC7, 0x04, 0x25, 0x20, 0x00, 0x01, 0x40, 0x00, 0x01, 0x01, 0x40,
    // mov qword ptr [0x40010028], SUBMIT_TOTAL ; arg1=total bytes
    0x48, 0xC7, 0x04, 0x25, 0x28, 0x00, 0x01, 0x40, (SUBMIT_TOTAL_LEN as u8), 0x00, 0x00, 0x00,
    // mov dword ptr [0x40010004], 7         ; op=SubmitIr (2)
    0xC7, 0x04, 0x25, 0x04, 0x00, 0x01, 0x40, 0x07, 0x00, 0x00, 0x00,
    // int 0x80
    0xCD, 0x80,
    // mov rcx, qword ptr [0x40010010]      ; ret0=handle2
    0x48, 0x8B, 0x0C, 0x25, 0x10, 0x00, 0x01, 0x40,

    // .L1: poll status until Done
    // mov qword ptr [0x40010020], rbx      ; arg0=handle
    0x48, 0x89, 0x1C, 0x25, 0x20, 0x00, 0x01, 0x40,
    // mov dword ptr [0x40010004], 2        ; op=GetStatus
    0xC7, 0x04, 0x25, 0x04, 0x00, 0x01, 0x40, 0x02, 0x00, 0x00, 0x00,
    // int 0x80
    0xCD, 0x80,
    // mov rax, qword ptr [0x40010010]      ; ret0=state
    0x48, 0x8B, 0x04, 0x25, 0x10, 0x00, 0x01, 0x40,
    // cmp eax, 3                           ; Done=3?
    0x83, 0xF8, 0x03,
    // jne .L1                              ; loop if not Done
    0x75, 0xDE,
    // mov qword ptr [0x40010020], rbx      ; arg0=handle
    0x48, 0x89, 0x1C, 0x25, 0x20, 0x00, 0x01, 0x40,
    // mov dword ptr [0x40010004], 3        ; op=GetResult
    0xC7, 0x04, 0x25, 0x04, 0x00, 0x01, 0x40, 0x03, 0x00, 0x00, 0x00,
    // int 0x80
    0xCD, 0x80,

    // .L2: poll status until Done
    // mov qword ptr [0x40010020], rcx      ; arg0=handle2
    0x48, 0x89, 0x0C, 0x25, 0x20, 0x00, 0x01, 0x40,
    // mov dword ptr [0x40010004], 2        ; op=GetStatus
    0xC7, 0x04, 0x25, 0x04, 0x00, 0x01, 0x40, 0x02, 0x00, 0x00, 0x00,
    // int 0x80
    0xCD, 0x80,
    // mov rax, qword ptr [0x40010010]      ; ret0=state
    0x48, 0x8B, 0x04, 0x25, 0x10, 0x00, 0x01, 0x40,
    // cmp eax, 3                           ; Done=3?
    0x83, 0xF8, 0x03,
    // jne .L2                              ; loop if not Done
    0x75, 0xDE,

    // mov qword ptr [0x40010020], rcx      ; arg0=handle2
    0x48, 0x89, 0x0C, 0x25, 0x20, 0x00, 0x01, 0x40,
    // mov dword ptr [0x40010004], 3        ; op=GetResult
    0xC7, 0x04, 0x25, 0x04, 0x00, 0x01, 0x40, 0x03, 0x00, 0x00, 0x00,
    // int 0x80
    0xCD, 0x80,

    // mov dword ptr [0x40010004], 4        ; op=Exit
    0xC7, 0x04, 0x25, 0x04, 0x00, 0x01, 0x40, 0x04, 0x00, 0x00, 0x00,
    // int 0x80
    0xCD, 0x80,
    // jmp $
    0xEB, 0xFE,
];

pub fn enter_user_mode(frame_allocator: &mut memory::BootInfoFrameAllocator) -> ! {
    serial::println!("[USER] Starting user-mode quantum program...");
    
    // Create a fresh user address space (shares kernel mappings).
    let (user_cr3, mut user_mapper) = memory::create_user_pagetable(frame_allocator);

    // Keep kernel VGA output working while the user CR3 is active.
    map_vga_identity(&mut user_mapper, frame_allocator);

    let code_page = Page::containing_address(VirtAddr::new(USER_CODE_START));
    let stack_page = Page::containing_address(VirtAddr::new(USER_STACK_START));
    let abi_page = Page::containing_address(VirtAddr::new(USER_ABI_CALL_START));

    map_user_page(&mut user_mapper, frame_allocator, code_page, false);
    map_user_page(&mut user_mapper, frame_allocator, stack_page, true);
    map_user_page(&mut user_mapper, frame_allocator, abi_page, true);

    // Switch to the user page table before touching user virtual addresses.
    // Otherwise, writes to the shared ABI call-frame would fault under the kernel CR3.
    memory::switch_cr3(user_cr3);

    // Preload a submit buffer (header + QASM2 payload) into the user-mapped ABI page so Ring3
    // can submit a realistic job (format + n_qubits + shots + payload_len + bytes).
    unsafe {
        let hdr_ptr: *mut qos_abi::shm::ShmSubmitIrHeader = VirtAddr::new(USER_ABI_SUBMITBUF_START).as_mut_ptr();
        core::ptr::write(
            hdr_ptr,
            qos_abi::shm::ShmSubmitIrHeader {
                version: qos_abi::shm::SUBMIT_HDR_VERSION,
                ir_format: qos_abi::shm::IRFMT_QASM2,
                n_qubits: USER_N_QUBITS,
                shots: USER_SHOTS,
                payload_len: USER_QASM2_BELL.len() as u32,
                _reserved: 0,
            },
        );

        let payload_dst: *mut u8 = VirtAddr::new(USER_ABI_SUBMITBUF_START + SUBMIT_HDR_LEN as u64).as_mut_ptr();
        core::ptr::copy_nonoverlapping(USER_QASM2_BELL.as_ptr(), payload_dst, USER_QASM2_BELL.len());
    }

    unsafe {
        // Copy quantum program into the mapped user page.
        let dst: *mut u8 = VirtAddr::new(USER_CODE_START).as_mut_ptr();
        serial::println!("[USER] Loading quantum program ({} bytes)", USER_PROG.len());
        core::ptr::copy_nonoverlapping(USER_PROG.as_ptr(), dst, USER_PROG.len());
    }

    let entry = VirtAddr::new(USER_CODE_START);
    let user_stack_top = VirtAddr::new(USER_STACK_START + 4096);
    unsafe { iretq_to_user(entry, user_stack_top) }
}

/// Enter user mode using the built-in Ring3 demo program.
///
/// This is callable from kernel subsystems (e.g. the shell) now that mapper/allocator
/// live in a global `memory::MemoryContext`.
pub fn exec_userdemo() -> ! {
    memory::with_ctx(|_mapper, frame_allocator| enter_user_mode(frame_allocator))
}

/// Enter user mode by loading an ELF64 (x86_64) image into user memory.
///
/// This is a first step toward a real process model: it loads a single ELF and
/// transfers control to its entrypoint. It does not return.
pub fn exec_elf(bytes: &[u8]) -> ! {
    memory::with_ctx(|_mapper, frame_allocator| enter_user_mode_elf(frame_allocator, bytes))
}

// ── Ring-3 preemption test (Phase 2.1b) ───────────────────────────────────────────────────

/// A Ring-3 process prepared for the preemptive scheduler. The fields are exactly what
/// `kthread::adopt_user` needs.
pub struct Ring3Handle {
    /// Saved kernel stack pointer pointing at a frame that `iretq`s into Ring 3.
    pub saved_rsp: u64,
    pub cr3: PhysFrame<Size4KiB>,
    /// Kernel stack top → TSS.RSP0 while this process runs.
    pub rsp0_top: VirtAddr,
}

/// Minimal Ring-3 payload: `jmp $` (spin forever). A runaway program with no syscalls and no
/// cooperative yield — the perfect stress test for preemption: if the OS stays responsive
/// while this runs, the timer is genuinely preempting Ring 3.
const SPIN_PROG: &[u8] = &[0xEB, 0xFE];

/// Ring-3 payload that runs a bounded busy loop, then dereferences address 0 (unmapped in its
/// address space) to trigger a page fault — a "crashing" program. Used to prove fault
/// isolation: the kernel kills only this process and keeps running.
#[rustfmt::skip]
const FAULT_PROG: &[u8] = &[
    0x48, 0xC7, 0xC1, 0x00, 0x00, 0x00, 0x01, // mov rcx, 0x01000000
    0x48, 0xFF, 0xC9,                         // dec rcx
    0x75, 0xFB,                               // jnz -5 (loop)
    0x8A, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00, // mov al, byte ptr [0]  -> page fault
    0xEB, 0xFE,                               // jmp $ (unreached)
];

/// Ring-3 payload that busy-loops then voluntarily exits via the OP_EXIT syscall
/// (`int 0x80` with op=4 in the shared ABI page). Proves clean process teardown.
#[rustfmt::skip]
const EXIT_PROG: &[u8] = &[
    0x48, 0xC7, 0xC1, 0x00, 0x00, 0x80, 0x00, // mov rcx, 0x00800000
    0x48, 0xFF, 0xC9,                         // dec rcx
    0x75, 0xFB,                               // jnz -5 (loop)
    // mov dword ptr [0x40010000], 1          ; abi_version (must match kernel)
    0xC7, 0x04, 0x25, 0x00, 0x00, 0x01, 0x40, 0x01, 0x00, 0x00, 0x00,
    // mov dword ptr [0x40010004], 4          ; op = OP_EXIT
    0xC7, 0x04, 0x25, 0x04, 0x00, 0x01, 0x40, 0x04, 0x00, 0x00, 0x00,
    0xCD, 0x80,                               // int 0x80
    0xEB, 0xFE,                               // jmp $ (unreached)
];

/// Kernel stacks for test Ring-3 processes, kept alive for the duration of the test.
static TEST_KSTACKS: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// Free the kernel stacks held for the Ring-3 preemption test. Call once the test is over.
pub fn clear_ring3_test_stacks() {
    TEST_KSTACKS.lock().clear();
}

/// Build a Ring-3 spinner process (infinite loop). Used by `proctest`.
pub fn spawn_ring3_spinner() -> Ring3Handle {
    spawn_ring3(SPIN_PROG)
}

/// Build a Ring-3 process that busy-loops then page-faults. Used by `faulttest` to prove the
/// kernel isolates and survives a crashing user process.
pub fn spawn_ring3_faulter() -> Ring3Handle {
    spawn_ring3(FAULT_PROG)
}

/// Ring-3 payload that attempts to write to its own (now read-execute) code page — a W^X
/// violation that must fault. Proves the W^X protection from Phase 2.3.
#[rustfmt::skip]
const WX_PROG: &[u8] = &[
    // mov byte ptr [0x40000000], 0x90    ; write into the read-only code page -> #PF
    0xC6, 0x04, 0x25, 0x00, 0x00, 0x00, 0x40, 0x90,
    0xEB, 0xFE,                            // jmp $ (unreached)
];

/// Build a Ring-3 process that violates W^X by writing to its code page. Used by `wxtest`.
pub fn spawn_ring3_wxviolator() -> Ring3Handle {
    spawn_ring3(WX_PROG)
}

/// Build a Ring-3 process that busy-loops then exits cleanly via OP_EXIT. Used by `exittest`.
pub fn spawn_ring3_exiter() -> Ring3Handle {
    spawn_ring3(EXIT_PROG)
}

/// Build a Ring-3 process that uses the register-based syscall ABI (int 0x81): it calls
/// SYS_WRITE to print a message, then SYS_EXIT. Used by `regabitest` (Phase 2.2). The program
/// is assembled at runtime because the message address depends on the code length.
pub fn spawn_ring3_regabi() -> Ring3Handle {
    let msg: &[u8] = b"hello from ring3 via register syscall ABI (int 0x81)\n";
    // Fixed-size prologue; the message bytes follow it in the same code page.
    const PROLOGUE_LEN: u64 = 26;
    let str_addr = (USER_CODE_START + PROLOGUE_LEN) as u32;

    let mut prog: Vec<u8> = Vec::with_capacity(PROLOGUE_LEN as usize + msg.len());
    prog.push(0xBF); // mov edi, imm32  (rdi = message ptr, zero-extended)
    prog.extend_from_slice(&str_addr.to_le_bytes());
    prog.push(0xBE); // mov esi, imm32  (rsi = length)
    prog.extend_from_slice(&(msg.len() as u32).to_le_bytes());
    prog.push(0xB8); // mov eax, imm32  (rax = SYS_WRITE = 1)
    prog.extend_from_slice(&1u32.to_le_bytes());
    prog.extend_from_slice(&[0xCD, 0x81]); // int 0x81
    prog.push(0xB8); // mov eax, imm32  (rax = SYS_EXIT = 0)
    prog.extend_from_slice(&0u32.to_le_bytes());
    prog.extend_from_slice(&[0xCD, 0x81]); // int 0x81
    prog.extend_from_slice(&[0xEB, 0xFE]); // jmp $ (unreached)
    debug_assert_eq!(prog.len() as u64, PROLOGUE_LEN);
    prog.extend_from_slice(msg);

    spawn_ring3(&prog)
}

/// Build a Ring-3 process running `payload` in its own address space, ready to hand to the
/// preemptive scheduler (Phase 2.1b).
fn spawn_ring3(payload: &[u8]) -> Ring3Handle {
    memory::with_ctx(|_mapper, fa| {
        // Fresh address space (shares kernel mappings; P4[0] is the private user region).
        let (cr3, mut um) = memory::create_user_pagetable(fa);
        map_vga_identity(&mut um, fa);

        let code_page = Page::containing_address(VirtAddr::new(USER_CODE_START));
        let stack_page = Page::containing_address(VirtAddr::new(USER_STACK_START));
        let abi_page = Page::containing_address(VirtAddr::new(USER_ABI_CALL_START));
        map_user_page(&mut um, fa, code_page, false); // executable code
        map_user_page(&mut um, fa, stack_page, true); // NX data stack
        map_user_page(&mut um, fa, abi_page, true); // shared syscall ABI page (NX)

        // Copy the payload into the code page. It is only mapped in the new CR3, so switch
        // there to write, then return to the kernel CR3.
        let saved = memory::switch_cr3(cr3);
        unsafe {
            let dst: *mut u8 = VirtAddr::new(USER_CODE_START).as_mut_ptr();
            core::ptr::copy_nonoverlapping(payload.as_ptr(), dst, payload.len());
        }
        memory::switch_cr3(saved);

        // W^X (Phase 2.3): after loading, demote the code page to read-execute (drop WRITABLE).
        // A process that tries to modify its own code now faults instead of self-modifying.
        // (The stack and ABI pages stay NX data; the page just below the stack is left unmapped
        // as an implicit guard page so a stack overflow faults rather than corrupting memory.)
        unsafe {
            let rx = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
            if let Ok(flush) = um.update_flags(code_page, rx) {
                flush.flush();
            }
        }

        // Allocate a kernel stack and build the iretq-to-Ring3 frame on it (same layout as
        // tasking::spawn_user_process and the asm timer ISR's save/restore).
        let mut kstack = alloc::vec![0u8; 4096 * 8];
        let base = kstack.as_ptr() as u64;
        let top = (base + kstack.len() as u64) & !0xF;

        let sel = gdt::selectors();
        let user_cs = (sel.user_code.0 | 0b11) as u64;
        let user_ss = (sel.user_data.0 | 0b11) as u64;
        let rflags: u64 = 0x202; // IF set
        let user_rsp = USER_STACK_START + 4096;

        unsafe fn push(sp: &mut u64, v: u64) {
            *sp -= 8;
            *(*sp as *mut u64) = v;
        }
        let mut sp = top;
        unsafe {
            push(&mut sp, user_ss);
            push(&mut sp, user_rsp);
            push(&mut sp, rflags);
            push(&mut sp, user_cs);
            push(&mut sp, USER_CODE_START); // rip
            for _ in 0..15 {
                push(&mut sp, 0); // r15..rax
            }
        }
        let saved_rsp = sp;
        let rsp0_top = VirtAddr::new(top);

        TEST_KSTACKS.lock().push(kstack);
        Ring3Handle { saved_rsp, cr3, rsp0_top }
    })
}

fn enter_user_mode_elf(
    frame_allocator: &mut memory::BootInfoFrameAllocator,
    bytes: &[u8],
) -> ! {
    vga::println!("loading ELF into user mode...");
    serial::println!("loading ELF into user mode...");

    // Create a fresh user address space (shares kernel mappings).
    let (user_cr3, mut user_mapper) = memory::create_user_pagetable(frame_allocator);

    // Keep kernel VGA output working while the user CR3 is active.
    map_vga_identity(&mut user_mapper, frame_allocator);

    // Map a dedicated stack + ABI page (shared call frame).
    let stack_page = Page::containing_address(VirtAddr::new(USER_ELF_STACK_START));
    let abi_page = Page::containing_address(VirtAddr::new(USER_ABI_CALL_START));
    map_user_page(&mut user_mapper, frame_allocator, stack_page, true);
    map_user_page(&mut user_mapper, frame_allocator, abi_page, true);

    // Basic ELF validation.
    let info = match elf::parse_elf64(bytes) {
        Ok(i) => i,
        Err(e) => {
            vga::println!("ELF parse error: {:?}", e);
            serial::println!("ELF parse error: {:?}", e);
            loop {
                crate::arch::hlt();
            }
        }
    };

    // Load PT_LOAD segments.
    let segs = match elf::iter_load_segments(bytes) {
        Ok(it) => it,
        Err(e) => {
            vga::println!("ELF phdr error: {:?}", e);
            serial::println!("ELF phdr error: {:?}", e);
            loop {
                crate::arch::hlt();
            }
        }
    };

    let mut copy_plan: Vec<(u64, usize, usize, usize)> = Vec::new();

    for ph in segs {
        let file_off = ph.p_offset as usize;
        let file_sz = ph.p_filesz as usize;
        let mem_sz = ph.p_memsz as usize;

        if file_off.checked_add(file_sz).map(|e| e <= bytes.len()).unwrap_or(false) == false {
            vga::println!("ELF segment out of range");
            serial::println!("ELF segment out of range");
            loop {
                crate::arch::hlt();
            }
        }

        let vaddr = ph.p_vaddr;
        // Minimal safety guard: keep user segments in a low user region.
        if vaddr < USER_CODE_START || vaddr >= 0x0000_0000_5000_0000 {
            vga::println!("ELF vaddr rejected: 0x{:x}", vaddr);
            serial::println!("ELF vaddr rejected: 0x{:x}", vaddr);
            loop {
                crate::arch::hlt();
            }
        }

        // Map pages for [vaddr, vaddr+mem_sz).
        // Important: if mem_sz ends on a page boundary, we must *not* map the next page.
        if mem_sz == 0 {
            continue;
        }
        let start = VirtAddr::new(vaddr);
        let end_inclusive = VirtAddr::new(vaddr + (mem_sz as u64) - 1);
        let start_page = Page::containing_address(start);
        let end_page = Page::containing_address(end_inclusive);

        // p_flags: PF_X=1, PF_W=2, PF_R=4
        let is_exec = (ph.p_flags & 0x1) != 0;
        let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        flags |= PageTableFlags::WRITABLE; // MVP: keep it simple
        if !is_exec {
            flags |= PageTableFlags::NO_EXECUTE;
        }

        for page in Page::range_inclusive(start_page, end_page) {
            map_user_page_with_flags(&mut user_mapper, frame_allocator, page, flags);
        }

        copy_plan.push((vaddr, file_off, file_sz, mem_sz));
    }

    // Switch to the user page table before touching user virtual addresses.
    memory::switch_cr3(user_cr3);

    for (vaddr, file_off, file_sz, mem_sz) in copy_plan {
        unsafe {
            let dst: *mut u8 = VirtAddr::new(vaddr).as_mut_ptr();
            let src = &bytes[file_off..file_off + file_sz];
            core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
            if mem_sz > file_sz {
                core::ptr::write_bytes(dst.add(file_sz), 0, mem_sz - file_sz);
            }
        }
    }

    let entry = info.entry;
    let user_stack_top = VirtAddr::new(USER_ELF_STACK_START + 4096);
    unsafe { iretq_to_user(entry, user_stack_top) }
}

fn map_user_page(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    page: Page<Size4KiB>,
    no_execute: bool,
) {
    // Be tolerant of callers mapping the same page twice.
    if mapper.translate_page(page).is_ok() {
        return;
    }

    let frame = frame_allocator
        .allocate_frame()
        .expect("no frames left for user mapping");

    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;
    if no_execute {
        flags |= PageTableFlags::NO_EXECUTE;
    }

    unsafe {
        mapper
            .map_to(page, frame, flags, frame_allocator)
            .expect("map_to failed (map_user_page)")
            .flush();
    }

    track_user_page(page);
}

fn map_user_page_tracked(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    page: Page<Size4KiB>,
    no_execute: bool,
    pages: &mut Vec<Page<Size4KiB>>,
) {
    // Be tolerant of callers mapping the same page twice.
    if mapper.translate_page(page).is_ok() {
        track_page_into(pages, page);
        return;
    }

    let frame = frame_allocator
        .allocate_frame()
        .expect("no frames left for user mapping");

    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;
    if no_execute {
        flags |= PageTableFlags::NO_EXECUTE;
    }

    unsafe {
        mapper
            .map_to(page, frame, flags, frame_allocator)
            .expect("map_to failed (map_user_page_tracked)")
            .flush();
    }

    track_page_into(pages, page);
}

fn map_user_page_with_flags(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    page: Page<Size4KiB>,
    flags: PageTableFlags,
) {
    // PT_LOAD segments may overlap on a page boundary (e.g., .text/.rodata).
    // For MVP we just reuse an existing mapping.
    if mapper.translate_page(page).is_ok() {
        return;
    }

    let frame = frame_allocator
        .allocate_frame()
        .expect("no frames left for user mapping");

    unsafe {
        mapper
            .map_to(page, frame, flags, frame_allocator)
            .expect("map_to failed (map_user_page_with_flags)")
            .flush();
    }

    track_user_page(page);
}

fn map_user_page_with_flags_tracked(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    page: Page<Size4KiB>,
    flags: PageTableFlags,
    pages: &mut Vec<Page<Size4KiB>>,
) {
    // PT_LOAD segments may overlap on a page boundary (e.g., .text/.rodata).
    // For MVP we just reuse an existing mapping.
    if mapper.translate_page(page).is_ok() {
        track_page_into(pages, page);
        return;
    }

    let frame = frame_allocator
        .allocate_frame()
        .expect("no frames left for user mapping");

    unsafe {
        mapper
            .map_to(page, frame, flags, frame_allocator)
            .expect("map_to failed (map_user_page_with_flags_tracked)")
            .flush();
    }

    track_page_into(pages, page);
}

/// Prepare a user-mode process by loading an ELF64 image into a fresh per-process address space.
///
/// Unlike `exec_elf`, this does not transfer control; it returns a `SpawnedUserProcess` that can
/// be scheduled later.
pub fn spawn_elf_process(
    frame_allocator: &mut memory::BootInfoFrameAllocator,
    bytes: &[u8],
) -> Result<SpawnedUserProcess, &'static str> {
    // Create a fresh user address space (shares kernel mappings).
    let (user_cr3, mut user_mapper) = memory::create_user_pagetable(frame_allocator);

    // Keep kernel VGA output working while the user CR3 is active.
    map_vga_identity(&mut user_mapper, frame_allocator);

    let mut mapped_pages: Vec<Page<Size4KiB>> = Vec::new();

    // Map a dedicated stack + ABI page (shared call frame).
    let stack_page = Page::containing_address(VirtAddr::new(USER_ELF_STACK_START));
    let abi_page = Page::containing_address(VirtAddr::new(USER_ABI_CALL_START));
    map_user_page_tracked(&mut user_mapper, frame_allocator, stack_page, true, &mut mapped_pages);
    map_user_page_tracked(&mut user_mapper, frame_allocator, abi_page, true, &mut mapped_pages);

    let info = elf::parse_elf64(bytes).map_err(|_| "ELF parse error")?;
    let segs = elf::iter_load_segments(bytes).map_err(|_| "ELF phdr error")?;

    let mut copy_plan: Vec<(u64, usize, usize, usize)> = Vec::new();
    for ph in segs {
        let file_off = ph.p_offset as usize;
        let file_sz = ph.p_filesz as usize;
        let mem_sz = ph.p_memsz as usize;

        let in_range = file_off
            .checked_add(file_sz)
            .map(|e| e <= bytes.len())
            .unwrap_or(false);
        if !in_range {
            return Err("ELF segment out of range");
        }

        let vaddr = ph.p_vaddr;
        if vaddr < USER_CODE_START || vaddr >= 0x0000_0000_5000_0000 {
            return Err("ELF vaddr rejected");
        }

        if mem_sz == 0 {
            continue;
        }

        let start = VirtAddr::new(vaddr);
        let end_inclusive = VirtAddr::new(vaddr + (mem_sz as u64) - 1);
        let start_page = Page::containing_address(start);
        let end_page = Page::containing_address(end_inclusive);

        // p_flags: PF_X=1, PF_W=2, PF_R=4
        let is_exec = (ph.p_flags & 0x1) != 0;
        let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        flags |= PageTableFlags::WRITABLE; // MVP: keep it simple
        if !is_exec {
            flags |= PageTableFlags::NO_EXECUTE;
        }

        for page in Page::range_inclusive(start_page, end_page) {
            map_user_page_with_flags_tracked(&mut user_mapper, frame_allocator, page, flags, &mut mapped_pages);
        }

        copy_plan.push((vaddr, file_off, file_sz, mem_sz));
    }

    // Switch to the user page table before touching user virtual addresses.
    memory::switch_cr3(user_cr3);

    for (vaddr, file_off, file_sz, mem_sz) in copy_plan {
        unsafe {
            let dst: *mut u8 = VirtAddr::new(vaddr).as_mut_ptr();
            let src = &bytes[file_off..file_off + file_sz];
            core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
            if mem_sz > file_sz {
                core::ptr::write_bytes(dst.add(file_sz), 0, mem_sz - file_sz);
            }
        }
    }

    // Return to the kernel address space; the scheduler will switch back to user_cr3 when running.
    memory::switch_to_kernel_cr3();

    let entry = info.entry;
    let user_stack_top = VirtAddr::new(USER_ELF_STACK_START + 4096);
    Ok(SpawnedUserProcess {
        user_cr3,
        entry,
        user_stack_top,
        mapped_pages,
    })
}

/// Prepare a scheduled user-mode process using the built-in Ring3 demo program.
///
/// This is handy for experimentation: it doesn't require an external ELF binary.
pub fn spawn_userdemo_process(
    frame_allocator: &mut memory::BootInfoFrameAllocator,
) -> Result<SpawnedUserProcess, &'static str> {
    // Create a fresh user address space (shares kernel mappings).
    let (user_cr3, mut user_mapper) = memory::create_user_pagetable(frame_allocator);

    // Keep kernel VGA output working while the user CR3 is active.
    map_vga_identity(&mut user_mapper, frame_allocator);

    let mut mapped_pages: Vec<Page<Size4KiB>> = Vec::new();

    let code_page = Page::containing_address(VirtAddr::new(USER_CODE_START));
    let stack_page = Page::containing_address(VirtAddr::new(USER_STACK_START));
    let abi_page = Page::containing_address(VirtAddr::new(USER_ABI_CALL_START));

    map_user_page_tracked(&mut user_mapper, frame_allocator, code_page, false, &mut mapped_pages);
    map_user_page_tracked(&mut user_mapper, frame_allocator, stack_page, true, &mut mapped_pages);
    map_user_page_tracked(&mut user_mapper, frame_allocator, abi_page, true, &mut mapped_pages);

    // Switch to the user page table before touching user virtual addresses.
    memory::switch_cr3(user_cr3);

    // Preload submit buffer (header + QASM2 payload) into the user ABI page.
    unsafe {
        let hdr_ptr: *mut qos_abi::shm::ShmSubmitIrHeader =
            VirtAddr::new(USER_ABI_SUBMITBUF_START).as_mut_ptr();
        core::ptr::write(
            hdr_ptr,
            qos_abi::shm::ShmSubmitIrHeader {
                version: qos_abi::shm::SUBMIT_HDR_VERSION,
                ir_format: qos_abi::shm::IRFMT_QASM2,
                n_qubits: USER_N_QUBITS,
                shots: USER_SHOTS,
                payload_len: USER_QASM2_BELL.len() as u32,
                _reserved: 0,
            },
        );

        let payload_dst: *mut u8 = VirtAddr::new(USER_ABI_SUBMITBUF_START + SUBMIT_HDR_LEN as u64)
            .as_mut_ptr();
        core::ptr::copy_nonoverlapping(USER_QASM2_BELL.as_ptr(), payload_dst, USER_QASM2_BELL.len());
    }

    // Copy code into the mapped user page.
    unsafe {
        let dst: *mut u8 = VirtAddr::new(USER_CODE_START).as_mut_ptr();
        core::ptr::copy_nonoverlapping(USER_PROG.as_ptr(), dst, USER_PROG.len());
    }

    // Return to the kernel address space; the scheduler will switch back to user_cr3 when running.
    memory::switch_to_kernel_cr3();

    Ok(SpawnedUserProcess {
        user_cr3,
        entry: VirtAddr::new(USER_CODE_START),
        user_stack_top: VirtAddr::new(USER_STACK_START + 4096),
        mapped_pages,
    })
}

pub fn cleanup_spawned_user_process(
    user_cr3: PhysFrame<Size4KiB>,
    mapped_pages: &mut Vec<Page<Size4KiB>>,
) {
    // Switch to the target address space so we can unmap by virtual page.
    memory::switch_cr3(user_cr3);

    let pages = core::mem::take(mapped_pages);
    let mut mapper = unsafe { memory::init(memory::phys_offset()) };
    for page in pages {
        if mapper.translate_page(page).is_err() {
            continue;
        }
        unsafe {
            if let Ok((frame, flush)) = mapper.unmap(page) {
                flush.flush();
                memory::with_ctx(|_, frame_allocator| {
                    frame_allocator.deallocate_frame(frame);
                });
            }
        }
    }

    // Free page-table frames for the user subtree (P4[0]) while still on this CR3.
    let p4 = unsafe {
        let virt = memory::phys_offset() + user_cr3.start_address().as_u64();
        &mut *(virt.as_mut_ptr::<x86_64::structures::paging::PageTable>())
    };

    let entry0 = p4[0].clone();
    p4[0].set_unused();

    fn free_table_level(table_frame: PhysFrame<Size4KiB>, level: u8) {
        let table = unsafe {
            let virt = memory::phys_offset() + table_frame.start_address().as_u64();
            &mut *(virt.as_mut_ptr::<x86_64::structures::paging::PageTable>())
        };

        if level > 1 {
            for entry in table.iter_mut() {
                if !entry.flags().contains(PageTableFlags::PRESENT) {
                    continue;
                }
                if entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                    entry.set_unused();
                    continue;
                }
                if let Ok(child) = entry.frame() {
                    entry.set_unused();
                    free_table_level(child, level - 1);
                } else {
                    entry.set_unused();
                }
            }
        } else {
            for entry in table.iter_mut() {
                entry.set_unused();
            }
        }

        memory::with_ctx(|_, frame_allocator| {
            frame_allocator.deallocate_frame(table_frame);
        });
    }

    if entry0.flags().contains(PageTableFlags::PRESENT) {
        if !entry0.flags().contains(PageTableFlags::HUGE_PAGE) {
            if let Ok(p3) = entry0.frame() {
                free_table_level(p3, 3);
            }
        }
    }

    memory::switch_to_kernel_cr3();
    memory::with_ctx(|_, frame_allocator| {
        frame_allocator.deallocate_frame(user_cr3);
    });
}

unsafe fn iretq_to_user(entry: VirtAddr, user_stack_top: VirtAddr) -> ! {
    use x86_64::PrivilegeLevel;

    let sel = gdt::selectors();
    let user_cs = x86_64::structures::gdt::SegmentSelector::new(sel.user_code.index(), PrivilegeLevel::Ring3);
    let user_ss = x86_64::structures::gdt::SegmentSelector::new(sel.user_data.index(), PrivilegeLevel::Ring3);

    // RFLAGS: IF=1 (interrupts enabled), IOPL=0, Reserved bit 1 always set
    let rflags: u64 = 0x202;
    
    let ss_val = user_ss.0 as u64;
    let cs_val = user_cs.0 as u64;
    let rsp_val = user_stack_top.as_u64();
    let rip_val = entry.as_u64();
    
    serial::println!("[USER] Entering Ring 3 at {:#x}", rip_val);
    
    asm_iretq_to_user(rip_val, rsp_val, cs_val, ss_val, rflags);
}

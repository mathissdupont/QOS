use bootloader::bootinfo::{MemoryMap, MemoryRegionType};
use lazy_static::lazy_static;
use spin::Mutex;
use alloc::vec::Vec;
use x86_64::{
    PhysAddr, VirtAddr,
    registers::control::Cr3,
    structures::paging::{FrameAllocator, OffsetPageTable, PageTable, PhysFrame, Size4KiB},
};

pub struct MemoryContext {
    pub mapper: OffsetPageTable<'static>,
    pub frame_allocator: BootInfoFrameAllocator,
}

lazy_static! {
    static ref MEMORY_CTX: Mutex<Option<MemoryContext>> = Mutex::new(None);
    static ref PHYS_OFFSET: Mutex<Option<VirtAddr>> = Mutex::new(None);
    static ref KERNEL_CR3: Mutex<Option<PhysFrame<Size4KiB>>> = Mutex::new(None);
}

pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let level_4_table = active_level_4_table(physical_memory_offset);
    OffsetPageTable::new(level_4_table, physical_memory_offset)
}

pub unsafe fn init_global(physical_memory_offset: VirtAddr, memory_map: &'static MemoryMap) {
    *PHYS_OFFSET.lock() = Some(physical_memory_offset);
    let (kcr3, _) = Cr3::read();
    *KERNEL_CR3.lock() = Some(kcr3);

    let mapper = init(physical_memory_offset);
    let frame_allocator = BootInfoFrameAllocator::init(memory_map);
    *MEMORY_CTX.lock() = Some(MemoryContext {
        mapper,
        frame_allocator,
    });
}

pub fn phys_offset() -> VirtAddr {
    PHYS_OFFSET.lock().expect("phys offset not initialized")
}

pub fn kernel_cr3_frame() -> PhysFrame<Size4KiB> {
    KERNEL_CR3.lock().expect("kernel cr3 not initialized")
}

pub fn current_cr3_frame() -> PhysFrame<Size4KiB> {
    let (f, _) = Cr3::read();
    f
}

pub fn switch_cr3(new_frame: PhysFrame<Size4KiB>) -> PhysFrame<Size4KiB> {
    let (old, flags) = Cr3::read();
    // Keep flags unchanged (e.g., PCID off here), just swap the frame.
    unsafe { Cr3::write(new_frame, flags) };
    old
}

pub fn switch_to_kernel_cr3() {
    let k = kernel_cr3_frame();
    let (_, flags) = Cr3::read();
    unsafe { Cr3::write(k, flags) };
}

fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    phys_offset() + phys.as_u64()
}

/// Create a new page table (P4) for a user process.
///
/// To allow the kernel to keep running while a user CR3 is active (interrupts, traps,
/// kernel stacks, etc.), we copy all current P4 entries **except** index 0.
///
/// Index 0 is reserved for per-process user mappings (e.g., 0x4000_0000..).
pub fn create_user_pagetable(
    frame_allocator: &mut BootInfoFrameAllocator,
) -> (PhysFrame<Size4KiB>, OffsetPageTable<'static>) {
    let new_p4 = frame_allocator
        .allocate_frame()
        .expect("no frames left for user page table");

    let cur_p4 = unsafe { active_level_4_table(phys_offset()) };
    let new_p4_table: &mut PageTable = unsafe {
        let virt = phys_to_virt(new_p4.start_address());
        &mut *(virt.as_mut_ptr())
    };

    // Zero and copy everything except P4[0] (reserved for per-process user space).
    *new_p4_table = PageTable::new();
    for i in 1..512 {
        new_p4_table[i] = cur_p4[i].clone();
    }

    let mapper = unsafe { OffsetPageTable::new(new_p4_table, phys_offset()) };
    (new_p4, mapper)
}

pub fn with_ctx<R>(f: impl FnOnce(&mut OffsetPageTable<'static>, &mut BootInfoFrameAllocator) -> R) -> R {
    let mut guard = MEMORY_CTX.lock();
    let ctx = guard.as_mut().expect("memory ctx not initialized");
    f(&mut ctx.mapper, &mut ctx.frame_allocator)
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();
    &mut *page_table_ptr
}

pub struct BootInfoFrameAllocator {
    memory_map: &'static MemoryMap,
    next: usize,
    recycled: Vec<PhysFrame<Size4KiB>>,
}

impl BootInfoFrameAllocator {
    pub unsafe fn init(memory_map: &'static MemoryMap) -> Self {
        BootInfoFrameAllocator {
            memory_map,
            next: 0,
            recycled: Vec::new(),
        }
    }

    pub fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        // MVP: simple LIFO recycle list.
        self.recycled.push(frame);
    }

    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame<Size4KiB>> {
        let regions = self.memory_map.iter();
        let usable = regions.filter(|r| r.region_type == MemoryRegionType::Usable);

        let frame_numbers = usable.flat_map(|r| r.range.start_frame_number..r.range.end_frame_number);

        frame_numbers.map(|frame_number| {
            let addr = PhysAddr::new(frame_number * 4096);
            PhysFrame::containing_address(addr)
        })
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        if let Some(frame) = self.recycled.pop() {
            return Some(frame);
        }
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}

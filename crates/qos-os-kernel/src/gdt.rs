use lazy_static::lazy_static;
use core::cell::UnsafeCell;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

struct TssCell(UnsafeCell<TaskStateSegment>);

// SAFETY: This kernel currently runs on a single core and we only mutate the TSS in
// controlled places (during scheduling). Interior mutability is required to update RSP0.
unsafe impl Sync for TssCell {}

static TSS: TssCell = TssCell(UnsafeCell::new(TaskStateSegment::new()));

pub fn set_rsp0(rsp0_end: VirtAddr) {
    unsafe {
        (*TSS.0.get()).privilege_stack_table[0] = rsp0_end;
    }
}

lazy_static! {
    static ref GDT: (GlobalDescriptorTable, Selectors) = {
        // Initialize the global TSS once, before installing it into the GDT.
        unsafe {
            const DF_STACK_SIZE: usize = 4096 * 5;
            static mut DF_STACK: [u8; DF_STACK_SIZE] = [0; DF_STACK_SIZE];

            let df_start = VirtAddr::from_ptr(&raw const DF_STACK);
            let df_end = df_start + DF_STACK_SIZE as u64;
            (*TSS.0.get()).interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = df_end;

            // Default RSP0 used when transitioning from Ring 3 to Ring 0.
            const RSP0_STACK_SIZE: usize = 4096 * 5;
            static mut RSP0_STACK: [u8; RSP0_STACK_SIZE] = [0; RSP0_STACK_SIZE];
            let rsp0_start = VirtAddr::from_ptr(&raw const RSP0_STACK);
            let rsp0_end = rsp0_start + RSP0_STACK_SIZE as u64;
            (*TSS.0.get()).privilege_stack_table[0] = rsp0_end;
        }

        let mut gdt = GlobalDescriptorTable::new();
        let kernel_code_selector = gdt.append(Descriptor::kernel_code_segment());
        let user_code_selector = gdt.append(Descriptor::user_code_segment());
        let user_data_selector = gdt.append(Descriptor::user_data_segment());
        let tss_selector = gdt.append(Descriptor::tss_segment(unsafe { &*TSS.0.get() }));
        (
            gdt,
            Selectors {
                kernel_code: kernel_code_selector,
                user_code: user_code_selector,
                user_data: user_data_selector,
                tss: tss_selector,
            },
        )
    };
}

#[derive(Clone, Copy)]
pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub user_code: SegmentSelector,
    pub user_data: SegmentSelector,
    pub tss: SegmentSelector,
}

pub fn init() {
    use x86_64::instructions::segmentation::{CS, Segment};
    use x86_64::instructions::tables::load_tss;

    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1.kernel_code);
        load_tss(GDT.1.tss);
    }
}

pub fn selectors() -> Selectors {
    GDT.1
}

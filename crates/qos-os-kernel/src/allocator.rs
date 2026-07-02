use core::alloc::Layout;

use linked_list_allocator::LockedHeap;
use x86_64::{
    VirtAddr,
    structures::paging::{
        FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB,
    },
};

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub const HEAP_START: usize = 0x4444_4444_0000;
// 64 MiB — headroom for the modern UI (WP-05): a native-resolution true-color back buffer is
// ~4 MB at 1280×800 and ~8 MB at 1920×1080, plus per-window surfaces, the glyph cache, and app
// state. Mapped eagerly at boot; fits comfortably on any machine with ≥256 MiB RAM.
pub const HEAP_SIZE: usize = 64 * 1024 * 1024;

pub fn init_heap(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), &'static str> {
    let heap_start = VirtAddr::new(HEAP_START as u64);
    let heap_end = heap_start + (HEAP_SIZE as u64 - 1);

    let start_page: Page<Size4KiB> = Page::containing_address(heap_start);
    let end_page: Page<Size4KiB> = Page::containing_address(heap_end);

    for page in Page::range_inclusive(start_page, end_page) {
        let frame = frame_allocator
            .allocate_frame()
            .ok_or("frame allocation failed")?;

        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;

        unsafe {
            mapper
                .map_to(page, frame, flags, frame_allocator)
                .map_err(|_| "map_to failed")?
                .flush();
        }
    }

    unsafe {
        ALLOCATOR.lock().init(HEAP_START as *mut u8, HEAP_SIZE);
    }

    Ok(())
}

/// Current heap usage as `(used_bytes, total_bytes)` — surfaced by the System Monitor app.
pub fn heap_stats() -> (usize, usize) {
    let h = ALLOCATOR.lock();
    (h.used(), HEAP_SIZE)
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    panic!("allocation error: {:?}", layout)
}

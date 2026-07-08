//! CPU security hardening (ADR-0020): enable and report the x86-64 memory-protection features a
//! modern OS is expected to run with.
//!
//! - **NXE** (EFER bit 11): no-execute page protection — data pages can be marked non-executable.
//! - **WP** (CR0 bit 16): the kernel honors read-only pages even in ring 0.
//! - **SMEP** (CR4 bit 20): the kernel cannot *execute* user-mode pages.
//! - **SMAP** (CR4 bit 21): the kernel cannot *access* user-mode data unless explicitly allowed.
//!
//! SMEP/SMAP are gated on CPUID leaf 7 feature bits (universal: read what the CPU reports, never
//! assume a model). Everything is best-effort and logged; a missing feature is reported, not fatal.

use core::sync::atomic::{AtomicU8, Ordering};

const F_NX: u8 = 1 << 0;
const F_WP: u8 = 1 << 1;
const F_SMEP: u8 = 1 << 2;
const F_SMAP: u8 = 1 << 3;

/// Bitmask of the protections that are active (filled in by [`init`]).
static ACTIVE: AtomicU8 = AtomicU8::new(0);

/// Enable every supported protection and record + log the result. Call once at boot (after
/// paging is up; before user-facing subsystems).
pub fn init() {
    use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};
    use x86_64::registers::model_specific::{Efer, EferFlags};

    let mut active = 0u8;

    // NXE: usually already set by the bootloader; make sure.
    unsafe {
        Efer::update(|f| f.insert(EferFlags::NO_EXECUTE_ENABLE));
    }
    if Efer::read().contains(EferFlags::NO_EXECUTE_ENABLE) {
        active |= F_NX;
    }

    // CR0.WP: ring-0 honors read-only mappings.
    unsafe {
        Cr0::update(|f| f.insert(Cr0Flags::WRITE_PROTECT));
    }
    if Cr0::read().contains(Cr0Flags::WRITE_PROTECT) {
        active |= F_WP;
    }

    // SMEP/SMAP: gated on CPUID.(EAX=7,ECX=0):EBX bits 7 / 20.
    let leaf7 = unsafe { core::arch::x86_64::__cpuid_count(7, 0) };
    let has_smep = leaf7.ebx & (1 << 7) != 0;
    let has_smap = leaf7.ebx & (1 << 20) != 0;
    unsafe {
        Cr4::update(|f| {
            if has_smep {
                f.insert(Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION);
            }
            if has_smap {
                f.insert(Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION);
            }
        });
    }
    let cr4 = Cr4::read();
    if cr4.contains(Cr4Flags::SUPERVISOR_MODE_EXECUTION_PROTECTION) {
        active |= F_SMEP;
    }
    if cr4.contains(Cr4Flags::SUPERVISOR_MODE_ACCESS_PREVENTION) {
        active |= F_SMAP;
    }

    ACTIVE.store(active, Ordering::Relaxed);
    crate::serial_println!(
        "[SEC] hardening: NX {}  WP {}  SMEP {}  SMAP {}",
        if active & F_NX != 0 { "on" } else { "OFF" },
        if active & F_WP != 0 { "on" } else { "OFF" },
        if active & F_SMEP != 0 { "on" } else { "unsupported" },
        if active & F_SMAP != 0 { "on" } else { "unsupported" },
    );
    // W^X audit (WP-08 s2): count writable+executable mappings after our own mappings exist.
    wx_audit();
}

/// W+X page count from the boot-time audit (usize::MAX until the audit has run).
static WX_PAGES: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(usize::MAX);

/// Walk the active 4-level page table and count **W+X** leaf mappings (PRESENT + WRITABLE and
/// not NO_EXECUTE) — pages where injected data could be executed. Counts 4 KiB units so huge
/// pages weigh their true size. Read-only analysis; safe any time after paging is up.
///
/// Returns `(wx_4k_pages, total_mapped_4k_pages)` and caches the W+X count for the UI.
pub fn wx_audit() -> (usize, usize) {
    use x86_64::registers::control::Cr3;
    use x86_64::structures::paging::PageTableFlags as F;

    let phys_off = crate::memory::phys_offset().as_u64();
    let table = |phys: u64| -> &'static [u64; 512] {
        unsafe { &*((phys_off + phys) as *const [u64; 512]) }
    };
    let addr_mask: u64 = 0x000f_ffff_ffff_f000;

    let (l4_frame, _) = Cr3::read();
    let l4 = table(l4_frame.start_address().as_u64());
    let (mut wx, mut total) = (0usize, 0usize);

    let is_wx = |e: u64| -> bool {
        let f = F::from_bits_truncate(e);
        f.contains(F::PRESENT) && f.contains(F::WRITABLE) && !f.contains(F::NO_EXECUTE)
    };

    for &e4 in l4.iter() {
        if e4 & 1 == 0 {
            continue;
        }
        let l3 = table(e4 & addr_mask);
        for &e3 in l3.iter() {
            if e3 & 1 == 0 {
                continue;
            }
            if e3 & (1 << 7) != 0 {
                // 1 GiB page = 262144 × 4 KiB.
                total += 262_144;
                if is_wx(e3) {
                    wx += 262_144;
                }
                continue;
            }
            let l2 = table(e3 & addr_mask);
            for &e2 in l2.iter() {
                if e2 & 1 == 0 {
                    continue;
                }
                if e2 & (1 << 7) != 0 {
                    // 2 MiB page = 512 × 4 KiB.
                    total += 512;
                    if is_wx(e2) {
                        wx += 512;
                    }
                    continue;
                }
                let l1 = table(e2 & addr_mask);
                for &e1 in l1.iter() {
                    if e1 & 1 == 0 {
                        continue;
                    }
                    total += 1;
                    if is_wx(e1) {
                        wx += 1;
                    }
                }
            }
        }
    }

    WX_PAGES.store(wx, Ordering::Relaxed);
    crate::serial_println!(
        "[SEC] W^X audit: {} of {} mapped 4K-pages are W+X ({} MiB)",
        wx,
        total,
        wx * 4 / 1024
    );
    (wx, total)
}

/// Cached W+X count for UI display (`None` before the audit ran).
pub fn wx_pages() -> Option<usize> {
    let v = WX_PAGES.load(Ordering::Relaxed);
    if v == usize::MAX { None } else { Some(v) }
}

/// Human-readable one-line status for the UI (Settings / Monitor).
pub fn status_line() -> alloc::string::String {
    let a = ACTIVE.load(Ordering::Relaxed);
    let mark = |bit: u8, name: &str| {
        if a & bit != 0 {
            alloc::format!("{} on", name)
        } else {
            alloc::format!("{} off", name)
        }
    };
    let wx = match wx_pages() {
        Some(0) => alloc::string::String::from("W^X clean"),
        Some(n) => alloc::format!("W+X {} pg", n),
        None => alloc::string::String::from("W^X n/a"),
    };
    alloc::format!(
        "{} · {} · {} · {} · {}",
        mark(F_NX, "NX"),
        mark(F_WP, "WP"),
        mark(F_SMEP, "SMEP"),
        mark(F_SMAP, "SMAP"),
        wx
    )
}

//! QOS image builder (bootloader 0.11). `build.rs` creates the BIOS + UEFI disk images from the
//! kernel ELF; this binary prints their paths (and can copy them out / launch QEMU).

use std::path::Path;

const BIOS_IMAGE: &str = env!("QOS_BIOS_IMAGE");
const UEFI_IMAGE: &str = env!("QOS_UEFI_IMAGE");

fn main() {
    println!("QOS bootable images:");
    println!("  BIOS: {BIOS_IMAGE}");
    println!("  UEFI: {UEFI_IMAGE}");

    // Copy them to dist/ for convenience (best-effort).
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let dist = Path::new(&manifest).join("../../dist");
        let _ = std::fs::create_dir_all(&dist);
        let _ = std::fs::copy(BIOS_IMAGE, dist.join("qos-bios.img"));
        let _ = std::fs::copy(UEFI_IMAGE, dist.join("qos-uefi.img"));
        println!("Copied to {}", dist.display());
    }
}

//! QOS image builder (bootloader 0.11). `build.rs` creates the UEFI disk image from the kernel
//! ELF; this binary prints its path and copies it to `dist/`.

use std::path::Path;

const UEFI_IMAGE: &str = env!("QOS_UEFI_IMAGE");

fn main() {
    println!("QOS bootable UEFI image: {UEFI_IMAGE}");

    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let dist = Path::new(&manifest).join("../../dist");
        let _ = std::fs::create_dir_all(&dist);
        if std::fs::copy(UEFI_IMAGE, dist.join("qos-uefi.img")).is_ok() {
            println!("Copied to {}/qos-uefi.img", dist.display());
        }
    }
}

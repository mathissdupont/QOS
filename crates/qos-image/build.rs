use std::path::PathBuf;

fn main() {
    // Path to the compiled kernel ELF, provided by the artifact dependency.
    let kernel = std::env::var_os("CARGO_BIN_FILE_OS_os")
        .or_else(|| std::env::var_os("CARGO_BIN_FILE_OS"))
        .expect("missing CARGO_BIN_FILE_OS_os (is [unstable] bindeps enabled?)");
    let kernel = PathBuf::from(kernel);

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let uefi = out_dir.join("qos-uefi.img");

    bootloader::UefiBoot::new(&kernel)
        .create_disk_image(&uefi)
        .expect("failed to create UEFI disk image");

    // Copy the image to dist/ at build time so `cargo build -p qos-image` alone produces it
    // (CI uploads dist/qos-uefi.img; no separate run step needed).
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest.join("../../dist");
    let _ = std::fs::create_dir_all(&dist);
    std::fs::copy(&uefi, dist.join("qos-uefi.img")).expect("failed to copy UEFI image to dist/");

    // Expose the image path to the binary via env!().
    println!("cargo:rustc-env=QOS_UEFI_IMAGE={}", uefi.display());
    println!("cargo:rerun-if-changed=build.rs");
}

use std::path::PathBuf;

fn main() {
    // Path to the compiled kernel ELF, provided by the artifact dependency.
    let kernel = std::env::var_os("CARGO_BIN_FILE_OS_os")
        .or_else(|| std::env::var_os("CARGO_BIN_FILE_OS"))
        .expect("missing CARGO_BIN_FILE_OS_os (is [unstable] bindeps enabled?)");
    let kernel = PathBuf::from(kernel);

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let bios = out_dir.join("qos-bios.img");
    let uefi = out_dir.join("qos-uefi.img");

    bootloader::BiosBoot::new(&kernel)
        .create_disk_image(&bios)
        .expect("failed to create BIOS disk image");
    bootloader::UefiBoot::new(&kernel)
        .create_disk_image(&uefi)
        .expect("failed to create UEFI disk image");

    // Expose the image paths to the binary via env!().
    println!("cargo:rustc-env=QOS_BIOS_IMAGE={}", bios.display());
    println!("cargo:rustc-env=QOS_UEFI_IMAGE={}", uefi.display());
}

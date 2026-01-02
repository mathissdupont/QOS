fn main() {
    // Apply the kernel linker script only to this crate.
    // This avoids breaking the `bootloader` crate build (which is also built for
    // `x86_64-unknown-none` during `cargo bootimage`).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "none" {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let script = std::path::PathBuf::from(manifest_dir).join("linker.ld");

        // Use an absolute path so the linker can always find it.
        // Note: `-T<path>` must be a single argument (no space after -T).
        println!("cargo:rustc-link-arg=-T{}", script.display());

        // The kernel must not be PIE; a dynamic/PT_DYNAMIC kernel won't be loadable
        // in our bare-metal boot flow.
        println!("cargo:rustc-link-arg=-no-pie");

        println!("cargo:rerun-if-changed={}", script.display());
        println!("cargo:rerun-if-changed=src/asm_stubs.s");
    }
}

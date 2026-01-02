fn main() {
    // Link the user program at a fixed virtual address range that the kernel's user loader accepts.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let linker = std::path::Path::new(&manifest_dir).join("linker.ld");
    println!("cargo:rustc-link-arg=-T{}", linker.display());
}

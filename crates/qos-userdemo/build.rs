fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();

    // Sadece bare-metal hedefte linker script kullan.
    if target != "x86_64-unknown-none" {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let linker = std::path::Path::new(&manifest_dir).join("linker.ld");
    println!("cargo:rustc-link-arg=-T{}", linker.display());
}

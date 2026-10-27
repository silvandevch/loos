fn main() {
    let d = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg=-T{d}/linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
}

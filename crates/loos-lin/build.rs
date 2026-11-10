//! Builds the vDSO image (`vdso/vdso.c`) that the loader maps into every Linux process.
//! Without a C compiler the image is empty and programs fall back to real clock syscalls.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("vdso.so");
    println!("cargo:rerun-if-changed=vdso/vdso.c");
    println!("cargo:rerun-if-changed=vdso/vdso.lds");
    let cc = std::env::var("CC").unwrap_or_else(|_| "gcc".into());
    let ok = Command::new(&cc)
        .args([
            "-shared", "-fPIC", "-O2", "-nostdlib", "-fno-stack-protector", "-fno-asynchronous-unwind-tables",
            "-mno-sse", "-mno-mmx", "-mno-red-zone", "-Wl,--version-script=vdso/vdso.lds", "-Wl,--hash-style=both",
            "-Wl,-soname=linux-vdso.so.1", "-Wl,-z,max-page-size=4096", "-Wl,--build-id=none", "-o",
        ])
        .arg(&out)
        .arg("vdso/vdso.c")
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        println!("cargo:warning=no C compiler for the vDSO: Linux programs will use clock syscalls");
        std::fs::write(&out, []).unwrap();
    }
}

//! Builds Prism's in-process C runtime (`libc/*.c`, see docs/LANGUAGE.md §C.6) into a universal
//! (x86_64 + arm64) Mach-O dylib that `loader.rs` embeds. Needs clang and rustup's `ld64.lld`;
//! without them the dylib is empty and Prism falls back to its Rust-only libSystem.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let src = Path::new("libc");
    println!("cargo:rerun-if-changed=libc");
    let dylib = out.join("libprism.dylib");
    match build(src, &out) {
        Ok(bytes) => std::fs::write(&dylib, bytes).unwrap(),
        Err(e) => {
            println!("cargo:warning=Prism C runtime not built ({e}); macOS programs get the Rust-only libSystem");
            std::fs::write(&dylib, []).unwrap();
        }
    }
}

fn ld64() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("LD64") {
        return Ok(p.into());
    }
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let sysroot = Command::new(rustc).args(["--print", "sysroot"]).output().map_err(|e| e.to_string())?;
    let host = std::env::var("HOST").unwrap();
    let p = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim()).join(format!("lib/rustlib/{host}/bin/gcc-ld/ld64.lld"));
    if p.exists() { Ok(p) } else { Err(format!("{} missing", p.display())) }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let o = cmd.output().map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    if o.status.success() { Ok(()) } else { Err(format!("{:?} failed: {}", cmd.get_program(), String::from_utf8_lossy(&o.stderr))) }
}

fn build(src: &Path, out: &Path) -> Result<Vec<u8>, String> {
    let ld = ld64()?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(src).map_err(|e| e.to_string())?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "c")).collect();
    files.sort();
    let mut slices = Vec::new();
    for (arch, cpu, sub) in [("x86_64", 0x0100_0007u32, 3u32), ("arm64", 0x0100_000c, 0)] {
        let mut objs = Vec::new();
        for f in &files {
            let o = out.join(format!("{}_{arch}.o", f.file_stem().unwrap().to_string_lossy()));
            run(Command::new("clang").args(["-target", &format!("{arch}-apple-macos11"), "-O2", "-fno-vectorize", "-fno-slp-vectorize", "-ffreestanding", "-nostdlibinc", "-fno-stack-protector", "-fno-builtin", "-Wall", "-Wno-unused-function", "-c"]).arg(f).arg("-o").arg(&o))?;
            objs.push(o);
        }
        let d = out.join(format!("libprism_{arch}.dylib"));
        run(Command::new(&ld).args(["-arch", arch, "-platform_version", "macos", "11.0", "11.0", "-dylib", "-install_name", "/usr/lib/system/libprism.dylib", "-undefined", "dynamic_lookup", "-no_fixup_chains"]).args(&objs).arg("-o").arg(&d))?;
        slices.push((cpu, sub, std::fs::read(&d).map_err(|e| e.to_string())?));
    }
    // Universal (fat) wrapper, slices 16K-aligned.
    let mut fat = Vec::new();
    fat.extend(0xcafe_babeu32.to_be_bytes());
    fat.extend((slices.len() as u32).to_be_bytes());
    let mut off = 0x4000u32;
    let mut body = Vec::new();
    for (cpu, sub, d) in &slices {
        for v in [*cpu, *sub, off, d.len() as u32, 14] {
            fat.extend(v.to_be_bytes());
        }
        body.push((off, d));
        off = (off + d.len() as u32).div_ceil(0x4000) * 0x4000;
    }
    for (o, d) in body {
        fat.resize(o as usize, 0);
        fat.extend_from_slice(d);
    }
    Ok(fat)
}

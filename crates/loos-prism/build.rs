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
        // The tools are there but the code does not build: that is a bug, not a missing tool.
        Err(e) if e.contains(" failed: ") => panic!("Prism C runtime: {e}"),
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
    let mut files: Vec<PathBuf> = std::fs::read_dir(src).map_err(|e| e.to_string())?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "c" || x == "m")).collect();
    files.sort();
    let mut slices = Vec::new();
    for (arch, cpu, sub) in [("x86_64", 0x0100_0007u32, 3u32), ("arm64", 0x0100_000c, 0)] {
        let mut objs = Vec::new();
        for f in &files {
            let o = out.join(format!("{}_{arch}.o", f.file_stem().unwrap().to_string_lossy()));
            let mut cc = Command::new("clang");
            if f.extension().is_some_and(|x| x == "m") {
                cc.args(["-x", "objective-c", "-fno-objc-arc", "-fobjc-runtime=macosx-11.0"]);
            }
            run(cc.args(["-target", &format!("{arch}-apple-macos11"), "-O2", "-fno-vectorize", "-fno-slp-vectorize", "-ffreestanding", "-nostdlibinc", "-fno-stack-protector", "-fno-builtin", "-Wall", "-Wno-unused-function", "-c"]).arg(f).arg("-o").arg(&o))?;
            objs.push(o);
        }
        let d = out.join(format!("libprism_{arch}.dylib"));
        run(Command::new(&ld).args(["-arch", arch, "-platform_version", "macos", "11.0", "11.0", "-dylib", "-install_name", "/usr/lib/system/libprism.dylib", "-undefined", "dynamic_lookup", "-no_fixup_chains"]).args(&objs).arg("-o").arg(&d))?;
        let mut bytes = std::fs::read(&d).map_err(|e| e.to_string())?;
        if arch == "arm64" {
            rewrite_arm64_stubs(&mut bytes)?;
        }
        slices.push((cpu, sub, bytes));
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

/// Sections of a 64-bit Mach-O: (name, addr, size, file offset).
fn sections(d: &[u8]) -> Vec<(String, u64, u64, u32)> {
    let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(d[o..o + 8].try_into().unwrap());
    let mut out = Vec::new();
    let mut o = 32;
    for _ in 0..u32_at(16) {
        let (cmd, size) = (u32_at(o), u32_at(o + 4) as usize);
        if cmd == 0x19 {
            for j in 0..u32_at(o + 64) as usize {
                let so = o + 72 + j * 80;
                let name = String::from_utf8_lossy(&d[so..so + 16]).trim_end_matches('\0').to_string();
                out.push((name, u64_at(so + 32), u64_at(so + 40), u32_at(so + 48)));
            }
        }
        o += size;
    }
    out
}

/// Write every arm64 lazy stub (`adrp x16, ptr@PAGE; ldr x16, [x16, ptr@PAGEOFF]; br x16`) for its
/// `__la_symbol_ptr` slot. ld64.lld can leave part of `__stubs` zero-filled (seen when the
/// section starts mid-page); the stub contents are fully determined, so they are regenerated.
fn rewrite_arm64_stubs(d: &mut [u8]) -> Result<(), String> {
    let secs = sections(d);
    let find = |n: &str| secs.iter().find(|s| s.0 == n).cloned();
    let (Some(stubs), Some(ptrs)) = (find("__stubs"), find("__la_symbol_ptr")) else { return Ok(()) };
    if stubs.2 / 12 != ptrs.2 / 8 {
        return Err(format!("arm64 stubs ({}) and lazy pointers ({}) differ", stubs.2 / 12, ptrs.2 / 8));
    }
    for i in 0..stubs.2 / 12 {
        let pc = stubs.1 + i * 12;
        let target = ptrs.1 + i * 8;
        let pages = ((target >> 12) as i64 - (pc >> 12) as i64) as u32;
        let adrp = 0x9000_0010 | (pages & 3) << 29 | (pages >> 2 & 0x7_ffff) << 5;
        let ldr = 0xf940_0210 | (((target & 0xfff) / 8) as u32) << 10;
        let br = 0xd61f_0200u32;
        let o = stubs.3 as usize + i as usize * 12;
        for (k, w) in [adrp, ldr, br].iter().enumerate() {
            d[o + k * 4..o + k * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
    Ok(())
}

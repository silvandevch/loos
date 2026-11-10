//! L0: ELF loader. Maps PT_LOAD segments, loads PT_INTERP, builds the initial
//! stack (argv/envp/auxv) exactly as the Linux kernel does, and handles `#!`.

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::elf::*;
use crate::errno::*;
use crate::fs::{resolve, LinFs};
use crate::mem::*;

/// Where PIE main programs load.
pub const PIE_BASE: u64 = 0x5555_5555_4000;
/// Where the dynamic linker loads.
pub const INTERP_BASE: u64 = 0x7fff_f7f0_0000;

/// AT_* auxv keys.
#[allow(missing_docs)]
pub mod at {
    pub const NULL: u64 = 0;
    pub const PHDR: u64 = 3;
    pub const PHENT: u64 = 4;
    pub const PHNUM: u64 = 5;
    pub const PAGESZ: u64 = 6;
    pub const BASE: u64 = 7;
    pub const FLAGS: u64 = 8;
    pub const ENTRY: u64 = 9;
    pub const UID: u64 = 11;
    pub const EUID: u64 = 12;
    pub const GID: u64 = 13;
    pub const EGID: u64 = 14;
    pub const PLATFORM: u64 = 15;
    pub const HWCAP: u64 = 16;
    pub const CLKTCK: u64 = 17;
    pub const SECURE: u64 = 23;
    pub const RANDOM: u64 = 25;
    pub const HWCAP2: u64 = 26;
    pub const EXECFN: u64 = 31;
    pub const MINSIGSTKSZ: u64 = 51;
    pub const SYSINFO_EHDR: u64 = 33;
}

/// Result of loading a program.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// First instruction to run (interpreter entry if dynamic).
    pub entry: u64,
    /// Initial stack pointer (16-byte aligned).
    pub sp: u64,
    /// Main program entry (AT_ENTRY).
    pub prog_entry: u64,
    /// AT_PHDR value.
    pub phdr: u64,
    /// Main image load bias.
    pub bias: u64,
    /// Interpreter base (0 if static).
    pub interp_base: u64,
    /// Interpreter path.
    pub interp: Option<String>,
    /// Main image TLS template (vaddr, filesz, memsz, align) if PT_TLS exists.
    pub tls: Option<(u64, u64, u64, u64)>,
    /// Final resolved path of the executable.
    pub exe: String,
    /// Effective argv (differs from the input after `#!` expansion).
    pub argv: Vec<String>,
}

fn prot_of(flags: u32) -> u32 {
    (if flags & 4 != 0 { PROT_READ } else { 0 }) | (if flags & 2 != 0 { PROT_WRITE } else { 0 }) | (if flags & 1 != 0 { PROT_EXEC } else { 0 })
}

fn read_file(fs: &mut dyn LinFs, path: &str) -> Result<Vec<u8>, i64> {
    let st = fs.lstat(path)?;
    if st.kind != crate::fs::Kind::File {
        return Err(EACCES);
    }
    if st.size > 1 << 30 {
        return Err(ENOMEM);
    }
    let mut buf = vec![0u8; st.size as usize];
    let mut off = 0;
    while off < buf.len() {
        let n = fs.read(path, off as u64, &mut buf[off..])?;
        if n == 0 {
            break;
        }
        off += n;
    }
    buf.truncate(off);
    Ok(buf)
}

fn map_image(mem: &mut UserMem, elf: &Elf, data: &[u8], bias: u64, name: &str) -> Result<(), i64> {
    for p in elf.phdrs.iter().filter(|p| p.kind == PT_LOAD) {
        let start = floor(p.vaddr).checked_add(bias).ok_or(ENOMEM)?;
        let lead = p.vaddr - floor(p.vaddr);
        let len = ceil(p.vaddr + p.memsz) - floor(p.vaddr);
        let mut init = vec![0u8; (lead + p.filesz) as usize];
        init[lead as usize..].copy_from_slice(&data[p.offset as usize..(p.offset + p.filesz) as usize]);
        mem.map_fixed(start, len, prot_of(p.flags), &init, name)?;
    }
    Ok(())
}

/// Load `path` into a fresh address space and build the entry state.
pub fn load(fs: &mut dyn LinFs, mem: &mut UserMem, path: &str, argv: &[String], envp: &[String], seed: u64) -> Result<Loaded, i64> {
    load_depth(fs, mem, path, argv, envp, seed, 0)
}

fn load_depth(fs: &mut dyn LinFs, mem: &mut UserMem, path: &str, argv: &[String], envp: &[String], seed: u64, depth: u32) -> Result<Loaded, i64> {
    let exe = resolve(fs, "/", path, true)?;
    let data = read_file(fs, &exe)?;
    if data.starts_with(b"#!") {
        if depth >= 4 {
            return Err(ELOOP);
        }
        let line_end = data.iter().position(|&b| b == b'\n').unwrap_or(data.len()).min(255);
        let line = String::from_utf8_lossy(&data[2..line_end]).trim().to_string();
        let (interp, arg) = match line.split_once([' ', '\t']) {
            Some((i, a)) => (i.to_string(), Some(a.trim().to_string()).filter(|a| !a.is_empty())),
            None => (line, None),
        };
        if interp.is_empty() {
            return Err(ENOEXEC);
        }
        let mut nargv = vec![interp.clone()];
        nargv.extend(arg);
        nargv.push(exe.clone());
        nargv.extend(argv.iter().skip(1).cloned());
        return load_depth(fs, mem, &interp, &nargv, envp, seed, depth + 1);
    }
    let elf = Elf::parse(&data)?;
    let bias = if elf.etype == ET_DYN { PIE_BASE.wrapping_sub(elf.span().0) } else { 0 };
    let (_, hi) = elf.span();
    map_image(mem, &elf, &data, bias, &exe)?;
    let mut interp_base = 0;
    let mut entry = elf.entry.checked_add(bias).ok_or(EINVAL)?;
    if let Some(ip) = &elf.interp {
        let ipath = resolve(fs, "/", ip, true)?;
        let idata = read_file(fs, &ipath)?;
        let ielf = Elf::parse(&idata)?;
        if ielf.interp.is_some() || ielf.etype != ET_DYN {
            return Err(ELOOP);
        }
        interp_base = INTERP_BASE - ielf.span().0;
        map_image(mem, &ielf, &idata, interp_base, &ipath)?;
        entry = ielf.entry.checked_add(interp_base).ok_or(EINVAL)?;
    }
    let prog_entry = elf.entry.checked_add(bias).ok_or(EINVAL)?;
    // AT_PHDR
    let phdr = match elf.phdrs.iter().find(|p| p.kind == PT_PHDR) {
        Some(p) => p.vaddr + bias,
        None => {
            let seg = elf.phdrs.iter().find(|p| p.kind == PT_LOAD && p.offset <= elf.phoff && elf.phoff < p.offset + p.filesz).ok_or(EINVAL)?;
            seg.vaddr + bias + (elf.phoff - seg.offset)
        }
    };
    mem.brk_start = ceil(hi + bias);
    mem.brk = mem.brk_start;
    let tls = elf.tls().map(|t| (t.vaddr + bias, t.filesz, t.memsz, t.align));

    // Stack.
    let init_stack = 256 * 1024u64;
    let low = STACK_TOP - init_stack;
    mem.map_fixed(low, init_stack, PROT_READ | PROT_WRITE | if elf.exec_stack { PROT_EXEC } else { 0 }, &[], "[stack]")?;
    mem.set_stack(low);
    let mut sp = STACK_TOP;
    let push_bytes = |mem: &mut UserMem, sp: &mut u64, b: &[u8]| -> Result<u64, i64> {
        *sp -= b.len() as u64;
        mem.write(*sp, b)?;
        Ok(*sp)
    };
    let cstr = |mem: &mut UserMem, sp: &mut u64, s: &str| -> Result<u64, i64> {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        push_bytes(mem, sp, &v)
    };
    let total: usize = argv.iter().chain(envp.iter()).map(|s| s.len() + 1).sum::<usize>() + exe.len() + 64;
    if total > 128 * 1024 || argv.len() + envp.len() > 8192 {
        return Err(7); // E2BIG
    }
    let execfn = cstr(mem, &mut sp, &exe)?;
    let mut envp_ptrs = Vec::new();
    for e in envp.iter().rev() {
        envp_ptrs.push(cstr(mem, &mut sp, e)?);
    }
    envp_ptrs.reverse();
    let mut argv_ptrs = Vec::new();
    for a in argv.iter().rev() {
        argv_ptrs.push(cstr(mem, &mut sp, a)?);
    }
    argv_ptrs.reverse();
    let platform = cstr(mem, &mut sp, "x86_64")?;
    let mut rnd = [0u8; 16];
    let mut x = seed | 1;
    for b in rnd.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = (x >> 24) as u8;
    }
    let random = push_bytes(mem, &mut sp, &rnd)?;
    let auxv: Vec<(u64, u64)> = vec![
        (at::PHDR, phdr),
        (at::PHENT, 56),
        (at::PHNUM, elf.phdrs.len() as u64),
        (at::PAGESZ, PAGE),
        (at::BASE, interp_base),
        (at::FLAGS, 0),
        (at::ENTRY, prog_entry),
        (at::UID, 0),
        (at::EUID, 0),
        (at::GID, 0),
        (at::EGID, 0),
        (at::PLATFORM, platform),
        (at::HWCAP, 0x178b_fbff),
        (at::CLKTCK, 100),
        (at::SECURE, 0),
        (at::RANDOM, random),
        (at::HWCAP2, 0),
        (at::EXECFN, execfn),
        (at::MINSIGSTKSZ, 2048),
    ];
    let mut auxv = auxv;
    if let Some(img) = crate::vdso::memory_image() {
        // The vDSO: clock_gettime & co. without system calls (the time page is mapped by the caller).
        mem.map_fixed(crate::vdso::VDSO_ADDR, img.len() as u64, PROT_READ | PROT_EXEC, &img, "[vdso]")?;
        auxv.push((at::SYSINFO_EHDR, crate::vdso::VDSO_ADDR));
    }
    auxv.push((at::NULL, 0));
    let words = 1 + argv_ptrs.len() + 1 + envp_ptrs.len() + 1 + auxv.len() * 2;
    sp &= !15;
    sp -= (words as u64) * 8;
    sp &= !15; // entry rsp is 16-byte aligned; argc sits at [rsp]
    let mut w = Vec::with_capacity(words);
    w.push(argv.len() as u64);
    w.extend(&argv_ptrs);
    w.push(0);
    w.extend(&envp_ptrs);
    w.push(0);
    for (k, v) in &auxv {
        w.push(*k);
        w.push(*v);
    }
    let bytes: Vec<u8> = w.iter().flat_map(|v| v.to_le_bytes()).collect();
    mem.write(sp, &bytes)?;
    Ok(Loaded { entry, sp, prog_entry, phdr, bias, interp_base, interp: elf.interp.clone(), tls, exe, argv: argv.to_vec() })
}

//! Mach-O image loader: maps the executable and the dylibs it ships with (resolved through
//! `@executable_path`, `@loader_path` and `@rpath`), applies rebases and binds, and routes every
//! import from a macOS system library to Prism's own libSystem ([`crate::libsys`]).

#[allow(unused_imports)]
use alloc::{collections::BTreeMap, format, string::{String, ToString}, vec, vec::Vec};
use crate::macho::*;
use loos_lin::errno::*;
use loos_lin::fs::{resolve, Kind, LinFs};
use loos_lin::mem::*;

/// Prism's libSystem stubs: 16 bytes each (`mov r10, rcx; mov eax, PRISM_NR + i; syscall; ret`).
/// The ARM64 interpreter recognises a branch into this range as a call into the same functions.
pub const SHIM_BASE: u64 = 0x7000_0000_0000;
/// Bytes per stub.
pub const STUB_SIZE: u64 = 16;
/// Data symbols libSystem exports (`___stdoutp`, `_environ`, `errno`, ...).
pub const SHIM_DATA: u64 = SHIM_BASE + 0x10_0000;
/// Per-process scratch buffer libSystem functions stage syscall arguments in.
pub const SCRATCH: u64 = SHIM_BASE + 0x20_0000;
/// Size of [`SCRATCH`].
pub const SCRATCH_LEN: u64 = 64 * 1024;
/// Syscall numbers `PRISM_NR..PRISM_NR + 0x1_0000` are Prism libSystem calls.
pub const PRISM_NR: u64 = 0x4C50_0000;
/// Where dylibs of an ARM64/x86_64 process are placed below (the mmap area grows down from there).
const MAX_IMAGES: usize = 256;

/// One mapped image.
#[derive(Clone, Debug)]
pub struct Image {
    /// Path in the LoOS filesystem.
    pub path: String,
    /// Parsed headers.
    pub macho: MachO,
    /// Load slide.
    pub slide: u64,
    /// For each dylib ordinal (1-based → index+1): the loaded image, or `None` for a Prism-provided library.
    pub deps: Vec<Option<usize>>,
}

/// A loaded Mach-O program.
#[derive(Clone, Debug)]
pub struct Program {
    /// CPU type (x86_64 or arm64).
    pub cpu: u32,
    /// Images; `images[0]` is the executable.
    pub images: Vec<Image>,
    /// Slid entry point.
    pub entry: Entry,
    /// Initializers to run before the entry point, dependencies first.
    pub inits: Vec<u64>,
    /// libSystem stub index → symbol name.
    pub stubs: Vec<String>,
    /// Initial stack pointer: `[argc][argv..][0][envp..][0][apple..][0]`.
    pub sp: u64,
    /// argc / argv / envp / apple pointers on the stack.
    pub argc: u64,
    /// `char **argv`.
    pub argv: u64,
    /// `char **envp`.
    pub envp: u64,
    /// `char **apple`.
    pub apple: u64,
    /// Thread-local-variable templates: (slid start, length of initialised data, total length).
    pub tlv: Vec<(u64, u64, u64)>,
    /// Diagnostics (missing dylibs, unresolved weak symbols).
    pub notes: String,
    /// Index of Prism's C runtime in `images`.
    pub runtime: Option<usize>,
}

/// Stub names every process gets first, in this order.
pub const FIXED_STUBS: [&str; 4] = ["__prism_start", "__prism_main_ret", "__prism_sigtramp", "__prism_sigreturn"];

pub(crate) fn read_file(fs: &mut dyn LinFs, path: &str) -> Result<Vec<u8>, i64> {
    let st = fs.lstat(path)?;
    if st.kind != Kind::File {
        return Err(EACCES);
    }
    if st.size > 1 << 31 {
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

fn dir_of(p: &str) -> &str {
    match p.rfind('/') {
        Some(0) => "/",
        Some(i) => &p[..i],
        None => ".",
    }
}

/// Prism's C runtime: a universal dylib built by `build.rs` from `libc/` (empty without clang).
static RUNTIME: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libprism.dylib"));
/// LLVM's libc++, embedded by `build.rs` when it was built (empty otherwise).
static LIBCXX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libc++.1.dylib"));

/// A system library Prism carries as a real Mach-O image (the rest are its libSystem stubs).
fn embedded_library(path: &str) -> Option<&'static [u8]> {
    match path {
        "/usr/lib/libc++.1.dylib" if !LIBCXX.is_empty() => Some(LIBCXX),
        _ => None,
    }
}

/// Libraries Prism implements itself instead of loading from disk.
pub fn is_system_library(path: &str) -> bool {
    path.starts_with("/usr/lib/") || path.starts_with("/System/Library/")
}

struct Ctx<'a> {
    fs: &'a mut dyn LinFs,
    mem: &'a mut UserMem,
    cpu: u32,
    images: Vec<Image>,
    by_path: BTreeMap<String, usize>,
    order: Vec<usize>,
    notes: &'a mut String,
}

impl Ctx<'_> {
    fn expand(&mut self, name: &str, loader: usize) -> Vec<String> {
        let exe_dir = dir_of(&self.images[0].path).to_string();
        let loader_dir = dir_of(&self.images[loader].path).to_string();
        let subst = |p: &str| p.replace("@executable_path", &exe_dir).replace("@loader_path", &loader_dir);
        if let Some(rest) = name.strip_prefix("@rpath/") {
            let mut rp: Vec<String> = self.images[loader].macho.rpaths.clone();
            rp.extend(self.images[0].macho.rpaths.iter().cloned());
            rp.iter().map(|r| format!("{}/{}", subst(r).trim_end_matches('/'), rest)).collect()
        } else {
            vec![subst(name)]
        }
    }

    fn load(&mut self, path: &str, main: bool) -> Result<usize, i64> {
        if let Some(&i) = self.by_path.get(path) {
            return Ok(i);
        }
        let data = read_file(self.fs, path)?;
        self.load_data(path, &data, main)
    }

    fn load_data(&mut self, path: &str, data: &[u8], main: bool) -> Result<usize, i64> {
        if self.images.len() >= MAX_IMAGES {
            return Err(ENOMEM);
        }
        let parsed = select_arch(data, &[self.cpu]).and_then(|(_, range)| Ok((MachO::parse(&data[range.clone()])?, range)));
        let (m, range) = match parsed {
            Ok(v) => v,
            Err(e) => {
                *self.notes += &format!("prism: {path}: not a Mach-O image for this processor (errno {e})\n");
                return Err(e);
            }
        };
        let d = &data[range];
        if main && m.filetype != MH_EXECUTE || !main && m.filetype != MH_DYLIB && m.filetype != MH_BUNDLE {
            *self.notes += &format!("prism: {path}: Mach-O file type {} cannot be loaded here\n", m.filetype);
            return Err(ENOEXEC);
        }
        let (lo, hi) = m.span();
        if hi <= lo {
            return Err(ENOEXEC);
        }
        // Executables keep their link address (PIE or not); dylibs go wherever there is room.
        let slide = if main && lo >= 0x1000 { 0 } else { self.mem.gap(hi - lo + 0x10_0000)?.wrapping_sub(lo) };
        for s in m.segments.iter().filter(|s| s.vmsize > 0 && s.initprot != 0) {
            let start = floor(s.vmaddr) + slide;
            let lead = (s.vmaddr - floor(s.vmaddr)) as usize;
            let mut init = vec![0u8; lead + s.filesize as usize];
            init[lead..].copy_from_slice(&d[s.fileoff as usize..(s.fileoff + s.filesize) as usize]);
            let name = format!("{path} {}", s.name);
            self.mem.map_fixed(start, ceil(s.vmaddr + s.vmsize) - floor(s.vmaddr), s.initprot & 7, &init, &name)?;
        }
        let idx = self.images.len();
        self.by_path.insert(path.to_string(), idx);
        let dylibs = m.dylibs.clone();
        self.images.push(Image { path: path.to_string(), macho: m, slide, deps: Vec::new() });
        for (name, weak) in dylibs {
            let dep = if let Some(data) = embedded_library(&name) {
                Some(match self.by_path.get(&name) {
                    Some(&i) => i,
                    None => self.load_data(&name, data, false)?,
                })
            } else if is_system_library(&name) {
                None
            } else {
                let mut found = None;
                for cand in self.expand(&name, idx) {
                    if let Ok(real) = resolve(self.fs, "/", &cand, true) {
                        found = Some(self.load(&real, false)?);
                        break;
                    }
                }
                if found.is_none() {
                    *self.notes += &format!("prism: {}dylib {name} not found; its symbols go to Prism's libSystem\n", if weak { "weak " } else { "" });
                }
                found
            };
            self.images[idx].deps.push(dep);
        }
        self.order.push(idx);
        Ok(idx)
    }
}

/// Symbol lookup across images, falling back to Prism's libSystem.
pub(crate) struct Resolver<'a> {
    pub images: &'a [Image],
    /// Prism's C runtime image: system-library imports it exports bind there.
    pub runtime: Option<usize>,
    pub stubs: Vec<String>,
    pub index: BTreeMap<String, usize>,
}

impl Resolver<'_> {
    fn stub(&mut self, name: &str) -> u64 {
        if let Some(a) = self.runtime.and_then(|r| self.in_image(r, name, 0)) {
            return a;
        }
        if let Some(off) = crate::libsys::data_symbol(name) {
            return SHIM_DATA + off;
        }
        let i = *self.index.entry(name.to_string()).or_insert_with(|| {
            self.stubs.push(name.to_string());
            self.stubs.len() - 1
        });
        SHIM_BASE + i as u64 * STUB_SIZE
    }

    fn in_image(&self, img: usize, name: &str, depth: u32) -> Option<u64> {
        let im = &self.images[img];
        match im.macho.export(name)? {
            Export::Addr(a) => Some(a.wrapping_add(im.slide)),
            Export::Reexport(ord, n) if depth < 8 => match im.deps.get((ord as usize).wrapping_sub(1)).copied().flatten() {
                Some(j) => self.in_image(j, &n, depth + 1),
                None => None,
            },
            _ => None,
        }
    }

    /// Address for `name` imported by image `img` through `ordinal`.
    pub fn lookup(&mut self, img: usize, ordinal: i64, name: &str) -> Option<u64> {
        let target = match ordinal {
            ORD_MAIN => Some(0),
            // A self bind of a symbol the image does not export is a coalesced weak definition
            // (C++ vtables, inline template code): dyld finds it in whichever image has it.
            ORD_SELF if self.in_image(img, name, 0).is_some() => Some(img),
            ORD_SELF | ORD_FLAT | ORD_WEAK => {
                return (0..self.images.len()).find_map(|i| self.in_image(i, name, 0)).or_else(|| Some(self.stub(name)));
            }
            o if o > 0 => match self.images[img].deps.get(o as usize - 1) {
                Some(Some(j)) => Some(*j),
                Some(None) => None,
                None => return None,
            },
            _ => return None,
        };
        match target {
            // An embedded system library imports the rest of libSystem itself, and programs bind
            // symbols to it that macOS's copy re-exports (libc++abi): those go to libSystem too.
            Some(j) if is_system_library(&self.images[j].path) => self.in_image(j, name, 0).or_else(|| Some(self.stub(name))),
            Some(j) => self.in_image(j, name, 0),
            None => Some(self.stub(name)),
        }
    }
}

/// Encode stub `i` (x86-64 machine code).
pub fn stub_code(i: usize) -> [u8; 16] {
    let nr = (PRISM_NR + i as u64) as u32;
    let mut c = [0xccu8; 16];
    if FIXED_STUBS[1] == FIXED_STUBS.get(i).copied().unwrap_or("") {
        c[..3].copy_from_slice(&[0x48, 0x89, 0xc7]); // mov rdi, rax: main's return value is exit's argument
    } else {
        c[..3].copy_from_slice(&[0x49, 0x89, 0xca]); // mov r10, rcx: `syscall` overwrites rcx (4th argument)
    }
    c[3] = 0xb8;
    c[4..8].copy_from_slice(&nr.to_le_bytes());
    c[8..10].copy_from_slice(&[0x0f, 0x05]);
    c[10] = 0xc3;
    c
}

/// Load `path` (a Mach-O executable, thin or universal) into `mem`.
/// `prefer` lists acceptable CPU types in order of preference.
pub fn load(fs: &mut dyn LinFs, mem: &mut UserMem, path: &str, argv: &[String], envp: &[String], prefer: &[u32]) -> Result<Program, i64> {
    let mut diag = String::new();
    load_noting(fs, mem, path, argv, envp, prefer, &mut diag)
}

/// [`load`], with the diagnostics of a failed load (the missing symbol, ...) appended to `diag`.
pub fn load_noting(fs: &mut dyn LinFs, mem: &mut UserMem, path: &str, argv: &[String], envp: &[String], prefer: &[u32], diag: &mut String) -> Result<Program, i64> {
    let exe = resolve(fs, "/", path, true)?;
    let cpu = {
        let data = read_file(fs, &exe)?;
        select_arch(&data, prefer)?.0
    };
    let mut cx = Ctx { fs, mem, cpu, images: Vec::new(), by_path: BTreeMap::new(), order: Vec::new(), notes: diag };
    cx.load(&exe, true)?;
    // Prism's C runtime (stdio, err/warn, ...) is part of every process; its initializers run first.
    let runtime = if RUNTIME.is_empty() {
        None
    } else {
        let r = cx.load_data("/usr/lib/system/libprism.dylib", RUNTIME, false)?;
        cx.order.retain(|&i| i != r);
        cx.order.insert(0, r);
        Some(r)
    };
    let Ctx { mem, images, order, notes, .. } = cx;
    let entry = match images[0].macho.entry.ok_or(ENOEXEC)? {
        Entry::Main(off) => Entry::Main(images[0].macho.base() + off),
        Entry::Thread(pc) => Entry::Thread(pc),
    };
    let entry = match entry {
        Entry::Main(a) => Entry::Main(a.wrapping_add(images[0].slide)),
        Entry::Thread(a) => Entry::Thread(a.wrapping_add(images[0].slide)),
    };
    let stubs = fixup(mem, &images, 0, runtime, FIXED_STUBS.iter().map(|s| s.to_string()).collect(), notes)?;
    // libSystem: stubs, data cells, scratch.
    map_stubs(mem, 0, stubs.len())?;
    mem.map_fixed(SHIM_DATA, crate::libsys::DATA_LEN, PROT_READ | PROT_WRITE, &crate::libsys::data_init(), "libSystem data (Prism)")?;
    mem.map_fixed(SCRATCH, SCRATCH_LEN, PROT_READ | PROT_WRITE, &[], "libSystem scratch (Prism)")?;
    let (inits, tlv) = initializers(mem, &images, &order)?;
    // Stack: strings, then the classic start layout.
    let size = 1u64 << 20;
    mem.map_fixed(STACK_TOP - size, size, PROT_READ | PROT_WRITE, &[], "[stack]")?;
    mem.set_stack(STACK_TOP - size);
    let mut sp = STACK_TOP;
    let mut push_str = |mem: &mut UserMem, s: &str| -> Result<u64, i64> {
        sp -= s.len() as u64 + 1;
        mem.write(sp, s.as_bytes())?;
        mem.write(sp + s.len() as u64, &[0])?;
        Ok(sp)
    };
    let apple_s = [format!("executable_path={exe}"), "ptr_munge=".to_string(), "main_stack=".to_string()];
    let mut ptrs = |mem: &mut UserMem, list: &[String]| -> Result<Vec<u64>, i64> { list.iter().map(|s| push_str(mem, s)).collect() };
    let a = ptrs(mem, argv)?;
    let e = ptrs(mem, envp)?;
    let ap = ptrs(mem, &apple_s)?;
    let total: usize = argv.iter().chain(envp).map(|s| s.len() + 1).sum();
    if total > 256 * 1024 {
        return Err(7); // E2BIG
    }
    let mut words = vec![argv.len() as u64];
    words.extend(&a);
    words.push(0);
    words.extend(&e);
    words.push(0);
    words.extend(&ap);
    words.push(0);
    let mut sp = (STACK_TOP - total as u64 - 1024 - words.len() as u64 * 8) & !15;
    sp -= 4096; // headroom below the strings
    sp &= !15;
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    mem.write(sp, &bytes)?;
    let argvp = sp + 8;
    let envpp = argvp + (a.len() as u64 + 1) * 8;
    let applep = envpp + (e.len() as u64 + 1) * 8;
    Ok(Program { cpu, images, entry, inits, stubs, sp, argc: argv.len() as u64, argv: argvp, envp: envpp, apple: applep, tlv, notes: String::new(), runtime })
}

/// Apply the rebases and binds of `images[first..]`. `stubs` are the libSystem stubs bound so far;
/// the list comes back with the ones these images added.
fn fixup(mem: &mut UserMem, images: &[Image], first: usize, runtime: Option<usize>, stubs: Vec<String>, notes: &mut String) -> Result<Vec<String>, i64> {
    let index = stubs.iter().enumerate().map(|(i, s)| (s.clone(), i)).collect();
    let mut r = Resolver { images, runtime, stubs, index };
    for (ii, im) in images.iter().enumerate().skip(first) {
        for f in &im.macho.fixups {
            let (addr, val) = match f {
                Fixup::Rebase { addr, target } => (*addr, target.wrapping_add(im.slide)),
                Fixup::Bind { addr, ordinal, name, addend, weak } => match r.lookup(ii, *ordinal, name) {
                    Some(a) => (*addr, a.wrapping_add(*addend as u64)),
                    None if *weak => (*addr, 0),
                    None => {
                        *notes += &format!("prism: {}: symbol {name} not found\n", im.path);
                        return Err(ENOEXEC);
                    }
                },
            };
            mem.write_u64(addr.wrapping_add(im.slide), val)?;
        }
    }
    Ok(r.stubs)
}

/// Make the code of stubs `from..to` executable at [`SHIM_BASE`] (`from` stubs are already there).
pub fn map_stubs(mem: &mut UserMem, from: usize, to: usize) -> Result<(), i64> {
    let code = |a: usize, b: usize| (a..b).flat_map(stub_code).collect::<Vec<u8>>();
    if from == 0 {
        return mem.map_fixed(SHIM_BASE, ceil(to as u64 * STUB_SIZE).max(PAGE), PROT_READ | PROT_EXEC, &code(0, to), "libSystem (Prism)");
    }
    // Stubs that fit in the pages already mapped, then new pages.
    let have = ceil(from as u64 * STUB_SIZE).max(PAGE);
    let fit = ((have / STUB_SIZE) as usize).min(to);
    if fit > from {
        let page = floor(SHIM_BASE + from as u64 * STUB_SIZE);
        let len = ceil(SHIM_BASE + fit as u64 * STUB_SIZE) - page;
        mem.protect(page, len, PROT_READ | PROT_WRITE)?;
        mem.write(SHIM_BASE + from as u64 * STUB_SIZE, &code(from, fit))?;
        mem.protect(page, len, PROT_READ | PROT_EXEC)?;
    }
    if to > fit {
        mem.map_fixed(SHIM_BASE + have, ceil((to - fit) as u64 * STUB_SIZE), PROT_READ | PROT_EXEC, &code(fit, to), "libSystem (Prism)")?;
    }
    Ok(())
}

/// Initializers (in `order`: dependencies first) and thread-local templates of images.
fn initializers(mem: &mut UserMem, images: &[Image], order: &[usize]) -> Result<(Vec<u64>, Vec<(u64, u64, u64)>), i64> {
    let mut inits = Vec::new();
    let mut tlv = Vec::new();
    for &ii in order {
        let im = &images[ii];
        for s in &im.macho.sections {
            let a = s.addr.wrapping_add(im.slide);
            match s.flags & 0xff {
                0x09 => {
                    for k in 0..(s.size / 8).min(4096) {
                        let f = mem.read_u64(a + k * 8)?;
                        if f != 0 {
                            inits.push(f);
                        }
                    }
                }
                0x16 => {
                    for k in 0..(s.size / 4).min(4096) {
                        inits.push((im.macho.base() + mem.read_u32(a + k * 4)? as u64).wrapping_add(im.slide));
                    }
                }
                0x11 | 0x12 => tlv.push((a, if s.flags & 0xff == 0x11 { s.size } else { 0 }, s.size)),
                _ => {}
            }
        }
    }
    Ok((inits, tlv))
}

/// `dlopen` handles: [`HANDLE_BASE`] + image index, or [`HANDLE_SYSTEM`] for a library Prism
/// implements itself.
pub const HANDLE_BASE: u64 = 0x10000;
/// Handle of every macOS system library.
pub const HANDLE_SYSTEM: u64 = HANDLE_BASE - 1;

/// What [`dlopen`] loaded.
pub struct Opened {
    /// The handle for `dlsym`.
    pub handle: u64,
    /// Initializers of the newly mapped images, dependencies first.
    pub inits: Vec<u64>,
    /// Thread-local templates of the new images.
    pub tlv: Vec<(u64, u64, u64)>,
    /// libSystem stubs, including those the new images added.
    pub stubs: Vec<String>,
}

/// Load `name` (and the dylibs it needs) into a running process whose images are `images`.
/// An image that is already loaded is only looked up. On failure `images` is unchanged.
pub fn dlopen(fs: &mut dyn LinFs, mem: &mut UserMem, cwd: &str, cpu: u32, images: &mut Vec<Image>, runtime: Option<usize>, stubs: &[String], name: &str, notes: &mut String) -> Result<Opened, i64> {
    let none = |handle| Opened { handle, inits: Vec::new(), tlv: Vec::new(), stubs: stubs.to_vec() };
    if is_system_library(name) && embedded_library(name).is_none() {
        return Ok(none(HANDLE_SYSTEM));
    }
    let first = images.len();
    let by_path = images.iter().enumerate().map(|(i, im)| (im.path.clone(), i)).collect();
    let mut cx = Ctx { fs, mem, cpu, images: core::mem::take(images), by_path, order: Vec::new(), notes };
    let mut loaded = Err(ENOENT);
    if let Some(data) = embedded_library(name) {
        loaded = match cx.by_path.get(name) {
            Some(&i) => Ok(i),
            None => cx.load_data(name, data, false),
        };
    }
    for cand in if loaded.is_ok() { Vec::new() } else { cx.expand(name, 0) } {
        if let Ok(real) = resolve(cx.fs, cwd, &cand, true) {
            loaded = cx.load(&real, false);
            break;
        }
    }
    let Ctx { mem, images: all, order, notes, .. } = cx;
    *images = all;
    let r = loaded.and_then(|idx| {
        let stubs = fixup(mem, images, first, runtime, stubs.to_vec(), notes)?;
        let (inits, tlv) = initializers(mem, images, &order)?;
        Ok(Opened { handle: HANDLE_BASE + idx as u64, inits, tlv, stubs })
    });
    match r {
        Ok(o) => {
            for im in &mut images[first..] {
                im.macho.fixups = Vec::new();
            }
            Ok(o)
        }
        Err(e) => {
            for im in images.drain(first..) {
                let (lo, hi) = im.macho.span();
                let _ = mem.unmap(floor(lo.wrapping_add(im.slide)), ceil(hi - lo));
            }
            Err(e)
        }
    }
}

/// Address of exported symbol `name` (with its leading underscore) in image `img` or a library it
/// re-exports or depends on.
pub fn symbol_in(images: &[Image], img: usize, name: &str) -> Option<u64> {
    let r = Resolver { images, runtime: None, stubs: Vec::new(), index: BTreeMap::new() };
    let mut seen = vec![false; images.len()];
    let mut todo = vec![img];
    while let Some(i) = todo.pop() {
        if i >= images.len() || core::mem::replace(&mut seen[i], true) {
            continue;
        }
        if let Some(a) = r.in_image(i, name, 0) {
            return Some(a);
        }
        todo.extend(images[i].deps.iter().rev().flatten());
    }
    None
}

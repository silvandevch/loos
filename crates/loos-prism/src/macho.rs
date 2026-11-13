//! Mach-O parsing (input is untrusted: every read is bounds-checked).
//!
//! Thin 64-bit images (x86_64, arm64), universal (fat/fat64) wrappers, segments and sections,
//! entry points (`LC_MAIN`, `LC_UNIXTHREAD`), dylib dependencies and rpaths, and all pointer
//! fixups — classic `LC_DYLD_INFO` rebase/bind opcode streams and `LC_DYLD_CHAINED_FIXUPS` —
//! decoded into one flat [`Fixup`] list. Exports are looked up in the export trie.

#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec, vec::Vec};
use loos_lin::errno::{EINVAL, ENOEXEC};

/// CPU_TYPE_X86_64.
pub const CPU_X86_64: u32 = 0x0100_0007;
/// CPU_TYPE_ARM64.
pub const CPU_ARM64: u32 = 0x0100_000c;
/// MH_EXECUTE.
pub const MH_EXECUTE: u32 = 2;
/// MH_DYLIB.
pub const MH_DYLIB: u32 = 6;
/// MH_BUNDLE.
pub const MH_BUNDLE: u32 = 8;

const LC_SEGMENT_64: u32 = 0x19;
const LC_UNIXTHREAD: u32 = 0x5;
const LC_LOAD_DYLIB: u32 = 0xc;
const LC_ID_DYLIB: u32 = 0xd;
const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
const LC_REEXPORT_DYLIB: u32 = 0x8000_001f;
const LC_LAZY_LOAD_DYLIB: u32 = 0x20;
const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
const LC_RPATH: u32 = 0x8000_001c;
const LC_DYLD_INFO: u32 = 0x22;
const LC_DYLD_INFO_ONLY: u32 = 0x8000_0022;
const LC_MAIN: u32 = 0x8000_0028;
const LC_DYLD_EXPORTS_TRIE: u32 = 0x8000_0033;
const LC_DYLD_CHAINED_FIXUPS: u32 = 0x8000_0034;

/// Dylib ordinal meaning "this image".
pub const ORD_SELF: i64 = 0;
/// Dylib ordinal meaning "the main executable".
pub const ORD_MAIN: i64 = -1;
/// Dylib ordinal meaning "search every image" (flat namespace).
pub const ORD_FLAT: i64 = -2;
/// Dylib ordinal meaning "weak definition lookup".
pub const ORD_WEAK: i64 = -3;

pub(crate) fn rd<const N: usize>(d: &[u8], o: usize) -> Option<[u8; N]> {
    d.get(o..o.checked_add(N)?)?.try_into().ok()
}
pub(crate) fn u16le(d: &[u8], o: usize) -> Option<u16> {
    rd(d, o).map(u16::from_le_bytes)
}
pub(crate) fn u32le(d: &[u8], o: usize) -> Option<u32> {
    rd(d, o).map(u32::from_le_bytes)
}
pub(crate) fn u64le(d: &[u8], o: usize) -> Option<u64> {
    rd(d, o).map(u64::from_le_bytes)
}
fn u32be(d: &[u8], o: usize) -> Option<u32> {
    rd(d, o).map(u32::from_be_bytes)
}
fn u64be(d: &[u8], o: usize) -> Option<u64> {
    rd(d, o).map(u64::from_be_bytes)
}
fn cstr(d: &[u8], o: usize) -> Option<String> {
    let s = d.get(o..)?;
    let n = s.iter().position(|&b| b == 0)?;
    String::from_utf8(s[..n].to_vec()).ok()
}
fn fixed_name(d: &[u8], o: usize) -> String {
    let s = d.get(o..o + 16).unwrap_or(&[]);
    let n = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..n]).into_owned()
}

/// Pick the slice of a (possibly universal) file for one of `prefer` CPU types, in order.
/// Returns (cpu type, byte range of the thin image).
pub fn select_arch(d: &[u8], prefer: &[u32]) -> Result<(u32, core::ops::Range<usize>), i64> {
    match u32be(d, 0) {
        Some(m @ (0xcafe_babe | 0xcafe_babf)) => {
            let n = u32be(d, 4).ok_or(EINVAL)? as usize;
            if n == 0 || n > 64 {
                return Err(EINVAL);
            }
            let wide = m == 0xcafe_babf;
            let esz = if wide { 32 } else { 20 };
            let mut slices = Vec::new();
            for i in 0..n {
                let e = 8 + i * esz;
                let cpu = u32be(d, e).ok_or(EINVAL)?;
                let (off, size) = if wide {
                    (u64be(d, e + 8).ok_or(EINVAL)?, u64be(d, e + 16).ok_or(EINVAL)?)
                } else {
                    (u32be(d, e + 8).ok_or(EINVAL)? as u64, u32be(d, e + 12).ok_or(EINVAL)? as u64)
                };
                let end = off.checked_add(size).filter(|&e| e <= d.len() as u64).ok_or(EINVAL)?;
                slices.push((cpu, off as usize..end as usize));
            }
            prefer.iter().find_map(|p| slices.iter().find(|s| s.0 == *p).cloned()).ok_or(ENOEXEC)
        }
        _ => {
            if u32le(d, 0) != Some(0xfeed_facf) {
                return Err(ENOEXEC);
            }
            let cpu = u32le(d, 4).ok_or(ENOEXEC)?;
            if !prefer.contains(&cpu) {
                return Err(ENOEXEC);
            }
            Ok((cpu, 0..d.len()))
        }
    }
}

/// True if `d` starts like a Mach-O or universal binary.
pub fn is_macho(d: &[u8]) -> bool {
    matches!(u32le(d, 0), Some(0xfeed_facf)) || (matches!(u32be(d, 0), Some(0xcafe_babe | 0xcafe_babf)) && u32be(d, 4).is_some_and(|n| n > 0 && n < 64))
}

/// A section.
#[derive(Clone, Debug)]
pub struct Section {
    /// Section name (`__text`, `__mod_init_func`, ...).
    pub name: String,
    /// Owning segment name.
    pub seg: String,
    /// Unslid address.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
    /// Section flags (type in the low byte).
    pub flags: u32,
}

/// A segment (`LC_SEGMENT_64`).
#[derive(Clone, Debug)]
pub struct Segment {
    /// Segment name.
    pub name: String,
    /// Unslid address.
    pub vmaddr: u64,
    /// Size in memory.
    pub vmsize: u64,
    /// File offset (within the thin image).
    pub fileoff: u64,
    /// Bytes present in the file.
    pub filesize: u64,
    /// VM_PROT_* (1 R, 2 W, 4 X) — the same bits as PROT_*.
    pub initprot: u32,
}

/// Program entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// `LC_MAIN`: unslid address of `main(argc, argv, envp, apple)`.
    Main(u64),
    /// `LC_UNIXTHREAD`: unslid initial pc; the stack holds argc/argv/envp like a classic `start`.
    Thread(u64),
}

/// One pointer-sized location to fix up after mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fixup {
    /// Store `target + slide` at `addr + slide`.
    Rebase {
        /// Unslid location.
        addr: u64,
        /// Unslid target address.
        target: u64,
    },
    /// Store the address of symbol `name` from dylib `ordinal` (+ `addend`) at `addr + slide`.
    Bind {
        /// Unslid location.
        addr: u64,
        /// Dylib ordinal (1-based index into `dylibs`, or one of the `ORD_*` specials).
        ordinal: i64,
        /// Symbol name (with leading underscore).
        name: String,
        /// Added to the symbol address.
        addend: i64,
        /// Weak import: may stay unresolved (0).
        weak: bool,
    },
}

/// A parsed thin 64-bit Mach-O image.
#[derive(Clone, Debug)]
pub struct MachO {
    /// CPU type.
    pub cpu: u32,
    /// MH_EXECUTE / MH_DYLIB / MH_BUNDLE.
    pub filetype: u32,
    /// Segments in load-command order.
    pub segments: Vec<Segment>,
    /// Sections.
    pub sections: Vec<Section>,
    /// Entry point (executables).
    pub entry: Option<Entry>,
    /// Dependent dylib install names, in ordinal order (ordinal 1 is `dylibs[0]`); bool = weak.
    pub dylibs: Vec<(String, bool)>,
    /// `LC_RPATH` entries.
    pub rpaths: Vec<String>,
    /// Install name (dylibs).
    pub id: Option<String>,
    /// Pointer fixups.
    pub fixups: Vec<Fixup>,
    /// Export trie bytes.
    pub exports: Vec<u8>,
}

fn uleb(d: &[u8], p: &mut usize) -> Result<u64, i64> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let b = *d.get(*p).ok_or(EINVAL)?;
        *p += 1;
        if shift < 64 {
            v |= ((b & 0x7f) as u64) << shift;
        }
        shift += 7;
        if b & 0x80 == 0 {
            return Ok(v);
        }
        if shift > 70 {
            return Err(EINVAL);
        }
    }
}

fn sleb(d: &[u8], p: &mut usize) -> Result<i64, i64> {
    let mut v = 0i64;
    let mut shift = 0;
    loop {
        let b = *d.get(*p).ok_or(EINVAL)?;
        *p += 1;
        if shift < 64 {
            v |= ((b & 0x7f) as i64) << shift;
        }
        shift += 7;
        if b & 0x80 == 0 {
            if shift < 64 && b & 0x40 != 0 {
                v |= -1i64 << shift;
            }
            return Ok(v);
        }
        if shift > 70 {
            return Err(EINVAL);
        }
    }
}

fn blob(d: &[u8], off: u32, size: u32) -> Result<&[u8], i64> {
    d.get(off as usize..(off as usize).checked_add(size as usize).ok_or(EINVAL)?).ok_or(EINVAL)
}

impl MachO {
    /// Parse a thin image (use [`select_arch`] first for universal files).
    pub fn parse(d: &[u8]) -> Result<MachO, i64> {
        if u32le(d, 0) != Some(0xfeed_facf) {
            return Err(ENOEXEC);
        }
        let bad = || EINVAL;
        let cpu = u32le(d, 4).ok_or_else(bad)?;
        let filetype = u32le(d, 12).ok_or_else(bad)?;
        let ncmds = u32le(d, 16).ok_or_else(bad)? as usize;
        let sizeofcmds = u32le(d, 20).ok_or_else(bad)? as usize;
        if ncmds > 4096 || 32 + sizeofcmds > d.len() {
            return Err(EINVAL);
        }
        let mut m = MachO { cpu, filetype, segments: Vec::new(), sections: Vec::new(), entry: None, dylibs: Vec::new(), rpaths: Vec::new(), id: None, fixups: Vec::new(), exports: Vec::new() };
        let mut info: Option<[u32; 10]> = None;
        let mut chained: Option<(u32, u32)> = None;
        let mut o = 32;
        for _ in 0..ncmds {
            let cmd = u32le(d, o).ok_or_else(bad)?;
            let size = u32le(d, o + 4).ok_or_else(bad)? as usize;
            if size < 8 || o + size > 32 + sizeofcmds {
                return Err(EINVAL);
            }
            let c = &d[o..o + size];
            match cmd {
                LC_SEGMENT_64 => {
                    let seg = Segment {
                        name: fixed_name(c, 8),
                        vmaddr: u64le(c, 24).ok_or_else(bad)?,
                        vmsize: u64le(c, 32).ok_or_else(bad)?,
                        fileoff: u64le(c, 40).ok_or_else(bad)?,
                        filesize: u64le(c, 48).ok_or_else(bad)?,
                        initprot: u32le(c, 60).ok_or_else(bad)?,
                    };
                    if seg.fileoff.checked_add(seg.filesize).is_none_or(|e| e > d.len() as u64) || seg.filesize > seg.vmsize || seg.vmaddr.checked_add(seg.vmsize).is_none_or(|e| e > loos_lin::mem::USER_TOP) {
                        return Err(EINVAL);
                    }
                    let nsects = u32le(c, 64).ok_or_else(bad)? as usize;
                    for i in 0..nsects {
                        let s = 72 + i * 80;
                        if s + 80 > c.len() {
                            return Err(EINVAL);
                        }
                        m.sections.push(Section { name: fixed_name(c, s), seg: fixed_name(c, s + 16), addr: u64le(c, s + 32).ok_or_else(bad)?, size: u64le(c, s + 40).ok_or_else(bad)?, flags: u32le(c, s + 64).ok_or_else(bad)? });
                    }
                    m.segments.push(seg);
                }
                LC_MAIN => m.entry = Some(Entry::Main(u64le(c, 8).ok_or_else(bad)?)),
                LC_UNIXTHREAD => {
                    let flavor = u32le(c, 8).ok_or_else(bad)?;
                    let pc = match (cpu, flavor) {
                        (CPU_X86_64, 4) => u64le(c, 16 + 16 * 8),
                        (CPU_ARM64, 6) => u64le(c, 16 + 32 * 8),
                        _ => None,
                    };
                    m.entry = Some(Entry::Thread(pc.ok_or_else(bad)?));
                }
                LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LAZY_LOAD_DYLIB | LC_LOAD_UPWARD_DYLIB => {
                    let name = cstr(c, u32le(c, 8).ok_or_else(bad)? as usize).ok_or_else(bad)?;
                    m.dylibs.push((name, cmd == LC_LOAD_WEAK_DYLIB));
                }
                LC_ID_DYLIB => m.id = cstr(c, u32le(c, 8).ok_or_else(bad)? as usize),
                LC_RPATH => m.rpaths.push(cstr(c, u32le(c, 8).ok_or_else(bad)? as usize).ok_or_else(bad)?),
                LC_DYLD_INFO | LC_DYLD_INFO_ONLY => {
                    let mut v = [0u32; 10];
                    for (i, x) in v.iter_mut().enumerate() {
                        *x = u32le(c, 8 + i * 4).ok_or_else(bad)?;
                    }
                    info = Some(v);
                }
                LC_DYLD_CHAINED_FIXUPS => chained = Some((u32le(c, 8).ok_or_else(bad)?, u32le(c, 12).ok_or_else(bad)?)),
                LC_DYLD_EXPORTS_TRIE => m.exports = blob(d, u32le(c, 8).ok_or_else(bad)?, u32le(c, 12).ok_or_else(bad)?)?.to_vec(),
                _ => {}
            }
            o += size;
        }
        if let Some(v) = info {
            m.rebase_opcodes(d, blob(d, v[0], v[1])?)?;
            m.bind_opcodes(blob(d, v[2], v[3])?, false)?;
            m.bind_opcodes(blob(d, v[4], v[5])?, false)?;
            m.bind_opcodes(blob(d, v[6], v[7])?, true)?;
            if v[9] != 0 {
                m.exports = blob(d, v[8], v[9])?.to_vec();
            }
        }
        if let Some((off, size)) = chained {
            m.chained_fixups(d, blob(d, off, size)?)?;
        }
        Ok(m)
    }

    /// Lowest and highest (page-rounded) unslid addresses of the mapped segments (`__PAGEZERO` excluded).
    pub fn span(&self) -> (u64, u64) {
        let segs = || self.segments.iter().filter(|s| s.vmsize > 0 && s.initprot != 0);
        let lo = segs().map(|s| loos_lin::mem::floor(s.vmaddr)).min().unwrap_or(0);
        let hi = segs().map(|s| loos_lin::mem::ceil(s.vmaddr + s.vmsize)).max().unwrap_or(0);
        (lo, hi)
    }

    /// Unslid address of the Mach header (the `__TEXT` segment, file offset 0).
    pub fn base(&self) -> u64 {
        self.segments.iter().find(|s| s.fileoff == 0 && s.filesize > 0).map(|s| s.vmaddr).unwrap_or(0)
    }

    fn seg_addr(&self, idx: u64, off: u64) -> Result<u64, i64> {
        let s = self.segments.get(idx as usize).ok_or(EINVAL)?;
        if off >= s.vmsize {
            return Err(EINVAL);
        }
        Ok(s.vmaddr + off)
    }

    /// Read the pointer stored in the file at unslid `addr`.
    fn file_ptr(&self, d: &[u8], addr: u64) -> Result<u64, i64> {
        let s = self.segments.iter().find(|s| addr >= s.vmaddr && addr + 8 <= s.vmaddr + s.filesize).ok_or(EINVAL)?;
        u64le(d, (s.fileoff + addr - s.vmaddr) as usize).ok_or(EINVAL)
    }

    fn rebase_opcodes(&mut self, d: &[u8], ops: &[u8]) -> Result<(), i64> {
        let (mut p, mut addr) = (0usize, 0u64);
        let mut out = Vec::new();
        while p < ops.len() {
            let b = ops[p];
            p += 1;
            let (op, imm) = (b & 0xf0, (b & 0x0f) as u64);
            match op {
                0x00 => break,
                0x10 => {}
                0x20 => addr = self.seg_addr(imm, uleb(ops, &mut p)?)?,
                0x30 => addr = addr.wrapping_add(uleb(ops, &mut p)?),
                0x40 => addr = addr.wrapping_add(imm * 8),
                0x50 | 0x60 => {
                    let n = if op == 0x50 { imm } else { uleb(ops, &mut p)? };
                    for _ in 0..n.min(1 << 24) {
                        out.push(addr);
                        addr += 8;
                    }
                }
                0x70 => {
                    out.push(addr);
                    addr = addr.wrapping_add(8 + uleb(ops, &mut p)?);
                }
                0x80 => {
                    let n = uleb(ops, &mut p)?;
                    let skip = uleb(ops, &mut p)?;
                    for _ in 0..n.min(1 << 24) {
                        out.push(addr);
                        addr = addr.wrapping_add(8 + skip);
                    }
                }
                _ => return Err(EINVAL),
            }
        }
        for a in out {
            let target = self.file_ptr(d, a)?;
            self.fixups.push(Fixup::Rebase { addr: a, target });
        }
        Ok(())
    }

    fn bind_opcodes(&mut self, ops: &[u8], lazy: bool) -> Result<(), i64> {
        let (mut p, mut addr, mut ordinal, mut addend) = (0usize, 0u64, 0i64, 0i64);
        let mut name = String::new();
        let mut weak = false;
        while p < ops.len() {
            let b = ops[p];
            p += 1;
            let (op, imm) = (b & 0xf0, (b & 0x0f) as u64);
            let bind = |addr: u64| Fixup::Bind { addr, ordinal, name: name.clone(), addend, weak };
            match op {
                0x00 => {
                    if !lazy {
                        break;
                    }
                }
                0x10 => ordinal = imm as i64,
                0x20 => ordinal = uleb(ops, &mut p)? as i64,
                0x30 => ordinal = if imm == 0 { 0 } else { (imm | !0xf) as i64 },
                0x40 => {
                    weak = imm & 1 != 0;
                    let n = ops[p..].iter().position(|&c| c == 0).ok_or(EINVAL)?;
                    name = String::from_utf8(ops[p..p + n].to_vec()).map_err(|_| EINVAL)?;
                    p += n + 1;
                }
                0x50 => {}
                0x60 => addend = sleb(ops, &mut p)?,
                0x70 => addr = self.seg_addr(imm, uleb(ops, &mut p)?)?,
                0x80 => addr = addr.wrapping_add(uleb(ops, &mut p)?),
                0x90 => {
                    let f = bind(addr);
                    self.fixups.push(f);
                    addr += 8;
                }
                0xa0 => {
                    let f = bind(addr);
                    self.fixups.push(f);
                    addr = addr.wrapping_add(8 + uleb(ops, &mut p)?);
                }
                0xb0 => {
                    let f = bind(addr);
                    self.fixups.push(f);
                    addr = addr.wrapping_add(8 + imm * 8);
                }
                0xc0 => {
                    let n = uleb(ops, &mut p)?;
                    let skip = uleb(ops, &mut p)?;
                    for _ in 0..n.min(1 << 24) {
                        let f = bind(addr);
                        self.fixups.push(f);
                        addr = addr.wrapping_add(8 + skip);
                    }
                }
                _ => return Err(EINVAL),
            }
        }
        Ok(())
    }

    fn chained_fixups(&mut self, _file: &[u8], c: &[u8]) -> Result<(), i64> {
        let bad = || EINVAL;
        let starts = u32le(c, 4).ok_or_else(bad)? as usize;
        let imports_off = u32le(c, 8).ok_or_else(bad)? as usize;
        let symbols_off = u32le(c, 12).ok_or_else(bad)? as usize;
        let count = u32le(c, 16).ok_or_else(bad)? as usize;
        let format = u32le(c, 20).ok_or_else(bad)?;
        if u32le(c, 24) != Some(0) {
            return Err(EINVAL); // compressed symbol names are not used by ld64
        }
        let mut imports = Vec::with_capacity(count.min(1 << 20));
        for i in 0..count {
            let (ord, weak, name_off, addend) = match format {
                1 | 2 => {
                    let w = u32le(c, imports_off + i * if format == 1 { 4 } else { 8 }).ok_or_else(bad)?;
                    let add = if format == 2 { u32le(c, imports_off + i * 8 + 4).ok_or_else(bad)? as i32 as i64 } else { 0 };
                    ((w & 0xff) as u8 as i8 as i64, w & 0x100 != 0, (w >> 9) as usize, add)
                }
                3 => {
                    let w = u64le(c, imports_off + i * 16).ok_or_else(bad)?;
                    ((w & 0xffff) as u16 as i16 as i64, w & 0x1_0000 != 0, (w >> 32) as usize, u64le(c, imports_off + i * 16 + 8).ok_or_else(bad)? as i64)
                }
                _ => return Err(EINVAL),
            };
            // Ordinals are stored unsigned, with the specials as the top values (0xff = -1, ...).
            let ord = if ord < -3 { ord & 0xff } else { ord };
            imports.push((ord, weak, cstr(c, symbols_off + name_off).ok_or_else(bad)?, addend));
        }
        let segs = u32le(c, starts).ok_or_else(bad)? as usize;
        let base = self.base();
        let mut out = Vec::new();
        for si in 0..segs.min(self.segments.len()) {
            let so = u32le(c, starts + 4 + si * 4).ok_or_else(bad)? as usize;
            if so == 0 {
                continue;
            }
            let s = starts + so;
            let page_size = u16le(c, s + 4).ok_or_else(bad)? as u64;
            let fmt = u16le(c, s + 6).ok_or_else(bad)?;
            let seg_off = u64le(c, s + 8).ok_or_else(bad)?;
            let pages = u16le(c, s + 20).ok_or_else(bad)? as usize;
            if !matches!(fmt, 1 | 2 | 6 | 9 | 12) {
                return Err(ENOEXEC); // 32-bit and kernel-cache formats
            }
            for pi in 0..pages {
                let start = u16le(c, s + 22 + pi * 2).ok_or_else(bad)?;
                if start == 0xffff {
                    continue;
                }
                if start & 0x8000 != 0 {
                    return Err(EINVAL); // DYLD_CHAINED_PTR_START_MULTI is only used by 32-bit formats
                }
                out.push((base + seg_off + pi as u64 * page_size + start as u64, fmt));
            }
        }
        for (mut addr, fmt) in out {
            // arm64e formats (1, 9, 12) step in 8-byte units; PTR_64 formats in 4-byte units.
            let arm64e = matches!(fmt, 1 | 9 | 12);
            loop {
                let v = self.file_ptr(_file, addr)?;
                let next = if arm64e { (v >> 51) & 0x7ff } else { (v >> 51) & 0xfff };
                let bind = if arm64e { v >> 62 & 1 != 0 } else { v >> 63 != 0 };
                let auth = arm64e && v >> 63 != 0;
                if bind {
                    let ord_bits = if fmt == 12 { 0xff_ffff } else if arm64e { 0xffff } else { 0xff_ffff };
                    let (ord, weak, name, add) = imports.get((v & ord_bits) as usize).cloned().ok_or_else(bad)?;
                    let inline = if !arm64e {
                        ((v >> 24) & 0xff) as i64
                    } else if auth {
                        0
                    } else {
                        sext19((v >> 32) & 0x7_ffff)
                    };
                    self.fixups.push(Fixup::Bind { addr, ordinal: ord, name, addend: add + inline, weak });
                } else {
                    // Prism runs arm64e code without PAC keys: authenticated pointers are stored plain.
                    let target = if auth {
                        base + (v & 0xffff_ffff)
                    } else if arm64e {
                        let low = v & 0x7ff_ffff_ffff;
                        let high8 = ((v >> 43) & 0xff) << 56;
                        (if fmt == 1 { low } else { base + low }) | high8
                    } else {
                        let low = v & 0xf_ffff_ffff;
                        let high8 = ((v >> 36) & 0xff) << 56;
                        (if fmt == 6 { base + low } else { low }) | high8
                    };
                    self.fixups.push(Fixup::Rebase { addr, target });
                }
                if next == 0 {
                    break;
                }
                addr += next * if arm64e { 8 } else { 4 };
            }
        }
        Ok(())
    }

    /// Look `name` up in the export trie. Returns the unslid address, or a re-export (ordinal, name).
    pub fn export(&self, name: &str) -> Option<Export> {
        let t = &self.exports;
        let (mut node, mut rest) = (0usize, name.as_bytes());
        for _ in 0..512 {
            let mut p = node;
            let term = uleb(t, &mut p).ok()? as usize;
            if rest.is_empty() {
                if term == 0 {
                    return None;
                }
                let flags = uleb(t, &mut p).ok()?;
                if flags & 0x08 != 0 {
                    let ord = uleb(t, &mut p).ok()? as i64;
                    let n = t[p..].iter().position(|&b| b == 0)?;
                    let imported = String::from_utf8(t[p..p + n].to_vec()).ok()?;
                    return Some(Export::Reexport(ord, if imported.is_empty() { name.to_string() } else { imported }));
                }
                return Some(Export::Addr(self.base() + uleb(t, &mut p).ok()?));
            }
            // Children follow the terminal payload.
            p = p.checked_add(term)?;
            let kids = *t.get(p)? as usize;
            p += 1;
            let mut found = None;
            for _ in 0..kids {
                let n = t.get(p..)?.iter().position(|&b| b == 0)?;
                let label = &t[p..p + n];
                p += n + 1;
                let child = uleb(t, &mut p).ok()? as usize;
                if rest.starts_with(label) && !label.is_empty() {
                    found = Some((child, label.len()));
                    break;
                }
            }
            let (child, used) = found?;
            node = child;
            rest = &rest[used..];
        }
        None
    }
}

/// An export-trie hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Export {
    /// Unslid address in the image.
    Addr(u64),
    /// Re-exported from dependency `ordinal` under the given name.
    Reexport(i64, String),
}

/// Sign-extend a 19-bit field (arm64e inline bind addends).
fn sext19(v: u64) -> i64 {
    ((v << 45) as i64) >> 45
}

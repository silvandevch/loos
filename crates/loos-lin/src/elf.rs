//! ELF64 x86-64 parsing with full bounds checking (input is untrusted).

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::errno::*;

/// ET_EXEC.
pub const ET_EXEC: u16 = 2;
/// ET_DYN.
pub const ET_DYN: u16 = 3;
/// PT_LOAD.
pub const PT_LOAD: u32 = 1;
/// PT_INTERP.
pub const PT_INTERP: u32 = 3;
/// PT_PHDR.
pub const PT_PHDR: u32 = 6;
/// PT_TLS.
pub const PT_TLS: u32 = 7;
/// PT_GNU_STACK.
pub const PT_GNU_STACK: u32 = 0x6474_e551;

/// Program header.
#[derive(Clone, Copy, Debug)]
pub struct Phdr {
    /// p_type.
    pub kind: u32,
    /// p_flags (1=X 2=W 4=R).
    pub flags: u32,
    /// p_offset.
    pub offset: u64,
    /// p_vaddr.
    pub vaddr: u64,
    /// p_filesz.
    pub filesz: u64,
    /// p_memsz.
    pub memsz: u64,
    /// p_align.
    pub align: u64,
}

/// A validated ELF image description.
#[derive(Clone, Debug)]
pub struct Elf {
    /// ET_EXEC or ET_DYN.
    pub etype: u16,
    /// Entry (unbiased).
    pub entry: u64,
    /// e_phoff.
    pub phoff: u64,
    /// Program headers.
    pub phdrs: Vec<Phdr>,
    /// PT_INTERP path.
    pub interp: Option<String>,
    /// Executable stack requested.
    pub exec_stack: bool,
}

fn u16le(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn u32le(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}
fn u64le(d: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(o..o + 8)?.try_into().ok()?))
}

impl Elf {
    /// Parse and validate. Errors are errnos (`ENOEXEC` for non-ELF/foreign, `EINVAL` for malformed).
    pub fn parse(d: &[u8]) -> Result<Elf, i64> {
        if d.len() < 64 || &d[..4] != b"\x7fELF" {
            return Err(ENOEXEC);
        }
        if d[4] != 2 || d[5] != 1 || d[6] != 1 {
            return Err(ENOEXEC);
        }
        let bad = || EINVAL;
        let etype = u16le(d, 16).ok_or_else(bad)?;
        if u16le(d, 18) != Some(62) || !(etype == ET_EXEC || etype == ET_DYN) {
            return Err(ENOEXEC);
        }
        let entry = u64le(d, 24).ok_or_else(bad)?;
        let phoff = u64le(d, 32).ok_or_else(bad)?;
        let phentsize = u16le(d, 54).ok_or_else(bad)? as usize;
        let phnum = u16le(d, 56).ok_or_else(bad)? as usize;
        if phentsize != 56 || phnum == 0 || phnum > 128 {
            return Err(EINVAL);
        }
        let tab_end = phoff.checked_add((phnum * 56) as u64).ok_or_else(bad)?;
        if tab_end > d.len() as u64 {
            return Err(EINVAL);
        }
        let mut phdrs = Vec::with_capacity(phnum);
        let mut interp = None;
        let mut exec_stack = false;
        let mut loads: Vec<(u64, u64)> = Vec::new();
        for i in 0..phnum {
            let o = phoff as usize + i * 56;
            let p = Phdr {
                kind: u32le(d, o).ok_or_else(bad)?,
                flags: u32le(d, o + 4).ok_or_else(bad)?,
                offset: u64le(d, o + 8).ok_or_else(bad)?,
                vaddr: u64le(d, o + 16).ok_or_else(bad)?,
                filesz: u64le(d, o + 32).ok_or_else(bad)?,
                memsz: u64le(d, o + 40).ok_or_else(bad)?,
                align: u64le(d, o + 48).ok_or_else(bad)?,
            };
            match p.kind {
                PT_LOAD => {
                    let end = p.vaddr.checked_add(p.memsz).filter(|&e| e <= crate::mem::USER_TOP).ok_or_else(bad)?;
                    if p.filesz > p.memsz || p.offset.checked_add(p.filesz).is_none_or(|e| e > d.len() as u64) {
                        return Err(EINVAL);
                    }
                    if p.align > 1 && (!p.align.is_power_of_two() || p.vaddr % p.align != p.offset % p.align) {
                        return Err(EINVAL);
                    }
                    let (s, e) = (crate::mem::floor(p.vaddr), crate::mem::ceil(end));
                    if loads.iter().any(|&(a, b)| s < b && e > a) {
                        return Err(EINVAL);
                    }
                    loads.push((s, e));
                }
                PT_INTERP => {
                    let end = p.offset.checked_add(p.filesz).filter(|&e| e <= d.len() as u64 && p.filesz > 1 && p.filesz <= 4096).ok_or_else(bad)?;
                    let raw = &d[p.offset as usize..end as usize];
                    if *raw.last().unwrap() != 0 {
                        return Err(EINVAL);
                    }
                    interp = Some(String::from_utf8(raw[..raw.len() - 1].to_vec()).map_err(|_| EINVAL)?);
                }
                PT_GNU_STACK => exec_stack = p.flags & 1 != 0,
                _ => {}
            }
            phdrs.push(p);
        }
        if loads.is_empty() {
            return Err(EINVAL);
        }
        Ok(Elf { etype, entry, phoff, phdrs, interp, exec_stack })
    }

    /// Lowest/highest page-aligned address spanned by PT_LOAD (unbiased).
    pub fn span(&self) -> (u64, u64) {
        let lo = self.phdrs.iter().filter(|p| p.kind == PT_LOAD).map(|p| crate::mem::floor(p.vaddr)).min().unwrap_or(0);
        let hi = self.phdrs.iter().filter(|p| p.kind == PT_LOAD).map(|p| crate::mem::ceil(p.vaddr + p.memsz)).max().unwrap_or(0);
        (lo, hi)
    }

    /// PT_TLS segment, if any.
    pub fn tls(&self) -> Option<&Phdr> {
        self.phdrs.iter().find(|p| p.kind == PT_TLS)
    }
}

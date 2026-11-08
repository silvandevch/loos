//! PE32+ loader: maps headers and sections at the preferred base, binds imports to syscall stubs.

use crate::api;
use crate::PeError;
use alloc::string::String;
use alloc::vec::Vec;
use loos_lin::mem::{ceil, UserMem, PAGE, PROT_EXEC, PROT_READ, PROT_WRITE, USER_TOP};

const SCN_EXEC: u32 = 0x2000_0000;
const SCN_READ: u32 = 0x4000_0000;
const SCN_WRITE: u32 = 0x8000_0000;
/// Bytes per generated stub.
pub const STUB: usize = 16;

/// Result of a successful load.
pub struct Loaded {
    /// Entry point address.
    pub entry: u64,
    /// Image base.
    pub base: u64,
    /// Stub that terminates the process (the entry point's return address).
    pub exit_stub: u64,
    /// Stub that returns from a window-procedure callback.
    pub cb_stub: u64,
    /// Stub that ends a thread (its return address).
    pub thread_exit_stub: u64,
    /// Names of imports without an implementation.
    pub unresolved: Vec<String>,
    /// Data symbols and runtime cells of the C runtime.
    pub arena: crate::crt::DataArena,
}

fn u16_at(b: &[u8], o: usize) -> Result<u16, PeError> {
    b.get(o..o + 2).map(|x| u16::from_le_bytes([x[0], x[1]])).ok_or(PeError::Malformed("truncated"))
}
fn u32_at(b: &[u8], o: usize) -> Result<u32, PeError> {
    b.get(o..o + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]])).ok_or(PeError::Malformed("truncated"))
}
fn u64_at(b: &[u8], o: usize) -> Result<u64, PeError> {
    b.get(o..o + 8).map(|x| u64::from_le_bytes(x.try_into().unwrap())).ok_or(PeError::Malformed("truncated"))
}

fn cstr(b: &[u8], o: usize) -> Result<&str, PeError> {
    let s = b.get(o..).ok_or(PeError::Malformed("string rva"))?;
    let n = s.iter().position(|&c| c == 0).ok_or(PeError::Malformed("unterminated string"))?;
    core::str::from_utf8(&s[..n]).map_err(|_| PeError::Malformed("non-ascii name"))
}

/// `mov r10, rcx; mov eax, nr; syscall; ret` (the NT stub shape), padded to [`STUB`] bytes.
pub fn stub(nr: u32) -> [u8; STUB] {
    let mut s = [0xccu8; STUB];
    s[0..3].copy_from_slice(&[0x4c, 0x8b, 0xd1]);
    s[3] = 0xb8;
    s[4..8].copy_from_slice(&nr.to_le_bytes());
    s[8..10].copy_from_slice(&[0x0f, 0x05]);
    s[10] = 0xc3;
    s
}

/// `mov r10, rax; mov eax, CB_RET; syscall`: ends a window-procedure callback, handing back its result.
pub fn cb_stub_bytes() -> [u8; STUB] {
    let mut s = [0xccu8; STUB];
    s[0..3].copy_from_slice(&[0x49, 0x89, 0xc2]);
    s[3] = 0xb8;
    s[4..8].copy_from_slice(&crate::gui::CB_RET.to_le_bytes());
    s[8..10].copy_from_slice(&[0x0f, 0x05]);
    s[10] = 0xc3;
    s
}

/// Maps `file` into `mem`.
pub fn load(mem: &mut UserMem, file: &[u8]) -> Result<Loaded, PeError> {
    if file.get(0..2) != Some(b"MZ") {
        return Err(PeError::Malformed("no MZ"));
    }
    let nt = u32_at(file, 0x3c)? as usize;
    if file.get(nt..nt + 4) != Some(b"PE\0\0") {
        return Err(PeError::Malformed("no PE signature"));
    }
    if u16_at(file, nt + 4)? != 0x8664 {
        return Err(PeError::Unsupported("not x86-64"));
    }
    let nsec = usize::from(u16_at(file, nt + 6)?);
    let opt_size = usize::from(u16_at(file, nt + 20)?);
    let opt = nt + 24;
    if u16_at(file, opt)? != 0x20b {
        return Err(PeError::Unsupported("not PE32+"));
    }
    let entry_rva = u32_at(file, opt + 16)? as u64;
    let base = u64_at(file, opt + 24)?;
    let size_of_image = u32_at(file, opt + 56)? as usize;
    let size_of_headers = u32_at(file, opt + 60)? as usize;
    if base % PAGE != 0 || base < 0x1_0000 || size_of_image == 0 || size_of_image > (256 << 20) || base + ceil(size_of_image as u64) + 0x10_0000 > USER_TOP {
        return Err(PeError::NoMemory);
    }
    if size_of_headers > size_of_image || size_of_headers > file.len() {
        return Err(PeError::Malformed("headers"));
    }
    let mut img = alloc::vec![0u8; size_of_image];
    img[..size_of_headers].copy_from_slice(&file[..size_of_headers]);

    struct Sec {
        rva: usize,
        len: usize,
        prot: u32,
    }
    let mut secs = Vec::new();
    for i in 0..nsec {
        let h = opt + opt_size + 40 * i;
        let vsize = u32_at(file, h + 8)? as usize;
        let rva = u32_at(file, h + 12)? as usize;
        let rsize = u32_at(file, h + 16)? as usize;
        let rptr = u32_at(file, h + 20)? as usize;
        let ch = u32_at(file, h + 36)?;
        let len = vsize.max(rsize);
        if rva % PAGE as usize != 0 || rva.checked_add(len).is_none_or(|e| e > size_of_image) {
            return Err(PeError::Malformed("section outside image"));
        }
        let take = rsize.min(vsize.max(1).max(rsize)).min(len);
        if rsize > 0 {
            let raw = file.get(rptr..rptr + take).ok_or(PeError::Malformed("section raw data"))?;
            img[rva..rva + take].copy_from_slice(raw);
        }
        let mut prot = 0;
        if ch & SCN_READ != 0 {
            prot |= PROT_READ;
        }
        if ch & SCN_WRITE != 0 {
            prot |= PROT_WRITE | PROT_READ;
        }
        if ch & SCN_EXEC != 0 {
            prot |= PROT_EXEC | PROT_READ;
        }
        secs.push(Sec { rva, len, prot });
    }

    // Imports: patch the IAT in the staging buffer, pointing each slot at a stub.
    let stub_base = base + ceil(size_of_image as u64);
    let mut stubs: Vec<[u8; STUB]> = Vec::new();
    let mut unresolved = Vec::new();
    let arena_base = mem.map_anywhere(0x8000, PROT_READ | PROT_WRITE, &[], "crtdata").unwrap_or(0);
    let mut arena = crate::crt::DataArena::new(arena_base, 0x8000);
    let exit_stub_index = {
        stubs.push(stub(api::EXIT_PROCESS));
        stubs.push(cb_stub_bytes());
        let mut te = cb_stub_bytes();
        te[4..8].copy_from_slice(&crate::thread::THREAD_EXIT.to_le_bytes());
        stubs.push(te);
        0
    };
    let imp_rva = u32_at(file, opt + 112 + 8)? as usize;
    if imp_rva != 0 {
        let mut d = imp_rva;
        loop {
            let (oft, name_rva, ft) = (u32_at(&img, d)? as usize, u32_at(&img, d + 12)? as usize, u32_at(&img, d + 16)? as usize);
            if name_rva == 0 && ft == 0 {
                break;
            }
            let dll = String::from(cstr(&img, name_rva)?);
            let mut t = if oft != 0 { oft } else { ft };
            let mut slot = ft;
            loop {
                let thunk = u64_at(&img, t)?;
                if thunk == 0 {
                    break;
                }
                if thunk >> 63 == 0 {
                    // Data imports (`_fmode`, `_iob`, ...): the IAT slot holds the variable's address, not a code stub.
                    let fname = cstr(&img, thunk as usize + 2)?;
                    if let Some(size) = crate::crt::data_size(fname) {
                        if let Some(addr) = arena.cell(fname, size) {
                            img.get_mut(slot..slot + 8).ok_or(PeError::Malformed("IAT outside image"))?.copy_from_slice(&addr.to_le_bytes());
                            t += 8;
                            slot += 8;
                            continue;
                        }
                    }
                }
                let nr = if thunk >> 63 != 0 {
                    unresolved.push(alloc::format!("{dll}!#{}", thunk & 0xffff));
                    api::UNRESOLVED_BASE + unresolved.len() as u32 - 1
                } else {
                    let fname = cstr(&img, thunk as usize + 2)?;
                    match api::lookup(fname) {
                        Some(nr) => nr,
                        None => {
                            unresolved.push(alloc::format!("{dll}!{fname}"));
                            api::UNRESOLVED_BASE + unresolved.len() as u32 - 1
                        }
                    }
                };
                stubs.push(stub(nr));
                let addr = stub_base + ((stubs.len() - 1) * STUB) as u64;
                img.get_mut(slot..slot + 8).ok_or(PeError::Malformed("IAT outside image"))?.copy_from_slice(&addr.to_le_bytes());
                t += 8;
                slot += 8;
            }
            d += 20;
        }
    }

    // Map headers read-only, then sections, then the stub page.
    mem.map_fixed(base, size_of_headers.max(1) as u64, PROT_READ, &img[..size_of_headers], "headers").map_err(|_| PeError::NoMemory)?;
    for s in &secs {
        if s.len == 0 {
            continue;
        }
        let end = (s.rva + s.len).min(size_of_image);
        mem.map_fixed(base + s.rva as u64, s.len as u64, s.prot, &img[s.rva..end], "section").map_err(|_| PeError::NoMemory)?;
    }
    let flat: Vec<u8> = stubs.iter().flatten().copied().collect();
    mem.map_fixed(stub_base, flat.len() as u64, PROT_READ | PROT_EXEC, &flat, "stubs").map_err(|_| PeError::NoMemory)?;
    Ok(Loaded { entry: base + entry_rva, base, exit_stub: stub_base + (exit_stub_index * STUB) as u64, cb_stub: stub_base + STUB as u64, thread_exit_stub: stub_base + 2 * STUB as u64, unresolved, arena })
}

/// One import of an image: by name or by ordinal, with the RVA of its IAT slot.
pub struct Import {
    /// Imported DLL file name.
    pub dll: String,
    /// Function name (None: by ordinal).
    pub name: Option<String>,
    /// Ordinal when imported by ordinal.
    pub ord: u32,
    /// IAT slot RVA.
    pub slot: usize,
}

/// One export of an image.
pub struct Export {
    /// Exported name, if any.
    pub name: Option<String>,
    /// Ordinal (biased).
    pub ord: u32,
    /// Function RVA.
    pub rva: u32,
}

/// A parsed, not yet mapped PE32+ image (headers and sections copied into a flat buffer).
pub struct Staged {
    /// Flat image, `size` bytes.
    pub img: Vec<u8>,
    /// Image size.
    pub size: usize,
    /// Entry point RVA.
    pub entry_rva: u64,
    /// Preferred base from the headers.
    pub pref_base: u64,
    headers: usize,
    opt: usize,
    secs: Vec<(usize, usize, u32)>,
}

/// Parses `file` into a [`Staged`] image.
pub fn stage(file: &[u8]) -> Result<Staged, PeError> {
    if file.get(0..2) != Some(b"MZ") {
        return Err(PeError::Malformed("no MZ"));
    }
    let nt = u32_at(file, 0x3c)? as usize;
    if file.get(nt..nt + 4) != Some(b"PE\0\0") {
        return Err(PeError::Malformed("no PE signature"));
    }
    if u16_at(file, nt + 4)? != 0x8664 {
        return Err(PeError::Unsupported("not x86-64"));
    }
    let nsec = usize::from(u16_at(file, nt + 6)?);
    let opt_size = usize::from(u16_at(file, nt + 20)?);
    let opt = nt + 24;
    if u16_at(file, opt)? != 0x20b {
        return Err(PeError::Unsupported("not PE32+"));
    }
    let size = u32_at(file, opt + 56)? as usize;
    let headers = u32_at(file, opt + 60)? as usize;
    if size == 0 || size > (256 << 20) || headers > size || headers > file.len() {
        return Err(PeError::Malformed("sizes"));
    }
    let mut img = alloc::vec![0u8; size];
    img[..headers].copy_from_slice(&file[..headers]);
    let mut secs = Vec::new();
    for i in 0..nsec {
        let h = opt + opt_size + 40 * i;
        let (vsize, rva, rsize, rptr, ch) = (u32_at(file, h + 8)? as usize, u32_at(file, h + 12)? as usize, u32_at(file, h + 16)? as usize, u32_at(file, h + 20)? as usize, u32_at(file, h + 36)?);
        let len = vsize.max(rsize);
        if rva % PAGE as usize != 0 || rva.checked_add(len).is_none_or(|e| e > size) {
            return Err(PeError::Malformed("section outside image"));
        }
        if rsize > 0 {
            let raw = file.get(rptr..rptr + rsize).ok_or(PeError::Malformed("section raw data"))?;
            img[rva..rva + rsize].copy_from_slice(raw);
        }
        let mut prot = 0;
        if ch & SCN_READ != 0 {
            prot |= PROT_READ;
        }
        if ch & SCN_WRITE != 0 {
            prot |= PROT_WRITE | PROT_READ;
        }
        if ch & SCN_EXEC != 0 {
            prot |= PROT_EXEC | PROT_READ;
        }
        secs.push((rva, len, prot));
    }
    Ok(Staged { size, entry_rva: u32_at(file, opt + 16)? as u64, pref_base: u64_at(file, opt + 24)?, img, headers, opt, secs })
}

impl Staged {
    fn dir(&self, n: usize) -> Result<(usize, usize), PeError> {
        Ok((u32_at(&self.img, self.opt + 112 + n * 8)? as usize, u32_at(&self.img, self.opt + 116 + n * 8)? as usize))
    }

    /// Import table entries.
    pub fn imports(&self) -> Result<Vec<Import>, PeError> {
        let mut out = Vec::new();
        let (mut d, _) = self.dir(1)?;
        if d == 0 {
            return Ok(out);
        }
        loop {
            let (oft, name_rva, ft) = (u32_at(&self.img, d)? as usize, u32_at(&self.img, d + 12)? as usize, u32_at(&self.img, d + 16)? as usize);
            if name_rva == 0 && ft == 0 {
                break;
            }
            let dll = String::from(cstr(&self.img, name_rva)?);
            let (mut t, mut slot) = (if oft != 0 { oft } else { ft }, ft);
            loop {
                let thunk = u64_at(&self.img, t)?;
                if thunk == 0 {
                    break;
                }
                out.push(if thunk >> 63 != 0 {
                    Import { dll: dll.clone(), name: None, ord: (thunk & 0xffff) as u32, slot }
                } else {
                    Import { dll: dll.clone(), name: Some(String::from(cstr(&self.img, thunk as usize + 2)?)), ord: 0, slot }
                });
                t += 8;
                slot += 8;
            }
            d += 20;
        }
        Ok(out)
    }

    /// Export table entries (forwarders are skipped).
    pub fn exports(&self) -> Result<Vec<Export>, PeError> {
        let (e, esize) = self.dir(0)?;
        let mut out = Vec::new();
        if e == 0 {
            return Ok(out);
        }
        let (ord_base, nfuncs, nnames) = (u32_at(&self.img, e + 16)?, u32_at(&self.img, e + 20)? as usize, u32_at(&self.img, e + 24)? as usize);
        let (funcs, names, ords) = (u32_at(&self.img, e + 28)? as usize, u32_at(&self.img, e + 32)? as usize, u32_at(&self.img, e + 36)? as usize);
        let mut named = alloc::vec![None; nfuncs];
        for i in 0..nnames {
            let idx = u16_at(&self.img, ords + 2 * i)? as usize;
            if idx < nfuncs {
                named[idx] = Some(String::from(cstr(&self.img, u32_at(&self.img, names + 4 * i)? as usize)?));
            }
        }
        for (i, name) in named.into_iter().enumerate() {
            let rva = u32_at(&self.img, funcs + 4 * i)?;
            if rva != 0 && !(e..e + esize).contains(&(rva as usize)) {
                out.push(Export { name, ord: ord_base + i as u32, rva });
            }
        }
        Ok(out)
    }

    /// `(rva, size)` of the exception table.
    pub fn pdata(&self) -> (u32, u32) {
        self.dir(3).map_or((0, 0), |(a, b)| (a as u32, b as u32))
    }

    /// Applies IAT `patches` (slot RVA, address) and base relocations for `base`, then maps the image.
    pub fn map(mut self, mem: &mut UserMem, base: u64, patches: &[(usize, u64)]) -> Result<(), PeError> {
        for &(slot, addr) in patches {
            self.img.get_mut(slot..slot + 8).ok_or(PeError::Malformed("IAT outside image"))?.copy_from_slice(&addr.to_le_bytes());
        }
        let delta = base.wrapping_sub(self.pref_base);
        let (r, rsize) = self.dir(5)?;
        if r != 0 && delta != 0 {
            let mut b = r;
            while b < r + rsize {
                let (page, bsize) = (u32_at(&self.img, b)? as usize, u32_at(&self.img, b + 4)? as usize);
                if bsize < 8 {
                    break;
                }
                for k in 0..(bsize - 8) / 2 {
                    let e = u16_at(&self.img, b + 8 + 2 * k)?;
                    let at = page + (e & 0xfff) as usize;
                    match e >> 12 {
                        10 => {
                            let v = u64_at(&self.img, at)?.wrapping_add(delta);
                            self.img.get_mut(at..at + 8).ok_or(PeError::Malformed("reloc"))?.copy_from_slice(&v.to_le_bytes());
                        }
                        3 => {
                            let v = u32_at(&self.img, at)?.wrapping_add(delta as u32);
                            self.img.get_mut(at..at + 4).ok_or(PeError::Malformed("reloc"))?.copy_from_slice(&v.to_le_bytes());
                        }
                        _ => {}
                    }
                }
                b += bsize;
            }
        }
        mem.map_fixed(base, self.headers.max(1) as u64, PROT_READ, &self.img[..self.headers], "dll headers").map_err(|_| PeError::NoMemory)?;
        for &(rva, len, prot) in &self.secs {
            if len == 0 {
                continue;
            }
            let end = (rva + len).min(self.size);
            mem.map_fixed(base + rva as u64, len as u64, prot, &self.img[rva..end], "dll section").map_err(|_| PeError::NoMemory)?;
        }
        Ok(())
    }
}

//! User address-space model: page-granular regions with protections, brk, mmap area,
//! and a growable stack. The kernel backs this with its VM manager; the semantics
//! (overlap replacement, splitting on partial munmap/mprotect) match Linux.

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::errno::*;
use core::cell::RefCell;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// One 4 KiB page. Backing memory is page-aligned so the kernel can map it straight into a
/// user page table (the model's bytes *are* the process's physical pages).
#[repr(C, align(4096))]
#[derive(Clone)]
pub struct Page(pub [u8; 4096]);

/// Page-aligned, zero-initialised byte buffer (length is a multiple of the page size).
#[derive(Clone, Default)]
pub struct PageBuf {
    pages: Vec<Page>,
}

impl core::fmt::Debug for PageBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PageBuf({} pages)", self.pages.len())
    }
}

impl PageBuf {
    /// Zeroed buffer of `len` bytes (rounded up to pages).
    pub fn zeroed(len: u64) -> Self {
        Self { pages: vec![Page([0; 4096]); (len as usize).div_ceil(4096)] }
    }
    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.pages.len() * 4096
    }
    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
    /// Bytes.
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: Page is repr(C) [u8; 4096] with no padding; the Vec owns len()*4096 contiguous bytes.
        unsafe { core::slice::from_raw_parts(self.pages.as_ptr().cast::<u8>(), self.len()) }
    }
    /// Mutable bytes.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as above, exclusive borrow.
        unsafe { core::slice::from_raw_parts_mut(self.pages.as_mut_ptr().cast::<u8>(), self.len()) }
    }
    /// Split at `byte` (page aligned), returning the tail.
    pub fn split_off(&mut self, byte: usize) -> PageBuf {
        PageBuf { pages: self.pages.split_off(byte / 4096) }
    }
    /// Prepend `bytes` of zero pages (stack growth).
    pub fn prepend_zero(&mut self, bytes: usize) {
        let mut v = vec![Page([0; 4096]); bytes / 4096];
        v.append(&mut self.pages);
        self.pages = v;
    }
    /// Address of page `i` (identity-mapped kernel memory: this is its physical address).
    pub fn page_addr(&self, i: usize) -> u64 {
        &self.pages[i] as *const Page as u64
    }
}

/// Shared backing store (MAP_SHARED / vfork-shared memory).
pub type SharedBuf = Rc<RefCell<PageBuf>>;

/// PROT_READ.
pub const PROT_READ: u32 = 1;
/// PROT_WRITE.
pub const PROT_WRITE: u32 = 2;
/// PROT_EXEC.
pub const PROT_EXEC: u32 = 4;
/// Page size.
pub const PAGE: u64 = 4096;
/// Highest user address + 1.
pub const USER_TOP: u64 = 0x0000_8000_0000_0000;
/// Stack top.
pub const STACK_TOP: u64 = 0x7fff_ffff_f000;
/// Maximum stack size (RLIMIT_STACK default).
pub const STACK_MAX: u64 = 8 << 20;
/// Top of the mmap area (grows down).
pub const MMAP_TOP: u64 = 0x7fff_f7e0_0000;
/// Per-process mapped-memory cap for the model.
pub const MEM_CAP: u64 = 2 << 30;

/// Round down to page.
pub fn floor(a: u64) -> u64 {
    a & !(PAGE - 1)
}
/// Round up to page.
pub fn ceil(a: u64) -> u64 {
    a.checked_add(PAGE - 1).map_or(u64::MAX & !(PAGE - 1), |v| v & !(PAGE - 1))
}

/// One mapped region.
#[derive(Clone, Debug)]
pub struct Region {
    /// Start (page aligned).
    pub start: u64,
    /// Protection bits.
    pub prot: u32,
    /// Backing bytes (`len() == size`).
    pub data: PageBuf,
    /// Label shown in /proc/self/maps.
    pub name: String,
    /// When set the bytes live in a buffer shared with other address spaces
    /// (`data` is empty); `(buffer, offset, length)`.
    pub shared: Option<(SharedBuf, usize, usize)>,
}

impl Region {
    /// Size in bytes.
    pub fn size(&self) -> u64 {
        match &self.shared {
            Some((_, _, n)) => *n as u64,
            None => self.data.len() as u64,
        }
    }
    /// End (exclusive).
    pub fn end(&self) -> u64 {
        self.start + self.size()
    }
}

/// Process address space.
#[derive(Clone, Debug)]
pub struct UserMem {
    regions: BTreeMap<u64, Region>,
    /// Initial program break.
    pub brk_start: u64,
    /// Current program break.
    pub brk: u64,
    mmap_next: u64,
    stack: Option<u64>,
    total: u64,
    /// Bumped on every change of the mapping layout / protections (page-table cache key).
    pub version: u64,
}

impl Default for UserMem {
    fn default() -> Self {
        Self::new()
    }
}

impl UserMem {
    /// Empty address space.
    pub fn new() -> Self {
        Self { regions: BTreeMap::new(), brk_start: 0, brk: 0, mmap_next: MMAP_TOP, stack: None, total: 0, version: NEXT_EPOCH.fetch_add(1 << 20, Ordering::Relaxed) }
    }

    /// All regions in address order.
    pub fn regions(&self) -> impl Iterator<Item = &Region> {
        self.regions.values()
    }

    /// Bytes currently mapped.
    pub fn mapped(&self) -> u64 {
        self.total
    }

    fn overlaps(&self, start: u64, end: u64) -> bool {
        self.regions.values().any(|r| r.start < end && r.end() > start)
    }

    /// Is [start,end) completely unmapped?
    pub fn is_free(&self, start: u64, end: u64) -> bool {
        !self.overlaps(start, end)
    }

    fn split_at(&mut self, addr: u64) {
        let Some((&k, r)) = self.regions.range_mut(..addr).next_back() else { return };
        if r.end() > addr {
            let cut = (addr - k) as usize;
            let (tail, shared) = match r.shared.as_mut() {
                Some((buf, off, n)) => {
                    let t = (buf.clone(), *off + cut, *n - cut);
                    *n = cut;
                    (PageBuf::default(), Some(t))
                }
                None => (r.data.split_off(cut), None),
            };
            let nr = Region { start: addr, prot: r.prot, data: tail, name: r.name.clone(), shared };
            self.regions.insert(addr, nr);
        }
    }

    /// Map [start, start+len) replacing whatever was there. `init` is copied to the start.
    pub fn map_fixed(&mut self, start: u64, len: u64, prot: u32, init: &[u8], name: &str) -> Result<(), i64> {
        if start % PAGE != 0 || len == 0 {
            return Err(EINVAL);
        }
        let len = ceil(len);
        let end = start.checked_add(len).filter(|&e| e <= USER_TOP).ok_or(ENOMEM)?;
        let freed: u64 = self.regions.values().filter(|r| r.start < end && r.end() > start).map(|r| r.end().min(end) - r.start.max(start)).sum();
        if self.total - freed + len > MEM_CAP {
            return Err(ENOMEM);
        }
        self.unmap(start, len)?;
        let mut data = PageBuf::zeroed(len);
        let n = init.len().min(data.len());
        data.as_mut_slice()[..n].copy_from_slice(&init[..n]);
        self.version += 1;
        self.regions.insert(start, Region { start, prot, data, name: name.to_string(), shared: None });
        self.total += len;
        Ok(())
    }

    /// Convert a region (or new mapping) to shared backing. Used for `MAP_SHARED`.
    pub fn make_shared(&mut self, start: u64, len: u64) {
        let end = start + ceil(len);
        self.split_at(start);
        self.split_at(end);
        self.version += 1;
        for (_, r) in self.regions.range_mut(start..end) {
            if r.shared.is_none() {
                let d = core::mem::take(&mut r.data);
                let n = d.len();
                r.shared = Some((Rc::new(RefCell::new(d)), 0, n));
            }
        }
    }

    /// Make every mapping shared and return a second address space aliasing the same bytes
    /// (`CLONE_VM` without `CLONE_THREAD`: vfork / posix_spawn). Later (un)mappings are private.
    pub fn share_all(&mut self) -> UserMem {
        let all: Vec<(u64, u64)> = self.regions.values().map(|r| (r.start, r.size())).collect();
        for (s, l) in all {
            self.make_shared(s, l);
        }
        self.stack = None;
        let mut c = self.clone();
        c.total = self.total;
        c
    }

    /// Find a free gap below the mmap pointer and map it.
    pub fn map_anywhere(&mut self, len: u64, prot: u32, init: &[u8], name: &str) -> Result<u64, i64> {
        if len == 0 {
            return Err(EINVAL);
        }
        let len = ceil(len);
        let mut top = self.mmap_next;
        loop {
            let start = top.checked_sub(len).filter(|&s| s >= 0x1_0000).ok_or(ENOMEM)?;
            match self.regions.values().find(|r| r.start < top && r.end() > start) {
                Some(r) => top = r.start,
                None => {
                    self.map_fixed(start, len, prot, init, name)?;
                    self.mmap_next = start;
                    return Ok(start);
                }
            }
        }
    }

    /// Unmap [start, start+len) (partial regions are split). Unmapped holes are fine.
    pub fn unmap(&mut self, start: u64, len: u64) -> Result<(), i64> {
        if start % PAGE != 0 {
            return Err(EINVAL);
        }
        let end = start.checked_add(ceil(len)).ok_or(EINVAL)?;
        self.split_at(start);
        self.split_at(end);
        self.version += 1;
        let keys: Vec<u64> = self.regions.range(start..end).map(|(&k, _)| k).collect();
        for k in keys {
            if let Some(r) = self.regions.remove(&k) {
                self.total -= r.size();
            }
        }
        Ok(())
    }

    /// `mprotect`: the whole range must be mapped.
    pub fn protect(&mut self, start: u64, len: u64, prot: u32) -> Result<(), i64> {
        if start % PAGE != 0 {
            return Err(EINVAL);
        }
        let end = start.checked_add(ceil(len)).ok_or(EINVAL)?;
        let mut cur = start;
        while cur < end {
            let r = self.find(cur).ok_or(ENOMEM)?;
            cur = r.end();
        }
        self.split_at(start);
        self.split_at(end);
        self.version += 1;
        for (_, r) in self.regions.range_mut(start..end) {
            r.prot = prot;
        }
        Ok(())
    }

    fn find(&self, addr: u64) -> Option<&Region> {
        self.regions.range(..=addr).next_back().map(|(_, r)| r).filter(|r| r.end() > addr)
    }

    /// Protection of the page containing `addr`.
    pub fn prot_at(&self, addr: u64) -> Option<u32> {
        self.find(addr).map(|r| r.prot)
    }

    /// Mark the region at `top` as the growable stack (grows down to `top - STACK_MAX`).
    pub fn set_stack(&mut self, start: u64) {
        self.stack = Some(start);
    }

    fn grow_stack(&mut self, addr: u64) {
        let Some(s) = self.stack else { return };
        let Some(cur) = self.regions.get(&s).filter(|r| r.shared.is_none()).map(|r| (r.end(), r.prot, r.data.len())) else { return };
        let low = cur.0 - STACK_MAX;
        let want = floor(addr);
        if want >= s || want < low || self.regions.range(want..s).next().is_some() {
            return;
        }
        let mut r = self.regions.remove(&s).unwrap();
        r.data.prepend_zero((s - want) as usize);
        self.version += 1;
        r.start = want;
        self.total += s - want;
        self.stack = Some(want);
        self.regions.insert(want, r);
    }

    /// Copy `buf.len()` bytes out of user memory (kernel access: ignores PROT).
    pub fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), i64> {
        let mut a = addr;
        let mut off = 0;
        while off < buf.len() {
            if self.find(a).is_none() {
                self.grow_stack(a);
            }
            let r = self.find(a).ok_or(EFAULT)?;
            let n = ((r.end() - a) as usize).min(buf.len() - off);
            let s = (a - r.start) as usize;
            match &r.shared {
                Some((b, o, _)) => buf[off..off + n].copy_from_slice(&b.borrow().as_slice()[o + s..o + s + n]),
                None => buf[off..off + n].copy_from_slice(&r.data.as_slice()[s..s + n]),
            }
            off += n;
            a = a.checked_add(n as u64).ok_or(EFAULT)?;
        }
        Ok(())
    }

    /// Copy bytes into user memory (kernel access: ignores PROT).
    pub fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), i64> {
        let mut a = addr;
        let mut off = 0;
        while off < data.len() {
            if self.find(a).is_none() {
                self.grow_stack(a);
            }
            let k = self.find(a).ok_or(EFAULT)?.start;
            let r = self.regions.get_mut(&k).unwrap();
            let n = ((r.end() - a) as usize).min(data.len() - off);
            let s = (a - r.start) as usize;
            match &r.shared {
                Some((b, o, _)) => b.borrow_mut().as_mut_slice()[o + s..o + s + n].copy_from_slice(&data[off..off + n]),
                None => r.data.as_mut_slice()[s..s + n].copy_from_slice(&data[off..off + n]),
            }
            off += n;
            a = a.checked_add(n as u64).ok_or(EFAULT)?;
        }
        Ok(())
    }

    /// Read a `Vec`.
    pub fn read_vec(&mut self, addr: u64, len: usize) -> Result<Vec<u8>, i64> {
        if len > (1 << 28) {
            return Err(EINVAL);
        }
        let mut v = vec![0u8; len];
        self.read(addr, &mut v)?;
        Ok(v)
    }
    /// Read a u64.
    pub fn read_u64(&mut self, addr: u64) -> Result<u64, i64> {
        let mut b = [0u8; 8];
        self.read(addr, &mut b)?;
        Ok(u64::from_le_bytes(b))
    }
    /// Read a u32.
    pub fn read_u32(&mut self, addr: u64) -> Result<u32, i64> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    /// Write a u64.
    pub fn write_u64(&mut self, addr: u64, v: u64) -> Result<(), i64> {
        self.write(addr, &v.to_le_bytes())
    }
    /// Write a u32.
    pub fn write_u32(&mut self, addr: u64, v: u32) -> Result<(), i64> {
        self.write(addr, &v.to_le_bytes())
    }
    /// Read a NUL-terminated string (max `max` bytes).
    pub fn read_cstr(&mut self, addr: u64, max: usize) -> Result<String, i64> {
        let mut out = Vec::new();
        loop {
            let mut b = [0u8; 1];
            self.read(addr + out.len() as u64, &mut b)?;
            if b[0] == 0 {
                break;
            }
            if out.len() >= max {
                return Err(ENAMETOOLONG);
            }
            out.push(b[0]);
        }
        String::from_utf8(out).map_err(|e| {
            let _ = e;
            EINVAL
        })
    }

    /// `brk(addr)`: returns the resulting break.
    pub fn set_brk(&mut self, want: u64) -> u64 {
        if self.brk_start == 0 || want < self.brk_start || want >= MMAP_TOP - (1 << 30) {
            return self.brk;
        }
        let (old_end, new_end) = (ceil(self.brk), ceil(want));
        if new_end > old_end {
            if !self.is_free(old_end, new_end) || new_end - self.brk_start > (1 << 30) {
                return self.brk;
            }
            if self.map_fixed(old_end, new_end - old_end, PROT_READ | PROT_WRITE, &[], "[heap]").is_err() {
                return self.brk;
            }
        } else if new_end < old_end {
            self.unmap(new_end, old_end - new_end).ok();
        }
        self.brk = want;
        self.brk
    }

    /// Calls `f(virtual address, backing page address, prot)` for every mapped page.
    pub fn for_each_page(&self, mut f: impl FnMut(u64, u64, u32)) {
        for r in self.regions.values() {
            for i in 0..(r.size() / PAGE) as usize {
                let pa = match &r.shared {
                    Some((b, o, _)) => b.borrow().page_addr(o / 4096 + i),
                    None => r.data.page_addr(i),
                };
                f(r.start + i as u64 * PAGE, pa, r.prot);
            }
        }
    }

    /// Resolve a user page fault at `addr`: grows the stack when the address is just below it.
    /// Returns true if the access should be retried.
    pub fn handle_fault(&mut self, addr: u64) -> bool {
        if self.find(addr).is_some() {
            return false; // mapped: a protection fault, not ours to fix
        }
        self.grow_stack(addr);
        self.find(addr).is_some()
    }

    /// `/proc/self/maps` text.
    pub fn maps(&self) -> String {
        let mut s = String::new();
        for r in self.regions.values() {
            let p = |b, c| if r.prot & b != 0 { c } else { '-' };
            s.push_str(&format!("{:08x}-{:08x} {}{}{}p 00000000 00:00 0 {}\n", r.start, r.end(), p(PROT_READ, 'r'), p(PROT_WRITE, 'w'), p(PROT_EXEC, 'x'), r.name));
        }
        s
    }
}

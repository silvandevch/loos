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

/// A present page; one whose `Rc` is shared with another address space is copy-on-write.
type Slot = Rc<Box<Page>>;

static ZERO: Page = Page([0; 4096]);

/// Page-granular, lazily allocated, copy-on-write byte buffer (length is a multiple of the page size).
/// Sparse: only touched pages are stored, so a huge `PROT_NONE` reservation costs nothing.
#[derive(Clone, Default)]
pub struct PageBuf {
    pages: BTreeMap<usize, Slot>,
    n: usize,
}

impl core::fmt::Debug for PageBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PageBuf({} pages, {} resident)", self.n, self.pages.len())
    }
}

impl PageBuf {
    /// Zeroed buffer of `len` bytes (rounded up to pages); nothing is allocated yet.
    pub fn zeroed(len: u64) -> Self {
        Self { pages: BTreeMap::new(), n: (len as usize).div_ceil(4096) }
    }
    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.n * 4096
    }
    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// Bytes of page `i` (the shared zero page when untouched).
    pub fn page(&self, i: usize) -> &[u8; 4096] {
        match self.pages.get(&i) {
            Some(p) => &p.0,
            None => &ZERO.0,
        }
    }
    /// Writable page `i`: allocates a zero page or breaks copy-on-write.
    /// The flag is true when the page's physical address changed (its PTE must be refreshed).
    pub fn page_mut(&mut self, i: usize) -> (&mut [u8; 4096], bool) {
        assert!(i < self.n, "page {i} beyond {}", self.n);
        let mut moved = false;
        let slot = self.pages.entry(i).or_insert_with(|| {
            moved = true;
            Rc::new(Box::new(Page([0; 4096])))
        });
        moved |= Rc::strong_count(slot) > 1;
        (&mut Rc::make_mut(slot).0, moved)
    }
    /// Make every present page exclusively owned (shared-memory buffers never copy-on-write).
    pub fn unshare(&mut self) {
        for p in self.pages.values_mut() {
            Rc::make_mut(p);
        }
    }
    /// Physical address of page `i` and whether it may be mapped writable (exclusively owned);
    /// `None` for an untouched page (left unmapped, filled on first fault).
    pub fn page_addr(&self, i: usize) -> Option<(u64, bool)> {
        self.pages.get(&i).map(Self::addr)
    }
    fn addr(p: &Slot) -> (u64, bool) {
        (&***p as *const Page as u64, Rc::strong_count(p) == 1)
    }
    /// Present pages with index in `[lo, hi)`: (index, physical address, exclusively owned).
    pub fn present(&self, lo: usize, hi: usize) -> impl Iterator<Item = (usize, u64, bool)> + '_ {
        self.pages.range(lo..hi.max(lo)).map(|(&i, p)| {
            let (a, own) = Self::addr(p);
            (i, a, own)
        })
    }
    /// Drop the pages with index in `[lo, hi)`: they read as zero again (MADV_DONTNEED).
    pub fn discard(&mut self, lo: usize, hi: usize) {
        let gone: Vec<usize> = self.pages.range(lo..hi.max(lo)).map(|(&i, _)| i).collect();
        for i in gone {
            self.pages.remove(&i);
        }
    }
    /// Extend with demand-zero pages to at least `len` bytes (existing pages never move).
    pub fn grow_to(&mut self, len: usize) {
        self.n = self.n.max(len.div_ceil(4096));
    }
    /// Copy `out.len()` bytes starting at `off` (zero beyond the end).
    pub fn read_at(&self, off: usize, out: &mut [u8]) {
        let mut done = 0;
        while done < out.len() {
            let (i, po) = ((off + done) / 4096, (off + done) % 4096);
            let n = (4096 - po).min(out.len() - done);
            out[done..done + n].copy_from_slice(&self.page(i)[po..po + n]);
            done += n;
        }
    }
    /// Write `data` at `off`, growing as needed.
    pub fn write_at(&mut self, off: usize, data: &[u8]) {
        self.grow_to(off + data.len());
        let mut done = 0;
        while done < data.len() {
            let (i, po) = ((off + done) / 4096, (off + done) % 4096);
            let n = (4096 - po).min(data.len() - done);
            if data[done..done + n].iter().any(|&b| b != 0) || self.pages.contains_key(&i) {
                self.page_mut(i).0[po..po + n].copy_from_slice(&data[done..done + n]);
            }
            done += n;
        }
    }
    /// Pages actually backed by memory.
    pub fn resident(&self) -> usize {
        self.pages.len()
    }
    /// Split at `byte` (page aligned), returning the tail.
    pub fn split_off(&mut self, byte: usize) -> PageBuf {
        let at = byte / 4096;
        let tail = self.pages.split_off(&at).into_iter().map(|(i, p)| (i - at, p)).collect();
        let t = PageBuf { pages: tail, n: self.n.saturating_sub(at) };
        self.n = self.n.min(at);
        t
    }
    /// Prepend `bytes` of zero pages (stack growth).
    pub fn prepend_zero(&mut self, bytes: usize) {
        let k = bytes / 4096;
        self.pages = core::mem::take(&mut self.pages).into_iter().map(|(i, p)| (i + k, p)).collect();
        self.n += k;
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
/// Per-process cap on mapped (virtual) memory; physical pages are only allocated on first touch.
pub const MEM_CAP: u64 = 1 << 46;

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
    /// Shared only for a `vfork` child's lifetime; private again afterwards ([`UserMem::end_vfork`]).
    pub vfork: bool,
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
    /// Calls `f(page index, physical address, exclusively owned)` for resident pages in `[lo, hi)`.
    fn each_present(&self, lo: usize, hi: usize, mut f: impl FnMut(usize, u64, bool)) {
        match &self.shared {
            Some((b, o, _)) => {
                let o = *o / 4096;
                b.borrow().present(o + lo, o + hi).for_each(|(i, a, own)| f(i - o, a, own))
            }
            None => self.data.present(lo, hi).for_each(|(i, a, own)| f(i, a, own)),
        }
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
    /// Pages whose physical backing changed outside a fault (kernel writes allocating or
    /// breaking copy-on-write); the page-table owner refreshes them via [`UserMem::take_remap`].
    remap: Vec<u64>,
}

impl Default for UserMem {
    fn default() -> Self {
        Self::new()
    }
}

impl UserMem {
    /// Empty address space.
    pub fn new() -> Self {
        Self { regions: BTreeMap::new(), brk_start: 0, brk: 0, mmap_next: MMAP_TOP, stack: None, total: 0, version: NEXT_EPOCH.fetch_add(1 << 20, Ordering::Relaxed), remap: Vec::new() }
    }

    /// All regions in address order.
    pub fn regions(&self) -> impl Iterator<Item = &Region> {
        self.regions.values()
    }

    /// Bytes currently mapped.
    pub fn mapped(&self) -> u64 {
        self.total
    }

    /// Pages backed by memory (private pages; shared buffers counted once per mapping).
    pub fn resident_pages(&self) -> usize {
        self.regions.values().map(|r| match &r.shared {
            Some((b, o, n)) => b.borrow().present(o / 4096, (o + n) / 4096).count(),
            None => r.data.resident(),
        }).sum()
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
            let nr = Region { start: addr, prot: r.prot, data: tail, name: r.name.clone(), shared, vfork: r.vfork };
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
        for (i, c) in init[..init.len().min(data.len())].chunks(4096).enumerate() {
            if c.iter().any(|&b| b != 0) {
                data.page_mut(i).0[..c.len()].copy_from_slice(c);
            }
        }
        // page tables: updated per page through `remap` (new pages are demand-zero; init pages are queued below)
        self.regions.insert(start, Region { start, prot, data, name: name.to_string(), shared: None, vfork: false });
        self.total += len;
        self.queue_resident(start, start + len);
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
                let mut d = core::mem::take(&mut r.data);
                d.unshare();
                let n = d.len();
                r.shared = Some((Rc::new(RefCell::new(d)), 0, n));
            }
        }
    }

    /// Make every mapping shared and return a second address space aliasing the same bytes
    /// (`CLONE_VM` without `CLONE_THREAD`: vfork / posix_spawn). Later (un)mappings are private.
    pub fn share_all(&mut self) -> UserMem {
        let all: Vec<(u64, u64)> = self.regions.values().filter(|r| r.shared.is_none()).map(|r| (r.start, r.size())).collect();
        for (s, l) in all {
            self.make_shared(s, l);
            for (_, r) in self.regions.range_mut(s..s + l) {
                r.vfork = true;
            }
        }
        self.stack = None;
        let mut c = self.clone();
        c.total = self.total;
        c
    }

    /// The `vfork` child exec'd or exited: regions shared only for it become private again (once
    /// the child no longer holds them), so later forks get copy-on-write snapshots, not live memory.
    pub fn end_vfork(&mut self) {
        let mut changed = false;
        for r in self.regions.values_mut().filter(|r| r.vfork) {
            let Some((buf, off, len)) = r.shared.take() else { continue };
            match Rc::try_unwrap(buf) {
                Ok(cell) if off == 0 && cell.borrow().len() == len => r.data = cell.into_inner(),
                Ok(cell) => r.shared = Some((Rc::new(cell), off, len)),
                Err(buf) => {
                    r.shared = Some((buf, off, len)); // the child still runs on it
                    continue;
                }
            }
            r.vfork = r.shared.is_some();
            changed = true;
        }
        if changed {
            self.version += 1;
        }
    }

    /// Address space for a `fork` child: private pages become copy-on-write in both spaces
    /// (the parent's page tables are rebuilt read-only), shared mappings stay shared.
    pub fn fork(&mut self) -> UserMem {
        self.version += 1;
        let mut c = self.clone();
        c.version = NEXT_EPOCH.fetch_add(1 << 20, Ordering::Relaxed);
        c.remap.clear();
        c
    }

    /// Queue every resident page of the regions overlapping [start, end) for a page-table refresh.
    fn queue_resident(&mut self, start: u64, end: u64) {
        let mut q = Vec::new();
        for r in self.regions.values().filter(|r| r.start < end && r.end() > start) {
            let lo = (start.max(r.start) - r.start) / PAGE;
            let hi = (end.min(r.end()) - r.start) / PAGE;
            r.each_present(lo as usize, hi as usize, |i, _, _| q.push(r.start + i as u64 * PAGE));
        }
        self.remap.extend(q);
    }

    /// `madvise(MADV_DONTNEED)`: private pages in [start, start+len) become demand-zero again
    /// (shared mappings keep their contents, as on Linux).
    pub fn discard(&mut self, start: u64, len: u64) {
        let end = start.saturating_add(ceil(len));
        let mut q = Vec::new();
        for r in self.regions.values_mut().filter(|r| r.start < end && r.end() > start && r.shared.is_none()) {
            let lo = ((start.max(r.start) - r.start) / PAGE) as usize;
            let hi = ((end.min(r.end()) - r.start) / PAGE) as usize;
            r.each_present(lo, hi, |i, _, _| q.push(r.start + i as u64 * PAGE));
            r.data.discard(lo, hi);
        }
        self.remap.extend(q);
    }

    /// Pages to refresh in the page tables (see `remap`).
    pub fn take_remap(&mut self) -> Vec<u64> {
        core::mem::take(&mut self.remap)
    }

    /// Current page-table entry for `va`: (physical address, effective prot), `None` if unmapped/untouched.
    pub fn pte(&self, va: u64) -> Option<(u64, u32)> {
        let r = self.find(va)?;
        let i = ((floor(va) - r.start) / PAGE) as usize;
        let (pa, own) = match &r.shared {
            Some((b, o, _)) => b.borrow().page_addr(o / 4096 + i)?,
            None => r.data.page_addr(i)?,
        };
        Some((pa, if own { r.prot } else { r.prot & !PROT_WRITE }))
    }

    /// A free gap of `len` bytes below the mmap pointer.
    pub fn gap(&self, len: u64) -> Result<u64, i64> {
        if len == 0 {
            return Err(EINVAL);
        }
        let len = ceil(len);
        let mut top = self.mmap_next;
        loop {
            let start = top.checked_sub(len).filter(|&s| s >= 0x1_0000).ok_or(ENOMEM)?;
            match self.regions.values().find(|r| r.start < top && r.end() > start) {
                Some(r) => top = r.start,
                None => return Ok(start),
            }
        }
    }

    /// Find a free gap below the mmap pointer and map it.
    pub fn map_anywhere(&mut self, len: u64, prot: u32, init: &[u8], name: &str) -> Result<u64, i64> {
        let start = self.gap(len)?;
        self.map_fixed(start, len, prot, init, name)?;
        self.mmap_next = start;
        Ok(start)
    }

    /// Map [start, start+len) onto bytes `off..` of a buffer other address spaces (or a file's
    /// page cache) also map: writes are visible through every mapping. `start == None` picks a gap.
    pub fn map_shared_buf(&mut self, start: Option<u64>, len: u64, prot: u32, buf: SharedBuf, off: usize, name: &str) -> Result<u64, i64> {
        let len = ceil(len);
        let start = match start {
            Some(s) => s,
            None => {
                let s = self.gap(len)?;
                self.mmap_next = s;
                s
            }
        };
        if start % PAGE != 0 || off % 4096 != 0 || len == 0 {
            return Err(EINVAL);
        }
        let end = start.checked_add(len).filter(|&e| e <= USER_TOP).ok_or(ENOMEM)?;
        let freed: u64 = self.regions.values().filter(|r| r.start < end && r.end() > start).map(|r| r.end().min(end) - r.start.max(start)).sum();
        if self.total - freed + len > MEM_CAP {
            return Err(ENOMEM);
        }
        self.unmap(start, len)?;
        {
            let mut b = buf.borrow_mut();
            b.grow_to(off + len as usize);
            b.unshare();
        }
        self.version += 1;
        self.regions.insert(start, Region { start, prot, data: PageBuf::default(), name: name.to_string(), shared: Some((buf, off, len as usize)), vfork: false });
        self.total += len;
        Ok(start)
    }

    /// Unmap [start, start+len) (partial regions are split). Unmapped holes are fine.
    pub fn unmap(&mut self, start: u64, len: u64) -> Result<(), i64> {
        if start % PAGE != 0 {
            return Err(EINVAL);
        }
        let end = start.checked_add(ceil(len)).ok_or(EINVAL)?;
        self.split_at(start);
        self.split_at(end);
        // page tables: updated per page through `remap` (the removed pages are queued below)
        self.queue_resident(start, end);
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
        // page tables: updated per page through `remap` (the affected pages are queued below)
        for (_, r) in self.regions.range_mut(start..end) {
            r.prot = prot;
        }
        self.queue_resident(start, end);
        Ok(())
    }

    /// For a shared mapping containing `addr`: its buffer, the buffer offset of `addr`, and the region name.
    pub fn shared_at(&self, addr: u64) -> Option<(SharedBuf, usize, String)> {
        let r = self.find(addr)?;
        let (b, o, _) = r.shared.as_ref()?;
        Some((b.clone(), o + (floor(addr) - r.start) as usize, r.name.clone()))
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
        // page tables: updated per page through `remap` (new stack pages are demand-zero)
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
            if r.prot & PROT_READ == 0 {
                return Err(EFAULT); // as copy_from_user: PROT_NONE pages are not readable
            }
            let s = (a - r.start) as usize;
            let n = (4096 - s % 4096).min(buf.len() - off);
            let (i, po) = (s / 4096, s % 4096);
            match &r.shared {
                Some((b, o, _)) => buf[off..off + n].copy_from_slice(&b.borrow().page(o / 4096 + i)[po..po + n]),
                None => buf[off..off + n].copy_from_slice(&r.data.page(i)[po..po + n]),
            }
            off += n;
            a = a.checked_add(n as u64).ok_or(EFAULT)?;
        }
        Ok(())
    }

    /// Copy bytes into user memory on behalf of a syscall: like Linux's copy_to_user, a page the
    /// process cannot write is `EFAULT` (programs probe protections this way).
    pub fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), i64> {
        let mut a = addr;
        let mut off = 0;
        while off < data.len() {
            if self.find(a).is_none() {
                self.grow_stack(a);
            }
            let found = self.find(a).ok_or(EFAULT)?;
            if found.prot & PROT_WRITE == 0 {
                return Err(EFAULT);
            }
            let k = found.start;
            let r = self.regions.get_mut(&k).unwrap();
            let s = (a - r.start) as usize;
            let n = (4096 - s % 4096).min(data.len() - off);
            let (i, po) = (s / 4096, s % 4096);
            let moved = match &r.shared {
                Some((b, o, _)) => {
                    let mut b = b.borrow_mut();
                    let (pg, m) = b.page_mut(o / 4096 + i);
                    pg[po..po + n].copy_from_slice(&data[off..off + n]);
                    m
                }
                None => {
                    let (pg, m) = r.data.page_mut(i);
                    pg[po..po + n].copy_from_slice(&data[off..off + n]);
                    m
                }
            };
            if moved {
                self.remap.push(floor(a));
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

    /// Calls `f(virtual address, backing page address, prot)` for every resident page;
    /// copy-on-write pages are reported without `PROT_WRITE`.
    pub fn for_each_page(&self, mut f: impl FnMut(u64, u64, u32)) {
        for r in self.regions.values() {
            r.each_present(0, (r.size() / PAGE) as usize, |i, pa, own| {
                f(r.start + i as u64 * PAGE, pa, if own { r.prot } else { r.prot & !PROT_WRITE })
            });
        }
    }

    /// Resolve a user page fault at `addr` (`write` from the error code): grows the stack,
    /// fills demand-zero pages and breaks copy-on-write. Returns the page to (re)map as
    /// `(va, pa, prot)`, or `None` when the access is a genuine fault.
    pub fn handle_fault(&mut self, addr: u64, write: bool) -> Option<(u64, u64, u32)> {
        if self.find(addr).is_none() {
            self.grow_stack(addr);
        }
        let r = self.find(addr)?;
        let need = if write { PROT_WRITE } else { PROT_READ | PROT_EXEC };
        if r.prot & need == 0 {
            return None;
        }
        let k = r.start;
        let va = floor(addr);
        let i = ((va - k) / PAGE) as usize;
        let r = self.regions.get_mut(&k).unwrap();
        match &r.shared {
            Some((b, o, _)) => {
                let mut b = b.borrow_mut();
                if write || b.page_addr(o / 4096 + i).is_none() {
                    b.page_mut(o / 4096 + i);
                }
            }
            None => {
                if write || r.data.page_addr(i).is_none() {
                    r.data.page_mut(i);
                }
            }
        }
        self.pte(va).map(|(pa, prot)| (va, pa, prot))
    }

    /// Mapping containing `addr`: (its name, offset of `addr` from the mapping start) — for crash reports.
    pub fn locate(&self, addr: u64) -> Option<(&str, u64)> {
        let r = self.find(addr)?;
        // A library is several mappings with one name (with gaps between segments): measure from its first one.
        let start = self.regions.range(..=r.start).map(|(_, q)| q).find(|q| q.name == r.name).map_or(r.start, |q| q.start);
        Some((r.name.as_str(), addr - start))
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

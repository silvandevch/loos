//! Virtual address descriptors: the per-process map of reserved / committed regions.
//!
//! Mirrors the NT `VirtualAlloc` model: an *allocation* is reserved as a unit at 64 KiB granularity,
//! its pages can then be committed, decommitted and re-protected individually (regions split and
//! re-merge automatically), and the whole allocation is released by its base address.

use crate::{Error, PAGE_SIZE};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Allocation granularity of reservations.
pub const ALLOC_GRANULARITY: u64 = 64 * 1024;
const PAGE: u64 = PAGE_SIZE as u64;

/// Page protection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protect {
    /// No access.
    None,
    /// Read only.
    Read,
    /// Read and write.
    ReadWrite,
    /// Read and execute.
    ReadExecute,
    /// Read, write and execute – refused when the space enforces W^X.
    ReadWriteExecute,
}

/// What backs a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backing {
    /// Zero-filled private memory.
    Private,
    /// A file/section mapping at byte `offset` of section `id` (demand paged).
    Section {
        /// Section object identifier.
        id: u64,
        /// Offset of the region's first byte inside the section.
        offset: u64,
    },
}

/// Region state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Address space only.
    Reserved,
    /// Backed by memory.
    Committed,
}

/// One contiguous region with uniform attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vad {
    /// First byte (page aligned).
    pub start: u64,
    /// One past the last byte.
    pub end: u64,
    /// Base of the owning allocation.
    pub alloc_base: u64,
    /// Reserved or committed.
    pub state: State,
    /// Current protection (meaningful when committed).
    pub protect: Protect,
    /// Protection given at reservation time (upper bound for later commits).
    pub max_protect: Protect,
    /// Backing store.
    pub backing: Backing,
}

/// A process address space.
pub struct AddressSpace {
    lo: u64,
    hi: u64,
    wx_enforced: bool,
    map: BTreeMap<u64, Vad>,
}

fn aligned(v: u64, a: u64) -> bool {
    v % a == 0
}

fn round_up(v: u64, a: u64) -> Option<u64> {
    v.checked_add(a - 1).map(|x| x / a * a)
}

fn rank(p: Protect) -> u8 {
    match p {
        Protect::None => 0,
        Protect::Read => 1,
        Protect::ReadWrite | Protect::ReadExecute => 2,
        Protect::ReadWriteExecute => 3,
    }
}

impl AddressSpace {
    /// User range `[lo, hi)`; with `wx_enforced` RWX protections are rejected.
    pub fn new(lo: u64, hi: u64, wx_enforced: bool) -> Self {
        Self { lo, hi, wx_enforced, map: BTreeMap::new() }
    }

    /// Reserves `size` bytes. With `at = Some(base)` the range must be free and granularity-aligned;
    /// otherwise the lowest (or highest, with `top_down`) fitting gap is used.
    pub fn reserve(&mut self, at: Option<u64>, size: u64, max_protect: Protect, backing: Backing, top_down: bool) -> Result<u64, Error> {
        if size == 0 || (self.wx_enforced && max_protect == Protect::ReadWriteExecute) {
            return Err(Error::BadSize);
        }
        let size = round_up(size, PAGE).ok_or(Error::BadSize)?;
        let base = match at {
            Some(b) => {
                if !aligned(b, ALLOC_GRANULARITY) || b < self.lo || b.checked_add(size).map_or(true, |e| e > self.hi) || self.overlaps(b, b + size) {
                    return Err(Error::BadAddress);
                }
                b
            }
            None => self.find_gap(size, top_down).ok_or(Error::OutOfMemory)?,
        };
        let v = Vad { start: base, end: base + size, alloc_base: base, state: State::Reserved, protect: Protect::None, max_protect, backing };
        self.map.insert(base, v);
        Ok(base)
    }

    fn overlaps(&self, s: u64, e: u64) -> bool {
        self.map.range(..e).next_back().map_or(false, |(_, v)| v.end > s) && self.map.range(..e).any(|(_, v)| v.end > s && v.start < e)
    }

    fn find_gap(&self, size: u64, top_down: bool) -> Option<u64> {
        let mut gaps = Vec::new();
        let mut cur = round_up(self.lo, ALLOC_GRANULARITY)?;
        for v in self.map.values() {
            if v.start > cur {
                gaps.push((cur, v.start));
            }
            cur = cur.max(round_up(v.end, ALLOC_GRANULARITY)?);
        }
        if self.hi > cur {
            gaps.push((cur, self.hi));
        }
        if top_down {
            gaps.iter().rev().find_map(|&(s, e)| {
                let b = (e.checked_sub(size)?) / ALLOC_GRANULARITY * ALLOC_GRANULARITY;
                (b >= s).then_some(b)
            })
        } else {
            gaps.iter().find_map(|&(s, e)| (e - s >= size).then_some(s))
        }
    }

    /// Splits so that `addr` is a region boundary.
    fn split_at(&mut self, addr: u64) {
        let (k, v) = match self.map.range(..=addr).next_back() {
            Some((&k, &v)) => (k, v),
            None => return,
        };
        if v.start < addr && addr < v.end {
            let mut lo = v;
            let mut hi = v;
            lo.end = addr;
            hi.start = addr;
            if let Backing::Section { id, offset } = v.backing {
                hi.backing = Backing::Section { id, offset: offset + (addr - v.start) };
            }
            self.map.insert(k, lo);
            self.map.insert(addr, hi);
        }
    }

    fn merge_around(&mut self, s: u64, e: u64) {
        let keys: Vec<u64> = self.map.range(..=e).rev().take_while(|(_, v)| v.end >= s).map(|(k, _)| *k).collect();
        for k in keys.into_iter().rev() {
            let a = match self.map.get(&k) {
                Some(&a) => a,
                None => continue,
            };
            let mut a = a;
            while let Some(&b) = self.map.get(&a.end) {
                let same = a.alloc_base == b.alloc_base && a.state == b.state && a.protect == b.protect && a.max_protect == b.max_protect;
                let contiguous = match (a.backing, b.backing) {
                    (Backing::Private, Backing::Private) => true,
                    (Backing::Section { id: i, offset: o }, Backing::Section { id: j, offset: p }) => i == j && p == o + (a.end - a.start),
                    _ => false,
                };
                if !(same && contiguous) {
                    break;
                }
                self.map.remove(&b.start);
                a.end = b.end;
                if let Some(x) = self.map.get_mut(&k) {
                    x.end = b.end;
                }
            }
        }
    }

    /// Checks `[addr, addr+size)` lies inside one allocation and returns the page-rounded range.
    fn range_in_alloc(&self, addr: u64, size: u64) -> Result<(u64, u64, u64), Error> {
        if size == 0 || !aligned(addr, PAGE) {
            return Err(Error::BadAddress);
        }
        let end = round_up(addr.checked_add(size).ok_or(Error::BadAddress)?, PAGE).ok_or(Error::BadAddress)?;
        let first = self.query(addr).ok_or(Error::BadAddress)?;
        let mut cur = addr;
        while cur < end {
            let v = self.query(cur).ok_or(Error::BadAddress)?;
            if v.alloc_base != first.alloc_base {
                return Err(Error::BadAddress);
            }
            cur = v.end;
        }
        Ok((addr, end, first.alloc_base))
    }

    /// Commits pages inside a reservation with protection `prot` (no-op for pages already committed).
    pub fn commit(&mut self, addr: u64, size: u64, prot: Protect) -> Result<(), Error> {
        let (s, e, _) = self.range_in_alloc(addr, size)?;
        if (self.wx_enforced && prot == Protect::ReadWriteExecute) || rank(prot) > rank(self.query(s).ok_or(Error::BadAddress)?.max_protect) {
            return Err(Error::BadSize);
        }
        self.split_at(s);
        self.split_at(e);
        for (_, v) in self.map.range_mut(s..e) {
            if v.state == State::Reserved {
                v.state = State::Committed;
                v.protect = prot;
            }
        }
        self.merge_around(s, e);
        Ok(())
    }

    /// Returns committed pages to the reserved state.
    pub fn decommit(&mut self, addr: u64, size: u64) -> Result<(), Error> {
        let (s, e, _) = self.range_in_alloc(addr, size)?;
        self.split_at(s);
        self.split_at(e);
        for (_, v) in self.map.range_mut(s..e) {
            v.state = State::Reserved;
            v.protect = Protect::None;
        }
        self.merge_around(s, e);
        Ok(())
    }

    /// Changes protection of committed pages; returns the previous protection of the first page.
    pub fn protect(&mut self, addr: u64, size: u64, prot: Protect) -> Result<Protect, Error> {
        let (s, e, _) = self.range_in_alloc(addr, size)?;
        if (self.wx_enforced && prot == Protect::ReadWriteExecute) || rank(prot) > rank(self.query(s).ok_or(Error::BadAddress)?.max_protect) {
            return Err(Error::BadSize);
        }
        if self.map.range(..e).any(|(_, v)| v.end > s && v.state != State::Committed) {
            return Err(Error::BadAddress);
        }
        let old = self.query(s).ok_or(Error::BadAddress)?.protect;
        self.split_at(s);
        self.split_at(e);
        for (_, v) in self.map.range_mut(s..e) {
            v.protect = prot;
        }
        self.merge_around(s, e);
        Ok(old)
    }

    /// Releases the whole allocation whose base address is `base`.
    pub fn release(&mut self, base: u64) -> Result<(), Error> {
        let v = self.map.get(&base).ok_or(Error::BadAddress)?;
        if v.alloc_base != base {
            return Err(Error::BadAddress);
        }
        let keys: Vec<u64> = self.map.iter().filter(|(_, v)| v.alloc_base == base).map(|(k, _)| *k).collect();
        for k in keys {
            self.map.remove(&k);
        }
        Ok(())
    }

    /// Region containing `addr`.
    pub fn query(&self, addr: u64) -> Option<Vad> {
        self.map.range(..=addr).next_back().map(|(_, v)| *v).filter(|v| addr < v.end)
    }

    /// Whether `[addr, addr+len)` is committed and permits the access (the user-pointer check).
    pub fn check_access(&self, addr: u64, len: u64, write: bool, exec: bool) -> bool {
        let end = match addr.checked_add(len) {
            Some(end) => end,
            None => return false,
        };
        if len == 0 {
            return true;
        }
        let mut cur = addr;
        while cur < end {
            let v = match self.query(cur) {
                Some(v) => v,
                None => return false,
            };
            let ok = v.state == State::Committed
                && match v.protect {
                    Protect::None => false,
                    Protect::Read => !write && !exec,
                    Protect::ReadWrite => !exec,
                    Protect::ReadExecute => !write,
                    Protect::ReadWriteExecute => true,
                };
            if !ok {
                return false;
            }
            cur = v.end;
        }
        true
    }

    /// Number of regions (diagnostics/tests).
    pub fn region_count(&self) -> usize {
        self.map.len()
    }

    /// Total committed bytes.
    pub fn committed_bytes(&self) -> u64 {
        self.map.values().filter(|v| v.state == State::Committed).map(|v| v.end - v.start).sum()
    }
}

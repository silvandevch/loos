//! Size-class slab heap.
//!
//! Small requests (≤ 2 KiB) come from one-page slabs carved into equal objects; larger requests take
//! whole page runs straight from the [`PageSource`]. All metadata lives off-page.

use crate::{Error, PAGE_SIZE};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Supplies and reclaims page runs (physical or virtual addresses, page aligned).
pub trait PageSource {
    /// Allocates `pages` contiguous pages; returns the base address.
    fn alloc_pages(&mut self, pages: usize) -> Result<usize, Error>;
    /// Returns a run obtained from `alloc_pages` with the same `pages`.
    fn free_pages(&mut self, addr: usize, pages: usize);
}

const CLASSES: [usize; 8] = [16, 32, 64, 128, 256, 512, 1024, 2048];

struct Slab {
    class: usize,
    free: Vec<u16>,        // free object indices
    used: u16,
}

/// The kernel heap.
pub struct SlabHeap<P: PageSource> {
    pages: P,
    slabs: BTreeMap<usize, Slab>,              // page base -> slab
    partial: [Vec<usize>; CLASSES.len()],      // slabs with free objects, per class
    large: BTreeMap<usize, usize>,             // base -> page count
    bytes_in_use: usize,
}

fn class_of(size: usize) -> Option<usize> {
    CLASSES.iter().position(|&c| size <= c)
}

impl<P: PageSource> SlabHeap<P> {
    /// Creates a heap drawing pages from `pages`.
    pub fn new(pages: P) -> Self {
        Self { pages, slabs: BTreeMap::new(), partial: Default::default(), large: BTreeMap::new(), bytes_in_use: 0 }
    }

    /// Allocates `size` bytes (alignment: the size class, or a page for large blocks).
    pub fn alloc(&mut self, size: usize) -> Result<usize, Error> {
        if size == 0 {
            return Err(Error::BadSize);
        }
        let Some(c) = class_of(size) else {
            let pages = size.div_ceil(PAGE_SIZE);
            let a = self.pages.alloc_pages(pages)?;
            self.large.insert(a, pages);
            self.bytes_in_use += pages * PAGE_SIZE;
            return Ok(a);
        };
        let base = match self.partial[c].last() {
            Some(&b) => b,
            None => {
                let b = self.pages.alloc_pages(1)?;
                let n = (PAGE_SIZE / CLASSES[c]) as u16;
                self.slabs.insert(b, Slab { class: c, free: (0..n).rev().collect(), used: 0 });
                self.partial[c].push(b);
                b
            }
        };
        let slab = self.slabs.get_mut(&base).ok_or(Error::BadAddress)?;
        let idx = slab.free.pop().ok_or(Error::OutOfMemory)?;
        slab.used += 1;
        if slab.free.is_empty() {
            self.partial[c].retain(|&b| b != base);
        }
        self.bytes_in_use += CLASSES[c];
        Ok(base + usize::from(idx) * CLASSES[c])
    }

    /// Frees an address returned by [`Self::alloc`].
    pub fn free(&mut self, addr: usize) -> Result<(), Error> {
        if let Some(pages) = self.large.remove(&addr) {
            self.pages.free_pages(addr, pages);
            self.bytes_in_use -= pages * PAGE_SIZE;
            return Ok(());
        }
        let base = addr & !(PAGE_SIZE - 1);
        let slab = self.slabs.get_mut(&base).ok_or(Error::BadAddress)?;
        let size = CLASSES[slab.class];
        let off = addr - base;
        if off % size != 0 || off / size >= PAGE_SIZE / size {
            return Err(Error::BadAddress);
        }
        let idx = (off / size) as u16;
        if slab.free.contains(&idx) {
            return Err(Error::DoubleFree);
        }
        let was_full = slab.free.is_empty();
        slab.free.push(idx);
        slab.used -= 1;
        let (class, empty) = (slab.class, slab.used == 0);
        self.bytes_in_use -= size;
        if empty {
            self.partial[class].retain(|&b| b != base);
            self.slabs.remove(&base);
            self.pages.free_pages(base, 1);
        } else if was_full {
            self.partial[class].push(base);
        }
        Ok(())
    }

    /// Usable size of a live allocation.
    pub fn usable_size(&self, addr: usize) -> Option<usize> {
        if let Some(p) = self.large.get(&addr) {
            return Some(p * PAGE_SIZE);
        }
        self.slabs.get(&(addr & !(PAGE_SIZE - 1))).map(|s| CLASSES[s.class])
    }

    /// Bytes handed out (rounded to class / page size).
    pub fn bytes_in_use(&self) -> usize {
        self.bytes_in_use
    }

    /// Access to the underlying page source.
    pub fn pages(&self) -> &P {
        &self.pages
    }
}


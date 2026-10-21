//! Buddy allocator over a contiguous range of physical frames.

use crate::Error;
use alloc::vec;
use alloc::vec::Vec;

/// Largest block is `2^MAX_ORDER` frames (4 MiB with 4 KiB pages).
pub const MAX_ORDER: usize = 10;

/// One bit per possible block start per order; a set bit means "free block of this order starts here".
pub struct BuddyAllocator {
    base: u64,
    frames: u64,
    free_bits: [Vec<u64>; MAX_ORDER + 1],
    /// Order of every allocated block, indexed by its first frame (0xff = not an allocation start).
    alloc_order: Vec<u8>,
    free_frames: u64,
}

const NOT_ALLOC: u8 = 0xff;

impl BuddyAllocator {
    /// Manages frames `base .. base + frames`; everything starts allocated until [`Self::add_free`].
    pub fn new(base: u64, frames: u64) -> Self {
        let words = |o: usize| (((frames >> o) + 64) / 64) as usize;
        Self {
            base,
            frames,
            free_bits: core::array::from_fn(|o| vec![0u64; words(o)]),
            alloc_order: vec![NOT_ALLOC; frames as usize],
            free_frames: 0,
        }
    }

    fn bit(&self, order: usize, idx: u64) -> bool {
        self.free_bits[order][(idx / 64) as usize] >> (idx % 64) & 1 != 0
    }

    fn set(&mut self, order: usize, idx: u64, v: bool) {
        let w = &mut self.free_bits[order][(idx / 64) as usize];
        if v {
            *w |= 1 << (idx % 64);
        } else {
            *w &= !(1 << (idx % 64));
        }
    }

    /// Finds the lowest free block of `order`, if any.
    fn first_free(&self, order: usize) -> Option<u64> {
        self.free_bits[order]
            .iter()
            .enumerate()
            .find(|(_, w)| **w != 0)
            .map(|(i, w)| i as u64 * 64 + u64::from(w.trailing_zeros()))
    }

    /// Releases `count` frames starting at absolute frame `start` into the free pool (boot-time memory map).
    pub fn add_free(&mut self, start: u64, count: u64) -> Result<(), Error> {
        if start < self.base || start + count > self.base + self.frames {
            return Err(Error::BadAddress);
        }
        let (mut rel, mut left) = (start - self.base, count);
        while left > 0 {
            let mut o = MAX_ORDER;
            while o > 0 && (rel % (1 << o) != 0 || (1u64 << o) > left) {
                o -= 1;
            }
            if (rel..rel + (1 << o)).any(|f| self.alloc_order[f as usize] != NOT_ALLOC || self.is_free_anywhere(f)) {
                return Err(Error::DoubleFree);
            }
            self.free_block(rel, o);
            rel += 1 << o;
            left -= 1 << o;
        }
        Ok(())
    }

    fn is_free_anywhere(&self, rel: u64) -> bool {
        (0..=MAX_ORDER).any(|o| self.bit(o, rel >> o))      // the aligned order-`o` block containing `rel` is free
    }

    /// Inserts a block and coalesces with free buddies.
    fn free_block(&mut self, mut rel: u64, mut order: usize) {
        self.free_frames += 1 << order;
        while order < MAX_ORDER {
            let buddy = rel ^ (1 << order);
            if buddy + (1 << order) > self.frames || !self.bit(order, buddy >> order) {
                break;
            }
            self.set(order, buddy >> order, false);
            rel &= !(1 << order);
            order += 1;
        }
        self.set(order, rel >> order, true);
    }

    /// Allocates `2^order` aligned frames; returns the absolute first frame number.
    pub fn alloc(&mut self, order: usize) -> Result<u64, Error> {
        if order > MAX_ORDER {
            return Err(Error::BadSize);
        }
        let from = (order..=MAX_ORDER).find(|&o| self.first_free(o).is_some()).ok_or(Error::OutOfMemory)?;
        let rel = self.first_free(from).ok_or(Error::OutOfMemory)? << from;
        self.set(from, rel >> from, false);
        for o in (order..from).rev() {
            self.set(o, (rel >> o) + 1, true);                       // upper half stays free
        }
        self.free_frames -= 1 << order;
        self.alloc_order[rel as usize] = order as u8;
        Ok(rel + self.base)
    }

    /// Frees a block previously returned by [`Self::alloc`].
    pub fn free(&mut self, frame: u64) -> Result<(), Error> {
        if frame < self.base || frame >= self.base + self.frames {
            return Err(Error::BadAddress);
        }
        let rel = frame - self.base;
        match self.alloc_order[rel as usize] {
            NOT_ALLOC if self.is_free_anywhere(rel) => Err(Error::DoubleFree),
            NOT_ALLOC => Err(Error::BadAddress),
            o => {
                self.alloc_order[rel as usize] = NOT_ALLOC;
                self.free_block(rel, o as usize);
                Ok(())
            }
        }
    }

    /// Order of the live allocation starting at `frame`.
    pub fn order_of(&self, frame: u64) -> Option<usize> {
        let rel = frame.checked_sub(self.base)?;
        match self.alloc_order.get(rel as usize)? {
            &NOT_ALLOC => None,
            &o => Some(o as usize),
        }
    }

    /// Frames currently free.
    pub fn free_frames(&self) -> u64 {
        self.free_frames
    }

    /// Total managed frames.
    pub fn total_frames(&self) -> u64 {
        self.frames
    }
}

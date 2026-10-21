//! Kernel memory management.
//!
//! * [`buddy`] – physical frame allocator (power-of-two blocks, coalescing, double-free detection).
//! * [`vad`] – per-process virtual address descriptors (reserve/commit/protect/release, user-range checks).
//! * [`workingset`] – per-process resident set with clock trimming.
//! * [`slab`] – size-class heap on top of any [`slab::PageSource`] (the buddy allocator in the kernel).
//!
//! Both are address-space models: they hand out frame numbers / addresses and keep all bookkeeping
//! off-page, so they are testable on the host and never touch the memory they manage.
#![no_std]

extern crate alloc;

pub mod buddy;
pub mod slab;
pub mod vad;
pub mod workingset;

/// Page size in bytes.
pub const PAGE_SIZE: usize = 4096;

pub use buddy::{BuddyAllocator, MAX_ORDER};
pub use slab::{PageSource, SlabHeap};

/// Memory-manager failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No block of the requested size is available.
    OutOfMemory,
    /// The frame/address is outside the managed range or not an allocation start.
    BadAddress,
    /// The block is already free.
    DoubleFree,
    /// Requested order or size is not supported.
    BadSize,
}

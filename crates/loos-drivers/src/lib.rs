//! Hardware-agnostic device drivers.
//!
//! Every driver talks to hardware only through the traits in [`hw`], so the whole crate is safe code
//! that is unit-tested on the host against mock devices. The kernel supplies the real port/MMIO/DMA
//! implementations (the only `unsafe`).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod ahci;
pub mod display;
pub mod e1000;
pub mod gpu;
pub mod hw;
pub mod input;
pub mod nvme;
pub mod ps2;
pub mod storage;
pub mod usb;
pub mod xhci;
pub mod virtio_net;

/// Errors common to all drivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// Device did not respond within the polling budget.
    Timeout,
    /// Device reported something the driver does not understand.
    Protocol,
    /// Caller passed an invalid argument (size, geometry, format).
    Invalid,
    /// No descriptor / buffer space available right now.
    Busy,
}

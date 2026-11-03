//! Block-device abstraction shared by the AHCI, NVMe and USB mass-storage drivers.

use crate::hw::{DmaArena, Mmio};
use crate::DriverError;

/// A randomly addressable block device.
pub trait BlockDev {
    /// Logical sector size in bytes.
    fn sector_size(&self) -> u32;
    /// Capacity in sectors.
    fn sectors(&self) -> u64;
    /// Reads `buf.len() / sector_size` sectors starting at `lba`; `buf.len()` must be a sector multiple.
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), DriverError>;
    /// Writes whole sectors starting at `lba`.
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), DriverError>;
}

/// Polling budget for device completion (register reads).
pub(crate) const SPIN: u32 = 200_000;

pub(crate) fn wait(mut f: impl FnMut() -> bool) -> Result<(), DriverError> {
    for _ in 0..SPIN {
        if f() {
            return Ok(());
        }
    }
    Err(DriverError::Timeout)
}

pub(crate) fn w64<M: Mmio>(m: &mut M, off: usize, v: u64) {
    m.write32(off, v as u32);
    m.write32(off + 4, (v >> 32) as u32);
}

pub(crate) fn dma_u32<D: DmaArena>(d: &D, off: usize) -> u32 {
    let mut b = [0u8; 4];
    d.read(off, &mut b);
    u32::from_le_bytes(b)
}

pub(crate) trait DmaExt: DmaArena {
    fn w32(&mut self, off: usize, v: u32) {
        self.write(off, &v.to_le_bytes());
    }
    fn w64(&mut self, off: usize, v: u64) {
        self.write(off, &v.to_le_bytes());
    }
}
impl<T: DmaArena> DmaExt for T {}

pub(crate) fn check_chunk(len: usize, ss: u32) -> Result<usize, DriverError> {
    if ss == 0 || len == 0 || len % ss as usize != 0 {
        return Err(DriverError::Invalid);
    }
    Ok(len / ss as usize)
}

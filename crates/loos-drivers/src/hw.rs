//! Hardware access traits implemented by the kernel (and by mocks in tests).

/// x86 port I/O.
pub trait PortIo {
    /// Read a byte from `port`.
    fn inb(&mut self, port: u16) -> u8;
    /// Write a byte to `port`.
    fn outb(&mut self, port: u16, val: u8);
}

/// Memory-mapped 32-bit register window.
///
/// Widths are separate methods because some devices (virtio common config) have side effects on
/// neighbouring registers when accessed with the wrong width.
pub trait Mmio {
    /// Read the 32-bit register at byte offset `off`.
    fn read32(&mut self, off: usize) -> u32;
    /// Write the 32-bit register at byte offset `off`.
    fn write32(&mut self, off: usize, val: u32);
    /// Read a 16-bit register.
    fn read16(&mut self, off: usize) -> u16;
    /// Write a 16-bit register.
    fn write16(&mut self, off: usize, val: u16);
    /// Read an 8-bit register.
    fn read8(&mut self, off: usize) -> u8;
    /// Write an 8-bit register.
    fn write8(&mut self, off: usize, val: u8);
}

/// Device-visible (DMA) memory the driver owns; addressed by an index-free byte offset into one arena.
/// `phys(off)` is the bus address the device must be given for byte `off` of the arena.
pub trait DmaArena {
    /// Arena length in bytes.
    fn len(&self) -> usize;
    /// True when the arena is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Bus address of byte `off`.
    fn phys(&self, off: usize) -> u64;
    /// Read `buf.len()` bytes at `off`.
    fn read(&self, off: usize, buf: &mut [u8]);
    /// Write `data` at `off`.
    fn write(&mut self, off: usize, data: &[u8]);
    /// Order prior arena writes before the following device-visible register/index write.
    fn barrier(&self) {}
}

/// A network interface: raw Ethernet frames in and out.
pub trait Nic {
    /// Station MAC address.
    fn mac(&self) -> [u8; 6];
    /// Whether the link is up.
    fn link_up(&mut self) -> bool;
    /// Queue one frame for transmission (14..=1514 bytes). `Busy` when the ring is full.
    fn send(&mut self, frame: &[u8]) -> Result<(), crate::DriverError>;
    /// Copy the next received frame into `buf`, returning its length, or `None` when idle.
    fn recv(&mut self, buf: &mut [u8]) -> Option<usize>;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::DmaArena;
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    /// Plain byte arena with a fake bus base address.
    pub struct Arena {
        pub base: u64,
        pub mem: Vec<u8>,
    }
    impl Arena {
        pub fn new(base: u64, len: usize) -> Self {
            Arena { base, mem: alloc::vec![0; len] }
        }
        pub fn read(&self, off: usize, buf: &mut [u8]) {
            buf.copy_from_slice(&self.mem[off..off + buf.len()]);
        }
        pub fn write(&mut self, off: usize, data: &[u8]) {
            self.mem[off..off + data.len()].copy_from_slice(data);
        }
    }

    /// Arena shared between the driver under test and the device model.
    pub struct SharedArena(pub Rc<RefCell<Arena>>);
    impl DmaArena for SharedArena {
        fn len(&self) -> usize {
            self.0.borrow().mem.len()
        }
        fn phys(&self, off: usize) -> u64 {
            self.0.borrow().base + off as u64
        }
        fn read(&self, off: usize, buf: &mut [u8]) {
            self.0.borrow().read(off, buf)
        }
        fn write(&mut self, off: usize, data: &[u8]) {
            self.0.borrow_mut().write(off, data)
        }
    }
}

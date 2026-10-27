//! Intel 8254x / 82540EM (QEMU `e1000`) Ethernet driver: legacy descriptor rings, polled.

use crate::hw::{DmaArena, Mmio, Nic};
use crate::DriverError;

const CTRL: usize = 0x0000;
const STATUS: usize = 0x0008;
const EERD: usize = 0x0014;
const IMC: usize = 0x00d8;
const RCTL: usize = 0x0100;
const TCTL: usize = 0x0400;
const TIPG: usize = 0x0410;
const RDBAL: usize = 0x2800;
const RDBAH: usize = 0x2804;
const RDLEN: usize = 0x2808;
const RDH: usize = 0x2810;
const RDT: usize = 0x2818;
const TDBAL: usize = 0x3800;
const TDBAH: usize = 0x3804;
const TDLEN: usize = 0x3808;
const TDH: usize = 0x3810;
const TDT: usize = 0x3818;
const MTA: usize = 0x5200;
const RAL0: usize = 0x5400;
const RAH0: usize = 0x5404;

const CTRL_RST: u32 = 1 << 26;
const CTRL_SLU: u32 = 1 << 6;
const STATUS_LU: u32 = 1 << 1;
const RCTL_EN: u32 = 1 << 1;
const RCTL_BAM: u32 = 1 << 15;
const RCTL_SECRC: u32 = 1 << 26;
const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3;
const RAH_AV: u32 = 1 << 31;

const DESC: usize = 16;
/// Descriptors per ring (multiple of 8, as the hardware requires).
pub const RING: usize = 16;
const BUF: usize = 2048;

/// Arena bytes the driver needs (4-byte aligned rings must be 16-byte aligned on the bus).
pub const ARENA_BYTES: usize = 2 * RING * DESC + 2 * RING * BUF;

const RX_RING: usize = 0;
const TX_RING: usize = RING * DESC;
const RX_BUF: usize = 2 * RING * DESC;
const TX_BUF: usize = RX_BUF + RING * BUF;

const RX_DD: u8 = 1;
const RX_EOP: u8 = 2;
const TX_CMD_EOP_IFCS_RS: u8 = 0b1001 | 0b10;
const TX_DD: u8 = 1;

/// The driver. `dma` must be at least [`ARENA_BYTES`], with a 16-byte aligned bus address.
pub struct E1000<M: Mmio, D: DmaArena> {
    regs: M,
    dma: D,
    mac: [u8; 6],
    rx_next: usize,
    tx_next: usize,
}

impl<M: Mmio, D: DmaArena> E1000<M, D> {
    /// Reset and bring up the NIC: read the MAC, program both rings, enable rx/tx.
    pub fn new(mut regs: M, mut dma: D) -> Result<Self, DriverError> {
        if dma.len() < ARENA_BYTES || dma.phys(0) % 16 != 0 {
            return Err(DriverError::Invalid);
        }
        regs.write32(IMC, 0xffff_ffff);
        let c = regs.read32(CTRL);
        regs.write32(CTRL, c | CTRL_RST);
        let mut spins = 0;
        while regs.read32(CTRL) & CTRL_RST != 0 {
            spins += 1;
            if spins > 100_000 {
                return Err(DriverError::Timeout);
            }
        }
        regs.write32(IMC, 0xffff_ffff);
        let c = regs.read32(CTRL);
        regs.write32(CTRL, c | CTRL_SLU);

        let mac = Self::read_mac(&mut regs)?;
        // Always program RAL0/RAH0 so the filter matches what we report.
        regs.write32(RAL0, u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]));
        regs.write32(RAH0, u32::from_le_bytes([mac[4], mac[5], 0, 0]) | RAH_AV);
        for i in 0..128 {
            regs.write32(MTA + i * 4, 0);
        }

        // RX ring: every descriptor points at its own buffer.
        for i in 0..RING {
            let mut d = [0u8; DESC];
            d[..8].copy_from_slice(&dma.phys(RX_BUF + i * BUF).to_le_bytes());
            dma.write(RX_RING + i * DESC, &d);
        }
        for i in 0..RING {
            dma.write(TX_RING + i * DESC, &[0u8; DESC]);
        }
        let rx = dma.phys(RX_RING);
        let tx = dma.phys(TX_RING);
        regs.write32(RDBAL, rx as u32);
        regs.write32(RDBAH, (rx >> 32) as u32);
        regs.write32(RDLEN, (RING * DESC) as u32);
        regs.write32(RDH, 0);
        regs.write32(RDT, (RING - 1) as u32);
        regs.write32(TDBAL, tx as u32);
        regs.write32(TDBAH, (tx >> 32) as u32);
        regs.write32(TDLEN, (RING * DESC) as u32);
        regs.write32(TDH, 0);
        regs.write32(TDT, 0);
        regs.write32(TIPG, 10 | 8 << 10 | 6 << 20);
        dma.barrier();
        regs.write32(RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC); // 2048-byte buffers (BSIZE=0)
        regs.write32(TCTL, TCTL_EN | TCTL_PSP | 0x0f << 4 | 0x40 << 12);
        Ok(E1000 { regs, dma, mac, rx_next: 0, tx_next: 0 })
    }

    fn read_mac(regs: &mut M) -> Result<[u8; 6], DriverError> {
        let ral = regs.read32(RAL0);
        let rah = regs.read32(RAH0);
        if rah & RAH_AV != 0 && (ral != 0 || rah & 0xffff != 0) {
            let l = ral.to_le_bytes();
            let h = rah.to_le_bytes();
            return Ok([l[0], l[1], l[2], l[3], h[0], h[1]]);
        }
        // Fall back to the EEPROM (words 0..3).
        let mut mac = [0u8; 6];
        for w in 0..3u32 {
            regs.write32(EERD, 1 | w << 8);
            let mut v = 0;
            let mut ok = false;
            for _ in 0..100_000 {
                v = regs.read32(EERD);
                if v & 0x10 != 0 {
                    ok = true;
                    break;
                }
            }
            if !ok {
                return Err(DriverError::Timeout);
            }
            mac[w as usize * 2] = (v >> 16) as u8;
            mac[w as usize * 2 + 1] = (v >> 24) as u8;
        }
        Ok(mac)
    }
}

impl<M: Mmio, D: DmaArena> Nic for E1000<M, D> {
    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn link_up(&mut self) -> bool {
        self.regs.read32(STATUS) & STATUS_LU != 0
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), DriverError> {
        if frame.len() < 14 || frame.len() > 1514 {
            return Err(DriverError::Invalid);
        }
        let i = self.tx_next;
        let mut d = [0u8; DESC];
        self.dma.read(TX_RING + i * DESC, &mut d);
        // A slot that was used before is free only once the device wrote DD back.
        let used_before = d[..8] != [0; 8];
        if used_before && d[12] & TX_DD == 0 {
            return Err(DriverError::Busy);
        }
        self.dma.write(TX_BUF + i * BUF, frame);
        let mut d = [0u8; DESC];
        d[..8].copy_from_slice(&self.dma.phys(TX_BUF + i * BUF).to_le_bytes());
        d[8..10].copy_from_slice(&(frame.len() as u16).to_le_bytes());
        d[11] = TX_CMD_EOP_IFCS_RS;
        self.dma.write(TX_RING + i * DESC, &d);
        self.dma.barrier();
        self.tx_next = (i + 1) % RING;
        self.regs.write32(TDT, self.tx_next as u32);
        Ok(())
    }

    fn recv(&mut self, buf: &mut [u8]) -> Option<usize> {
        let i = self.rx_next;
        let mut d = [0u8; DESC];
        self.dma.read(RX_RING + i * DESC, &mut d);
        if d[12] & RX_DD == 0 {
            return None;
        }
        let len = u16::from_le_bytes([d[8], d[9]]) as usize;
        let good = d[12] & RX_EOP != 0 && d[13] == 0 && len <= BUF && len <= buf.len();
        let n = if good { Some(len) } else { None };
        if let Some(len) = n {
            self.dma.read(RX_BUF + i * BUF, &mut buf[..len]);
        }
        // Recycle the descriptor.
        let mut nd = [0u8; DESC];
        nd[..8].copy_from_slice(&self.dma.phys(RX_BUF + i * BUF).to_le_bytes());
        self.dma.write(RX_RING + i * DESC, &nd);
        self.dma.barrier();
        self.regs.write32(RDT, i as u32);
        self.rx_next = (i + 1) % RING;
        n
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::hw::tests::{Arena, SharedArena};
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    /// Register file + the device-side ring behaviour.
    pub struct Dev {
        pub regs: alloc::collections::BTreeMap<usize, u32>,
        pub arena: Rc<RefCell<Arena>>,
        pub sent: Vec<Vec<u8>>,
        pub tdh: usize,
        pub eeprom_only: bool,
    }
    impl Mmio for Dev {
        fn read32(&mut self, off: usize) -> u32 {
            match off {
                CTRL => self.regs.get(&CTRL).copied().unwrap_or(0) & !CTRL_RST,
                STATUS => STATUS_LU,
                EERD => {
                    let w = (self.regs[&EERD] >> 8) & 0xff;
                    let v = [0x5452u32, 0x0012, 0x3456][w as usize % 3];
                    v << 16 | 0x10
                }
                RAL0 | RAH0 if self.eeprom_only => 0,
                _ => self.regs.get(&off).copied().unwrap_or(0),
            }
        }
        fn write32(&mut self, off: usize, v: u32) {
            self.regs.insert(off, v);
            if off == TDT {
                // device consumes descriptors up to the tail
                let base = self.regs[&TDBAL] as u64;
                assert_eq!(base, self.arena.borrow().base + TX_RING as u64);
                while self.tdh != v as usize {
                    let mut d = [0u8; DESC];
                    self.arena.borrow().read(TX_RING + self.tdh * DESC, &mut d);
                    let len = u16::from_le_bytes([d[8], d[9]]) as usize;
                    assert_eq!(d[11] & 1, 1, "EOP must be set");
                    let mut f = alloc::vec![0u8; len];
                    let addr = u64::from_le_bytes(d[..8].try_into().unwrap());
                    let boff = (addr - self.arena.borrow().base) as usize;
                    self.arena.borrow().read(boff, &mut f);
                    self.sent.push(f);
                    d[12] |= TX_DD;
                    self.arena.borrow_mut().write(TX_RING + self.tdh * DESC, &d);
                    self.tdh = (self.tdh + 1) % RING;
                }
            }
        }
        fn read16(&mut self, _: usize) -> u16 { 0 }
        fn write16(&mut self, _: usize, _: u16) {}
        fn read8(&mut self, _: usize) -> u8 { 0 }
        fn write8(&mut self, _: usize, _: u8) {}
    }

    fn setup(eeprom_only: bool) -> (E1000<Dev, SharedArena>, Rc<RefCell<Arena>>) {
        let arena = Rc::new(RefCell::new(Arena::new(0x10_0000, ARENA_BYTES)));
        let mut regs = alloc::collections::BTreeMap::new();
        if !eeprom_only {
            regs.insert(RAL0, 0x5254_0012u32);
            regs.insert(RAH0, 0x5634 | RAH_AV);
        }
        let dev = Dev { regs, arena: arena.clone(), sent: Vec::new(), tdh: 0, eeprom_only };
        (E1000::new(dev, SharedArena(arena.clone())).unwrap(), arena)
    }

    fn inject(arena: &Rc<RefCell<Arena>>, slot: usize, frame: &[u8], status: u8, errors: u8) {
        arena.borrow_mut().write(RX_BUF + slot * BUF, frame);
        let mut d = [0u8; DESC];
        arena.borrow().read(RX_RING + slot * DESC, &mut d);
        d[8..10].copy_from_slice(&(frame.len() as u16).to_le_bytes());
        d[12] = status;
        d[13] = errors;
        arena.borrow_mut().write(RX_RING + slot * DESC, &d);
    }

    #[test]
    fn init_programs_rings_and_mac() {
        let (mut n, _) = setup(false);
        assert_eq!(n.mac(), [0x12, 0x00, 0x54, 0x52, 0x34, 0x56]);
        assert!(n.link_up());
        assert_eq!(n.regs.regs[&RDLEN], (RING * DESC) as u32);
        assert_eq!(n.regs.regs[&RDT], (RING - 1) as u32);
        assert!(n.regs.regs[&RCTL] & RCTL_EN != 0 && n.regs.regs[&TCTL] & TCTL_EN != 0);
    }

    #[test]
    fn eeprom_mac_fallback() {
        let (n, _) = setup(true);
        assert_eq!(n.mac(), [0x52, 0x54, 0x12, 0x00, 0x56, 0x34]);
    }

    #[test]
    fn rejects_small_dma() {
        let a = Rc::new(RefCell::new(Arena::new(0x1000, 64)));
        let dev = Dev { regs: Default::default(), arena: a.clone(), sent: Vec::new(), tdh: 0, eeprom_only: false };
        assert_eq!(E1000::new(dev, SharedArena(a)).err(), Some(DriverError::Invalid));
    }

    #[test]
    fn transmit_wraps_ring_and_validates() {
        let (mut n, _) = setup(false);
        assert_eq!(n.send(&[0; 10]), Err(DriverError::Invalid));
        for k in 0..(RING * 2 + 3) {
            let f = alloc::vec![k as u8; 60 + k];
            n.send(&f).unwrap();
        }
        assert_eq!(n.regs.sent.len(), RING * 2 + 3);
        assert_eq!(n.regs.sent[RING + 1], alloc::vec![(RING + 1) as u8; 60 + RING + 1]);
    }

    #[test]
    fn transmit_busy_when_device_stalls() {
        let (mut n, arena) = setup(false);
        n.regs.tdh = usize::MAX; // device never consumes: stop the model by making loop impossible
        // Pre-fill every slot as "used but not done".
        for i in 0..RING {
            let mut d = [0u8; DESC];
            d[0] = 1;
            arena.borrow_mut().write(TX_RING + i * DESC, &d);
        }
        assert_eq!(n.send(&[0; 60]), Err(DriverError::Busy));
    }

    #[test]
    fn receive_good_bad_and_wrap() {
        let (mut n, arena) = setup(false);
        let mut buf = [0u8; 2048];
        assert_eq!(n.recv(&mut buf), None);
        inject(&arena, 0, &[7; 64], RX_DD | RX_EOP, 0);
        assert_eq!(n.recv(&mut buf), Some(64));
        assert_eq!(&buf[..64], &[7; 64]);
        assert_eq!(n.regs.regs[&RDT], 0);
        inject(&arena, 1, &[1; 64], RX_DD | RX_EOP, 0x01); // CRC error
        assert_eq!(n.recv(&mut buf), None);
        for k in 2..RING + 3 {
            inject(&arena, k % RING, &[k as u8; 80], RX_DD | RX_EOP, 0);
            assert_eq!(n.recv(&mut buf), Some(80));
            assert_eq!(buf[0], k as u8);
        }
    }
}

//! virtio-net over the virtio 1.x PCI transport (split virtqueues, polled).
//!
//! The transport registers are reached through one [`Mmio`] window (the BAR holding the virtio
//! capabilities); [`Layout`] carries the capability offsets the PCI layer parsed.

use crate::hw::{DmaArena, Mmio, Nic};
use crate::DriverError;

// Common configuration offsets (virtio 1.x §4.1.4.3).
const DEV_FEAT_SEL: usize = 0x00;
const DEV_FEAT: usize = 0x04;
const DRV_FEAT_SEL: usize = 0x08;
const DRV_FEAT: usize = 0x0c;
const STATUS: usize = 0x14;
const Q_SELECT: usize = 0x16;
const Q_SIZE: usize = 0x18;
const Q_ENABLE: usize = 0x1c;
const Q_NOTIFY_OFF: usize = 0x1e;
const Q_DESC: usize = 0x20;
const Q_DRIVER: usize = 0x28;
const Q_DEVICE: usize = 0x30;

const S_ACK: u8 = 1;
const S_DRIVER: u8 = 2;
const S_DRIVER_OK: u8 = 4;
const S_FEATURES_OK: u8 = 8;
const S_FAILED: u8 = 0x80;

const F_MAC: u32 = 1 << 5;
const F_STATUS: u32 = 1 << 16;
const F_VERSION_1_HI: u32 = 1; // bit 32 = bit 0 of the upper word

const NET_S_LINK_UP: u16 = 1;
/// `virtio_net_hdr` with `num_buffers` (always present in virtio 1.x).
const HDR: usize = 12;
const BUF: usize = 2048;
/// Descriptors per queue (power of two, <= the device maximum).
const QSZ: usize = 32;

const F_NEXT_NONE: u16 = 0;
const F_WRITE: u16 = 2;

/// Where the transport structures live inside the BAR window.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// Common configuration offset.
    pub common: usize,
    /// Notification region offset.
    pub notify: usize,
    /// `notify_off_multiplier` from the notify capability.
    pub notify_mult: u32,
    /// Device-specific configuration offset (MAC, status).
    pub device: usize,
}

// Per-queue arena layout.
const DESC_OFF: usize = 0;
const AVAIL_OFF: usize = QSZ * 16;
const USED_OFF: usize = 4096; // page-aligned used ring (avail ring ends well before)
const BUFS_OFF: usize = USED_OFF + 4096;
const Q_BYTES: usize = BUFS_OFF + QSZ * BUF;
/// Arena bytes required (RX queue then TX queue).
pub const ARENA_BYTES: usize = 2 * Q_BYTES;

struct Queue {
    base: usize,
    avail_idx: u16,
    last_used: u16,
    notify_off: usize,
}

impl Queue {
    fn desc<D: DmaArena>(&self, dma: &mut D, i: usize, addr: u64, len: u32, flags: u16) {
        let mut d = [0u8; 16];
        d[..8].copy_from_slice(&addr.to_le_bytes());
        d[8..12].copy_from_slice(&len.to_le_bytes());
        d[12..14].copy_from_slice(&flags.to_le_bytes());
        dma.write(self.base + DESC_OFF + i * 16, &d);
    }

    fn buf(&self, i: usize) -> usize {
        self.base + BUFS_OFF + i * BUF
    }

    /// Publish descriptor chain head `head` on the avail ring.
    fn push<D: DmaArena>(&mut self, dma: &mut D, head: u16) {
        let slot = self.avail_idx as usize % QSZ;
        dma.write(self.base + AVAIL_OFF + 4 + slot * 2, &head.to_le_bytes());
        dma.barrier();
        self.avail_idx = self.avail_idx.wrapping_add(1);
        dma.write(self.base + AVAIL_OFF + 2, &self.avail_idx.to_le_bytes());
        dma.barrier();
    }

    /// Pop one used element `(head, written_len)`.
    fn pop<D: DmaArena>(&mut self, dma: &D) -> Option<(u16, u32)> {
        let mut idx = [0u8; 2];
        dma.read(self.base + USED_OFF + 2, &mut idx);
        if u16::from_le_bytes(idx) == self.last_used {
            return None;
        }
        let mut e = [0u8; 8];
        dma.read(self.base + USED_OFF + 4 + (self.last_used as usize % QSZ) * 8, &mut e);
        self.last_used = self.last_used.wrapping_add(1);
        Some((u32::from_le_bytes([e[0], e[1], e[2], e[3]]) as u16, u32::from_le_bytes([e[4], e[5], e[6], e[7]])))
    }
}

/// The driver.
pub struct VirtioNet<M: Mmio, D: DmaArena> {
    regs: M,
    dma: D,
    lay: Layout,
    mac: [u8; 6],
    has_status: bool,
    rx: Queue,
    tx: Queue,
    tx_free: [bool; QSZ],
}

impl<M: Mmio, D: DmaArena> VirtioNet<M, D> {
    /// Run the virtio init sequence: negotiate `VERSION_1` + `MAC`, set up both queues, go live.
    pub fn new(mut regs: M, mut dma: D, lay: Layout) -> Result<Self, DriverError> {
        if dma.len() < ARENA_BYTES || dma.phys(0) % 4096 != 0 {
            return Err(DriverError::Invalid);
        }
        let c = lay.common;
        regs.write8(c + STATUS, 0);
        regs.write8(c + STATUS, S_ACK);
        regs.write8(c + STATUS, S_ACK | S_DRIVER);

        regs.write32(c + DEV_FEAT_SEL, 0);
        let lo = regs.read32(c + DEV_FEAT);
        regs.write32(c + DEV_FEAT_SEL, 1);
        let hi = regs.read32(c + DEV_FEAT);
        if hi & F_VERSION_1_HI == 0 || lo & F_MAC == 0 {
            regs.write8(c + STATUS, S_FAILED);
            return Err(DriverError::Protocol);
        }
        let want_lo = F_MAC | (lo & F_STATUS);
        regs.write32(c + DRV_FEAT_SEL, 0);
        regs.write32(c + DRV_FEAT, want_lo);
        regs.write32(c + DRV_FEAT_SEL, 1);
        regs.write32(c + DRV_FEAT, F_VERSION_1_HI);
        regs.write8(c + STATUS, S_ACK | S_DRIVER | S_FEATURES_OK);
        if regs.read8(c + STATUS) & S_FEATURES_OK == 0 {
            regs.write8(c + STATUS, S_FAILED);
            return Err(DriverError::Protocol);
        }

        let mut mac = [0u8; 6];
        for (i, m) in mac.iter_mut().enumerate() {
            *m = regs.read8(lay.device + i);
        }

        let mut queues = [None, None];
        for qi in 0..2u16 {
            regs.write16(c + Q_SELECT, qi);
            let max = regs.read16(c + Q_SIZE) as usize;
            if max < QSZ {
                regs.write8(c + STATUS, S_FAILED);
                return Err(DriverError::Protocol);
            }
            regs.write16(c + Q_SIZE, QSZ as u16);
            let base = qi as usize * Q_BYTES;
            for b in 0..Q_BYTES / 8 {
                dma.write(base + b * 8, &[0u8; 8]);
            }
            let wr64 = |regs: &mut M, off: usize, v: u64| {
                regs.write32(off, v as u32);
                regs.write32(off + 4, (v >> 32) as u32);
            };
            wr64(&mut regs, c + Q_DESC, dma.phys(base + DESC_OFF));
            wr64(&mut regs, c + Q_DRIVER, dma.phys(base + AVAIL_OFF));
            wr64(&mut regs, c + Q_DEVICE, dma.phys(base + USED_OFF));
            let noff = regs.read16(c + Q_NOTIFY_OFF) as usize;
            regs.write16(c + Q_ENABLE, 1);
            queues[qi as usize] = Some(Queue {
                base,
                avail_idx: 0,
                last_used: 0,
                notify_off: lay.notify + noff * lay.notify_mult as usize,
            });
        }
        let [Some(mut rx), Some(tx)] = queues else { return Err(DriverError::Protocol) };

        // Hand every RX buffer to the device.
        for i in 0..QSZ {
            let p = dma.phys(rx.buf(i));
            rx.desc(&mut dma, i, p, BUF as u32, F_WRITE);
            rx.push(&mut dma, i as u16);
        }
        let has_status = want_lo & F_STATUS != 0;
        regs.write8(c + STATUS, S_ACK | S_DRIVER | S_FEATURES_OK | S_DRIVER_OK);
        let mut s = VirtioNet { regs, dma, lay, mac, has_status, rx, tx, tx_free: [true; QSZ] };
        s.regs.write16(s.rx.notify_off, 0);
        Ok(s)
    }

    fn reap_tx(&mut self) {
        while let Some((head, _)) = self.tx.pop(&self.dma) {
            if let Some(f) = self.tx_free.get_mut(head as usize) {
                *f = true;
            }
        }
    }
}

impl<M: Mmio, D: DmaArena> Nic for VirtioNet<M, D> {
    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn link_up(&mut self) -> bool {
        !self.has_status || self.regs.read16(self.lay.device + 6) & NET_S_LINK_UP != 0
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), DriverError> {
        if frame.len() < 14 || frame.len() > 1514 {
            return Err(DriverError::Invalid);
        }
        self.reap_tx();
        let i = self.tx_free.iter().position(|&f| f).ok_or(DriverError::Busy)?;
        self.tx_free[i] = false;
        let b = self.tx.buf(i);
        self.dma.write(b, &[0u8; HDR]);
        self.dma.write(b + HDR, frame);
        let p = self.dma.phys(b);
        self.tx.desc(&mut self.dma, i, p, (HDR + frame.len()) as u32, F_NEXT_NONE);
        self.tx.push(&mut self.dma, i as u16);
        self.regs.write16(self.tx.notify_off, 1);
        Ok(())
    }

    fn recv(&mut self, buf: &mut [u8]) -> Option<usize> {
        let (head, written) = self.rx.pop(&self.dma)?;
        let head = head as usize;
        let n = (written as usize).checked_sub(HDR).filter(|&n| head < QSZ && n <= BUF - HDR && n <= buf.len());
        if let Some(n) = n {
            self.dma.read(self.rx.buf(head) + HDR, &mut buf[..n]);
        }
        // Give the buffer back.
        let p = self.dma.phys(self.rx.buf(head));
        self.rx.desc(&mut self.dma, head, p, BUF as u32, F_WRITE);
        self.rx.push(&mut self.dma, head as u16);
        self.regs.write16(self.rx.notify_off, 0);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::tests::{Arena, SharedArena};
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    const LAY: Layout = Layout { common: 0, notify: 0x1000, notify_mult: 4, device: 0x2000 };

    /// Device model: common-config registers plus ring processing on notify.
    struct Dev {
        arena: Rc<RefCell<Arena>>,
        status: u8,
        sel: u16,
        feat_sel: u32,
        drv_feat: [u32; 2],
        qaddr: [[u64; 3]; 2],
        enabled: [bool; 2],
        qsize: [u16; 2],
        sent: Vec<Vec<u8>>,
        tx_used: u16,
        link: bool,
        offer_v1: bool,
    }
    impl Dev {
        fn q(&self) -> usize {
            self.sel as usize & 1
        }
        fn rd(&self, a: u64, n: usize) -> Vec<u8> {
            let ar = self.arena.borrow();
            ar.mem[(a - ar.base) as usize..][..n].to_vec()
        }
        fn wr(&self, a: u64, d: &[u8]) {
            let mut ar = self.arena.borrow_mut();
            let o = (a - ar.base) as usize;
            ar.mem[o..o + d.len()].copy_from_slice(d);
        }
        fn process_tx(&mut self) {
            let [desc, avail, used] = self.qaddr[1];
            let aidx = u16::from_le_bytes(self.rd(avail + 2, 2).try_into().unwrap());
            while self.tx_used != aidx {
                let head = u16::from_le_bytes(self.rd(avail + 4 + (self.tx_used as u64 % QSZ as u64) * 2, 2).try_into().unwrap());
                let d = self.rd(desc + head as u64 * 16, 16);
                let addr = u64::from_le_bytes(d[..8].try_into().unwrap());
                let len = u32::from_le_bytes(d[8..12].try_into().unwrap()) as usize;
                self.sent.push(self.rd(addr, len));
                let mut e = [0u8; 8];
                e[..4].copy_from_slice(&(head as u32).to_le_bytes());
                self.wr(used + 4 + (self.tx_used as u64 % QSZ as u64) * 8, &e);
                self.tx_used += 1;
                self.wr(used + 2, &self.tx_used.to_le_bytes());
            }
        }
    }
    impl Mmio for Dev {
        fn read32(&mut self, off: usize) -> u32 {
            match off {
                DEV_FEAT => {
                    if self.feat_sel == 0 { F_MAC | F_STATUS } else if self.offer_v1 { F_VERSION_1_HI } else { 0 }
                }
                _ => 0,
            }
        }
        fn write32(&mut self, off: usize, v: u32) {
            let q = self.q();
            match off {
                DEV_FEAT_SEL | DRV_FEAT_SEL => self.feat_sel = v,
                DRV_FEAT => self.drv_feat[self.feat_sel as usize & 1] = v,
                o if (Q_DESC..Q_DESC + 24).contains(&o) => {
                    let (idx, hi) = ((o - Q_DESC) / 8, (o - Q_DESC) % 8 == 4);
                    let cur = &mut self.qaddr[q][idx];
                    *cur = if hi { *cur & 0xffff_ffff | (v as u64) << 32 } else { *cur & !0xffff_ffff | v as u64 };
                }
                _ => {}
            }
        }
        fn read16(&mut self, off: usize) -> u16 {
            match off {
                Q_SIZE => 256,
                Q_NOTIFY_OFF => self.sel,
                o if o == LAY.device + 6 => self.link as u16,
                _ => 0,
            }
        }
        fn write16(&mut self, off: usize, v: u16) {
            match off {
                Q_SELECT => self.sel = v,
                Q_SIZE => self.qsize[self.q()] = v,
                Q_ENABLE => {
                    let q = self.q();
                    self.enabled[q] = v == 1
                }
                o if o == LAY.notify + 4 => self.process_tx(), // queue 1, mult 4
                _ => {}
            }
        }
        fn read8(&mut self, off: usize) -> u8 {
            if off == STATUS {
                // device accepts FEATURES_OK only if VERSION_1 was negotiated
                return if self.drv_feat[1] & 1 == 1 { self.status } else { self.status & !S_FEATURES_OK };
            }
            if (LAY.device..LAY.device + 6).contains(&off) {
                return [0x52, 0x54, 0x00, 0x12, 0x34, 0x56][off - LAY.device];
            }
            0
        }
        fn write8(&mut self, off: usize, v: u8) {
            if off == STATUS {
                self.status = v;
            }
        }
    }

    fn setup(offer_v1: bool) -> (Result<VirtioNet<Dev, SharedArena>, DriverError>, Rc<RefCell<Arena>>) {
        let arena = Rc::new(RefCell::new(Arena::new(0x20_0000, ARENA_BYTES)));
        let dev = Dev {
            arena: arena.clone(), status: 0, sel: 0, feat_sel: 0, drv_feat: [0; 2], qaddr: [[0; 3]; 2],
            enabled: [false; 2], qsize: [0; 2], sent: Vec::new(), tx_used: 0, link: true, offer_v1,
        };
        (VirtioNet::new(dev, SharedArena(arena.clone()), LAY), arena)
    }

    /// Device-side RX: take the next avail buffer, write hdr+frame, publish used.
    fn deliver(n: &mut VirtioNet<Dev, SharedArena>, frame: &[u8], rx_used: &mut u16) {
        let [desc, avail, used] = n.regs.qaddr[0];
        let slot = *rx_used as u64 % QSZ as u64;
        let head = u16::from_le_bytes(n.regs.rd(avail + 4 + slot * 2, 2).try_into().unwrap());
        let d = n.regs.rd(desc + head as u64 * 16, 16);
        let addr = u64::from_le_bytes(d[..8].try_into().unwrap());
        let mut pkt = alloc::vec![0u8; HDR];
        pkt.extend_from_slice(frame);
        n.regs.wr(addr, &pkt);
        let mut e = [0u8; 8];
        e[..4].copy_from_slice(&(head as u32).to_le_bytes());
        e[4..].copy_from_slice(&(pkt.len() as u32).to_le_bytes());
        n.regs.wr(used + 4 + slot * 8, &e);
        *rx_used += 1;
        n.regs.wr(used + 2, &rx_used.to_le_bytes());
    }

    #[test]
    fn init_negotiates_and_goes_live() {
        let (n, _) = setup(true);
        let mut n = n.unwrap();
        assert_eq!(n.mac(), [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        assert!(n.link_up());
        assert_eq!(n.regs.status, S_ACK | S_DRIVER | S_FEATURES_OK | S_DRIVER_OK);
        assert_eq!(n.regs.drv_feat[1], 1);
        assert_eq!(n.regs.drv_feat[0] & F_MAC, F_MAC);
        assert!(n.regs.enabled == [true, true] && n.regs.qsize == [QSZ as u16; 2]);
        // all RX buffers posted
        let avail = n.regs.qaddr[0][1];
        assert_eq!(u16::from_le_bytes(n.regs.rd(avail + 2, 2).try_into().unwrap()), QSZ as u16);
        n.regs.link = false;
        assert!(!n.link_up());
    }

    #[test]
    fn legacy_only_device_rejected() {
        let (n, _) = setup(false);
        assert_eq!(n.err(), Some(DriverError::Protocol));
    }

    #[test]
    fn rejects_unaligned_or_small_dma() {
        let arena = Rc::new(RefCell::new(Arena::new(0x20_0010, ARENA_BYTES)));
        let dev = Dev {
            arena: arena.clone(), status: 0, sel: 0, feat_sel: 0, drv_feat: [0; 2], qaddr: [[0; 3]; 2],
            enabled: [false; 2], qsize: [0; 2], sent: Vec::new(), tx_used: 0, link: true, offer_v1: true,
        };
        assert_eq!(VirtioNet::new(dev, SharedArena(arena), LAY).err(), Some(DriverError::Invalid));
    }

    #[test]
    fn send_and_reap() {
        let mut n = setup(true).0.unwrap();
        assert_eq!(n.send(&[0; 5]), Err(DriverError::Invalid));
        for k in 0..(QSZ * 3) {
            n.send(&alloc::vec![k as u8; 64]).unwrap();
        }
        assert_eq!(n.regs.sent.len(), QSZ * 3);
        let last = &n.regs.sent[QSZ * 3 - 1];
        assert_eq!(last.len(), HDR + 64);
        assert_eq!(&last[HDR..], &alloc::vec![(QSZ * 3 - 1) as u8; 64][..]);
    }

    #[test]
    fn send_busy_when_queue_full() {
        let mut n = setup(true).0.unwrap();
        n.tx_free = [false; QSZ];
        assert_eq!(n.send(&[0; 60]), Err(DriverError::Busy));
    }

    #[test]
    fn receive_recycles_buffers() {
        let mut n = setup(true).0.unwrap();
        let mut buf = [0u8; 2048];
        assert_eq!(n.recv(&mut buf), None);
        let mut used = 0u16;
        for k in 0..(QSZ * 2 + 5) {
            let f = alloc::vec![k as u8; 70];
            deliver(&mut n, &f, &mut used);
            assert_eq!(n.recv(&mut buf), Some(70));
            assert_eq!(&buf[..70], &f[..]);
        }
        // runt used-length (shorter than the header) is dropped, not panicked on
        let [_, avail, used_a] = n.regs.qaddr[0];
        let _ = avail;
        let slot = used as u64 % QSZ as u64;
        let head = u16::from_le_bytes(n.regs.rd(n.regs.qaddr[0][1] + 4 + slot * 2, 2).try_into().unwrap());
        let mut e = [0u8; 8];
        e[..4].copy_from_slice(&(head as u32).to_le_bytes());
        e[4..].copy_from_slice(&4u32.to_le_bytes());
        n.regs.wr(used_a + 4 + slot * 8, &e);
        n.regs.wr(used_a + 2, &(used + 1).to_le_bytes());
        assert_eq!(n.recv(&mut buf), None);
    }
}

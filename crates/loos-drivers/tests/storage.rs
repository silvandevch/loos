mod common;
use common::*;
use loos_drivers::ahci::{Ahci, PORT_REGION};
use loos_drivers::hw::Mmio;
use loos_drivers::nvme::{Nvme, ARENA_BYTES};
use loos_drivers::storage::BlockDev;
use loos_drivers::DriverError;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

type Disk = Rc<RefCell<Vec<u8>>>;

// ---------------------------------------------------------------- AHCI model
struct AhciModel {
    mem: Mem,
    disk: Disk,
    regs: HashMap<usize, u32>,
    present: bool,
}

impl AhciModel {
    fn new(mem: Mem, disk: Disk, present: bool) -> Self {
        let mut regs = HashMap::new();
        regs.insert(0x0c, 1); // PI: port 0
        regs.insert(0x100 + 0x28, 0x113);
        regs.insert(0x100 + 0x24, 0x101);
        regs.insert(0x100 + 0x20, 0x50);
        AhciModel { mem, disk, regs, present }
    }
    fn exec(&mut self) {
        let clb = u64::from(self.regs[&0x100]) | u64::from(*self.regs.get(&0x104).unwrap_or(&0)) << 32;
        let hdr = self.mem.rd(clb, 16);
        let flags = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        assert_eq!(flags & 0x1f, 5, "CFL");
        assert_eq!(flags >> 16, 1, "one PRDT entry");
        let ctba = u64::from_le_bytes(hdr[8..16].try_into().unwrap());
        let t = self.mem.rd(ctba, 0x90);
        assert_eq!((t[0], t[1]), (0x27, 0x80), "H2D FIS with C bit");
        let lba = t[4] as u64 | (t[5] as u64) << 8 | (t[6] as u64) << 16 | (t[8] as u64) << 24 | (t[9] as u64) << 32 | (t[10] as u64) << 40;
        let count = u16::from(t[12]) | u16::from(t[13]) << 8;
        let dba = u64::from_le_bytes(t[0x80..0x88].try_into().unwrap());
        let dbc = (u32::from_le_bytes(t[0x8c..0x90].try_into().unwrap()) & 0x3f_ffff) as usize + 1;
        let mut tfd = 0x50;
        match t[2] {
            0xec => {
                let mut id = vec![0u8; 512];
                let sectors = self.disk.borrow().len() as u64 / 512;
                id[83 * 2 + 1] = 1 << 2; // word 83 bit 10: LBA48
                for i in 0..4 {
                    id[(100 + i) * 2..(101 + i) * 2].copy_from_slice(&((sectors >> (16 * i)) as u16).to_le_bytes());
                }
                for (i, c) in b"LoOS Virtual SATA Disk  ".chunks(2).enumerate() {
                    id[(27 + i) * 2] = c[1];
                    id[(27 + i) * 2 + 1] = c[0];
                }
                assert_eq!(dbc, 512);
                self.mem.wr(dba, &id);
            }
            op @ (0x25 | 0x35) => {
                assert_eq!(dbc, count as usize * 512, "PRDT byte count matches sector count");
                let n = dbc;
                let mut d = self.disk.borrow_mut();
                if (lba as usize + count as usize) * 512 > d.len() {
                    tfd |= 1;
                } else if op == 0x25 {
                    self.mem.wr(dba, &d[lba as usize * 512..lba as usize * 512 + n]);
                } else {
                    assert!(flags & (1 << 6) != 0, "write bit set for WRITE DMA EXT");
                    let src = self.mem.rd(dba, n);
                    d[lba as usize * 512..lba as usize * 512 + n].copy_from_slice(&src);
                }
            }
            c => panic!("unexpected ATA command {c:#x}"),
        }
        self.regs.insert(0x100 + 0x20, tfd);
    }
}

impl Mmio for AhciModel {
    fn read32(&mut self, off: usize) -> u32 {
        if !self.present && off == 0x100 + 0x28 {
            return 0;
        }
        let v = *self.regs.get(&off).unwrap_or(&0);
        if off == 0x100 + 0x18 {
            return v | if v & 0x10 != 0 { 1 << 14 } else { 0 } | if v & 1 != 0 { 1 << 15 } else { 0 };
        }
        v
    }
    fn write32(&mut self, off: usize, v: u32) {
        if off == 0x100 + 0x38 {
            if v & 1 != 0 {
                assert!(self.regs[&(0x100 + 0x18)] & 1 != 0, "ST must be set before issuing");
                self.exec();
            }
            return;
        }
        if off == 0x100 + 0x10 || off == 0x100 + 0x30 {
            return; // write-1-to-clear status
        }
        self.regs.insert(off, v);
    }
    fn read16(&mut self, _: usize) -> u16 { 0 }
    fn write16(&mut self, _: usize, _: u16) {}
    fn read8(&mut self, _: usize) -> u8 { 0 }
    fn write8(&mut self, _: usize, _: u8) {}
}

fn disk(sectors: usize) -> Disk {
    Rc::new(RefCell::new(pattern(sectors * 512, 3)))
}

#[test]
fn ahci_identify_read_write() {
    let mem = Mem::new(0x1000_0000, PORT_REGION);
    let d = disk(4096);
    let mut a = Ahci::new(AhciModel::new(mem.clone(), d.clone(), true), mem).unwrap();
    assert_eq!(a.disks().len(), 1);
    assert_eq!(a.disks()[0].sectors, 4096);
    assert_eq!(a.disks()[0].model, "LoOS Virtual SATA Disk");
    let mut dev = a.disk(0).unwrap();
    assert_eq!((dev.sector_size(), dev.sectors()), (512, 4096));
    let mut buf = vec![0u8; 512 * 3];
    dev.read(10, &mut buf).unwrap();
    assert_eq!(buf, d.borrow()[10 * 512..13 * 512]);
    // Larger than the 64 KiB bounce buffer: must be chunked.
    let mut big = vec![0u8; 512 * 300];
    dev.read(100, &mut big).unwrap();
    assert_eq!(big, d.borrow()[100 * 512..400 * 512]);
    let w = pattern(512 * 200, 99);
    dev.write(1000, &w).unwrap();
    assert_eq!(w, d.borrow()[1000 * 512..1200 * 512]);
    let mut back = vec![0u8; w.len()];
    dev.read(1000, &mut back).unwrap();
    assert_eq!(back, w);
}

#[test]
fn ahci_errors_and_no_disk() {
    let mem = Mem::new(0x1000_0000, PORT_REGION);
    let d = disk(64);
    let mut a = Ahci::new(AhciModel::new(mem.clone(), d, true), mem).unwrap();
    let mut dev = a.disk(0).unwrap();
    let mut b = vec![0u8; 512];
    assert_eq!(dev.read(64, &mut b), Err(DriverError::Invalid), "past the end");
    assert_eq!(dev.read(0, &mut vec![0u8; 100]), Err(DriverError::Invalid), "not a sector multiple");
    assert_eq!(dev.read(0, &mut []), Err(DriverError::Invalid));
    let mem = Mem::new(0x1000_0000, PORT_REGION);
    let a = Ahci::new(AhciModel::new(mem.clone(), disk(8), false), mem).unwrap();
    assert!(a.disks().is_empty(), "no link, no disk");
    let misaligned = Mem::new(0x1000_0100, PORT_REGION);
    assert!(Ahci::new(AhciModel::new(misaligned.clone(), disk(8), true), misaligned).is_err());
}

// ---------------------------------------------------------------- NVMe model
struct NvmeModel {
    mem: Mem,
    disk: Disk,
    regs: HashMap<usize, u32>,
    queues: HashMap<u16, Q>,
    ios: usize,
}
#[derive(Default, Clone, Copy)]
struct Q {
    sq: u64,
    cq: u64,
    sq_head: u32,
    cq_tail: u32,
    phase: u32,
}

impl NvmeModel {
    fn new(mem: Mem, disk: Disk) -> Self {
        NvmeModel { mem, disk, regs: HashMap::new(), queues: HashMap::new(), ios: 0 }
    }
    fn post(&mut self, qid: u16, cid: u16, status: u32) {
        let q = self.queues.get_mut(&qid).unwrap();
        let at = q.cq + u64::from(q.cq_tail) * 16;
        let dw2 = q.sq_head | u32::from(qid) << 16;
        let dw3 = u32::from(cid) | (q.phase << 16) | (status << 17);
        self.mem.w32(at + 8, dw2);
        self.mem.w32(at + 12, dw3);
        q.cq_tail += 1;
        if q.cq_tail == 16 {
            q.cq_tail = 0;
            q.phase ^= 1;
        }
    }
    fn prp_copy(&self, e: &[u8], bytes: usize, to_dev: bool, disk_off: usize) {
        let prp1 = u64::from_le_bytes(e[24..32].try_into().unwrap());
        let prp2 = u64::from_le_bytes(e[32..40].try_into().unwrap());
        let pages = bytes.div_ceil(4096);
        let addr = |i: usize| match (i, pages) {
            (0, _) => prp1,
            (1, 2) => prp2,
            (i, _) => self.mem.r64(prp2 + (i as u64 - 1) * 8),
        };
        for i in 0..pages {
            let n = (bytes - i * 4096).min(4096);
            let a = addr(i);
            let mut d = self.disk.borrow_mut();
            if to_dev {
                d[disk_off + i * 4096..disk_off + i * 4096 + n].copy_from_slice(&self.mem.rd(a, n));
            } else {
                self.mem.wr(a, &d[disk_off + i * 4096..disk_off + i * 4096 + n]);
            }
        }
    }
    fn run(&mut self, qid: u16, tail: u32) {
        loop {
            let q = self.queues[&qid];
            if q.sq_head == tail {
                break;
            }
            let e = self.mem.rd(q.sq + u64::from(q.sq_head) * 64, 64);
            self.queues.get_mut(&qid).unwrap().sq_head = (q.sq_head + 1) % 16;
            let cid = u16::from_le_bytes([e[2], e[3]]);
            let cdw = |i: usize| u32::from_le_bytes(e[40 + 4 * i..44 + 4 * i].try_into().unwrap());
            let prp1 = u64::from_le_bytes(e[24..32].try_into().unwrap());
            let mut status = 0;
            if qid == 0 {
                match e[0] {
                    0x09 => assert_eq!(cdw(0) & 0xff, 7),
                    0x05 => {
                        assert_eq!(cdw(1) & 1, 1, "physically contiguous");
                        let id = cdw(0) as u16;
                        self.queues.insert(id, Q { cq: prp1, phase: 1, ..Q::default() });
                    }
                    0x01 => {
                        let id = cdw(0) as u16;
                        assert!(self.queues.contains_key(&((cdw(1) >> 16) as u16)), "CQ created before SQ");
                        let cq = self.queues[&((cdw(1) >> 16) as u16)];
                        self.queues.insert(id, Q { sq: prp1, ..cq });
                    }
                    0x06 => {
                        let mut id = vec![0u8; 4096];
                        if cdw(0) & 0xff == 1 {
                            id[24..24 + 15].copy_from_slice(b"LoOS NVMe Drive");
                        } else {
                            let n = self.disk.borrow().len() as u64 / 512;
                            id[0..8].copy_from_slice(&n.to_le_bytes());
                            id[130] = 9; // LBAF0.LBADS
                        }
                        self.mem.wr(prp1, &id);
                    }
                    op => panic!("admin opcode {op:#x}"),
                }
            } else {
                self.ios += 1;
                let slba = u64::from(cdw(0)) | u64::from(cdw(1)) << 32;
                let n = (cdw(2) & 0xffff) as usize + 1;
                if (slba as usize + n) * 512 > self.disk.borrow().len() {
                    status = 0x80; // LBA out of range
                } else {
                    assert_eq!(e[4], 1, "nsid 1");
                    self.prp_copy(&e, n * 512, e[0] == 1, slba as usize * 512);
                }
            }
            self.post(qid, cid, status);
        }
    }
}

impl Mmio for NvmeModel {
    fn read32(&mut self, off: usize) -> u32 {
        match off {
            0x00 => 0x00ff, // CAP low: MQES
            0x04 => 0,      // CAP high: DSTRD 0
            0x1c => u32::from(*self.regs.get(&0x14).unwrap_or(&0) & 1),
            _ => *self.regs.get(&off).unwrap_or(&0),
        }
    }
    fn write32(&mut self, off: usize, v: u32) {
        if off >= 0x1000 {
            let idx = (off - 0x1000) / 4;
            if idx % 2 == 0 {
                self.run((idx / 2) as u16, v);
            }
            return;
        }
        if off == 0x28 {
            let q = self.queues.entry(0).or_default();
            q.sq = (q.sq & !0xffff_ffff) | u64::from(v);
        }
        if off == 0x30 {
            let q = self.queues.entry(0).or_default();
            q.cq = u64::from(v);
            q.phase = 1;
        }
        self.regs.insert(off, v);
    }
    fn read16(&mut self, _: usize) -> u16 { 0 }
    fn write16(&mut self, _: usize, _: u16) {}
    fn read8(&mut self, _: usize) -> u8 { 0 }
    fn write8(&mut self, _: usize, _: u8) {}
}

#[test]
fn nvme_init_and_io() {
    let mem = Mem::new(0x2000_0000, ARENA_BYTES);
    let d = disk(8192);
    let model = NvmeModel::new(mem.clone(), d.clone());
    let mut n = Nvme::new(model, mem).unwrap();
    assert_eq!(n.model, "LoOS NVMe Drive");
    assert_eq!((n.sector_size(), n.sectors()), (512, 8192));
    let mut b = vec![0u8; 512];
    n.read(7, &mut b).unwrap(); // one page
    assert_eq!(b, d.borrow()[7 * 512..8 * 512]);
    let mut b = vec![0u8; 512 * 16]; // two pages: PRP2 direct
    n.read(100, &mut b).unwrap();
    assert_eq!(b, d.borrow()[100 * 512..116 * 512]);
    let mut b = vec![0u8; 512 * 256]; // 32 pages: PRP list
    n.read(1000, &mut b).unwrap();
    assert_eq!(b, d.borrow()[1000 * 512..1256 * 512]);
    let w = pattern(512 * 600, 42); // > bounce: multiple commands, wraps the 16-entry queue several times
    n.write(2000, &w).unwrap();
    assert_eq!(w, d.borrow()[2000 * 512..2600 * 512]);
    let mut back = vec![0u8; w.len()];
    n.read(2000, &mut back).unwrap();
    assert_eq!(back, w);
}

#[test]
fn nvme_errors() {
    let mem = Mem::new(0x2000_0000, ARENA_BYTES);
    let mut n = Nvme::new(NvmeModel::new(mem.clone(), disk(64)), mem).unwrap();
    assert_eq!(n.read(64, &mut vec![0u8; 512]), Err(DriverError::Invalid));
    assert_eq!(n.read(0, &mut vec![0u8; 7]), Err(DriverError::Invalid));
    for i in 0..40 {
        n.read(i % 60, &mut vec![0u8; 512]).unwrap(); // queue wrap + phase flips keep working
    }
    let small = Mem::new(0x2000_0000, 0x1000);
    assert!(Nvme::new(NvmeModel::new(small.clone(), disk(8)), small).is_err());
}

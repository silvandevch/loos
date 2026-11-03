//! NVMe 1.4 controller driver: admin queue + one I/O queue pair, polled, PRP lists, bounce buffer.

use crate::hw::{DmaArena, Mmio};
use crate::storage::*;
use crate::DriverError;
use alloc::string::String;

const CAP: usize = 0x00;
const CC: usize = 0x14;
const CSTS: usize = 0x1c;
const AQA: usize = 0x24;
const ASQ: usize = 0x28;
const ACQ: usize = 0x30;
const DB: usize = 0x1000;
const QD: usize = 16;

/// Arena layout.
const ADMIN_SQ: usize = 0x0000;
const ADMIN_CQ: usize = 0x1000;
const IO_SQ: usize = 0x2000;
const IO_CQ: usize = 0x3000;
const IDENT: usize = 0x4000;
const PRP_LIST: usize = 0x5000;
const DATA: usize = 0x6000;
const DATA_LEN: usize = 0x2_0000;
/// Minimum arena size.
pub const ARENA_BYTES: usize = DATA + DATA_LEN;

struct Queue {
    sq: usize,
    cq: usize,
    tail: usize,
    head: usize,
    phase: u32,
    sq_db: usize,
    cq_db: usize,
}

/// The controller with namespace 1 attached.
pub struct Nvme<M: Mmio, D: DmaArena> {
    m: M,
    dma: D,
    admin: Queue,
    io: Queue,
    cid: u16,
    /// Controller model string.
    pub model: String,
    sector: u32,
    sectors: u64,
}

impl<M: Mmio, D: DmaArena> Nvme<M, D> {
    /// Resets and initialises the controller, creates the I/O queues and identifies namespace 1.
    /// `dma` must be 4 KiB aligned and at least [`ARENA_BYTES`] long.
    pub fn new(mut m: M, mut dma: D) -> Result<Self, DriverError> {
        if dma.phys(0) & 0xfff != 0 || dma.len() < ARENA_BYTES {
            return Err(DriverError::Invalid);
        }
        let stride = 4usize << (m.read32(CAP + 4) & 0xf);
        let db = |q: usize, cq: bool| DB + (2 * q + usize::from(cq)) * stride;
        if m.read32(CC) & 1 != 0 {
            m.write32(CC, 0);
            wait(|| m.read32(CSTS) & 1 == 0)?;
        }
        m.write32(AQA, ((QD as u32 - 1) << 16) | (QD as u32 - 1));
        w64(&mut m, ASQ, dma.phys(ADMIN_SQ));
        w64(&mut m, ACQ, dma.phys(ADMIN_CQ));
        for off in [ADMIN_CQ, IO_CQ] {
            dma.write(off, &[0u8; QD * 16]);
        }
        m.write32(CC, 1 | (6 << 16) | (4 << 20));
        wait(|| {
            let s = m.read32(CSTS);
            s & 1 != 0 || s & 2 != 0
        })?;
        if m.read32(CSTS) & 2 != 0 {
            return Err(DriverError::Protocol);
        }
        let admin = Queue { sq: ADMIN_SQ, cq: ADMIN_CQ, tail: 0, head: 0, phase: 1, sq_db: db(0, false), cq_db: db(0, true) };
        let io = Queue { sq: IO_SQ, cq: IO_CQ, tail: 0, head: 0, phase: 1, sq_db: db(1, false), cq_db: db(1, true) };
        let mut n = Nvme { m, dma, admin, io, cid: 0, model: String::new(), sector: 512, sectors: 0 };
        // Set Features: number of queues = 1 SQ + 1 CQ.
        n.admin_cmd(0x09, 0, 0, 0, [7, 0, 0, 0, 0, 0])?;
        let cq = n.dma.phys(IO_CQ);
        let sq = n.dma.phys(IO_SQ);
        n.admin_cmd(0x05, 0, cq, 0, [((QD as u32 - 1) << 16) | 1, 1, 0, 0, 0, 0])?;
        n.admin_cmd(0x01, 0, sq, 0, [((QD as u32 - 1) << 16) | 1, (1 << 16) | 1, 0, 0, 0, 0])?;
        let id = n.dma.phys(IDENT);
        n.admin_cmd(0x06, 0, id, 0, [1, 0, 0, 0, 0, 0])?;
        let mut raw = [0u8; 4096];
        n.dma.read(IDENT, &mut raw);
        n.model = String::from_utf8_lossy(&raw[24..64]).trim_matches(|c: char| c == ' ' || c == '\0').into();
        n.admin_cmd(0x06, 1, id, 0, [0, 0, 0, 0, 0, 0])?;
        n.dma.read(IDENT, &mut raw);
        n.sectors = u64::from_le_bytes(raw[0..8].try_into().unwrap());
        let fmt = usize::from(raw[26] & 0xf);
        let lbads = raw[128 + 4 * fmt + 2];
        if !(9..=12).contains(&lbads) || n.sectors == 0 {
            return Err(DriverError::Protocol);
        }
        n.sector = 1 << lbads;
        Ok(n)
    }

    fn submit(m: &mut M, dma: &mut D, q: &mut Queue, sqe: &[u8; 64]) -> Result<u32, DriverError> {
        dma.write(q.sq + q.tail * 64, sqe);
        q.tail = (q.tail + 1) % QD;
        dma.barrier();
        m.write32(q.sq_db, q.tail as u32);
        let (cq, head, phase) = (q.cq, q.head, q.phase);
        wait(|| (dma_u32(dma, cq + head * 16 + 12) >> 16) & 1 == phase)?;
        let dw0 = dma_u32(dma, q.cq + q.head * 16);
        let dw3 = dma_u32(dma, q.cq + q.head * 16 + 12);
        q.head += 1;
        if q.head == QD {
            q.head = 0;
            q.phase ^= 1;
        }
        m.write32(q.cq_db, q.head as u32);
        if (dw3 >> 17) & 0x7fff != 0 {
            return Err(DriverError::Protocol);
        }
        Ok(dw0)
    }

    fn sqe(&mut self, op: u8, nsid: u32, prp1: u64, prp2: u64, cdw: [u32; 6]) -> [u8; 64] {
        self.cid = self.cid.wrapping_add(1);
        let mut e = [0u8; 64];
        e[0] = op;
        e[2..4].copy_from_slice(&self.cid.to_le_bytes());
        e[4..8].copy_from_slice(&nsid.to_le_bytes());
        e[24..32].copy_from_slice(&prp1.to_le_bytes());
        e[32..40].copy_from_slice(&prp2.to_le_bytes());
        for (i, v) in cdw.iter().enumerate() {
            e[40 + 4 * i..44 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        e
    }

    fn admin_cmd(&mut self, op: u8, nsid: u32, prp1: u64, prp2: u64, cdw: [u32; 6]) -> Result<u32, DriverError> {
        let e = self.sqe(op, nsid, prp1, prp2, cdw);
        Self::submit(&mut self.m, &mut self.dma, &mut self.admin, &e)
    }

    fn io(&mut self, write: bool, lba: u64, buf: &mut [u8]) -> Result<(), DriverError> {
        let ss = self.sector as usize;
        let total = check_chunk(buf.len(), self.sector)?;
        if lba.checked_add(total as u64).map_or(true, |e| e > self.sectors) {
            return Err(DriverError::Invalid);
        }
        let per = DATA_LEN / ss;
        let (mut done, mut cur) = (0usize, lba);
        while done < total {
            let n = (total - done).min(per);
            let bytes = n * ss;
            let slice = &mut buf[done * ss..done * ss + bytes];
            if write {
                self.dma.write(DATA, slice);
            }
            let p0 = self.dma.phys(DATA);
            let pages = bytes.div_ceil(4096);
            let prp2 = match pages {
                1 => 0,
                2 => p0 + 4096,
                _ => {
                    for i in 1..pages {
                        self.dma.w64(PRP_LIST + (i - 1) * 8, p0 + 4096 * i as u64);
                    }
                    self.dma.phys(PRP_LIST)
                }
            };
            let e = self.sqe(if write { 1 } else { 2 }, 1, p0, prp2, [cur as u32, (cur >> 32) as u32, n as u32 - 1, 0, 0, 0]);
            Self::submit(&mut self.m, &mut self.dma, &mut self.io, &e)?;
            if !write {
                self.dma.read(DATA, slice);
            }
            done += n;
            cur += n as u64;
        }
        Ok(())
    }
}

impl<M: Mmio, D: DmaArena> BlockDev for Nvme<M, D> {
    fn sector_size(&self) -> u32 {
        self.sector
    }
    fn sectors(&self) -> u64 {
        self.sectors
    }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), DriverError> {
        self.io(false, lba, buf)
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), DriverError> {
        let mut t = data.to_vec();
        self.io(true, lba, &mut t)
    }
}

//! AHCI 1.3.1 SATA host controller driver (one command slot, polled, DMA through a bounce buffer).

use crate::hw::{DmaArena, Mmio};
use crate::storage::*;
use crate::DriverError;
use alloc::string::String;
use alloc::vec::Vec;

const GHC: usize = 0x04;
const PI: usize = 0x0c;
const GHC_AE: u32 = 1 << 31;
const P_BASE: usize = 0x100;
const P_STRIDE: usize = 0x80;
const P_CLB: usize = 0x00;
const P_FB: usize = 0x08;
const P_IS: usize = 0x10;
const P_CMD: usize = 0x18;
const P_TFD: usize = 0x20;
const P_SIG: usize = 0x24;
const P_SSTS: usize = 0x28;
const P_SERR: usize = 0x30;
const P_CI: usize = 0x38;
const CMD_ST: u32 = 1;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;
const TFD_ERR: u32 = 1;
const TFD_DRQ: u32 = 1 << 3;
const TFD_BSY: u32 = 1 << 7;
const SIG_ATA: u32 = 0x0000_0101;

const ATA_IDENTIFY: u8 = 0xec;
const ATA_READ48: u8 = 0x25;
const ATA_WRITE48: u8 = 0x35;

/// Arena bytes used per port: command structures (4 KiB) + 64 KiB bounce buffer.
pub const PORT_REGION: usize = 0x1_1000;
const DATA_OFF: usize = 0x1000;
const BOUNCE: usize = 0x1_0000;
const SECTOR: usize = 512;

/// One attached SATA disk.
#[derive(Debug, Clone)]
pub struct DiskInfo {
    /// Controller port number.
    pub port: usize,
    /// Capacity in 512-byte sectors.
    pub sectors: u64,
    /// IDENTIFY model string.
    pub model: String,
    region: usize,
}

/// The controller.
pub struct Ahci<M: Mmio, D: DmaArena> {
    m: M,
    dma: D,
    disks: Vec<DiskInfo>,
}

impl<M: Mmio, D: DmaArena> Ahci<M, D> {
    /// Resets nothing, enables AHCI mode, and brings up every port with a SATA disk attached.
    /// `dma` must start on a 4 KiB bus-address boundary and hold `PORT_REGION` bytes per disk.
    pub fn new(mut m: M, dma: D) -> Result<Self, DriverError> {
        if dma.phys(0) & 0xfff != 0 {
            return Err(DriverError::Invalid);
        }
        let g = m.read32(GHC);
        m.write32(GHC, g | GHC_AE);
        let pi = m.read32(PI);
        let mut a = Ahci { m, dma, disks: Vec::new() };
        for n in 0..32usize {
            if pi & (1 << n) == 0 {
                continue;
            }
            let pb = P_BASE + P_STRIDE * n;
            let ssts = a.m.read32(pb + P_SSTS);
            if ssts & 0xf != 3 || (ssts >> 8) & 0xf != 1 || a.m.read32(pb + P_SIG) != SIG_ATA {
                continue;
            }
            let region = a.disks.len() * PORT_REGION;
            if region + PORT_REGION > a.dma.len() {
                break;
            }
            a.start_port(n, region)?;
            let (sectors, model) = a.identify(n, region)?;
            a.disks.push(DiskInfo { port: n, sectors, model, region });
        }
        Ok(a)
    }

    /// Attached disks.
    pub fn disks(&self) -> &[DiskInfo] {
        &self.disks
    }

    /// Borrows disk `idx` as a [`BlockDev`].
    pub fn disk(&mut self, idx: usize) -> Option<AhciDisk<'_, M, D>> {
        (idx < self.disks.len()).then_some(AhciDisk { ctl: self, idx })
    }

    fn start_port(&mut self, n: usize, region: usize) -> Result<(), DriverError> {
        let pb = P_BASE + P_STRIDE * n;
        let cmd = self.m.read32(pb + P_CMD);
        self.m.write32(pb + P_CMD, cmd & !(CMD_ST | CMD_FRE));
        let m = &mut self.m;
        wait(|| m.read32(pb + P_CMD) & (CMD_FR | CMD_CR) == 0)?;
        let clb = self.dma.phys(region);
        let fb = self.dma.phys(region + 0x400);
        self.m.write32(pb + P_CLB, clb as u32);
        self.m.write32(pb + P_CLB + 4, (clb >> 32) as u32);
        self.m.write32(pb + P_FB, fb as u32);
        self.m.write32(pb + P_FB + 4, (fb >> 32) as u32);
        self.m.write32(pb + P_SERR, 0xffff_ffff);
        self.m.write32(pb + P_IS, 0xffff_ffff);
        self.m.write32(pb + P_CMD, CMD_FRE);
        self.m.write32(pb + P_CMD, CMD_FRE | CMD_ST);
        Ok(())
    }

    /// Issues one command through slot 0 and waits for completion.
    fn issue(&mut self, port: usize, region: usize, cmd: u8, lba: u64, count: u16, write: bool, bytes: usize) -> Result<(), DriverError> {
        let pb = P_BASE + P_STRIDE * port;
        let m = &mut self.m;
        wait(|| m.read32(pb + P_TFD) & (TFD_BSY | TFD_DRQ) == 0)?;
        let ctba = self.dma.phys(region + 0x500);
        // Command header (slot 0): CFL=5 dwords, W, one PRDT entry.
        let hdr: u32 = 5 | if write { 1 << 6 } else { 0 } | (1 << 16);
        let mut h = [0u8; 16];
        h[0..4].copy_from_slice(&hdr.to_le_bytes());
        h[8..12].copy_from_slice(&(ctba as u32).to_le_bytes());
        h[12..16].copy_from_slice(&((ctba >> 32) as u32).to_le_bytes());
        self.dma.write(region, &h);
        // Command table: H2D register FIS + PRDT.
        let mut t = [0u8; 0x90];
        t[0] = 0x27;
        t[1] = 0x80;
        t[2] = cmd;
        t[4] = lba as u8;
        t[5] = (lba >> 8) as u8;
        t[6] = (lba >> 16) as u8;
        t[7] = 0x40;
        t[8] = (lba >> 24) as u8;
        t[9] = (lba >> 32) as u8;
        t[10] = (lba >> 40) as u8;
        t[12] = count as u8;
        t[13] = (count >> 8) as u8;
        let data = self.dma.phys(region + DATA_OFF);
        t[0x80..0x84].copy_from_slice(&(data as u32).to_le_bytes());
        t[0x84..0x88].copy_from_slice(&((data >> 32) as u32).to_le_bytes());
        t[0x8c..0x90].copy_from_slice(&((bytes as u32 - 1) & 0x3f_ffff).to_le_bytes());
        self.dma.write(region + 0x500, &t);
        self.dma.barrier();
        self.m.write32(pb + P_CI, 1);
        let m = &mut self.m;
        wait(|| m.read32(pb + P_CI) & 1 == 0 || m.read32(pb + P_TFD) & TFD_ERR != 0)?;
        let err = self.m.read32(pb + P_TFD) & TFD_ERR != 0 || self.m.read32(pb + P_IS) & (1 << 30) != 0;
        self.m.write32(pb + P_IS, 0xffff_ffff);
        if err {
            self.m.write32(pb + P_SERR, 0xffff_ffff);
            return Err(DriverError::Protocol);
        }
        Ok(())
    }

    fn identify(&mut self, port: usize, region: usize) -> Result<(u64, String), DriverError> {
        self.issue(port, region, ATA_IDENTIFY, 0, 0, false, SECTOR)?;
        let mut raw = [0u8; SECTOR];
        self.dma.read(region + DATA_OFF, &mut raw);
        let w = |i: usize| u16::from_le_bytes([raw[2 * i], raw[2 * i + 1]]);
        let sectors = if w(83) & (1 << 10) != 0 {
            u64::from(w(100)) | u64::from(w(101)) << 16 | u64::from(w(102)) << 32 | u64::from(w(103)) << 48
        } else {
            u64::from(w(60)) | u64::from(w(61)) << 16
        };
        let mut model = String::new();
        for i in 27..47 {
            model.push((w(i) >> 8) as u8 as char);
            model.push(w(i) as u8 as char);
        }
        Ok((sectors, model.trim_matches(|c: char| c == ' ' || c == '\0').into()))
    }

    fn rw(&mut self, idx: usize, lba: u64, buf: &mut [u8], write: bool) -> Result<(), DriverError> {
        let total = check_chunk(buf.len(), SECTOR as u32)?;
        let (port, region, cap) = {
            let d = &self.disks[idx];
            (d.port, d.region, d.sectors)
        };
        if lba.checked_add(total as u64).map_or(true, |e| e > cap) {
            return Err(DriverError::Invalid);
        }
        let per = BOUNCE / SECTOR;
        let (mut done, mut cur) = (0usize, lba);
        while done < total {
            let n = (total - done).min(per);
            let bytes = n * SECTOR;
            let slice = &mut buf[done * SECTOR..done * SECTOR + bytes];
            if write {
                self.dma.write(region + DATA_OFF, slice);
            }
            self.issue(port, region, if write { ATA_WRITE48 } else { ATA_READ48 }, cur, n as u16, write, bytes)?;
            if !write {
                self.dma.read(region + DATA_OFF, slice);
            }
            done += n;
            cur += n as u64;
        }
        Ok(())
    }
}

/// A borrowed SATA disk.
pub struct AhciDisk<'a, M: Mmio, D: DmaArena> {
    ctl: &'a mut Ahci<M, D>,
    idx: usize,
}

impl<M: Mmio, D: DmaArena> BlockDev for AhciDisk<'_, M, D> {
    fn sector_size(&self) -> u32 {
        SECTOR as u32
    }
    fn sectors(&self) -> u64 {
        self.ctl.disks[self.idx].sectors
    }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), DriverError> {
        self.ctl.rw(self.idx, lba, buf, false)
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), DriverError> {
        let mut tmp = data.to_vec();
        self.ctl.rw(self.idx, lba, &mut tmp, true)
    }
}

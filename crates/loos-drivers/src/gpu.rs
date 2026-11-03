//! Display modesetting: EDID parsing, mode selection, and the Bochs/QEMU "dispi" VGA adapter
//! (also used by VirtualBox VBoxVGA and many hypervisors), with double-buffer page flipping.

use crate::display::{FbInfo, PixelFormat};
use crate::hw::Mmio;
use crate::DriverError;
use alloc::string::String;
use alloc::vec::Vec;

/// A display timing mode (only the resolution matters for linear-framebuffer adapters).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mode {
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

/// What an EDID base block says about the monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edid {
    /// Three-letter PnP manufacturer id.
    pub vendor: String,
    /// Product name descriptor, if present.
    pub name: String,
    /// Native (preferred) resolution from the first detailed timing descriptor.
    pub preferred: Option<Mode>,
    /// Established-timing modes (the legacy VGA/SVGA/XGA bitmap).
    pub established: Vec<Mode>,
}

/// Parses a 128-byte EDID base block, verifying header and checksum.
pub fn parse_edid(b: &[u8]) -> Result<Edid, DriverError> {
    if b.len() < 128 || b[0..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return Err(DriverError::Protocol);
    }
    if b[..128].iter().fold(0u8, |a, &x| a.wrapping_add(x)) != 0 {
        return Err(DriverError::Protocol);
    }
    let m = u16::from_be_bytes([b[8], b[9]]);
    let vendor: String = [(m >> 10) & 31, (m >> 5) & 31, m & 31].iter().map(|&c| (b'A' + c as u8 - 1) as char).collect();
    let mut name = String::new();
    let mut preferred = None;
    for (i, d) in b[54..126].chunks_exact(18).enumerate() {
        let clock = u16::from_le_bytes([d[0], d[1]]);
        if clock != 0 {
            if i == 0 {
                let w = u32::from(d[2]) | (u32::from(d[4] >> 4) << 8);
                let h = u32::from(d[5]) | (u32::from(d[7] >> 4) << 8);
                preferred = Some(Mode { w, h });
            }
        } else if d[3] == 0xfc {
            name = d[5..18].iter().take_while(|&&c| c != b'\n').map(|&c| c as char).collect::<String>().trim().into();
        }
    }
    let mut established = Vec::new();
    for (byte, bit, w, h) in [(35, 5, 640, 480), (35, 0, 800, 600), (36, 3, 1024, 768), (36, 2, 1152, 870)] {
        if b[byte] & (1 << bit) != 0 {
            established.push(Mode { w, h });
        }
    }
    Ok(Edid { vendor, name, preferred, established })
}

/// Chooses the mode to set: the EDID-preferred one if the adapter supports it, else the largest
/// supported mode that fits it, else the largest supported mode.
pub fn select_mode(supported: &[Mode], edid: Option<&Edid>) -> Option<Mode> {
    if let Some(p) = edid.and_then(|e| e.preferred) {
        if supported.contains(&p) {
            return Some(p);
        }
        if let Some(m) = supported.iter().copied().filter(|m| m.w <= p.w && m.h <= p.h).max_by_key(|m| u64::from(m.w) * u64::from(m.h)) {
            return Some(m);
        }
    }
    supported.iter().copied().max_by_key(|m| u64::from(m.w) * u64::from(m.h))
}

/// A display adapter that can change resolution.
pub trait Gpu {
    /// Modes the adapter accepts.
    fn modes(&self) -> Vec<Mode>;
    /// Switches to `mode` at 32 bpp with two vertically stacked pages; returns the scanout geometry.
    fn set_mode(&mut self, mode: Mode) -> Result<FbInfo, DriverError>;
    /// Makes page 0 or 1 the visible one (tear-free double buffering).
    fn flip(&mut self, page: u32) -> Result<(), DriverError>;
}

// dispi registers, 16-bit each, at `base + index * 2` in the adapter's MMIO BAR (offset 0x500 on QEMU).
const R_ID: usize = 0;
const R_XRES: usize = 1;
const R_YRES: usize = 2;
const R_BPP: usize = 3;
const R_ENABLE: usize = 4;
const R_VIRT_W: usize = 6;
const R_VIRT_H: usize = 7;
const R_YOFF: usize = 9;
const R_VIDEO_MEM_64K: usize = 10;
const EN_ENABLED: u16 = 1;
const EN_LFB: u16 = 0x40;
const EN_NOCLEAR: u16 = 0x80;

/// Bochs dispi adapter.
pub struct BochsVga<M: Mmio> {
    m: M,
    base: usize,
    vram: usize,
    mode: Option<Mode>,
}

impl<M: Mmio> BochsVga<M> {
    /// Probes the adapter. `base` is the dispi register window offset inside `m` (0x500 on QEMU).
    pub fn new(mut m: M, base: usize) -> Result<Self, DriverError> {
        let id = m.read16(base + R_ID * 2);
        if !(0xb0c0..=0xb0c5).contains(&id) {
            return Err(DriverError::Protocol);
        }
        let vram = usize::from(m.read16(base + R_VIDEO_MEM_64K * 2)) * 0x1_0000;
        Ok(BochsVga { m, base, vram, mode: None })
    }

    fn reg(&self, r: usize) -> usize {
        self.base + r * 2
    }
}

impl<M: Mmio> Gpu for BochsVga<M> {
    fn modes(&self) -> Vec<Mode> {
        [(640, 480), (800, 600), (1024, 768), (1280, 720), (1280, 1024), (1366, 768), (1600, 900), (1920, 1080), (2560, 1440), (3840, 2160)]
            .iter()
            .map(|&(w, h)| Mode { w, h })
            .filter(|m| self.vram == 0 || (m.w as usize) * (m.h as usize) * 4 * 2 <= self.vram)
            .collect()
    }

    fn set_mode(&mut self, mode: Mode) -> Result<FbInfo, DriverError> {
        if !self.modes().contains(&mode) {
            return Err(DriverError::Invalid);
        }
        let (en, xr, yr, bp, vw, vh, yo) = (self.reg(R_ENABLE), self.reg(R_XRES), self.reg(R_YRES), self.reg(R_BPP), self.reg(R_VIRT_W), self.reg(R_VIRT_H), self.reg(R_YOFF));
        self.m.write16(en, 0);
        self.m.write16(bp, 32);
        self.m.write16(xr, mode.w as u16);
        self.m.write16(yr, mode.h as u16);
        self.m.write16(vw, mode.w as u16);
        self.m.write16(vh, (mode.h * 2) as u16);
        self.m.write16(yo, 0);
        self.m.write16(en, EN_ENABLED | EN_LFB | EN_NOCLEAR);
        // The adapter latches what it accepted; verify.
        if self.m.read16(xr) != mode.w as u16 || self.m.read16(yr) != mode.h as u16 {
            return Err(DriverError::Protocol);
        }
        self.mode = Some(mode);
        Ok(FbInfo { width: mode.w, height: mode.h, pitch: mode.w * 4, format: PixelFormat::Bgrx32 })
    }

    fn flip(&mut self, page: u32) -> Result<(), DriverError> {
        let m = self.mode.ok_or(DriverError::Invalid)?;
        if page > 1 {
            return Err(DriverError::Invalid);
        }
        let yo = self.reg(R_YOFF);
        self.m.write16(yo, (page * m.h) as u16);
        Ok(())
    }
}

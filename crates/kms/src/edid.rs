//! EDID 1.x base block (128 bytes) parser.

use crate::mode::{cvt_rb, flags, Mode};
use alloc::string::String;
use alloc::vec::Vec;

/// Parse errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Fewer than 128 bytes.
    Short,
    /// Header is not `00 FF FF FF FF FF FF 00`.
    BadHeader,
    /// Block checksum nonzero.
    BadChecksum,
}

/// Parsed EDID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edid {
    /// Three-letter PNP manufacturer id.
    pub manufacturer: [u8; 3],
    /// Product code.
    pub product: u16,
    /// EDID version.revision.
    pub version: (u8, u8),
    /// Digital input?
    pub digital: bool,
    /// Physical size in mm (0 if unknown / projector).
    pub size_mm: (u32, u32),
    /// Monitor name descriptor.
    pub name: String,
    /// Number of extension blocks that follow.
    pub extensions: u8,
    /// Detailed timings (first is preferred when `preferred_first`).
    pub detailed: Vec<Mode>,
    /// Established + standard timings expanded to modes.
    pub standard: Vec<Mode>,
}

fn dtd(b: &[u8]) -> Option<Mode> {
    let clock = u16::from_le_bytes([b[0], b[1]]) as u32 * 10;
    if clock == 0 {
        return None;
    }
    let hact = b[2] as u32 | ((b[4] as u32 & 0xF0) << 4);
    let hbl = b[3] as u32 | ((b[4] as u32 & 0x0F) << 8);
    let vact = b[5] as u32 | ((b[7] as u32 & 0xF0) << 4);
    let vbl = b[6] as u32 | ((b[7] as u32 & 0x0F) << 8);
    let hso = b[8] as u32 | ((b[11] as u32 & 0xC0) << 2);
    let hsw = b[9] as u32 | ((b[11] as u32 & 0x30) << 4);
    let vso = (b[10] as u32 >> 4) | ((b[11] as u32 & 0x0C) << 2);
    let vsw = (b[10] as u32 & 0x0F) | ((b[11] as u32 & 0x03) << 4);
    let f = b[17];
    let mut fl = 0;
    if f & 0x80 != 0 {
        fl |= flags::INTERLACE;
    }
    if f & 0x18 == 0x18 {
        fl |= if f & 0x04 != 0 { flags::PVSYNC } else { flags::NVSYNC };
        fl |= if f & 0x02 != 0 { flags::PHSYNC } else { flags::NHSYNC };
    }
    let m = Mode {
        clock,
        hdisplay: hact as u16,
        hsync_start: (hact + hso) as u16,
        hsync_end: (hact + hso + hsw) as u16,
        htotal: (hact + hbl) as u16,
        vdisplay: vact as u16,
        vsync_start: (vact + vso) as u16,
        vsync_end: (vact + vso + vsw) as u16,
        vtotal: (vact + vbl) as u16,
        flags: fl,
        preferred: false,
    };
    m.valid().then_some(m)
}

const ESTABLISHED: [(u16, u16, u32); 16] = [
    (720, 400, 70), (720, 400, 88), (640, 480, 60), (640, 480, 67), (640, 480, 72), (640, 480, 75), (800, 600, 56), (800, 600, 60),
    (800, 600, 72), (800, 600, 75), (832, 624, 75), (1024, 768, 87), (1024, 768, 60), (1024, 768, 70), (1024, 768, 75), (1280, 1024, 75),
];

impl Edid {
    /// Parse the base block.
    pub fn parse(b: &[u8]) -> Result<Edid, Error> {
        if b.len() < 128 {
            return Err(Error::Short);
        }
        if b[..8] != [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0] {
            return Err(Error::BadHeader);
        }
        if b[..128].iter().fold(0u8, |a, &x| a.wrapping_add(x)) != 0 {
            return Err(Error::BadChecksum);
        }
        let id = u16::from_be_bytes([b[8], b[9]]);
        let manufacturer = [(((id >> 10) & 31) as u8 + b'@'), (((id >> 5) & 31) as u8 + b'@'), ((id & 31) as u8 + b'@')];
        let digital = b[20] & 0x80 != 0;
        let mut e = Edid {
            manufacturer,
            product: u16::from_le_bytes([b[10], b[11]]),
            version: (b[18], b[19]),
            digital,
            size_mm: (b[21] as u32 * 10, b[22] as u32 * 10),
            name: String::new(),
            extensions: b[126],
            detailed: Vec::new(),
            standard: Vec::new(),
        };
        for d in 0..4 {
            let o = 54 + d * 18;
            let blk = &b[o..o + 18];
            if blk[0] != 0 || blk[1] != 0 {
                if let Some(m) = dtd(blk) {
                    e.detailed.push(m);
                }
            } else if blk[3] == 0xFC {
                e.name = blk[5..18].iter().take_while(|&&c| c != 0x0A && c != 0).map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '?' }).collect::<String>().trim().into();
            }
        }
        // preferred = first DTD when the feature bit says so (EDID 1.3+)
        if b[24] & 0x02 != 0 {
            if let Some(m) = e.detailed.first_mut() {
                m.preferred = true;
            }
        }
        for (i, &(w, h, hz)) in ESTABLISHED.iter().enumerate() {
            let byte = if i < 8 { b[35] } else { b[36] };
            let bit = if i < 8 { 7 - i } else { 15 - i };
            if byte >> bit & 1 == 1 {
                if let Some(m) = cvt_rb(w as u32, h as u32, hz) {
                    e.standard.push(m);
                }
            }
        }
        for s in 0..8 {
            let (x, y) = (b[38 + s * 2], b[39 + s * 2]);
            if x == 0x01 && y == 0x01 {
                continue; // unused slot
            }
            let w = (x as u32 + 31) * 8;
            let h = match y >> 6 {
                0 => w * 10 / 16, // EDID 1.3+: 16:10
                1 => w * 3 / 4,
                2 => w * 4 / 5,
                _ => w * 9 / 16,
            };
            if let Some(m) = cvt_rb(w, h, (y & 63) as u32 + 60) {
                e.standard.push(m);
            }
        }
        Ok(e)
    }

    /// All modes: detailed first, then standard; sorted and de-duplicated.
    pub fn modes(&self) -> Vec<Mode> {
        let mut v: Vec<Mode> = self.detailed.iter().chain(self.standard.iter()).copied().collect();
        crate::mode::sort_and_dedup(&mut v);
        v
    }

    /// Manufacturer as text.
    pub fn manufacturer_str(&self) -> String {
        self.manufacturer.iter().map(|&c| c as char).collect()
    }

    /// Pixel density along x in dots per inch, if the size is known.
    pub fn dpi_x(&self) -> Option<u32> {
        let m = self.detailed.first()?;
        (self.size_mm.0 > 0).then(|| (m.hdisplay as u64 * 254 / (self.size_mm.0 as u64 * 10)) as u32)
    }
}

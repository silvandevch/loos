//! Device-independent bitmaps: `BITMAPINFO` + pixel data <-> 0xAARRGGBB rows (1/4/8/16/24/32 bpp, uncompressed).

use alloc::vec::Vec;

/// Decoded header facts.
pub struct Info {
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels (absolute).
    pub h: i32,
    /// Rows are stored top-down (negative `biHeight`).
    pub top_down: bool,
    /// Bits per pixel.
    pub bpp: u16,
    /// Colour table (0x00RRGGBB), for <= 8 bpp.
    pub palette: Vec<u32>,
    /// Bytes from the start of `BITMAPINFO` to the colour table's end.
    pub header_len: usize,
}

/// Row stride in bytes (DWORD aligned).
pub fn stride(w: i32, bpp: u16) -> usize {
    ((w.max(0) as usize * usize::from(bpp) + 31) / 32) * 4
}

/// Parses a `BITMAPINFOHEADER` (or larger V4/V5) and its colour table from `bmi`.
pub fn parse_info(bmi: &[u8]) -> Option<Info> {
    let size = u32::from_le_bytes(bmi.get(0..4)?.try_into().ok()?) as usize;
    if size < 40 {
        return None; // BITMAPCOREHEADER is not supported
    }
    let i32_at = |o: usize| Some(i32::from_le_bytes(bmi.get(o..o + 4)?.try_into().ok()?));
    let (w, hh) = (i32_at(4)?, i32_at(8)?);
    let bpp = u16::from_le_bytes(bmi.get(14..16)?.try_into().ok()?);
    let compression = i32_at(16)?;
    let used = i32_at(32)? as u32 as usize;
    if w <= 0 || hh == 0 || w > 16384 || hh.unsigned_abs() > 16384 || !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) || !matches!(compression, 0 | 3) {
        return None;
    }
    let mut palette = Vec::new();
    let mut header_len = size;
    if bpp <= 8 {
        let n = if used == 0 { 1usize << bpp } else { used.min(256) };
        for i in 0..n {
            let q = bmi.get(size + 4 * i..size + 4 * i + 4)?; // RGBQUAD: B, G, R, reserved
            palette.push(u32::from(q[2]) << 16 | u32::from(q[1]) << 8 | u32::from(q[0]));
        }
        header_len += 4 * n;
    } else if compression == 3 && size == 40 {
        header_len += 12; // BI_BITFIELDS masks (assumed the standard 565 / 888 layouts)
    }
    Some(Info { w, h: hh.abs(), top_down: hh < 0, bpp, palette, header_len })
}

/// Decodes `bits` into top-down 0xFFRRGGBB pixels.
pub fn decode(info: &Info, bits: &[u8]) -> Option<Vec<u32>> {
    let st = stride(info.w, info.bpp);
    if bits.len() < st * info.h as usize {
        return None;
    }
    let mut out = alloc::vec![0u32; (info.w * info.h) as usize];
    for y in 0..info.h as usize {
        let src_row = if info.top_down { y } else { info.h as usize - 1 - y };
        let row = &bits[src_row * st..src_row * st + st];
        for x in 0..info.w as usize {
            let rgb = match info.bpp {
                // Memory order is B,G,R(,pad): little-endian reads give 0x00RRGGBB.
                32 => u32::from_le_bytes([row[4 * x], row[4 * x + 1], row[4 * x + 2], 0]),
                24 => u32::from_le_bytes([row[3 * x], row[3 * x + 1], row[3 * x + 2], 0]),
                16 => {
                    let p = u32::from(u16::from_le_bytes([row[2 * x], row[2 * x + 1]]));
                    let (r, g, b) = ((p >> 10 & 31) * 255 / 31, (p >> 5 & 31) * 255 / 31, (p & 31) * 255 / 31); // 5-5-5
                    r << 16 | g << 8 | b
                }
                8 => *info.palette.get(usize::from(row[x]))?,
                4 => *info.palette.get(usize::from(if x % 2 == 0 { row[x / 2] >> 4 } else { row[x / 2] & 15 }))?,
                _ => *info.palette.get(usize::from(row[x / 8] >> (7 - x % 8) & 1))?,
            };
            let px = 0xff00_0000 | rgb;
            out[y * info.w as usize + x] = px;
        }
    }
    Some(out)
}

/// Encodes top-down pixels as 32 or 24 bpp rows (bottom-up unless `top_down`).
pub fn encode(px: &[u32], w: i32, h: i32, bpp: u16, top_down: bool) -> Vec<u8> {
    let st = stride(w, bpp);
    let mut out = alloc::vec![0u8; st * h as usize];
    for y in 0..h as usize {
        let dst_row = if top_down { y } else { h as usize - 1 - y };
        for x in 0..w as usize {
            let p = px[y * w as usize + x];
            let (b, g, r) = (p as u8, (p >> 8) as u8, (p >> 16) as u8);
            let o = dst_row * st;
            if bpp == 32 {
                out[o + 4 * x..o + 4 * x + 4].copy_from_slice(&[b, g, r, 0]);
            } else {
                out[o + 3 * x..o + 3 * x + 3].copy_from_slice(&[b, g, r]);
            }
        }
    }
    out
}

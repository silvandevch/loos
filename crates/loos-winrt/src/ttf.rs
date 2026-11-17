//! TrueType (`glyf`) font files: table directory, character map, metrics, names and quadratic outlines.
//! Serves DirectWrite (`dwrite.rs`); outlines feed the shell's scanline rasteriser. Malformed data never panics:
//! lookups answer `None`/0 and the font is skipped at load.

use alloc::string::String;
use alloc::vec::Vec;

fn be16(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}
fn bei16(d: &[u8], o: usize) -> Option<i16> {
    be16(d, o).map(|v| v as i16)
}
fn be32(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Table tag as DirectWrite passes it (`DWRITE_MAKE_OPENTYPE_TAG`: first character in the low byte).
pub fn tag_le(t: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*t)
}

/// A parsed font file.
pub struct Ttf {
    /// The whole file.
    pub data: Vec<u8>,
    tables: Vec<([u8; 4], usize, usize)>,
    /// Units per em.
    pub upm: u16,
    /// Ascent above the baseline, font units (Windows metrics when present).
    pub ascent: u16,
    /// Descent below the baseline (positive), font units.
    pub descent: u16,
    /// Extra line gap, font units.
    pub line_gap: i16,
    /// Capital height.
    pub cap_height: u16,
    /// x height.
    pub x_height: u16,
    /// Underline position and thickness.
    pub underline: (i16, u16),
    /// Strikeout position and thickness.
    pub strikeout: (i16, u16),
    /// Glyph count.
    pub glyphs: u16,
    /// `usWeightClass` (400 regular, 700 bold).
    pub weight: u16,
    /// `usWidthClass` (5 normal).
    pub stretch: u16,
    /// Italic or oblique.
    pub italic: bool,
    /// Symbol font (Windows symbol cmap).
    pub symbol: bool,
    hmetrics: u16,
    long_loca: bool,
    cmap: Option<(usize, u16)>,
}

/// One glyph outline: TrueType points (font units, y up), on-curve flags and exclusive contour ends.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Outline {
    /// x, y pairs.
    pub xy: Vec<i16>,
    /// 1 for on-curve points.
    pub on: Vec<u8>,
    /// Exclusive end index of every contour.
    pub ends: Vec<u16>,
}

impl Ttf {
    /// Parses a TrueType font (a single font, not a collection). `None` for CFF/OTF or damaged files.
    pub fn parse(data: Vec<u8>) -> Option<Ttf> {
        let n = be16(&data, 4)? as usize;
        let mut tables = Vec::with_capacity(n);
        for i in 0..n {
            let r = 12 + 16 * i;
            let tag: [u8; 4] = data.get(r..r + 4)?.try_into().ok()?;
            let (off, len) = (be32(&data, r + 8)? as usize, be32(&data, r + 12)? as usize);
            if off.checked_add(len)? > data.len() {
                return None;
            }
            tables.push((tag, off, len));
        }
        let mut f = Ttf { data, tables, upm: 0, ascent: 0, descent: 0, line_gap: 0, cap_height: 0, x_height: 0, underline: (0, 0), strikeout: (0, 0), glyphs: 0, weight: 400, stretch: 5, italic: false, symbol: false, hmetrics: 0, long_loca: false, cmap: None };
        f.table(b"glyf")?;
        let head = f.table_off(b"head")?;
        f.upm = be16(&f.data, head + 18)?;
        f.long_loca = bei16(&f.data, head + 50)? == 1;
        f.italic = be16(&f.data, head + 44)? & 2 != 0;
        let hhea = f.table_off(b"hhea")?;
        f.ascent = bei16(&f.data, hhea + 4)?.max(0) as u16;
        f.descent = bei16(&f.data, hhea + 6)?.unsigned_abs();
        f.line_gap = bei16(&f.data, hhea + 8)?;
        f.hmetrics = be16(&f.data, hhea + 34)?;
        f.glyphs = be16(&f.data, f.table_off(b"maxp")? + 4)?;
        if let Some(os2) = f.table_off(b"OS/2") {
            let d = &f.data;
            f.weight = be16(d, os2 + 4).unwrap_or(400);
            f.stretch = be16(d, os2 + 6).unwrap_or(5);
            f.strikeout = (bei16(d, os2 + 28).unwrap_or(0), bei16(d, os2 + 26).unwrap_or(0).max(0) as u16);
            f.italic |= be16(d, os2 + 62).unwrap_or(0) & 1 != 0;
            // Windows metrics: usWinAscent/usWinDescent decide line spacing like DirectWrite does.
            if let (Some(a), Some(b)) = (be16(d, os2 + 74), be16(d, os2 + 76)) {
                if a > 0 {
                    (f.ascent, f.descent, f.line_gap) = (a, b, 0);
                }
            }
            if be16(d, os2).unwrap_or(0) >= 2 {
                f.x_height = bei16(d, os2 + 86).unwrap_or(0).max(0) as u16;
                f.cap_height = bei16(d, os2 + 88).unwrap_or(0).max(0) as u16;
            }
        }
        if f.cap_height == 0 {
            f.cap_height = f.upm * 7 / 10;
        }
        if f.x_height == 0 {
            f.x_height = f.upm / 2;
        }
        if let Some(post) = f.table_off(b"post") {
            f.underline = (bei16(&f.data, post + 8).unwrap_or(0), bei16(&f.data, post + 10).unwrap_or(0).max(0) as u16);
        }
        f.cmap = f.pick_cmap();
        f.cmap?;
        Some(f)
    }

    fn table_off(&self, tag: &[u8; 4]) -> Option<usize> {
        self.tables.iter().find(|t| &t.0 == tag).map(|t| t.1)
    }

    /// `(offset, length)` of a table in [`Ttf::data`].
    pub fn table(&self, tag: &[u8; 4]) -> Option<(usize, usize)> {
        self.tables.iter().find(|t| &t.0 == tag).map(|t| (t.1, t.2))
    }

    /// Table by DirectWrite tag (see [`tag_le`]).
    pub fn table_le(&self, tag: u32) -> Option<(usize, usize)> {
        self.table(&tag.to_le_bytes())
    }

    fn pick_cmap(&mut self) -> Option<(usize, u16)> {
        let cmap = self.table_off(b"cmap")?;
        let n = be16(&self.data, cmap + 2)? as usize;
        let mut best: Option<(u8, usize, u16)> = None;
        for i in 0..n {
            let r = cmap + 4 + 8 * i;
            let (pid, eid, off) = (be16(&self.data, r)?, be16(&self.data, r + 2)?, be32(&self.data, r + 4)? as usize);
            let sub = cmap + off;
            let fmt = be16(&self.data, sub)?;
            // Prefer full Unicode (format 12), then the BMP (format 4); a Windows symbol map marks a symbol font.
            let rank = match (pid, eid, fmt) {
                (3, 10, 12) | (0, 4, 12) | (0, 6, 12) => 4,
                (3, 1, 4) | (0, 3, 4) => 3,
                (0, _, 4) => 2,
                (3, 0, 4) => {
                    self.symbol = true;
                    1
                }
                _ => continue,
            };
            if best.is_none_or(|b| b.0 < rank) {
                best = Some((rank, sub, fmt));
            }
        }
        best.map(|b| (b.1, b.2))
    }

    /// Glyph index for a code point (0 = .notdef). Symbol fonts also map U+F000 + c for 8-bit codes.
    pub fn glyph_index(&self, cp: u32) -> u16 {
        let g = self.lookup(cp);
        if g == 0 && self.symbol && cp < 0x100 { self.lookup(0xF000 + cp) } else { g }
    }

    fn lookup(&self, cp: u32) -> u16 {
        let Some((sub, fmt)) = self.cmap else { return 0 };
        let d = &self.data;
        let r = || -> Option<u16> {
            if fmt == 12 {
                let n = be32(d, sub + 12)? as usize;
                let (mut lo, mut hi) = (0, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let g = sub + 16 + 12 * mid;
                    let (s, e) = (be32(d, g)?, be32(d, g + 4)?);
                    if cp < s {
                        hi = mid;
                    } else if cp > e {
                        lo = mid + 1;
                    } else {
                        return Some((be32(d, g + 8)? + (cp - s)) as u16);
                    }
                }
                return Some(0);
            }
            if cp > 0xFFFF {
                return Some(0);
            }
            let c = cp as u16;
            let seg2 = be16(d, sub + 6)? as usize;
            let ends = sub + 14;
            let starts = ends + seg2 + 2;
            let deltas = starts + seg2;
            let ranges = deltas + seg2;
            for i in 0..seg2 / 2 {
                if be16(d, ends + 2 * i)? < c {
                    continue;
                }
                let start = be16(d, starts + 2 * i)?;
                if start > c {
                    return Some(0);
                }
                let delta = be16(d, deltas + 2 * i)?;
                let ro = be16(d, ranges + 2 * i)? as usize;
                if ro == 0 {
                    return Some(c.wrapping_add(delta));
                }
                let gi = be16(d, ranges + 2 * i + ro + 2 * (c - start) as usize)?;
                return Some(if gi == 0 { 0 } else { gi.wrapping_add(delta) });
            }
            Some(0)
        };
        r().unwrap_or(0)
    }

    /// Advance width and left side bearing of a glyph, font units.
    pub fn hmetric(&self, g: u16) -> (u16, i16) {
        let Some(h) = self.table_off(b"hmtx") else { return (0, 0) };
        let n = self.hmetrics.max(1);
        let d = &self.data;
        if g < n {
            (be16(d, h + 4 * g as usize).unwrap_or(0), bei16(d, h + 4 * g as usize + 2).unwrap_or(0))
        } else {
            let adv = be16(d, h + 4 * (n as usize - 1)).unwrap_or(0);
            (adv, bei16(d, h + 4 * n as usize + 2 * (g - n) as usize).unwrap_or(0))
        }
    }

    fn glyph_range(&self, g: u16) -> Option<(usize, usize)> {
        let loca = self.table_off(b"loca")?;
        let glyf = self.table_off(b"glyf")?;
        let at = |i: usize| -> Option<usize> {
            Some(if self.long_loca { be32(&self.data, loca + 4 * i)? as usize } else { be16(&self.data, loca + 2 * i)? as usize * 2 })
        };
        let (s, e) = (at(g as usize)?, at(g as usize + 1)?);
        (e > s).then_some((glyf + s, glyf + e))
    }

    /// Bounding box `(xMin, yMin, xMax, yMax)` in font units; `None` for empty glyphs (space).
    pub fn bbox(&self, g: u16) -> Option<(i16, i16, i16, i16)> {
        let (s, _) = self.glyph_range(g)?;
        let d = &self.data;
        Some((bei16(d, s + 2)?, bei16(d, s + 4)?, bei16(d, s + 6)?, bei16(d, s + 8)?))
    }

    /// Quadratic outline of a glyph (composites flattened, up to 8 levels deep).
    pub fn outline(&self, g: u16) -> Outline {
        let mut o = Outline::default();
        self.append(g, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], &mut o, 0);
        o
    }

    fn append(&self, g: u16, m: [f32; 6], o: &mut Outline, depth: u32) -> Option<()> {
        let (s, e) = self.glyph_range(g)?;
        let d = &self.data;
        let nc = bei16(d, s)?;
        if nc >= 0 {
            let nc = nc as usize;
            let base = o.on.len() as u16;
            let mut ends = Vec::with_capacity(nc);
            for i in 0..nc {
                ends.push(be16(d, s + 10 + 2 * i)? as usize + 1);
            }
            let npts = *ends.last().unwrap_or(&0);
            let ins = be16(d, s + 10 + 2 * nc)? as usize;
            let mut p = s + 12 + 2 * nc + ins;
            let mut flags = Vec::with_capacity(npts);
            while flags.len() < npts {
                let f = *d.get(p)?;
                p += 1;
                flags.push(f);
                if f & 8 != 0 {
                    let rep = *d.get(p)?;
                    p += 1;
                    for _ in 0..rep {
                        flags.push(f);
                    }
                }
            }
            flags.truncate(npts);
            let mut coords = |short: u8, same: u8| -> Option<Vec<i32>> {
                let mut v = Vec::with_capacity(npts);
                let mut c = 0i32;
                for &f in &flags {
                    if f & short != 0 {
                        let b = i32::from(*d.get(p)?);
                        p += 1;
                        c += if f & same != 0 { b } else { -b };
                    } else if f & same == 0 {
                        c += i32::from(bei16(d, p)?);
                        p += 2;
                    }
                    v.push(c);
                }
                Some(v)
            };
            let xs = coords(2, 16)?;
            let ys = coords(4, 32)?;
            if p > e {
                return None;
            }
            for i in 0..npts {
                let (x, y) = (xs[i] as f32, ys[i] as f32);
                o.xy.push((m[0] * x + m[2] * y + m[4]) as i16);
                o.xy.push((m[1] * x + m[3] * y + m[5]) as i16);
                o.on.push(flags[i] & 1);
            }
            for end in ends {
                o.ends.push(base + end as u16);
            }
            return Some(());
        }
        if depth >= 8 {
            return None;
        }
        // Composite: components with offsets and optional scale / 2x2 transforms (point matching is ignored).
        let mut p = s + 10;
        loop {
            let flags = be16(d, p)?;
            let gi = be16(d, p + 2)?;
            p += 4;
            let (dx, dy) = if flags & 1 != 0 {
                let r = (f32::from(bei16(d, p)?), f32::from(bei16(d, p + 2)?));
                p += 4;
                r
            } else {
                let r = (f32::from(*d.get(p)? as i8), f32::from(*d.get(p + 1)? as i8));
                p += 2;
                r
            };
            let f2 = |o: usize| -> Option<f32> { Some(f32::from(bei16(d, o)?) / 16384.0) };
            let mut t = [1.0, 0.0, 0.0, 1.0];
            if flags & 8 != 0 {
                let s = f2(p)?;
                t = [s, 0.0, 0.0, s];
                p += 2;
            } else if flags & 0x40 != 0 {
                t = [f2(p)?, 0.0, 0.0, f2(p + 2)?];
                p += 4;
            } else if flags & 0x80 != 0 {
                t = [f2(p)?, f2(p + 2)?, f2(p + 4)?, f2(p + 6)?];
                p += 8;
            }
            let (dx, dy) = if flags & 2 != 0 { (dx, dy) } else { (0.0, 0.0) };
            let cm = [
                m[0] * t[0] + m[2] * t[1],
                m[1] * t[0] + m[3] * t[1],
                m[0] * t[2] + m[2] * t[3],
                m[1] * t[2] + m[3] * t[3],
                m[0] * dx + m[2] * dy + m[4],
                m[1] * dx + m[3] * dy + m[5],
            ];
            self.append(gi, cm, o, depth + 1);
            if flags & 0x20 == 0 {
                return Some(());
            }
        }
    }

    /// A `name` table string (Windows Unicode English preferred, then any Windows Unicode, then Mac Roman).
    pub fn name(&self, id: u16) -> Option<String> {
        let t = self.table_off(b"name")?;
        let d = &self.data;
        let n = be16(d, t + 2)? as usize;
        let strings = t + be16(d, t + 4)? as usize;
        let mut best: Option<(u8, usize, usize, bool)> = None;
        for i in 0..n {
            let r = t + 6 + 12 * i;
            if be16(d, r + 6)? != id {
                continue;
            }
            let (pid, eid, lang) = (be16(d, r)?, be16(d, r + 2)?, be16(d, r + 4)?);
            let (len, off) = (be16(d, r + 8)? as usize, be16(d, r + 10)? as usize);
            let rank = match (pid, eid, lang) {
                (3, 1, 0x409) => 3,
                (3, 1, _) | (0, _, _) => 2,
                (1, 0, 0) => 1,
                _ => continue,
            };
            if best.is_none_or(|b| b.0 < rank) {
                best = Some((rank, strings + off, len, pid != 1));
            }
        }
        let (_, off, len, wide) = best?;
        let raw = d.get(off..off + len)?;
        Some(if wide {
            let u: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&u)
        } else {
            raw.iter().map(|&b| char::from(b)).collect()
        })
    }

    /// Family name for grouping (typographic family, else the legacy family).
    pub fn family(&self) -> String {
        self.name(16).or_else(|| self.name(1)).unwrap_or_default()
    }

    /// Face name within the family ("Regular", "Bold Italic").
    pub fn face(&self) -> String {
        self.name(17).or_else(|| self.name(2)).unwrap_or_else(|| String::from("Regular"))
    }
}

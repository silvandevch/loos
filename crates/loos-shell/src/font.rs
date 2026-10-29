//! Outline font engine (C `font.c`, `font.h`): UTF-8 decoding, glyph lookup, advances,
//! fitting/indexing and the glyph coverage-bitmap cache.

use crate::raster::{self, RASTER_MAX_SIZE};

/// C `loos_glyph`: advance in font units; `pt0`/`end0` index the point/contour tables.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoosGlyph {
    /// Code point.
    pub cp: u16,
    /// Advance in font units.
    pub adv: u16,
    /// First point in `pts`/`on`.
    pub pt0: u16,
    /// First contour in `ends`.
    pub end0: u16,
    /// Contour count.
    pub ncont: u16,
}

/// C `loos_font`: immutable outline tables (y up).
#[repr(C)]
pub struct LoosFont {
    /// Units per em.
    pub upm: u16,
    /// Ascent in font units.
    pub ascent: i16,
    /// Descent in font units (negative).
    pub descent: i16,
    /// Number of glyphs.
    pub nglyphs: u16,
    /// Glyph table (sorted by code point).
    pub glyphs: *const LoosGlyph,
    /// x,y point pairs.
    pub pts: *const i16,
    /// On-curve flag per point.
    pub on: *const u8,
    /// Exclusive end point of every contour.
    pub ends: *const u16,
}

// SAFETY: the outline tables are immutable and only read; the pointers never change.
unsafe impl Sync for LoosFont {}

/// C `shell_font`: a face at a logical size in px (em height).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShellFont {
    /// Face.
    pub face: *const LoosFont,
    /// Logical px.
    pub size: i32,
}

/// C `font_bitmap`: coverage bitmap relative to the pen origin / baseline.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FontBitmap {
    /// Bitmap left from the pen origin, device px.
    pub left: i32,
    /// Bitmap top from the baseline, device px.
    pub top: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
    /// `w * h` coverage bytes, valid until the next cache flush.
    pub cov: *const u8,
}

impl FontBitmap {
    /// Empty bitmap.
    pub const EMPTY: Self = Self { left: 0, top: 0, w: 0, h: 0, cov: core::ptr::null() };
}

/// Borrowed view over a font's outline tables.
#[derive(Clone, Copy)]
pub struct Font<'a> {
    /// Units per em.
    pub upm: u16,
    /// Ascent in font units.
    pub ascent: i16,
    /// Descent in font units.
    pub descent: i16,
    /// Glyphs, sorted by code point.
    pub glyphs: &'a [LoosGlyph],
    /// Point pairs.
    pub pts: &'a [i16],
    /// On-curve flags.
    pub on: &'a [u8],
    /// Contour ends.
    pub ends: &'a [u16],
}

impl<'a> Font<'a> {
    /// Builds a view over already-sliced tables.
    pub fn new(upm: u16, ascent: i16, descent: i16, glyphs: &'a [LoosGlyph], pts: &'a [i16], on: &'a [u8], ends: &'a [u16]) -> Self {
        Self { upm, ascent, descent, glyphs, pts, on, ends }
    }
}

/// Decodes the next UTF-8 code point from `s[*pos..]` and advances `pos`; invalid bytes decode
/// as U+FFFD and consume one byte, exactly like C `font_next_cp`.
pub fn next_cp(s: &[u8], pos: &mut usize) -> u32 {
    let i = *pos;
    let c = match s.get(i) {
        Some(&c) => c,
        None => return 0,
    };
    if c == 0 {
        return 0;
    }
    let b1 = *s.get(i + 1).unwrap_or(&0);
    let b2 = *s.get(i + 2).unwrap_or(&0);
    let cont = |b: u8| b & 0xc0 == 0x80;
    let (cp, n) = if c < 0x80 {
        (c as u32, 1)
    } else if (0xc2..0xe0).contains(&c) && cont(b1) {
        (((c as u32 & 0x1f) << 6) | (b1 as u32 & 0x3f), 2)
    } else if (0xe0..0xf0).contains(&c) && cont(b1) && cont(b2) {
        let mut cp = ((c as u32 & 0x0f) << 12) | ((b1 as u32 & 0x3f) << 6) | (b2 as u32 & 0x3f);
        if cp < 0x800 || (0xd800..0xe000).contains(&cp) {
            cp = 0xfffd;
        }
        (cp, 3)
    } else {
        (0xfffd, 1)
    };
    *pos += n;
    cp
}

/// Binary-searches the glyph table; returns its index.
pub fn find(glyphs: &[LoosGlyph], cp: u32) -> Option<usize> {
    // C uses the inclusive form; the generated table has a few out-of-order tail entries, so the
    // exact search path matters for matching the C engine.
    let mut lo = 0i32;
    let mut hi = glyphs.len() as i32 - 1;
    while lo <= hi {
        let mid = ((lo + hi) / 2) as usize;
        let mid_cp = glyphs[mid].cp as u32;
        if mid_cp == cp {
            return Some(mid);
        }
        if mid_cp < cp {
            lo = mid as i32 + 1;
        } else {
            hi = mid as i32 - 1;
        }
    }
    None
}

/// Glyph index, falling back to U+FFFD then `?` like C `glyph_or_fallback`.
pub fn glyph_or_fallback(font: &Font, cp: u32) -> Option<usize> {
    find(font.glyphs, cp)
        .or_else(|| find(font.glyphs, 0xfffd))
        .or_else(|| find(font.glyphs, b'?' as u32))
}

/// Advance of glyph `gi` in 1/256 logical px.
pub fn advance_index(font: &Font, size: i32, gi: usize) -> i32 {
    if font.upm == 0 {
        return 0;
    }
    let adv = font.glyphs[gi].adv as i64;
    ((adv * size as i64 * 256 + font.upm as i64 / 2) / font.upm as i64) as i32
}

/// Horizontal advance in 1/256 logical px.
pub fn advance_q8(font: &Font, size: i32, cp: u32) -> i32 {
    match glyph_or_fallback(font, cp) {
        Some(gi) => advance_index(font, size, gi),
        None => 0,
    }
}

/// Width in logical px, rounded.
pub fn text_width(font: &Font, size: i32, s: &[u8]) -> i32 {
    let mut pos = 0usize;
    let mut q8 = 0i64;
    loop {
        let cp = next_cp(s, &mut pos);
        if cp == 0 {
            break;
        }
        q8 += advance_q8(font, size, cp) as i64;
    }
    ((q8 + 128) >> 8) as i32
}

/// Byte offset of the prefix of `s` that fits within `max_px`.
pub fn fit(font: &Font, size: i32, s: &[u8], max_px: i32) -> usize {
    let mut q8 = 0i64;
    let mut p = 0usize;
    let mut pos = 0usize;
    loop {
        let mut q = pos;
        let cp = next_cp(s, &mut q);
        if cp == 0 {
            break;
        }
        q8 += advance_q8(font, size, cp) as i64;
        if ((q8 + 128) >> 8) > max_px as i64 {
            break;
        }
        p = q;
        pos = q;
    }
    p
}

/// Byte offset of the caret position nearest `x_px`.
pub fn index_at(font: &Font, size: i32, s: &[u8], x_px: i32) -> usize {
    let mut q8 = 0i64;
    let mut p = 0usize;
    let mut pos = 0usize;
    loop {
        let mut q = pos;
        let cp = next_cp(s, &mut q);
        if cp == 0 {
            break;
        }
        let a = advance_q8(font, size, cp) as i64;
        // The caret goes before this glyph when x is in its left half.
        if q8 + a / 2 > x_px as i64 * 256 {
            break;
        }
        q8 += a;
        p = q;
        pos = q;
    }
    p
}

const POOL_BYTES: usize = 384 * 1024;
const SLOT_COUNT: usize = 2048;
const PROBES: u32 = 8;

#[derive(Clone, Copy)]
struct Slot {
    used: bool,
    face: usize,
    gi: u16,
    size_q4: u16,
    subx: u8,
    bm: FontBitmap,
}

const EMPTY_SLOT: Slot = Slot { used: false, face: 0, gi: 0, size_q4: 0, subx: 0, bm: FontBitmap::EMPTY };

/// Glyph bitmap cache (C `pool`/`slots` statics).
pub struct GlyphCache {
    pool: [u8; POOL_BYTES],
    used: usize,
    slots: [Slot; SLOT_COUNT],
}

/// Text contrast: both light-on-dark and dark-on-light read better with a mild coverage boost.
/// C `init_boost`: `cov' = 255 * (cov/255)^0.8`, approximated by a smooth integer curve.
const BOOST: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let x = (i * 256 / 255) as i32; // 0..256
        let mut y = x + ((x * (256 - x)) >> 9) * 3 / 4; // lift mid-tones by up to ~19%
        if y > 256 {
            y = 256;
        }
        t[i] = (y * 255 / 256) as u8;
        i += 1;
    }
    t
};

/// Borrowed coverage bitmap from the cache.
pub struct Bitmap<'a> {
    /// Left from the pen origin.
    pub left: i32,
    /// Top from the baseline.
    pub top: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
    /// Coverage.
    pub cov: &'a [u8],
}

enum Render {
    Bitmap(FontBitmap),
    Retry,
    Fail,
}

impl GlyphCache {
    /// Empty cache.
    pub const fn new() -> Self {
        Self { pool: [0u8; POOL_BYTES], used: 0, slots: [EMPTY_SLOT; SLOT_COUNT] }
    }

    /// Drops every cached bitmap.
    pub fn flush(&mut self) {
        self.used = 0;
        self.slots.fill(EMPTY_SLOT);
    }

    /// Returns the cache slot for `glyph` at `size_q4` / quarter-pixel phase `subx`, rendering it
    /// first if needed.
    pub fn get_slot(&mut self, font: &Font, gi: usize, size_q4: i32, subx: i32) -> Option<usize> {
        if gi >= font.glyphs.len() || font.upm == 0 || size_q4 < 0 || subx < 0 {
            return None;
        }
        let face = font.glyphs.as_ptr() as usize;
        let h = (gi as u32)
            .wrapping_mul(2_654_435_761)
            .wrapping_add((size_q4 as u32).wrapping_mul(40503))
            .wrapping_add((subx as u32).wrapping_mul(977))
            .wrapping_add(face as u32)
            % SLOT_COUNT as u32;
        loop {
            for probe in 0..PROBES {
                let idx = ((h + probe) % SLOT_COUNT as u32) as usize;
                let s = &self.slots[idx];
                if s.used {
                    if s.face == face && s.gi == gi as u16 && s.size_q4 == size_q4 as u16 && s.subx == subx as u8 {
                        return Some(idx);
                    }
                    continue;
                }
                match self.render(font, gi, size_q4, subx) {
                    Render::Bitmap(bm) => {
                        self.slots[idx] = Slot {
                            used: true,
                            face,
                            gi: gi as u16,
                            size_q4: size_q4 as u16,
                            subx: subx as u8,
                            bm,
                        };
                        return Some(idx);
                    }
                    Render::Retry => break, // the pool was flushed; probe the empty slots again
                    Render::Fail => return None,
                }
            }
            self.flush();
        }
    }

    /// Coverage bitmap of a slot returned by [`Self::get_slot`].
    pub fn bitmap_at(&self, idx: usize) -> Option<Bitmap<'_>> {
        let s = self.slots.get(idx)?;
        if !s.used {
            return None;
        }
        let n = s.bm.w.max(0) as usize * s.bm.h.max(0) as usize;
        let cov = if n == 0 || s.bm.cov.is_null() {
            &[][..]
        } else {
            // SAFETY: `cov` points at `n` bytes of this cache's pool, written by `render`.
            unsafe { core::slice::from_raw_parts(s.bm.cov, n) }
        };
        Some(Bitmap { left: s.bm.left, top: s.bm.top, w: s.bm.w, h: s.bm.h, cov })
    }

    /// Raw slot bitmap for the C ABI.
    pub fn slot_ptr(&self, idx: usize) -> *const FontBitmap {
        match self.slots.get(idx) {
            Some(s) if s.used => &s.bm,
            _ => core::ptr::null(),
        }
    }

    fn render(&mut self, font: &Font, gi: usize, size_q4: i32, subx: i32) -> Render {
        let g = font.glyphs[gi];
        let ncont = g.ncont as usize;
        if ncont > 256 {
            return Render::Fail;
        }
        let mut rel = [0u16; 256];
        let mut npts = 0usize;
        for c in 0..ncont {
            let e = match font.ends.get(g.end0 as usize + c) {
                Some(&e) => e,
                None => return Render::Fail,
            };
            rel[c] = e.wrapping_sub(g.pt0);
            npts = rel[c] as usize;
        }
        let k_q24 = ((size_q4 as i64) << 24) / (font.upm as i64 * 16);

        let pt0 = g.pt0 as usize;
        if pt0 + npts > font.on.len() || (pt0 + npts) * 2 > font.pts.len() {
            return Render::Fail;
        }
        let xy = &font.pts[2 * pt0..2 * (pt0 + npts)];
        let on = &font.on[pt0..pt0 + npts];

        // Bounding box in device px (with a 1 px margin for the filter).
        let (mut xmin, mut ymin, mut xmax, mut ymax) = (32767i32, 32767i32, -32768i32, -32768i32);
        for i in 0..npts {
            let x = xy[2 * i] as i32;
            let y = xy[2 * i + 1] as i32;
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            ymin = ymin.min(y);
            ymax = ymax.max(y);
        }
        let mut bm = FontBitmap::EMPTY;
        if npts > 0 {
            let k = k_q24 >> 8; // px per unit * 2^16
            let left = ((xmin as i64 * k) >> 16) as i32 - 1;
            let right = ((xmax as i64 * k) >> 16) as i32 + 2;
            let top = -(((ymax as i64 * k) >> 16) as i32) - 1;
            let bottom = -(((ymin as i64 * k) >> 16) as i32) + 2;
            bm.left = left;
            bm.top = top;
            bm.w = right - left + 1;
            bm.h = bottom - top + 1;
            if bm.w > RASTER_MAX_SIZE || bm.h > RASTER_MAX_SIZE || bm.w <= 0 || bm.h <= 0 {
                return Render::Fail;
            }
            let n = bm.w as usize * bm.h as usize;
            if self.used + n > POOL_BYTES {
                self.flush();
                return Render::Retry;
            }
            let ox = -left * 256 + subx * 64;
            let oy = -top * 256;
            let base = self.pool.as_mut_ptr();
            // SAFETY: `self.used + n <= POOL_BYTES`; the region is inside `pool` and not aliased.
            let cov = unsafe { core::slice::from_raw_parts_mut(base.add(self.used), n) };
            if !raster::fill(xy, on, &rel[..ncont], k_q24 as i32, ox, oy, bm.w, bm.h, cov) {
                return Render::Fail;
            }
            for c in cov.iter_mut() {
                *c = BOOST[*c as usize];
            }
            bm.cov = cov.as_ptr();
            self.used += n;
        }
        Render::Bitmap(bm)
    }
}

static mut CACHE: GlyphCache = GlyphCache::new();

/// The process-wide glyph cache.
pub(crate) fn cache() -> &'static mut GlyphCache {
    // SAFETY: the shell is single-threaded (C used the same static pool/slots), and no caller
    // holds a cache borrow across another call that could flush it.
    unsafe { &mut *core::ptr::addr_of_mut!(CACHE) }
}

/// Drops every cached glyph bitmap (C `font_cache_flush`).
pub fn cache_flush() {
    cache().flush();
}

/// C `font_bitmap_get`: returns a pointer into the cache, valid until the next flush.
pub fn bitmap_get(font: &Font, gi: usize, size_q4: i32, subx: i32) -> *const FontBitmap {
    let cache = cache();
    match cache.get_slot(font, gi, size_q4, subx) {
        Some(idx) => cache.slot_ptr(idx),
        None => core::ptr::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_data;

    fn fnv(data: &[u8]) -> u32 {
        let mut h = 2_166_136_261u32;
        for &b in data {
            h ^= b as u32;
            h = h.wrapping_mul(16_777_619);
        }
        h
    }

    #[test]
    fn utf8_matches_c() {
        let _g = crate::test_lock();
        let cases: &[(&[u8], &[(u32, u32)])] = &[
            (b"a\xffb", &[(97, 1), (0xfffd, 1), (98, 1), (0, 0)]),
            (b"\xc3", &[(0xfffd, 1), (0, 0)]),
            (b"\xe2\x80", &[(0xfffd, 1), (0xfffd, 1), (0, 0)]),
            (b"\xe2\x28\xa1", &[(0xfffd, 1), (40, 1), (0xfffd, 1), (0, 0)]),
            (b"\xed\xa0\x80", &[(0xfffd, 3), (0, 0)]),
            (b"ok", &[(111, 1), (107, 1), (0, 0)]),
        ];
        for (bytes, want) in cases {
            let mut pos = 0;
            for &(cp, n) in *want {
                let start = pos;
                assert_eq!(next_cp(bytes, &mut pos), cp);
                assert_eq!((pos - start) as u32, n);
            }
        }
    }

    #[test]
    fn widths_match_c() {
        let _g = crate::test_lock();
        let reg = font_data::regular();
        let sem = font_data::semibold();
        assert_eq!(text_width(&reg, 16, b"Hello"), 39);
        assert_eq!(text_width(&reg, 14, b"Hello"), 34);
        assert_eq!(text_width(&sem, 16, b"Hello"), 40);
        assert_eq!(text_width(&reg, 12, b"Quick Settings"), 82);
        assert_eq!(text_width(&reg, 64, "caf\u{e9}".as_bytes()), 134);
        assert_eq!(text_width(&reg, 14, "\u{2713}".as_bytes()), 12);
        assert_eq!(text_width(&reg, 14, "\u{2026}".as_bytes()), 12);
    }

    #[test]
    fn fit_and_index_round_trip() {
        let _g = crate::test_lock();
        let reg = font_data::regular();
        for max in 0..40 {
            let n = fit(&reg, 14, b"Hello, world", max);
            assert!(n <= 12);
            assert!(text_width(&reg, 14, &b"Hello, world"[..n]) <= max);
            assert_eq!(index_at(&reg, 14, b"Hello, world", text_width(&reg, 14, &b"Hello, world"[..n])), n);
        }
        assert_eq!(fit(&reg, 14, b"Hello, world", 0), 0);
        assert_eq!(fit(&reg, 14, b"Hello, world", 10), 1);
        assert_eq!(fit(&reg, 14, b"Hello, world", 39), 6);
        assert_eq!(index_at(&reg, 14, b"Hello, world", 6), 1);
        assert_eq!(index_at(&reg, 14, b"Hello, world", 0), 0);
        assert_eq!(index_at(&reg, 14, b"Hello, world", 1000), 12);
        // Multi-byte input keeps byte offsets on code point boundaries.
        let s = "caf\u{e9} \u{2026} ok".as_bytes();
        for x in 0..40 {
            let i = index_at(&reg, 14, s, x);
            assert_eq!(i, fit(&reg, 14, s, text_width(&reg, 14, &s[..i])));
            assert!(core::str::from_utf8(&s[..i]).is_ok());
        }
    }

    #[test]
    fn glyph_bitmaps_match_c() {
        let _g = crate::test_lock();
        let reg = font_data::regular();
        let h = find(reg.glyphs, b'H' as u32).unwrap();
        // Captured from the C engine before the port: (size_q4, subx) -> geometry + FNV of coverage.
        let want: &[(i32, i32, i32, i32, i32, i32, u32)] = &[
            (224, 0, 0, -11, 12, 14, 3_842_825_143),
            (256, 0, 0, -12, 13, 15, 2_778_055_580),
            (256, 3, 0, -12, 13, 15, 2_772_531_746),
            (1024, 0, 4, -47, 40, 50, 3_366_814_134),
        ];
        for &(size_q4, subx, left, top, w, h_expected, hash) in want {
            let b = bitmap_get(&reg, h, size_q4, subx);
            assert!(!b.is_null());
            // SAFETY: pointer into the static cache, valid until the next flush.
            let b = unsafe { &*b };
            assert_eq!((b.left, b.top, b.w, b.h), (left, top, w, h_expected));
            // SAFETY: pointer into the static glyph cache, valid until the next flush.
            let cov = unsafe { core::slice::from_raw_parts(b.cov, (b.w * b.h) as usize) };
            assert_eq!(fnv(cov), hash, "size_q4={size_q4} subx={subx}");
        }
    }

    #[test]
    fn cache_reuses_and_flushes() {
        let _g = crate::test_lock();
        let reg = font_data::regular();
        let g = find(reg.glyphs, b'e' as u32).unwrap();
        let first = bitmap_get(&reg, g, 256, 1);
        assert!(!first.is_null());
        assert_eq!(bitmap_get(&reg, g, 256, 1), first);
        cache_flush();
        let after = bitmap_get(&reg, g, 256, 1);
        // The slot may move but the content must be identical.
        // SAFETY: pointer into the static glyph cache, valid until the next flush.
        let a = unsafe { &*first };
        // SAFETY: pointer into the static glyph cache, valid until the next flush.
        let b = unsafe { &*after };
        assert_eq!((a.left, a.top, a.w, a.h), (b.left, b.top, b.w, b.h));
        // SAFETY: pointer into the static glyph cache, valid until the next flush.
        assert_eq!(fnv(unsafe { core::slice::from_raw_parts(a.cov, (a.w * a.h) as usize) }),
                   // SAFETY: pointer into the static glyph cache, valid until the next flush.
                   fnv(unsafe { core::slice::from_raw_parts(b.cov, (b.w * b.h) as usize) }));
    }
}

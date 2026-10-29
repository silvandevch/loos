//! Text layout and rendering (C `text.c`) over the [`Canvas`] primitives.

use crate::draw::{blend, Canvas, Rect};
use crate::font::{self, Font};

/// Body line height in logical px (C `shell_text_height`).
pub const TEXT_HEIGHT: i32 = 16;

/// Width in logical px (C `shell_text_width_ex`).
pub fn width(font: &Font, size: i32, s: &[u8]) -> i32 {
    font::text_width(font, size, s)
}

/// Byte offset of the prefix that fits within `max_px` (C `shell_text_fit`).
pub fn fit(font: &Font, size: i32, s: &[u8], max_px: i32) -> usize {
    font::fit(font, size, s, max_px)
}

/// Byte offset of the caret nearest `x` (C `shell_text_index_at`).
pub fn index_at(font: &Font, size: i32, s: &[u8], x: i32) -> usize {
    font::index_at(font, size, s, x)
}

fn is_boundary(s: &[u8], n: usize) -> bool {
    n >= s.len() || (s[n] & 0xc0) != 0x80
}

/// Copies `s` into `out`, shortened with an ellipsis to fit `max_px` (C `shell_ellipsize`).
/// The result is always NUL-terminated and valid UTF-8 when `s` is. Returns the bytes written
/// excluding the NUL.
pub fn ellipsize(out: &mut [u8], s: &[u8], max_px: i32, font: &Font, size: i32) -> usize {
    const DOTS: &[u8] = "\u{2026}".as_bytes();
    let cap = out.len();
    if cap == 0 {
        return 0;
    }
    if width(font, size, s) <= max_px {
        let mut n = s.len().min(cap - 1);
        while n > 0 && !is_boundary(s, n) {
            n -= 1;
        }
        out[..n].copy_from_slice(&s[..n]);
        out[n] = 0;
        return n;
    }
    let room = max_px - width(font, size, DOTS);
    let mut n = if room > 0 { fit(font, size, s, room) } else { 0 };
    // C: if (n + sizeof dots > cap) n = cap > sizeof dots ? cap - sizeof dots : 0.
    if n + DOTS.len() + 1 > cap {
        n = if cap > DOTS.len() + 1 { cap - DOTS.len() - 1 } else { 0 };
    }
    if n + DOTS.len() + 1 > cap {
        out[0] = 0; // buffer too small even for the ellipsis (C would overrun it)
        return 0;
    }
    while n > 0 && !is_boundary(s, n) {
        n -= 1;
    }
    out[..n].copy_from_slice(&s[..n]);
    out[n..n + DOTS.len()].copy_from_slice(DOTS);
    out[n + DOTS.len()] = 0;
    n + DOTS.len()
}

/// Draws `s` with its em-box top at `y`; returns the advance in logical px (C `fb_text_ex`).
#[allow(clippy::too_many_arguments)]
pub fn draw(c: &mut Canvas, clip: Rect, x: i32, y: i32, s: &[u8], color: u32, font: &Font, size: i32) -> i32 {
    let sc = c.sc();
    let cd = c.clip_dev(clip);
    let size_q4 = ((size as i64 * 16 * sc as i64 + 50) / 100) as i32;
    let mut pen = x as i64 * 256 * sc as i64 / 100; // device pen position, 1/256 px
    let mut logical_q8 = 0i64;
    let baseline = c.dv(y) as i64 + ((size as i64 * sc as i64 * 86 + 5000) / 10000);
    let a0 = color >> 24;
    let cache = font::cache();
    let mut pos = 0usize;
    loop {
        let cp = font::next_cp(s, &mut pos);
        if cp == 0 {
            break;
        }
        let gi = match font::glyph_or_fallback(font, cp) {
            Some(gi) => gi,
            None => continue,
        };
        let adv = font::advance_index(font, size, gi);
        if font.glyphs[gi].ncont != 0 {
            let subx = ((pen >> 6) & 3) as i32;
            if let Some(idx) = cache.get_slot(font, gi, size_q4, subx) {
                if let Some(b) = cache.bitmap_at(idx) {
                    let gx = (pen >> 8) as i32 + b.left;
                    let gy = baseline as i32 + b.top;
                    let gr = Rect::new(gx, gy, b.w, b.h).intersect(cd);
                    if !gr.is_empty() {
                        for yy in gr.y..gr.y + gr.h {
                            let so = (yy - gy) as usize * b.w as usize + (gr.x - gx) as usize;
                            let src = &b.cov[so..so + gr.w as usize];
                            let row = c.row(yy);
                            for xx in 0..gr.w as usize {
                                let cov = src[xx] as u32;
                                if cov == 0 {
                                    continue;
                                }
                                let a = if a0 == 255 { cov } else { cov * a0 / 255 };
                                let dst = &mut row[gr.x as usize + xx];
                                *dst = if a == 255 {
                                    color | 0xFF00_0000
                                } else {
                                    blend(*dst, (color & 0x00FF_FFFF) | a << 24)
                                };
                            }
                        }
                    }
                }
            }
        }
        pen += adv as i64 * sc as i64 / 100;
        logical_q8 += adv as i64;
    }
    ((logical_q8 + 128) >> 8) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_data;

    fn fnv(px: &[u32]) -> u32 {
        let mut h = 2_166_136_261u32;
        for p in px {
            for b in p.to_le_bytes() {
                h ^= b as u32;
                h = h.wrapping_mul(16_777_619);
            }
        }
        h
    }

    #[allow(clippy::too_many_arguments)]
    fn render(scale: i32, x: i32, y: i32, s: &[u8], color: u32, clip: Rect, font: &Font, size: i32) -> (i32, u32) {
        let mut px = [0u32; 400 * 80];
        let mut canvas = Canvas { px: &mut px, w: 400, h: 80, stride: 400, scale };
        let adv = draw(&mut canvas, clip, x, y, s, color, font, size);
        (adv, fnv(canvas.px))
    }

    #[test]
    fn renders_pixels_like_c() {
        let _g = crate::test_lock();
        let reg = font_data::regular();
        let sem = font_data::semibold();
        let full = Rect::new(0, 0, 400, 80);
        // (scale, x, y, string, color, clip, face, size) -> (advance, FNV of the device pixels),
        // captured from the C engine before the port.
        let cases: &[(i32, i32, i32, &[u8], u32, Rect, &Font, i32, i32, u32)] = &[
            (100, 2, 2, b"Hello, world", 0xFFFF_FFFF, full, &reg, 14, 79, 1_945_475_924),
            (100, 2, 2, b"Hello, world", 0x80FF_FFFF, full, &reg, 14, 79, 1_356_119_374),
            (100, 2, 2, b"Hello, world", 0xFFFF_FFFF, Rect::new(0, 0, 20, 20), &reg, 14, 79, 2_569_310_941),
            (125, 3, 3, b"Hello, world", 0xFFFF_FFFF, full, &reg, 14, 79, 3_668_095_256),
            (150, 3, 3, b"Quick Settings", 0xFFFF_FFFF, full, &sem, 13, 92, 492_527_590),
            (200, 3, 3, b"Quick Settings", 0xFFFF_FFFF, full, &sem, 13, 92, 252_714_537),
            (100, 2, 2, "\u{2713} caf\u{e9} \u{65e5}".as_bytes(), 0xFFFF_FFFF, full, &reg, 22, 89, 2_621_993_326),
            (100, 2, 2, b"LoOS", 0xFFFF_FFFF, full, &sem, 64, 166, 433_037_917),
        ];
        for &(scale, x, y, s, color, clip, font, size, adv, hash) in cases {
            let (got_adv, got) = render(scale, x, y, s, color, clip, font, size);
            assert_eq!(got_adv, adv, "{s:?} at {size}");
            assert_eq!(got, hash, "{s:?} at {size} scale {scale} colour {color:08x}");
        }
    }

    #[test]
    fn ellipsize_matches_c_and_is_valid_utf8() {
        let _g = crate::test_lock();
        let reg = font_data::regular();
        let mut out = [0u8; 64];
        let n = ellipsize(&mut out, b"Hello, world", 40, &reg, 14);
        assert_eq!(&out[..n], "Hell\u{2026}".as_bytes());
        let n = ellipsize(&mut out, b"Hello, world", 80, &reg, 14);
        assert_eq!(&out[..n], b"Hello, world");
        let n = ellipsize(&mut out, "caf\u{e9} \u{2026} ok".as_bytes(), 50, &reg, 14);
        assert_eq!(&out[..n], "caf\u{e9} \u{2026}".as_bytes());

        // Small buffers: C's cap-4 rule, but never split a code point or overrun.
        let mut small = [0xAAu8; 8];
        let n = ellipsize(&mut small, b"Hello, world", 40, &reg, 14);
        assert_eq!(&small[..n], "Hell\u{2026}".as_bytes());
        assert_eq!(small[n], 0);
        let mut four = [0u8; 4];
        assert_eq!(ellipsize(&mut four, b"Hello, world", 40, &reg, 14), 3);
        assert_eq!(&four[..], "\u{2026}\0".as_bytes());
        let mut three = [0u8; 3];
        assert_eq!(ellipsize(&mut three, b"Hello, world", 40, &reg, 14), 0);
        assert_eq!(three[0], 0);
        let mut none = [0u8; 0];
        assert_eq!(ellipsize(&mut none, b"x", 0, &reg, 14), 0);

        let s = "caf\u{e9} \u{2026} ok".as_bytes();
        for cap in 1..24 {
            for max in 0..90 {
                let mut out = [0xAAu8; 24];
                let n = ellipsize(&mut out[..cap], s, max, &reg, 14);
                assert!(n < cap.max(1));
                assert_eq!(out[n], 0, "cap={cap} max={max}");
                assert!(core::str::from_utf8(&out[..n]).is_ok(), "cap={cap} max={max}");
            }
        }
    }
}

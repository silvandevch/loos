//! Software rasterizer. Callers use logical coordinates; `scale` (percent) maps them to
//! device pixels so one layout renders crisply at any UI scale. Rect edges round
//! individually (adjacent rects never gap), strokes keep a constant device thickness,
//! curves are anti-aliased at device resolution.

/// Axis-aligned rectangle (C `shell_rect`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    /// Left.
    pub x: i32,
    /// Top.
    pub y: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

impl Rect {
    /// Constructor.
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    /// Zero area.
    pub fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    /// Overlap (empty when disjoint).
    pub fn intersect(self, b: Rect) -> Rect {
        let x0 = self.x.max(b.x);
        let y0 = self.y.max(b.y);
        let x1 = (self.x + self.w).min(b.x + b.w);
        let y1 = (self.y + self.h).min(b.y + b.h);
        if x1 < x0 || y1 < y0 { Rect::new(x0, y0, 0, 0) } else { Rect::new(x0, y0, x1 - x0, y1 - y0) }
    }
    /// Bounding box of both (an empty side is ignored).
    pub fn union(self, b: Rect) -> Rect {
        if self.is_empty() {
            return b;
        }
        if b.is_empty() {
            return self;
        }
        let x0 = self.x.min(b.x);
        let y0 = self.y.min(b.y);
        Rect::new(x0, y0, (self.x + self.w).max(b.x + b.w) - x0, (self.y + self.h).max(b.y + b.h) - y0)
    }
    /// Non-empty intersection.
    pub fn overlaps(self, b: Rect) -> bool {
        !self.intersect(b).is_empty()
    }
}

/// UI palette (C `shell_theme`), colors are 0xAARRGGBB.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Theme {
    pub desktop: u32,
    pub surface: u32,
    pub surface_hi: u32,
    pub border: u32,
    pub text: u32,
    pub text_dim: u32,
    pub accent: u32,
    pub accent_text: u32,
    pub danger: u32,
    pub title_inactive: u32,
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    0xFF00_0000 | r << 16 | g << 8 | b
}

impl Theme {
    /// Default dark palette.
    pub fn dark(accent: u32) -> Self {
        Theme {
            desktop: rgb(0x0B, 0x10, 0x1A), surface: rgb(0x12, 0x18, 0x24), surface_hi: rgb(0x1C, 0x25, 0x36),
            border: rgb(0x2A, 0x35, 0x4A), text: rgb(0xF3, 0xF4, 0xF6), text_dim: rgb(0x9C, 0xA3, 0xAF),
            accent: accent | 0xFF00_0000, accent_text: rgb(0xFF, 0xFF, 0xFF), danger: rgb(0xEF, 0x44, 0x44),
            title_inactive: rgb(0x16, 0x1E, 0x2C),
        }
    }
    /// Default light palette.
    pub fn light(accent: u32) -> Self {
        Theme {
            desktop: rgb(0xE5, 0xE9, 0xF0), surface: rgb(0xF7, 0xF8, 0xFA), surface_hi: rgb(0xFF, 0xFF, 0xFF),
            border: rgb(0xC9, 0xCF, 0xDA), text: rgb(0x11, 0x18, 0x27), text_dim: rgb(0x6B, 0x72, 0x80),
            accent: accent | 0xFF00_0000, accent_text: rgb(0xFF, 0xFF, 0xFF), danger: rgb(0xDC, 0x26, 0x26),
            title_inactive: rgb(0xEC, 0xEF, 0xF4),
        }
    }
    /// High-contrast palette.
    pub fn high_contrast() -> Self {
        Theme {
            desktop: rgb(0, 0, 0), surface: rgb(0, 0, 0), surface_hi: rgb(0x20, 0x20, 0x20),
            border: rgb(0xFF, 0xFF, 0xFF), text: rgb(0xFF, 0xFF, 0xFF), text_dim: rgb(0xFF, 0xFF, 0x00),
            accent: rgb(0x00, 0xFF, 0xFF), accent_text: rgb(0, 0, 0), danger: rgb(0xFF, 0x40, 0x40),
            title_inactive: rgb(0x30, 0x30, 0x30),
        }
    }
}

fn div255(v: u32) -> u32 {
    let v = v + 128;
    (v + (v >> 8)) >> 8
}

/// Src-over: `s` carries alpha.
pub fn blend(d: u32, s: u32) -> u32 {
    let a = s >> 24;
    match a {
        255 => s | 0xFF00_0000,
        0 => d,
        _ => {
            let ia = 255 - a;
            let ch = |sh: u32| div255(((s >> sh) & 255) * a + ((d >> sh) & 255) * ia);
            0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
        }
    }
}

/// Opaque mix: `t` 0 -> a, 256 -> b.
pub fn mix(a: u32, b: u32, t: i32) -> u32 {
    let t = t.clamp(0, 256) as u32;
    let ch = |sh: u32| (((a >> sh) & 255) * (256 - t) + ((b >> sh) & 255) * t) >> 8;
    0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

fn isqrt(mut v: u32) -> u32 {
    let (mut r, mut b) = (0u32, 1u32 << 30);
    while b > v {
        b >>= 2;
    }
    while b != 0 {
        if v >= r + b {
            v -= r + b;
            r = (r >> 1) + b;
        } else {
            r >>= 1;
        }
        b >>= 2;
    }
    r
}

/// 0..255 coverage of rounded rect `r` (device px, radius `rad`) at pixel (px, py).
fn rr_cov(r: Rect, rad: i32, px: i32, py: i32) -> i32 {
    if px < r.x || py < r.y || px >= r.x + r.w || py >= r.y + r.h {
        return 0;
    }
    let cx = if px < r.x + rad { r.x + rad } else if px >= r.x + r.w - rad { r.x + r.w - rad } else { return 255 };
    let cy = if py < r.y + rad { r.y + rad } else if py >= r.y + r.h - rad { r.y + r.h - rad } else { return 255 };
    let dx = px * 16 + 8 - cx * 16;
    let dy = py * 16 + 8 - cy * 16;
    let d = isqrt((dx * dx + dy * dy) as u32) as i32;
    let c = rad * 16 - d + 8;
    if c <= 0 { 0 } else if c >= 16 { 255 } else { c * 255 / 16 }
}

fn fdiv(a: i64, b: i64) -> i32 {
    let mut q = a / b;
    if a % b != 0 && ((a < 0) != (b < 0)) {
        q -= 1;
    }
    q as i32
}

/// A pixel buffer with a logical-to-device scale (C `shell_fb`).
pub struct Canvas<'a> {
    /// Pixels, row-major, `stride` per row.
    pub px: &'a mut [u32],
    /// Device width.
    pub w: i32,
    /// Device height.
    pub h: i32,
    /// Pixels per row.
    pub stride: i32,
    /// UI scale percent (0 = 100).
    pub scale: i32,
}

impl Canvas<'_> {
    pub(crate) fn sc(&self) -> i32 {
        if self.scale > 0 { self.scale } else { 100 }
    }
    /// Logical width.
    pub fn lw(&self) -> i32 {
        self.w * 100 / self.sc()
    }
    /// Logical height.
    pub fn lh(&self) -> i32 {
        self.h * 100 / self.sc()
    }
    pub(crate) fn dv(&self, v: i32) -> i32 {
        let sc = self.sc();
        if sc == 100 { v } else { fdiv(v as i64 * sc as i64 * 2 + 100, 200) }
    }
    fn drect(&self, r: Rect) -> Rect {
        if self.sc() == 100 {
            return r;
        }
        let (x0, y0, x1, y1) = (self.dv(r.x), self.dv(r.y), self.dv(r.x + r.w), self.dv(r.y + r.h));
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
    fn stroke_px(&self) -> i32 {
        ((self.sc() + 50) / 100).max(1)
    }
    pub(crate) fn clip_dev(&self, clip: Rect) -> Rect {
        Rect::new(0, 0, self.w, self.h).intersect(self.drect(clip))
    }
    pub(crate) fn row(&mut self, y: i32) -> &mut [u32] {
        let s = (y * self.stride) as usize;
        &mut self.px[s..s + self.w as usize]
    }

    fn fill_dev(&mut self, cd: Rect, r: Rect, c: u32) {
        let r = r.intersect(cd);
        if r.is_empty() {
            return;
        }
        let opaque = c >> 24 == 255;
        for y in r.y..r.y + r.h {
            let row = &mut self.row(y)[r.x as usize..(r.x + r.w) as usize];
            if opaque {
                row.fill(c);
            } else {
                row.iter_mut().for_each(|p| *p = blend(*p, c));
            }
        }
    }

    /// Solid (or translucent) rectangle.
    pub fn fill(&mut self, clip: Rect, r: Rect, c: u32) {
        let (cd, d) = (self.clip_dev(clip), self.drect(r));
        self.fill_dev(cd, d, c);
    }
    /// Horizontal line, one device stroke thick.
    pub fn hline(&mut self, clip: Rect, x: i32, y: i32, len: i32, c: u32) {
        let mut r = self.drect(Rect::new(x, y, len, 1));
        r.h = self.stroke_px();
        let cd = self.clip_dev(clip);
        self.fill_dev(cd, r, c);
    }
    /// Vertical line.
    pub fn vline(&mut self, clip: Rect, x: i32, y: i32, len: i32, c: u32) {
        let mut r = self.drect(Rect::new(x, y, 1, len));
        r.w = self.stroke_px();
        let cd = self.clip_dev(clip);
        self.fill_dev(cd, r, c);
    }
    /// 1-stroke outline.
    pub fn frame(&mut self, clip: Rect, r: Rect, c: u32) {
        if r.is_empty() {
            return;
        }
        let (d, cd, t) = (self.drect(r), self.clip_dev(clip), self.stroke_px());
        if t * 2 > d.w || t * 2 > d.h {
            return self.fill_dev(cd, d, c);
        }
        self.fill_dev(cd, Rect::new(d.x, d.y, d.w, t), c);
        self.fill_dev(cd, Rect::new(d.x, d.y + d.h - t, d.w, t), c);
        self.fill_dev(cd, Rect::new(d.x, d.y + t, t, d.h - 2 * t), c);
        self.fill_dev(cd, Rect::new(d.x + d.w - t, d.y + t, t, d.h - 2 * t), c);
    }

    fn fill_round_dev(&mut self, cd: Rect, r: Rect, mut rad: i32, c: u32) {
        if r.is_empty() {
            return;
        }
        rad = rad.min(r.w / 2).min(r.h / 2);
        if rad <= 0 {
            return self.fill_dev(cd, r, c);
        }
        let v = r.intersect(cd);
        let a0 = c >> 24;
        for y in v.y..v.y + v.h {
            let corner_row = y < r.y + rad || y >= r.y + r.h - rad;
            let row = self.row(y);
            for x in v.x..v.x + v.w {
                let cov = if corner_row && (x < r.x + rad || x >= r.x + r.w - rad) { rr_cov(r, rad, x, y) } else { 255 };
                if cov == 0 {
                    continue;
                }
                let a = if cov == 255 { a0 } else { a0 * cov as u32 / 255 };
                let p = &mut row[x as usize];
                *p = if a == 255 { c | 0xFF00_0000 } else { blend(*p, (c & 0x00FF_FFFF) | a << 24) };
            }
        }
    }
    /// Anti-aliased rounded rectangle.
    pub fn fill_round(&mut self, clip: Rect, r: Rect, rad: i32, c: u32) {
        let (cd, d, rd) = (self.clip_dev(clip), self.drect(r), self.dv(rad));
        self.fill_round_dev(cd, d, rd, c);
    }
    /// Filled circle.
    pub fn fill_circle(&mut self, clip: Rect, cx: i32, cy: i32, rad: i32, c: u32) {
        self.fill_round(clip, Rect::new(cx - rad, cy - rad, rad * 2, rad * 2), rad, c);
    }

    /// Rounded border ring: outer coverage minus inner coverage.
    pub fn stroke_round(&mut self, clip: Rect, r: Rect, rad: i32, thick: i32, c: u32) {
        let (d, cd) = (self.drect(r), self.clip_dev(clip));
        let t = (thick * self.sc() / 100).max(1);
        let ro = self.dv(rad).min(d.w / 2).min(d.h / 2);
        let inner = Rect::new(d.x + t, d.y + t, d.w - 2 * t, d.h - 2 * t);
        let ri = if ro > t { ro - t } else { 0 };
        let v = d.intersect(cd);
        let a0 = c >> 24;
        for y in v.y..v.y + v.h {
            let band_y = y < d.y + t + ro || y >= d.y + d.h - t - ro;
            let row = self.row(y);
            let mut x = v.x;
            while x < v.x + v.w {
                if !band_y && x >= d.x + t + ro && x < d.x + d.w - t - ro {
                    x = d.x + d.w - t - ro;                                   /* skip the hollow middle */
                    continue;
                }
                let ci = if inner.w > 0 && inner.h > 0 { rr_cov(inner, ri, x, y) } else { 0 };
                let cov = rr_cov(d, ro, x, y) - ci;
                if cov > 0 {
                    let a = a0 * cov as u32 / 255;
                    let p = &mut row[x as usize];
                    *p = blend(*p, (c & 0x00FF_FFFF) | a << 24);
                }
                x += 1;
            }
        }
    }

    /// Soft drop shadow around a rounded rect, never painted under the rect itself.
    pub fn shadow(&mut self, clip: Rect, r: Rect, rad: i32, blur: i32, dy: i32, c: u32) {
        let (d, cd) = (self.drect(r), self.clip_dev(clip));
        let b = self.dv(blur).max(1);
        let (off, rd) = (self.dv(dy), self.dv(rad));
        let bx = Rect::new(d.x, d.y + off, d.w, d.h);
        let area = Rect::new(bx.x - b, bx.y - b, bx.w + 2 * b, bx.h + 2 * b).intersect(cd);
        let (hx, hy) = (bx.w * 8, bx.h * 8);
        let (ccx, ccy) = (bx.x * 16 + hx, bx.y * 16 + hy);
        let a0 = (c >> 24) as i64;
        for y in area.y..area.y + area.h {
            for x in area.x..area.x + area.w {
                let qx = (x * 16 + 8 - ccx).abs() - (hx - rd * 16);
                let qy = (y * 16 + 8 - ccy).abs() - (hy - rd * 16);
                let (mx, my) = (qx.max(0), qy.max(0));
                let mut dist = if mx == 0 && my == 0 {
                    0
                } else if mx == 0 || my == 0 {
                    mx + my - rd * 16
                } else {
                    isqrt((mx * mx + my * my) as u32) as i32 - rd * 16
                };
                if dist >= b * 16 || rr_cov(d, rd, x, y) > 200 {
                    continue;
                }
                dist = dist.max(0);
                let t = (b * 16 - dist) as i64;
                let a = (a0 * t * t / ((b as i64 * 16) * (b as i64 * 16))) as u32;
                if a == 0 {
                    continue;
                }
                let p = &mut self.row(y)[x as usize];
                *p = blend(*p, (c & 0x00FF_FFFF) | a << 24);
            }
        }
    }

    /// Vertical two-color gradient.
    pub fn gradient_v(&mut self, clip: Rect, r: Rect, top: u32, bottom: u32) {
        let (d, cd) = (self.drect(r), self.clip_dev(clip));
        let v = d.intersect(cd);
        for y in v.y..v.y + v.h {
            let c = mix(top, bottom, if d.h > 1 { (y - d.y) * 256 / (d.h - 1) } else { 0 });
            self.fill_dev(cd, Rect::new(v.x, y, v.w, 1), c);
        }
    }

    /// Copies `sr` of `src` (device space) to (dx, dy); `round` + `rad` mask the bottom corners when rad > 0.
    #[allow(clippy::too_many_arguments)]
    pub fn blit(&mut self, clip: Rect, dx: i32, dy: i32, src: &[u32], sw: i32, sh: i32, sstride: i32, sr: Rect, round: Rect, rad: i32) {
        let sr = sr.intersect(Rect::new(0, 0, sw, sh));
        let vis = Rect::new(dx, dy, sr.w, sr.h).intersect(Rect::new(0, 0, self.w, self.h).intersect(clip));
        if vis.is_empty() {
            return;
        }
        for y in vis.y..vis.y + vis.h {
            let so = ((sr.y + (y - dy)) * sstride + sr.x + (vis.x - dx)) as usize;
            let s = &src[so..so + vis.w as usize];
            let corner_row = rad > 0 && y >= round.y + round.h - rad;
            let row = &mut self.row(y)[vis.x as usize..(vis.x + vis.w) as usize];
            for (i, (d, &sp)) in row.iter_mut().zip(s).enumerate() {
                let x = vis.x + i as i32;
                let mut p = sp;
                if corner_row && (x < round.x + rad || x >= round.x + round.w - rad) {
                    let cov = rr_cov(round, rad, x, y);
                    if cov < 255 {
                        p = (p & 0x00FF_FFFF) | ((p >> 24) * cov as u32 / 255) << 24;
                    }
                }
                *d = if p >> 24 == 255 { p } else { blend(*d, p) };
            }
        }
    }

    /// Radial glow fading quadratically to nothing at `radius`, ordered-dithered against banding.
    pub fn radial_glow(&mut self, clip: Rect, cx: i32, cy: i32, radius: i32, c: u32, strength: i32) {
        const BAYER: [i32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
        let cd = self.clip_dev(clip);
        let (dx0, dy0, dr) = (self.dv(cx), self.dv(cy), self.dv(radius));
        if dr < 1 {
            return;
        }
        let bx = Rect::new(dx0 - dr, dy0 - dr, 2 * dr, 2 * dr).intersect(cd);
        let r2 = dr as i64 * dr as i64;
        let inv = (256i64 << 24) / r2;
        for y in bx.y..bx.y + bx.h {
            let dy = (y - dy0) as i64;
            let rem = r2 - dy * dy;
            if rem <= 0 {
                continue;
            }
            let xr = isqrt(rem.min(0x7fff_ffff) as u32) as i32;
            let (x0, x1) = ((dx0 - xr).max(bx.x), (dx0 + xr + 1).min(bx.x + bx.w));
            let row = self.row(y);
            for x in x0..x1 {
                let dx = (x - dx0) as i64;
                let q = ((dx * dx + dy * dy) * inv) >> 24;
                if q >= 256 {
                    continue;
                }
                let a = (((256 - q) * (256 - q) * strength as i64) >> 8) as i32;
                let a8 = ((a + BAYER[((y & 3) * 4 + (x & 3)) as usize] * 16 + 8) >> 8).min(255);
                if a8 <= 0 {
                    continue;
                }
                let p = &mut row[x as usize];
                *p = blend(*p, (c & 0x00FF_FFFF) | (a8 as u32) << 24);
            }
        }
    }
}

/// Max damage rectangles before collapsing to one bounding box.
pub const DAMAGE_MAX: usize = 16;

/// Damage list (C `shell_damage`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Damage {
    /// Rectangles.
    pub r: [Rect; DAMAGE_MAX],
    /// Count.
    pub n: i32,
}

impl Damage {
    /// Bounding box of everything.
    pub fn bounds(&self) -> Rect {
        self.r[..self.n as usize].iter().fold(Rect::default(), |b, &r| b.union(r))
    }
    /// Adds `r`, merging overlapping or edge-adjacent rects; collapses when full.
    pub fn add(&mut self, mut r: Rect) {
        if r.is_empty() {
            return;
        }
        let mut i = 0;
        while i < self.n as usize {
            let o = self.r[i];
            if o.overlaps(r) || (o.x == r.x + r.w && o.y == r.y && o.h == r.h) {
                r = o.union(r);
                self.n -= 1;
                self.r[i] = self.r[self.n as usize];
                i = 0;                                                       /* merged rect may touch others */
                continue;
            }
            i += 1;
        }
        if self.n as usize == DAMAGE_MAX {
            let b = self.bounds().union(r);
            self.n = 1;
            self.r[0] = b;
            return;
        }
        self.r[self.n as usize] = r;
        self.n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rects_and_colors() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.intersect(Rect::new(5, 5, 10, 10)), Rect::new(5, 5, 5, 5));
        assert!(a.intersect(Rect::new(20, 0, 5, 5)).is_empty());
        assert_eq!(a.union(Rect::new(20, 0, 5, 5)), Rect::new(0, 0, 25, 10));
        assert_eq!(blend(0xFF00_0000, 0x80FF_FFFF), 0xFF80_8080);
        assert_eq!(mix(0xFF00_0000, 0xFFFF_FFFF, 256), 0xFFFF_FFFF);
    }

    #[test]
    fn fills_scale_and_round() {
        let mut px = [0u32; 40 * 20];
        let mut c = Canvas { px: &mut px, w: 40, h: 20, stride: 40, scale: 200 };
        let all = Rect::new(0, 0, 100, 100);
        c.fill(all, Rect::new(1, 1, 2, 2), 0xFFFF_0000);              /* logical 2x2 at 200% = 4x4 device */
        assert_eq!(c.px[2 * 40 + 2], 0xFFFF_0000);
        assert_eq!(c.px[5 * 40 + 5], 0xFFFF_0000);
        assert_eq!(c.px[6 * 40 + 6], 0);
        c.scale = 100;
        c.fill_round(all, Rect::new(20, 0, 20, 20), 10, 0xFF00_FF00);
        assert_eq!(c.px[20], 0, "corner pixel stays outside the circle");
        assert_eq!(c.px[10 * 40 + 30], 0xFF00_FF00);
    }

    #[test]
    fn damage_merges() {
        let mut d = Damage { r: [Rect::default(); DAMAGE_MAX], n: 0 };
        d.add(Rect::new(0, 0, 10, 10));
        d.add(Rect::new(5, 5, 10, 10));
        assert_eq!((d.n, d.r[0]), (1, Rect::new(0, 0, 15, 15)));
        for i in 0..20 {
            d.add(Rect::new(100 + i * 20, 0, 5, 5));
        }
        assert!(d.n as usize <= DAMAGE_MAX);
        assert_eq!(d.bounds(), Rect::new(0, 0, 485, 15));
    }
}

//! Exact-area anti-aliased scanline rasteriser for quadratic outlines (C `raster.c`).
//! Integer only and allocation-free so the kernel can use it.

/// Longest side of a bitmap (C `RASTER_MAX_SIZE`).
pub const RASTER_MAX_SIZE: i32 = 256;

const SS: i32 = 16;                     // sub-scanlines per pixel row
const MAX_EDGES: usize = 12000;
const MAX_CONTOUR: usize = 1024;
const MAX_CROSSINGS: usize = 64;

#[derive(Clone, Copy)]
struct Edge {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

const EMPTY_EDGE: Edge = Edge { x0: 0, y0: 0, x1: 0, y1: 0 };

/// Scratch buffers (C file statics): one rasterizer runs at a time in the single-threaded shell.
struct Raster {
    edges: [Edge; MAX_EDGES],
    nedge: usize,
    overflow: bool,
    x: [i32; MAX_CONTOUR],
    y: [i32; MAX_CONTOUR],
    on: [u8; MAX_CONTOUR],
    acc: [u16; RASTER_MAX_SIZE as usize],
}

static mut RASTER: Raster = Raster {
    edges: [EMPTY_EDGE; MAX_EDGES],
    nedge: 0,
    overflow: false,
    x: [0; MAX_CONTOUR],
    y: [0; MAX_CONTOUR],
    on: [0; MAX_CONTOUR],
    acc: [0; RASTER_MAX_SIZE as usize],
};

fn state() -> &'static mut Raster {
    // SAFETY: the shell is single-threaded (the C code used the same file-static buffers) and
    // `fill` never calls itself.
    unsafe { &mut *core::ptr::addr_of_mut!(RASTER) }
}

impl Raster {
    fn add_edge(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        if y0 == y1 {
            return;
        }
        if self.nedge >= MAX_EDGES {
            self.overflow = true;
            return;
        }
        self.edges[self.nedge] = Edge { x0, y0, x1, y1 };
        self.nedge += 1;
    }

    fn quad(&mut self, x0: i32, y0: i32, cx: i32, cy: i32, x1: i32, y1: i32) {
        // Segment count from the control polygon's deviation (in 1/256 px).
        let dx = x0 as i64 - 2 * cx as i64 + x1 as i64;
        let dy = y0 as i64 - 2 * cy as i64 + y1 as i64;
        let dev = dx.abs() + dy.abs();
        let mut n = 1i64;
        while n < 16 && dev / 4 > n * n * 24 {
            n += 1;
        }
        let (mut px, mut py) = (x0, y0);
        for i in 1..=n {
            let t = i * 65536 / n;
            let u = 65536 - t;
            let x = ((u * u * x0 as i64 + 2 * u * t * cx as i64 + t * t * x1 as i64) >> 32) as i32;
            let y = ((u * u * y0 as i64 + 2 * u * t * cy as i64 + t * t * y1 as i64) >> 32) as i32;
            self.add_edge(px, py, x, y);
            px = x;
            py = y;
        }
    }
}

/// Renders the path into `cov` (`w * h` bytes, cleared first). `k_q24` is pixels per unit * 2^24;
/// `(ox, oy)` is the path origin in 1/256 px from the bitmap's top-left (path y up, bitmap y down).
/// Non-zero winding. Returns false when the path is too complex or the geometry is invalid.
#[allow(clippy::too_many_arguments)]
pub fn fill(xy: &[i16], on: &[u8], ends: &[u16], k_q24: i32, ox: i32, oy: i32, w: i32, h: i32, cov: &mut [u8]) -> bool {
    if w <= 0 || h <= 0 {
        return false;
    }
    let area = w as usize * h as usize;
    if area > cov.len() {
        return false;
    }
    cov[..area].fill(0);
    if w > RASTER_MAX_SIZE || h > RASTER_MAX_SIZE {
        return false;
    }

    let st = state();
    st.nedge = 0;
    st.overflow = false;

    let mut start: i32 = 0;
    for &end in ends {
        let end = end as i32;
        let n = end - start;
        if n >= 2 && n as usize <= MAX_CONTOUR && start >= 0 {
            let n = n as usize;
            let next = start as usize + n;
            if next * 2 <= xy.len() && next <= on.len() {
                for i in 0..n {
                    let p = start as usize + i;
                    st.x[i] = (((xy[2 * p] as i64 * k_q24 as i64) >> 16) as i32).wrapping_add(ox);
                    st.y[i] = oy.wrapping_sub(((xy[2 * p + 1] as i64 * k_q24 as i64) >> 16) as i32);
                    st.on[i] = on[p];
                }
                let mut s0 = None;
                for (i, &o) in st.on[..n].iter().enumerate() {
                    if o != 0 {
                        s0 = Some(i);
                        break;
                    }
                }
                let (sx, sy, first, count);
                if let Some(s0) = s0 {
                    sx = st.x[s0];
                    sy = st.y[s0];
                    first = s0 + 1;
                    count = n - 1;
                } else {
                    // All off-curve: start at an implied point.
                    sx = st.x[0].wrapping_add(st.x[1]) / 2;
                    sy = st.y[0].wrapping_add(st.y[1]) / 2;
                    first = 1;
                    count = n;
                }
                let (mut cx, mut cy, mut qx, mut qy) = (sx, sy, 0, 0);
                let mut have_ctrl = false;
                for j in 0..=count {
                    let (x, y, is_on) = if j == count {
                        (sx, sy, true) // close the contour
                    } else {
                        let idx = (first + j) % n;
                        (st.x[idx], st.y[idx], st.on[idx] != 0)
                    };
                    if is_on {
                        if have_ctrl {
                            st.quad(cx, cy, qx, qy, x, y);
                            have_ctrl = false;
                        } else {
                            st.add_edge(cx, cy, x, y);
                        }
                        cx = x;
                        cy = y;
                    } else {
                        if have_ctrl {
                            // Implied on-curve midpoint.
                            let mx = qx.wrapping_add(x) / 2;
                            let my = qy.wrapping_add(y) / 2;
                            st.quad(cx, cy, qx, qy, mx, my);
                            cx = mx;
                            cy = my;
                        }
                        qx = x;
                        qy = y;
                        have_ctrl = true;
                    }
                }
            }
        }
        start = end;
    }
    if st.overflow {
        return false;
    }

    for row in 0..h {
        let wu = w as usize;
        st.acc[..wu].fill(0);
        for s in 0..SS {
            let ys = row * 256 + (2 * s + 1) * (256 / SS) / 2; // sub-scanline centre
            let mut xs = [0i32; MAX_CROSSINGS];
            let mut dir = [0i8; MAX_CROSSINGS];
            let mut nx = 0usize;
            let mut i = 0usize;
            while i < st.nedge && nx < MAX_CROSSINGS {
                let e = st.edges[i];
                let lo = e.y0.min(e.y1);
                let hi = e.y0.max(e.y1);
                i += 1;
                if ys < lo || ys >= hi {
                    continue;
                }
                let x = e.x0.wrapping_add(
                    ((ys.wrapping_sub(e.y0) as i64 * (e.x1 as i64 - e.x0 as i64)) / (e.y1 as i64 - e.y0 as i64)) as i32,
                );
                let mut j = nx;
                nx += 1;
                while j > 0 && xs[j - 1] > x {
                    xs[j] = xs[j - 1];
                    dir[j] = dir[j - 1];
                    j -= 1;
                }
                xs[j] = x;
                dir[j] = if e.y1 > e.y0 { 1 } else { -1 };
            }
            let mut wind = 0i32;
            let mut xa = 0i32;
            for i in 0..nx {
                let was = wind;
                wind += dir[i] as i32;
                if was == 0 && wind != 0 {
                    xa = xs[i];
                    continue;
                }
                if !(was != 0 && wind == 0) {
                    continue;
                }
                let mut xb = xs[i];
                if xa < 0 {
                    xa = 0;
                }
                let right = w * 256;
                if xb > right {
                    xb = right;
                }
                if xb <= xa {
                    continue;
                }
                let mut px = xa >> 8;
                let last = (xb - 1) >> 8;
                while px <= last {
                    let l = (px * 256).max(xa);
                    let r = ((px + 1) * 256).min(xb);
                    let idx = px as usize;
                    st.acc[idx] = st.acc[idx].wrapping_add((r - l) as u16);
                    px += 1;
                }
            }
        }
        for x in 0..w {
            let v = st.acc[x as usize] as u32 * 255 / (256 * SS) as u32;
            cov[(row * w + x) as usize] = v.min(255) as u8;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_is_solid() {
        let _g = crate::test_lock();
        // 8-unit square scaled to 8 px, origin at device (0, 8): covers y 0..8, y up.
        let xy = [0i16, 0, 8, 0, 8, 8, 0, 8];
        let on = [1u8; 4];
        let ends = [4u16];
        let k = 1i32 << 24;
        let mut cov = [0u8; 8 * 8];
        assert!(fill(&xy, &on, &ends, k, 0, 8 * 256, 8, 8, &mut cov));
        assert!(cov.iter().all(|&c| c == 255), "full square covers every pixel");
    }

    #[test]
    fn half_pixel_shift_antialiases() {
        let _g = crate::test_lock();
        let xy = [0i16, 0, 8, 0, 8, 8, 0, 8];
        let on = [1u8; 4];
        let ends = [4u16];
        let k = 1i32 << 24;
        let mut cov = [0u8; 8 * 8];
        assert!(fill(&xy, &on, &ends, k, 128, 8 * 256, 8, 8, &mut cov));
        assert!((100..=155).contains(&(cov[4 * 8] as i32)), "left column is half covered: {}", cov[4 * 8]);
        assert_eq!(cov[4 * 8 + 7], 255, "right column still solid");
    }

    #[test]
    fn rejects_oversized_and_clears() {
        let _g = crate::test_lock();
        let xy = [0i16, 0, 100, 0, 100, 100];
        let on = [1u8; 3];
        let ends = [3u16];
        let mut cov = [0xAAu8; 300];
        assert!(!fill(&xy, &on, &ends, 1 << 16, 0, 0, 300, 1, &mut cov));
        assert!(cov.iter().all(|&c| c == 0), "cleared before the size check like C");
        let mut cov = [0xAAu8; 4];
        assert!(fill(&xy, &on, &ends, 1 << 16, 0, 0, 2, 2, &mut cov));
        assert_eq!(cov[0], 0, "cleared before rasterising");
    }
}

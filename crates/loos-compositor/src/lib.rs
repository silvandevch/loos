//! Desktop compositor and D3D11/DXGI bridge.
//!
//! Window contents live in client-owned [`Surface`]s (0xAARRGGBB, straight alpha). The [`Compositor`]
//! keeps them in z-order, tracks damage, and on [`Compositor::compose`] repaints only the damaged
//! regions into a `0x00RRGGBB` target that `loos_drivers::display::Display::present` can take.
//! The [`d3d11`] module maps swap chains and textures onto surfaces so Direct3D 11 applications render
//! through the same path (software rasteriser backends write into the back buffer).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod d3d11;

use alloc::vec::Vec;

/// Axis-aligned rectangle (pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    /// True when it covers no pixels.
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    /// Overlap of two rectangles (empty when disjoint).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        if x1 > x0 && y1 > y0 { Rect::new(x0, y0, x1 - x0, y1 - y0) } else { Rect::default() }
    }
    /// Smallest rectangle containing both.
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.w).max(o.x + o.w);
        let y1 = (self.y + self.h).max(o.y + o.h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

/// Surface handle.
pub type SurfaceId = u32;

/// A client-owned pixel buffer placed on the desktop.
#[derive(Debug, Clone)]
pub struct Surface {
    /// Handle.
    pub id: SurfaceId,
    /// Desktop position of the top-left corner.
    pub x: i32,
    /// Desktop position of the top-left corner.
    pub y: i32,
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
    /// 0xAARRGGBB, straight (non-premultiplied) alpha.
    pub pixels: Vec<u32>,
    /// Whole-surface opacity 0..=255.
    pub opacity: u8,
    /// Hidden surfaces are skipped.
    pub visible: bool,
    /// Ignore the per-pixel alpha (fast path for ordinary windows).
    pub opaque: bool,
}

impl Surface {
    fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }
}

/// Compositor errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompError {
    /// No such surface.
    NoSurface,
    /// Pixel buffer length/rectangle does not match the surface.
    BadSize,
}

const CURSOR_W: i32 = 12;
const CURSOR_H: i32 = 19;
const CURSOR: [&str; CURSOR_H as usize] = [
    "1...........", "11..........", "121.........", "1221........", "12221.......", "122221......",
    "1222221.....", "12222221....", "122222221...", "1222222221..", "12222211111.", "1221221.....",
    "121.1221....", "11..1221....", "1....1221...", ".....1221...", "......121...", "......11....",
    "............",
];

/// The compositor.
pub struct Compositor {
    w: i32,
    h: i32,
    surfaces: Vec<Surface>,
    damage: Vec<Rect>,
    next_id: SurfaceId,
    /// Desktop background (0x00RRGGBB).
    pub background: u32,
    cursor: Option<(i32, i32)>,
    frame: u64,
}

fn blend(dst: u32, src: u32, a: u32) -> u32 {
    let mix = |s: u32, d: u32| (s * a + d * (255 - a) + 127) / 255;
    let r = mix((src >> 16) & 255, (dst >> 16) & 255);
    let g = mix((src >> 8) & 255, (dst >> 8) & 255);
    let b = mix(src & 255, dst & 255);
    (r << 16) | (g << 8) | b
}

impl Compositor {
    /// New compositor for a `w` x `h` output; the first compose repaints everything.
    pub fn new(w: i32, h: i32) -> Self {
        Compositor { w, h, surfaces: Vec::new(), damage: alloc::vec![Rect::new(0, 0, w, h)], next_id: 1, background: 0x0020_3040, cursor: None, frame: 0 }
    }

    /// Output size.
    pub fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    /// Frames composed so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Changes the output size (full repaint).
    pub fn resize(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.damage_all();
    }

    /// Marks the whole output dirty.
    pub fn damage_all(&mut self) {
        self.damage.clear();
        self.damage.push(Rect::new(0, 0, self.w, self.h));
    }

    /// Marks `r` (desktop coordinates) dirty.
    pub fn damage_rect(&mut self, r: Rect) {
        let r = r.intersect(&Rect::new(0, 0, self.w, self.h));
        if r.is_empty() || self.damage.iter().any(|d| d.intersect(&r) == r) {
            return;
        }
        self.damage.retain(|d| d.intersect(&r) != *d);
        self.damage.push(r);
        if self.damage.len() > 16 {
            let all = self.damage.iter().fold(Rect::default(), |a, b| a.union(b));
            self.damage.clear();
            self.damage.push(all);
        }
    }

    fn idx(&self, id: SurfaceId) -> Result<usize, CompError> {
        self.surfaces.iter().position(|s| s.id == id).ok_or(CompError::NoSurface)
    }

    /// Creates a transparent, visible surface on top of the stack.
    pub fn create_surface(&mut self, x: i32, y: i32, w: i32, h: i32) -> Result<SurfaceId, CompError> {
        if w <= 0 || h <= 0 || w > 16384 || h > 16384 {
            return Err(CompError::BadSize);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.surfaces.push(Surface { id, x, y, w, h, pixels: alloc::vec![0; (w * h) as usize], opacity: 255, visible: true, opaque: false });
        Ok(id)
    }

    /// Destroys a surface.
    pub fn destroy_surface(&mut self, id: SurfaceId) -> Result<(), CompError> {
        let i = self.idx(id)?;
        let s = self.surfaces.remove(i);
        self.damage_rect(s.bounds());
        Ok(())
    }

    /// Surface by id.
    pub fn surface(&self, id: SurfaceId) -> Option<&Surface> {
        self.surfaces.iter().find(|s| s.id == id)
    }

    /// Bottom-to-top order of surface ids.
    pub fn stack(&self) -> Vec<SurfaceId> {
        self.surfaces.iter().map(|s| s.id).collect()
    }

    /// Copies `src` (a `w`x`h` buffer for the whole surface) in and damages `region` (surface coordinates; `None` = all).
    pub fn commit(&mut self, id: SurfaceId, src: &[u32], region: Option<Rect>) -> Result<(), CompError> {
        let i = self.idx(id)?;
        let s = &mut self.surfaces[i];
        if src.len() != s.pixels.len() {
            return Err(CompError::BadSize);
        }
        let r = region.unwrap_or(Rect::new(0, 0, s.w, s.h)).intersect(&Rect::new(0, 0, s.w, s.h));
        for y in r.y..r.y + r.h {
            let a = (y * s.w + r.x) as usize;
            s.pixels[a..a + r.w as usize].copy_from_slice(&src[a..a + r.w as usize]);
        }
        let d = Rect::new(s.x + r.x, s.y + r.y, r.w, r.h);
        if s.visible {
            self.damage_rect(d);
        }
        Ok(())
    }

    /// Moves a surface, damaging old and new areas.
    pub fn move_to(&mut self, id: SurfaceId, x: i32, y: i32) -> Result<(), CompError> {
        let i = self.idx(id)?;
        let old = self.surfaces[i].bounds();
        self.surfaces[i].x = x;
        self.surfaces[i].y = y;
        let new = self.surfaces[i].bounds();
        if self.surfaces[i].visible {
            self.damage_rect(old);
            self.damage_rect(new);
        }
        Ok(())
    }

    /// Replaces a surface's buffer with a new size (contents cleared).
    pub fn resize_surface(&mut self, id: SurfaceId, w: i32, h: i32) -> Result<(), CompError> {
        if w <= 0 || h <= 0 || w > 16384 || h > 16384 {
            return Err(CompError::BadSize);
        }
        let i = self.idx(id)?;
        let old = self.surfaces[i].bounds();
        let s = &mut self.surfaces[i];
        s.w = w;
        s.h = h;
        s.pixels = alloc::vec![0; (w * h) as usize];
        let new = s.bounds();
        self.damage_rect(old);
        self.damage_rect(new);
        Ok(())
    }

    /// Sets whole-surface opacity.
    pub fn set_opacity(&mut self, id: SurfaceId, o: u8) -> Result<(), CompError> {
        let i = self.idx(id)?;
        self.surfaces[i].opacity = o;
        let b = self.surfaces[i].bounds();
        self.damage_rect(b);
        Ok(())
    }

    /// Treats the surface as opaque regardless of pixel alpha.
    pub fn set_opaque(&mut self, id: SurfaceId, opaque: bool) -> Result<(), CompError> {
        let i = self.idx(id)?;
        self.surfaces[i].opaque = opaque;
        let b = self.surfaces[i].bounds();
        self.damage_rect(b);
        Ok(())
    }

    /// Shows or hides a surface.
    pub fn set_visible(&mut self, id: SurfaceId, v: bool) -> Result<(), CompError> {
        let i = self.idx(id)?;
        self.surfaces[i].visible = v;
        let b = self.surfaces[i].bounds();
        self.damage_rect(b);
        Ok(())
    }

    /// Brings a surface to the top.
    pub fn raise(&mut self, id: SurfaceId) -> Result<(), CompError> {
        let i = self.idx(id)?;
        let s = self.surfaces.remove(i);
        let b = s.bounds();
        self.surfaces.push(s);
        self.damage_rect(b);
        Ok(())
    }

    /// Sends a surface to the bottom.
    pub fn lower(&mut self, id: SurfaceId) -> Result<(), CompError> {
        let i = self.idx(id)?;
        let s = self.surfaces.remove(i);
        let b = s.bounds();
        self.surfaces.insert(0, s);
        self.damage_rect(b);
        Ok(())
    }

    /// Shows the software cursor at `(x, y)`, or hides it with `None`.
    pub fn set_cursor(&mut self, at: Option<(i32, i32)>) {
        if let Some((x, y)) = self.cursor {
            self.damage_rect(Rect::new(x, y, CURSOR_W, CURSOR_H));
        }
        self.cursor = at;
        if let Some((x, y)) = at {
            self.damage_rect(Rect::new(x, y, CURSOR_W, CURSOR_H));
        }
    }

    /// Topmost visible surface containing the desktop point, for hit-testing.
    pub fn surface_at(&self, x: i32, y: i32) -> Option<SurfaceId> {
        self.surfaces.iter().rev().find(|s| s.visible && s.bounds().intersect(&Rect::new(x, y, 1, 1)) == Rect::new(x, y, 1, 1)).map(|s| s.id)
    }

    /// Repaints all damaged regions into `out` (`stride` pixels per row, `0x00RRGGBB`) and returns them.
    pub fn compose(&mut self, out: &mut [u32], stride: usize) -> Vec<Rect> {
        let regions = core::mem::take(&mut self.damage);
        for r in &regions {
            for y in r.y..r.y + r.h {
                let row = &mut out[y as usize * stride + r.x as usize..][..r.w as usize];
                row.fill(self.background);
            }
            for s in self.surfaces.iter().filter(|s| s.visible) {
                let o = s.bounds().intersect(r);
                if o.is_empty() {
                    continue;
                }
                for y in o.y..o.y + o.h {
                    let srow = &s.pixels[((y - s.y) * s.w + (o.x - s.x)) as usize..][..o.w as usize];
                    let drow = &mut out[y as usize * stride + o.x as usize..][..o.w as usize];
                    for (d, &p) in drow.iter_mut().zip(srow) {
                        let a = if s.opaque { 255 } else { (p >> 24) * u32::from(s.opacity) / 255 };
                        let a = if s.opaque { u32::from(s.opacity) } else { a };
                        *d = match a {
                            255 => p & 0x00ff_ffff,
                            0 => *d,
                            _ => blend(*d, p, a),
                        };
                    }
                }
            }
            if let Some((cx, cy)) = self.cursor {
                let o = Rect::new(cx, cy, CURSOR_W, CURSOR_H).intersect(r);
                for y in o.y..o.y + o.h {
                    for x in o.x..o.x + o.w {
                        match CURSOR[(y - cy) as usize].as_bytes()[(x - cx) as usize] {
                            b'1' => out[y as usize * stride + x as usize] = 0,
                            b'2' => out[y as usize * stride + x as usize] = 0x00ff_ffff,
                            _ => {}
                        }
                    }
                }
            }
        }
        self.frame += 1;
        regions
    }
}

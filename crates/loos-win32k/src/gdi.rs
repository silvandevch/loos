//! GDI: device contexts, stock/created objects, drawing into window and memory bitmaps.

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::*;
use loos_shell::draw::{Canvas, Rect as SRect};
use loos_shell::{font_data, text};

/// Stock object ids for `get_stock_object` (Win32 values).
pub const WHITE_BRUSH: i32 = 0;
/// LTGRAY_BRUSH.
pub const LTGRAY_BRUSH: i32 = 1;
/// GRAY_BRUSH.
pub const GRAY_BRUSH: i32 = 2;
/// DKGRAY_BRUSH.
pub const DKGRAY_BRUSH: i32 = 3;
/// BLACK_BRUSH.
pub const BLACK_BRUSH: i32 = 4;
/// NULL_BRUSH.
pub const NULL_BRUSH: i32 = 5;
/// WHITE_PEN.
pub const WHITE_PEN: i32 = 6;
/// BLACK_PEN.
pub const BLACK_PEN: i32 = 7;
/// NULL_PEN.
pub const NULL_PEN: i32 = 8;
/// SYSTEM_FONT.
pub const SYSTEM_FONT: i32 = 13;
/// COLOR_WINDOW.
pub const COLOR_WINDOW: i32 = 5;
/// COLOR_BTNFACE.
pub const COLOR_BTNFACE: i32 = 15;
/// COLOR_WINDOWTEXT.
pub const COLOR_WINDOWTEXT: i32 = 8;
/// COLOR_DESKTOP.
pub const COLOR_DESKTOP: i32 = 1;
/// SRCCOPY.
pub const SRCCOPY: u32 = 0x00CC_0020;
/// DrawText flags.
pub const DT_LEFT: u32 = 0;
/// DT_CENTER.
pub const DT_CENTER: u32 = 1;
/// DT_RIGHT.
pub const DT_RIGHT: u32 = 2;
/// DT_VCENTER.
pub const DT_VCENTER: u32 = 4;
/// DT_SINGLELINE.
pub const DT_SINGLELINE: u32 = 0x20;

const STOCK_BASE: u32 = 0x0100_0000;
const DEFAULT_FONT_PX: i32 = 14;

/// COLORREF (0x00BBGGRR) → 0xFFRRGGBB.
pub fn colorref_to_argb(c: u32) -> u32 {
    0xFF00_0000 | (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16 & 0xFF)
}

/// System colour table (COLORREF).
pub fn sys_color(idx: i32) -> u32 {
    match idx {
        COLOR_DESKTOP => rgb(0x3A, 0x6E, 0xA5),
        COLOR_WINDOW => rgb(255, 255, 255),
        COLOR_WINDOWTEXT => rgb(0, 0, 0),
        COLOR_BTNFACE => rgb(0xD4, 0xD0, 0xC8),
        _ => rgb(0xD4, 0xD0, 0xC8),
    }
}

struct Ds {
    brush: Option<u32>,
    pen: Option<(u32, i32)>,
    font: i32,
    tc: u32,
    bc: u32,
    opaque: bool,
    cur: Point,
    clip: Rect,
}

impl Win32k {
    pub(crate) fn init_stock(&mut self) {
        let brushes = [(WHITE_BRUSH, 0xFFFF_FFFFu32), (LTGRAY_BRUSH, 0xFFC0_C0C0), (GRAY_BRUSH, 0xFF80_8080), (DKGRAY_BRUSH, 0xFF40_4040), (BLACK_BRUSH, 0xFF00_0000)];
        for (id, c) in brushes {
            self.gdi.insert(STOCK_BASE + id as u32, Gdi::Brush { color: c, null: false });
        }
        self.gdi.insert(STOCK_BASE + NULL_BRUSH as u32, Gdi::Brush { color: 0, null: true });
        self.gdi.insert(STOCK_BASE + WHITE_PEN as u32, Gdi::Pen { color: 0xFFFF_FFFF, width: 1, null: false });
        self.gdi.insert(STOCK_BASE + BLACK_PEN as u32, Gdi::Pen { color: 0xFF00_0000, width: 1, null: false });
        self.gdi.insert(STOCK_BASE + NULL_PEN as u32, Gdi::Pen { color: 0, width: 1, null: true });
        self.gdi.insert(STOCK_BASE + SYSTEM_FONT as u32, Gdi::Font { height: DEFAULT_FONT_PX });
    }

    /// `GetStockObject`.
    pub fn get_stock_object(&self, id: i32) -> Hgdi {
        STOCK_BASE + id as u32
    }

    /// Brush for a system colour index (`GetSysColorBrush`).
    pub fn sys_color_brush(&mut self, idx: i32) -> Hgdi {
        self.create_solid_brush(sys_color(idx))
    }

    fn new_gdi(&mut self, o: Gdi) -> Hgdi {
        let h = self.next_gdi;
        self.next_gdi += 4;
        self.gdi.insert(h, o);
        h
    }

    /// `CreateSolidBrush(COLORREF)`.
    pub fn create_solid_brush(&mut self, c: u32) -> Hgdi {
        self.new_gdi(Gdi::Brush { color: colorref_to_argb(c), null: false })
    }
    /// `CreatePen(PS_SOLID, width, COLORREF)`.
    pub fn create_pen(&mut self, width: i32, c: u32) -> Hgdi {
        self.new_gdi(Gdi::Pen { color: colorref_to_argb(c), width: width.max(1), null: false })
    }
    /// `CreateFont` (height in pixels; the face is the built-in UI font).
    pub fn create_font(&mut self, height: i32) -> Hgdi {
        self.new_gdi(Gdi::Font { height: height.abs().clamp(6, 96) })
    }
    /// `CreateCompatibleBitmap`.
    pub fn create_bitmap(&mut self, w: i32, h: i32) -> Result<Hgdi, Win32kError> {
        if w <= 0 || h <= 0 || w > 16384 || h > 16384 || w as i64 * h as i64 > (64 << 20) {
            return self.fail(Win32kError::TooLarge);
        }
        Ok(self.new_gdi(Gdi::Bitmap { w, h, px: vec![0xFFFF_FFFF; (w * h) as usize] }))
    }
    /// `DeleteObject` (stock objects and selected objects are refused).
    pub fn delete_object(&mut self, h: Hgdi) -> bool {
        if h >= STOCK_BASE || self.dcs.values().any(|d| d.brush == h || d.pen == h || d.font == h || d.bitmap == h) {
            return false;
        }
        self.gdi.remove(&h).is_some()
    }

    fn new_dc(&mut self, kind: DcKind) -> Hdc {
        let bm = self.new_gdi(Gdi::Bitmap { w: 1, h: 1, px: vec![0xFFFF_FFFF] });
        let h = self.next_gdi;
        self.next_gdi += 4;
        let bitmap = if matches!(kind, DcKind::Mem) { bm } else { self.gdi.remove(&bm); 0 };
        self.dcs.insert(
            h,
            Dc {
                kind,
                brush: STOCK_BASE + WHITE_BRUSH as u32,
                pen: STOCK_BASE + BLACK_PEN as u32,
                font: STOCK_BASE + SYSTEM_FONT as u32,
                bitmap,
                text_color: 0,
                bk_color: rgb(255, 255, 255),
                opaque: true,
                cur: Point::default(),
                clip: None,
            },
        );
        h
    }

    /// `GetDC(hwnd)` — client-area DC.
    pub fn get_dc(&mut self, h: Hwnd) -> Result<Hdc, Win32kError> {
        if !self.is_window(h) {
            return self.fail(Win32kError::InvalidWindow);
        }
        Ok(self.new_dc(DcKind::Window(h)))
    }
    /// `ReleaseDC`.
    pub fn release_dc(&mut self, _h: Hwnd, dc: Hdc) -> bool {
        self.dcs.remove(&dc).is_some()
    }
    /// `CreateCompatibleDC`.
    pub fn create_compatible_dc(&mut self) -> Hdc {
        self.new_dc(DcKind::Mem)
    }
    /// `DeleteDC`.
    pub fn delete_dc(&mut self, dc: Hdc) -> bool {
        match self.dcs.remove(&dc) {
            Some(d) => {
                if matches!(d.kind, DcKind::Mem) {
                    // The default 1x1 bitmap is owned by the DC; user bitmaps stay alive.
                    if let Some(Gdi::Bitmap { w: 1, h: 1, .. }) = self.gdi.get(&d.bitmap) {
                        self.gdi.remove(&d.bitmap);
                    }
                }
                true
            }
            None => false,
        }
    }
    /// `SelectObject`; returns the previous object of that kind (0 on bad handle).
    pub fn select_object(&mut self, dc: Hdc, o: Hgdi) -> Hgdi {
        let kind = match self.gdi.get(&o) {
            Some(Gdi::Brush { .. }) => 0,
            Some(Gdi::Pen { .. }) => 1,
            Some(Gdi::Font { .. }) => 2,
            Some(Gdi::Bitmap { .. }) => 3,
            None => return 0,
        };
        let Some(d) = self.dcs.get_mut(&dc) else { return 0 };
        match kind {
            0 => core::mem::replace(&mut d.brush, o),
            1 => core::mem::replace(&mut d.pen, o),
            2 => core::mem::replace(&mut d.font, o),
            _ if matches!(d.kind, DcKind::Mem) => core::mem::replace(&mut d.bitmap, o),
            _ => 0,
        }
    }
    /// `SetTextColor`; returns previous.
    pub fn set_text_color(&mut self, dc: Hdc, c: u32) -> u32 {
        self.dcs.get_mut(&dc).map_or(0, |d| core::mem::replace(&mut d.text_color, c))
    }
    /// `SetBkColor`; returns previous.
    pub fn set_bk_color(&mut self, dc: Hdc, c: u32) -> u32 {
        self.dcs.get_mut(&dc).map_or(0, |d| core::mem::replace(&mut d.bk_color, c))
    }
    /// `SetBkMode(TRANSPARENT=1 / OPAQUE=2)`; returns previous mode.
    pub fn set_bk_mode(&mut self, dc: Hdc, mode: i32) -> i32 {
        self.dcs.get_mut(&dc).map_or(0, |d| if core::mem::replace(&mut d.opaque, mode == 2) { 2 } else { 1 })
    }
    /// `MoveToEx`.
    pub fn move_to(&mut self, dc: Hdc, x: i32, y: i32) {
        if let Some(d) = self.dcs.get_mut(&dc) {
            d.cur = Point { x, y };
        }
    }

    fn ds(&self, dc: Hdc, w: i32, h: i32) -> Option<Ds> {
        let d = self.dcs.get(&dc)?;
        let brush = match self.gdi.get(&d.brush) {
            Some(Gdi::Brush { color, null: false }) => Some(*color),
            _ => None,
        };
        let pen = match self.gdi.get(&d.pen) {
            Some(Gdi::Pen { color, width, null: false }) => Some((*color, *width)),
            _ => None,
        };
        let font = match self.gdi.get(&d.font) {
            Some(Gdi::Font { height }) => *height,
            _ => DEFAULT_FONT_PX,
        };
        let full = Rect::new(0, 0, w, h);
        Some(Ds {
            brush,
            pen,
            font,
            tc: colorref_to_argb(d.text_color),
            bc: colorref_to_argb(d.bk_color),
            opaque: d.opaque,
            cur: d.cur,
            clip: d.clip.map_or(full, |c| c.intersect(&full)),
        })
    }

    /// Run `f` against the DC's target pixels.
    fn with_surface<R>(&mut self, dc: Hdc, f: impl FnOnce(&mut Ds, &mut [u32], i32, i32) -> R) -> Option<R> {
        let (kind_win, bitmap) = match self.dcs.get(&dc) {
            Some(Dc { kind: DcKind::Window(h), .. }) => (Some(*h), 0),
            Some(Dc { bitmap, .. }) => (None, *bitmap),
            None => return None,
        };
        let (w, h) = match kind_win {
            Some(hw) => self.wins.get(&hw).map(|x| (x.cw, x.ch))?,
            None => match self.gdi.get(&bitmap) {
                Some(Gdi::Bitmap { w, h, .. }) => (*w, *h),
                _ => return None,
            },
        };
        let mut ds = self.ds(dc, w, h)?;
        let r = match kind_win {
            Some(hw) => f(&mut ds, &mut self.wins.get_mut(&hw)?.bmp, w, h),
            None => match self.gdi.get_mut(&bitmap) {
                Some(Gdi::Bitmap { px, .. }) => f(&mut ds, px, w, h),
                _ => return None,
            },
        };
        if let Some(d) = self.dcs.get_mut(&dc) {
            d.cur = ds.cur;
        }
        Some(r)
    }

    // ---------- drawing ----------

    /// `FillRect`.
    pub fn fill_rect(&mut self, dc: Hdc, r: Rect, brush: Hgdi) -> bool {
        let Some(Gdi::Brush { color, null }) = self.gdi.get(&brush) else { return false };
        let (c, null) = (*color, *null);
        self.with_surface(dc, |ds, px, w, _| {
            if !null {
                fill(px, w, ds.clip.intersect(&r), c);
            }
        })
        .is_some()
    }

    /// `Rectangle` (pen outline, brush interior).
    pub fn rectangle(&mut self, dc: Hdc, l: i32, t: i32, r: i32, b: i32) -> bool {
        self.with_surface(dc, |ds, px, w, _| {
            if let Some(bc) = ds.brush {
                fill(px, w, ds.clip.intersect(&Rect::new(l + 1, t + 1, r - 1, b - 1)), bc);
            }
            if let Some((pc, _)) = ds.pen {
                for e in [Rect::new(l, t, r, t + 1), Rect::new(l, b - 1, r, b), Rect::new(l, t, l + 1, b), Rect::new(r - 1, t, r, b)] {
                    fill(px, w, ds.clip.intersect(&e), pc);
                }
            }
        })
        .is_some()
    }

    /// `LineTo` from the current position (1px Bresenham, pen colour).
    pub fn line_to(&mut self, dc: Hdc, x: i32, y: i32) -> bool {
        self.with_surface(dc, |ds, px, w, _| {
            let Some((pc, _)) = ds.pen else {
                ds.cur = Point { x, y };
                return;
            };
            let (mut x0, mut y0) = (ds.cur.x, ds.cur.y);
            let (dx, dy) = ((x - x0).abs(), -(y - y0).abs());
            let (sx, sy) = (if x0 < x { 1 } else { -1 }, if y0 < y { 1 } else { -1 });
            let mut err = dx + dy;
            loop {
                if ds.clip.contains(Point { x: x0, y: y0 }) {
                    px[(y0 * w + x0) as usize] = pc;
                }
                if x0 == x && y0 == y {
                    break;
                }
                let e2 = 2 * err;
                if e2 >= dy {
                    err += dy;
                    x0 += sx;
                }
                if e2 <= dx {
                    err += dx;
                    y0 += sy;
                }
            }
            ds.cur = Point { x, y };
        })
        .is_some()
    }

    /// `SetPixel`.
    pub fn set_pixel(&mut self, dc: Hdc, x: i32, y: i32, c: u32) -> bool {
        self.with_surface(dc, |ds, px, w, _| {
            if ds.clip.contains(Point { x, y }) {
                px[(y * w + x) as usize] = colorref_to_argb(c);
            }
        })
        .is_some()
    }
    /// `GetPixel` (COLORREF, or `None` out of range).
    pub fn get_pixel(&mut self, dc: Hdc, x: i32, y: i32) -> Option<u32> {
        self.with_surface(dc, |_, px, w, h| {
            (x >= 0 && y >= 0 && x < w && y < h).then(|| {
                let p = px[(y * w + x) as usize];
                (p >> 16 & 0xFF) | (p & 0xFF00) | (p & 0xFF) << 16
            })
        })
        .flatten()
    }

    /// `TextOut` at (x,y) = top-left of the text box.
    pub fn text_out(&mut self, dc: Hdc, x: i32, y: i32, s: &str) -> bool {
        self.with_surface(dc, |ds, px, w, h| {
            let size = ds.font;
            let font = font_data::regular();
            if ds.opaque {
                let tw = text::width(&font, size, s.as_bytes());
                fill(px, w, ds.clip.intersect(&Rect::new(x, y, x + tw, y + size + size / 4)), ds.bc);
            }
            let mut c = Canvas { px, w, h, stride: w, scale: 0 };
            let clip = SRect::new(ds.clip.left, ds.clip.top, ds.clip.width(), ds.clip.height());
            text::draw(&mut c, clip, x, y, s.as_bytes(), ds.tc, &font, size);
        })
        .is_some()
    }

    /// `GetTextExtentPoint32`: (width, height) in pixels.
    pub fn text_extent(&mut self, dc: Hdc, s: &str) -> (i32, i32) {
        let size = self.dcs.get(&dc).and_then(|d| match self.gdi.get(&d.font) {
            Some(Gdi::Font { height }) => Some(*height),
            _ => None,
        });
        let size = size.unwrap_or(DEFAULT_FONT_PX);
        (text::width(&font_data::regular(), size, s.as_bytes()), size + size / 4)
    }

    /// `DrawText` (single-line layout: DT_LEFT/CENTER/RIGHT, DT_VCENTER).
    pub fn draw_text(&mut self, dc: Hdc, s: &str, r: Rect, flags: u32) -> i32 {
        let (tw, th) = self.text_extent(dc, s);
        let x = if flags & DT_CENTER != 0 {
            r.left + (r.width() - tw) / 2
        } else if flags & DT_RIGHT != 0 {
            r.right - tw
        } else {
            r.left
        };
        let y = if flags & DT_VCENTER != 0 { r.top + (r.height() - th) / 2 } else { r.top };
        let prev = self.dcs.get(&dc).and_then(|d| d.clip);
        self.dcs.get_mut(&dc).map(|d| d.clip = Some(prev.map_or(r, |p| p.intersect(&r))));
        self.text_out(dc, x, y, s);
        self.dcs.get_mut(&dc).map(|d| d.clip = prev);
        th
    }

    /// `BitBlt(SRCCOPY)` between DCs.
    #[allow(clippy::too_many_arguments)]
    pub fn bit_blt(&mut self, dst: Hdc, dx: i32, dy: i32, w: i32, h: i32, src: Hdc, sx: i32, sy: i32) -> bool {
        // Snapshot the source rectangle first (src and dst may alias the same bitmap).
        let Some(block) = self.with_surface(src, |_, px, sw, sh| {
            let mut v = vec![None; (w.max(0) * h.max(0)) as usize];
            for j in 0..h {
                for i in 0..w {
                    let (x, y) = (sx + i, sy + j);
                    if x >= 0 && y >= 0 && x < sw && y < sh {
                        v[(j * w + i) as usize] = Some(px[(y * sw + x) as usize]);
                    }
                }
            }
            v
        }) else {
            return false;
        };
        self.with_surface(dst, |ds, px, pw, _| {
            for j in 0..h {
                for i in 0..w {
                    let (x, y) = (dx + i, dy + j);
                    if let Some(c) = block[(j * w + i) as usize] {
                        if ds.clip.contains(Point { x, y }) {
                            px[(y * pw + x) as usize] = c;
                        }
                    }
                }
            }
        })
        .is_some()
    }

    /// Pixels of a window's client bitmap (0xAARRGGBB) — for tests/screenshots.
    pub fn window_pixels(&self, h: Hwnd) -> Option<(&[u32], i32, i32)> {
        self.wins.get(&h).map(|w| (w.bmp.as_slice(), w.cw, w.ch))
    }
}

pub(crate) fn fill(px: &mut [u32], stride: i32, r: Rect, c: u32) {
    if r.is_empty() {
        return;
    }
    for y in r.top..r.bottom {
        let row = (y * stride) as usize;
        px[row + r.left as usize..row + r.right as usize].fill(c);
    }
}

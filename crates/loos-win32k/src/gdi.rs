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

/// System colour table (COLORREF), `COLOR_SCROLLBAR`..`COLOR_MENUBAR` (0..=30).
pub fn sys_color(idx: i32) -> u32 {
    let face = rgb(0xD4, 0xD0, 0xC8);
    match idx {
        0 => rgb(0xC8, 0xC8, 0xC8),                 // SCROLLBAR
        COLOR_DESKTOP => rgb(0x3A, 0x6E, 0xA5),     // BACKGROUND
        2 => rgb(0x0A, 0x24, 0x6A),                 // ACTIVECAPTION
        3 => rgb(0x80, 0x80, 0x80),                 // INACTIVECAPTION
        4 => rgb(255, 255, 255),                    // MENU
        COLOR_WINDOW => rgb(255, 255, 255),
        6 => rgb(0, 0, 0),                          // WINDOWFRAME
        7 | COLOR_WINDOWTEXT => rgb(0, 0, 0),       // MENUTEXT, WINDOWTEXT
        9 => rgb(255, 255, 255),                    // CAPTIONTEXT
        10 | 11 => face,                            // ACTIVEBORDER, INACTIVEBORDER
        12 => rgb(0x80, 0x80, 0x80),                // APPWORKSPACE
        13 => rgb(0x0A, 0x24, 0x6A),                // HIGHLIGHT
        14 => rgb(255, 255, 255),                   // HIGHLIGHTTEXT
        COLOR_BTNFACE => face,
        16 => rgb(0x80, 0x80, 0x80),                // BTNSHADOW
        17 => rgb(0x80, 0x80, 0x80),                // GRAYTEXT
        18 => rgb(0, 0, 0),                         // BTNTEXT
        19 => rgb(0xD4, 0xD0, 0xC8),                // INACTIVECAPTIONTEXT
        20 => rgb(255, 255, 255),                   // BTNHIGHLIGHT
        21 => rgb(0x40, 0x40, 0x40),                // 3DDKSHADOW
        22 => rgb(0xD4, 0xD0, 0xC8),                // 3DLIGHT
        23 => rgb(0, 0, 0),                         // INFOTEXT
        24 => rgb(0xFF, 0xFF, 0xE1),                // INFOBK
        _ => face,
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
        if h == 0 {
            // GetDC(NULL): the screen; a memory DC serves measuring and font selection.
            return Ok(self.create_compatible_dc());
        }
        if !self.is_window(h) {
            return self.fail(Win32kError::InvalidWindow);
        }
        Ok(self.new_dc(DcKind::Window(h)))
    }
    /// The window a window DC draws into.
    pub fn dc_window(&self, dc: Hdc) -> Option<Hwnd> {
        match self.dcs.get(&dc) {
            Some(Dc { kind: DcKind::Window(h), .. }) => Some(*h),
            _ => None,
        }
    }

    /// Copies a `w` x `h` 0xAARRGGBB image into the top-left of a window's client area (OpenGL `SwapBuffers`).
    pub fn present_pixels(&mut self, h: Hwnd, w: i32, hh: i32, px: &[u32]) {
        let Some(win) = self.wins.get_mut(&h) else { return };
        let (cw, ch) = (win.cw, win.ch);
        for y in 0..hh.min(ch) {
            let n = w.min(cw) as usize;
            let (s, d) = ((y * w) as usize, (y * cw) as usize);
            if s + n <= px.len() && d + n <= win.bmp.len() {
                win.bmp[d..d + n].copy_from_slice(&px[s..s + n]);
            }
        }
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

    /// Imports a Wayland wl_shm frame into a window client surface.
    pub fn import_window_pixels(&mut self, h: Hwnd, width: i32, height: i32, pixels: &[u32]) -> bool {
        if width <= 0 || height <= 0 || pixels.len() < (width as usize).saturating_mul(height as usize) {
            return false;
        }
        let Some(w) = self.wins.get_mut(&h) else { return false };
        w.cw = width;
        w.ch = height;
        w.bmp.clear();
        w.bmp.extend_from_slice(&pixels[..(width as usize) * (height as usize)]);
        w.invalid = Some(Rect::new(0, 0, width, height));
        w.erase = false;
        true
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

fn draw_line(px: &mut [u32], stride: i32, clip: Rect, (mut x0, mut y0): (i32, i32), (x1, y1): (i32, i32), c: u32) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        if clip.contains(Point { x: x0, y: y0 }) {
            px[(y0 * stride + x0) as usize] = c;
        }
        if x0 == x1 && y0 == y1 {
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
}

impl Win32k {
    /// `Ellipse`: brush interior, 1px pen outline, inscribed in `[l,r) x [t,b)`.
    pub fn ellipse(&mut self, dc: Hdc, l: i32, t: i32, r: i32, b: i32) -> bool {
        let (bw, bh) = (i64::from(r - l), i64::from(b - t));
        if bw <= 0 || bh <= 0 {
            return false;
        }
        // Pixel (x,y) is inside when ((2x+1-(l+r))/bw)^2 + ((2y+1-(t+b))/bh)^2 <= 1.
        let inside = move |x: i32, y: i32| {
            let (dx, dy) = (i64::from(2 * x + 1 - (l + r)), i64::from(2 * y + 1 - (t + b)));
            dx * dx * bh * bh + dy * dy * bw * bw <= bw * bw * bh * bh
        };
        self.with_surface(dc, |ds, px, w, _| {
            for y in t..b {
                for x in l..r {
                    if !inside(x, y) || !ds.clip.contains(Point { x, y }) {
                        continue;
                    }
                    let edge = !inside(x - 1, y) || !inside(x + 1, y) || !inside(x, y - 1) || !inside(x, y + 1);
                    let c = if edge { ds.pen.map(|p| p.0).or(ds.brush) } else { ds.brush };
                    if let Some(c) = c {
                        px[(y * w + x) as usize] = c;
                    }
                }
            }
        })
        .is_some()
    }

    /// `Polyline`: pen segments through `pts` (the current position is unchanged).
    pub fn polyline(&mut self, dc: Hdc, pts: &[Point]) -> bool {
        self.with_surface(dc, |ds, px, w, _| {
            if let Some((pc, _)) = ds.pen {
                for s in pts.windows(2) {
                    draw_line(px, w, ds.clip, (s[0].x, s[0].y), (s[1].x, s[1].y), pc);
                }
            }
        })
        .is_some()
    }

    /// `Polygon`: even-odd brush fill plus a closed pen outline.
    pub fn polygon(&mut self, dc: Hdc, pts: &[Point]) -> bool {
        if pts.len() < 2 {
            return false;
        }
        self.with_surface(dc, |ds, px, w, _| {
            if let Some(bc) = ds.brush {
                let (ymin, ymax) = (pts.iter().map(|p| p.y).min().unwrap(), pts.iter().map(|p| p.y).max().unwrap());
                for y in ymin..ymax {
                    let mut xs: Vec<i32> = Vec::new();
                    for i in 0..pts.len() {
                        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                        if (a.y <= y && b.y > y) || (b.y <= y && a.y > y) {
                            xs.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
                        }
                    }
                    xs.sort_unstable();
                    for pair in xs.chunks_exact(2) {
                        fill(px, w, ds.clip.intersect(&Rect::new(pair[0], y, pair[1], y + 1)), bc);
                    }
                }
            }
            if let Some((pc, _)) = ds.pen {
                for i in 0..pts.len() {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    draw_line(px, w, ds.clip, (a.x, a.y), (b.x, b.y), pc);
                }
            }
        })
        .is_some()
    }

    /// `PatBlt` for PATCOPY, PATINVERT, DSTINVERT, BLACKNESS and WHITENESS.
    pub fn pat_blt(&mut self, dc: Hdc, x: i32, y: i32, w: i32, h: i32, rop: u32) -> bool {
        let r = Rect::new(x, y, x + w, y + h);
        self.with_surface(dc, |ds, px, stride, _| {
            let r = ds.clip.intersect(&r);
            if r.is_empty() {
                return true;
            }
            match rop {
                0x00F0_0021 => ds.brush.map(|c| fill(px, stride, r, c)).is_some(),
                0x0000_0042 => {
                    fill(px, stride, r, 0xFF00_0000);
                    true
                }
                0x00FF_0062 => {
                    fill(px, stride, r, 0xFFFF_FFFF);
                    true
                }
                0x0055_0009 | 0x005A_0049 => {
                    let pat = if rop == 0x005A_0049 { ds.brush.unwrap_or(0xFFFF_FFFF) & 0x00FF_FFFF } else { 0x00FF_FFFF };
                    for yy in r.top..r.bottom {
                        for xx in r.left..r.right {
                            px[(yy * stride + xx) as usize] ^= pat;
                        }
                    }
                    true
                }
                _ => false,
            }
        })
        .unwrap_or(false)
    }

    /// `FrameRect`: 1px border in `brush`.
    pub fn frame_rect(&mut self, dc: Hdc, r: Rect, brush: Hgdi) -> bool {
        let Some(Gdi::Brush { color, null: false }) = self.gdi.get(&brush) else { return false };
        let c = *color;
        self.with_surface(dc, |ds, px, w, _| {
            for e in [Rect::new(r.left, r.top, r.right, r.top + 1), Rect::new(r.left, r.bottom - 1, r.right, r.bottom), Rect::new(r.left, r.top, r.left + 1, r.bottom), Rect::new(r.right - 1, r.top, r.right, r.bottom)] {
                fill(px, w, ds.clip.intersect(&e), c);
            }
        })
        .is_some()
    }

    /// `StretchBlt` (nearest neighbour, SRCCOPY only); negative sizes mirror like Windows.
    #[allow(clippy::too_many_arguments)]
    pub fn stretch_blt(&mut self, dst: Hdc, dx: i32, dy: i32, dw: i32, dh: i32, src: Hdc, sx: i32, sy: i32, sw: i32, sh: i32) -> bool {
        if dw == 0 || dh == 0 || sw == 0 || sh == 0 || i64::from(dw.abs()) * i64::from(dh.abs()) > (64 << 20) {
            return false;
        }
        let Some(block) = self.with_surface(src, |_, px, w, h| {
            let mut v = vec![None; (sw.abs() * sh.abs()) as usize];
            for j in 0..sh.abs() {
                for i in 0..sw.abs() {
                    let (x, y) = (sx + if sw < 0 { -i - 1 } else { i }, sy + if sh < 0 { -j - 1 } else { j });
                    if x >= 0 && y >= 0 && x < w && y < h {
                        v[(j * sw.abs() + i) as usize] = Some(px[(y * w + x) as usize]);
                    }
                }
            }
            v
        }) else {
            return false;
        };
        self.with_surface(dst, |ds, px, stride, _| {
            for j in 0..dh.abs() {
                for i in 0..dw.abs() {
                    let (u, v) = (i * sw.abs() / dw.abs(), j * sh.abs() / dh.abs());
                    let (x, y) = (dx + if dw < 0 { -i - 1 } else { i }, dy + if dh < 0 { -j - 1 } else { j });
                    if let Some(c) = block[(v * sw.abs() + u) as usize] {
                        if ds.clip.contains(Point { x, y }) {
                            px[(y * stride + x) as usize] = c;
                        }
                    }
                }
            }
        })
        .is_some()
    }

    /// Size of bitmap `h`.
    pub fn bitmap_size(&self, h: Hgdi) -> Option<(i32, i32)> {
        match self.gdi.get(&h) {
            Some(Gdi::Bitmap { w, h, .. }) => Some((*w, *h)),
            _ => None,
        }
    }

    /// Pixels (0xAARRGGBB, top-down) of bitmap `h`.
    pub fn bitmap_pixels(&self, h: Hgdi) -> Option<&[u32]> {
        match self.gdi.get(&h) {
            Some(Gdi::Bitmap { px, .. }) => Some(px),
            _ => None,
        }
    }

    /// Replaces the pixels of bitmap `h` (same size required).
    pub fn set_bitmap_pixels(&mut self, h: Hgdi, src: &[u32]) -> bool {
        match self.gdi.get_mut(&h) {
            Some(Gdi::Bitmap { px, .. }) if px.len() == src.len() => {
                px.copy_from_slice(src);
                true
            }
            _ => false,
        }
    }

    /// The bitmap selected into a memory DC (0 for window DCs).
    pub fn dc_bitmap(&self, dc: Hdc) -> Hgdi {
        match self.dcs.get(&dc) {
            Some(Dc { kind: DcKind::Mem, bitmap, .. }) => *bitmap,
            _ => 0,
        }
    }

    /// Draws a `sw` x `sh` image (0xAARRGGBB, top-down) into `[dx,dx+dw) x [dy,dy+dh)` of `dc`, nearest-neighbour scaled.
    pub fn draw_pixels(&mut self, dc: Hdc, dx: i32, dy: i32, dw: i32, dh: i32, src: &[u32], sw: i32, sh: i32) -> bool {
        if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 || src.len() < (sw as usize) * (sh as usize) {
            return false;
        }
        self.with_surface(dc, |ds, px, stride, _| {
            for j in 0..dh {
                for i in 0..dw {
                    let (x, y) = (dx + i, dy + j);
                    if ds.clip.contains(Point { x, y }) {
                        px[(y * stride + x) as usize] = src[((j * sh / dh) * sw + i * sw / dw) as usize];
                    }
                }
            }
        })
        .is_some()
    }
}

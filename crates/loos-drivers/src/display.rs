//! Basic display driver: a firmware (GOP/VBE) linear framebuffer exposed as a display-class
//! surface. Format-agnostic (BGRX/RGBX 32bpp, RGB888, RGB565); the compositor renders in
//! `0x00RRGGBB` and calls [`Display::present`].

use crate::DriverError;

/// Pixel layouts the firmware hands us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32bpp, bytes B,G,R,x.
    Bgrx32,
    /// 32bpp, bytes R,G,B,x.
    Rgbx32,
    /// 24bpp, bytes B,G,R.
    Bgr24,
    /// 16bpp little-endian 5:6:5 (R in the high bits).
    Rgb565,
}

impl PixelFormat {
    /// Bytes per pixel.
    pub const fn bytes(self) -> usize {
        match self {
            PixelFormat::Bgrx32 | PixelFormat::Rgbx32 => 4,
            PixelFormat::Bgr24 => 3,
            PixelFormat::Rgb565 => 2,
        }
    }

    fn put(self, dst: &mut [u8], rgb: u32) {
        let (r, g, b) = ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        match self {
            PixelFormat::Bgrx32 => dst[..4].copy_from_slice(&[b, g, r, 0]),
            PixelFormat::Rgbx32 => dst[..4].copy_from_slice(&[r, g, b, 0]),
            PixelFormat::Bgr24 => dst[..3].copy_from_slice(&[b, g, r]),
            PixelFormat::Rgb565 => {
                let v = ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3);
                dst[..2].copy_from_slice(&v.to_le_bytes());
            }
        }
    }

    fn get(self, src: &[u8]) -> u32 {
        match self {
            PixelFormat::Bgrx32 | PixelFormat::Bgr24 => (src[2] as u32) << 16 | (src[1] as u32) << 8 | src[0] as u32,
            PixelFormat::Rgbx32 => (src[0] as u32) << 16 | (src[1] as u32) << 8 | src[2] as u32,
            PixelFormat::Rgb565 => {
                let v = u16::from_le_bytes([src[0], src[1]]) as u32;
                let (r, g, b) = ((v >> 11) & 31, (v >> 5) & 63, v & 31);
                ((r << 3 | r >> 2) << 16) | ((g << 2 | g >> 4) << 8) | (b << 3 | b >> 2)
            }
        }
    }
}

/// Geometry reported by the bootloader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FbInfo {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per scanline.
    pub pitch: u32,
    /// Pixel layout.
    pub format: PixelFormat,
}

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// 12x19 arrow cursor, 1 = black outline, 2 = white fill.
const CURSOR_W: usize = 12;
const CURSOR_H: usize = 19;
const CURSOR: [&str; CURSOR_H] = [
    "1...........", "11..........", "121.........", "1221........", "12221.......", "122221......",
    "1222221.....", "12222221....", "122222221...", "1222222221..", "12222211111.", "1221221.....",
    "121.1221....", "11..1221....", "1....1221...", ".....1221...", "......121...", "......11....",
    "............",
];

/// A framebuffer surface (`mem` is the mapped scanout memory) plus a software cursor.
pub struct Display<'a> {
    info: FbInfo,
    mem: &'a mut [u8],
    cursor_at: Option<(i32, i32)>,
    under: [u32; CURSOR_W * CURSOR_H],
}

impl<'a> Display<'a> {
    /// Validate `info` against `mem` and wrap it.
    pub fn new(info: FbInfo, mem: &'a mut [u8]) -> Result<Self, DriverError> {
        let bpp = info.format.bytes() as u32;
        if info.width == 0 || info.height == 0 || info.width > 16384 || info.height > 16384 {
            return Err(DriverError::Invalid);
        }
        if info.pitch < info.width * bpp || (info.pitch as usize) * (info.height as usize) > mem.len() {
            return Err(DriverError::Invalid);
        }
        Ok(Display { info, mem, cursor_at: None, under: [0; CURSOR_W * CURSOR_H] })
    }

    /// Surface geometry.
    pub fn info(&self) -> FbInfo {
        self.info
    }

    fn off(&self, x: i32, y: i32) -> usize {
        y as usize * self.info.pitch as usize + x as usize * self.info.format.bytes()
    }

    fn clip(&self, r: Rect) -> Option<Rect> {
        let x0 = r.x.max(0);
        let y0 = r.y.max(0);
        let x1 = r.x.saturating_add(r.w).min(self.info.width as i32);
        let y1 = r.y.saturating_add(r.h).min(self.info.height as i32);
        (x1 > x0 && y1 > y0).then_some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }

    /// Read one pixel as `0x00RRGGBB` (0 when out of range).
    pub fn pixel(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.info.width as i32 || y >= self.info.height as i32 {
            return 0;
        }
        let o = self.off(x, y);
        self.info.format.get(&self.mem[o..])
    }

    fn put(&mut self, x: i32, y: i32, rgb: u32) {
        if x < 0 || y < 0 || x >= self.info.width as i32 || y >= self.info.height as i32 {
            return;
        }
        let o = self.off(x, y);
        let f = self.info.format;
        f.put(&mut self.mem[o..], rgb);
    }

    /// Fill a rectangle (clipped) with `0x00RRGGBB`.
    pub fn fill(&mut self, r: Rect, rgb: u32) {
        let Some(r) = self.clip(r) else { return };
        let f = self.info.format;
        let bpp = f.bytes();
        let mut px = [0u8; 4];
        f.put(&mut px, rgb);
        for y in r.y..r.y + r.h {
            let o = self.off(r.x, y);
            for chunk in self.mem[o..o + r.w as usize * bpp].chunks_exact_mut(bpp) {
                chunk.copy_from_slice(&px[..bpp]);
            }
        }
    }

    /// Copy the `dst` region from a `0x00RRGGBB` backbuffer of `src_stride` pixels per row, where
    /// `src` is the whole-screen buffer (pixel (x,y) at `src[y*src_stride+x]`).
    pub fn present(&mut self, src: &[u32], src_stride: usize, dst: Rect) -> Result<(), DriverError> {
        let Some(r) = self.clip(dst) else { return Ok(()) };
        if src_stride < self.info.width as usize || src.len() < src_stride * self.info.height as usize {
            return Err(DriverError::Invalid);
        }
        let hid = self.cursor_hide();
        let f = self.info.format;
        let bpp = f.bytes();
        for y in r.y..r.y + r.h {
            let row = &src[y as usize * src_stride + r.x as usize..][..r.w as usize];
            let o = self.off(r.x, y);
            for (d, &p) in self.mem[o..o + r.w as usize * bpp].chunks_exact_mut(bpp).zip(row) {
                f.put(d, p);
            }
        }
        if hid {
            self.cursor_show();
        }
        Ok(())
    }

    fn cursor_hide(&mut self) -> bool {
        let Some((cx, cy)) = self.cursor_at else { return false };
        for j in 0..CURSOR_H {
            for i in 0..CURSOR_W {
                if CURSOR[j].as_bytes()[i] != b'.' {
                    let v = self.under[j * CURSOR_W + i];
                    self.put(cx + i as i32, cy + j as i32, v);
                }
            }
        }
        true
    }

    fn cursor_show(&mut self) {
        let Some((cx, cy)) = self.cursor_at else { return };
        for j in 0..CURSOR_H {
            for i in 0..CURSOR_W {
                let c = CURSOR[j].as_bytes()[i];
                if c != b'.' {
                    self.under[j * CURSOR_W + i] = self.pixel(cx + i as i32, cy + j as i32);
                    self.put(cx + i as i32, cy + j as i32, if c == b'1' { 0 } else { 0xffffff });
                }
            }
        }
    }

    /// Move the software cursor to `(x, y)` (hotspot = top-left), restoring the pixels beneath the old one.
    pub fn move_cursor(&mut self, x: i32, y: i32) {
        self.cursor_hide();
        self.cursor_at = Some((x, y));
        self.cursor_show();
    }

    /// Remove the cursor from the screen.
    pub fn hide_cursor(&mut self) {
        self.cursor_hide();
        self.cursor_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn disp(fmt: PixelFormat, mem: &mut [u8]) -> Display<'_> {
        let bpp = fmt.bytes() as u32;
        Display::new(FbInfo { width: 32, height: 24, pitch: 32 * bpp + 8, format: fmt }, mem).unwrap()
    }

    #[test]
    fn rejects_bad_geometry() {
        let mut m = vec![0u8; 100];
        let i = FbInfo { width: 32, height: 24, pitch: 128, format: PixelFormat::Bgrx32 };
        assert_eq!(Display::new(i, &mut m).err(), Some(DriverError::Invalid));
        let mut m = vec![0u8; 100_000];
        assert!(Display::new(FbInfo { pitch: 100, ..i }, &mut m).is_err());
    }

    #[test]
    fn fill_roundtrip_all_formats() {
        for f in [PixelFormat::Bgrx32, PixelFormat::Rgbx32, PixelFormat::Bgr24] {
            let mut m = vec![0u8; 32 * 24 * 4 + 24 * 8];
            let mut d = disp(f, &mut m);
            d.fill(Rect { x: -5, y: 2, w: 10, h: 3 }, 0x123456);
            assert_eq!(d.pixel(0, 2), 0x123456);
            assert_eq!(d.pixel(4, 4), 0x123456);
            assert_eq!(d.pixel(5, 4), 0);
            assert_eq!(d.pixel(0, 5), 0);
        }
        let mut m = vec![0u8; 32 * 24 * 2 + 24 * 8];
        let mut d = disp(PixelFormat::Rgb565, &mut m);
        d.fill(Rect { x: 0, y: 0, w: 4, h: 4 }, 0xff0000);
        assert_eq!(d.pixel(1, 1), 0xff0000);
        d.fill(Rect { x: 0, y: 0, w: 4, h: 4 }, 0x00ff00);
        assert_eq!(d.pixel(1, 1), 0x00ff00);
    }

    #[test]
    fn byte_order_bgrx_vs_rgbx() {
        let mut m = vec![0u8; 32 * 24 * 4 + 24 * 8];
        disp(PixelFormat::Bgrx32, &mut m).fill(Rect { x: 0, y: 0, w: 1, h: 1 }, 0x112233);
        assert_eq!(&m[..4], &[0x33, 0x22, 0x11, 0]);
        let mut m = vec![0u8; 32 * 24 * 4 + 24 * 8];
        disp(PixelFormat::Rgbx32, &mut m).fill(Rect { x: 0, y: 0, w: 1, h: 1 }, 0x112233);
        assert_eq!(&m[..4], &[0x11, 0x22, 0x33, 0]);
    }

    #[test]
    fn present_and_cursor_restore() {
        let mut m = vec![0u8; 32 * 24 * 4 + 24 * 8];
        let mut d = disp(PixelFormat::Bgrx32, &mut m);
        let back = vec![0x00abcdefu32; 32 * 24];
        d.present(&back, 32, Rect { x: 0, y: 0, w: 32, h: 24 }).unwrap();
        assert_eq!(d.pixel(31, 23), 0xabcdef);
        d.move_cursor(3, 3);
        assert_eq!(d.pixel(3, 3), 0); // outline
        assert_eq!(d.pixel(4, 5), 0xffffff); // fill
        d.move_cursor(10, 10);
        assert_eq!(d.pixel(3, 3), 0xabcdef); // restored
        // presenting over the cursor keeps it on top
        d.present(&back, 32, Rect { x: 0, y: 0, w: 32, h: 24 }).unwrap();
        assert_eq!(d.pixel(10, 10), 0);
        d.hide_cursor();
        assert_eq!(d.pixel(10, 10), 0xabcdef);
        assert!(d.present(&back[..10], 32, Rect { x: 0, y: 0, w: 4, h: 4 }).is_err());
    }
}

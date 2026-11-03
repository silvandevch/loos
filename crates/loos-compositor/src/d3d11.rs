//! Direct3D 11 / DXGI bridge: textures, swap chains and the immediate-context copy/clear operations a
//! software-rasterised D3D11 device needs, presenting into compositor surfaces.

use crate::{CompError, Compositor, Rect, SurfaceId};
use alloc::vec::Vec;

/// HRESULT codes used by the bridge.
pub mod hr {
    /// Success.
    pub const S_OK: u32 = 0;
    /// Window occluded; frame discarded.
    pub const DXGI_STATUS_OCCLUDED: u32 = 0x087a_0001;
    /// Invalid argument.
    pub const E_INVALIDARG: u32 = 0x8007_0057;
    /// Invalid call (wrong state).
    pub const DXGI_ERROR_INVALID_CALL: u32 = 0x887a_0001;
    /// Device lost (surface vanished).
    pub const DXGI_ERROR_DEVICE_REMOVED: u32 = 0x887a_0005;
}

/// The DXGI formats the bridge converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// DXGI_FORMAT_B8G8R8A8_UNORM.
    B8G8R8A8,
    /// DXGI_FORMAT_R8G8B8A8_UNORM.
    R8G8B8A8,
    /// DXGI_FORMAT_B8G8R8X8_UNORM (alpha ignored).
    B8G8R8X8,
}

impl Format {
    /// Converts one app-format pixel (little-endian dword as stored in memory) to internal 0xAARRGGBB.
    pub fn to_argb(self, px: u32) -> u32 {
        match self {
            Format::B8G8R8A8 => px,
            Format::B8G8R8X8 => px | 0xff00_0000,
            Format::R8G8B8A8 => (px & 0xff00_ff00) | ((px & 0xff) << 16) | ((px >> 16) & 0xff),
        }
    }
    /// Converts internal 0xAARRGGBB back to the app format.
    pub fn from_argb(self, px: u32) -> u32 {
        match self {
            Format::B8G8R8A8 => px,
            Format::B8G8R8X8 => px & 0x00ff_ffff,
            Format::R8G8B8A8 => (px & 0xff00_ff00) | ((px & 0xff) << 16) | ((px >> 16) & 0xff),
        }
    }
}

/// A 2D texture, stored internally as 0xAARRGGBB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture2D {
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
    /// App-visible format.
    pub format: Format,
    data: Vec<u32>,
}

impl Texture2D {
    /// Creates a zeroed texture.
    pub fn new(w: u32, h: u32, format: Format) -> Result<Self, u32> {
        if w == 0 || h == 0 || w > 16384 || h > 16384 {
            return Err(hr::E_INVALIDARG);
        }
        Ok(Texture2D { w, h, format, data: alloc::vec![0; (w * h) as usize] })
    }

    /// Internal pixels (0xAARRGGBB, row-major).
    pub fn pixels(&self) -> &[u32] {
        &self.data
    }

    /// One pixel in the app format.
    pub fn get(&self, x: u32, y: u32) -> Option<u32> {
        (x < self.w && y < self.h).then(|| self.format.from_argb(self.data[(y * self.w + x) as usize]))
    }
}

/// Immediate-context operations.
#[derive(Debug, Default)]
pub struct Context;

fn clamp8(f: f32) -> u32 {
    (f.clamp(0.0, 1.0) * 255.0 + 0.5) as u32
}

impl Context {
    /// ClearRenderTargetView with an RGBA float colour.
    pub fn clear_render_target(&self, t: &mut Texture2D, rgba: [f32; 4]) {
        let argb = (clamp8(rgba[3]) << 24) | (clamp8(rgba[0]) << 16) | (clamp8(rgba[1]) << 8) | clamp8(rgba[2]);
        let argb = if t.format == Format::B8G8R8X8 { argb | 0xff00_0000 } else { argb };
        t.data.fill(argb);
    }

    /// CopyResource: both textures must have identical size and format.
    pub fn copy_resource(&self, dst: &mut Texture2D, src: &Texture2D) -> Result<(), u32> {
        if (dst.w, dst.h, dst.format) != (src.w, src.h, src.format) {
            return Err(hr::E_INVALIDARG);
        }
        dst.data.copy_from_slice(&src.data);
        Ok(())
    }

    /// CopySubresourceRegion of `src_box` (clipped) to `(dx, dy)`.
    pub fn copy_subresource_region(&self, dst: &mut Texture2D, dx: u32, dy: u32, src: &Texture2D, src_box: Rect) -> Result<(), u32> {
        let b = src_box.intersect(&Rect::new(0, 0, src.w as i32, src.h as i32));
        if b != src_box || dx as i64 + i64::from(b.w) > i64::from(dst.w) || dy as i64 + i64::from(b.h) > i64::from(dst.h) {
            return Err(hr::E_INVALIDARG);
        }
        for r in 0..b.h as u32 {
            let s = ((b.y as u32 + r) * src.w + b.x as u32) as usize;
            let d = ((dy + r) * dst.w + dx) as usize;
            dst.data[d..d + b.w as usize].copy_from_slice(&src.data[s..s + b.w as usize]);
        }
        Ok(())
    }

    /// UpdateSubresource: uploads app-format rows (`pitch` pixels per row) into `region`.
    pub fn update_subresource(&self, dst: &mut Texture2D, region: Rect, src: &[u32], pitch: usize) -> Result<(), u32> {
        let b = region.intersect(&Rect::new(0, 0, dst.w as i32, dst.h as i32));
        if b != region || region.is_empty() || pitch < region.w as usize || src.len() < pitch * (region.h as usize - 1) + region.w as usize {
            return Err(hr::E_INVALIDARG);
        }
        for r in 0..b.h as usize {
            for c in 0..b.w as usize {
                dst.data[(b.y as usize + r) * dst.w as usize + b.x as usize + c] = dst.format.to_argb(src[r * pitch + c]);
            }
        }
        Ok(())
    }
}

/// Swap effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapEffect {
    /// Back buffers are discarded after Present (contents undefined).
    Discard,
    /// Back buffers rotate and keep their contents of the frame before last.
    FlipSequential,
}

/// IDXGISwapChain backed by a compositor surface.
pub struct SwapChain {
    surface: SurfaceId,
    buffers: Vec<Texture2D>,
    cur: usize,
    effect: SwapEffect,
    presented: u64,
}

impl SwapChain {
    /// Creates a swap chain with `buffer_count` back buffers and a compositor surface at `(x, y)`.
    pub fn new(comp: &mut Compositor, x: i32, y: i32, w: u32, h: u32, format: Format, buffer_count: usize, effect: SwapEffect) -> Result<Self, u32> {
        if !(1..=8).contains(&buffer_count) {
            return Err(hr::E_INVALIDARG);
        }
        let mut buffers = Vec::new();
        for _ in 0..buffer_count {
            buffers.push(Texture2D::new(w, h, format)?);
        }
        let surface = comp.create_surface(x, y, w as i32, h as i32).map_err(|_| hr::E_INVALIDARG)?;
        comp.set_opaque(surface, format == Format::B8G8R8X8).map_err(|_| hr::E_INVALIDARG)?;
        Ok(SwapChain { surface, buffers, cur: 0, effect, presented: 0 })
    }

    /// Swap effect chosen at creation.
    pub fn effect(&self) -> SwapEffect {
        self.effect
    }

    /// The compositor surface this chain presents into.
    pub fn surface(&self) -> SurfaceId {
        self.surface
    }

    /// GetBuffer(0): the current back buffer.
    pub fn back_buffer(&mut self) -> &mut Texture2D {
        &mut self.buffers[self.cur]
    }

    /// Number of successful presents.
    pub fn present_count(&self) -> u64 {
        self.presented
    }

    /// Present: copies the back buffer to the surface and advances to the next buffer.
    /// `sync_interval` 0 presents immediately; any other value waits for the compositor frame (the caller paces).
    /// Returns an HRESULT value.
    pub fn present(&mut self, comp: &mut Compositor, _sync_interval: u32) -> u32 {
        let Some(s) = comp.surface(self.surface) else { return hr::DXGI_ERROR_DEVICE_REMOVED };
        if !s.visible {
            return hr::DXGI_STATUS_OCCLUDED;
        }
        let b = &self.buffers[self.cur];
        match comp.commit(self.surface, &b.data, None) {
            Ok(()) => {}
            Err(CompError::NoSurface) => return hr::DXGI_ERROR_DEVICE_REMOVED,
            Err(_) => return hr::DXGI_ERROR_INVALID_CALL,
        }
        self.presented += 1;
        // Discard: next buffer contents are undefined; FlipSequential: it holds the frame before last.
        self.cur = (self.cur + 1) % self.buffers.len();
        hr::S_OK
    }

    /// ResizeBuffers: recreates the buffers and the surface at the new size.
    pub fn resize(&mut self, comp: &mut Compositor, w: u32, h: u32) -> u32 {
        let fmt = self.buffers[0].format;
        let mut nb = Vec::new();
        for _ in 0..self.buffers.len() {
            match Texture2D::new(w, h, fmt) {
                Ok(t) => nb.push(t),
                Err(e) => return e,
            }
        }
        if comp.resize_surface(self.surface, w as i32, h as i32).is_err() {
            return hr::DXGI_ERROR_INVALID_CALL;
        }
        self.buffers = nb;
        self.cur = 0;
        hr::S_OK
    }

    /// Releases the swap chain and its surface.
    pub fn destroy(self, comp: &mut Compositor) {
        let _ = comp.destroy_surface(self.surface);
    }
}

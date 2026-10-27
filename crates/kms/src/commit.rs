//! Atomic commit validation (`DRM_MODE_ATOMIC_TEST_ONLY` equivalent).

use crate::mode::Mode;
use alloc::vec::Vec;

/// Pixel formats the scanout engine supports (fourcc-style).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// 32-bit, x ignored.
    Xrgb8888,
    /// 32-bit with alpha.
    Argb8888,
    /// 16-bit 5:6:5.
    Rgb565,
    /// 32-bit 10-bit channels.
    Xrgb2101010,
}

impl Format {
    /// Bytes per pixel.
    pub fn bpp(self) -> u32 {
        match self {
            Format::Rgb565 => 2,
            _ => 4,
        }
    }
}

/// A scanout buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fb {
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
    /// Bytes per row.
    pub pitch: u32,
    /// Pixel format.
    pub format: Format,
    /// Backing allocation size in bytes.
    pub size: u64,
}

/// Rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// X.
    pub x: i32,
    /// Y.
    pub y: i32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

/// Primary-plane state for one CRTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrtcState {
    /// CRTC index.
    pub crtc: u8,
    /// Mode (None = disabled).
    pub mode: Option<Mode>,
    /// Buffer shown.
    pub fb: Option<Fb>,
    /// Source rect within the buffer.
    pub src: Rect,
    /// Destination rect on the display.
    pub dst: Rect,
    /// Connector index driven.
    pub connector: u8,
}

/// Hardware limits.
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    /// CRTC count.
    pub crtcs: u8,
    /// Max buffer dimension.
    pub max_dim: u32,
    /// Pitch alignment in bytes (power of two).
    pub pitch_align: u32,
    /// Total pixel rate the memory system sustains (pixels/s, all CRTCs).
    pub max_pixel_rate: u64,
    /// Plane scaling allowed (src != dst size)?
    pub scaling: bool,
    /// Per-connector possible CRTC masks.
    pub possible_crtcs: &'static [u8],
}

/// Why a commit is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// CRTC index out of range.
    NoSuchCrtc(u8),
    /// Connector cannot drive that CRTC, or two states use one CRTC / connector.
    Routing(u8),
    /// Mode invalid.
    BadMode(u8),
    /// Enabled without a framebuffer, or fb inconsistent.
    BadFb(u8),
    /// Pitch too small/misaligned or buffer too small for pitch*h.
    BadPitch(u8),
    /// Source rect outside the buffer, or destination outside the mode.
    BadRect(u8),
    /// Scaling requested but unsupported.
    NoScaling(u8),
    /// Combined pixel rate over budget.
    Bandwidth,
}

/// Validate a full atomic state.
pub fn check(caps: &Caps, states: &[CrtcState]) -> Result<(), Reject> {
    let mut used_crtc = 0u32;
    let mut used_conn = 0u32;
    let mut rate = 0u64;
    for s in states {
        if s.crtc >= caps.crtcs {
            return Err(Reject::NoSuchCrtc(s.crtc));
        }
        if used_crtc >> s.crtc & 1 == 1 || used_conn >> s.connector.min(31) & 1 == 1 {
            return Err(Reject::Routing(s.crtc));
        }
        used_crtc |= 1 << s.crtc;
        used_conn |= 1 << s.connector.min(31);
        let Some(mode) = s.mode else { continue };
        let allowed = caps.possible_crtcs.get(s.connector as usize).copied().unwrap_or(0);
        if allowed >> s.crtc & 1 == 0 {
            return Err(Reject::Routing(s.crtc));
        }
        if !mode.valid() {
            return Err(Reject::BadMode(s.crtc));
        }
        let fb = s.fb.ok_or(Reject::BadFb(s.crtc))?;
        if fb.w == 0 || fb.h == 0 || fb.w > caps.max_dim || fb.h > caps.max_dim {
            return Err(Reject::BadFb(s.crtc));
        }
        let min_pitch = fb.w.checked_mul(fb.format.bpp()).ok_or(Reject::BadPitch(s.crtc))?;
        if fb.pitch < min_pitch || fb.pitch % caps.pitch_align != 0 || (fb.pitch as u64) * (fb.h as u64) > fb.size {
            return Err(Reject::BadPitch(s.crtc));
        }
        let (sx, sy) = (s.src.x as i64, s.src.y as i64);
        if s.src.x < 0 || s.src.y < 0 || s.src.w == 0 || s.src.h == 0 || sx + s.src.w as i64 > fb.w as i64 || sy + s.src.h as i64 > fb.h as i64 {
            return Err(Reject::BadRect(s.crtc));
        }
        let (dx, dy) = (s.dst.x as i64, s.dst.y as i64);
        if s.dst.w == 0 || s.dst.h == 0 || dx < 0 || dy < 0 || dx + s.dst.w as i64 > mode.hdisplay as i64 || dy + s.dst.h as i64 > mode.vdisplay as i64 {
            return Err(Reject::BadRect(s.crtc));
        }
        if (s.src.w, s.src.h) != (s.dst.w, s.dst.h) && !caps.scaling {
            return Err(Reject::NoScaling(s.crtc));
        }
        rate += mode.pixel_rate();
    }
    if rate > caps.max_pixel_rate {
        return Err(Reject::Bandwidth);
    }
    Ok(())
}

/// Assign CRTCs to connectors (greedy bipartite matching with augmenting paths) honoring
/// `possible_crtcs`. Returns `crtc` per connector, or `None` for a connector left dark.
pub fn assign_crtcs(caps: &Caps, connected: &[u8]) -> Vec<Option<u8>> {
    let n = connected.len();
    let mut owner: Vec<Option<usize>> = alloc::vec![None; caps.crtcs as usize];
    fn try_assign(i: usize, connected: &[u8], caps: &Caps, owner: &mut Vec<Option<usize>>, seen: &mut Vec<bool>) -> bool {
        let mask = caps.possible_crtcs.get(connected[i] as usize).copied().unwrap_or(0);
        for c in 0..caps.crtcs as usize {
            if mask >> c & 1 == 0 || seen[c] {
                continue;
            }
            seen[c] = true;
            if owner[c].is_none() || try_assign(owner[c].unwrap(), connected, caps, owner, seen) {
                owner[c] = Some(i);
                return true;
            }
        }
        false
    }
    for i in 0..n {
        let mut seen = alloc::vec![false; caps.crtcs as usize];
        try_assign(i, connected, caps, &mut owner, &mut seen);
    }
    let mut out = alloc::vec![None; n];
    for (c, o) in owner.iter().enumerate() {
        if let Some(i) = o {
            out[*i] = Some(c as u8);
        }
    }
    out
}

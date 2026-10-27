//! Display modes.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Sync polarity / scan flags (DRM_MODE_FLAG_*).
pub mod flags {
    /// Positive hsync.
    pub const PHSYNC: u32 = 1 << 0;
    /// Negative hsync.
    pub const NHSYNC: u32 = 1 << 1;
    /// Positive vsync.
    pub const PVSYNC: u32 = 1 << 2;
    /// Negative vsync.
    pub const NVSYNC: u32 = 1 << 3;
    /// Interlaced.
    pub const INTERLACE: u32 = 1 << 4;
}

/// A mode, same fields as `drm_mode_modeinfo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    /// Pixel clock, kHz.
    pub clock: u32,
    /// Active width.
    pub hdisplay: u16,
    /// Hsync start.
    pub hsync_start: u16,
    /// Hsync end.
    pub hsync_end: u16,
    /// Total width.
    pub htotal: u16,
    /// Active height.
    pub vdisplay: u16,
    /// Vsync start.
    pub vsync_start: u16,
    /// Vsync end.
    pub vsync_end: u16,
    /// Total height.
    pub vtotal: u16,
    /// `flags::*`.
    pub flags: u32,
    /// Preferred by the display.
    pub preferred: bool,
}

impl Mode {
    /// Refresh rate in millihertz (rounded), accounting for interlace.
    pub fn refresh_mhz(&self) -> u32 {
        let total = self.htotal as u64 * self.vtotal as u64;
        if total == 0 {
            return 0;
        }
        let mut r = (self.clock as u64 * 1_000_000 + total / 2) / total;
        if self.flags & flags::INTERLACE != 0 {
            r = r * 2 / 2; // vtotal already counts a frame's two fields' lines once; refresh is the field rate
        }
        r as u32
    }

    /// `1920x1080` / `1920x1080i`.
    pub fn name(&self) -> String {
        format!("{}x{}{}", self.hdisplay, self.vdisplay, if self.flags & flags::INTERLACE != 0 { "i" } else { "" })
    }

    /// Sanity: monotonic timings and a nonzero clock.
    pub fn valid(&self) -> bool {
        self.clock > 0
            && self.hdisplay > 0
            && self.vdisplay > 0
            && self.hdisplay <= self.hsync_start
            && self.hsync_start < self.hsync_end
            && self.hsync_end <= self.htotal
            && self.vdisplay <= self.vsync_start
            && self.vsync_start < self.vsync_end
            && self.vsync_end <= self.vtotal
    }

    /// Bandwidth in pixels per second.
    pub fn pixel_rate(&self) -> u64 {
        self.clock as u64 * 1000
    }
}

fn vsync_lines(h: u32, v: u32) -> u32 {
    // CVT aspect-ratio table
    match (h * 9 / v.max(1)).max(1) {
        _ if h * 3 == v * 4 => 4,
        _ if h * 9 == v * 16 => 5,
        _ if h * 10 == v * 16 => 6,
        _ if h * 4 == v * 5 => 7,
        _ if h * 9 == v * 15 => 7,
        _ => 10,
    }
}

/// CVT 1.2 reduced-blanking (v1) timing for `h`x`v`@`refresh_hz`. `None` for absurd inputs.
pub fn cvt_rb(h: u32, v: u32, refresh_hz: u32) -> Option<Mode> {
    if !(160..=16384).contains(&h) || !(120..=16384).contains(&v) || !(24..=240).contains(&refresh_hz) {
        return None;
    }
    let h = h & !7; // cell granularity 8
    let vsync = vsync_lines(h, v);
    // estimated line period (us*1000 for integer math): ((1e6/refresh) - 460) / v
    let frame_us_x1000 = 1_000_000_000u64 / refresh_hz as u64; // us * 1000
    let period_x1000 = (frame_us_x1000.saturating_sub(460_000)) / v as u64;
    if period_x1000 == 0 {
        return None;
    }
    let vbi = (460_000 / period_x1000 + 1).max(3 + vsync as u64 + 6) as u32;
    let vtotal = v + vbi;
    let htotal = h + 160;
    let clock_khz = (htotal as u64 * vtotal as u64 * refresh_hz as u64 / 1000) as u32;
    let clock = clock_khz / 250 * 250;
    let back = vbi - 3 - vsync;
    let _ = back;
    Some(Mode {
        clock,
        hdisplay: h as u16,
        hsync_start: (h + 48) as u16,
        hsync_end: (h + 48 + 32) as u16,
        htotal: htotal as u16,
        vdisplay: v as u16,
        vsync_start: (v + 3) as u16,
        vsync_end: (v + 3 + vsync) as u16,
        vtotal: vtotal as u16,
        flags: flags::PHSYNC | flags::NVSYNC,
        preferred: false,
    })
}

/// How the user wants a connector's mode chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// The display's preferred mode, else the largest.
    Preferred,
    /// Exact size; best refresh at or below `max_hz` (0 = no cap).
    Size {
        /// Width.
        w: u16,
        /// Height.
        h: u16,
        /// Refresh cap in Hz.
        max_hz: u32,
    },
}

/// Pick a mode from a connector's list. Interlaced modes are only used when nothing else fits.
pub fn pick(modes: &[Mode], req: Request) -> Option<Mode> {
    let best_of = |it: Vec<&Mode>| -> Option<Mode> {
        it.into_iter()
            .max_by_key(|m| (m.flags & flags::INTERLACE == 0, m.refresh_mhz(), m.hdisplay as u32 * m.vdisplay as u32))
            .copied()
    };
    match req {
        Request::Preferred => modes
            .iter()
            .find(|m| m.preferred && m.valid())
            .copied()
            .or_else(|| modes.iter().filter(|m| m.valid() && m.flags & flags::INTERLACE == 0).max_by_key(|m| (m.hdisplay as u32 * m.vdisplay as u32, m.refresh_mhz())).copied()),
        Request::Size { w, h, max_hz } => {
            let fit: Vec<&Mode> = modes.iter().filter(|m| m.valid() && m.hdisplay == w && m.vdisplay == h && (max_hz == 0 || m.refresh_mhz() <= max_hz * 1000 + 500)).collect();
            best_of(fit)
        }
    }
}

/// Sort in DRM's preferred order: preferred first, then area, then refresh, descending; drop duplicates.
pub fn sort_and_dedup(modes: &mut Vec<Mode>) {
    modes.sort_by_key(|m| core::cmp::Reverse((m.preferred, m.hdisplay as u32 * m.vdisplay as u32, m.refresh_mhz() / 1000)));
    modes.dedup_by(|a, b| a.hdisplay == b.hdisplay && a.vdisplay == b.vdisplay && a.refresh_mhz() / 1000 == b.refresh_mhz() / 1000 && (a.flags & flags::INTERLACE) == (b.flags & flags::INTERLACE));
}

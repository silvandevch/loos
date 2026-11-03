//! Theme engine (C `theme.c`): specs, presets, HSL, contrast guard, persistence and the text format.
#![allow(missing_docs)]

use crate::backend::{cfg_bool, cfg_int, cfg_set_bool, cfg_set_int, ShellBackend};
use crate::compositor::{Compositor, Style};
use crate::draw::{mix, Theme};
use core::ffi::c_char;

pub const TB_DARK: i32 = 0;
pub const TB_LIGHT: i32 = 1;
pub const TB_HC: i32 = 2;
pub const TB_AMOLED: i32 = 3;
pub const TB_NORD: i32 = 4;
pub const TB_DRACULA: i32 = 5;
pub const TB_SOLARIZED: i32 = 6;
pub const TB_GRUVBOX: i32 = 7;
pub const TB_PAPER: i32 = 8;
pub const TB_COUNT: usize = 9;

pub const TK_DESKTOP: usize = 0;
pub const TK_SURFACE: usize = 1;
pub const TK_SURFACE_HI: usize = 2;
pub const TK_BORDER: usize = 3;
pub const TK_TEXT: usize = 4;
pub const TK_TEXT_DIM: usize = 5;
pub const TK_ACCENT: usize = 6;
pub const TK_ACCENT_TEXT: usize = 7;
pub const TK_DANGER: usize = 8;
pub const TK_TITLE_INACTIVE: usize = 9;
pub const TK_COUNT: usize = 10;

/// C `theme_spec`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ThemeSpec {
    pub base: i32,
    pub accent: u32,
    pub tint: i32,
    pub radius: i32,
    pub title_h: i32,
    pub border: i32,
    pub shadow: i32,
    pub glow: i32,
    pub guard: bool,
    pub opacity: i32,
    pub icons: i32,
    pub wallpaper: i32,
    pub ovr: [u32; TK_COUNT],
    pub ovr_mask: u16,
}

impl ThemeSpec {
    pub const EMPTY: Self = Self {
        base: 0,
        accent: 0,
        tint: 0,
        radius: 0,
        title_h: 0,
        border: 0,
        shadow: 0,
        glow: 0,
        guard: false,
        opacity: 0,
        icons: 0,
        wallpaper: 0,
        ovr: [0; TK_COUNT],
        ovr_mask: 0,
    };
}

#[repr(transparent)]
pub struct CStrPtr(pub *const c_char);
// SAFETY: the pointed-to strings are immutable static data.
unsafe impl Sync for CStrPtr {}

pub const BASE_NAMES: [&[u8]; TB_COUNT] = [b"dark\0", b"light\0", b"hc\0", b"amoled\0", b"nord\0", b"dracula\0", b"solarized\0", b"gruvbox\0", b"paper\0"];
pub const TOKEN_NAMES: [&[u8]; TK_COUNT] = [
    b"desktop\0",
    b"surface\0",
    b"surface_hi\0",
    b"border\0",
    b"text\0",
    b"text_dim\0",
    b"accent\0",
    b"accent_text\0",
    b"danger\0",
    b"title_inactive\0",
];

#[no_mangle]
pub static theme_base_name: [CStrPtr; TB_COUNT] = [
    CStrPtr(b"dark\0".as_ptr() as *const c_char),
    CStrPtr(b"light\0".as_ptr() as *const c_char),
    CStrPtr(b"hc\0".as_ptr() as *const c_char),
    CStrPtr(b"amoled\0".as_ptr() as *const c_char),
    CStrPtr(b"nord\0".as_ptr() as *const c_char),
    CStrPtr(b"dracula\0".as_ptr() as *const c_char),
    CStrPtr(b"solarized\0".as_ptr() as *const c_char),
    CStrPtr(b"gruvbox\0".as_ptr() as *const c_char),
    CStrPtr(b"paper\0".as_ptr() as *const c_char),
];
#[no_mangle]
pub static theme_token_name: [CStrPtr; TK_COUNT] = [
    CStrPtr(b"desktop\0".as_ptr() as *const c_char),
    CStrPtr(b"surface\0".as_ptr() as *const c_char),
    CStrPtr(b"surface_hi\0".as_ptr() as *const c_char),
    CStrPtr(b"border\0".as_ptr() as *const c_char),
    CStrPtr(b"text\0".as_ptr() as *const c_char),
    CStrPtr(b"text_dim\0".as_ptr() as *const c_char),
    CStrPtr(b"accent\0".as_ptr() as *const c_char),
    CStrPtr(b"accent_text\0".as_ptr() as *const c_char),
    CStrPtr(b"danger\0".as_ptr() as *const c_char),
    CStrPtr(b"title_inactive\0".as_ptr() as *const c_char),
];

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    0xFF00_0000 | r << 16 | g << 8 | b
}

fn pal(d: u32, s: u32, h: u32, b: u32, t: u32, td: u32, dg: u32, ti: u32) -> Theme {
    Theme { desktop: d, surface: s, surface_hi: h, border: b, text: t, text_dim: td, accent: 0, accent_text: 0xFFFF_FFFF, danger: dg, title_inactive: ti }
}

fn base_palette(b: i32) -> Theme {
    match b {
        TB_LIGHT => Theme::light(0xFF3B_82F6),
        TB_HC => Theme::high_contrast(),
        TB_AMOLED => pal(rgb(0, 0, 0), rgb(0, 0, 0), rgb(0x10, 0x10, 0x12), rgb(0x28, 0x28, 0x2E), rgb(0xF5, 0xF5, 0xF7), rgb(0x9A, 0x9A, 0xA3), rgb(0xEF, 0x44, 0x44), rgb(0x0A, 0x0A, 0x0C)),
        TB_NORD => pal(rgb(0x2E, 0x34, 0x40), rgb(0x3B, 0x42, 0x52), rgb(0x43, 0x4C, 0x5E), rgb(0x4C, 0x56, 0x6A), rgb(0xEC, 0xEF, 0xF4), rgb(0xA9, 0xB4, 0xC8), rgb(0xBF, 0x61, 0x6A), rgb(0x35, 0x3C, 0x4A)),
        TB_DRACULA => pal(rgb(0x21, 0x22, 0x2C), rgb(0x28, 0x2A, 0x36), rgb(0x34, 0x37, 0x46), rgb(0x44, 0x47, 0x5A), rgb(0xF8, 0xF8, 0xF2), rgb(0xA9, 0xAE, 0xD0), rgb(0xFF, 0x55, 0x55), rgb(0x25, 0x27, 0x32)),
        TB_SOLARIZED => pal(rgb(0x00, 0x1F, 0x27), rgb(0x00, 0x2B, 0x36), rgb(0x07, 0x40, 0x4F), rgb(0x58, 0x6E, 0x75), rgb(0xEE, 0xE8, 0xD5), rgb(0x93, 0xA1, 0xA1), rgb(0xDC, 0x32, 0x2F), rgb(0x04, 0x36, 0x42)),
        TB_GRUVBOX => pal(rgb(0x1D, 0x20, 0x21), rgb(0x28, 0x28, 0x28), rgb(0x3C, 0x38, 0x36), rgb(0x50, 0x49, 0x45), rgb(0xEB, 0xDB, 0xB2), rgb(0xA8, 0x99, 0x84), rgb(0xFB, 0x49, 0x34), rgb(0x2C, 0x2B, 0x2A)),
        TB_PAPER => pal(rgb(0xED, 0xE6, 0xD6), rgb(0xFA, 0xF6, 0xEC), rgb(0xFF, 0xFD, 0xF7), rgb(0xD8, 0xCF, 0xBB), rgb(0x2B, 0x2A, 0x26), rgb(0x6B, 0x64, 0x55), rgb(0xC0, 0x39, 0x2B), rgb(0xF1, 0xEC, 0xDF)),
        _ => Theme::dark(0xFF3B_82F6),
    }
}

fn iabs(v: i32) -> i32 {
    if v < 0 { -v } else { v }
}
fn clampi(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo { lo } else if v > hi { hi } else { v }
}

const LIN_TAB: [i32; 17] = [0, 561, 1600, 3310, 5781, 9084, 13287, 18447, 24620, 31855, 40198, 49693, 60383, 72306, 85499, 100000, 100000];
fn lin(c: i32) -> i32 {
    let i = c / 17;
    let f = c % 17;
    LIN_TAB[i as usize] + (LIN_TAB[(i + 1) as usize] - LIN_TAB[i as usize]) * f / 17
}
fn lum(c: u32) -> i32 {
    (2126 * lin(((c >> 16) & 255) as i32) + 7152 * lin(((c >> 8) & 255) as i32) + 722 * lin((c & 255) as i32)) / 10000
}

pub fn contrast_x100(a: u32, b: u32) -> i32 {
    let mut la = lum(a);
    let mut lb = lum(b);
    if la < lb {
        core::mem::swap(&mut la, &mut lb);
    }
    (((la + 5000) as i64 * 100) / (lb + 5000) as i64) as i32
}

pub fn to_hsl(c: u32, h: *mut i32, s: *mut i32, l: *mut i32) {
    let r = ((c >> 16) & 255) as i32;
    let g = ((c >> 8) & 255) as i32;
    let b = (c & 255) as i32;
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    let ll = (mx + mn) * 50 / 255;
    let mut ss = 0;
    let mut hh = 0;
    if d != 0 {
        let den = 255 - iabs(mx + mn - 255);
        ss = if den != 0 { d * 100 / den } else { 0 };
        if mx == r {
            hh = 60 * (g - b) / d;
        } else if mx == g {
            hh = 120 + 60 * (b - r) / d;
        } else {
            hh = 240 + 60 * (r - g) / d;
        }
        if hh < 0 {
            hh += 360;
        }
    }
    unsafe {
        if !h.is_null() {
            *h = hh % 360;
        }
        if !s.is_null() {
            *s = ss.min(100);
        }
        if !l.is_null() {
            *l = ll;
        }
    }
}

pub fn hsl(h: i32, s: i32, l: i32) -> u32 {
    let h = ((h % 360) + 360) % 360;
    let s = clampi(s, 0, 100);
    let l = clampi(l, 0, 100);
    let c = (100 - iabs(2 * l - 100)) * s / 100;
    let hp = h * 100 / 60;
    let x = c * (100 - iabs(hp % 200 - 100)) / 100;
    let (mut r, mut g, mut b) = (0, 0, 0);
    match hp / 100 {
        0 => {
            r = c;
            g = x;
        }
        1 => {
            r = x;
            g = c;
        }
        2 => {
            g = c;
            b = x;
        }
        3 => {
            g = x;
            b = c;
        }
        4 => {
            r = x;
            b = c;
        }
        _ => {
            r = c;
            b = x;
        }
    }
    let m = l - c / 2;
    let rr = clampi((r + m) * 255 / 100, 0, 255) as u32;
    let gg = clampi((g + m) * 255 / 100, 0, 255) as u32;
    let bb = clampi((b + m) * 255 / 100, 0, 255) as u32;
    rgb(rr, gg, bb)
}

pub fn parse_hex(s: &[u8]) -> Option<u32> {
    let s = if s.first() == Some(&b'#') { &s[1..] } else { s };
    if s.len() != 6 {
        return None;
    }
    let mut v = 0u32;
    for &ch in s {
        let d = match ch {
            b'0'..=b'9' => ch - b'0',
            b'a'..=b'f' => ch - b'a' + 10,
            b'A'..=b'F' => ch - b'A' + 10,
            _ => return None,
        };
        v = v << 4 | d as u32;
    }
    Some(0xFF00_0000 | v)
}

fn hex_digit(v: u32) -> u8 {
    if v < 10 { b'0' + v as u8 } else { b'A' + (v - 10) as u8 }
}

pub fn format_hex(c: u32, out: &mut [u8; 8]) {
    out[0] = b'#';
    for i in 0..6 {
        out[1 + i] = hex_digit((c >> (20 - 4 * i)) & 15);
    }
    out[7] = 0;
}

pub fn spec_default(accent: u32) -> ThemeSpec {
    ThemeSpec { base: TB_DARK, accent: accent | 0xFF00_0000, radius: -1, shadow: 100, glow: 100, guard: true, opacity: 92, ..ThemeSpec::EMPTY }
}

fn clamp_spec(s: &mut ThemeSpec) {
    s.tint = clampi(s.tint, 0, 100);
    s.radius = clampi(s.radius, -1, 24);
    s.title_h = if s.title_h != 0 { clampi(s.title_h, 20, 48) } else { 0 };
    s.border = if s.border != 0 { clampi(s.border, 1, 4) } else { 0 };
    s.shadow = clampi(s.shadow, 0, 150);
    s.glow = clampi(s.glow, 0, 150);
    s.opacity = clampi(s.opacity, 40, 100);
    s.icons = clampi(s.icons, 0, 2);
    s.wallpaper = clampi(s.wallpaper, 0, 3);
    if s.base < 0 || s.base as usize >= TB_COUNT {
        s.base = TB_DARK;
    }
    s.accent |= 0xFF00_0000;
}

fn slot(t: &mut Theme, k: usize) -> &mut u32 {
    match k {
        TK_DESKTOP => &mut t.desktop,
        TK_SURFACE => &mut t.surface,
        TK_SURFACE_HI => &mut t.surface_hi,
        TK_BORDER => &mut t.border,
        TK_TEXT => &mut t.text,
        TK_TEXT_DIM => &mut t.text_dim,
        TK_ACCENT => &mut t.accent,
        TK_ACCENT_TEXT => &mut t.accent_text,
        TK_DANGER => &mut t.danger,
        _ => &mut t.title_inactive,
    }
}

fn fix_contrast(fg: u32, bg: u32, want: i32) -> u32 {
    if contrast_x100(fg, bg) >= want {
        return fg;
    }
    let target = if lum(bg) > 18000 { 0xFF00_0000 } else { 0xFFFF_FFFF };
    let mut t = 16;
    while t <= 256 {
        let c = mix(fg, target, t);
        if contrast_x100(c, bg) >= want {
            return c;
        }
        t += 16;
    }
    target
}

pub fn resolve(s: &ThemeSpec) -> Theme {
    let mut t = base_palette(s.base);
    if s.base != TB_HC {
        t.accent = s.accent | 0xFF00_0000;
    }
    let tint = clampi(s.tint, 0, 100);
    if tint != 0 && s.base != TB_HC {
        let k = tint * 64 / 100;
        t.desktop = mix(t.desktop, t.accent, k);
        t.surface = mix(t.surface, t.accent, k);
        t.surface_hi = mix(t.surface_hi, t.accent, k);
        t.border = mix(t.border, t.accent, k);
        t.title_inactive = mix(t.title_inactive, t.accent, k);
    }
    for i in 0..TK_COUNT {
        if s.ovr_mask & (1u16 << i) != 0 {
            *slot(&mut t, i) = s.ovr[i] | 0xFF00_0000;
        }
    }
    if s.guard && s.base != TB_HC {
        t.text = fix_contrast(t.text, t.surface, 450);
        t.text_dim = fix_contrast(t.text_dim, t.surface, 300);
        t.accent_text = fix_contrast(t.accent_text, t.accent, 300);
    }
    t
}

pub fn apply(s: &ThemeSpec, c: &mut Compositor) {
    let st = Style {
        radius: s.radius,
        title_h: s.title_h,
        border: s.border,
        shadow_pct: s.shadow,
        glow_pct: s.glow,
        opacity_pct: s.opacity,
        icon_shape: s.icons,
        wallpaper: s.wallpaper,
    };
    c.theme = resolve(s);
    c.set_style(&st);
}

pub(crate) fn key_for(prefix: &[u8], name: &[u8]) -> [u8; 48] {
    let mut k = [0u8; 48];
    let mut n = 0;
    while n < k.len() - 1 && n < prefix.len() && prefix[n] != 0 {
        k[n] = prefix[n];
        n += 1;
    }
    let mut i = 0;
    while n < k.len() - 1 && i < name.len() && name[i] != 0 {
        k[n] = name[i];
        n += 1;
        i += 1;
    }
    k
}

pub fn spec_load(s: &mut ThemeSpec, be: &ShellBackend, legacy_accent: u32) {
    *s = spec_default(legacy_accent);
    let mut v = [0u8; 96];
    let legacy = cfg_int(be, b"ui.theme\0", 0);
    s.base = if legacy == 1 { TB_LIGHT } else if legacy == 2 { TB_HC } else { TB_DARK };
    if be.cfg_get(b"ui.base\0", &mut v) {
        let n = v.iter().position(|&b| b == 0).unwrap_or(v.len());
        for (i, name) in BASE_NAMES.iter().enumerate() {
            if &v[..n] == &name[..name.len() - 1] {
                s.base = i as i32;
            }
        }
    }
    if be.cfg_get(b"ui.accent_hex\0", &mut v) {
        let n = v.iter().position(|&b| b == 0).unwrap_or(v.len());
        if let Some(a) = parse_hex(&v[..n]) {
            s.accent = a;
        }
    }
    s.tint = cfg_int(be, b"ui.tint\0", 0);
    s.radius = cfg_int(be, b"ui.radius\0", -1);
    s.title_h = cfg_int(be, b"ui.title_h\0", 0);
    s.border = cfg_int(be, b"ui.border\0", 0);
    s.shadow = cfg_int(be, b"ui.shadow\0", 100);
    s.glow = cfg_int(be, b"ui.glow\0", 100);
    s.opacity = cfg_int(be, b"ui.opacity\0", 92);
    s.icons = cfg_int(be, b"ui.icons\0", 0);
    s.wallpaper = cfg_int(be, b"ui.wallpaper\0", 0);
    s.guard = cfg_bool(be, b"ui.guard\0", true);
    for i in 0..TK_COUNT {
        let k = key_for(b"ui.c.", TOKEN_NAMES[i]);
        if be.cfg_get(&k, &mut v) {
            let n = v.iter().position(|&b| b == 0).unwrap_or(v.len());
            if let Some(a) = parse_hex(&v[..n]) {
                s.ovr[i] = a;
                s.ovr_mask |= 1u16 << i;
            }
        }
    }
    clamp_spec(s);
}

pub fn spec_store(s: &ThemeSpec, be: &ShellBackend) {
    let mut hex = [0u8; 8];
    be.cfg_set(b"ui.base\0", BASE_NAMES[s.base as usize]);
    format_hex(s.accent, &mut hex);
    be.cfg_set(b"ui.accent_hex\0", &hex);
    cfg_set_int(be, b"ui.tint\0", s.tint);
    cfg_set_int(be, b"ui.radius\0", s.radius);
    cfg_set_int(be, b"ui.title_h\0", s.title_h);
    cfg_set_int(be, b"ui.border\0", s.border);
    cfg_set_int(be, b"ui.shadow\0", s.shadow);
    cfg_set_int(be, b"ui.glow\0", s.glow);
    cfg_set_bool(be, b"ui.guard\0", s.guard);
    cfg_set_int(be, b"ui.opacity\0", s.opacity);
    cfg_set_int(be, b"ui.icons\0", s.icons);
    cfg_set_int(be, b"ui.wallpaper\0", s.wallpaper);
    for i in 0..TK_COUNT {
        let k = key_for(b"ui.c.", TOKEN_NAMES[i]);
        if s.ovr_mask & (1u16 << i) != 0 {
            format_hex(s.ovr[i], &mut hex);
            be.cfg_set(&k, &hex);
        } else {
            be.cfg_set(&k, b"\0");
        }
    }
    let legacy = if s.base == TB_LIGHT || s.base == TB_PAPER { 1 } else if s.base == TB_HC { 2 } else { 0 };
    cfg_set_int(be, b"ui.theme\0", legacy);
    cfg_set_bool(be, b"ui.high_contrast\0", s.base == TB_HC);
}

struct Out {
    ptr: *mut u8,
    cap: usize,
    len: usize,
}
impl Out {
    fn new(ptr: *mut u8, cap: usize) -> Self {
        Self { ptr, cap, len: 0 }
    }
    fn put(&mut self, s: &[u8]) {
        if self.len < self.cap {
            let room = self.cap - self.len;
            let n = s.len().min(room);
            for i in 0..n {
                // SAFETY: caller provided cap writable bytes.
                unsafe { *self.ptr.add(self.len + i) = s[i] };
            }
        }
        self.len += s.len();
    }
    fn u32(&mut self, mut v: u32) {
        let mut tmp = [0u8; 10];
        let mut n = 0;
        if v == 0 {
            tmp[0] = b'0';
            n = 1;
        }
        while v != 0 {
            tmp[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
        }
        for i in (0..n).rev() {
            self.put(&tmp[i..i + 1]);
        }
    }
    fn i32(&mut self, v: i32) {
        if v < 0 {
            self.put(b"-");
            self.u32(v.unsigned_abs());
        } else {
            self.u32(v as u32);
        }
    }
    fn finish(&mut self) {
        if self.cap > 0 {
            let n = self.len.min(self.cap - 1);
            // SAFETY: within the caller's buffer.
            unsafe { *self.ptr.add(n) = 0 };
        }
    }
}

pub fn export(s: &ThemeSpec, out: *mut u8, cap: usize) -> usize {
    let mut w = Out::new(out, cap);
    let mut hex = [0u8; 8];
    w.put(b"base=");
    w.put(&BASE_NAMES[s.base as usize][..BASE_NAMES[s.base as usize].len() - 1]);
    w.put(b";accent=");
    format_hex(s.accent, &mut hex);
    w.put(&hex[..7]);
    w.put(b";tint=");
    w.i32(s.tint);
    w.put(b";radius=");
    w.i32(s.radius);
    w.put(b";title_h=");
    w.i32(s.title_h);
    w.put(b";border=");
    w.i32(s.border);
    w.put(b";shadow=");
    w.i32(s.shadow);
    w.put(b";glow=");
    w.i32(s.glow);
    w.put(b";guard=");
    w.i32(s.guard as i32);
    w.put(b";opacity=");
    w.i32(s.opacity);
    w.put(b";icons=");
    w.i32(s.icons);
    w.put(b";wallpaper=");
    w.i32(s.wallpaper);
    for i in 0..TK_COUNT {
        if s.ovr_mask & (1u16 << i) != 0 {
            format_hex(s.ovr[i], &mut hex);
            w.put(b";c.");
            w.put(&TOKEN_NAMES[i][..TOKEN_NAMES[i].len() - 1]);
            w.put(b"=");
            w.put(&hex[..7]);
        }
    }
    w.finish();
    w.len
}

fn parse_int(s: &[u8]) -> Option<i64> {
    let mut i = 0;
    let mut neg = false;
    if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
        neg = s[i] == b'-';
        i += 1;
    }
    if i >= s.len() {
        return None;
    }
    let mut v: i64 = 0;
    while i < s.len() {
        if !s[i].is_ascii_digit() {
            return None;
        }
        v = v * 10 + (s[i] - b'0') as i64;
        if v > 100_000_000 {
            v = 100_000_000;
        }
        i += 1;
    }
    Some(if neg { -v } else { v })
}

pub fn import(s: &mut ThemeSpec, src: &[u8]) -> i32 {
    let mut t = *s;
    let mut bad = 0;
    let mut buf = [0u8; 640];
    let n = src.len().min(639);
    buf[..n].copy_from_slice(&src[..n]);
    let mut p = 0usize;
    while p < n {
        let mut e = p;
        while e < n && buf[e] != b';' && buf[e] != b'\n' && buf[e] != b'\r' {
            e += 1;
        }
        let sep = if e < n { buf[e] } else { 0 };
        let mut a = p;
        while a < e && buf[a] == b' ' {
            a += 1;
        }
        if a < e && buf[a] != b'#' {
            let mut eq = None;
            for i in a..e {
                if buf[i] == b'=' {
                    eq = Some(i);
                    break;
                }
            }
            match eq {
                None => bad += 1,
                Some(q) => {
                    let key = &buf[a..q];
                    let val = &buf[q + 1..e];
                    let num = parse_int(val);
                    if key == b"base" {
                        let mut f = -1;
                        for i in 0..TB_COUNT {
                            if val == &BASE_NAMES[i][..BASE_NAMES[i].len() - 1] {
                                f = i as i32;
                            }
                        }
                        if f < 0 {
                            bad += 1;
                        } else {
                            t.base = f;
                        }
                    } else if key == b"accent" {
                        if let Some(c) = parse_hex(val) {
                            t.accent = c;
                        } else {
                            bad += 1;
                        }
                    } else if key == b"tint" && num.is_some() {
                        t.tint = num.unwrap() as i32;
                    } else if key == b"radius" && num.is_some() {
                        t.radius = num.unwrap() as i32;
                    } else if key == b"title_h" && num.is_some() {
                        t.title_h = num.unwrap() as i32;
                    } else if key == b"border" && num.is_some() {
                        t.border = num.unwrap() as i32;
                    } else if key == b"shadow" && num.is_some() {
                        t.shadow = num.unwrap() as i32;
                    } else if key == b"glow" && num.is_some() {
                        t.glow = num.unwrap() as i32;
                    } else if key == b"guard" && num.is_some() {
                        t.guard = num.unwrap() != 0;
                    } else if key == b"opacity" && num.is_some() {
                        t.opacity = num.unwrap() as i32;
                    } else if key == b"icons" && num.is_some() {
                        t.icons = num.unwrap() as i32;
                    } else if key == b"wallpaper" && num.is_some() {
                        t.wallpaper = num.unwrap() as i32;
                    } else if key.starts_with(b"c.") {
                        let mut f = None;
                        for i in 0..TK_COUNT {
                            if &key[2..] == &TOKEN_NAMES[i][..TOKEN_NAMES[i].len() - 1] {
                                f = Some(i);
                            }
                        }
                        match (f, parse_hex(val)) {
                            (Some(i), Some(c)) => {
                                t.ovr[i] = c;
                                t.ovr_mask |= 1u16 << i;
                            }
                            _ => bad += 1,
                        }
                    } else {
                        bad += 1;
                    }
                }
            }
        }
        if sep == 0 {
            break;
        }
        p = e + 1;
    }
    clamp_spec(&mut t);
    *s = t;
    bad
}

struct Preset {
    name: &'static [u8],
    base: i32,
    accent: u32,
    tint: i32,
    radius: i32,
    title_h: i32,
    border: i32,
    shadow: i32,
    glow: i32,
}
const PRESETS: [Preset; 12] = [
    Preset { name: b"LoOS Dark\0", base: TB_DARK, accent: 0xFF3B_82F6, tint: 0, radius: -1, title_h: 0, border: 0, shadow: 100, glow: 100 },
    Preset { name: b"LoOS Light\0", base: TB_LIGHT, accent: 0xFF3B_82F6, tint: 0, radius: -1, title_h: 0, border: 0, shadow: 100, glow: 100 },
    Preset { name: b"High contrast\0", base: TB_HC, accent: 0xFF00_FFFF, tint: 0, radius: -1, title_h: 0, border: 2, shadow: 0, glow: 0 },
    Preset { name: b"AMOLED\0", base: TB_AMOLED, accent: 0xFF22_D3EE, tint: 0, radius: 12, title_h: 0, border: 0, shadow: 0, glow: 60 },
    Preset { name: b"Nord\0", base: TB_NORD, accent: 0xFF88_C0D0, tint: 0, radius: 10, title_h: 0, border: 0, shadow: 80, glow: 70 },
    Preset { name: b"Dracula\0", base: TB_DRACULA, accent: 0xFFBD_93F9, tint: 8, radius: 10, title_h: 0, border: 0, shadow: 110, glow: 120 },
    Preset { name: b"Solarized\0", base: TB_SOLARIZED, accent: 0xFF2A_A198, tint: 0, radius: 6, title_h: 0, border: 0, shadow: 70, glow: 60 },
    Preset { name: b"Gruvbox\0", base: TB_GRUVBOX, accent: 0xFFFA_BD2F, tint: 0, radius: 4, title_h: 0, border: 0, shadow: 80, glow: 50 },
    Preset { name: b"Paper\0", base: TB_PAPER, accent: 0xFFB4_5309, tint: 0, radius: 6, title_h: 0, border: 0, shadow: 60, glow: 40 },
    Preset { name: b"Glass\0", base: TB_DARK, accent: 0xFF60_A5FA, tint: 18, radius: 16, title_h: 32, border: 0, shadow: 140, glow: 150 },
    Preset { name: b"Compact\0", base: TB_DARK, accent: 0xFF3B_82F6, tint: 0, radius: 4, title_h: 24, border: 1, shadow: 40, glow: 0 },
    Preset { name: b"Performance\0", base: TB_DARK, accent: 0xFF3B_82F6, tint: 0, radius: 0, title_h: 24, border: 1, shadow: 0, glow: 0 },
];

pub fn preset_count() -> i32 {
    PRESETS.len() as i32
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn theme_spec_default(s: *mut ThemeSpec, accent: u32) {
    if let Some(s) = unsafe { s.as_mut() } {
        *s = spec_default(accent);
    }
}
#[no_mangle]
pub unsafe extern "C" fn theme_resolve(s: *const ThemeSpec, out: *mut Theme) {
    if let (Some(s), Some(out)) = (unsafe { s.as_ref() }, unsafe { out.as_mut() }) {
        *out = resolve(s);
    }
}
#[no_mangle]
pub unsafe extern "C" fn theme_apply(s: *const ThemeSpec, c: *mut Compositor) {
    if let (Some(s), Some(c)) = (unsafe { s.as_ref() }, unsafe { c.as_mut() }) {
        apply(s, c);
    }
}
#[no_mangle]
pub unsafe extern "C" fn theme_spec_load(s: *mut ThemeSpec, be: *const ShellBackend, legacy: u32) {
    if let (Some(s), Some(be)) = (unsafe { s.as_mut() }, unsafe { be.as_ref() }) {
        spec_load(s, be, legacy);
    }
}
#[no_mangle]
pub unsafe extern "C" fn theme_spec_store(s: *const ThemeSpec, be: *const ShellBackend) {
    if let (Some(s), Some(be)) = (unsafe { s.as_ref() }, unsafe { be.as_ref() }) {
        spec_store(s, be);
    }
}
#[no_mangle]
pub unsafe extern "C" fn theme_export(s: *const ThemeSpec, out: *mut c_char, cap: usize) -> usize {
    match unsafe { s.as_ref() } {
        Some(s) => export(s, out as *mut u8, cap),
        None => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn theme_import(s: *mut ThemeSpec, text: *const c_char) -> i32 {
    match (unsafe { s.as_mut() }, unsafe { text.as_ref() }) {
        (Some(s), Some(_)) => {
            let mut n = 0usize;
            // SAFETY: NUL-terminated string.
            while unsafe { *text.add(n) } != 0 {
                n += 1;
            }
            // SAFETY: n bytes before the NUL terminator.
            import(s, unsafe { core::slice::from_raw_parts(text as *const u8, n) })
        }
        _ => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn theme_preset_count() -> i32 {
    preset_count()
}
#[no_mangle]
pub unsafe extern "C" fn theme_preset_name(i: i32) -> *const c_char {
    if i >= 0 && (i as usize) < PRESETS.len() { PRESETS[i as usize].name.as_ptr() as *const c_char } else { b"\0".as_ptr() as *const c_char }
}
#[no_mangle]
pub unsafe extern "C" fn theme_preset(i: i32, out: *mut ThemeSpec) -> bool {
    let Some(out) = (unsafe { out.as_mut() }) else { return false };
    if i < 0 || i as usize >= PRESETS.len() {
        return false;
    }
    let p = &PRESETS[i as usize];
    *out = spec_default(p.accent);
    out.base = p.base;
    out.tint = p.tint;
    out.radius = p.radius;
    out.title_h = p.title_h;
    out.border = p.border;
    out.shadow = p.shadow;
    out.glow = p.glow;
    true
}
#[no_mangle]
pub unsafe extern "C" fn theme_hsl(h: i32, s: i32, l: i32) -> u32 {
    hsl(h, s, l)
}
#[no_mangle]
pub unsafe extern "C" fn theme_to_hsl(c: u32, h: *mut i32, s: *mut i32, l: *mut i32) {
    to_hsl(c, h, s, l)
}
#[no_mangle]
pub unsafe extern "C" fn theme_contrast_x100(a: u32, b: u32) -> i32 {
    contrast_x100(a, b)
}
#[no_mangle]
pub unsafe extern "C" fn theme_parse_hex(s: *const c_char, out: *mut u32) -> bool {
    let Some(out) = (unsafe { out.as_mut() }) else { return false };
    let mut n = 0usize;
    if s.is_null() {
        return false;
    }
    // SAFETY: NUL-terminated string.
    while unsafe { *s.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: n bytes before the NUL.
    match parse_hex(unsafe { core::slice::from_raw_parts(s as *const u8, n) }) {
        Some(v) => {
            *out = v;
            true
        }
        None => false,
    }
}
#[no_mangle]
pub unsafe extern "C" fn theme_format_hex(c: u32, out: *mut c_char) {
    if out.is_null() {
        return;
    }
    let mut b = [0u8; 8];
    format_hex(c, &mut b);
    // SAFETY: caller provides 8 bytes (theme.h).
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out as *mut u8, 8) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ThemeSpec>(), 92);
        assert_eq!(offset_of!(ThemeSpec, accent), 4);
        assert_eq!(offset_of!(ThemeSpec, guard), 32);
        assert_eq!(offset_of!(ThemeSpec, opacity), 36);
        assert_eq!(offset_of!(ThemeSpec, ovr), 48);
        assert_eq!(offset_of!(ThemeSpec, ovr_mask), 88);
    }

    #[test]
    fn contrast_known_pairs() {
        assert!((2089..=2111).contains(&contrast_x100(0xFF00_0000, 0xFFFF_FFFF)));
        assert!((440..470).contains(&contrast_x100(0xFF77_7777, 0xFFFF_FFFF)));
        assert_eq!(contrast_x100(0xFF12_3456, 0xFF12_3456), 100);
    }

    #[test]
    fn hsl_round_trip() {
        let (mut h, mut s, mut l) = (0, 0, 0);
        to_hsl(0xFFFF_0000, &mut h, &mut s, &mut l);
        assert_eq!((h, s, l), (0, 100, 50));
        to_hsl(0xFF00_FF00, &mut h, &mut s, &mut l);
        assert_eq!(h, 120);
        assert_eq!(hsl(240, 100, 50), 0xFF00_00FF);
        assert_eq!(hsl(0, 0, 100), 0xFFFF_FFFF);
        assert_eq!(hsl(0, 0, 0), 0xFF00_0000);
        for i in (0..360).step_by(17) {
            let c = hsl(i, 70, 55);
            to_hsl(c, &mut h, &mut s, &mut l);
            assert!(iabs(h - i) <= 2 || iabs(h - i) >= 358, "h {i} -> {h}");
        }
    }

    #[test]
    fn export_import_round_trip_with_effects() {
        let mut s = spec_default(0xFF3B_82F6);
        s.base = TB_NORD;
        s.tint = 10;
        s.radius = 12;
        s.opacity = 80;
        s.icons = 2;
        s.wallpaper = 2;
        s.ovr[TK_SURFACE] = 0xFF10_1820;
        s.ovr_mask = 1 << TK_SURFACE;
        let mut buf = [0u8; 512];
        let n = export(&s, buf.as_mut_ptr(), buf.len());
        assert!(n > 0 && n < buf.len());
        let mut t = spec_default(0);
        assert_eq!(import(&mut t, &buf[..n]), 0);
        assert_eq!(t.base, TB_NORD);
        assert_eq!((t.tint, t.radius, t.opacity, t.icons, t.wallpaper), (10, 12, 80, 2, 2));
        assert_eq!(t.ovr_mask & (1 << TK_SURFACE), 1 << TK_SURFACE);
        assert_eq!(t.ovr[TK_SURFACE], 0xFF10_1820);
    }

    #[test]
    fn import_rejects_bad_entries_and_clamps() {
        let mut s = spec_default(0xFF3B_82F6);
        let mut text = b"base=nope;tint=999;radius=-9;guard=2;accent=#GGGGGG;bogus=1\0".to_vec();
        let n = import(&mut s, &mut text);
        assert_eq!(n, 3);
        assert_eq!(s.tint, 100);
        assert_eq!(s.radius, -1);
        assert!(s.guard);
    }

    #[test]
    fn presets_cover_the_documented_set() {
        assert_eq!(preset_count(), 12);
        let mut s = ThemeSpec::EMPTY;
        assert!(unsafe { theme_preset(0, &mut s) });
        assert_eq!(s.base, TB_DARK);
        assert!(!unsafe { theme_preset(99, &mut s) });
        let hc = resolve(&{
            let mut p = ThemeSpec::EMPTY;
            unsafe { theme_preset(2, &mut p) };
            p
        });
        assert_eq!(hc.border, 0xFFFF_FFFF);
    }

    #[test]
    fn guard_repairs_unreadable_text() {
        let mut s = spec_default(0xFF3B_82F6);
        s.ovr[TK_TEXT] = 0xFF1A_2230;
        s.ovr_mask = 1 << TK_TEXT;
        let t = resolve(&s);
        assert!(contrast_x100(t.text, t.surface) >= 450);
        s.guard = false;
        let t = resolve(&s);
        assert_eq!(t.text, 0xFF1A_2230);
    }
}

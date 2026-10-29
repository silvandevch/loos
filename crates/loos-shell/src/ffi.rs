//! C ABI under the names in `src/shell/shell.h`. Callers pass valid buffers (C contract):
//! `px` holds `stride * h` pixels, blit sources `sstride * sh`.
#![allow(missing_docs, clippy::missing_safety_doc, clippy::too_many_arguments)]

use crate::draw::{blend, mix, Canvas, Damage, Rect, Theme};

/// C `shell_fb`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Fb {
    pub(crate) px: *mut u32,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) stride: i32,
    pub(crate) scale: i32,
}

unsafe fn canvas<'a>(fb: *mut Fb) -> Option<Canvas<'a>> {
    // SAFETY: caller passes a live shell_fb whose px covers stride*h pixels.
    let f = unsafe { fb.as_mut()? };
    if f.px.is_null() || f.w <= 0 || f.h <= 0 || f.stride < f.w {
        return None;
    }
    // SAFETY: as above.
    let px = unsafe { core::slice::from_raw_parts_mut(f.px, (f.stride * f.h) as usize) };
    Some(Canvas { px, w: f.w, h: f.h, stride: f.stride, scale: f.scale })
}

macro_rules! with {
    ($fb:expr, $c:ident => $body:expr) => {
        // SAFETY: shell_fb contract.
        if let Some(mut $c) = unsafe { canvas($fb) } {
            $body;
        }
    };
}

#[no_mangle] pub extern "C" fn rect_intersect(a: Rect, b: Rect) -> Rect { let r = a.intersect(b); if r.is_empty() { Rect::new(r.x, r.y, 0, 0) } else { r } }
#[no_mangle] pub extern "C" fn rect_union(a: Rect, b: Rect) -> Rect { a.union(b) }
#[no_mangle] pub extern "C" fn rect_overlaps(a: Rect, b: Rect) -> bool { a.overlaps(b) }
#[no_mangle] pub extern "C" fn color_blend(d: u32, s: u32) -> u32 { blend(d, s) }
#[no_mangle] pub extern "C" fn color_mix(a: u32, b: u32, t: i32) -> u32 { mix(a, b, t) }

#[no_mangle] pub unsafe extern "C" fn shell_theme_dark(t: *mut Theme, accent: u32) { if let Some(t) = unsafe { t.as_mut() } { *t = Theme::dark(accent) } }
#[no_mangle] pub unsafe extern "C" fn shell_theme_light(t: *mut Theme, accent: u32) { if let Some(t) = unsafe { t.as_mut() } { *t = Theme::light(accent) } }
#[no_mangle] pub unsafe extern "C" fn shell_theme_high_contrast(t: *mut Theme) { if let Some(t) = unsafe { t.as_mut() } { *t = Theme::high_contrast() } }

#[no_mangle] pub unsafe extern "C" fn fb_lw(fb: *const Fb) -> i32 { unsafe { fb.as_ref() }.map_or(0, |f| f.w * 100 / if f.scale > 0 { f.scale } else { 100 }) }
#[no_mangle] pub unsafe extern "C" fn fb_lh(fb: *const Fb) -> i32 { unsafe { fb.as_ref() }.map_or(0, |f| f.h * 100 / if f.scale > 0 { f.scale } else { 100 }) }
#[no_mangle] pub unsafe extern "C" fn fb_fill(fb: *mut Fb, clip: Rect, r: Rect, c: u32) { with!(fb, k => k.fill(clip, r, c)) }
#[no_mangle] pub unsafe extern "C" fn fb_hline(fb: *mut Fb, clip: Rect, x: i32, y: i32, len: i32, c: u32) { with!(fb, k => k.hline(clip, x, y, len, c)) }
#[no_mangle] pub unsafe extern "C" fn fb_vline(fb: *mut Fb, clip: Rect, x: i32, y: i32, len: i32, c: u32) { with!(fb, k => k.vline(clip, x, y, len, c)) }
#[no_mangle] pub unsafe extern "C" fn fb_frame(fb: *mut Fb, clip: Rect, r: Rect, c: u32) { with!(fb, k => k.frame(clip, r, c)) }
#[no_mangle] pub unsafe extern "C" fn fb_fill_round(fb: *mut Fb, clip: Rect, r: Rect, rad: i32, c: u32) { with!(fb, k => k.fill_round(clip, r, rad, c)) }
#[no_mangle] pub unsafe extern "C" fn fb_fill_circle(fb: *mut Fb, clip: Rect, cx: i32, cy: i32, rad: i32, c: u32) { with!(fb, k => k.fill_circle(clip, cx, cy, rad, c)) }
#[no_mangle] pub unsafe extern "C" fn fb_stroke_round(fb: *mut Fb, clip: Rect, r: Rect, rad: i32, thick: i32, c: u32) { with!(fb, k => k.stroke_round(clip, r, rad, thick, c)) }
#[no_mangle] pub unsafe extern "C" fn fb_shadow(fb: *mut Fb, clip: Rect, r: Rect, rad: i32, blur: i32, dy: i32, c: u32) { with!(fb, k => k.shadow(clip, r, rad, blur, dy, c)) }
#[no_mangle] pub unsafe extern "C" fn fb_gradient_v(fb: *mut Fb, clip: Rect, r: Rect, top: u32, bottom: u32) { with!(fb, k => k.gradient_v(clip, r, top, bottom)) }
#[no_mangle] pub unsafe extern "C" fn fb_radial_glow(fb: *mut Fb, clip: Rect, cx: i32, cy: i32, radius: i32, c: u32, strength: i32) { with!(fb, k => k.radial_glow(clip, cx, cy, radius, c, strength)) }

unsafe fn src<'a>(p: *const u32, sh: i32, sstride: i32) -> &'a [u32] {
    if p.is_null() || sh <= 0 || sstride <= 0 { &[] } else { unsafe { core::slice::from_raw_parts(p, (sstride * sh) as usize) } }
}
#[no_mangle]
pub unsafe extern "C" fn fb_blit(fb: *mut Fb, clip: Rect, dx: i32, dy: i32, s: *const u32, sw: i32, sh: i32, sstride: i32, sr: Rect) {
    let s = unsafe { src(s, sh, sstride) };
    if !s.is_empty() { with!(fb, k => k.blit(clip, dx, dy, s, sw, sh, sstride, sr, Rect::default(), 0)) }
}
#[no_mangle]
pub unsafe extern "C" fn fb_blit_rounded(fb: *mut Fb, clip: Rect, dx: i32, dy: i32, s: *const u32, sw: i32, sh: i32, sstride: i32, sr: Rect, round: Rect, rad: i32) {
    let s = unsafe { src(s, sh, sstride) };
    if !s.is_empty() { with!(fb, k => k.blit(clip, dx, dy, s, sw, sh, sstride, sr, round, rad)) }
}

#[no_mangle] pub unsafe extern "C" fn damage_clear(d: *mut Damage) { if let Some(d) = unsafe { d.as_mut() } { d.n = 0 } }
#[no_mangle] pub unsafe extern "C" fn damage_add(d: *mut Damage, r: Rect) { if let Some(d) = unsafe { d.as_mut() } { d.add(r) } }
#[no_mangle] pub unsafe extern "C" fn damage_bounds(d: *const Damage) -> Rect { unsafe { d.as_ref() }.map_or(Rect::default(), Damage::bounds) }

// ---------------------------------------------------------------- fonts and text

use crate::font::{self, Font, FontBitmap, LoosFont, LoosGlyph, ShellFont};
use crate::raster;
use crate::text;
use core::ffi::{c_char, CStr};

/// C `raster_path`.
#[repr(C)]
pub struct RasterPath {
    xy: *const i16,
    on: *const u8,
    ends: *const u16,
    ncont: i32,
}

/// C string bytes (empty for null).
pub(crate) unsafe fn c_bytes<'a>(p: *const c_char) -> &'a [u8] {
    if p.is_null() {
        &[]
    } else {
        // SAFETY: the caller passes a NUL-terminated string (module contract).
        unsafe { CStr::from_ptr(p) }.to_bytes()
    }
}

/// Glyph/advance view: only the glyph table is needed (no outline data).
unsafe fn glyph_view(f: *const LoosFont) -> Option<Font<'static>> {
    if f.is_null() {
        return None;
    }
    // SAFETY: `f` points to a live loos_font.
    let f = unsafe { &*f };
    let glyphs = if f.nglyphs == 0 || f.glyphs.is_null() {
        &[][..]
    } else {
        // SAFETY: nglyphs entries as declared by the font.
        unsafe { core::slice::from_raw_parts(f.glyphs, f.nglyphs as usize) }
    };
    Some(Font::new(f.upm, f.ascent, f.descent, glyphs, &[], &[], &[]))
}

/// Full view: lengths implied by the glyph metadata, exactly like the C engine's indexing.
unsafe fn font_view(f: *const LoosFont) -> Option<Font<'static>> {
    if f.is_null() {
        return None;
    }
    // SAFETY: `f` points to a live loos_font.
    let f = unsafe { &*f };
    let glyphs = if f.nglyphs == 0 || f.glyphs.is_null() {
        &[][..]
    } else {
        // SAFETY: nglyphs entries as declared by the font.
        unsafe { core::slice::from_raw_parts(f.glyphs, f.nglyphs as usize) }
    };
    let nends = glyphs.iter().map(|g| g.end0 as usize + g.ncont as usize).max().unwrap_or(0);
    let ends = if nends == 0 || f.ends.is_null() {
        &[][..]
    } else {
        // SAFETY: the contour table covers every glyph's ends.
        unsafe { core::slice::from_raw_parts(f.ends, nends) }
    };
    let npts = glyphs
        .iter()
        .filter(|g| g.ncont > 0)
        .filter_map(|g| ends.get(g.end0 as usize + g.ncont as usize - 1))
        .map(|&e| e as usize)
        .max()
        .unwrap_or(0);
    let pts = if npts == 0 || f.pts.is_null() {
        &[][..]
    } else {
        // SAFETY: the point table holds npts x,y pairs.
        unsafe { core::slice::from_raw_parts(f.pts, npts * 2) }
    };
    let on = if npts == 0 || f.on.is_null() {
        &[][..]
    } else {
        // SAFETY: one flag per point.
        unsafe { core::slice::from_raw_parts(f.on, npts) }
    };
    Some(Font::new(f.upm, f.ascent, f.descent, glyphs, pts, on, ends))
}

#[no_mangle]
pub unsafe extern "C" fn raster_fill(p: *const RasterPath, k_q24: i32, ox: i32, oy: i32, w: i32, h: i32, cov: *mut u8) -> i32 {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    let Some(p) = (unsafe { p.as_ref() }) else { return -1 };
    if p.xy.is_null() || p.on.is_null() || p.ends.is_null() || cov.is_null() || p.ncont < 0 || w <= 0 || h <= 0 {
        return -1;
    }
    let ncont = p.ncont as usize;
    // SAFETY: `ncont` contour ends, and the last one bounds the point tables (C contract).
    let ends = unsafe { core::slice::from_raw_parts(p.ends, ncont) };
    let npts = ends.last().map_or(0, |&e| e as usize);
    // SAFETY: the path owns npts points and flags; the bitmap owns w*h bytes.
    let (xy, on) = unsafe { (core::slice::from_raw_parts(p.xy, npts * 2), core::slice::from_raw_parts(p.on, npts)) };
    let Some(area) = (w as usize).checked_mul(h as usize) else { return -1 };
    if area > isize::MAX as usize {
        return -1;
    }
    // SAFETY: caller provides `area` writable bytes.
    let cov = unsafe { core::slice::from_raw_parts_mut(cov, area) };
    if raster::fill(xy, on, ends, k_q24, ox, oy, w, h, cov) {
        0
    } else {
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn font_next_cp(sp: *mut *const c_char) -> u32 {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    let Some(sp) = (unsafe { sp.as_mut() }) else { return 0 };
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    let bytes = unsafe { c_bytes(*sp) };
    let mut pos = 0usize;
    let cp = font::next_cp(bytes, &mut pos);
    if pos > 0 {
        let p = *sp;
        // SAFETY: pos bytes were consumed from the NUL-terminated string.
        unsafe { *sp = p.add(pos) };
    }
    cp
}

#[no_mangle]
pub unsafe extern "C" fn font_find(f: *const LoosFont, cp: u32) -> *const LoosGlyph {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    let Some(font) = (unsafe { glyph_view(f) }) else { return core::ptr::null() };
    match font::find(font.glyphs, cp) {
        // SAFETY: index within the glyph table.
        Some(i) => unsafe { font.glyphs.as_ptr().add(i) },
        None => core::ptr::null(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn font_advance_q8(f: ShellFont, cp: u32) -> i32 {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    match unsafe { glyph_view(f.face) } {
        Some(font) => font::advance_q8(&font, f.size, cp),
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn font_text_width(f: ShellFont, s: *const c_char) -> i32 {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    match unsafe { glyph_view(f.face) } {
        // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
        Some(font) => font::text_width(&font, f.size, unsafe { c_bytes(s) }),
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn font_fit(f: ShellFont, s: *const c_char, max_px: i32) -> i32 {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    match unsafe { glyph_view(f.face) } {
        // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
        Some(font) => font::fit(&font, f.size, unsafe { c_bytes(s) }, max_px) as i32,
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn font_index_at(f: ShellFont, s: *const c_char, x_px: i32) -> i32 {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    match unsafe { glyph_view(f.face) } {
        // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
        Some(font) => font::index_at(&font, f.size, unsafe { c_bytes(s) }, x_px) as i32,
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn font_bitmap_get(f: *const LoosFont, g: *const LoosGlyph, size_q4: i32, subx: i32) -> *const FontBitmap {
    if g.is_null() {
        return core::ptr::null();
    }
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    let Some(font) = (unsafe { font_view(f) }) else { return core::ptr::null() };
    let base = font.glyphs.as_ptr() as usize;
    let gpos = g as usize;
    if gpos < base {
        return core::ptr::null();
    }
    let gi = (gpos - base) / core::mem::size_of::<LoosGlyph>();
    if gi >= font.glyphs.len() {
        return core::ptr::null();
    }
    font::bitmap_get(&font, gi, size_q4, subx)
}

#[no_mangle]
pub extern "C" fn font_cache_flush() {
    font::cache_flush();
}

#[no_mangle]
pub unsafe extern "C" fn shell_text_width_ex(s: *const c_char, f: ShellFont) -> i32 {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    unsafe { font_text_width(f, s) }
}

#[no_mangle]
pub unsafe extern "C" fn shell_text_width(s: *const c_char) -> i32 {
    let body = crate::font_data::regular();
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    font::text_width(&body, 14, unsafe { c_bytes(s) })
}

#[no_mangle]
pub extern "C" fn shell_text_height() -> i32 {
    text::TEXT_HEIGHT
}

#[no_mangle]
pub unsafe extern "C" fn shell_ellipsize(out: *mut c_char, cap: usize, s: *const c_char, max_px: i32) {
    if out.is_null() || cap == 0 {
        return;
    }
    let body = crate::font_data::regular();
    // SAFETY: caller provides cap writable bytes; c_bytes gives the NUL-terminated source.
    let out = unsafe { core::slice::from_raw_parts_mut(out as *mut u8, cap) };
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    text::ellipsize(out, unsafe { c_bytes(s) }, max_px, &body, 14);
}

#[no_mangle]
pub unsafe extern "C" fn shell_text_fit(s: *const c_char, max_px: i32) -> i32 {
    let body = crate::font_data::regular();
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    font::fit(&body, 14, unsafe { c_bytes(s) }, max_px) as i32
}

#[no_mangle]
pub unsafe extern "C" fn shell_text_index_at(s: *const c_char, x: i32) -> i32 {
    let body = crate::font_data::regular();
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    font::index_at(&body, 14, unsafe { c_bytes(s) }, x) as i32
}

#[no_mangle]
pub unsafe extern "C" fn fb_text(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: *const c_char, c: u32) -> i32 {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    unsafe { fb_text_ex(fb, clip, x, y, s, c, ShellFont { face: core::ptr::addr_of!(crate::font_data::loos_font_regular), size: 14 }) }
}

#[no_mangle]
pub unsafe extern "C" fn fb_text_ex(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: *const c_char, c: u32, f: ShellFont) -> i32 {
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    let Some(font) = (unsafe { font_view(f.face) }) else { return 0 };
    // SAFETY: caller contract: pointers are valid for the stated lengths (see module docs).
    let text = unsafe { c_bytes(s) };
    // SAFETY: caller passes a live shell_fb (see `canvas`).
    if let Some(mut canvas) = unsafe { canvas(fb) } {
        text::draw(&mut canvas, clip, x, y, text, c, &font, f.size)
    } else {
        0
    }
}

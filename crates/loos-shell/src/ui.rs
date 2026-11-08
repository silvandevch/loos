//! Immediate-mode widget toolkit (C `ui.c`): widgets, focus, accessibility, clipping.
#![allow(missing_docs)]

use crate::compositor::{KEY_BACKSPACE, KEY_DELETE, KEY_DOWN, KEY_END, KEY_ENTER, KEY_HOME, KEY_LEFT, KEY_PGDN, KEY_PGUP, KEY_RIGHT, KEY_SPACE, KEY_TAB, KEY_UP, MOD_SHIFT};
use crate::draw::{mix, Rect, Theme};
use crate::ffi::{self, Fb};
use crate::font::ShellFont;
use crate::font_data;
use crate::text;
use core::ffi::{c_char, c_void};

pub const UI_KEYQ: usize = 128;
pub const UI_A11Y_MAX: usize = 192;

pub const A11Y_LABEL: i32 = 0;
pub const A11Y_BUTTON: i32 = 1;
pub const A11Y_TOGGLE: i32 = 2;
pub const A11Y_RADIO: i32 = 3;
pub const A11Y_TEXT: i32 = 4;
pub const A11Y_LIST: i32 = 5;
pub const A11Y_TAB: i32 = 6;
pub const A11Y_SLIDER: i32 = 7;
pub const A11Y_PROGRESS: i32 = 8;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct A11yItem {
    pub role: i32,
    pub name: [u8; 48],
    pub value: [u8; 48],
    pub rect: Rect,
    pub focused: bool,
    pub checked: bool,
    pub id: u32,
}

impl A11yItem {
    pub const EMPTY: Self = Self { role: 0, name: [0; 48], value: [0; 48], rect: Rect::new(0, 0, 0, 0), focused: false, checked: false, id: 0 };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyInput {
    pub key: u32,
    pub mods: u32,
    pub is_char: bool,
}

impl KeyInput {
    pub const EMPTY: Self = Self { key: 0, mods: 0, is_char: false };
}

/// C `ui_ctx`.
#[repr(C)]
#[allow(missing_docs)]
pub struct UiCtx {
    pub fb: *mut Fb,
    pub theme: *const Theme,
    pub clip: Rect,
    pub mx: i32,
    pub my: i32,
    pub down: bool,
    pub pressed: bool,
    pub released: bool,
    pub wheel: i32,
    pub keys: [KeyInput; UI_KEYQ],
    pub nkeys: i32,
    pub active: u32,
    pub focus: u32,
    pub hot: u32,
    pub next_hot: u32,
    pub consumed_click: bool,
    pub scale: i32,
    pub mods: u32,
    pub frame: u32,
    pub now_ms: u32,
    pub lc_id: u32,
    pub lc_ms: u32,
    pub lc_row: i32,
    pub a11y: [A11yItem; UI_A11Y_MAX],
    pub na11y: i32,
    pub order: [u32; UI_A11Y_MAX],
    pub norder: i32,
    pub reduce_motion: bool,
    pub high_contrast: bool,
    pub mask_next: bool,
}

pub type RowFn = unsafe extern "C" fn(*mut UiCtx, Rect, i32, bool, *mut c_void);

fn t(u: &UiCtx) -> &Theme {
    // SAFETY: the caller keeps the theme alive for the frame (ui_begin contract).
    unsafe { &*u.theme }
}
fn contains(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}
fn bytes0(a: &[u8]) -> &[u8] {
    let n = a.iter().position(|&b| b == 0).unwrap_or(a.len());
    &a[..n]
}
fn put(dst: &mut [u8], src: &[u8]) {
    dst.fill(0);
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
}

fn fw(s: &[u8]) -> i32 {
    text::width(&font_data::regular(), 14, bytes0(s))
}
fn draw_text(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: &[u8], col: u32) {
    let mut b = [0u8; 256];
    let n = bytes0(s).len().min(255);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    unsafe { ffi::fb_text(fb, clip, x, y, b.as_ptr() as *const c_char, col) };
}
fn draw_text_ex(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: &[u8], col: u32, size: i32) {
    let mut b = [0u8; 256];
    let n = bytes0(s).len().min(255);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    let f = ShellFont { face: core::ptr::addr_of!(font_data::loos_font_semibold), size };
    unsafe { ffi::fb_text_ex(fb, clip, x, y, b.as_ptr() as *const c_char, col, f) };
}

fn a11y_reg(u: &mut UiCtx, id: u32, role: i32, name: &[u8], value: &[u8], r: Rect, checked: bool, focusable: bool) {
    if (u.na11y as usize) < UI_A11Y_MAX {
        let it = &mut u.a11y[u.na11y as usize];
        u.na11y += 1;
        *it = A11yItem::EMPTY;
        it.role = role;
        it.id = id;
        it.rect = r;
        it.checked = checked;
        it.focused = id != 0 && u.focus == id;
        put(&mut it.name, bytes0(name));
        put(&mut it.value, bytes0(value));
    }
    if focusable && (u.norder as usize) < UI_A11Y_MAX {
        u.order[u.norder as usize] = id;
        u.norder += 1;
    }
}

pub fn id(s: &[u8]) -> u32 {
    let mut h = 2_166_136_261u32;
    for &b in bytes0(s) {
        h ^= b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    if h == 0 { 1 } else { h }
}

pub fn begin(u: &mut UiCtx, fb: *mut Fb, theme: *const Theme, scale: i32) {
    u.na11y = 0;
    u.norder = 0;
    u.fb = fb;
    u.theme = theme;
    u.scale = 100;
    if scale > 0 && scale != 100 && !fb.is_null() && unsafe { (*fb).scale } == 0 {
        unsafe { (*fb).scale = scale };
    }
    let (w, h) = if fb.is_null() {
        (0, 0)
    } else {
        let sc = if unsafe { (*fb).scale } > 0 { unsafe { (*fb).scale } } else { 100 };
        (unsafe { (*fb).w } * 100 / sc, unsafe { (*fb).h } * 100 / sc)
    };
    u.clip = Rect::new(0, 0, w, h);
    u.hot = u.next_hot;
    u.next_hot = 0;
    u.frame = u.frame.wrapping_add(1);
}

pub fn end(u: &mut UiCtx) {
    for i in 0..u.nkeys as usize {
        if !u.keys[i].is_char && u.keys[i].key == KEY_TAB {
            focus_next(u, (u.keys[i].mods & MOD_SHIFT) != 0);
            break;
        }
    }
    u.nkeys = 0;
    if u.released {
        u.active = 0;
    }
    u.pressed = false;
    u.released = false;
    u.wheel = 0;
    u.consumed_click = false;
}

pub fn feed(u: &mut UiCtx, e: &crate::compositor::ShellEvent) {
    use crate::compositor::ev;
    match e.ev_type {
        ev::MOUSE_MOVE => {
            u.mx = e.x;
            u.my = e.y;
            u.mods = e.mods;
        }
        ev::MOUSE_DOWN => {
            u.mx = e.x;
            u.my = e.y;
            u.mods = e.mods;
            if e.button == 1 {
                u.down = true;
                u.pressed = true;
            }
        }
        ev::MOUSE_UP => {
            u.mx = e.x;
            u.my = e.y;
            if e.button == 1 {
                u.down = false;
                u.released = true;
            }
        }
        ev::MOUSE_WHEEL => u.wheel += e.button,
        ev::KEY_DOWN | ev::CHAR => {
            u.mods = e.mods;
            if (u.nkeys as usize) < UI_KEYQ {
                let k = &mut u.keys[u.nkeys as usize];
                k.key = e.key;
                k.mods = e.mods;
                k.is_char = e.ev_type == ev::CHAR;
                u.nkeys += 1;
            }
        }
        ev::BLUR => {
            u.down = false;
            u.active = 0;
        }
        _ => {}
    }
}

pub fn focus_next(u: &mut UiCtx, back: bool) {
    if u.norder == 0 {
        return;
    }
    let n = u.norder as usize;
    let mut cur = -1i32;
    for i in 0..n {
        if u.order[i] == u.focus {
            cur = i as i32;
        }
    }
    let next = if cur < 0 {
        if back { n - 1 } else { 0 }
    } else {
        ((cur + if back { n as i32 - 1 } else { 1 }) % n as i32) as usize
    };
    u.focus = u.order[next];
}

pub fn take_key(u: &mut UiCtx, key: u32, need_mods: u32) -> bool {
    for i in 0..u.nkeys as usize {
        if u.keys[i].is_char {
            continue;
        }
        let mut k = u.keys[i].key;
        if (b'A' as u32..=b'Z' as u32).contains(&k) {
            k += 32;
        }
        if k == key && (u.keys[i].mods & need_mods) == need_mods {
            let n = u.nkeys as usize - i - 1;
            u.keys.copy_within(i + 1..i + 1 + n, i);
            u.nkeys -= 1;
            return true;
        }
    }
    false
}

pub fn pop_key(u: &mut UiCtx, want_char: bool) -> Option<(u32, u32)> {
    for i in 0..u.nkeys as usize {
        if u.keys[i].is_char != want_char {
            continue;
        }
        if !want_char && u.keys[i].key == KEY_TAB {
            continue;
        }
        let out = (u.keys[i].key, u.keys[i].mods);
        let n = u.nkeys as usize - i - 1;
        u.keys.copy_within(i + 1..i + 1 + n, i);
        u.nkeys -= 1;
        return Some(out);
    }
    None
}

pub fn hovered(u: &UiCtx, r: Rect) -> bool {
    contains(r, u.mx, u.my) && contains(u.clip, u.mx, u.my)
}

pub fn clip_push(u: &mut UiCtx, r: Rect) -> Rect {
    let saved = u.clip;
    u.clip = u.clip.intersect(r);
    saved
}
pub fn clip_pop(u: &mut UiCtx, saved: Rect) {
    u.clip = saved;
}

/// Vertical scroll bar for a view of `content_h` pixels shown in `view`: mouse wheel over the view, drag the thumb or click the
/// track to move `scroll` (pixels from the top, clamped here). Draws nothing when the content fits. Call it after the content
/// is drawn, inside the view's clip, so the bar sits on top.
pub fn vscroll(u: &mut UiCtx, id_s: &[u8], view: Rect, content_h: i32, scroll: &mut i32) {
    let max = (content_h - view.h).max(0);
    if hovered(u, view) && u.wheel != 0 {
        *scroll -= u.wheel * 24;
    }
    *scroll = (*scroll).clamp(0, max);
    if max == 0 {
        return;
    }
    let th = *t(u);
    let track = Rect::new(view.x + view.w - 10, view.y + 2, 8, view.h - 4);
    let thumb_h = (track.h * view.h / content_h).clamp(28.min(track.h), track.h);
    let span = (track.h - thumb_h).max(1);
    let wid = id(id_s) ^ 0x5c01_1ba0;
    click_logic(u, wid, track);
    if u.active == wid {
        *scroll = ((u.my - track.y - thumb_h / 2).clamp(0, span) * max / span).clamp(0, max);
    }
    let ty = track.y + span * *scroll / max;
    let thumb = Rect::new(track.x, ty, track.w, thumb_h);
    let lit = u.active == wid || hovered(u, track);
    unsafe {
        ffi::fb_fill_round(u.fb, u.clip, track, 4, mix(th.surface, th.border, 90));
        ffi::fb_fill_round(u.fb, u.clip, thumb, 4, if lit { th.accent } else { th.text_dim });
    }
}

fn click_logic(u: &mut UiCtx, id: u32, r: Rect) -> bool {
    let over = hovered(u, r);
    if over {
        u.next_hot = id;
    }
    let mut clicked = false;
    if u.pressed && over && !u.consumed_click {
        u.active = id;
        u.focus = id;
        u.consumed_click = true;
    }
    if u.released && u.active == id {
        clicked = over;
        u.active = 0;
    }
    clicked
}

fn kbd_activate(u: &mut UiCtx, id: u32) -> bool {
    if u.focus != id {
        return false;
    }
    take_key(u, KEY_ENTER, 0) || take_key(u, KEY_SPACE, 0)
}

fn focus_ring(u: &UiCtx, r: Rect) {
    let th = t(u);
    unsafe {
        ffi::fb_frame(u.fb, u.clip, r, th.accent);
        if u.high_contrast {
            ffi::fb_frame(u.fb, u.clip, Rect::new(r.x - 1, r.y - 1, r.w + 2, r.h + 2), th.accent);
        }
    }
}

fn button_impl(u: &mut UiCtx, r: Rect, label: &[u8], accent: bool) -> bool {
    let id = id(label) ^ (r.x * 31 + r.y * 17) as u32;
    let mut clicked = click_logic(u, id, r);
    a11y_reg(u, id, A11Y_BUTTON, label, b"", r, false, true);
    clicked |= kbd_activate(u, id);
    let th = *t(u);
    let hot = u.hot == id || u.next_hot == id;
    let act = u.active == id;
    let bg = if accent {
        if act {
            mix(th.accent, 0xFF00_0000, 60)
        } else if hot {
            mix(th.accent, 0xFFFF_FFFF, 40)
        } else {
            th.accent
        }
    } else if act {
        mix(th.surface_hi, th.accent, 90)
    } else if hot {
        mix(th.surface_hi, th.accent, 40)
    } else {
        th.surface_hi
    };
    unsafe { ffi::fb_fill_round(u.fb, u.clip, r, 6, bg) };
    if u.focus == id {
        focus_ring(u, r);
    }
    let tw = fw(label);
    draw_text(u.fb, u.clip, r.x + (r.w - tw) / 2, r.y + (r.h - 16) / 2, label, if accent { th.accent_text } else { th.text });
    clicked
}

pub fn button(u: &mut UiCtx, r: Rect, label: &[u8]) -> bool {
    button_impl(u, r, label, false)
}
pub fn button_accent(u: &mut UiCtx, r: Rect, label: &[u8]) -> bool {
    button_impl(u, r, label, true)
}

pub fn toggle(u: &mut UiCtx, r: Rect, label: &[u8], value: &mut bool) -> bool {
    let id = id(label) ^ (r.x * 31 + r.y * 17) as u32;
    let mut clicked = click_logic(u, id, r);
    a11y_reg(u, id, A11Y_TOGGLE, label, b"", r, *value, true);
    clicked |= kbd_activate(u, id);
    let th = *t(u);
    let tw = 36 * u.scale / 100;
    let tht = 20 * u.scale / 100;
    let sw = Rect::new(r.x + r.w - tw, r.y + (r.h - tht) / 2, tw, tht);
    if clicked {
        *value = !*value;
    }
    let on = *value;
    draw_text(u.fb, u.clip, r.x, r.y + (r.h - 16) / 2, label, th.text);
    unsafe { ffi::fb_fill_round(u.fb, u.clip, sw, tht / 2, if on { th.accent } else { th.border }) };
    let kd = tht - 6;
    let knob = Rect::new(if on { sw.x + sw.w - kd - 3 } else { sw.x + 3 }, sw.y + 3, kd, kd);
    unsafe { ffi::fb_fill_round(u.fb, u.clip, knob, kd / 2, 0xFFFF_FFFF) };
    if u.focus == id {
        focus_ring(u, Rect::new(sw.x - 2, sw.y - 2, sw.w + 4, sw.h + 4));
    }
    clicked
}

pub fn radio(u: &mut UiCtx, r: Rect, label: &[u8], group: &mut i32, value: i32) -> bool {
    let id = id(label) ^ (r.x * 31 + r.y * 17) as u32;
    let mut clicked = click_logic(u, id, r);
    a11y_reg(u, id, A11Y_RADIO, label, b"", r, *group == value, true);
    clicked |= kbd_activate(u, id);
    let th = *t(u);
    if clicked {
        *group = value;
    }
    let dot = Rect::new(r.x, r.y + (r.h - 16) / 2, 16, 16);
    unsafe {
        ffi::fb_fill_round(u.fb, u.clip, dot, 8, th.border);
        ffi::fb_fill_round(u.fb, u.clip, Rect::new(dot.x + 2, dot.y + 2, 12, 12), 6, th.surface);
        if *group == value {
            ffi::fb_fill_round(u.fb, u.clip, Rect::new(dot.x + 4, dot.y + 4, 8, 8), 4, th.accent);
        }
    }
    if u.focus == id {
        focus_ring(u, Rect::new(r.x - 2, r.y, 20, r.h));
    }
    draw_text(u.fb, u.clip, r.x + 24, r.y + (r.h - 16) / 2, label, th.text);
    clicked
}

pub fn textfield_id(placeholder: &[u8], r: Rect) -> u32 {
    let p = if placeholder.is_empty() { b"textfield".as_slice() } else { placeholder };
    id(p) ^ (r.x * 31 + r.y * 17) as u32
}

fn prefix_w(s: &[u8], from: i32, to: i32) -> i32 {
    let n = to - from;
    if n <= 0 {
        return 0;
    }
    let from = from.max(0) as usize;
    let to = (from + n as usize).min(s.len());
    if from >= to {
        return 0;
    }
    fw(&s[from..to])
}

fn text_off(s: &[u8], caret: i32, avail: i32, step: i32) -> i32 {
    let mut off = 0;
    while off < caret && prefix_w(s, off, caret) > avail - 2 {
        off += step;
    }
    off
}

fn shown_text<'a>(buf: &'a [u8], mask: bool, disp: &'a mut [u8; 292]) -> &'a [u8] {
    if !mask {
        return bytes0(buf);
    }
    let mut n = 0usize;
    for &b in buf {
        if b == 0 || n + 4 > disp.len() {
            break;
        }
        disp[n..n + 3].copy_from_slice(b"\xE2\x80\xA2");
        n += 3;
    }
    disp[n] = 0;
    &disp[..n]
}

pub fn textfield(u: &mut UiCtx, r: Rect, buf: &mut [u8], caret: &mut i32, placeholder: &[u8]) -> bool {
    let cap = buf.len();
    let id = textfield_id(placeholder, r);
    let mask = u.mask_next;
    u.mask_next = false;
    let step = if mask { 3 } else { 1 };
    let mut disp = [0u8; 292];
    click_logic(u, id, r);
    let demo = placeholder;
    a11y_reg(u, id, A11Y_TEXT, if demo.is_empty() { b"text" } else { demo }, if mask { b"" } else { bytes0(buf) }, r, false, true);
    let th = *t(u);
    let focused = u.focus == id;
    let mut entered = false;
    let mut len = bytes0(buf).len() as i32;
    if *caret > len {
        *caret = len;
    }
    if *caret < 0 {
        *caret = 0;
    }
    if u.pressed && hovered(u, r) {
        let sh = shown_text(buf, mask, &mut disp);
        let off0 = text_off(sh, *caret * step, r.w - 16, step);
        let idx = {
            let s = &sh[(off0 as usize).min(sh.len())..];
            crate::font::index_at(&font_data::regular(), 14, s, u.mx - (r.x + 8)) as i32
        };
        let pos = (off0 + idx) / step;
        *caret = pos.max(0).min(len);
    }
    if focused {
        let mut ki = 0usize;
        while ki < u.nkeys as usize {
            let k = u.keys[ki].key;
            let ch = u.keys[ki].is_char;
            if !ch && k == KEY_TAB {
                ki += 1;
                continue;
            }
            let n = u.nkeys as usize - ki - 1;
            u.keys.copy_within(ki + 1..ki + 1 + n, ki);
            u.nkeys -= 1;
            if ch {
                if (32..127).contains(&k) && (len as usize + 1) < cap {
                    let c = *caret as usize;
                    buf.copy_within(c..(len as usize + 1), c + 1);
                    buf[c] = k as u8;
                    *caret += 1;
                    len += 1;
                }
            } else if k == KEY_BACKSPACE && *caret > 0 {
                let c = *caret as usize;
                buf.copy_within(c..(len as usize + 1), c - 1);
                *caret -= 1;
                len -= 1;
            } else if k == KEY_DELETE && *caret < len {
                let c = *caret as usize;
                buf.copy_within(c + 1..(len as usize + 1), c);
                len -= 1;
            } else if k == KEY_LEFT && *caret > 0 {
                *caret -= 1;
            } else if k == KEY_RIGHT && *caret < len {
                *caret += 1;
            } else if k == KEY_HOME {
                *caret = 0;
            } else if k == KEY_END {
                *caret = len;
            } else if k == KEY_ENTER {
                entered = true;
            }
        }
    }
    let f = u.fb;
    unsafe {
        ffi::fb_fill_round(f, u.clip, r, 6, th.surface_hi);
        ffi::fb_frame(f, u.clip, r, if focused { th.accent } else { th.border });
        if focused && u.high_contrast {
            ffi::fb_frame(f, u.clip, Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2), th.accent);
        }
    }
    let saved = clip_push(u, Rect::new(r.x + 2, r.y + 2, r.w - 4, r.h - 4));
    let ty = r.y + (r.h - 16) / 2;
    let sh = shown_text(buf, mask, &mut disp);
    let off = text_off(sh, *caret * step, r.w - 16, step);
    if len != 0 {
        draw_text(u.fb, u.clip, r.x + 8, ty, &sh[(off as usize).min(sh.len())..], th.text);
    } else if !placeholder.is_empty() {
        draw_text(u.fb, u.clip, r.x + 8, ty, placeholder, th.text_dim);
    }
    if focused && (u.reduce_motion || ((u.frame / 30) & 1) == 0) {
        let cx = r.x + 8 + prefix_w(sh, off, *caret * step);
        unsafe { ffi::fb_fill(u.fb, u.clip, Rect::new(cx, ty, 1, 16), th.text) };
    }
    clip_pop(u, saved);
    entered
}

pub fn textfield_password(u: &mut UiCtx, r: Rect, buf: &mut [u8], caret: &mut i32, placeholder: &[u8]) -> bool {
    u.mask_next = true;
    textfield(u, r, buf, caret, placeholder)
}

pub fn tabs(u: &mut UiCtx, r: Rect, labels: *const *const c_char, n: i32, cur: &mut i32) -> i32 {
    let th = *t(u);
    let mut x = r.x;
    let mut picked = -1;
    unsafe { ffi::fb_hline(u.fb, u.clip, r.x, r.y + r.h - 1, r.w, th.border) };
    for i in 0..n {
        let label = unsafe { ffi::c_bytes(*labels.add(i as usize)) };
        let w = fw(label) + 28;
        let tr = Rect::new(x, r.y, w, r.h);
        let id = id(label) ^ 0x7ab5;
        let mut tc = click_logic(u, id, tr);
        a11y_reg(u, id, A11Y_TAB, label, b"", tr, *cur == i, true);
        tc |= kbd_activate(u, id);
        if tc {
            if *cur != i {
                picked = i;
            }
            *cur = i;
        }
        let sel = *cur == i;
        let hot = u.hot == id || u.next_hot == id;
        if hot && !sel {
            unsafe { ffi::fb_fill_round(u.fb, u.clip, tr, 6, th.surface_hi) };
        }
        draw_text(u.fb, u.clip, x + 14, r.y + (r.h - 16) / 2 - 1, label, if sel { th.text } else { th.text_dim });
        if sel {
            unsafe { ffi::fb_fill(u.fb, u.clip, Rect::new(x + 8, r.y + r.h - 3, w - 16, 3), th.accent) };
        }
        x += w;
    }
    picked
}

pub fn progress(u: &mut UiCtx, r: Rect, pct_in: i32) {
    let mut vs = [0u8; 16];
    let mut n = 0;
    let mut p = pct_in;
    if p < 0 {
        p = 0;
    }
    if p > 100 {
        p = 100;
    }
    if p / 100 != 0 {
        vs[n] = b'0' + (p / 100) as u8;
        n += 1;
    }
    if p >= 10 {
        vs[n] = b'0' + (p / 10 % 10) as u8;
        n += 1;
    }
    vs[n] = b'0' + (p % 10) as u8;
    n += 1;
    vs[n] = b'%';
    n += 1;
    a11y_reg(u, 0, A11Y_PROGRESS, b"progress", &vs[..n], r, false, false);
    let th = *t(u);
    unsafe { ffi::fb_fill_round(u.fb, u.clip, r, r.h / 2, th.border) };
    if p != 0 {
        let w = (r.w * p / 100).max(r.h);
        unsafe { ffi::fb_fill_round(u.fb, u.clip, Rect::new(r.x, r.y, w, r.h), r.h / 2, th.accent) };
    }
}

pub fn slider(u: &mut UiCtx, r: Rect, id_s: &[u8], value: &mut i32, lo: i32, hi: i32) -> bool {
    let id = id(id_s) ^ (r.x * 31 + r.y * 17) as u32;
    click_logic(u, id, r);
    let mut vs = [0u8; 16];
    let mut n = 0;
    let v = *value;
    if v < 0 {
        vs[n] = b'-';
        n += 1;
    }
    let mut a = v.unsigned_abs();
    let mut tmp = [0u8; 12];
    let mut k = 0;
    if a == 0 {
        tmp[0] = b'0';
        k = 1;
    }
    while a != 0 {
        tmp[k] = b'0' + (a % 10) as u8;
        k += 1;
        a /= 10;
    }
    for i in (0..k).rev() {
        vs[n] = tmp[i];
        n += 1;
    }
    a11y_reg(u, id, A11Y_SLIDER, id_s, &vs[..n], r, false, true);
    let mut changed = false;
    if u.focus == id {
        if take_key(u, KEY_RIGHT, 0) && *value < hi {
            *value += 1;
            changed = true;
        }
        if take_key(u, KEY_LEFT, 0) && *value > lo {
            *value -= 1;
            changed = true;
        }
    }
    if u.active == id && hi > lo {
        let mut v = lo + (u.mx - r.x) * (hi - lo) / if r.w > 1 { r.w - 1 } else { 1 };
        if v < lo {
            v = lo;
        }
        if v > hi {
            v = hi;
        }
        if v != *value {
            *value = v;
            changed = true;
        }
    }
    let th = *t(u);
    let track = Rect::new(r.x, r.y + r.h / 2 - 2, r.w, 4);
    unsafe { ffi::fb_fill_round(u.fb, u.clip, track, 2, th.border) };
    let kx = if hi > lo { r.x + (*value - lo) * (r.w - 14) / (hi - lo) } else { r.x };
    unsafe {
        ffi::fb_fill_round(u.fb, u.clip, Rect::new(r.x, track.y, kx - r.x + 7, 4), 2, th.accent);
        ffi::fb_fill_round(u.fb, u.clip, Rect::new(kx, r.y + r.h / 2 - 7, 14, 14), 7, 0xFFFF_FFFF);
    }
    changed
}

pub fn list(u: &mut UiCtx, r: Rect, id_s: &[u8], n: i32, row_h: i32, draw: Option<RowFn>, user: *mut c_void, sel: &mut i32, scroll: &mut i32) -> i32 {
    let id = id(id_s);
    let th = *t(u);
    {
        let mut vs = [0u8; 16];
        let mut k = 0;
        let mut a = *sel + 1;
        if a < 0 {
            vs[k] = b'-';
            k += 1;
            a = -a;
        }
        let mut tmp = [0u8; 12];
        let mut m = 0;
        let mut aa = a as u32;
        if aa == 0 {
            tmp[0] = b'0';
            m = 1;
        }
        while aa != 0 {
            tmp[m] = b'0' + (aa % 10) as u8;
            m += 1;
            aa /= 10;
        }
        for i in (0..m).rev() {
            vs[k] = tmp[i];
            k += 1;
        }
        vs[k] = b'/';
        k += 1;
        let mut b = n;
        let mut mm = 0;
        if b < 0 {
            b = 0;
        }
        let mut bb = b as u32;
        if bb == 0 {
            tmp[0] = b'0';
            mm = 1;
        }
        while bb != 0 {
            tmp[mm] = b'0' + (bb % 10) as u8;
            mm += 1;
            bb /= 10;
        }
        for i in (0..mm).rev() {
            vs[k] = tmp[i];
            k += 1;
        }
        a11y_reg(u, id, A11Y_LIST, id_s, &vs[..k], r, false, true);
    }
    let visible = r.h / row_h;
    let mut activated = -1;
    let maxscroll = if n > visible { n - visible } else { 0 };
    let over = hovered(u, r);
    if over && u.wheel != 0 {
        *scroll -= u.wheel;
    }
    if u.pressed && over && !u.consumed_click {
        u.focus = id;
        u.consumed_click = true;
        let row = *scroll + (u.my - r.y) / row_h;
        if row >= 0 && row < n {
            if u.lc_id == id && u.lc_row == row && u.now_ms.wrapping_sub(u.lc_ms) <= 400 {
                activated = row;
                u.lc_id = 0;
            } else {
                u.lc_id = id;
                u.lc_row = row;
                u.lc_ms = u.now_ms;
            }
            *sel = row;
        }
    }
    if u.focus == id {
        while let Some((k, _m)) = pop_key(u, false) {
            if k == KEY_DOWN && *sel + 1 < n {
                *sel += 1;
            } else if k == KEY_UP && *sel > 0 {
                *sel -= 1;
            } else if k == KEY_HOME {
                *sel = 0;
            } else if k == KEY_END {
                *sel = n - 1;
            } else if k == KEY_PGDN {
                *sel = if *sel + visible < n { *sel + visible } else { n - 1 };
            } else if k == KEY_PGUP {
                *sel = if *sel - visible > 0 { *sel - visible } else { 0 };
            } else if k == KEY_ENTER && *sel >= 0 && *sel < n {
                activated = *sel;
            }
            if *sel < *scroll {
                *scroll = *sel;
            }
            if *sel >= *scroll + visible {
                *scroll = *sel - visible + 1;
            }
        }
    }
    if *scroll > maxscroll {
        *scroll = maxscroll;
    }
    if *scroll < 0 {
        *scroll = 0;
    }
    unsafe { ffi::fb_fill_round(u.fb, u.clip, r, 6, th.surface) };
    let saved = clip_push(u, r);
    let mut i = 0;
    while i < visible + 1 && *scroll + i < n {
        let idx = *scroll + i;
        let row = Rect::new(r.x, r.y + i * row_h, r.w - if maxscroll != 0 { 8 } else { 0 }, row_h);
        let selected = idx == *sel;
        if selected {
            let col = if u.focus == id { mix(th.surface_hi, th.accent, 110) } else { th.surface_hi };
            unsafe { ffi::fb_fill(u.fb, u.clip, row, col) };
        } else if over && contains(row, u.mx, u.my) {
            unsafe { ffi::fb_fill(u.fb, u.clip, row, mix(th.surface, th.surface_hi, 128)) };
        }
        if let Some(f) = draw {
            // SAFETY: host callback; `u` stays live for the call.
            unsafe { f(u as *mut UiCtx, row, idx, selected, user) };
        }
        i += 1;
    }
    if maxscroll != 0 {
        let sb = Rect::new(r.x + r.w - 6, r.y + 2, 4, r.h - 4);
        unsafe { ffi::fb_fill_round(u.fb, u.clip, sb, 2, th.border) };
        let mut tht = sb.h * visible / n;
        if tht < 16 {
            tht = 16;
        }
        let ty = sb.y + (sb.h - tht) * *scroll / maxscroll;
        unsafe { ffi::fb_fill_round(u.fb, u.clip, Rect::new(sb.x, ty, 4, tht), 2, th.text_dim) };
    }
    clip_pop(u, saved);
    activated
}

pub fn a11y_dump(u: &UiCtx, out: *mut u8, cap: usize) -> usize {
    const ROLE: [&[u8]; 9] = [b"label", b"button", b"toggle", b"radio", b"text", b"list", b"tab", b"slider", b"progress"];
    let mut w = Out { ptr: out, cap, len: 0 };
    for i in 0..u.na11y as usize {
        let it = &u.a11y[i];
        w.put(ROLE[it.role as usize]);
        w.put(b": ");
        w.put(bytes0(&it.name));
        if it.role == A11Y_TOGGLE || it.role == A11Y_RADIO {
            w.put(if it.checked { b" [on]" } else { b" [off]" });
        }
        if it.value[0] != 0 {
            w.put(b" = ");
            w.put(bytes0(&it.value));
        }
        if it.focused {
            w.put(b" (focused)");
        }
        w.put(b"\n");
    }
    w.finish();
    w.len
}

struct Out {
    ptr: *mut u8,
    cap: usize,
    len: usize,
}
impl Out {
    fn put(&mut self, s: &[u8]) {
        if self.len < self.cap {
            let n = s.len().min(self.cap - self.len);
            for i in 0..n {
                // SAFETY: caller provided cap bytes.
                unsafe { *self.ptr.add(self.len + i) = s[i] };
            }
        }
        self.len += s.len();
    }
    fn finish(&mut self) {
        if self.cap > 0 {
            let n = self.len.min(self.cap - 1);
            // SAFETY: within the buffer.
            unsafe { *self.ptr.add(n) = 0 };
        }
    }
}

pub fn paragraph(u: &mut UiCtx, x: i32, y: i32, w: i32, s: &[u8], c: u32) -> i32 {
    let y0 = y;
    let mut off = 0usize;
    let mut yy = y;
    let s = bytes0(s);
    while off < s.len() {
        let mut n = 0usize;
        let mut cut = 0usize;
        while off + n < s.len() && n < 255 {
            let cand = &s[off..off + n + 1];
            if fw(cand) > w {
                break;
            }
            n += 1;
            let next = s.get(off + n).copied();
            if next == Some(b' ') || next.is_none() {
                cut = n;
            }
        }
        if cut == 0 {
            cut = if n != 0 { n } else { 1 };
        }
        while cut > 0 && off + cut < s.len() && (s[off + cut] & 0xc0) == 0x80 {
            cut -= 1;
        }
        if cut == 0 {
            cut = 1;
            while off + cut < s.len() && (s[off + cut] & 0xc0) == 0x80 {
                cut += 1;
            }
        }
        label(u, x, yy, &s[off..off + cut], c);
        yy += 22;
        off += cut;
        while off < s.len() && s[off] == b' ' {
            off += 1;
        }
    }
    yy - y0
}

pub fn label(u: &mut UiCtx, x: i32, y: i32, s: &[u8], c: u32) {
    let s = bytes0(s);
    a11y_reg(u, 0, A11Y_LABEL, s, b"", Rect::new(x, y, fw(s), 16), false, false);
    draw_text(u.fb, u.clip, x, y, s, c);
}
pub fn label_right(u: &mut UiCtx, rx: i32, y: i32, s: &[u8], c: u32) {
    let s = bytes0(s);
    draw_text(u.fb, u.clip, rx - fw(s), y, s, c);
}
pub fn heading(u: &mut UiCtx, x: i32, y: i32, s: &[u8]) {
    let th = *t(u);
    draw_text_ex(u.fb, u.clip, x, y, s, th.text, 26);
}
pub fn separator(u: &mut UiCtx, x: i32, y: i32, w: i32) {
    let th = *t(u);
    unsafe { ffi::fb_hline(u.fb, u.clip, x, y, w, th.border) };
}
pub fn panel(u: &mut UiCtx, r: Rect, c: u32) {
    unsafe { ffi::fb_fill_round(u.fb, u.clip, r, 6, c) };
}
pub fn clear(u: &mut UiCtx, c: u32) {
    let (w, h) = if u.fb.is_null() {
        (0, 0)
    } else {
        let sc = if unsafe { (*u.fb).scale } > 0 { unsafe { (*u.fb).scale } } else { 100 };
        (unsafe { (*u.fb).w } * 100 / sc, unsafe { (*u.fb).h } * 100 / sc)
    };
    unsafe { ffi::fb_fill(u.fb, u.clip, Rect::new(0, 0, w, h), c) };
}
pub fn backdrop(u: &mut UiCtx, w: i32, h: i32) {
    let all = Rect::new(0, 0, w, h);
    unsafe {
        ffi::fb_gradient_v(u.fb, u.clip, all, 0xFF0D_1320, 0xFF07_0A12);
        ffi::fb_radial_glow(u.fb, u.clip, w * 80 / 100, h * 12 / 100, w * 60 / 100, 0xFF3B_82F6, 56);
        ffi::fb_radial_glow(u.fb, u.clip, w * 10 / 100, h * 98 / 100, w * 48 / 100, 0xFF7C_3AED, 40);
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn ui_id(s: *const c_char) -> u32 {
    id(unsafe { ffi::c_bytes(s) })
}
#[no_mangle]
pub unsafe extern "C" fn ui_begin(u: *mut UiCtx, fb: *mut Fb, theme: *const Theme, scale: i32) {
    if let Some(u) = unsafe { u.as_mut() } {
        begin(u, fb, theme, scale);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_feed(u: *mut UiCtx, e: *const crate::compositor::ShellEvent) {
    if let (Some(u), Some(e)) = (unsafe { u.as_mut() }, unsafe { e.as_ref() }) {
        feed(u, e);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_end(u: *mut UiCtx) {
    if let Some(u) = unsafe { u.as_mut() } {
        end(u);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_key(u: *mut UiCtx, _id: u32, want_char: bool, mods: *mut u32) -> bool {
    let Some(u) = (unsafe { u.as_mut() }) else { return false };
    for i in 0..u.nkeys as usize {
        if u.keys[i].is_char != want_char {
            continue;
        }
        if !mods.is_null() {
            unsafe { *mods = u.keys[i].mods };
        }
        let n = u.nkeys as usize - i - 1;
        u.keys.copy_within(i + 1..i + 1 + n, i);
        u.nkeys -= 1;
        return true;
    }
    false
}
#[no_mangle]
pub unsafe extern "C" fn ui_clear(u: *mut UiCtx, c: u32) {
    if let Some(u) = unsafe { u.as_mut() } {
        clear(u, c);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_label(u: *mut UiCtx, x: i32, y: i32, s: *const c_char, c: u32) {
    if let Some(u) = unsafe { u.as_mut() } {
        label(u, x, y, unsafe { ffi::c_bytes(s) }, c);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_paragraph(u: *mut UiCtx, x: i32, y: i32, w: i32, s: *const c_char, c: u32) -> i32 {
    match unsafe { u.as_mut() } {
        Some(u) => paragraph(u, x, y, w, unsafe { ffi::c_bytes(s) }, c),
        None => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_label_right(u: *mut UiCtx, rx: i32, y: i32, s: *const c_char, c: u32) {
    if let Some(u) = unsafe { u.as_mut() } {
        label_right(u, rx, y, unsafe { ffi::c_bytes(s) }, c);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_heading(u: *mut UiCtx, x: i32, y: i32, s: *const c_char) {
    if let Some(u) = unsafe { u.as_mut() } {
        heading(u, x, y, unsafe { ffi::c_bytes(s) });
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_separator(u: *mut UiCtx, x: i32, y: i32, w: i32) {
    if let Some(u) = unsafe { u.as_mut() } {
        separator(u, x, y, w);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_panel(u: *mut UiCtx, r: Rect, c: u32) {
    if let Some(u) = unsafe { u.as_mut() } {
        panel(u, r, c);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_backdrop(u: *mut UiCtx, w: i32, h: i32) {
    if let Some(u) = unsafe { u.as_mut() } {
        backdrop(u, w, h);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_button(u: *mut UiCtx, r: Rect, label: *const c_char) -> bool {
    unsafe { u.as_mut() }.map_or(false, |u| button(u, r, unsafe { ffi::c_bytes(label) }))
}
#[no_mangle]
pub unsafe extern "C" fn ui_button_accent(u: *mut UiCtx, r: Rect, label: *const c_char) -> bool {
    unsafe { u.as_mut() }.map_or(false, |u| button_accent(u, r, unsafe { ffi::c_bytes(label) }))
}
#[no_mangle]
pub unsafe extern "C" fn ui_toggle(u: *mut UiCtx, r: Rect, label: *const c_char, value: *mut bool) -> bool {
    match (unsafe { u.as_mut() }, unsafe { value.as_mut() }) {
        (Some(u), Some(v)) => toggle(u, r, unsafe { ffi::c_bytes(label) }, v),
        _ => false,
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_radio(u: *mut UiCtx, r: Rect, label: *const c_char, group: *mut i32, value: i32) -> bool {
    match (unsafe { u.as_mut() }, unsafe { group.as_mut() }) {
        (Some(u), Some(g)) => radio(u, r, unsafe { ffi::c_bytes(label) }, g, value),
        _ => false,
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_textfield_id(placeholder: *const c_char, r: Rect) -> u32 {
    textfield_id(unsafe { ffi::c_bytes(placeholder) }, r)
}
#[no_mangle]
pub unsafe extern "C" fn ui_textfield(u: *mut UiCtx, r: Rect, buf: *mut c_char, cap: usize, caret: *mut i32, placeholder: *const c_char) -> bool {
    let (Some(u), Some(c)) = (unsafe { u.as_mut() }, unsafe { caret.as_mut() }) else { return false };
    if buf.is_null() || cap == 0 {
        return false;
    }
    // SAFETY: caller provides cap bytes and a live caret.
    let b = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, cap) };
    textfield(u, r, b, c, unsafe { ffi::c_bytes(placeholder) })
}
#[no_mangle]
pub unsafe extern "C" fn ui_textfield_password(u: *mut UiCtx, r: Rect, buf: *mut c_char, cap: usize, caret: *mut i32, placeholder: *const c_char) -> bool {
    let (Some(u), Some(c)) = (unsafe { u.as_mut() }, unsafe { caret.as_mut() }) else { return false };
    if buf.is_null() || cap == 0 {
        return false;
    }
    // SAFETY: caller provides cap bytes and a live caret.
    let b = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, cap) };
    textfield_password(u, r, b, c, unsafe { ffi::c_bytes(placeholder) })
}
#[no_mangle]
pub unsafe extern "C" fn ui_tabs(u: *mut UiCtx, r: Rect, labels: *const *const c_char, n: i32, current: *mut i32) -> i32 {
    match (unsafe { u.as_mut() }, unsafe { current.as_mut() }) {
        (Some(u), Some(cur)) => tabs(u, r, labels, n, cur),
        _ => -1,
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_progress(u: *mut UiCtx, r: Rect, pct: i32) {
    if let Some(u) = unsafe { u.as_mut() } {
        progress(u, r, pct);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_slider(u: *mut UiCtx, r: Rect, id_s: *const c_char, value: *mut i32, lo: i32, hi: i32) -> bool {
    match (unsafe { u.as_mut() }, unsafe { value.as_mut() }) {
        (Some(u), Some(v)) => slider(u, r, unsafe { ffi::c_bytes(id_s) }, v, lo, hi),
        _ => false,
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_list(u: *mut UiCtx, r: Rect, id_s: *const c_char, n: i32, row_h: i32, draw: Option<RowFn>, user: *mut c_void, sel: *mut i32, scroll: *mut i32) -> i32 {
    match (unsafe { u.as_mut() }, unsafe { sel.as_mut() }, unsafe { scroll.as_mut() }) {
        (Some(u), Some(s), Some(sc)) => list(u, r, unsafe { ffi::c_bytes(id_s) }, n, row_h, draw, user, s, sc),
        _ => -1,
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_a11y_count(u: *const UiCtx) -> i32 {
    unsafe { u.as_ref() }.map_or(0, |u| u.na11y)
}
#[no_mangle]
pub unsafe extern "C" fn ui_a11y_get(u: *const UiCtx, i: i32) -> *const A11yItem {
    match unsafe { u.as_ref() } {
        Some(u) if i >= 0 && i < u.na11y => &u.a11y[i as usize],
        _ => core::ptr::null(),
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_a11y_dump(u: *const UiCtx, out: *mut c_char, cap: usize) -> usize {
    match unsafe { u.as_ref() } {
        Some(u) => a11y_dump(u, out as *mut u8, cap),
        None => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_focus_next(u: *mut UiCtx, back: bool) {
    if let Some(u) = unsafe { u.as_mut() } {
        focus_next(u, back);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_focus(u: *mut UiCtx, id: u32) {
    if let Some(u) = unsafe { u.as_mut() } {
        u.focus = id;
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_focused(u: *const UiCtx, id: u32) -> bool {
    unsafe { u.as_ref() }.map_or(false, |u| id != 0 && u.focus == id)
}
#[no_mangle]
pub unsafe extern "C" fn ui_clip_push(u: *mut UiCtx, r: Rect, saved: *mut Rect) {
    if let (Some(u), Some(s)) = (unsafe { u.as_mut() }, unsafe { saved.as_mut() }) {
        *s = clip_push(u, r);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_clip_pop(u: *mut UiCtx, saved: Rect) {
    if let Some(u) = unsafe { u.as_mut() } {
        clip_pop(u, saved);
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_hovered(u: *const UiCtx, r: Rect) -> bool {
    unsafe { u.as_ref() }.map_or(false, |u| hovered(u, r))
}
#[no_mangle]
pub unsafe extern "C" fn ui_take_key(u: *mut UiCtx, key: u32, need_mods: u32) -> bool {
    unsafe { u.as_mut() }.map_or(false, |u| take_key(u, key, need_mods))
}
#[no_mangle]
pub unsafe extern "C" fn ui_fb(u: *mut UiCtx) -> *mut Fb {
    unsafe { u.as_ref() }.map_or(core::ptr::null_mut(), |u| u.fb)
}
#[no_mangle]
pub unsafe extern "C" fn ui_theme_of(u: *const UiCtx) -> *const Theme {
    unsafe { u.as_ref() }.map_or(core::ptr::null(), |u| u.theme)
}
#[no_mangle]
pub unsafe extern "C" fn ui_pointer(u: *const UiCtx, x: *mut i32, y: *mut i32, pressed: *mut bool, wheel: *mut i32) {
    if let Some(u) = unsafe { u.as_ref() } {
        unsafe {
            if !x.is_null() {
                *x = u.mx;
            }
            if !y.is_null() {
                *y = u.my;
            }
            if !pressed.is_null() {
                *pressed = u.pressed && !u.consumed_click;
            }
            if !wheel.is_null() {
                *wheel = u.wheel;
            }
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_blur(u: *mut UiCtx) {
    if let Some(u) = unsafe { u.as_mut() } {
        u.focus = 0;
    }
}
#[no_mangle]
pub unsafe extern "C" fn ui_next_input(u: *mut UiCtx, key: *mut u32, mods: *mut u32, is_char: *mut bool) -> bool {
    let Some(u) = (unsafe { u.as_mut() }) else { return false };
    if u.nkeys == 0 {
        return false;
    }
    unsafe {
        if !key.is_null() {
            *key = u.keys[0].key;
        }
        if !mods.is_null() {
            *mods = u.keys[0].mods;
        }
        if !is_char.is_null() {
            *is_char = u.keys[0].is_char;
        }
    }
    let n = u.nkeys as usize - 1;
    u.keys.copy_within(1..1 + n, 0);
    u.nkeys -= 1;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<A11yItem>(), 124);
        assert_eq!(offset_of!(A11yItem, name), 4);
        assert_eq!(offset_of!(A11yItem, value), 52);
        assert_eq!(offset_of!(A11yItem, rect), 100);
        assert_eq!(offset_of!(A11yItem, id), 120);
        assert_eq!(size_of::<KeyInput>(), 12);
        assert_eq!(size_of::<UiCtx>(), 26224);
        assert_eq!(offset_of!(UiCtx, theme), 8);
        assert_eq!(offset_of!(UiCtx, clip), 16);
        assert_eq!(offset_of!(UiCtx, mx), 32);
        assert_eq!(offset_of!(UiCtx, down), 40);
        assert_eq!(offset_of!(UiCtx, wheel), 44);
        assert_eq!(offset_of!(UiCtx, keys), 48);
        assert_eq!(offset_of!(UiCtx, nkeys), 1584);
        assert_eq!(offset_of!(UiCtx, active), 1588);
        assert_eq!(offset_of!(UiCtx, scale), 1608);
        assert_eq!(offset_of!(UiCtx, frame), 1616);
        assert_eq!(offset_of!(UiCtx, lc_id), 1624);
        assert_eq!(offset_of!(UiCtx, a11y), 1636);
        assert_eq!(offset_of!(UiCtx, na11y), 25444);
        assert_eq!(offset_of!(UiCtx, order), 25448);
        assert_eq!(offset_of!(UiCtx, norder), 26216);
        assert_eq!(offset_of!(UiCtx, reduce_motion), 26220);
        assert_eq!(offset_of!(UiCtx, mask_next), 26222);
    }

    #[test]
    fn paragraph_keeps_utf8_and_wraps() {
        let u = Box::leak(Box::new(unsafe { core::mem::zeroed::<UiCtx>() }));
        u.fb = core::ptr::null_mut();
        u.clip = Rect::new(0, 0, 300, 400);
        let h = paragraph(u, 0, 0, 60, "caf\u{e9} \u{65e5}\u{672c}\u{8a9e} words here".as_bytes(), 0xFFFF_FFFF);
        assert!(h > 0);
        // Skip drawing with a null fb; the function must not panic on a long unicode run.
        let h = paragraph(u, 0, 0, 30, "\u{65e5}\u{672c}\u{8a9e}\u{65e5}\u{672c}\u{8a9e}".as_bytes(), 0xFFFF_FFFF);
        assert!(h >= 22);
    }

    #[test]
    fn a11y_dump_formats_roles_states_and_focus() {
        let u = Box::leak(Box::new(unsafe { core::mem::zeroed::<UiCtx>() }));
        u.na11y = 2;
        u.a11y[0].role = A11Y_BUTTON;
        put(&mut u.a11y[0].name, b"OK");
        u.a11y[1].role = A11Y_TOGGLE;
        put(&mut u.a11y[1].name, b"Dark mode");
        u.a11y[1].checked = true;
        u.a11y[1].focused = true;
        let mut out = [0u8; 128];
        a11y_dump(u, out.as_mut_ptr(), out.len());
        let want = b"button: OK\ntoggle: Dark mode [on] (focused)\n";
        assert_eq!(&out[..want.len()], want);
    }

    #[test]
    fn textfield_edits_and_masks() {
        let theme = Box::leak(Box::new(Theme::dark(0xFF3B_82F6)));
        let u = Box::leak(Box::new(unsafe { core::mem::zeroed::<UiCtx>() }));
        u.theme = theme;
        u.fb = core::ptr::null_mut();
        u.clip = Rect::new(0, 0, 300, 100);
        u.focus = textfield_id(b"name", Rect::new(10, 10, 200, 28));
        let mut buf = [0u8; 16];
        buf[0] = b'h';
        buf[1] = b'i';
        let mut caret = 2;
        u.keys[0] = KeyInput { key: b'!' as u32, mods: 0, is_char: true };
        u.keys[0].key = b'!' as u32;
        u.keys[1] = KeyInput { key: KEY_LEFT, mods: 0, is_char: false };
        u.nkeys = 2;
        assert!(!textfield(u, Rect::new(10, 10, 200, 28), &mut buf, &mut caret, b"name"));
        assert_eq!(&buf[..4], b"hi!\0");
        // Caret moved left before the '!', so it sits between i and !.
        assert_eq!(caret, 2);
        u.mask_next = true;
        assert!(!textfield(u, Rect::new(10, 10, 200, 28), &mut buf, &mut caret, b""));
        assert!(!u.mask_next);
    }

    #[test]
    fn toggle_flips_on_click() {
        let theme = Box::leak(Box::new(Theme::dark(0xFF3B_82F6)));
        let u = Box::leak(Box::new(unsafe { core::mem::zeroed::<UiCtx>() }));
        u.theme = theme;
        u.fb = core::ptr::null_mut();
        u.clip = Rect::new(0, 0, 300, 100);
        u.pressed = true;
        u.mx = 20;
        u.my = 20;
        let mut on = false;
        assert!(!toggle(u, Rect::new(10, 10, 200, 28), b"Dark", &mut on));
        u.pressed = false;
        u.released = true;
        u.active = id(b"Dark") ^ (10 * 31 + 10 * 17) as u32;
        assert!(toggle(u, Rect::new(10, 10, 200, 28), b"Dark", &mut on));
        assert!(on);
    }
}

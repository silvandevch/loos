//! Window manager (C `wm.c`): monitors, snapping, virtual desktops and tiling.
#![allow(missing_docs)]

use crate::compositor::{
    comp_add_overlay, ev, zone, Compositor, ShellEvent, Window, MOD_SHIFT, MOD_SUPER, WF_MAXIMIZED, WF_NOCHROME, WF_NOFOCUS,
    WF_NORESIZE, WF_SNAPPED, WF_STICKY, WF_TOPMOST,
};
use crate::draw::Rect;
use crate::ffi::{self, Fb};
use core::ffi::{c_char, c_void};

pub const SHELL_MAX_MONITORS: usize = 4;
const SNAP_EDGE: i32 = 6;
const SNAP_CORNER: i32 = 48;

pub type Snap = i32;
pub mod snap {
    use super::Snap;
    pub const NONE: Snap = 0;
    pub const LEFT: Snap = 1;
    pub const RIGHT: Snap = 2;
    pub const MAXIMIZE: Snap = 3;
    pub const TL: Snap = 4;
    pub const TR: Snap = 5;
    pub const BL: Snap = 6;
    pub const BR: Snap = 7;
    pub const THIRD_L: Snap = 8;
    pub const THIRD_C: Snap = 9;
    pub const THIRD_R: Snap = 10;
    pub const TWO_THIRDS_L: Snap = 11;
    pub const TWO_THIRDS_R: Snap = 12;
}

pub type FocusMode = i32;
pub const FOCUS_CLICK: FocusMode = 0;
pub const FOCUS_FOLLOWS_MOUSE: FocusMode = 1;

/// C `shell_monitor`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Monitor {
    pub rect: Rect,
    pub scale_pct: u32,
    pub name: [u8; 16],
}

impl Monitor {
    pub const EMPTY: Self = Self { rect: Rect::new(0, 0, 0, 0), scale_pct: 0, name: [0; 16] };
}

/// C `shell_wm`.
#[repr(C)]
#[allow(missing_docs)]
pub struct ShellWm {
    pub comp: *mut Compositor,
    pub mon: [Monitor; SHELL_MAX_MONITORS],
    pub nmon: i32,
    pub focus_mode: FocusMode,
    pub tiling: bool,
    pub gap: i32,
    pub ndesktops: i32,
    pub preview: Snap,
    pub preview_mon: i32,
    pub was_grab: i32,
    pub master_pct: i32,
}

// Re-exported under the C enum names.
pub use snap as shell_snap;

fn taskbar_h() -> i32 {
    crate::taskbar::taskbar_h()
}

fn comp<'a>(wm: &ShellWm) -> &'a mut Compositor {
    // SAFETY: `comp` points at the live compositor that owns this wm.
    unsafe { &mut *wm.comp }
}

pub fn init(wm: &mut ShellWm, c: *mut Compositor) {
    wm.comp = c;
    wm.mon = [Monitor::EMPTY; SHELL_MAX_MONITORS];
    wm.nmon = 0;
    wm.focus_mode = FOCUS_CLICK;
    wm.tiling = false;
    wm.gap = 6;
    wm.ndesktops = crate::compositor::SHELL_DESKTOPS;
    wm.preview = snap::NONE;
    wm.preview_mon = 0;
    wm.was_grab = 0;
    wm.master_pct = 55;
    // SAFETY: comp points at a live compositor.
    let (sw, sh, scale) = unsafe { ((*c).screen_w, (*c).screen_h, (*c).scale_pct) };
    add_monitor(wm, Rect::new(0, 0, sw, sh), scale, b"primary");
}

pub fn add_monitor(wm: &mut ShellWm, r: Rect, scale: u32, name: &[u8]) -> bool {
    if wm.nmon as usize >= SHELL_MAX_MONITORS {
        return false;
    }
    let m = &mut wm.mon[wm.nmon as usize];
    wm.nmon += 1;
    m.rect = r;
    m.scale_pct = if scale != 0 { scale } else { 100 };
    m.name = [0; 16];
    let n = name.len().min(15);
    m.name[..n].copy_from_slice(&name[..n]);
    true
}

pub fn monitor_at(wm: &ShellWm, x: i32, y: i32) -> i32 {
    let mut best = 0;
    let mut bestd: i64 = -1;
    for i in 0..wm.nmon as usize {
        let r = wm.mon[i].rect;
        if x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h {
            return i as i32;
        }
        let dx = if x < r.x { r.x - x } else if x >= r.x + r.w { x - (r.x + r.w - 1) } else { 0 };
        let dy = if y < r.y { r.y - y } else if y >= r.y + r.h { y - (r.y + r.h - 1) } else { 0 };
        let d = dx as i64 * dx as i64 + dy as i64 * dy as i64;
        if bestd < 0 || d < bestd {
            bestd = d;
            best = i as i32;
        }
    }
    best
}

pub fn monitor_work_area(wm: &ShellWm, mon: i32) -> Rect {
    let r = wm.mon[mon as usize].rect;
    let wa = r.intersect(comp(wm).work_area);
    if wa.is_empty() { r } else { wa }
}

pub fn snap_rect(wm: &ShellWm, mon: i32, s: Snap) -> Rect {
    let a = monitor_work_area(wm, mon);
    let g = wm.gap;
    let hw = a.w / 2;
    let hh = a.h / 2;
    let t = a.w / 3;
    let mut r = match s {
        snap::LEFT => Rect::new(a.x, a.y, hw, a.h),
        snap::RIGHT => Rect::new(a.x + hw, a.y, a.w - hw, a.h),
        snap::MAXIMIZE => return a,
        snap::TL => Rect::new(a.x, a.y, hw, hh),
        snap::TR => Rect::new(a.x + hw, a.y, a.w - hw, hh),
        snap::BL => Rect::new(a.x, a.y + hh, hw, a.h - hh),
        snap::BR => Rect::new(a.x + hw, a.y + hh, a.w - hw, a.h - hh),
        snap::THIRD_L => Rect::new(a.x, a.y, t, a.h),
        snap::THIRD_C => Rect::new(a.x + t, a.y, t, a.h),
        snap::THIRD_R => Rect::new(a.x + 2 * t, a.y, a.w - 2 * t, a.h),
        snap::TWO_THIRDS_L => Rect::new(a.x, a.y, 2 * t, a.h),
        snap::TWO_THIRDS_R => Rect::new(a.x + t, a.y, a.w - t, a.h),
        _ => return a,
    };
    let l = if r.x > a.x { g / 2 } else { 0 };
    let rr = if r.x + r.w < a.x + a.w { g / 2 } else { 0 };
    let tp = if r.y > a.y { g / 2 } else { 0 };
    let b = if r.y + r.h < a.y + a.h { g / 2 } else { 0 };
    r.x += l;
    r.w -= l + rr;
    r.y += tp;
    r.h -= tp + b;
    r
}

pub fn detect_snap(wm: &ShellWm, x: i32, y: i32) -> (Snap, i32) {
    let m = monitor_at(wm, x, y);
    let a = wm.mon[m as usize].rect;
    let l = x < a.x + SNAP_EDGE;
    let r = x >= a.x + a.w - SNAP_EDGE;
    let t = y < a.y + SNAP_EDGE;
    let nl = x < a.x + SNAP_CORNER;
    let nr = x >= a.x + a.w - SNAP_CORNER;
    let nt = y < a.y + SNAP_CORNER;
    let s = if (l && nt) || (t && nl) {
        snap::TL
    } else if (r && nt) || (t && nr) {
        snap::TR
    } else if l && y >= a.y + a.h - SNAP_CORNER - taskbar_h() {
        snap::BL
    } else if r && y >= a.y + a.h - SNAP_CORNER - taskbar_h() {
        snap::BR
    } else if t {
        snap::MAXIMIZE
    } else if l {
        snap::LEFT
    } else if r {
        snap::RIGHT
    } else {
        snap::NONE
    };
    (s, m)
}

pub fn snap_window(wm: &mut ShellWm, win: i32, s: Snap, mon: i32) -> bool {
    let c = comp(wm);
    let Some(w) = c.get(win).copied() else { return false };
    if (w.flags & WF_NORESIZE) != 0 || mon < 0 || mon >= wm.nmon {
        return false;
    }
    if s == snap::NONE {
        unsnap(wm, win);
        return true;
    }
    if (w.flags & (WF_SNAPPED | WF_MAXIMIZED)) == 0 {
        if let Some(w) = c.get_mut(win) {
            w.restore = w.rect;
        }
    }
    let r = snap_rect(wm, mon, s);
    if let Some(w) = c.get_mut(win) {
        if s == snap::MAXIMIZE {
            w.flags |= WF_MAXIMIZED;
            w.flags &= !WF_SNAPPED;
        } else {
            w.flags |= WF_SNAPPED;
            w.flags &= !WF_MAXIMIZED;
        }
    }
    c.set_rect(win, r);
    c.notify(win);
    true
}

pub fn unsnap(wm: &mut ShellWm, win: i32) {
    let c = comp(wm);
    let Some(w) = c.get(win).copied() else { return };
    if (w.flags & (WF_SNAPPED | WF_MAXIMIZED)) == 0 {
        return;
    }
    if let Some(w) = c.get_mut(win) {
        w.flags &= !(WF_SNAPPED | WF_MAXIMIZED);
    }
    c.set_rect(win, w.restore);
    c.notify(win);
}

fn apply_dpi(wm: &mut ShellWm, w: &Window) {
    let mon = monitor_at(wm, w.rect.x + w.rect.w / 2, w.rect.y + w.rect.h / 2);
    let dpi = 96 * wm.mon[mon as usize].scale_pct / 100;
    if dpi != w.dpi {
        let c = comp(wm);
        let id = w.id;
        if let Some(w) = c.get_mut(id) {
            w.dpi = dpi;
        }
        c.post_event(id, ShellEvent { ev_type: ev::DPI_CHANGED, x: dpi as i32, y: 0, button: 0, key: 0, mods: 0 });
    }
}

pub fn move_to_monitor(wm: &mut ShellWm, win: i32, mon: i32) {
    let c = comp(wm);
    let Some(w) = c.get(win).copied() else { return };
    if mon < 0 || mon >= wm.nmon {
        return;
    }
    let from = monitor_at(wm, w.rect.x + w.rect.w / 2, w.rect.y + w.rect.h / 2);
    if from == mon {
        return;
    }
    let a = monitor_work_area(wm, from);
    let b = monitor_work_area(wm, mon);
    if (w.flags & (WF_MAXIMIZED | WF_SNAPPED)) != 0 {
        let mut s = snap::MAXIMIZE;
        if (w.flags & WF_SNAPPED) != 0 {
            let left = w.rect.x < a.x + a.w / 4;
            let right = w.rect.x + w.rect.w > a.x + a.w - a.w / 4;
            s = if left && !right { snap::LEFT } else if right && !left { snap::RIGHT } else { snap::MAXIMIZE };
        }
        let keep = w.restore;
        snap_window(wm, win, s, mon);
        if let Some(w) = comp(wm).get_mut(win) {
            w.restore = Rect::new(b.x + (keep.x - a.x), b.y + (keep.y - a.y), keep.w, keep.h);
        }
    } else {
        let mut r = w.rect;
        r.x = b.x + (r.x - a.x) * b.w / (if a.w != 0 { a.w } else { 1 });
        r.y = b.y + (r.y - a.y) * b.h / (if a.h != 0 { a.h } else { 1 });
        if r.w > b.w {
            r.w = b.w;
        }
        if r.h > b.h {
            r.h = b.h;
        }
        comp(wm).set_rect(win, r);
    }
    if let Some(w) = comp(wm).get(win).copied() {
        apply_dpi(wm, &w);
    }
}

pub fn switch_desktop(wm: &mut ShellWm, n: i32) {
    let c = comp(wm);
    if n < 0 || n >= wm.ndesktops || n == c.cur_desktop {
        return;
    }
    let f = c.get(c.focus).copied();
    if let Some(f) = f {
        if (f.flags & WF_STICKY) == 0 {
            c.post_event(f.id, ShellEvent { ev_type: ev::BLUR, x: 0, y: 0, button: 0, key: 0, mods: 0 });
            c.focus = 0;
        }
    }
    c.cur_desktop = n;
    c.grab_win = 0;
    c.capture = 0;
    c.press_win = 0;
    c.damage_all();
    for i in (0..c.nwin as usize).rev() {
        let id = c.order[i];
        if let Some(w) = c.get(id).copied() {
            if c.is_shown(&w) && (w.flags & WF_NOFOCUS) == 0 {
                c.focus(id);
                break;
            }
        }
    }
    if wm.tiling {
        retile(wm);
    }
    c.notify(0);
}

pub fn move_to_desktop(wm: &mut ShellWm, win: i32, n: i32) -> bool {
    let c = comp(wm);
    let Some(w) = c.get(win).copied() else { return false };
    if n < 0 || n >= wm.ndesktops {
        return false;
    }
    if w.desktop == n {
        return true;
    }
    let had_focus = c.focus == win;
    if let Some(w) = c.get_mut(win) {
        w.desktop = n;
    }
    c.damage_all();
    if !c.get(win).map_or(false, |w| c.is_shown(w)) && had_focus {
        c.focus = 0;
        c.post_event(win, ShellEvent { ev_type: ev::BLUR, x: 0, y: 0, button: 0, key: 0, mods: 0 });
        for i in (0..c.nwin as usize).rev() {
            let id = c.order[i];
            if let Some(o) = c.get(id).copied() {
                if c.is_shown(&o) && (o.flags & WF_NOFOCUS) == 0 {
                    c.focus(id);
                    break;
                }
            }
        }
    }
    if wm.tiling {
        retile(wm);
    }
    c.notify(win);
    true
}

pub fn set_tiling(wm: &mut ShellWm, on: bool) {
    wm.tiling = on;
    if on {
        retile(wm);
    }
}

pub fn retile(wm: &mut ShellWm) {
    let c = comp(wm);
    let mut ids = [0i32; crate::compositor::SHELL_MAX_WINDOWS];
    let mut n = 0usize;
    for i in 0..c.nwin as usize {
        let id = c.order[i];
        let Some(w) = c.get(id).copied() else { continue };
        if c.is_shown(&w) && (w.flags & (WF_NORESIZE | WF_NOCHROME | WF_TOPMOST)) == 0 {
            ids[n] = id;
            n += 1;
        }
    }
    if n == 0 {
        return;
    }
    let a = monitor_work_area(wm, 0);
    let g = wm.gap;
    let area = Rect::new(a.x + g, a.y + g, a.w - 2 * g, a.h - 2 * g);
    for &id in &ids[..n] {
        if let Some(w) = c.get_mut(id) {
            w.flags &= !WF_MAXIMIZED;
            w.flags |= WF_SNAPPED;
        }
    }
    if n == 1 {
        c.set_rect(ids[0], area);
        return;
    }
    let mw = area.w * wm.master_pct / 100;
    c.set_rect(ids[n - 1], Rect::new(area.x, area.y, mw - g / 2, area.h));
    let stack = n - 1;
    let sh = (area.h - (stack as i32 - 1) * g) / stack as i32;
    for i in 0..stack {
        let y = area.y + i as i32 * (sh + g);
        let h = if i == stack - 1 { area.y + area.h - y } else { sh };
        c.set_rect(ids[n - 2 - i], Rect::new(area.x + mw + g / 2, y, area.w - mw - g / 2, h));
    }
}

pub fn after_mouse(wm: &mut ShellWm) {
    let c = comp(wm);
    let g = c.grab_win;
    let wid = if g != 0 { g } else { wm.was_grab };
    let w = c.get(wid).copied();
    if g != 0 && c.grab_zone == zone::TITLE {
        if let Some(w) = w {
            let _ = w;
            let (s, mon) = detect_snap(wm, c.mouse_x, c.mouse_y);
            if s != wm.preview || mon != wm.preview_mon {
                if wm.preview != snap::NONE {
                    c.damage.add(snap_rect(wm, wm.preview_mon, wm.preview));
                }
                wm.preview = s;
                wm.preview_mon = mon;
                if s != snap::NONE {
                    c.damage.add(snap_rect(wm, mon, s));
                }
            }
        }
    } else if g == 0 && wm.was_grab != 0 {
        if wm.preview != snap::NONE {
            if let Some(w) = w {
                let win = w.id;
                let s = wm.preview;
                let mon = wm.preview_mon;
                c.damage.add(snap_rect(wm, mon, s));
                wm.preview = snap::NONE;
                snap_window(wm, win, s, mon);
            }
        }
        if let Some(w) = w {
            apply_dpi(wm, &w);
        }
        wm.preview = snap::NONE;
        if wm.tiling {
            retile(wm);
        }
    }
    wm.was_grab = g;
    if wm.focus_mode == FOCUS_FOLLOWS_MOUSE && g == 0 && c.capture == 0 {
        let (z, hw) = c.hit_test(c.mouse_x, c.mouse_y);
        if z != zone::NONE && hw != c.focus {
            c.focus(hw);
        }
    }
}

pub fn draw_preview(c: &mut Compositor, fb: *mut Fb, clip: Rect, wm: &ShellWm) {
    if wm.preview == snap::NONE {
        return;
    }
    let r = snap_rect(wm, wm.preview_mon, wm.preview);
    unsafe {
        ffi::fb_fill_round(fb, clip, r, 8, (c.theme.accent & 0x00FF_FFFF) | 0x3000_0000);
        let inside = Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2);
        ffi::fb_frame(fb, clip, r, c.theme.accent);
        ffi::fb_frame(fb, clip, inside, (c.theme.accent & 0x00FF_FFFF) | 0x8000_0000);
    }
}

pub fn install(wm: &mut ShellWm) {
    // SAFETY: the compositor pointer is live; the wm outlives the overlay registration.
    unsafe {
        comp_add_overlay(wm.comp, Some(wm_draw_preview), wm as *mut ShellWm as *mut c_void);
    }
}

pub fn key(wm: &mut ShellWm, key: u32, down: bool, mods: u32) -> bool {
    let c = comp(wm);
    if (mods & MOD_SUPER) == 0 {
        return false;
    }
    if !down {
        return true;
    }
    let f = c.focus;
    let w = c.get(f).copied();
    if key >= b'1' as u32 && key < b'1' as u32 + wm.ndesktops as u32 {
        let n = (key - b'1' as u32) as i32;
        if (mods & MOD_SHIFT) != 0 {
            if f != 0 {
                move_to_desktop(wm, f, n);
            }
        } else {
            switch_desktop(wm, n);
        }
        return true;
    }
    if key == b't' as u32 || key == b'T' as u32 {
        set_tiling(wm, !wm.tiling);
        return true;
    }
    let Some(w) = w else {
        return key == crate::compositor::KEY_LEFT
            || key == crate::compositor::KEY_RIGHT
            || key == crate::compositor::KEY_UP
            || key == crate::compositor::KEY_DOWN;
    };
    let mon = monitor_at(wm, w.rect.x + w.rect.w / 2, w.rect.y + w.rect.h / 2);
    let wa = monitor_work_area(wm, mon);
    let left_half = (w.flags & WF_SNAPPED) != 0 && w.rect.x < wa.x + 4;
    let right_half = (w.flags & WF_SNAPPED) != 0 && w.rect.x > wa.x + 4;
    if (mods & MOD_SHIFT) != 0 {
        if key == crate::compositor::KEY_LEFT && mon > 0 {
            move_to_monitor(wm, f, mon - 1);
            return true;
        }
        if key == crate::compositor::KEY_RIGHT && mon + 1 < wm.nmon {
            move_to_monitor(wm, f, mon + 1);
            return true;
        }
        return key == crate::compositor::KEY_LEFT || key == crate::compositor::KEY_RIGHT;
    }
    match key {
        crate::compositor::KEY_LEFT => {
            if right_half {
                unsnap(wm, f);
            } else {
                snap_window(wm, f, snap::LEFT, mon);
            }
            true
        }
        crate::compositor::KEY_RIGHT => {
            if left_half {
                unsnap(wm, f);
            } else {
                snap_window(wm, f, snap::RIGHT, mon);
            }
            true
        }
        crate::compositor::KEY_UP => {
            snap_window(wm, f, snap::MAXIMIZE, mon);
            true
        }
        crate::compositor::KEY_DOWN => {
            if (w.flags & (WF_MAXIMIZED | WF_SNAPPED)) != 0 {
                unsnap(wm, f);
            } else {
                c.minimize(f);
            }
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn wm_init(wm: *mut ShellWm, c: *mut Compositor) {
    if let Some(wm) = unsafe { wm.as_mut() } {
        init(wm, c);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_add_monitor(wm: *mut ShellWm, r: Rect, scale: u32, name: *const c_char) -> bool {
    match unsafe { wm.as_mut() } {
        Some(wm) => add_monitor(wm, r, scale, unsafe { ffi::c_bytes(name) }),
        None => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_monitor_at(wm: *const ShellWm, x: i32, y: i32) -> i32 {
    unsafe { wm.as_ref() }.map_or(0, |wm| monitor_at(wm, x, y))
}

#[no_mangle]
pub unsafe extern "C" fn wm_monitor_work_area(wm: *const ShellWm, mon: i32) -> Rect {
    unsafe { wm.as_ref() }.map_or(Rect::new(0, 0, 0, 0), |wm| monitor_work_area(wm, mon))
}

#[no_mangle]
pub unsafe extern "C" fn wm_snap_rect(wm: *const ShellWm, mon: i32, s: Snap) -> Rect {
    unsafe { wm.as_ref() }.map_or(Rect::new(0, 0, 0, 0), |wm| snap_rect(wm, mon, s))
}

#[no_mangle]
pub unsafe extern "C" fn wm_detect_snap(wm: *const ShellWm, x: i32, y: i32, mon: *mut i32) -> Snap {
    match unsafe { wm.as_ref() } {
        Some(wm) => {
            let (s, m) = detect_snap(wm, x, y);
            if !mon.is_null() {
                unsafe { *mon = m };
            }
            s
        }
        None => snap::NONE,
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_snap_window(wm: *mut ShellWm, win: i32, s: Snap, mon: i32) -> bool {
    match unsafe { wm.as_mut() } {
        Some(wm) => snap_window(wm, win, s, mon),
        None => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_unsnap(wm: *mut ShellWm, win: i32) {
    if let Some(wm) = unsafe { wm.as_mut() } {
        unsnap(wm, win);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_after_mouse(wm: *mut ShellWm) {
    if let Some(wm) = unsafe { wm.as_mut() } {
        after_mouse(wm);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_key(wm: *mut ShellWm, key: u32, down: bool, mods: u32) -> bool {
    match unsafe { wm.as_mut() } {
        Some(wm) => self::key(wm, key, down, mods),
        None => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_switch_desktop(wm: *mut ShellWm, n: i32) {
    if let Some(wm) = unsafe { wm.as_mut() } {
        switch_desktop(wm, n);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_move_to_desktop(wm: *mut ShellWm, win: i32, n: i32) -> bool {
    match unsafe { wm.as_mut() } {
        Some(wm) => move_to_desktop(wm, win, n),
        None => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_move_to_monitor(wm: *mut ShellWm, win: i32, mon: i32) {
    if let Some(wm) = unsafe { wm.as_mut() } {
        move_to_monitor(wm, win, mon);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_set_tiling(wm: *mut ShellWm, on: bool) {
    if let Some(wm) = unsafe { wm.as_mut() } {
        set_tiling(wm, on);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_retile(wm: *mut ShellWm) {
    if let Some(wm) = unsafe { wm.as_mut() } {
        retile(wm);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_draw_preview(c: *mut Compositor, fb: *mut Fb, clip: Rect, user: *mut c_void) {
    if let (Some(c), Some(wm)) = (unsafe { c.as_mut() }, unsafe { (user as *const ShellWm).as_ref() }) {
        draw_preview(c, fb, clip, wm);
    }
}

#[no_mangle]
pub unsafe extern "C" fn wm_install(wm: *mut ShellWm) {
    if let Some(wm) = unsafe { wm.as_mut() } {
        install(wm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use crate::compositor::Compositor;
    use core::mem::{offset_of, size_of};

    fn setup() -> (&'static mut ShellWm, &'static mut Compositor) {
        let c = Box::leak(Box::new(unsafe { core::mem::zeroed::<Compositor>() }));
        c.init(1920, 1080);
        c.bottom_reserve = 48;
        c.work_area = Rect::new(0, 0, 1920, 1032);
        let wm = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellWm>() }));
        init(wm, c);
        (wm, c)
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<Monitor>(), 36);
        assert_eq!(size_of::<ShellWm>(), 192);
        assert_eq!(offset_of!(ShellWm, mon), 8);
        assert_eq!(offset_of!(ShellWm, nmon), 152);
        assert_eq!(offset_of!(ShellWm, focus_mode), 156);
        assert_eq!(offset_of!(ShellWm, tiling), 160);
        assert_eq!(offset_of!(ShellWm, gap), 164);
        assert_eq!(offset_of!(ShellWm, ndesktops), 168);
        assert_eq!(offset_of!(ShellWm, preview), 172);
        assert_eq!(offset_of!(ShellWm, master_pct), 184);
    }

    #[test]
    fn snap_rects_have_half_gaps_on_interior_edges() {
        let (wm, _c) = setup();
        assert_eq!(snap_rect(wm, 0, snap::LEFT), Rect::new(0, 0, 957, 1032));
        assert_eq!(snap_rect(wm, 0, snap::RIGHT), Rect::new(963, 0, 957, 1032));
        assert_eq!(snap_rect(wm, 0, snap::TL), Rect::new(0, 0, 957, 513));
        assert_eq!(snap_rect(wm, 0, snap::TR), Rect::new(963, 0, 957, 513));
        assert_eq!(snap_rect(wm, 0, snap::MAXIMIZE), Rect::new(0, 0, 1920, 1032));
        assert_eq!(snap_rect(wm, 0, snap::THIRD_C), Rect::new(643, 0, 634, 1032));
    }

    #[test]
    fn snap_window_sets_flags_and_restore() {
        let (wm, c) = setup();
        let id = c.create(b"Snap", Rect::new(100, 100, 400, 300), 0, 1);
        assert!(snap_window(wm, id, snap::LEFT, 0));
        let w = c.get(id).unwrap();
        assert_eq!((w.flags & WF_SNAPPED) != 0, true);
        assert_eq!(w.rect, Rect::new(0, 0, 957, 1032));
        assert_eq!(w.restore, Rect::new(100, 100, 400, 300));
        unsnap(wm, id);
        let w = c.get(id).unwrap();
        assert_eq!(w.rect, Rect::new(100, 100, 400, 300));
        assert_eq!((w.flags & WF_SNAPPED) != 0, false);
    }

    #[test]
    fn tiling_masters_the_most_recent_window() {
        let (wm, c) = setup();
        let a = c.create(b"A", Rect::new(0, 0, 200, 200), 0, 1);
        let b = c.create(b"B", Rect::new(0, 0, 200, 200), 0, 1);
        set_tiling(wm, true);
        let wa = c.get(a).unwrap().rect;
        let wb = c.get(b).unwrap().rect;
        // Master (B, most recently raised) is on the left, the stack (A) fills the right.
        assert!(wb.x < wa.x);
        assert!(wa.w > 0 && wa.h > 0 && wb.w > 0 && wb.h > 0);
    }

    #[test]
    fn keyboard_snaps_and_switches_desktops() {
        let (wm, c) = setup();
        let id = c.create(b"Keys", Rect::new(300, 200, 400, 300), 0, 1);
        assert!(key(wm, crate::compositor::KEY_LEFT, true, MOD_SUPER));
        assert_eq!(c.get(id).unwrap().rect, Rect::new(0, 0, 957, 1032));
        assert!(key(wm, b'2' as u32, true, MOD_SUPER));
        assert_eq!(c.cur_desktop, 1);
        assert!(!c.is_shown(c.get(id).unwrap()));
        assert!(key(wm, b'1' as u32, true, MOD_SUPER));
        assert_eq!(c.cur_desktop, 0);
    }
}

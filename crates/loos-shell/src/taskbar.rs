//! Taskbar, tray, Quick Settings and calendar flyouts (C `taskbar.c`).
#![allow(missing_docs)]

use crate::compositor::{
    Compositor, KEY_ESC, KEY_F4, KEY_SPACE, KEY_SUPER, KEY_TAB, MOD_ALT, MOD_CTRL, MOD_SHIFT, SHELL_MAX_WINDOWS, WF_MINIMIZED,
    WF_NOTASKBAR, WF_STICKY, WF_VISIBLE,
};
use crate::draw::{mix, Rect};
use crate::ffi::{self, Fb};
use crate::font::ShellFont;
use crate::font_data;
use crate::launcher::{self, ShellLauncher};
use crate::text;
use core::ffi::{c_char, c_void};

const START_W: i32 = 40;
const BTN_ICON: i32 = 44;
const TRAY_W: i32 = 170;
const LED_AMBER: u32 = 0xFFF5_9E0B;
const LED_RED: u32 = 0xFFEF_4444;
const LED_GREY: u32 = 0xFF4B_5563;

const QS_W: i32 = 360;
const QS_H: i32 = 300;
const CAL_W: i32 = 300;
const CAL_H: i32 = 290;

pub const NET_IDLE: i32 = 0;
pub const NET_ACTIVE: i32 = 1;
pub const NET_BLOCKED: i32 = 2;

pub const QS_NET: i32 = 0;
pub const QS_DARK: i32 = 1;
pub const QS_LARGE: i32 = 2;
pub const QS_CENTER: i32 = 3;
pub const QS_LABELS: i32 = 4;
pub const QS_MOTION: i32 = 5;
pub const QS_ACCENT: i32 = 6;
pub const QS_OPACITY: i32 = 7;
pub const QS_SETTINGS: i32 = 8;

#[no_mangle]
pub static mut shell_taskbar_h: i32 = 48;

pub fn taskbar_h() -> i32 {
    // SAFETY: single-threaded shell; the global is only set by taskbar_configure.
    unsafe { shell_taskbar_h }
}
fn set_taskbar_h(v: i32) {
    // SAFETY: as above.
    unsafe { shell_taskbar_h = v };
}

pub type QuickFn = unsafe extern "C" fn(i32, u32, *mut c_void);
pub type NetToggleFn = unsafe extern "C" fn(bool, *mut c_void);

/// C `shell_taskbar`.
#[repr(C)]
#[allow(missing_docs)]
pub struct ShellTaskbar {
    pub comp: *mut Compositor,
    pub launcher: *mut ShellLauncher,
    pub hover_btn: i32,
    pub net_sockets: i32,
    pub net_hard_off: bool,
    pub hour: i32,
    pub minute: i32,
    pub day: i32,
    pub month: i32,
    pub year: i32,
    pub flyout: i32,
    pub size: i32,
    pub on_quick: Option<QuickFn>,
    pub centered: bool,
    pub labels: bool,
    pub top: bool,
    pub on_net_toggle: Option<NetToggleFn>,
    pub user: *mut c_void,
}

impl ShellTaskbar {
    pub const EMPTY: Self = Self {
        comp: core::ptr::null_mut(),
        launcher: core::ptr::null_mut(),
        hover_btn: -1,
        net_sockets: 0,
        net_hard_off: false,
        hour: 0,
        minute: 0,
        day: 0,
        month: 0,
        year: 0,
        flyout: 0,
        size: 0,
        on_quick: None,
        centered: false,
        labels: false,
        top: false,
        on_net_toggle: None,
        user: core::ptr::null_mut(),
    };
}

fn comp<'a>(t: &ShellTaskbar) -> Option<&'a mut Compositor> {
    if t.comp.is_null() { None } else { Some(unsafe { &mut *t.comp }) }
}

pub fn init(t: &mut ShellTaskbar, c: *mut Compositor, l: *mut ShellLauncher) {
    *t = ShellTaskbar::EMPTY;
    t.comp = c;
    t.launcher = l;
    t.hover_btn = -1;
    if let Some(c) = comp(t) {
        c.taskbar = t as *mut ShellTaskbar as *mut c_void;
        c.add_overlay(overlay, t as *mut ShellTaskbar as *mut c_void);
    }
    configure(t, true, false, false, 1);
}

pub fn configure(t: &mut ShellTaskbar, centered: bool, labels: bool, top: bool, size: i32) {
    t.centered = centered;
    t.labels = labels;
    t.top = top;
    t.size = if !(0..=2).contains(&size) { 1 } else { size };
    set_taskbar_h(40 + 8 * t.size);
    let Some(c) = comp(t) else { return };
    let dev_h = taskbar_h() * c.scale_pct as i32 / 100;
    c.bottom_reserve = if top { 0 } else { taskbar_h() };
    c.top_reserve = if top { taskbar_h() } else { 0 };
    c.set_work_area(Rect::new(0, if top { dev_h } else { 0 }, c.screen_w, c.screen_h - dev_h));
    c.damage_all();
}

pub fn rect(c: &Compositor) -> Rect {
    let top = if c.taskbar.is_null() { false } else { unsafe { (*(c.taskbar as *const ShellTaskbar)).top } };
    Rect::new(0, if top { 0 } else { c.lh() - taskbar_h() }, c.lw(), taskbar_h())
}

fn dirty(t: &mut ShellTaskbar) {
    if let Some(c) = comp(t) {
        let r = rect(c);
        c.damage_logical(r);
    }
}

pub fn set_time(t: &mut ShellTaskbar, h: i32, m: i32) {
    if t.hour != h || t.minute != m {
        t.hour = h;
        t.minute = m;
        dirty(t);
    }
}
pub fn set_date(t: &mut ShellTaskbar, d: i32, m: i32, y: i32) {
    if t.day != d || t.month != m || t.year != y {
        t.day = d;
        t.month = m;
        t.year = y;
        dirty(t);
    }
}
pub fn set_net(t: &mut ShellTaskbar, sockets: i32) {
    if t.net_sockets != sockets {
        t.net_sockets = sockets;
        dirty(t);
    }
}
pub fn net_state(t: &ShellTaskbar) -> i32 {
    if t.net_hard_off {
        return NET_BLOCKED;
    }
    if t.net_sockets > 0 { NET_ACTIVE } else { NET_IDLE }
}

fn collect(c: &Compositor) -> [i32; SHELL_MAX_WINDOWS] {
    let mut ids = [0i32; SHELL_MAX_WINDOWS];
    let mut n = 0usize;
    for i in 0..SHELL_MAX_WINDOWS {
        let w = &c.win[i];
        if w.id == 0 || (w.flags & WF_VISIBLE) == 0 || (w.flags & WF_NOTASKBAR) != 0 {
            continue;
        }
        if (w.flags & WF_STICKY) == 0 && w.desktop != c.cur_desktop {
            continue;
        }
        let max = SHELL_MAX_WINDOWS;
        let mut pos = if n < max { n } else { max - 1 };
        if n < max {
            n += 1;
        }
        while pos > 0 && ids[pos - 1] > w.id {
            ids[pos] = ids[pos - 1];
            pos -= 1;
        }
        ids[pos] = w.id;
    }
    ids
}

pub fn button_count(_t: &ShellTaskbar, c: &Compositor) -> i32 {
    collect(c).iter().filter(|&&id| id != 0).count() as i32
}
pub fn button_window(_t: &ShellTaskbar, c: &Compositor, index: i32) -> i32 {
    let ids = collect(c);
    if index >= 0 && (index as usize) < SHELL_MAX_WINDOWS {
        ids[index as usize]
    } else {
        0
    }
}

fn button_w(t: &ShellTaskbar, c: &Compositor, n: i32) -> i32 {
    if !t.labels {
        return BTN_ICON;
    }
    let avail = c.lw() - TRAY_W - 16 - START_W - 8;
    let w = if n != 0 { avail / n - 4 } else { 180 };
    if w > 180 { 180 } else if w < BTN_ICON { BTN_ICON } else { w }
}
fn group_x(t: &ShellTaskbar, c: &Compositor) -> i32 {
    let n = button_count(t, c);
    let w = button_w(t, c, n);
    let total = START_W + 4 + n * (w + 4);
    let x = if t.centered { (c.lw() - total) / 2 } else { 8 };
    if x < 8 { 8 } else { x }
}
pub fn button_rect(t: &ShellTaskbar, c: &Compositor, index: i32) -> Rect {
    let n = button_count(t, c);
    let w = button_w(t, c, n);
    let bar = rect(c);
    Rect::new(group_x(t, c) + START_W + 4 + index * (w + 4), bar.y + 4, w, taskbar_h() - 8)
}

fn start_rect(c: &Compositor) -> Rect {
    let b = rect(c);
    let x = if c.taskbar.is_null() { 8 } else { group_x(unsafe { &*(c.taskbar as *const ShellTaskbar) }, c) };
    Rect::new(x, b.y + 4, START_W, taskbar_h() - 8)
}
pub fn start_rect_pub(c: &Compositor) -> Rect {
    start_rect(c)
}
fn net_toggle_rect(c: &Compositor) -> Rect {
    let b = rect(c);
    Rect::new(c.lw() - TRAY_W + 24, b.y + 10, 40, taskbar_h() - 20)
}
fn clock_rect(c: &Compositor) -> Rect {
    let b = rect(c);
    Rect::new(c.lw() - 70, b.y + 4, 66, taskbar_h() - 8)
}
fn led_rect(c: &Compositor) -> Rect {
    let b = rect(c);
    Rect::new(c.lw() - TRAY_W + 6, b.y + taskbar_h() / 2 - 5, 10, 10)
}

fn contains(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

fn hit_button(t: &ShellTaskbar, c: &Compositor, x: i32, y: i32) -> i32 {
    if contains(start_rect(c), x, y) {
        return 0;
    }
    if contains(net_toggle_rect(c), x, y) {
        return -2;
    }
    if contains(led_rect(c), x, y) {
        return -3;
    }
    if contains(clock_rect(c), x, y) {
        return -4;
    }
    let n = button_count(t, c);
    for i in 0..n {
        if contains(button_rect(t, c, i), x, y) {
            return 1 + i;
        }
    }
    -1
}

fn activate_window(c: &mut Compositor, id: i32) {
    let Some(w) = c.get(id).copied() else { return };
    if (w.flags & WF_MINIMIZED) != 0 {
        c.restore(id);
    } else if c.focus == id {
        c.minimize(id);
    } else {
        c.raise(id);
        c.focus(id);
    }
}

const QS_ACCENTS: [u32; 8] = [0xFF3B_82F6, 0xFF8B_5CF6, 0xFFEC_4899, 0xFFEF_4444, 0xFFF5_9E0B, 0xFF10_B981, 0xFF06_B6D4, 0xFF64_748B];
const QS_OPAC: [i32; 4] = [60, 80, 92, 100];
const QS_NAME: [&[u8]; 6] = [b"Network\0", b"Dark mode\0", b"Large text\0", b"Centered\0", b"Titles\0", b"Less motion\0"];

pub fn flyout_rect(t: &ShellTaskbar, c: &Compositor) -> Rect {
    if t.flyout == 0 {
        return Rect::new(0, 0, 0, 0);
    }
    let (w, h) = if t.flyout == 1 { (QS_W, QS_H) } else { (CAL_W, CAL_H) };
    Rect::new(c.lw() - w - 12, if t.top { taskbar_h() + 12 } else { c.lh() - taskbar_h() - 12 - h }, w, h)
}

fn qs_state(t: &ShellTaskbar, c: &Compositor, i: i32) -> bool {
    let th = &c.theme;
    let lum = ((th.surface >> 16) & 255) + ((th.surface >> 8) & 255) + (th.surface & 255);
    match i {
        0 => !t.net_hard_off,
        1 => lum < 384,
        2 => c.scale_pct >= 150,
        3 => t.centered,
        4 => t.labels,
        _ => c.reduce_motion,
    }
}
fn qs_tile(i: i32) -> Rect {
    Rect::new(16 + (i % 3) * 112, 16 + (i / 3) * 72, 104, 64)
}
fn qs_swatch(i: i32) -> Rect {
    Rect::new(16 + i * 41, 196, 32, 32)
}
fn qs_opac_btn(i: i32) -> Rect {
    Rect::new(16 + i * 82, 252, 76, 30)
}

fn flyout_click(t: &mut ShellTaskbar, c: &mut Compositor, x: i32, y: i32) {
    if t.flyout != 1 {
        return;
    }
    let Some(quick) = t.on_quick else { return };
    let act = [QS_NET, QS_DARK, QS_LARGE, QS_CENTER, QS_LABELS, QS_MOTION];
    for i in 0..6 {
        if contains(qs_tile(i), x, y) {
            // SAFETY: host callback installed with a matching user pointer.
            unsafe { quick(act[i as usize], 0, t.user) };
        }
    }
    for i in 0..8 {
        if contains(qs_swatch(i), x, y) {
            // SAFETY: as above.
            unsafe { quick(QS_ACCENT, QS_ACCENTS[i as usize], t.user) };
        }
    }
    for i in 0..4 {
        if contains(qs_opac_btn(i), x, y) {
            // SAFETY: as above.
            unsafe { quick(QS_OPACITY, QS_OPAC[i as usize] as u32, t.user) };
        }
    }
    if contains(Rect::new(QS_W - 100, 160, 84, 24), x, y) {
        t.flyout = 0;
        // SAFETY: as above.
        unsafe { quick(QS_SETTINGS, 0, t.user) };
    }
    c.damage_all();
}

pub fn days_in(m: i32, y: i32) -> i32 {
    let d = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if m == 2 && ((y % 4 == 0 && y % 100 != 0) || y % 400 == 0) { 29 } else { d[((m - 1) % 12) as usize] }
}
/// 0 = Monday.
pub fn weekday(y: i32, m: i32, d: i32) -> i32 {
    let k = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    (y + y / 4 - y / 100 + y / 400 + k[((m - 1) % 12) as usize] + d + 6) % 7
}

fn text(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: &[u8], col: u32) {
    unsafe { ffi::fb_text(fb, clip, x, y, s.as_ptr() as *const c_char, col) };
}
fn text_ex(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: &[u8], col: u32, f: ShellFont) {
    unsafe { ffi::fb_text_ex(fb, clip, x, y, s.as_ptr() as *const c_char, col, f) };
}
fn width(f: ShellFont, s: &[u8]) -> i32 {
    unsafe { ffi::shell_text_width_ex(s.as_ptr() as *const c_char, f) }
}
fn reg(size: i32) -> ShellFont {
    ShellFont { face: core::ptr::addr_of!(font_data::loos_font_regular), size }
}
fn semi(size: i32) -> ShellFont {
    ShellFont { face: core::ptr::addr_of!(font_data::loos_font_semibold), size }
}

fn draw_flyout(t: &ShellTaskbar, c: &Compositor, fb: *mut Fb, clip: Rect) {
    let r = flyout_rect(t, c);
    if r.is_empty() || !r.overlaps(clip) {
        return;
    }
    let th = &c.theme;
    unsafe {
        ffi::fb_shadow(fb, clip, r, 12, 24, 6, 0x6000_0000);
        ffi::fb_fill_round(fb, clip, r, 12, c.panel_color());
        ffi::fb_stroke_round(fb, clip, r, 12, 1, th.border);
    }
    let sm = reg(12);
    let hd = semi(14);
    if t.flyout == 1 {
        for i in 0..6 {
            let mut q = qs_tile(i);
            q.x += r.x;
            q.y += r.y;
            let on = qs_state(t, c, i);
            unsafe { ffi::fb_fill_round(fb, clip, q, 8, if on { th.accent } else { (th.text & 0x00FF_FFFF) | 0x1400_0000 }) };
            text_ex(fb, clip, q.x + 12, q.y + 38, QS_NAME[i as usize], if on { th.accent_text } else { th.text }, sm);
            unsafe { ffi::fb_fill_round(fb, clip, Rect::new(q.x + 12, q.y + 12, 18, 18), 9, if on { th.accent_text } else { th.text_dim }) };
        }
        text_ex(fb, clip, r.x + 16, r.y + 168, b"Accent\0", th.text_dim, sm);
        text_ex(fb, clip, r.x + QS_W - 100 + 8, r.y + 164, b"All settings\0", th.accent, sm);
        for i in 0..8 {
            let mut q = qs_swatch(i);
            q.x += r.x;
            q.y += r.y;
            if (th.accent | 0xFF00_0000) == QS_ACCENTS[i as usize] {
                unsafe { ffi::fb_fill_round(fb, clip, Rect::new(q.x - 3, q.y - 3, q.w + 6, q.h + 6), 19, th.text) };
            }
            unsafe { ffi::fb_fill_round(fb, clip, q, 16, QS_ACCENTS[i as usize]) };
        }
        text_ex(fb, clip, r.x + 16, r.y + 236, b"Panel opacity\0", th.text_dim, sm);
        for i in 0..4 {
            let mut q = qs_opac_btn(i);
            q.x += r.x;
            q.y += r.y;
            let sel = c.style.opacity_pct == QS_OPAC[i as usize];
            unsafe { ffi::fb_fill_round(fb, clip, q, 6, if sel { th.accent } else { (th.text & 0x00FF_FFFF) | 0x1400_0000 }) };
            let mut lb = [0u8; 8];
            let mut n = 0;
            if QS_OPAC[i as usize] >= 100 {
                lb[n] = b'1';
                n += 1;
            }
            lb[n] = b'0' + (QS_OPAC[i as usize] / 10 % 10) as u8;
            n += 1;
            lb[n] = b'0' + (QS_OPAC[i as usize] % 10) as u8;
            n += 1;
            lb[n] = b'%';
            n += 1;
            lb[n] = 0;
            text_ex(fb, clip, q.x + (q.w - width(sm, &lb[..n])) / 2, q.y + 8, &lb, if sel { th.accent_text } else { th.text }, sm);
        }
        return;
    }
    const MONTHS: [&[u8]; 12] = [
        b"January\0", b"February\0", b"March\0", b"April\0", b"May\0", b"June\0", b"July\0", b"August\0", b"September\0",
        b"October\0", b"November\0", b"December\0",
    ];
    let y = if t.year != 0 { t.year } else { 2026 };
    let m = if t.month != 0 { t.month } else { 1 };
    let mut title = [0u8; 32];
    let mut n = 0;
    let mon = MONTHS[((m - 1) % 12) as usize];
    for &b in mon {
        if b == 0 {
            break;
        }
        title[n] = b;
        n += 1;
    }
    title[n] = b' ';
    n += 1;
    let mut k = 1000;
    while k != 0 {
        title[n] = b'0' + (y / k % 10) as u8;
        n += 1;
        k /= 10;
    }
    title[n] = 0;
    text_ex(fb, clip, r.x + 20, r.y + 16, &title[..n + 1], th.text, hd);
    const WD: [&[u8]; 7] = [b"Mo\0", b"Tu\0", b"We\0", b"Th\0", b"Fr\0", b"Sa\0", b"Su\0"];
    for i in 0..7 {
        text_ex(fb, clip, r.x + 20 + i * 38 + 6, r.y + 50, WD[i as usize], th.text_dim, sm);
    }
    let first = weekday(y, m, 1);
    let nd = days_in(m, y);
    for d in 1..=nd {
        let cell = first + d - 1;
        let q = Rect::new(r.x + 20 + (cell % 7) * 38, r.y + 74 + (cell / 7) * 34, 30, 30);
        if d == t.day {
            unsafe { ffi::fb_fill_round(fb, clip, q, 15, th.accent) };
        }
        let mut s = [0u8; 3];
        s[0] = if d >= 10 { b'0' + (d / 10) as u8 } else { b'0' + d as u8 };
        s[1] = if d >= 10 { b'0' + (d % 10) as u8 } else { 0 };
        let sl = if d >= 10 { 2 } else { 1 };
        text_ex(fb, clip, q.x + (q.w - width(sm, &s[..sl])) / 2, q.y + 8, &s[..sl], if d == t.day { th.accent_text } else { th.text }, sm);
    }
}

pub fn mouse(t: &mut ShellTaskbar, c: &mut Compositor, x: i32, y: i32, button: i32, down: bool) -> bool {
    let x = c.logical(x);
    let y = c.logical(y);
    let in_bar = contains(rect(c), x, y);
    if t.flyout != 0 {
        let fr = flyout_rect(t, c);
        if contains(fr, x, y) {
            if down && button == 1 {
                flyout_click(t, c, x - fr.x, y - fr.y);
            }
            return true;
        }
        if down && !in_bar {
            t.flyout = 0;
            c.damage_all();
        }
    }
    if t.launcher.is_null() == false && unsafe { (*t.launcher).open } {
        let in_start = contains(start_rect(c), x, y);
        if down && !in_start && launcher::mouse(unsafe { &mut *t.launcher }, c, x, y, true) {
            c.damage_all();
            return true;
        }
        let lr = launcher::rect(c);
        if down && !in_start && !contains(lr, x, y) {
            launcher::close(unsafe { &mut *t.launcher });
            c.damage_all();
        }
        if !down && contains(lr, x, y) {
            return true;
        }
    }
    let b = if in_bar { hit_button(t, c, x, y) } else { -1 };
    if b != t.hover_btn {
        t.hover_btn = b;
        dirty(t);
    }
    if !in_bar {
        return false;
    }
    if !down || button != 1 {
        return true;
    }
    if b == 0 && !t.launcher.is_null() {
        let l = unsafe { &mut *t.launcher };
        if l.open { launcher::close(l); } else { launcher::open(l); }
        c.damage_all();
    } else if b == -2 {
        t.net_hard_off = !t.net_hard_off;
        dirty(t);
        if let Some(f) = t.on_net_toggle {
            // SAFETY: host callback installed with a matching user pointer.
            unsafe { f(t.net_hard_off, t.user) };
        }
    } else if b == -3 || b == -4 {
        let want = if b == -3 { 1 } else { 2 };
        t.flyout = if t.flyout == want { 0 } else { want };
        if !t.launcher.is_null() {
            launcher::close(unsafe { &mut *t.launcher });
        }
        c.damage_all();
    } else if b >= 1 {
        let id = button_window(t, c, b - 1);
        activate_window(c, id);
    }
    true
}

pub fn key(t: &mut ShellTaskbar, c: &mut Compositor, key: u32, down: bool, mods: u32) -> bool {
    if !down {
        return false;
    }
    if t.flyout != 0 && key == KEY_ESC {
        t.flyout = 0;
        c.damage_all();
        return true;
    }
    if !t.launcher.is_null() && unsafe { (*t.launcher).open } {
        let r = launcher::key(unsafe { &mut *t.launcher }, key, mods);
        c.damage_all();
        return r;
    }
    if key == KEY_SUPER || (key == KEY_SPACE && (mods & MOD_CTRL) != 0) {
        if !t.launcher.is_null() {
            launcher::open(unsafe { &mut *t.launcher });
            c.damage_all();
        }
        return true;
    }
    if key == KEY_TAB && (mods & MOD_ALT) != 0 {
        let ids = collect(c);
        let n = ids.iter().filter(|&&id| id != 0).count() as i32;
        if n == 0 {
            return true;
        }
        let mut cur = -1;
        for i in 0..n as usize {
            if ids[i] == c.focus {
                cur = i as i32;
            }
        }
        let step = if (mods & MOD_SHIFT) != 0 { n - 1 } else { 1 };
        let mut next = ids[((cur + step + n) % n) as usize];
        if cur < 0 {
            next = ids[0];
        }
        let Some(w) = c.get(next).copied() else { return true };
        if (w.flags & WF_MINIMIZED) != 0 {
            c.restore(next);
        } else {
            c.raise(next);
            c.focus(next);
        }
        return true;
    }
    if key == KEY_F4 && (mods & MOD_ALT) != 0 {
        if c.focus != 0 {
            c.request_close(c.focus);
        }
        return true;
    }
    false
}

fn ellipsize(out: &mut [u8], s: &[u8], max_px: i32) {
    if max_px < 24 {
        if !out.is_empty() {
            out[0] = 0;
        }
        return;
    }
    text::ellipsize(out, s, max_px, &font_data::regular(), 14);
}

pub fn app_color(name: &[u8]) -> u32 {
    let mut h = 2_166_136_261u32;
    for &b in name {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    const PAL: [u32; 8] = [0xFF3B_82F6, 0xFF8B_5CF6, 0xFF10_B981, 0xFFF5_9E0B, 0xFFEF_4444, 0xFF06_B6D4, 0xFFEC_4899, 0xFF84_CC16];
    PAL[(h as usize) % 8]
}

pub fn draw_app_tile(fb: *mut Fb, clip: Rect, r: Rect, name: &[u8], radius: i32) {
    let mut name = name;
    if name.len() > 5 && name.starts_with(b"LoOS ") {
        name = &name[5..];
    } else if name.len() > 2 && name[0] == b'L' && name[1] == b'o' && name[2].is_ascii_uppercase() {
        name = &name[2..];
    }
    let col = app_color(name);
    let top = if radius > 1 { radius - 1 } else { 0 };
    unsafe {
        ffi::fb_fill_round(fb, clip, r, radius, col);
        ffi::fb_fill_round(fb, clip, Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2), top, mix(col, 0xFF00_0000, 26));
        ffi::fb_fill_round(fb, clip, Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h * 2 / 3), top, col);
    }
    let mut m = [0u8; 2];
    m[0] = match name.first() {
        Some(&c) if c.is_ascii_lowercase() => c - 32,
        Some(&c) => c,
        None => b'?',
    };
    let f = semi(r.h * 55 / 100);
    let tw = width(f, &m[..1]);
    unsafe { ffi::fb_text_ex(fb, clip, r.x + (r.w - tw) / 2, r.y + (r.h - f.size) / 2 - 1, m.as_ptr() as *const c_char, 0xFFFF_FFFF, f) };
}

fn draw_logo(fb: *mut Fb, clip: Rect, cx: i32, cy: i32, a: u32, hot: bool) {
    let hi = mix(a, 0xFFFF_FFFF, if hot { 110 } else { 70 });
    let lo = a;
    unsafe {
        ffi::fb_fill_round(fb, clip, Rect::new(cx - 9, cy - 9, 8, 8), 2, hi);
        ffi::fb_fill_round(fb, clip, Rect::new(cx - 9, cy + 1, 8, 8), 2, mix(hi, lo, 128));
        ffi::fb_fill_round(fb, clip, Rect::new(cx + 1, cy + 1, 8, 8), 2, lo);
        ffi::fb_fill_round(fb, clip, Rect::new(cx + 2, cy - 8, 6, 6), 3, hi);
    }
}

pub fn draw(t: &ShellTaskbar, c: &Compositor, fb: *mut Fb, clip: Rect) {
    let th = &c.theme;
    let bar = rect(c);
    if !bar.overlaps(clip) {
        return;
    }
    unsafe {
        ffi::fb_fill(fb, clip, bar, c.panel_color());
        ffi::fb_hline(fb, clip, bar.x, if t.top { bar.y + bar.h - 1 } else { bar.y }, bar.w, (th.border & 0x00FF_FFFF) | 0x9000_0000);
    }
    let sb = start_rect(c);
    let open = !t.launcher.is_null() && unsafe { (*t.launcher).open };
    if open || t.hover_btn == 0 {
        unsafe { ffi::fb_fill_round(fb, clip, sb, 6, (th.text & 0x00FF_FFFF) | 0x1800_0000) };
    }
    draw_logo(fb, clip, sb.x + sb.w / 2, sb.y + sb.h / 2, th.accent, open || t.hover_btn == 0);
    let n = button_count(t, c);
    for i in 0..n {
        let Some(w) = c.get(button_window(t, c, i)).copied() else { continue };
        let r = button_rect(t, c, i);
        let focused = c.focus == w.id && (w.flags & WF_MINIMIZED) == 0;
        let hov = t.hover_btn == 1 + i;
        if focused || hov {
            unsafe { ffi::fb_fill_round(fb, clip, r, 6, (th.text & 0x00FF_FFFF) | (if focused { 0x2000_0000 } else { 0x1400_0000 })) };
        }
        let ic = Rect::new(r.x + if t.labels { 8 } else { (r.w - 24) / 2 }, r.y + (r.h - 24) / 2 - 1, 24, 24);
        draw_app_tile(fb, clip, ic, title(&w), c.tile_radius(24));
        if t.labels {
            let mut label = [0u8; 96];
            ellipsize(&mut label, title(&w), r.w - 44);
            text(fb, clip, ic.x + 32, r.y + (r.h - 16) / 2, &label, if (w.flags & WF_MINIMIZED) != 0 { th.text_dim } else { th.text });
        }
        if (w.flags & WF_MINIMIZED) == 0 {
            let pw = if focused { 16 } else { 6 };
            unsafe { ffi::fb_fill_round(fb, clip, Rect::new(r.x + r.w / 2 - pw / 2, r.y + r.h - 3, pw, 3), 1, if focused { th.accent } else { th.text_dim }) };
        }
    }
    let ns = net_state(t);
    let led = if ns == NET_BLOCKED { LED_RED } else if ns == NET_ACTIVE { LED_AMBER } else { LED_GREY };
    let lr = led_rect(c);
    if ns == NET_ACTIVE {
        unsafe { ffi::fb_fill_round(fb, clip, Rect::new(lr.x - 2, lr.y - 2, lr.w + 4, lr.h + 4), 7, (led & 0x00FF_FFFF) | 0x5000_0000) };
    }
    unsafe { ffi::fb_fill_round(fb, clip, lr, 5, led) };
    let tg = net_toggle_rect(c);
    let off = t.net_hard_off;
    unsafe {
        ffi::fb_fill_round(fb, clip, tg, 6, if off { th.danger } else if t.hover_btn == -2 { (th.text & 0x00FF_FFFF) | 0x1800_0000 } else { 0 });
        if !off {
            ffi::fb_stroke_round(fb, clip, tg, 6, 1, th.border);
        }
    }
    let sf = reg(12);
    text_ex(fb, clip, tg.x + (tg.w - width(sf, if off { b"OFF\0" } else { b"NET\0" })) / 2, tg.y + (tg.h - 12) / 2 - 1,
            if off { b"OFF\0" } else { b"NET\0" }, if off { 0xFFFF_FFFF } else { th.text_dim }, sf);
    let clock = [
        b'0' + (t.hour / 10 % 10) as u8,
        b'0' + (t.hour % 10) as u8,
        b':',
        b'0' + (t.minute / 10 % 10) as u8,
        b'0' + (t.minute % 10) as u8,
        0,
    ];
    let right = c.lw() - 12;
    if t.day != 0 {
        let mut date = [0u8; 8];
        let mut k = 0;
        if t.day >= 10 {
            date[k] = b'0' + (t.day / 10) as u8;
            k += 1;
        }
        date[k] = b'0' + (t.day % 10) as u8;
        k += 1;
        date[k] = b'.';
        k += 1;
        if t.month >= 10 {
            date[k] = b'0' + (t.month / 10) as u8;
            k += 1;
        }
        date[k] = b'0' + (t.month % 10) as u8;
        k += 1;
        date[k] = b'.';
        k += 1;
        date[k] = 0;
        text_ex(fb, clip, right - width(sf, &clock), bar.y + taskbar_h() / 2 - 16, &clock, th.text, sf);
        text_ex(fb, clip, right - width(sf, &date), bar.y + taskbar_h() / 2 + 2, &date, th.text_dim, sf);
    } else {
        text(fb, clip, right - width(sf, &clock), bar.y + (taskbar_h() - 16) / 2, &clock, th.text);
    }
}

fn title(w: &crate::compositor::Window) -> &[u8] {
    let n = w.title.iter().position(|&b| b == 0).unwrap_or(w.title.len());
    &w.title[..n]
}

unsafe extern "C" fn overlay(c: *mut Compositor, fb: *mut Fb, clip: Rect, u: *mut c_void) {
    let (Some(c), Some(t)) = (unsafe { c.as_mut() }, unsafe { (u as *mut ShellTaskbar).as_mut() }) else { return };
    let mut view = unsafe { *fb };
    view.scale = c.scale_pct as i32;
    let lc = c.clip_logical(clip);
    draw(t, c, &mut view, lc);
    draw_flyout(t, c, &mut view, lc);
    if !t.launcher.is_null() {
        launcher::draw(unsafe { &*t.launcher }, c, &mut view, lc);
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn taskbar_init(t: *mut ShellTaskbar, c: *mut Compositor, l: *mut ShellLauncher) {
    if let Some(t) = unsafe { t.as_mut() } {
        init(t, c, l);
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_configure(t: *mut ShellTaskbar, centered: bool, labels: bool, top: bool, size: i32) {
    if let Some(t) = unsafe { t.as_mut() } {
        configure(t, centered, labels, top, size);
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_set_time(t: *mut ShellTaskbar, h: i32, m: i32) {
    if let Some(t) = unsafe { t.as_mut() } {
        set_time(t, h, m);
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_set_date(t: *mut ShellTaskbar, d: i32, m: i32, y: i32) {
    if let Some(t) = unsafe { t.as_mut() } {
        set_date(t, d, m, y);
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_set_net(t: *mut ShellTaskbar, sockets: i32) {
    if let Some(t) = unsafe { t.as_mut() } {
        set_net(t, sockets);
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_net_state(t: *const ShellTaskbar) -> i32 {
    unsafe { t.as_ref() }.map_or(NET_IDLE, net_state)
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_rect(c: *const Compositor) -> Rect {
    unsafe { c.as_ref() }.map_or(Rect::new(0, 0, 0, 0), rect)
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_mouse(t: *mut ShellTaskbar, c: *mut Compositor, x: i32, y: i32, button: i32, down: bool) -> bool {
    match (unsafe { t.as_mut() }, unsafe { c.as_mut() }) {
        (Some(t), Some(c)) => mouse(t, c, x, y, button, down),
        _ => false,
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_key(t: *mut ShellTaskbar, c: *mut Compositor, key: u32, down: bool, mods: u32) -> bool {
    match (unsafe { t.as_mut() }, unsafe { c.as_mut() }) {
        (Some(t), Some(c)) => self::key(t, c, key, down, mods),
        _ => false,
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_flyout_rect(t: *const ShellTaskbar, c: *const Compositor) -> Rect {
    match (unsafe { t.as_ref() }, unsafe { c.as_ref() }) {
        (Some(t), Some(c)) => flyout_rect(t, c),
        _ => Rect::new(0, 0, 0, 0),
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_start_rect(c: *const Compositor) -> Rect {
    unsafe { c.as_ref() }.map_or(Rect::new(0, 0, 0, 0), start_rect)
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_draw(t: *const ShellTaskbar, c: *const Compositor, fb: *mut Fb, clip: Rect) {
    if let (Some(t), Some(c)) = (unsafe { t.as_ref() }, unsafe { c.as_ref() }) {
        draw(t, c, fb, clip);
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_button_count(t: *const ShellTaskbar, c: *const Compositor) -> i32 {
    match (unsafe { t.as_ref() }, unsafe { c.as_ref() }) {
        (Some(t), Some(c)) => button_count(t, c),
        _ => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_button_rect(t: *const ShellTaskbar, c: *const Compositor, index: i32) -> Rect {
    match (unsafe { t.as_ref() }, unsafe { c.as_ref() }) {
        (Some(t), Some(c)) => button_rect(t, c, index),
        _ => Rect::new(0, 0, 0, 0),
    }
}
#[no_mangle]
pub unsafe extern "C" fn taskbar_button_window(t: *const ShellTaskbar, c: *const Compositor, index: i32) -> i32 {
    match (unsafe { t.as_ref() }, unsafe { c.as_ref() }) {
        (Some(t), Some(c)) => button_window(t, c, index),
        _ => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn shell_app_color(name: *const c_char) -> u32 {
    app_color(unsafe { ffi::c_bytes(name) })
}
#[no_mangle]
pub unsafe extern "C" fn shell_draw_app_tile(fb: *mut Fb, clip: Rect, r: Rect, name: *const c_char, radius: i32) {
    draw_app_tile(fb, clip, r, unsafe { ffi::c_bytes(name) }, radius);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ShellTaskbar>(), 88);
        assert_eq!(offset_of!(ShellTaskbar, launcher), 8);
        assert_eq!(offset_of!(ShellTaskbar, hover_btn), 16);
        assert_eq!(offset_of!(ShellTaskbar, net_hard_off), 24);
        assert_eq!(offset_of!(ShellTaskbar, hour), 28);
        assert_eq!(offset_of!(ShellTaskbar, flyout), 48);
        assert_eq!(offset_of!(ShellTaskbar, on_quick), 56);
        assert_eq!(offset_of!(ShellTaskbar, centered), 64);
        assert_eq!(offset_of!(ShellTaskbar, top), 66);
        assert_eq!(offset_of!(ShellTaskbar, on_net_toggle), 72);
        assert_eq!(offset_of!(ShellTaskbar, user), 80);
    }

    #[test]
    fn calendar_weekday_is_thursday() {
        assert_eq!(weekday(2026, 10, 1), 3, "0 = Monday");
        assert_eq!(weekday(2024, 2, 29), 3);
        assert_eq!(days_in(2, 2024), 29);
        assert_eq!(days_in(2, 2026), 28);
        assert_eq!(days_in(10, 2026), 31);
    }

    #[test]
    fn centered_layout_math() {
        let c = Box::leak(Box::new(unsafe { core::mem::zeroed::<Compositor>() }));
        c.init(1280, 720);
        c.create(b"A", Rect::new(0, 0, 200, 200), 0, 1);
        c.create(b"B", Rect::new(0, 0, 200, 200), 0, 1);
        let t = Box::leak(Box::new(ShellTaskbar::EMPTY));
        t.comp = c;
        t.centered = true;
        c.taskbar = t as *mut ShellTaskbar as *mut c_void;
        set_taskbar_h(48);
        let n = 2;
        let w = button_w(t, c, n);
        assert_eq!(w, 44);
        let total = START_W + 4 + n * (w + 4);
        assert_eq!(group_x(t, c), (1280 - total) / 2);
        let sr = start_rect(c);
        assert_eq!(sr, Rect::new(group_x(t, c), 720 - 48 + 4, 40, 40));
        assert_eq!(button_rect(t, c, 1), Rect::new(group_x(t, c) + 44 + 48, 720 - 48 + 4, 44, 40));
    }

    #[test]
    fn app_tile_colors_are_stable() {
        assert_eq!(app_color(b"Calculator"), app_color(b"Calculator"));
        assert_ne!(app_color(b"Calc"), app_color(b"Notepad") | 0);
        assert_eq!(taskbar_h(), 48);
    }
}

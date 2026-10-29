//! Notification service + center (C `notify.c`): local sources only, permission-gated and rate limited.
#![allow(missing_docs)]

use crate::compositor::{
    Compositor, KEY_DELETE, KEY_DOWN, KEY_ESC, KEY_UP, MOD_SUPER,
};
use crate::draw::{mix, Rect};
use crate::ffi::{self, Fb};
use core::ffi::{c_char, c_void};

pub const NOTIFY_MAX: usize = 64;
pub const NOTIFY_PENDING: usize = 8;
pub const NOTIFY_RATE_COUNT: usize = 5;
pub const NOTIFY_RATE_WINDOW_MS: u64 = 10_000;
pub const NOTIFY_TOAST_MS: u64 = 5_000;
pub const NOTIFY_MAX_TOASTS: usize = 3;

pub const NOTE_LOW: i32 = 0;
pub const NOTE_NORMAL: i32 = 1;
pub const NOTE_URGENT: i32 = 2;

pub const PERM_ASK: i32 = 0;
pub const PERM_ALLOW: i32 = 1;
pub const PERM_DENY: i32 = 2;
pub const RES_NOTIFY: i32 = 3;

/// C `shell_note`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShellNote {
    pub id: u32,
    pub app: [u8; 32],
    pub title: [u8; 48],
    pub body: [u8; 120],
    pub action: [u8; 24],
    pub action_id: u32,
    pub urgency: i32,
    pub time_ms: u64,
    pub read: bool,
    pub toast_until: u64,
}

impl ShellNote {
    pub const EMPTY: Self = Self {
        id: 0,
        app: [0; 32],
        title: [0; 48],
        body: [0; 120],
        action: [0; 24],
        action_id: 0,
        urgency: 0,
        time_ms: 0,
        read: false,
        toast_until: 0,
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Pending {
    pub app: [u8; 32],
    pub title: [u8; 48],
    pub body: [u8; 120],
    pub action: [u8; 24],
    pub action_id: u32,
    pub urgency: i32,
}

impl Pending {
    pub const EMPTY: Self = Self { app: [0; 32], title: [0; 48], body: [0; 120], action: [0; 24], action_id: 0, urgency: 0 };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Rate {
    pub app: [u8; 32],
    pub stamp: [u64; NOTIFY_RATE_COUNT],
    pub n: i32,
}

impl Rate {
    pub const EMPTY: Self = Self { app: [0; 32], stamp: [0; NOTIFY_RATE_COUNT], n: 0 };
}

pub type ActionFn = unsafe extern "C" fn(u32, u32, *mut c_void);

/// C `shell_notify`.
#[repr(C)]
#[allow(missing_docs)]
pub struct ShellNotify {
    pub perms: *mut c_void,
    pub comp: *mut Compositor,
    pub note: [ShellNote; NOTIFY_MAX],
    pub pending: [Pending; NOTIFY_PENDING],
    pub npending: i32,
    pub rate: [Rate; 16],
    pub nrate: i32,
    pub next_id: u32,
    pub dnd: bool,
    pub center_open: bool,
    pub scroll: i32,
    pub sel: i32,
    pub now_ms: u64,
    pub published: u32,
    pub dropped_denied: u32,
    pub dropped_rate: u32,
    pub dropped_full: u32,
    pub on_action: Option<ActionFn>,
    pub ctx: *mut c_void,
}

fn put(dst: &mut [u8], src: &[u8]) {
    dst.fill(0);
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
}
fn bytes0(a: &[u8]) -> &[u8] {
    let n = a.iter().position(|&b| b == 0).unwrap_or(a.len());
    &a[..n]
}
fn eq(a: &[u8], b: &[u8]) -> bool {
    bytes0(a) == bytes0(b)
}

extern "C" {
    fn perms_check(p: *mut c_void, app: *const c_char, res: i32) -> i32;
    fn perms_request(
        p: *mut c_void,
        app: *const c_char,
        res: i32,
        done: Option<unsafe extern "C" fn(*const c_char, i32, bool, *mut c_void)>,
        ctx: *mut c_void,
    ) -> i32;
}

fn comp<'a>(n: &ShellNotify) -> Option<&'a mut Compositor> {
    if n.comp.is_null() { None } else { Some(unsafe { &mut *n.comp }) }
}
fn dirty(n: &mut ShellNotify) {
    if let Some(c) = comp(n) {
        c.damage_all();
    }
}

pub fn init(n: &mut ShellNotify, perms: *mut c_void, c: *mut Compositor) {
    n.perms = perms;
    n.comp = c;
    n.note.fill(ShellNote::EMPTY);
    n.pending.fill(Pending::EMPTY);
    n.npending = 0;
    n.rate.fill(Rate::EMPTY);
    n.nrate = 0;
    n.next_id = 1;
    n.dnd = false;
    n.center_open = false;
    n.scroll = 0;
    n.sel = 0;
    n.now_ms = 0;
    n.published = 0;
    n.dropped_denied = 0;
    n.dropped_rate = 0;
    n.dropped_full = 0;
    n.on_action = None;
    n.ctx = core::ptr::null_mut();
}

pub fn install(n: &mut ShellNotify) {
    if let Some(c) = comp(n) {
        c.add_overlay(overlay, n as *mut ShellNotify as *mut c_void);
    }
}

fn rate_ok(n: &mut ShellNotify, app: &[u8]) -> bool {
    let mut i = 0usize;
    while i < n.nrate as usize {
        if eq(&n.rate[i].app, app) {
            break;
        }
        i += 1;
    }
    if i == n.nrate as usize {
        if n.nrate as usize == 16 {
            n.rate.copy_within(1..16, 0);
            i = 15;
            n.nrate = 15;
        }
        n.nrate += 1;
        n.rate[i] = Rate::EMPTY;
        put(&mut n.rate[i].app, app);
    }
    let mut keep = 0usize;
    for k in 0..n.rate[i].n as usize {
        if n.now_ms.wrapping_sub(n.rate[i].stamp[k]) < NOTIFY_RATE_WINDOW_MS {
            n.rate[i].stamp[keep] = n.rate[i].stamp[k];
            keep += 1;
        }
    }
    n.rate[i].n = keep as i32;
    if keep >= NOTIFY_RATE_COUNT {
        return false;
    }
    n.rate[i].stamp[n.rate[i].n as usize] = n.now_ms;
    n.rate[i].n += 1;
    true
}

fn add_note(n: &mut ShellNotify, app: &[u8], title: &[u8], body: &[u8], u: i32, action: &[u8], aid: u32) -> u32 {
    let mut slot = None;
    for i in 0..NOTIFY_MAX {
        if n.note[i].id == 0 {
            slot = Some(i);
            break;
        }
    }
    let idx = match slot {
        Some(i) => i,
        None => {
            let mut best = u64::MAX;
            let mut pick = 0usize;
            for i in 0..NOTIFY_MAX {
                let key = n.note[i].time_ms.wrapping_add(if n.note[i].read { 0 } else { 1u64 << 40 });
                if key < best {
                    best = key;
                    pick = i;
                }
            }
            n.dropped_full = n.dropped_full.wrapping_add(1);
            pick
        }
    };
    n.note[idx] = ShellNote::EMPTY;
    n.note[idx].id = n.next_id;
    n.next_id = n.next_id.wrapping_add(1);
    put(&mut n.note[idx].app, app);
    put(&mut n.note[idx].title, title);
    put(&mut n.note[idx].body, body);
    put(&mut n.note[idx].action, action);
    n.note[idx].action_id = aid;
    n.note[idx].urgency = u;
    n.note[idx].time_ms = n.now_ms;
    if !n.dnd || u == NOTE_URGENT {
        n.note[idx].toast_until = if u == NOTE_URGENT { u64::MAX } else { n.now_ms.wrapping_add(NOTIFY_TOAST_MS) };
    }
    n.published = n.published.wrapping_add(1);
    dirty(n);
    n.note[idx].id
}

unsafe extern "C" fn perm_done(app: *const c_char, _r: i32, allowed: bool, ctx: *mut c_void) {
    let Some(n) = (unsafe { (ctx as *mut ShellNotify).as_mut() }) else { return };
    let app = unsafe { ffi::c_bytes(app) };
    let mut i = 0usize;
    while i < n.npending as usize {
        if !eq(&n.pending[i].app, app) {
            i += 1;
            continue;
        }
        let p = n.pending[i];
        if allowed {
            let (title, body, action) = (p.title, p.body, p.action);
            add_note(n, app, bytes0(&title), bytes0(&body), p.urgency, bytes0(&action), p.action_id);
        } else {
            n.dropped_denied = n.dropped_denied.wrapping_add(1);
        }
        let cnt = n.npending as usize - i - 1;
        n.pending.copy_within(i + 1..i + 1 + cnt, i);
        n.npending -= 1;
    }
}

pub fn publish(n: &mut ShellNotify, app: &[u8], title: &[u8], body: &[u8], u: i32, action: &[u8], aid: u32) -> u32 {
    if app.is_empty() {
        return 0;
    }
    // SAFETY: perms points at the broker owned by the desktop; strings are NUL-terminated by callers.
    let s = unsafe { perms_check(n.perms, app.as_ptr() as *const c_char, RES_NOTIFY) };
    if s == PERM_DENY {
        n.dropped_denied = n.dropped_denied.wrapping_add(1);
        return 0;
    }
    if !rate_ok(n, app) {
        n.dropped_rate = n.dropped_rate.wrapping_add(1);
        return 0;
    }
    if s == PERM_ASK {
        if n.npending as usize >= NOTIFY_PENDING {
            n.dropped_full = n.dropped_full.wrapping_add(1);
            return 0;
        }
        let mut asked = false;
        for k in 0..n.npending as usize {
            if eq(&n.pending[k].app, app) {
                asked = true;
            }
        }
        let i = n.npending as usize;
        n.npending += 1;
        put(&mut n.pending[i].app, app);
        put(&mut n.pending[i].title, title);
        put(&mut n.pending[i].body, body);
        put(&mut n.pending[i].action, action);
        n.pending[i].action_id = aid;
        n.pending[i].urgency = u;
        if !asked {
            // SAFETY: broker + callback contract from perms.h.
            unsafe { perms_request(n.perms, app.as_ptr() as *const c_char, RES_NOTIFY, Some(perm_done), n as *mut ShellNotify as *mut c_void) };
        }
        return 0;
    }
    add_note(n, app, title, body, u, action, aid)
}

pub fn tick(n: &mut ShellNotify, now: u64) {
    n.now_ms = now;
    for i in 0..NOTIFY_MAX {
        if n.note[i].id != 0 && n.note[i].toast_until != 0 && n.note[i].toast_until != u64::MAX && now >= n.note[i].toast_until {
            n.note[i].toast_until = 0;
            dirty(n);
        }
    }
}

pub fn get(n: &ShellNotify, id: u32) -> Option<&ShellNote> {
    n.note.iter().find(|x| x.id == id)
}
pub fn dismiss(n: &mut ShellNotify, id: u32) {
    for i in 0..NOTIFY_MAX {
        if n.note[i].id == id {
            n.note[i] = ShellNote::EMPTY;
            dirty(n);
            return;
        }
    }
}
pub fn clear_all(n: &mut ShellNotify) {
    n.note.fill(ShellNote::EMPTY);
    n.scroll = 0;
    n.sel = 0;
    dirty(n);
}
pub fn clear_app(n: &mut ShellNotify, app: &[u8]) {
    for i in 0..NOTIFY_MAX {
        if n.note[i].id != 0 && eq(&n.note[i].app, app) {
            n.note[i] = ShellNote::EMPTY;
        }
    }
    dirty(n);
}
pub fn count(n: &ShellNotify) -> i32 {
    n.note.iter().filter(|x| x.id != 0).count() as i32
}
pub fn unread(n: &ShellNotify) -> i32 {
    n.note.iter().filter(|x| x.id != 0 && !x.read).count() as i32
}
pub fn visible_toasts(n: &ShellNotify) -> i32 {
    n.note.iter().filter(|x| x.id != 0 && x.toast_until != 0).count() as i32
}
pub fn set_dnd(n: &mut ShellNotify, on: bool) {
    n.dnd = on;
    if on {
        for x in n.note.iter_mut() {
            if x.urgency != NOTE_URGENT {
                x.toast_until = 0;
            }
        }
    }
    dirty(n);
}
pub fn set_center(n: &mut ShellNotify, open: bool) {
    n.center_open = open;
    if open {
        for x in n.note.iter_mut() {
            x.read = x.id != 0;
            x.toast_until = 0;
        }
    }
    dirty(n);
}

pub fn center_rect(c: &Compositor) -> Rect {
    Rect::new(c.lw() - 380, 8, 372, c.lh() - 40 - 16)
}
pub fn bell_rect(c: &Compositor) -> Rect {
    Rect::new(c.lw() - 150 + 70, c.lh() - 40 + 8, 24, 24)
}

fn ordered(n: &ShellNotify) -> [i32; NOTIFY_MAX] {
    let mut idx = [0i32; NOTIFY_MAX];
    let mut cnt = 0usize;
    for i in 0..NOTIFY_MAX {
        if n.note[i].id == 0 {
            continue;
        }
        let pos0 = cnt;
        cnt += 1;
        let mut pos = pos0;
        while pos > 0 && n.note[idx[pos - 1] as usize].id < n.note[i].id {
            idx[pos] = idx[pos - 1];
            pos -= 1;
        }
        idx[pos] = i as i32;
    }
    idx
}

const ROW_H: i32 = 76;
fn row_rect(c: &Compositor, row: i32, scroll: i32) -> Rect {
    let r = center_rect(c);
    Rect::new(r.x + 8, r.y + 76 + (row - scroll) * ROW_H, r.w - 16, ROW_H - 6)
}
fn toast_rect(c: &Compositor, slot: i32) -> Rect {
    Rect::new(c.lw() - 340, 8 + slot * 84, 332, 76)
}

pub fn mouse(n: &mut ShellNotify, x: i32, y: i32, down: bool) -> bool {
    let Some(c) = comp(n) else { return false };
    let x = c.logical(x);
    let y = c.logical(y);
    if down && contains(bell_rect(c), x, y) {
        let v = !n.center_open;
        set_center(n, v);
        return true;
    }
    if n.center_open {
        let cr = center_rect(c);
        if !contains(cr, x, y) {
            if down {
                set_center(n, false);
            }
            return false;
        }
        if !down {
            return true;
        }
        if contains(Rect::new(cr.x + cr.w - 110, cr.y + 10, 100, 28), x, y) {
            clear_all(n);
            return true;
        }
        if contains(Rect::new(cr.x + 12, cr.y + 42, 200, 24), x, y) {
            let v = !n.dnd;
            set_dnd(n, v);
            return true;
        }
        let idx = ordered(n);
        let cnt = count(n);
        for row in n.scroll..cnt {
            let rr = row_rect(c, row, n.scroll);
            if rr.y + rr.h > cr.y + cr.h {
                break;
            }
            if !contains(rr, x, y) {
                continue;
            }
            let nt = n.note[idx[row as usize] as usize];
            if contains(Rect::new(rr.x + rr.w - 26, rr.y + 4, 22, 22), x, y) {
                dismiss(n, nt.id);
                return true;
            }
            if nt.action[0] != 0 && contains(Rect::new(rr.x + rr.w - 108, rr.y + rr.h - 30, 100, 24), x, y) {
                if let Some(f) = n.on_action {
                    // SAFETY: host callback installed with a matching ctx.
                    unsafe { f(nt.id, nt.action_id, n.ctx) };
                }
                dismiss(n, nt.id);
                return true;
            }
        }
        return true;
    }
    if down {
        let mut slot = 0;
        for i in 0..NOTIFY_MAX {
            if slot >= NOTIFY_MAX_TOASTS {
                break;
            }
            if n.note[i].id == 0 || n.note[i].toast_until == 0 {
                continue;
            }
            let tr = toast_rect(c, slot as i32);
            slot += 1;
            if !contains(tr, x, y) {
                continue;
            }
            let nt = n.note[i];
            let act = nt.action[0] != 0 && x > tr.x + tr.w - 110 && y > tr.y + tr.h - 34;
            if act {
                if let Some(f) = n.on_action {
                    // SAFETY: host callback installed with a matching ctx.
                    unsafe { f(nt.id, nt.action_id, n.ctx) };
                }
            }
            dismiss(n, nt.id);
            return true;
        }
    }
    false
}

pub fn key(n: &mut ShellNotify, key: u32, down: bool, mods: u32) -> bool {
    if !down {
        return false;
    }
    if key == b'n' as u32 && (mods & MOD_SUPER) != 0 {
        let v = !n.center_open;
        set_center(n, v);
        return true;
    }
    if n.center_open && key == KEY_ESC {
        set_center(n, false);
        return true;
    }
    if n.center_open {
        let cnt = count(n);
        if key == KEY_DOWN && n.scroll + 1 < cnt {
            n.scroll += 1;
            dirty(n);
            return true;
        }
        if key == KEY_UP && n.scroll > 0 {
            n.scroll -= 1;
            dirty(n);
            return true;
        }
        if key == KEY_DELETE {
            clear_all(n);
            return true;
        }
    }
    false
}

fn fmt_u64(out: &mut [u8], v: u64) -> usize {
    let mut tmp = [0u8; 20];
    let mut n = 0;
    let mut v = v;
    if v == 0 {
        tmp[0] = b'0';
        n = 1;
    }
    while v != 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    let m = n.min(out.len().saturating_sub(1));
    for i in 0..m {
        out[i] = tmp[n - 1 - i];
    }
    out[m] = 0;
    m
}

fn age(now: u64, t: u64, out: &mut [u8; 12]) {
    let s = if now > t { (now - t) / 1000 } else { 0 };
    if s < 10 {
        out[..4].copy_from_slice(b"now\0");
    } else if s < 60 {
        let n = fmt_u64(out, s);
        out[n] = b's';
        out[n + 1] = 0;
    } else if s < 3600 {
        let n = fmt_u64(out, s / 60);
        out[n] = b'm';
        out[n + 1] = 0;
    } else {
        let n = fmt_u64(out, s / 3600);
        out[n] = b'h';
        out[n + 1] = 0;
    }
}

fn contains(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

fn text(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: &[u8], col: u32) {
    unsafe { ffi::fb_text(fb, clip, x, y, s.as_ptr() as *const c_char, col) };
}
fn width(s: &[u8]) -> i32 {
    unsafe { ffi::shell_text_width(s.as_ptr() as *const c_char) }
}

fn draw_note(c: &Compositor, fb: *mut Fb, clip: Rect, r: Rect, nt: &ShellNote, now: u64, toast: bool) {
    let t = &c.theme;
    if toast {
        unsafe { ffi::fb_fill_round(fb, clip, Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4), 10, if nt.urgency == NOTE_URGENT { t.danger } else { t.border }) };
    }
    unsafe { ffi::fb_fill_round(fb, clip, r, 8, if toast { t.surface } else { t.surface_hi }) };
    text(fb, clip, r.x + 10, r.y + 6, bytes0(&nt.app), t.text_dim);
    let mut tm = [0u8; 12];
    age(now, nt.time_ms, &mut tm);
    text(fb, clip, r.x + r.w - 30 - width(&tm), r.y + 6, &tm, t.text_dim);
    text(fb, clip, r.x + r.w - 20, r.y + 4, b"x\0", t.text_dim);
    text(fb, clip, r.x + 10, r.y + 24, bytes0(&nt.title), t.text);
    let mut l1 = [0u8; 44];
    let body = bytes0(&nt.body);
    let n = body.len().min(43);
    l1[..n].copy_from_slice(&body[..n]);
    text(fb, clip, r.x + 10, r.y + 44, &l1, t.text_dim);
    if nt.action[0] != 0 {
        let b = Rect::new(r.x + r.w - 108, r.y + r.h - 30, 100, 24);
        unsafe { ffi::fb_fill_round(fb, clip, b, 5, t.accent) };
        text(fb, clip, b.x + (b.w - width(bytes0(&nt.action))) / 2, b.y + 4, bytes0(&nt.action), t.accent_text);
    }
}

unsafe extern "C" fn overlay(c: *mut Compositor, fb: *mut Fb, clip: Rect, u: *mut c_void) {
    let (Some(c), Some(n)) = (unsafe { c.as_mut() }, unsafe { (u as *mut ShellNotify).as_mut() }) else { return };
    let mut view = unsafe { *fb };
    view.scale = c.scale_pct as i32;
    let lclip = c.clip_logical(clip);
    let t = &c.theme;
    let bell = bell_rect(c);
    let unread = unread(n);
    unsafe {
        ffi::fb_fill_round(&mut view, lclip, bell, 6, if n.center_open { mix(t.surface_hi, t.accent, 100) } else { t.surface });
        text(&mut view, lclip, bell.x + 8, bell.y + 4, if n.dnd { b"z\0" } else { b"!\0" }, if unread != 0 && !n.dnd { t.accent } else { t.text_dim });
        if unread != 0 && !n.dnd {
            ffi::fb_fill_round(&mut view, lclip, Rect::new(bell.x + 16, bell.y - 2, 10, 10), 5, t.danger);
        }
    }
    let idx = ordered(n);
    let cnt = count(n);
    if n.center_open {
        let cr = center_rect(c);
        unsafe {
            ffi::fb_fill_round(&mut view, lclip, Rect::new(cr.x - 2, cr.y - 2, cr.w + 4, cr.h + 4), 12, t.border);
            ffi::fb_fill_round(&mut view, lclip, cr, 10, t.surface);
        }
        text(&mut view, lclip, cr.x + 14, cr.y + 14, b"Notifications\0", t.text);
        let clr = Rect::new(cr.x + cr.w - 110, cr.y + 10, 100, 28);
        unsafe { ffi::fb_fill_round(&mut view, lclip, clr, 6, t.surface_hi) };
        text(&mut view, lclip, clr.x + 12, clr.y + 6, b"Clear all\0", t.text);
        text(&mut view, lclip, cr.x + 14, cr.y + 46, if n.dnd { b"[x] Do not disturb\0" } else { b"[ ] Do not disturb\0" }, t.text_dim);
        if cnt == 0 {
            text(&mut view, lclip, cr.x + 14, cr.y + 90, b"Nothing new.\0", t.text_dim);
        }
        for row in n.scroll..cnt {
            let rr = row_rect(c, row, n.scroll);
            if rr.y + rr.h > cr.y + cr.h {
                break;
            }
            let nt = n.note[idx[row as usize] as usize];
            draw_note(c, &mut view, lclip, rr, &nt, n.now_ms, false);
        }
        return;
    }
    let mut slot = 0;
    for i in 0..NOTIFY_MAX {
        if slot >= NOTIFY_MAX_TOASTS {
            break;
        }
        if n.note[i].id == 0 || n.note[i].toast_until == 0 {
            continue;
        }
        draw_note(c, &mut view, lclip, toast_rect(c, slot as i32), &n.note[i], n.now_ms, true);
        slot += 1;
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn notify_init(n: *mut ShellNotify, p: *mut c_void, c: *mut Compositor) {
    if let Some(n) = unsafe { n.as_mut() } {
        init(n, p, c);
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_install(n: *mut ShellNotify) {
    if let Some(n) = unsafe { n.as_mut() } {
        install(n);
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_publish(n: *mut ShellNotify, app: *const c_char, title: *const c_char, body: *const c_char, u: i32, action: *const c_char, aid: u32) -> u32 {
    if app.is_null() || title.is_null() {
        return 0;
    }
    match unsafe { n.as_mut() } {
        Some(n) => publish(n, unsafe { ffi::c_bytes(app) }, unsafe { ffi::c_bytes(title) }, unsafe { ffi::c_bytes(body) }, u, unsafe { ffi::c_bytes(action) }, aid),
        None => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_tick(n: *mut ShellNotify, now: u64) {
    if let Some(n) = unsafe { n.as_mut() } {
        tick(n, now);
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_dismiss(n: *mut ShellNotify, id: u32) {
    if let Some(n) = unsafe { n.as_mut() } {
        dismiss(n, id);
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_clear_all(n: *mut ShellNotify) {
    if let Some(n) = unsafe { n.as_mut() } {
        clear_all(n);
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_clear_app(n: *mut ShellNotify, app: *const c_char) {
    if let Some(n) = unsafe { n.as_mut() } {
        clear_app(n, unsafe { ffi::c_bytes(app) });
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_count(n: *const ShellNotify) -> i32 {
    unsafe { n.as_ref() }.map_or(0, count)
}
#[no_mangle]
pub unsafe extern "C" fn notify_unread(n: *const ShellNotify) -> i32 {
    unsafe { n.as_ref() }.map_or(0, unread)
}
#[no_mangle]
pub unsafe extern "C" fn notify_visible_toasts(n: *const ShellNotify) -> i32 {
    unsafe { n.as_ref() }.map_or(0, visible_toasts)
}
#[no_mangle]
pub unsafe extern "C" fn notify_get(n: *const ShellNotify, id: u32) -> *const ShellNote {
    match unsafe { n.as_ref() } {
        Some(n) => get(n, id).map_or(core::ptr::null(), |x| x as *const ShellNote),
        None => core::ptr::null(),
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_set_center(n: *mut ShellNotify, open: bool) {
    if let Some(n) = unsafe { n.as_mut() } {
        set_center(n, open);
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_set_dnd(n: *mut ShellNotify, on: bool) {
    if let Some(n) = unsafe { n.as_mut() } {
        set_dnd(n, on);
    }
}
#[no_mangle]
pub unsafe extern "C" fn notify_mouse(n: *mut ShellNotify, x: i32, y: i32, down: bool) -> bool {
    unsafe { n.as_mut() }.map_or(false, |n| mouse(n, x, y, down))
}
#[no_mangle]
pub unsafe extern "C" fn notify_key(n: *mut ShellNotify, key: u32, down: bool, mods: u32) -> bool {
    unsafe { n.as_mut() }.map_or(false, |n| self::key(n, key, down, mods))
}
#[no_mangle]
pub unsafe extern "C" fn notify_center_rect(c: *const Compositor) -> Rect {
    unsafe { c.as_ref() }.map_or(Rect::new(0, 0, 0, 0), center_rect)
}
#[no_mangle]
pub unsafe extern "C" fn notify_bell_rect(c: *const Compositor) -> Rect {
    unsafe { c.as_ref() }.map_or(Rect::new(0, 0, 0, 0), bell_rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ShellNote>(), 264);
        assert_eq!(offset_of!(ShellNote, app), 4);
        assert_eq!(offset_of!(ShellNote, title), 36);
        assert_eq!(offset_of!(ShellNote, body), 84);
        assert_eq!(offset_of!(ShellNote, action), 204);
        assert_eq!(offset_of!(ShellNote, action_id), 228);
        assert_eq!(offset_of!(ShellNote, urgency), 232);
        assert_eq!(offset_of!(ShellNote, time_ms), 240);
        assert_eq!(offset_of!(ShellNote, read), 248);
        assert_eq!(offset_of!(ShellNote, toast_until), 256);
        assert_eq!(size_of::<ShellNotify>(), 20120);
        assert_eq!(offset_of!(ShellNotify, note), 16);
        assert_eq!(offset_of!(ShellNotify, pending), 16912);
        assert_eq!(offset_of!(ShellNotify, npending), 18768);
        assert_eq!(offset_of!(ShellNotify, rate), 18776);
        assert_eq!(offset_of!(ShellNotify, nrate), 20056);
        assert_eq!(offset_of!(ShellNotify, next_id), 20060);
        assert_eq!(offset_of!(ShellNotify, center_open), 20065);
        assert_eq!(offset_of!(ShellNotify, scroll), 20068);
        assert_eq!(offset_of!(ShellNotify, now_ms), 20080);
        assert_eq!(offset_of!(ShellNotify, published), 20088);
        assert_eq!(offset_of!(ShellNotify, on_action), 20104);
        assert_eq!(offset_of!(ShellNotify, ctx), 20112);
    }

    #[test]
    fn history_counts_and_dismiss() {
        let n = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellNotify>() }));
        init(n, core::ptr::null_mut(), core::ptr::null_mut());
        // Bypass perms by publishing directly after setting ASK without a broker is not possible;
        // exercise add_note through the internal path instead.
        add_note(n, b"App", b"Title", b"Body", NOTE_NORMAL, b"", 0);
        add_note(n, b"App", b"Second", b"Body", NOTE_NORMAL, b"", 0);
        assert_eq!(count(n), 2);
        assert_eq!(unread(n), 2);
        let id = n.note[0].id;
        dismiss(n, id);
        assert_eq!(count(n), 1);
        set_center(n, true);
        assert_eq!(unread(n), 0);
        clear_all(n);
        assert_eq!(count(n), 0);
    }
}

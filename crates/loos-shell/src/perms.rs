//! Per-app permission broker prompts (C `perms.c`).
#![allow(missing_docs)]

use crate::backend::ShellBackend;
use crate::compositor::{Compositor, KEY_ESC};
use crate::draw::Rect;
use crate::draw::mix;
use crate::ffi::{self, Fb};
use core::ffi::{c_char, c_void};

pub const PERMS_MAX_REQ: usize = 16;
pub const PERMS_MAX_USE: usize = 16;
pub const RES_COUNT: usize = 5;
pub const PERM_ASK: i32 = 0;
pub const PERM_ALLOW: i32 = 1;
pub const PERM_DENY: i32 = 2;

pub const PERM_ANSWER_ALLOW_ONCE: i32 = 0;
pub const PERM_ANSWER_ALLOW_ALWAYS: i32 = 1;
pub const PERM_ANSWER_DENY: i32 = 2;

pub type DoneFn = unsafe extern "C" fn(*const c_char, i32, bool, *mut c_void);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PermRequest {
    pub app: [u8; 32],
    pub res: i32,
    pub done: Option<DoneFn>,
    pub ctx: *mut c_void,
}

impl PermRequest {
    pub const EMPTY: Self = Self { app: [0; 32], res: 0, done: None, ctx: core::ptr::null_mut() };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UseItem {
    pub app: [u8; 32],
    pub res: i32,
}

impl UseItem {
    pub const EMPTY: Self = Self { app: [0; 32], res: 0 };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OnceItem {
    pub app: [u8; 32],
    pub res: i32,
}

impl OnceItem {
    pub const EMPTY: Self = Self { app: [0; 32], res: 0 };
}

/// C `shell_perms`.
#[repr(C)]
#[allow(missing_docs)]
pub struct ShellPerms {
    pub be: *const ShellBackend,
    pub q: [PermRequest; PERMS_MAX_REQ],
    pub nq: i32,
    pub once: [OnceItem; 32],
    pub nonce: i32,
    pub use_: [UseItem; PERMS_MAX_USE],
    pub nuse: i32,
    pub prompts_shown: i32,
    pub denied_total: i32,
    pub comp: *mut Compositor,
}

const RES_NAMES: [&[u8]; RES_COUNT] = [b"Camera\0", b"Microphone\0", b"Files\0", b"Notifications\0", b"Location\0"];

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
fn be<'a>(p: &ShellPerms) -> Option<&'a ShellBackend> {
    if p.be.is_null() { None } else { Some(unsafe { &*p.be }) }
}
fn comp<'a>(p: &ShellPerms) -> Option<&'a mut Compositor> {
    if p.comp.is_null() { None } else { Some(unsafe { &mut *p.comp }) }
}
fn dirty(p: &ShellPerms) {
    if let Some(c) = comp(p) {
        c.damage_all();
    }
}

pub fn init(p: &mut ShellPerms, b: *const ShellBackend, c: *mut Compositor) {
    p.be = b;
    p.q = [PermRequest::EMPTY; PERMS_MAX_REQ];
    p.nq = 0;
    p.once = [OnceItem::EMPTY; 32];
    p.nonce = 0;
    p.use_ = [UseItem::EMPTY; PERMS_MAX_USE];
    p.nuse = 0;
    p.prompts_shown = 0;
    p.denied_total = 0;
    p.comp = c;
}

pub fn install(p: &mut ShellPerms) {
    if let Some(c) = comp(p) {
        c.add_overlay(overlay, p as *mut ShellPerms as *mut c_void);
    }
}

fn has_once(p: &ShellPerms, app: &[u8], r: i32) -> bool {
    for i in 0..p.nonce as usize {
        if p.once[i].res == r && eq(&p.once[i].app, app) {
            return true;
        }
    }
    false
}

pub fn check(p: &ShellPerms, app: &[u8], r: i32) -> i32 {
    let s = be(p).map_or(PERM_ASK, |b| b.perm_state_get(app, r));
    if s == PERM_ASK && has_once(p, app, r) { PERM_ALLOW } else { s }
}

pub fn request(p: &mut ShellPerms, app: &[u8], r: i32, done: Option<DoneFn>, ctx: *mut c_void) -> i32 {
    let s = check(p, app, r);
    if s != PERM_ASK {
        if let Some(f) = done {
            // SAFETY: host callback with matching signature.
            unsafe { f(app.as_ptr() as *const c_char, r, s == PERM_ALLOW, ctx) };
        }
        return s;
    }
    if p.nq as usize >= PERMS_MAX_REQ {
        if let Some(f) = done {
            // SAFETY: as above.
            unsafe { f(app.as_ptr() as *const c_char, r, false, ctx) };
        }
        return PERM_DENY;
    }
    let i = p.nq as usize;
    p.nq += 1;
    p.q[i] = PermRequest::EMPTY;
    put(&mut p.q[i].app, app);
    p.q[i].res = r;
    p.q[i].done = done;
    p.q[i].ctx = ctx;
    if p.nq == 1 {
        p.prompts_shown += 1;
    }
    dirty(p);
    PERM_ASK
}

pub fn prompting(p: &ShellPerms) -> bool {
    p.nq > 0
}

fn resolve_head(p: &mut ShellPerms, allowed: bool) {
    if p.nq == 0 {
        return;
    }
    let mut app = [0u8; 32];
    app.copy_from_slice(&p.q[0].app);
    let r = p.q[0].res;
    let mut done = [PermRequest::EMPTY; PERMS_MAX_REQ];
    let mut nd = 0usize;
    let mut i = 0usize;
    while i < p.nq as usize {
        if p.q[i].res == r && eq(&p.q[i].app, &app) {
            done[nd] = p.q[i];
            nd += 1;
            let n = p.nq as usize - i - 1;
            p.q.copy_within(i + 1..i + 1 + n, i);
            p.nq -= 1;
        } else {
            i += 1;
        }
    }
    if !allowed {
        p.denied_total += nd as i32;
    }
    for k in 0..nd {
        if let Some(f) = done[k].done {
            // SAFETY: host callback; app is NUL-terminated.
            unsafe { f(app.as_ptr() as *const c_char, r, allowed, done[k].ctx) };
        }
    }
    if p.nq > 0 {
        p.prompts_shown += 1;
    }
    dirty(p);
}

pub fn answer(p: &mut ShellPerms, a: i32) {
    if p.nq == 0 {
        return;
    }
    let mut app = [0u8; 32];
    app.copy_from_slice(&p.q[0].app);
    let r = p.q[0].res;
    match a {
        PERM_ANSWER_ALLOW_ONCE => {
            if p.nonce < 32 {
                p.once[p.nonce as usize] = OnceItem::EMPTY;
                p.once[p.nonce as usize].app = app;
                p.once[p.nonce as usize].res = r;
                p.nonce += 1;
            }
            resolve_head(p, true);
        }
        PERM_ANSWER_ALLOW_ALWAYS => {
            if let Some(b) = be(p) {
                b.perm_set(&app, r, true);
            }
            resolve_head(p, true);
        }
        _ => {
            if let Some(b) = be(p) {
                b.perm_set(&app, r, false);
            }
            resolve_head(p, false);
        }
    }
}

pub fn revoke_all(p: &mut ShellPerms, app: &[u8]) {
    if let Some(b) = be(p) {
        for r in 0..RES_COUNT as i32 {
            b.perm_reset(app, r);
        }
    }
    let mut i = 0usize;
    while i < p.nonce as usize {
        if eq(&p.once[i].app, app) {
            p.nonce -= 1;
            p.once[i] = p.once[p.nonce as usize];
        } else {
            i += 1;
        }
    }
    let mut i = 0usize;
    while i < p.nuse as usize {
        if eq(&p.use_[i].app, app) {
            p.nuse -= 1;
            p.use_[i] = p.use_[p.nuse as usize];
        } else {
            i += 1;
        }
    }
    dirty(p);
}

pub fn end_session(p: &mut ShellPerms) {
    p.nonce = 0;
}

pub fn set_in_use(p: &mut ShellPerms, app: &[u8], r: i32, active: bool) {
    for i in 0..p.nuse as usize {
        if p.use_[i].res == r && eq(&p.use_[i].app, app) {
            if !active {
                p.nuse -= 1;
                p.use_[i] = p.use_[p.nuse as usize];
                dirty(p);
            }
            return;
        }
    }
    if active && (p.nuse as usize) < PERMS_MAX_USE && check(p, app, r) == PERM_ALLOW {
        let i = p.nuse as usize;
        p.use_[i] = UseItem::EMPTY;
        put(&mut p.use_[i].app, app);
        p.use_[i].res = r;
        p.nuse += 1;
        dirty(p);
    }
}

pub fn in_use(p: &ShellPerms, r: i32, app_out: *mut c_char, cap: usize) -> bool {
    for i in 0..p.nuse as usize {
        if p.use_[i].res == r {
            if !app_out.is_null() && cap > 0 {
                // SAFETY: caller provides cap writable bytes.
                let out = unsafe { core::slice::from_raw_parts_mut(app_out as *mut u8, cap) };
                put(out, bytes0(&p.use_[i].app));
            }
            return true;
        }
    }
    false
}

pub fn prompt_rect(c: &Compositor) -> Rect {
    Rect::new(c.lw() / 2 - 230, c.lh() / 2 - 80, 460, 150)
}
fn btn_rect(c: &Compositor, i: i32) -> Rect {
    let r = prompt_rect(c);
    Rect::new(r.x + 16 + i * 146, r.y + r.h - 50, 136, 34)
}
fn contains(r: Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

pub fn mouse(p: &mut ShellPerms, x: i32, y: i32, down: bool) -> bool {
    if p.nq == 0 {
        return false;
    }
    let Some(c) = comp(p) else { return false };
    if !down {
        return true;
    }
    let x = c.logical(x);
    let y = c.logical(y);
    for i in 0..3 {
        if contains(btn_rect(c, i), x, y) {
            answer(p, if i == 0 { PERM_ANSWER_ALLOW_ONCE } else if i == 1 { PERM_ANSWER_ALLOW_ALWAYS } else { PERM_ANSWER_DENY });
            return true;
        }
    }
    true
}

pub fn key(p: &mut ShellPerms, key: u32) -> bool {
    if p.nq == 0 {
        return false;
    }
    if key == KEY_ESC {
        resolve_head(p, false);
    }
    true
}

fn text(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: &[u8], col: u32) {
    let mut b = [0u8; 128];
    let n = bytes0(s).len().min(127);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    unsafe { ffi::fb_text(fb, clip, x, y, b.as_ptr() as *const c_char, col) };
}
fn width(s: &[u8]) -> i32 {
    let mut b = [0u8; 128];
    let n = bytes0(s).len().min(127);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    unsafe { ffi::shell_text_width(b.as_ptr() as *const c_char) }
}

unsafe extern "C" fn overlay(c: *mut Compositor, fb: *mut Fb, clip: Rect, u: *mut c_void) {
    let (Some(c), Some(p)) = (unsafe { c.as_mut() }, unsafe { (u as *mut ShellPerms).as_mut() }) else { return };
    let mut view = unsafe { *fb };
    view.scale = c.scale_pct as i32;
    let lclip = c.clip_logical(clip);
    let t = &c.theme;
    let ind = [(0i32, 0xFFEF_4444u32, b"CAM\0".as_slice()), (1, 0xFFF5_9E0B, b"MIC\0".as_slice()), (4, 0xFF10_B981, b"LOC\0".as_slice())];
    let mut x = c.lw() - 150 - 12;
    for (r, col, tag) in ind {
        if !in_use(p, r, core::ptr::null_mut(), 0) {
            continue;
        }
        x -= 44;
        let rr = Rect::new(x, c.lh() - 40 + 8, 40, 24);
        unsafe { ffi::fb_fill_round(&mut view, lclip, rr, 6, col) };
        text(&mut view, lclip, rr.x + 8, rr.y + 4, tag, 0xFFFF_FFFF);
    }
    if p.nq == 0 {
        return;
    }
    let r = prompt_rect(c);
    unsafe {
        ffi::fb_fill(&mut view, lclip, Rect::new(0, 0, c.lw(), c.lh()), 0x6600_0000);
        ffi::fb_fill_round(&mut view, lclip, Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4), 12, t.accent);
        ffi::fb_fill_round(&mut view, lclip, r, 10, t.surface);
    }
    let mut line = [0u8; 96];
    let app = bytes0(&p.q[0].app);
    let mut n = app.len().min(60);
    line[..n].copy_from_slice(&app[..n]);
    let suffix = b" wants to use\0";
    for &b in suffix {
        line[n] = b;
        n += 1;
    }
    text(&mut view, lclip, r.x + 20, r.y + 18, &line[..n], t.text);
    let res_name = RES_NAMES[(p.q[0].res as usize).min(RES_COUNT - 1)];
    text(&mut view, lclip, r.x + 20, r.y + 42, res_name, t.accent);
    let waiting = p.nq - 1;
    if waiting != 0 {
        let mut w = [0u8; 48];
        let mut wn = 0;
        let mut v = waiting as u32;
        let mut tmp = [0u8; 12];
        let mut k = 0;
        if v == 0 {
            tmp[0] = b'0';
            k = 1;
        }
        while v != 0 {
            tmp[k] = b'0' + (v % 10) as u8;
            k += 1;
            v /= 10;
        }
        for i in (0..k).rev() {
            w[wn] = tmp[i];
            wn += 1;
        }
        for &b in b" more request(s) waiting\0" {
            w[wn] = b;
            wn += 1;
        }
        text(&mut view, lclip, r.x + 20, r.y + 68, &w[..wn], t.text_dim);
    }
    let lb = [b"Allow once\0".as_slice(), b"Always allow\0".as_slice(), b"Block\0".as_slice()];
    for i in 0..3 {
        let b = btn_rect(c, i);
        let hot = contains(b, c.mouse_x, c.mouse_y);
        let col = if i == 2 {
            if hot { t.danger } else { mix(t.surface_hi, t.danger, 70) }
        } else if hot {
            mix(t.surface_hi, t.accent, 120)
        } else {
            t.surface_hi
        };
        unsafe { ffi::fb_fill_round(&mut view, lclip, b, 6, col) };
        text(&mut view, lclip, b.x + (b.w - width(lb[i as usize])) / 2, b.y + 9, lb[i as usize], t.text);
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn perms_init(p: *mut ShellPerms, b: *const ShellBackend, c: *mut Compositor) {
    if let Some(p) = unsafe { p.as_mut() } {
        init(p, b, c);
    }
}
#[no_mangle]
pub unsafe extern "C" fn perms_install(p: *mut ShellPerms) {
    if let Some(p) = unsafe { p.as_mut() } {
        install(p);
    }
}
#[no_mangle]
pub unsafe extern "C" fn perms_check(p: *mut ShellPerms, app: *const c_char, r: i32) -> i32 {
    unsafe { p.as_ref() }.map_or(PERM_ASK, |p| check(p, unsafe { ffi::c_bytes(app) }, r))
}
#[no_mangle]
pub unsafe extern "C" fn perms_request(p: *mut ShellPerms, app: *const c_char, r: i32, done: Option<DoneFn>, ctx: *mut c_void) -> i32 {
    match unsafe { p.as_mut() } {
        Some(p) => request(p, unsafe { ffi::c_bytes(app) }, r, done, ctx),
        None => PERM_DENY,
    }
}
#[no_mangle]
pub unsafe extern "C" fn perms_answer(p: *mut ShellPerms, a: i32) {
    if let Some(p) = unsafe { p.as_mut() } {
        answer(p, a);
    }
}
#[no_mangle]
pub unsafe extern "C" fn perms_prompting(p: *const ShellPerms) -> bool {
    unsafe { p.as_ref() }.map_or(false, prompting)
}
#[no_mangle]
pub unsafe extern "C" fn perms_revoke_all(p: *mut ShellPerms, app: *const c_char) {
    if let Some(p) = unsafe { p.as_mut() } {
        revoke_all(p, unsafe { ffi::c_bytes(app) });
    }
}
#[no_mangle]
pub unsafe extern "C" fn perms_end_session(p: *mut ShellPerms) {
    if let Some(p) = unsafe { p.as_mut() } {
        end_session(p);
    }
}
#[no_mangle]
pub unsafe extern "C" fn perms_set_in_use(p: *mut ShellPerms, app: *const c_char, r: i32, active: bool) {
    if let Some(p) = unsafe { p.as_mut() } {
        set_in_use(p, unsafe { ffi::c_bytes(app) }, r, active);
    }
}
#[no_mangle]
pub unsafe extern "C" fn perms_in_use(p: *const ShellPerms, r: i32, app_out: *mut c_char, cap: usize) -> bool {
    unsafe { p.as_ref() }.map_or(false, |p| in_use(p, r, app_out, cap))
}
#[no_mangle]
pub unsafe extern "C" fn perms_prompt_rect(c: *const Compositor) -> Rect {
    unsafe { c.as_ref() }.map_or(Rect::new(0, 0, 0, 0), prompt_rect)
}
#[no_mangle]
pub unsafe extern "C" fn perms_mouse(p: *mut ShellPerms, x: i32, y: i32, down: bool) -> bool {
    unsafe { p.as_mut() }.map_or(false, |p| mouse(p, x, y, down))
}
#[no_mangle]
pub unsafe extern "C" fn perms_key(p: *mut ShellPerms, key: u32) -> bool {
    unsafe { p.as_mut() }.map_or(false, |p| self::key(p, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    fn zero() -> &'static mut ShellPerms {
        Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellPerms>() }))
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<PermRequest>(), 56);
        assert_eq!(size_of::<ShellPerms>(), 2664);
        assert_eq!(offset_of!(ShellPerms, be), 0);
        assert_eq!(offset_of!(ShellPerms, q), 8);
        assert_eq!(offset_of!(ShellPerms, nq), 904);
        assert_eq!(offset_of!(ShellPerms, once), 908);
        assert_eq!(offset_of!(ShellPerms, nonce), 2060);
        assert_eq!(offset_of!(ShellPerms, use_), 2064);
        assert_eq!(offset_of!(ShellPerms, nuse), 2640);
        assert_eq!(offset_of!(ShellPerms, prompts_shown), 2644);
        assert_eq!(offset_of!(ShellPerms, denied_total), 2648);
        assert_eq!(offset_of!(ShellPerms, comp), 2656);
    }

    #[test]
    fn queue_and_once_grants() {
        let p = zero();
        init(p, core::ptr::null(), core::ptr::null_mut());
        // No backend: every check is ASK. Two requests queue in order.
        assert_eq!(request(p, b"app.exe", 0, None, core::ptr::null_mut()), PERM_ASK);
        assert_eq!(request(p, b"app.exe", 0, None, core::ptr::null_mut()), PERM_ASK);
        assert_eq!(p.nq, 2);
        assert!(prompting(p));
        answer(p, PERM_ANSWER_ALLOW_ONCE);
        assert_eq!(p.nq, 0);
        assert_eq!(check(p, b"app.exe", 0), PERM_ALLOW);
        // Allow-once is scoped to the app+resource.
        assert_eq!(check(p, b"other.exe", 0), PERM_ASK);
        end_session(p);
        assert_eq!(check(p, b"app.exe", 0), PERM_ASK);
    }

    #[test]
    fn in_use_only_for_allowed_apps() {
        let p = zero();
        init(p, core::ptr::null(), core::ptr::null_mut());
        set_in_use(p, b"cam.exe", 0, true);
        assert_eq!(p.nuse, 0);
    }
}

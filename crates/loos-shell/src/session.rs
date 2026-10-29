//! Local accounts, sign-in policy, session state machine and login screen (C `session.c`).
//! Passwords are PBKDF2-HMAC-SHA256 via the crypto layer; failures back off exponentially.
#![allow(missing_docs)]

use crate::draw::mix;
use crate::ffi::{self, Fb};
use crate::font::ShellFont;
use crate::font_data;
use crate::ui::{self, UiCtx};
use core::ffi::{c_char, c_void};

pub const ACC_MAX: usize = 16;
pub const ACC_NAME_MAX: usize = 32;
pub const ACC_PW_MAX: usize = 64;
pub const ACC_FREE_TRIES: u32 = 5;
pub const ACC_LOCK_BASE_MS: u64 = 30_000;
pub const ACC_LOCK_MAX_MS: u64 = 900_000;

pub const AUTH_OK: i32 = 0;
pub const AUTH_BAD_PASSWORD: i32 = 1;
pub const AUTH_LOCKED_OUT: i32 = 2;
pub const AUTH_NO_SUCH_USER: i32 = 3;

pub const SESS_SIGNED_OUT: i32 = 0;
pub const SESS_ACTIVE: i32 = 1;
pub const SESS_LOCKED: i32 = 2;

pub const SESS_EV_LOGIN: i32 = 0;
pub const SESS_EV_LOCK: i32 = 1;
pub const SESS_EV_UNLOCK: i32 = 2;
pub const SESS_EV_LOGOUT: i32 = 3;
pub const SESS_EV_AUTOLOCK: i32 = 4;

pub type RandomFn = unsafe extern "C" fn(*mut u8, usize, *mut c_void);
pub type EventFn = unsafe extern "C" fn(i32, *const c_char, *mut c_void);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShellAccount {
    pub name: [u8; ACC_NAME_MAX],
    pub display: [u8; 48],
    pub salt: [u8; 16],
    pub hash: [u8; 32],
    pub iters: u32,
    pub has_password: bool,
    pub admin: bool,
    pub failures: u32,
    pub locked_until_ms: u64,
}

impl ShellAccount {
    pub const EMPTY: Self = Self {
        name: [0; ACC_NAME_MAX],
        display: [0; 48],
        salt: [0; 16],
        hash: [0; 32],
        iters: 0,
        has_password: false,
        admin: false,
        failures: 0,
        locked_until_ms: 0,
    };
}

#[repr(C)]
pub struct ShellAccounts {
    pub acc: [ShellAccount; ACC_MAX],
    pub n: i32,
    pub autologin: [u8; ACC_NAME_MAX],
    pub default_iters: u32,
    pub random: Option<RandomFn>,
    pub random_user: *mut c_void,
}

#[repr(C)]
pub struct ShellSession {
    pub db: *mut ShellAccounts,
    pub state: i32,
    pub user: [u8; ACC_NAME_MAX],
    pub login_count: u32,
    pub last_activity_ms: u64,
    pub idle_lock_ms: u32,
    pub on_event: Option<EventFn>,
    pub ctx: *mut c_void,
}

#[repr(C)]
pub struct ShellLogin {
    pub session: *mut ShellSession,
    pub sel: i32,
    pub pw: [u8; ACC_PW_MAX],
    pub pw_caret: i32,
    pub msg: [u8; 96],
    pub now_ms: u64,
    pub hour: i32,
    pub minute: i32,
    pub su_name: [u8; ACC_NAME_MAX],
    pub su_disp: [u8; 48],
    pub su_pw: [u8; ACC_PW_MAX],
    pub su_pw2: [u8; ACC_PW_MAX],
    pub su_c1: i32,
    pub su_c2: i32,
    pub su_c3: i32,
    pub su_c4: i32,
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
fn eq(a: &[u8], b: &[u8]) -> bool {
    bytes0(a) == bytes0(b)
}

pub fn accounts_init_core(db: &mut ShellAccounts, iters: u32) {
    db.acc.fill(ShellAccount::EMPTY);
    db.n = 0;
    db.autologin = [0; ACC_NAME_MAX];
    db.default_iters = if iters != 0 { iters } else { 100_000 };
    db.random = None;
    db.random_user = core::ptr::null_mut();
}

pub fn valid_name(n: &[u8]) -> bool {
    if n.is_empty() || n.len() >= ACC_NAME_MAX {
        return false;
    }
    n.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

pub fn find(db: &mut ShellAccounts, name: &[u8]) -> Option<usize> {
    for i in 0..db.n as usize {
        if eq(&db.acc[i].name, name) {
            return Some(i);
        }
    }
    None
}

fn derive(a: &ShellAccount, pw: &[u8], out: &mut [u8; 32]) {
    loos_security::hash::pbkdf2(pw, &a.salt, a.iters, out);
}

fn set_secret(db: &mut ShellAccounts, idx: usize, pw: &[u8], salt16: *const u8) -> bool {
    if pw.is_empty() {
        let a = &mut db.acc[idx];
        a.has_password = false;
        a.hash = [0; 32];
        a.salt = [0; 16];
        a.iters = db.default_iters;
        return true;
    }
    if pw.len() >= ACC_PW_MAX {
        return false;
    }
    if !salt16.is_null() {
        // SAFETY: caller supplies 16 salt bytes.
        db.acc[idx].salt.copy_from_slice(unsafe { core::slice::from_raw_parts(salt16, 16) });
    } else if let Some(r) = db.random {
        // SAFETY: host CSPRNG writes 16 bytes.
        unsafe { r(db.acc[idx].salt.as_mut_ptr(), 16, db.random_user) };
    } else {
        return false;
    }
    db.acc[idx].iters = db.default_iters;
    db.acc[idx].has_password = true;
    let mut h = [0u8; 32];
    derive(&db.acc[idx], pw, &mut h);
    db.acc[idx].hash = h;
    true
}

pub fn create(db: &mut ShellAccounts, name: &[u8], display: &[u8], pw: &[u8], salt16: *const u8, admin: bool) -> bool {
    if db.n as usize >= ACC_MAX || !valid_name(name) || find(db, name).is_some() {
        return false;
    }
    let idx = db.n as usize;
    db.acc[idx] = ShellAccount::EMPTY;
    put(&mut db.acc[idx].name, name);
    put(&mut db.acc[idx].display, if display.is_empty() { name } else { display });
    db.acc[idx].admin = admin || db.n == 0;
    if !set_secret(db, idx, pw, salt16) {
        return false;
    }
    db.n += 1;
    true
}

pub fn set_password(db: &mut ShellAccounts, name: &[u8], old_pw: Option<&[u8]>, new_pw: &[u8], salt16: *const u8) -> bool {
    let Some(idx) = find(db, name) else { return false };
    if db.acc[idx].has_password {
        let Some(old) = old_pw else { return false };
        let mut h = [0u8; 32];
        derive(&db.acc[idx], old, &mut h);
        if !loos_security::hash::ct_eq(&h, &db.acc[idx].hash) {
            return false;
        }
    }
    set_secret(db, idx, new_pw, salt16)
}

pub fn delete(db: &mut ShellAccounts, name: &[u8]) -> bool {
    let Some(idx) = find(db, name) else { return false };
    if db.acc[idx].admin {
        let admins = db.acc[..db.n as usize].iter().filter(|a| a.admin).count();
        if admins <= 1 {
            return false;
        }
    }
    if eq(&db.autologin, name) {
        db.autologin[0] = 0;
    }
    db.n -= 1;
    db.acc[idx] = db.acc[db.n as usize];
    true
}

pub fn verify(db: &mut ShellAccounts, name: &[u8], pw: &[u8], now: u64, retry_ms: *mut u32) -> i32 {
    let idx = match find(db, name) {
        Some(i) => i,
        None => {
            if !retry_ms.is_null() {
                unsafe { *retry_ms = 0 };
            }
            return AUTH_NO_SUCH_USER;
        }
    };
    if !retry_ms.is_null() {
        unsafe { *retry_ms = 0 };
    }
    if db.acc[idx].locked_until_ms > now {
        if !retry_ms.is_null() {
            unsafe { *retry_ms = (db.acc[idx].locked_until_ms - now) as u32 };
        }
        return AUTH_LOCKED_OUT;
    }
    let ok = if !db.acc[idx].has_password {
        pw.is_empty()
    } else {
        let mut h = [0u8; 32];
        derive(&db.acc[idx], pw, &mut h);
        loos_security::hash::ct_eq(&h, &db.acc[idx].hash)
    };
    if ok {
        db.acc[idx].failures = 0;
        db.acc[idx].locked_until_ms = 0;
        return AUTH_OK;
    }
    db.acc[idx].failures += 1;
    if db.acc[idx].failures >= ACC_FREE_TRIES {
        let shift = db.acc[idx].failures - ACC_FREE_TRIES;
        let mut d = if shift >= 16 { ACC_LOCK_MAX_MS } else { ACC_LOCK_BASE_MS << shift };
        if d > ACC_LOCK_MAX_MS {
            d = ACC_LOCK_MAX_MS;
        }
        db.acc[idx].locked_until_ms = now + d;
        if !retry_ms.is_null() {
            unsafe { *retry_ms = d as u32 };
        }
    }
    AUTH_BAD_PASSWORD
}

pub fn set_autologin(db: &mut ShellAccounts, name: &[u8]) -> bool {
    if name.is_empty() {
        db.autologin[0] = 0;
        return true;
    }
    if find(db, name).is_none() {
        return false;
    }
    put(&mut db.autologin, name);
    true
}

fn to_hex(b: &[u8], out: &mut [u8]) {
    const D: &[u8] = b"0123456789abcdef";
    for (i, &v) in b.iter().enumerate() {
        out[2 * i] = D[(v >> 4) as usize];
        out[2 * i + 1] = D[(v & 15) as usize];
    }
    let n = b.len() * 2;
    out[n] = 0;
}
fn from_hex(s: &[u8], out: &mut [u8]) -> bool {
    for i in 0..out.len() {
        let mut v = 0u8;
        for k in 0..2 {
            let c = *s.get(2 * i + k).unwrap_or(&0);
            let d = if c.is_ascii_digit() {
                c - b'0'
            } else if (b'a'..=b'f').contains(&c) {
                c - b'a' + 10
            } else {
                return false;
            };
            v = v * 16 + d;
        }
        out[i] = v;
    }
    true
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
                // SAFETY: caller buffer.
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
            // SAFETY: caller buffer.
            unsafe { *self.ptr.add(n) = 0 };
        }
    }
}

pub fn save(db: &ShellAccounts, out: *mut u8, cap: usize) -> usize {
    let mut w = Out { ptr: out, cap, len: 0 };
    w.put(b"autologin ");
    if db.autologin[0] != 0 { w.put(bytes0(&db.autologin)); } else { w.put(b"-"); }
    w.put(b"\n");
    for i in 0..db.n as usize {
        let a = &db.acc[i];
        let mut salt = [0u8; 33];
        let mut hash = [0u8; 65];
        to_hex(&a.salt, &mut salt);
        to_hex(&a.hash, &mut hash);
        w.put(b"account ");
        w.put(bytes0(&a.name));
        w.put(b" ");
        w.u32(a.iters);
        w.put(b" ");
        w.i32(a.admin as i32);
        w.put(b" ");
        w.i32(a.has_password as i32);
        w.put(b" ");
        w.put(&salt[..32]);
        w.put(b" ");
        w.put(&hash[..64]);
        w.put(b" ");
        w.put(bytes0(&a.display));
        w.put(b"\n");
    }
    w.finish();
    w.len
}

fn token<'a>(s: &'a [u8], pos: &mut usize, out: &mut [u8]) -> bool {
    let mut p = *pos;
    while p < s.len() && s[p] == b' ' {
        p += 1;
    }
    let start = p;
    while p < s.len() && s[p] != b' ' {
        p += 1;
    }
    let n = p - start;
    if n == 0 || n >= out.len() {
        return false;
    }
    out[..n].copy_from_slice(&s[start..p]);
    out[n] = 0;
    *pos = p;
    true
}
fn number(t: &[u8], v: &mut u32) -> bool {
    if t.is_empty() {
        return false;
    }
    let mut r: u32 = 0;
    for &c in t {
        if !c.is_ascii_digit() || r > 429_496_729 {
            return false;
        }
        r = r * 10 + (c - b'0') as u32;
    }
    *v = r;
    true
}

pub fn load(db: &mut ShellAccounts, text: &[u8]) -> i32 {
    let mut added = 0;
    let mut off = 0usize;
    while off < text.len() && text[off] != 0 {
        let rest = &text[off..];
        let len = rest.iter().position(|&b| b == b'\n' || b == 0).unwrap_or(rest.len());
        let line = &rest[..len.min(199)];
        off += len;
        if off < text.len() && text[off] == b'\n' {
            off += 1;
        }
        if line.starts_with(b"autologin ") {
            if &line[10..] != b"-" {
                put(&mut db.autologin, &line[10..]);
            }
            continue;
        }
        if !line.starts_with(b"account ") || db.n as usize >= ACC_MAX {
            continue;
        }
        let body = &line[8..];
        let mut p = 0usize;
        let mut name = [0u8; ACC_NAME_MAX];
        let mut tmp = [0u8; 72];
        let mut salt = [0u8; 40];
        let mut hash = [0u8; 72];
        let mut iters = 0u32;
        let mut admin = 0u32;
        let mut haspw = 0u32;
        if !token(body, &mut p, &mut name) || !token(body, &mut p, &mut tmp) || !number(bytes0(&tmp), &mut iters) {
            continue;
        }
        if !token(body, &mut p, &mut tmp) || !number(bytes0(&tmp), &mut admin) || !token(body, &mut p, &mut tmp) || !number(bytes0(&tmp), &mut haspw) {
            continue;
        }
        if !token(body, &mut p, &mut salt) || !token(body, &mut p, &mut hash) {
            continue;
        }
        if p < body.len() && body[p] == b' ' {
            p += 1;
        }
        if !valid_name(bytes0(&name)) || find(db, bytes0(&name)).is_some() || iters == 0 || iters > 10_000_000 {
            continue;
        }
        let idx = db.n as usize;
        db.acc[idx] = ShellAccount::EMPTY;
        put(&mut db.acc[idx].name, bytes0(&name));
        db.acc[idx].iters = iters;
        db.acc[idx].admin = admin != 0;
        db.acc[idx].has_password = haspw != 0;
        if bytes0(&salt).len() != 32 || bytes0(&hash).len() != 64 {
            continue;
        }
        if !from_hex(bytes0(&salt), &mut db.acc[idx].salt) || !from_hex(bytes0(&hash), &mut db.acc[idx].hash) {
            continue;
        }
        let disp = if p <= body.len() { &body[p..] } else { &[][..] };
        put(&mut db.acc[idx].display, disp);
        db.n += 1;
        added += 1;
    }
    added
}

// ---------------------------------------------------------------- session

pub fn session_init_core(s: &mut ShellSession, db: *mut ShellAccounts) {
    s.db = db;
    s.state = SESS_SIGNED_OUT;
    s.user = [0; ACC_NAME_MAX];
    s.login_count = 0;
    s.last_activity_ms = 0;
    s.idle_lock_ms = 0;
    s.on_event = None;
    s.ctx = core::ptr::null_mut();
}

fn fire(s: &ShellSession, e: i32) {
    if let Some(f) = s.on_event {
        // SAFETY: host callback; user is NUL-terminated.
        unsafe { f(e, s.user.as_ptr() as *const c_char, s.ctx) };
    }
}

pub fn session_boot_core(s: &mut ShellSession, now: u64) -> bool {
    let db = unsafe { &mut *s.db };
    let auto = db.autologin;
    if auto[0] != 0 {
        if let Some(i) = find(db, bytes0(&auto)) {
            let name = db.acc[i].name;
            s.user = name;
            s.state = SESS_ACTIVE;
            s.login_count += 1;
            s.last_activity_ms = now;
            fire(s, SESS_EV_LOGIN);
            return true;
        }
    }
    s.state = SESS_SIGNED_OUT;
    false
}

pub fn session_login_core(s: &mut ShellSession, user: &[u8], pw: &[u8], now: u64, retry: *mut u32) -> i32 {
    let db = unsafe { &mut *s.db };
    let r = verify(db, user, pw, now, retry);
    if r != AUTH_OK {
        return r;
    }
    put(&mut s.user, user);
    s.state = SESS_ACTIVE;
    s.login_count += 1;
    s.last_activity_ms = now;
    fire(s, SESS_EV_LOGIN);
    r
}

pub fn session_lock_core(s: &mut ShellSession) {
    if s.state == SESS_ACTIVE {
        s.state = SESS_LOCKED;
        fire(s, SESS_EV_LOCK);
    }
}

pub fn session_unlock_core(s: &mut ShellSession, pw: &[u8], now: u64, retry: *mut u32) -> i32 {
    if s.state != SESS_LOCKED {
        return AUTH_NO_SUCH_USER;
    }
    let db = unsafe { &mut *s.db };
    let user = s.user;
    let r = verify(db, bytes0(&user), pw, now, retry);
    if r == AUTH_OK {
        s.state = SESS_ACTIVE;
        s.last_activity_ms = now;
        fire(s, SESS_EV_UNLOCK);
    }
    r
}

pub fn session_logout_core(s: &mut ShellSession) {
    if s.state == SESS_SIGNED_OUT {
        return;
    }
    fire(s, SESS_EV_LOGOUT);
    s.state = SESS_SIGNED_OUT;
    s.user[0] = 0;
}

pub fn session_activity_core(s: &mut ShellSession, now: u64) {
    s.last_activity_ms = now;
}

pub fn session_tick_core(s: &mut ShellSession, now: u64) {
    if s.state == SESS_ACTIVE && s.idle_lock_ms != 0 && now.wrapping_sub(s.last_activity_ms) >= s.idle_lock_ms as u64 {
        s.state = SESS_LOCKED;
        fire(s, SESS_EV_AUTOLOCK);
    }
}

// ---------------------------------------------------------------- login screen

pub fn login_init_core(l: &mut ShellLogin, s: *mut ShellSession) {
    l.session = s;
    l.sel = 0;
    l.pw = [0; ACC_PW_MAX];
    l.pw_caret = 0;
    l.msg = [0; 96];
    l.now_ms = 0;
    l.hour = 0;
    l.minute = 0;
    l.su_name = [0; ACC_NAME_MAX];
    l.su_disp = [0; 48];
    l.su_pw = [0; ACC_PW_MAX];
    l.su_pw2 = [0; ACC_PW_MAX];
    l.su_c1 = 0;
    l.su_c2 = 0;
    l.su_c3 = 0;
    l.su_c4 = 0;
}

fn fb(u: &UiCtx) -> *mut Fb {
    u.fb
}
fn theme(u: &UiCtx) -> &crate::draw::Theme {
    // SAFETY: ui_begin contract.
    unsafe { &*u.theme }
}
fn set_msg(l: &mut ShellLogin, s: &[u8]) {
    put(&mut l.msg, s);
}
fn secs_msg(b: &mut [u8; 96], prefix: &[u8], secs: u32, suffix: &[u8]) {
    let mut n = 0;
    for &c in prefix {
        if c == 0 {
            break;
        }
        b[n] = c;
        n += 1;
    }
    let mut tmp = [0u8; 12];
    let mut k = 0;
    let mut v = secs;
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
        b[n] = tmp[i];
        n += 1;
    }
    for &c in suffix {
        if c == 0 {
            break;
        }
        b[n] = c;
        n += 1;
    }
    b[n] = 0;
}
fn msg_secs(l: &mut ShellLogin, prefix: &[u8], secs: u32, suffix: &[u8]) {
    let mut b = [0u8; 96];
    secs_msg(&mut b, prefix, secs, suffix);
    put(&mut l.msg, bytes0(&b));
}

fn account_tile(u: &mut UiCtx, r: crate::draw::Rect, a: &ShellAccount, sel: bool, clicked: &mut bool) {
    let t = theme(u);
    let over = ui::hovered(u, r);
    let f = fb(u);
    unsafe { ffi::fb_fill_round(f, u.clip, r, 10, if sel { mix(t.surface_hi, t.accent, 90) } else if over { t.surface_hi } else { t.surface }) };
    let av = crate::draw::Rect::new(r.x + r.w / 2 - 24, r.y + 14, 48, 48);
    let mut h = 0u32;
    for &c in bytes0(&a.name) {
        h = h.wrapping_mul(31).wrapping_add(c as u32);
    }
    unsafe { ffi::fb_fill_round(f, u.clip, av, 24, mix(t.accent, 0xFFFF_FFFF, (h % 120) as i32)) };
    let mut ini = [0u8; 2];
    ini[0] = match a.display.first() {
        Some(c) if c.is_ascii_lowercase() => c - 32,
        Some(&c) => c,
        None => b'?',
    };
    unsafe { ffi::fb_text(f, u.clip, av.x + 20, av.y + 16, ini.as_ptr() as *const c_char, t.accent_text) };
    let tw = crate::text::width(&font_data::regular(), 14, bytes0(&a.display));
    unsafe { ffi::fb_text(f, u.clip, r.x + (r.w - tw) / 2, r.y + 74, a.display.as_ptr() as *const c_char, t.text) };
    *clicked = u.pressed && over;
}

pub fn login_frame_core(l: &mut ShellLogin, u: &mut UiCtx, w: i32, h: i32) {
    // SAFETY: login_init installed a live session.
    let s = unsafe { &mut *l.session };
    let db = unsafe { &mut *s.db };
    let t = *theme(u);
    ui::backdrop(u, w, h);
    let clock = [
        b'0' + (l.hour / 10 % 10) as u8,
        b'0' + (l.hour % 10) as u8,
        b':',
        b'0' + (l.minute / 10 % 10) as u8,
        b'0' + (l.minute % 10) as u8,
        0,
    ];
    let big = ShellFont { face: core::ptr::addr_of!(font_data::loos_font_semibold), size: 64 };
    let cw = crate::text::width(&font_data::semibold(), 64, &clock);
    unsafe { ffi::fb_text_ex(u.fb, u.clip, w / 2 - cw / 2, 36, clock.as_ptr() as *const c_char, 0xFFF3_F4F6, big) };
    let lw = crate::text::width(&font_data::regular(), 14, b"Local only.");
    ui::label(u, w / 2 - lw / 2, 112, b"Local only.", t.text_dim);

    if db.n == 0 {
        let card = crate::draw::Rect::new(w / 2 - 200, h / 2 - 150, 400, 330);
        ui::panel(u, card, t.surface);
        ui::label(u, card.x + 20, card.y + 16, b"Create your local account", t.text);
        ui::label(u, card.x + 20, card.y + 38, b"It stays on this device. No cloud account.", t.text_dim);
        ui::textfield(u, crate::draw::Rect::new(card.x + 20, card.y + 70, 360, 30), &mut l.su_name, &mut l.su_c1, b"login name");
        ui::textfield(u, crate::draw::Rect::new(card.x + 20, card.y + 110, 360, 30), &mut l.su_disp, &mut l.su_c2, b"display name");
        ui::textfield_password(u, crate::draw::Rect::new(card.x + 20, card.y + 150, 360, 30), &mut l.su_pw, &mut l.su_c3, b"password");
        ui::textfield_password(u, crate::draw::Rect::new(card.x + 20, card.y + 190, 360, 30), &mut l.su_pw2, &mut l.su_c4, b"repeat password");
        if ui::button_accent(u, crate::draw::Rect::new(card.x + 20, card.y + 240, 160, 34), b"Create account") {
            if !valid_name(bytes0(&l.su_name)) {
                set_msg(l, b"Login name: a-z, 0-9, - and _ only.");
            } else if bytes0(&l.su_pw) != bytes0(&l.su_pw2) {
                set_msg(l, b"The passwords do not match.");
            } else if bytes0(&l.su_pw).len() < 4 && l.su_pw[0] != 0 {
                set_msg(l, b"Use at least 4 characters, or leave it empty.");
            } else if create(db, bytes0(&l.su_name), bytes0(&l.su_disp), bytes0(&l.su_pw), core::ptr::null(), true) {
                l.msg[0] = 0;
                l.su_pw = [0; ACC_PW_MAX];
                l.su_pw2 = [0; ACC_PW_MAX];
            } else {
                set_msg(l, b"Could not create the account.");
            }
        }
        ui::label(u, card.x + 20, card.y + 290, bytes0(&l.msg), t.danger);
        return;
    }

    if s.state == SESS_LOCKED {
        if let Some(i) = find(db, bytes0(&s.user)) {
            l.sel = i as i32;
        }
    }
    if l.sel >= db.n {
        l.sel = 0;
    }
    let shown = if s.state == SESS_LOCKED { 1 } else { db.n };
    let (tw, gap) = (120, 12);
    let x0 = w / 2 - (shown * (tw + gap) - gap) / 2;
    let y0 = 170;
    for i in 0..shown {
        let idx = if s.state == SESS_LOCKED { l.sel } else { i };
        let mut click = false;
        let a = db.acc[idx as usize];
        account_tile(u, crate::draw::Rect::new(x0 + i * (tw + gap), y0, tw, 110), &a, idx == l.sel, &mut click);
        if click && s.state != SESS_LOCKED && l.sel != idx {
            l.sel = idx;
            l.pw[0] = 0;
            l.pw_caret = 0;
            l.msg[0] = 0;
        }
    }
    let cur = db.acc[l.sel as usize];
    let mut py = y0 + 130;
    let mut submit = false;
    if cur.has_password {
        let pr = crate::draw::Rect::new(w / 2 - 150, py, 300, 34);
        if u.focus == 0 {
            u.focus = ui::textfield_id(b"password", pr);
        }
        submit = ui::textfield_password(u, pr, &mut l.pw, &mut l.pw_caret, b"password");
        py += 44;
    } else {
        let nw = crate::text::width(&font_data::regular(), 14, b"No password set");
        ui::label(u, w / 2 - nw / 2, py + 8, b"No password set", t.text_dim);
    }
    let label: &[u8] = if s.state == SESS_LOCKED { b"Unlock" } else { b"Sign in" };
    if ui::button_accent(u, crate::draw::Rect::new(w / 2 - 150, py, 300, 34), label) {
        submit = true;
    }
    if submit {
        let mut retry = 0u32;
        let name = cur.name;
        let r = if s.state == SESS_LOCKED {
            session_unlock_core(s, bytes0(&l.pw), l.now_ms, &mut retry)
        } else {
            session_login_core(s, bytes0(&name), bytes0(&l.pw), l.now_ms, &mut retry)
        };
        match r {
            AUTH_OK => l.msg[0] = 0,
            AUTH_BAD_PASSWORD => {
                if retry != 0 {
                    msg_secs(l, b"Wrong password. Try again in ", (retry + 999) / 1000, b" s.");
                } else {
                    set_msg(l, b"Wrong password.");
                }
            }
            AUTH_LOCKED_OUT => msg_secs(l, b"Too many attempts. Try again in ", (retry + 999) / 1000, b" s."),
            _ => set_msg(l, b"No such user."),
        }
        l.pw = [0; ACC_PW_MAX];
        l.pw_caret = 0;
    }
    ui::label(u, w / 2 - 150, py + 44, bytes0(&l.msg), t.danger);
    if cur.locked_until_ms > l.now_ms {
        let mut b = [0u8; 96];
        secs_msg(&mut b, b"Locked for ", ((cur.locked_until_ms - l.now_ms + 999) / 1000) as u32, b" more seconds");
        ui::label(u, w / 2 - 150, py + 66, bytes0(&b), t.text_dim);
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn accounts_init(db: *mut ShellAccounts, iters: u32) {
    if let Some(db) = unsafe { db.as_mut() } {
        accounts_init_core(db, iters);
    }
}
#[no_mangle]
pub unsafe extern "C" fn account_find(db: *mut ShellAccounts, name: *const c_char) -> *mut ShellAccount {
    match unsafe { db.as_mut() } {
        Some(db) => match find(db, unsafe { ffi::c_bytes(name) }) {
            Some(i) => &mut db.acc[i],
            None => core::ptr::null_mut(),
        },
        None => core::ptr::null_mut(),
    }
}
#[no_mangle]
pub unsafe extern "C" fn account_valid_name(name: *const c_char) -> bool {
    valid_name(unsafe { ffi::c_bytes(name) })
}
#[no_mangle]
pub unsafe extern "C" fn account_create(db: *mut ShellAccounts, name: *const c_char, display: *const c_char, pw: *const c_char, salt16: *const u8, admin: bool) -> bool {
    match unsafe { db.as_mut() } {
        Some(db) => create(db, unsafe { ffi::c_bytes(name) }, unsafe { ffi::c_bytes(display) }, unsafe { ffi::c_bytes(pw) }, salt16, admin),
        None => false,
    }
}
#[no_mangle]
pub unsafe extern "C" fn account_set_password(db: *mut ShellAccounts, name: *const c_char, old_pw: *const c_char, new_pw: *const c_char, salt16: *const u8) -> bool {
    match unsafe { db.as_mut() } {
        Some(db) => {
            let old = if old_pw.is_null() { None } else { Some(unsafe { ffi::c_bytes(old_pw) }) };
            set_password(db, unsafe { ffi::c_bytes(name) }, old, unsafe { ffi::c_bytes(new_pw) }, salt16)
        }
        None => false,
    }
}
#[no_mangle]
pub unsafe extern "C" fn account_delete(db: *mut ShellAccounts, name: *const c_char) -> bool {
    unsafe { db.as_mut() }.map_or(false, |db| delete(db, unsafe { ffi::c_bytes(name) }))
}
#[no_mangle]
pub unsafe extern "C" fn account_verify(db: *mut ShellAccounts, name: *const c_char, pw: *const c_char, now: u64, retry_ms: *mut u32) -> i32 {
    match unsafe { db.as_mut() } {
        Some(db) => verify(db, unsafe { ffi::c_bytes(name) }, unsafe { ffi::c_bytes(pw) }, now, retry_ms),
        None => AUTH_NO_SUCH_USER,
    }
}
#[no_mangle]
pub unsafe extern "C" fn accounts_set_autologin(db: *mut ShellAccounts, name: *const c_char) -> bool {
    unsafe { db.as_mut() }.map_or(false, |db| set_autologin(db, unsafe { ffi::c_bytes(name) }))
}
#[no_mangle]
pub unsafe extern "C" fn accounts_save(db: *const ShellAccounts, out: *mut c_char, cap: usize) -> usize {
    match unsafe { db.as_ref() } {
        Some(db) => save(db, out as *mut u8, cap),
        None => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn accounts_load(db: *mut ShellAccounts, text: *const c_char) -> i32 {
    match (unsafe { db.as_mut() }, unsafe { text.as_ref() }) {
        (Some(db), Some(_)) => {
            let mut n = 0usize;
            // SAFETY: NUL-terminated.
            while unsafe { *text.add(n) } != 0 {
                n += 1;
            }
            // SAFETY: n bytes before the NUL.
            load(db, unsafe { core::slice::from_raw_parts(text as *const u8, n) })
        }
        _ => 0,
    }
}
#[no_mangle]
pub unsafe extern "C" fn session_init(s: *mut ShellSession, db: *mut ShellAccounts) {
    if let Some(s) = unsafe { s.as_mut() } {
        session_init_core(s, db);
    }
}
#[no_mangle]
pub unsafe extern "C" fn session_boot(s: *mut ShellSession, now: u64) -> bool {
    unsafe { s.as_mut() }.map_or(false, |s| session_boot_core(s, now))
}
#[no_mangle]
pub unsafe extern "C" fn session_login(s: *mut ShellSession, user: *const c_char, pw: *const c_char, now: u64, retry: *mut u32) -> i32 {
    match unsafe { s.as_mut() } {
        Some(s) => session_login_core(s, unsafe { ffi::c_bytes(user) }, unsafe { ffi::c_bytes(pw) }, now, retry),
        None => AUTH_NO_SUCH_USER,
    }
}
#[no_mangle]
pub unsafe extern "C" fn session_lock(s: *mut ShellSession) {
    if let Some(s) = unsafe { s.as_mut() } {
        session_lock_core(s);
    }
}
#[no_mangle]
pub unsafe extern "C" fn session_unlock(s: *mut ShellSession, pw: *const c_char, now: u64, retry: *mut u32) -> i32 {
    unsafe { s.as_mut() }.map_or(AUTH_NO_SUCH_USER, |s| session_unlock_core(s, unsafe { ffi::c_bytes(pw) }, now, retry))
}
#[no_mangle]
pub unsafe extern "C" fn session_logout(s: *mut ShellSession) {
    if let Some(s) = unsafe { s.as_mut() } {
        session_logout_core(s);
    }
}
#[no_mangle]
pub unsafe extern "C" fn session_activity(s: *mut ShellSession, now: u64) {
    if let Some(s) = unsafe { s.as_mut() } {
        session_activity_core(s, now);
    }
}
#[no_mangle]
pub unsafe extern "C" fn session_tick(s: *mut ShellSession, now: u64) {
    if let Some(s) = unsafe { s.as_mut() } {
        session_tick_core(s, now);
    }
}
#[no_mangle]
pub unsafe extern "C" fn login_init(l: *mut ShellLogin, s: *mut ShellSession) {
    if let Some(l) = unsafe { l.as_mut() } {
        login_init_core(l, s);
    }
}
#[no_mangle]
pub unsafe extern "C" fn login_frame(l: *mut ShellLogin, u: *mut UiCtx, w: i32, h: i32) {
    if let (Some(l), Some(u)) = (unsafe { l.as_mut() }, unsafe { u.as_mut() }) {
        login_frame_core(l, u, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    fn db() -> &'static mut ShellAccounts {
        let d = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellAccounts>() }));
        accounts_init_core(d, 1000);
        d
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ShellAccount>(), 152);
        assert_eq!(offset_of!(ShellAccount, salt), 80);
        assert_eq!(offset_of!(ShellAccount, hash), 96);
        assert_eq!(offset_of!(ShellAccount, locked_until_ms), 144);
        assert_eq!(size_of::<ShellAccounts>(), 2488);
        assert_eq!(offset_of!(ShellAccounts, n), 2432);
        assert_eq!(offset_of!(ShellAccounts, autologin), 2436);
        assert_eq!(offset_of!(ShellAccounts, random), 2472);
        assert_eq!(size_of::<ShellSession>(), 80);
        assert_eq!(offset_of!(ShellSession, user), 12);
        assert_eq!(offset_of!(ShellSession, last_activity_ms), 48);
        assert_eq!(size_of::<ShellLogin>(), 416);
        assert_eq!(offset_of!(ShellLogin, pw), 12);
        assert_eq!(offset_of!(ShellLogin, su_name), 192);
        assert_eq!(offset_of!(ShellLogin, su_c1), 400);
    }

    #[test]
    fn name_rules_and_admin_delete() {
        let d = db();
        assert!(valid_name(b"alice"));
        assert!(valid_name(b"a-1_2"));
        assert!(!valid_name(b"Alice"));
        assert!(!valid_name(b""));
        let salt = [7u8; 16];
        assert!(create(d, b"alice", b"Alice", b"secret", salt.as_ptr(), true));
        assert!(!create(d, b"alice", b"", b"", core::ptr::null(), false));
        assert!(!delete(d, b"alice"), "the last administrator cannot be removed");
        assert!(create(d, b"bob", b"Bobby", b"pw", salt.as_ptr(), true));
        assert!(delete(d, b"alice"));
        assert_eq!(d.n, 1);
        assert_eq!(bytes0(&d.acc[0].name), b"bob");
    }

    #[test]
    fn lockout_doubles_and_is_enforced() {
        let d = db();
        let salt = [1u8; 16];
        assert!(create(d, b"user", b"User", b"pw", salt.as_ptr(), true));
        for i in 0..4 {
            let mut retry = 0;
            assert_eq!(verify(d, b"user", b"bad", 1000, &mut retry), AUTH_BAD_PASSWORD, "{i}");
            assert_eq!(retry, 0);
        }
        let mut retry = 0;
        assert_eq!(verify(d, b"user", b"bad", 1000, &mut retry), AUTH_BAD_PASSWORD);
        assert_eq!(retry, 30_000);
        assert_eq!(d.acc[0].locked_until_ms, 31_000);
        assert_eq!(verify(d, b"user", b"pw", 2000, &mut retry), AUTH_LOCKED_OUT);
        assert_eq!(retry, 29_000);
        // After the lock expires, the next failure doubles the delay.
        assert_eq!(verify(d, b"user", b"bad", 31_000, &mut retry), AUTH_BAD_PASSWORD);
        assert_eq!(retry, 60_000);
        assert_eq!(d.acc[0].locked_until_ms, 91_000);
    }

    #[test]
    fn save_load_round_trip() {
        let d = db();
        let salt = [0xABu8; 16];
        assert!(create(d, b"alice", b"Alice A", b"pw1", salt.as_ptr(), true));
        assert!(create(d, b"bob", b"Bob", b"", salt.as_ptr(), false));
        assert!(set_autologin(d, b"alice"));
        let mut out = [0u8; 512];
        let n = save(d, out.as_mut_ptr(), out.len());
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.starts_with("autologin alice\n"));
        assert!(s.contains("account alice 1000 1 1 ababab"));
        let d2 = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellAccounts>() }));
        accounts_init_core(d2, 1000);
        assert_eq!(load(d2, &out[..n]), 2);
        assert_eq!(d2.acc[0].salt, [0xAB; 16]);
        assert!(d2.acc[0].has_password && !d2.acc[1].has_password);
        assert_eq!(bytes0(&d2.autologin), b"alice");
    }

    #[test]
    fn session_transitions_and_idle_lock() {
        let d = db();
        let salt = [2u8; 16];
        create(d, b"alice", b"Alice", b"pw", salt.as_ptr(), true);
        let s = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellSession>() }));
        session_init_core(s, d);
        assert_eq!(s.state, SESS_SIGNED_OUT);
        let mut retry = 0;
        assert_eq!(session_login_core(s, b"alice", b"pw", 100, &mut retry), AUTH_OK);
        assert_eq!(s.state, SESS_ACTIVE);
        assert_eq!(bytes0(&s.user), b"alice");
        session_lock_core(s);
        assert_eq!(s.state, SESS_LOCKED);
        assert_eq!(session_unlock_core(s, b"wrong", 200, &mut retry), AUTH_BAD_PASSWORD);
        assert_eq!(session_unlock_core(s, b"pw", 300, &mut retry), AUTH_OK);
        s.idle_lock_ms = 50;
        session_activity_core(s, 1000);
        session_tick_core(s, 1049);
        assert_eq!(s.state, SESS_ACTIVE);
        session_tick_core(s, 1050);
        assert_eq!(s.state, SESS_LOCKED);
        session_logout_core(s);
        assert_eq!(s.state, SESS_SIGNED_OUT);
    }
}

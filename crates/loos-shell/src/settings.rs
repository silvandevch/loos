//! Settings / privacy control center (C `settings.c`), including the Personalize theme page.
#![allow(missing_docs)]

use crate::backend::{NetConn, ShellBackend, FwRule};
use crate::compositor::KEY_ESC;
use crate::draw::Rect;
use crate::draw::mix;
use crate::ffi::{self};
use crate::font::ShellFont;
use crate::theme::{self, ThemeSpec, CStrPtr, TB_HC};
use crate::ui::{self, UiCtx};
use crate::util;
use core::ffi::{c_char, c_void};

pub const PG_PRIVACY: i32 = 0;
pub const PG_NETWORK: i32 = 1;
pub const PG_PERMISSIONS: i32 = 2;
pub const PG_UPDATES: i32 = 3;
pub const PG_APPEARANCE: i32 = 4;
pub const PG_ACCESS: i32 = 5;
pub const PG_ACCOUNTS: i32 = 6;
pub const PG_PERSONALIZE: i32 = 7;
pub const PG_LOOKS: i32 = 8;
pub const PG_BEHAVIOR: i32 = 9;
pub const PG_COUNT: usize = 10;

const MEM_MAX_APPS: usize = 32;
const MEM_MAX_RULES: usize = 64;
const RES_COUNT: i32 = 5;
const FW_ALLOW: i32 = 1;
const FW_DENY: i32 = 2;
const NET_IDLE: i32 = 0;
const NET_ACTIVE: i32 = 1;
const NET_BLOCKED: i32 = 2;

pub const QS_NET: i32 = 0;
pub const QS_DARK: i32 = 1;
pub const QS_LARGE: i32 = 2;
pub const QS_CENTER: i32 = 3;
pub const QS_LABELS: i32 = 4;
pub const QS_MOTION: i32 = 5;
pub const QS_ACCENT: i32 = 6;
pub const QS_OPACITY: i32 = 7;

#[no_mangle]
pub static settings_page_name: [CStrPtr; PG_COUNT] = [
    CStrPtr(b"Privacy\0".as_ptr() as *const c_char),
    CStrPtr(b"Network firewall\0".as_ptr() as *const c_char),
    CStrPtr(b"App permissions\0".as_ptr() as *const c_char),
    CStrPtr(b"Updates\0".as_ptr() as *const c_char),
    CStrPtr(b"Appearance\0".as_ptr() as *const c_char),
    CStrPtr(b"Accessibility\0".as_ptr() as *const c_char),
    CStrPtr(b"Accounts\0".as_ptr() as *const c_char),
    CStrPtr(b"Personalize\0".as_ptr() as *const c_char),
    CStrPtr(b"Looks\0".as_ptr() as *const c_char),
    CStrPtr(b"Behavior\0".as_ptr() as *const c_char),
];
const PAGE_NAMES: [&[u8]; PG_COUNT] = [
    b"Privacy\0",
    b"Network firewall\0",
    b"App permissions\0",
    b"Updates\0",
    b"Appearance\0",
    b"Accessibility\0",
    b"Accounts\0",
    b"Personalize\0",
    b"Looks\0",
    b"Behavior\0",
];
const PAGE_IDS: [&[u8]; PG_COUNT] = [b"privacy", b"firewall", b"permissions", b"updates", b"appearance", b"accessibility", b"accounts", b"personalize", b"looks", b"behavior"];

#[no_mangle]
pub static settings_accents: [u32; 6] = [0xFF3B_82F6, 0xFF8B_5CF6, 0xFF10_B981, 0xFFF5_9E0B, 0xFFEF_4444, 0xFFEC_4899];

#[repr(C)]
pub struct ShellSettings {
    pub be: *const ShellBackend,
    pub comp: *mut crate::compositor::Compositor,
    pub taskbar: *mut crate::taskbar::ShellTaskbar,
    pub page: i32,
    pub conn_sel: i32,
    pub conn_scroll: i32,
    pub app_sel: i32,
    pub app_scroll: i32,
    pub account_sel: i32,
    pub account_scroll: i32,
    pub mirror: [u8; 96],
    pub mirror_caret: i32,
    pub theme: i32,
    pub scale: i32,
    pub accent: i32,
    pub conns: [NetConn; 32],
    pub nconns: i32,
    pub apps: [[u8; 32]; MEM_MAX_APPS],
    pub napps: i32,
    pub spec: ThemeSpec,
    pub p_scroll: i32,
    pub p_token: i32,
    pub p_hex: [u8; 12],
    pub p_hex_caret: i32,
    pub p_io: [u8; 480],
    pub p_io_caret: i32,
    pub changes: i32,
}

pub(crate) fn bytes0(a: &[u8]) -> &[u8] {
    let n = a.iter().position(|&b| b == 0).unwrap_or(a.len());
    &a[..n]
}
fn eq(a: &[u8], b: &[u8]) -> bool { bytes0(a) == bytes0(b) }
fn put(dst: &mut [u8], src: &[u8]) {
    dst.fill(0);
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
}
pub(crate) fn be<'a>(s: &ShellSettings) -> &'a ShellBackend { unsafe { &*s.be } }
fn comp<'a>(s: &ShellSettings) -> Option<&'a mut crate::compositor::Compositor> {
    if s.comp.is_null() { None } else { Some(unsafe { &mut *s.comp }) }
}
pub(crate) fn num_str(out: &mut [u8], mut v: u32) -> usize {
    let mut tmp = [0u8; 12];
    let mut n = 0;
    if v == 0 { tmp[0] = b'0'; n = 1; }
    while v != 0 { tmp[n] = b'0' + (v % 10) as u8; n += 1; v /= 10; }
    for i in (0..n).rev() { out[n - 1 - i] = tmp[i]; }
    n
}
fn line2(a: &[u8], n: u32, b: &[u8], out: &mut [u8; 96]) -> usize {
    let mut k = 0;
    for &c in a { out[k] = c; k += 1; }
    k += num_str(&mut out[k..], n);
    for &c in b { out[k] = c; k += 1; }
    k
}
fn line3(a: &[u8], n: u32, b: &[u8], n2: u32, c: &[u8], out: &mut [u8; 96]) -> usize {
    let mut k = line2(a, n, b, out);
    k += num_str(&mut out[k..], n2);
    for &x in c { out[k] = x; k += 1; }
    k
}

pub fn page_by_id(id: &[u8]) -> Option<i32> {
    if !id.starts_with(b"settings:") {
        return None;
    }
    let rest = &id[9..];
    for (i, p) in PAGE_IDS.iter().enumerate() {
        if rest == *p {
            return Some(i as i32);
        }
    }
    None
}

pub fn apply(s: &mut ShellSettings) {
    let b = be(s);
    s.theme = crate::backend::cfg_int(b, b"ui.theme\0", 0);
    s.accent = crate::backend::cfg_int(b, b"ui.accent\0", 0);
    s.scale = crate::backend::cfg_int(b, b"ui.scale\0", 100);
    if !(0..=5).contains(&s.accent) { s.accent = 0; }
    if crate::backend::cfg_bool(b, b"ui.high_contrast\0", false) { s.theme = 2; }
    let accent = settings_accents[s.accent as usize];
    theme::spec_load(&mut s.spec, b, accent);
    if crate::backend::cfg_bool(b, b"ui.high_contrast\0", false) { s.spec.base = TB_HC; }
    if let Some(c) = comp(s) {
        theme::apply(&s.spec, c);
        if c.scale_pct != s.scale as u32 { c.set_scale(s.scale as u32); }
        c.reduce_motion = crate::backend::cfg_bool(b, b"ui.reduce_motion\0", false);
        c.high_contrast = s.spec.base == TB_HC;
        c.damage_all();
    }
    if !s.taskbar.is_null() {
        let t = unsafe { &mut *s.taskbar };
        let off = b.net_off_get();
        if t.net_hard_off != off {
            t.net_hard_off = off;
            if let Some(c) = comp(s) {
                let r = crate::taskbar::rect(c);
                c.damage.add(r);
            }
        }
    }
}

pub fn init(s: &mut ShellSettings, b: *const ShellBackend, c: *mut crate::compositor::Compositor, tb: *mut crate::taskbar::ShellTaskbar) {
    s.be = b;
    s.comp = c;
    s.taskbar = tb;
    s.page = PG_PRIVACY;
    s.conn_sel = 0;
    s.conn_scroll = 0;
    s.app_sel = 0;
    s.app_scroll = 0;
    s.account_sel = 0;
    s.account_scroll = 0;
    s.mirror = [0; 96];
    s.mirror_caret = 0;
    s.theme = 0;
    s.scale = 100;
    s.accent = 0;
    s.conns = [NetConn::EMPTY; 32];
    s.nconns = 0;
    s.apps = [[0; 32]; MEM_MAX_APPS];
    s.napps = 0;
    s.spec = ThemeSpec::EMPTY;
    s.p_scroll = 0;
    s.p_token = 0;
    s.p_hex = [0; 12];
    s.p_hex_caret = 0;
    s.p_io = [0; 480];
    s.p_io_caret = 0;
    s.changes = 0;
    let b = be(s);
    if !b.cfg_get(b"updates.mirror\0", &mut s.mirror) { s.mirror[0] = 0; }
    s.mirror_caret = bytes0(&s.mirror).len() as i32;
    apply(s);
}

pub fn net_state(s: &ShellSettings) -> i32 {
    let b = be(s);
    if b.net_off_get() { return NET_BLOCKED; }
    if s.nconns > 0 { NET_ACTIVE } else { NET_IDLE }
}

pub(crate) fn changed(s: &mut ShellSettings) {
    s.changes += 1;
    apply(s);
}

pub(crate) fn cfg_toggle(s: &mut ShellSettings, u: &mut UiCtx, r: Rect, label: &[u8], key: &[u8], def: bool) {
    let b = be(s);
    let mut v = crate::backend::cfg_bool(b, key, def);
    if ui::toggle(u, r, label, &mut v) {
        crate::backend::cfg_set_bool(b, key, v);
        changed(s);
    }
}

fn text(u: &UiCtx, x: i32, y: i32, s: &[u8], col: u32) {
    let mut b = [0u8; 128];
    let n = bytes0(s).len().min(127);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    unsafe { ffi::fb_text(u.fb, u.clip, x, y, b.as_ptr() as *const c_char, col) };
}
fn text_ex(u: &UiCtx, x: i32, y: i32, s: &[u8], col: u32, size: i32) {
    let mut b = [0u8; 128];
    let n = bytes0(s).len().min(127);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    let f = ShellFont { face: core::ptr::addr_of!(crate::font_data::loos_font_semibold), size };
    unsafe { ffi::fb_text_ex(u.fb, u.clip, x, y, b.as_ptr() as *const c_char, col, f) };
}
fn width(s: &[u8]) -> i32 {
    let mut b = [0u8; 128];
    let n = bytes0(s).len().min(127);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    unsafe { ffi::shell_text_width(b.as_ptr() as *const c_char) }
}

unsafe extern "C" fn row_conn(u: *mut UiCtx, row: Rect, i: i32, sel: bool, user: *mut c_void) {
    let (Some(u), Some(s)) = (unsafe { u.as_mut() }, unsafe { (user as *mut ShellSettings).as_mut() }) else { return };
    let c = s.conns[i as usize];
    let mut rem = [0u8; 64];
    let mut k = 0;
    for &x in bytes0(&c.remote) { rem[k] = x; k += 1; }
    rem[k] = b':'; k += 1;
    k += num_str(&mut rem[k..], c.port as u32);
    let a = be(s).fw_check(c.proc.as_ptr() as *const c_char);
    let th = unsafe { *u.theme };
    let tc = if sel { th.accent_text } else { th.text };
    ui::label(u, row.x + 8, row.y + 4, bytes0(&c.proc), tc);
    ui::label(u, row.x + 150, row.y + 4, bytes0(&rem[..k]), tc);
    ui::label(u, row.x + 340, row.y + 4, bytes0(&c.proto), th.text_dim);
    ui::label(u, row.x + 390, row.y + 4, bytes0(&c.state), th.text_dim);
    ui::label_right(u, row.x + row.w - 8, row.y + 4, if a == FW_ALLOW { b"allowed" } else if a == FW_DENY { b"blocked" } else { b"ask" }, if a == FW_DENY { th.danger } else { th.text_dim });
}

unsafe extern "C" fn row_app(u: *mut UiCtx, row: Rect, i: i32, sel: bool, user: *mut c_void) {
    let (Some(u), Some(s)) = (unsafe { u.as_mut() }, unsafe { (user as *mut ShellSettings).as_mut() }) else { return };
    let b = be(s);
    let mut granted = 0;
    for r in 0..RES_COUNT {
        if b.perm_get(s.apps[i as usize].as_ptr() as *const c_char, r) { granted += 1; }
    }
    let mut buf = [0u8; 24];
    let k = num_str(&mut buf, granted);
    for &c in b" granted" { buf[k] = c; }
    let th = unsafe { *u.theme };
    ui::label(u, row.x + 8, row.y + 4, bytes0(&s.apps[i as usize]), if sel { th.accent_text } else { th.text });
    ui::label_right(u, row.x + row.w - 8, row.y + 4, bytes0(&buf[..k + 8]), th.text_dim);
}

unsafe extern "C" fn row_text(u: *mut UiCtx, row: Rect, i: i32, sel: bool, user: *mut c_void) {
    let (Some(u), _) = (unsafe { u.as_mut() }, ()) else { return };
    let name = unsafe { &*(user as *const [u8; 32]).add(i as usize) };
    let th = unsafe { *u.theme };
    ui::label(u, row.x + 8, row.y + 4, bytes0(name), if sel { th.accent_text } else { th.text });
}

fn page_privacy(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    let mut y = c.y;
    ui::heading(u, c.x, y, b"Privacy");
    y += 44;
    ui::label(u, c.x, y, b"Telemetry: none. This system contains no telemetry or analytics code.", t.text);
    y += 28;
    y += ui::paragraph(u, c.x, y, c.w - 8, b"Nothing leaves this device unless you start it, and the network LED shows when anything does.", t.text_dim) + 18;
    let mut line = [0u8; 96];
    let k = line2(b"Open network connections: ", s.nconns.max(0) as u32, b"", &mut line);
    ui::label(u, c.x, y, bytes0(&line[..k]), t.text);
    y += 24;
    let be = be(s);
    let mut rules = [FwRule::EMPTY; MEM_MAX_RULES];
    let nr = be.fw_list(rules.as_mut_ptr(), MEM_MAX_RULES as i32).max(0).min(MEM_MAX_RULES as i32);
    let blocked = rules[..nr as usize].iter().filter(|r| r.action == FW_DENY).count();
    let k = line3(b"Firewall rules: ", nr as u32, b" (", blocked as u32, b" blocking)", &mut line);
    ui::label(u, c.x, y, bytes0(&line[..k]), t.text);
    y += 24;
    let mut grants = 0;
    for a in 0..s.napps as usize {
        for r in 0..RES_COUNT {
            if be.perm_get(s.apps[a].as_ptr() as *const c_char, r) { grants += 1; }
        }
    }
    let k = line2(b"Permission grants across ", s.napps.max(0) as u32, b" apps: ", &mut line);
    let k = {
        let mut n = k;
        n += num_str(&mut line[n..], grants);
        n
    };
    ui::label(u, c.x, y, bytes0(&line[..k]), t.text);
    y += 40;
    let mut off = be.net_off_get();
    if ui::toggle(u, Rect::new(c.x, y, 360, 28), b"Block all network access", &mut off) {
        be.net_off_set(off);
        changed(s);
    }
    y += 40;
    ui::label(u, c.x, y, b"Updates are opt-in; see the Updates page.", t.text_dim);
}

fn page_network(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    ui::heading(u, c.x, c.y, b"Network firewall");
    let ns = net_state(s);
    let led = if ns == NET_BLOCKED { 0xFFEF_4444u32 } else if ns == NET_ACTIVE { 0xFFF5_9E0B } else { 0xFF4B_5563 };
    unsafe { ffi::fb_fill_round(u.fb, u.clip, Rect::new(c.x + c.w - 150, c.y + 8, 12, 12), 6, led) };
    ui::label(u, c.x + c.w - 130, c.y + 6, if ns == NET_BLOCKED { b"blocked" } else if ns == NET_ACTIVE { b"active" } else { b"idle" }, t.text_dim);
    let be = be(s);
    let mut off = be.net_off_get();
    if ui::toggle(u, Rect::new(c.x, c.y + 44, 300, 26), b"Block all network access", &mut off) {
        be.net_off_set(off);
        changed(s);
    }
    ui::label(u, c.x, c.y + 80, b"Process", t.text_dim);
    ui::label(u, c.x + 150, c.y + 80, b"Remote", t.text_dim);
    ui::label(u, c.x + 340, c.y + 80, b"Proto", t.text_dim);
    ui::label(u, c.x + 390, c.y + 80, b"State", t.text_dim);
    s.nconns = be.net_query(s.conns.as_mut_ptr(), 32).max(0).min(32);
    let lr = Rect::new(c.x, c.y + 100, c.w, 220);
    ui::list(u, lr, b"conns", s.nconns, 24, Some(row_conn), s as *mut ShellSettings as *mut c_void, &mut s.conn_sel, &mut s.conn_scroll);
    if s.nconns == 0 {
        ui::label(u, lr.x + 12, lr.y + 12, b"No open connections.", t.text_dim);
    }
    let y = lr.y + lr.h + 12;
    if s.conn_sel >= 0 && s.conn_sel < s.nconns {
        let proc_ = s.conns[s.conn_sel as usize].proc;
        let mut hd = [0u8; 64];
        let mut n = 0;
        for &c in b"Rule for " { hd[n] = c; n += 1; }
        for &c in bytes0(&proc_) { hd[n] = c; n += 1; }
        hd[n] = b':'; n += 1;
        ui::label(u, c.x, y + 6, bytes0(&hd[..n]), t.text);
        let cur = be.fw_check(proc_.as_ptr() as *const c_char);
        if ui::button(u, Rect::new(c.x + 240, y, 90, 28), b"Allow") && cur != FW_ALLOW {
            be.fw_set(proc_.as_ptr() as *const c_char, FW_ALLOW);
            changed(s);
        }
        if ui::button(u, Rect::new(c.x + 336, y, 90, 28), b"Block") && cur != FW_DENY {
            be.fw_set(proc_.as_ptr() as *const c_char, FW_DENY);
            changed(s);
        }
        if ui::button(u, Rect::new(c.x + 432, y, 90, 28), b"Ask") {
            be.fw_set(proc_.as_ptr() as *const c_char, 0);
            changed(s);
        }
    } else {
        ui::label(u, c.x, y + 6, b"Select a connection to allow or block its process.", t.text_dim);
    }
}

const RES_NAMES: [&[u8]; 5] = [b"Camera\0", b"Microphone\0", b"Files\0", b"Notifications\0", b"Location\0"];

fn page_permissions(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    ui::heading(u, c.x, c.y, b"App permissions");
    let be = be(s);
    s.napps = be.perm_apps(s.apps.as_mut_ptr() as *mut u8, MEM_MAX_APPS as i32).max(0).min(MEM_MAX_APPS as i32);
    let lr = Rect::new(c.x, c.y + 50, 240, 280);
    ui::list(u, lr, b"perm_apps", s.napps, 26, Some(row_app), s as *mut ShellSettings as *mut c_void, &mut s.app_sel, &mut s.app_scroll);
    if s.napps == 0 {
        ui::label(u, lr.x + 10, lr.y + 10, b"No apps registered.", t.text_dim);
    }
    if s.app_sel >= 0 && s.app_sel < s.napps {
        let app = s.apps[s.app_sel as usize];
        let x = c.x + 270;
        let mut y = c.y + 56;
        ui::label(u, x, y, bytes0(&app), t.text);
        y += 32;
        for r in 0..RES_COUNT {
            let mut v = be.perm_get(app.as_ptr() as *const c_char, r);
            if ui::toggle(u, Rect::new(x, y, 300, 28), RES_NAMES[r as usize], &mut v) {
                be.perm_set(bytes0(&app), r, v);
                changed(s);
            }
            y += 36;
        }
        ui::label(u, x, y + 8, b"Apps start with nothing granted.", t.text_dim);
    }
}

fn page_updates(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    ui::heading(u, c.x, c.y, b"Updates");
    cfg_toggle(s, u, Rect::new(c.x, c.y + 50, 420, 28), b"Check for updates (opt-in)", b"updates.enabled\0", false);
    cfg_toggle(s, u, Rect::new(c.x, c.y + 90, 420, 28), b"Offline mode (never contact a mirror)", b"updates.offline\0", false);
    ui::label(u, c.x, c.y + 136, b"Update mirror (self-hostable):", t.text);
    if ui::textfield(u, Rect::new(c.x, c.y + 160, 420, 30), &mut s.mirror, &mut s.mirror_caret, b"https://mirror.example/loos") {
        let mirror = s.mirror;
        be(s).cfg_set(b"updates.mirror\0", bytes0(&mirror));
        changed(s);
    }
    ui::label(u, c.x, c.y + 204, b"Press Enter to save. The default mirror is empty: no address is built in.", t.text_dim);
    let b = be(s);
    let en = crate::backend::cfg_bool(b, b"updates.enabled\0", false);
    let off = crate::backend::cfg_bool(b, b"updates.offline\0", false);
    let st: &[u8] = if !en {
        b"Status: updates are off."
    } else if off {
        b"Status: offline mode, no checks."
    } else if s.mirror[0] != 0 {
        b"Status: will check your mirror when you press Check."
    } else {
        b"Status: set a mirror first."
    };
    ui::label(u, c.x, c.y + 240, st, t.text);
}

fn page_appearance(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    ui::heading(u, c.x, c.y, b"Appearance");
    ui::label(u, c.x, c.y + 48, b"Theme", t.text_dim);
    let hc = crate::backend::cfg_bool(be(s), b"ui.high_contrast\0", false);
    let mut th = if s.theme == 2 && !hc { 2 } else { s.theme };
    const NAMES: [&[u8]; 3] = [b"Dark\0", b"Light\0", b"High contrast\0"];
    for i in 0..3 {
        if ui::radio(u, Rect::new(c.x + i * 150, c.y + 72, 140, 24), NAMES[i as usize], &mut th, i) {
            crate::backend::cfg_set_int(be(s), b"ui.theme\0", i);
            crate::backend::cfg_set_bool(be(s), b"ui.high_contrast\0", i == 2);
            changed(s);
        }
    }
    ui::label(u, c.x, c.y + 116, b"Accent", t.text_dim);
    for i in 0..6 {
        let sw = Rect::new(c.x + i * 44, c.y + 140, 32, 32);
        let hot = ui::hovered(u, sw);
        unsafe { ffi::fb_fill_round(u.fb, u.clip, sw, 8, settings_accents[i as usize]) };
        if s.accent == i {
            unsafe { ffi::fb_frame(u.fb, u.clip, Rect::new(sw.x - 3, sw.y - 3, sw.w + 6, sw.h + 6), t.text) };
        } else if hot {
            unsafe { ffi::fb_frame(u.fb, u.clip, Rect::new(sw.x - 2, sw.y - 2, sw.w + 4, sw.h + 4), t.text_dim) };
        }
        if u.pressed && hot {
            crate::backend::cfg_set_int(be(s), b"ui.accent\0", i);
            changed(s);
        }
    }
    ui::label(u, c.x, c.y + 192, b"Scale", t.text_dim);
    const SCALES: [i32; 4] = [100, 125, 150, 200];
    let mut cur = s.scale;
    for i in 0..4usize {
        let mut lb = [0u8; 8];
        let k = num_str(&mut lb, SCALES[i] as u32);
        lb[k] = b'%'; lb[k + 1] = 0;
        if ui::radio(u, Rect::new(c.x + i as i32 * 90, c.y + 216, 80, 24), bytes0(&lb), &mut cur, SCALES[i]) {
            crate::backend::cfg_set_int(be(s), b"ui.scale\0", SCALES[i]);
            changed(s);
        }
    }
    cfg_toggle(s, u, Rect::new(c.x, c.y + 262, 380, 28), b"Reduce motion", b"ui.reduce_motion\0", false);
}

fn page_access(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    ui::heading(u, c.x, c.y, b"Accessibility");
    cfg_toggle(s, u, Rect::new(c.x, c.y + 50, 420, 28), b"High contrast", b"ui.high_contrast\0", false);
    cfg_toggle(s, u, Rect::new(c.x, c.y + 90, 420, 28), b"Reduce motion", b"ui.reduce_motion\0", false);
    let mut large = s.scale >= 150;
    if ui::toggle(u, Rect::new(c.x, c.y + 130, 420, 28), b"Large text and controls (150%)", &mut large) {
        crate::backend::cfg_set_int(be(s), b"ui.scale\0", if large { 150 } else { 100 });
        changed(s);
    }
    cfg_toggle(s, u, Rect::new(c.x, c.y + 170, 420, 28), b"Sticky keys", b"access.sticky_keys\0", false);
    cfg_toggle(s, u, Rect::new(c.x, c.y + 210, 420, 28), b"Screen reader bridge (UI Automation)", b"access.uia\0", false);
    ui::label(u, c.x, c.y + 256, b"Full keyboard navigation is always on. Ctrl+Space opens the command palette.", t.text_dim);
}

fn pers_commit(s: &mut ShellSettings) {
    theme::spec_store(&s.spec, be(s));
    changed(s);
}

pub fn quick(s: &mut ShellSettings, action: i32, value: u32) {
    let b = be(s);
    let t = s.taskbar;
    match action {
        QS_NET => {
            let off = !b.net_off_get();
            b.net_off_set(off);
            changed(s);
        }
        QS_DARK => {
            s.spec.base = if s.spec.base == theme::TB_DARK { theme::TB_LIGHT } else { theme::TB_DARK };
            pers_commit(s);
        }
        QS_ACCENT => {
            s.spec.accent = value | 0xFF00_0000;
            pers_commit(s);
        }
        QS_OPACITY => {
            s.spec.opacity = value as i32;
            pers_commit(s);
        }
        QS_LARGE => {
            let v = if crate::backend::cfg_int(b, b"ui.scale\0", 100) >= 150 { 100 } else { 150 };
            crate::backend::cfg_set_int(b, b"ui.scale\0", v);
            changed(s);
        }
        QS_MOTION => {
            let v = !crate::backend::cfg_bool(b, b"ui.reduce_motion\0", false);
            crate::backend::cfg_set_bool(b, b"ui.reduce_motion\0", v);
            changed(s);
        }
        QS_CENTER | QS_LABELS => {
            if t.is_null() { return; }
            let tb = unsafe { &mut *t };
            let v = if action == QS_CENTER { !tb.centered } else { !tb.labels };
            crate::backend::cfg_set_bool(b, if action == QS_CENTER { b"ui.taskbar.center\0" } else { b"ui.taskbar.labels\0" }, v);
            let c = crate::backend::cfg_bool(b, b"ui.taskbar.center\0", true);
            let l = crate::backend::cfg_bool(b, b"ui.taskbar.labels\0", false);
            crate::taskbar::configure(tb, c, l, tb.top, tb.size);
            s.changes += 1;
        }
        _ => {}
    }
}

pub(crate) fn pers_slider(u: &mut UiCtx, x: i32, y: i32, w: i32, label: &[u8], v: &mut i32, lo: i32, hi: i32) -> bool {
    let mut txt = [0u8; 48];
    let mut n = 0;
    for &c in bytes0(label) { txt[n] = c; n += 1; }
    txt[n] = b':'; n += 1;
    txt[n] = b' '; n += 1;
    let vv = *v;
    if vv < 0 {
        txt[n] = b'-'; n += 1;
        n += num_str(&mut txt[n..], vv.unsigned_abs());
    } else {
        n += num_str(&mut txt[n..], vv as u32);
    }
    ui::label(u, x, y, bytes0(&txt[..n]), unsafe { (*u.theme).text_dim });
    ui::slider(u, Rect::new(x, y + 16, w, 22), label, v, lo, hi)
}

fn page_personalize(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    let saved = ui::clip_push(u, c);
    s.p_scroll = s.p_scroll.max(0);
    let y = |v: i32, s: &ShellSettings| v + c.y - s.p_scroll;
    ui::heading(u, c.x, y(0, s), b"Personalize");
    ui::label(u, c.x, y(46, s), b"Presets", t.text_dim);
    let n = theme::preset_count();
    let cols = 4;
    let bw = (c.w - 3 * 8) / cols;
    for i in 0..n {
        let r = Rect::new(c.x + (i % cols) * (bw + 8), y(66 + (i / cols) * 32, s), bw, 26);
        let name = unsafe { ffi::c_bytes(theme::theme_preset_name(i)) };
        if ui::button(u, r, name) {
            unsafe { theme::theme_preset(i, &mut s.spec) };
            pers_commit(s);
        }
    }
    let top = 66 + ((n + cols - 1) / cols) * 32 + 12;
    ui::label(u, c.x, y(top, s), b"Accent color", t.text_dim);
    let mut hh = 0;
    let mut ss = 0;
    let mut ll = 0;
    theme::to_hsl(s.spec.accent, &mut hh, &mut ss, &mut ll);
    let half = (c.w - 16) / 2;
    let x2 = c.x + half + 16;
    let mut ch = false;
    ch |= pers_slider(u, c.x, y(top + 22, s), half - 80, b"Hue", &mut hh, 0, 359);
    ch |= pers_slider(u, c.x, y(top + 66, s), half - 80, b"Saturation", &mut ss, 0, 100);
    ch |= pers_slider(u, c.x, y(top + 110, s), half - 80, b"Lightness", &mut ll, 10, 90);
    if ch {
        s.spec.accent = theme::hsl(hh, ss, ll);
        pers_commit(s);
    }
    unsafe { ffi::fb_fill_round(u.fb, u.clip, Rect::new(c.x + half - 56, y(top + 40, s), 56, 56), 12, s.spec.accent) };
    let mut hx = [0u8; 8];
    theme::format_hex(s.spec.accent, &mut hx);
    ui::label(u, c.x + half - 56, y(top + 100, s), bytes0(&hx), t.text_dim);
    ui::label(u, x2, y(top, s), b"Shape and effects", t.text_dim);
    let mut v = if s.spec.radius < 0 { 8 } else { s.spec.radius };
    if pers_slider(u, x2, y(top + 22, s), half, b"Corner radius", &mut v, 0, 24) { s.spec.radius = v; pers_commit(s); }
    v = if s.spec.title_h != 0 { s.spec.title_h } else { 28 };
    if pers_slider(u, x2, y(top + 66, s), half, b"Title bar", &mut v, 20, 48) { s.spec.title_h = v; pers_commit(s); }
    v = s.spec.shadow;
    if pers_slider(u, x2, y(top + 110, s), half, b"Shadow %", &mut v, 0, 150) { s.spec.shadow = v; pers_commit(s); }
    v = s.spec.glow;
    if pers_slider(u, x2, y(top + 154, s), half, b"Wallpaper glow %", &mut v, 0, 150) { s.spec.glow = v; pers_commit(s); }
    v = s.spec.tint;
    if pers_slider(u, c.x, y(top + 154, s), half - 80, b"Surface tint %", &mut v, 0, 100) { s.spec.tint = v; pers_commit(s); }
    v = if s.spec.border != 0 { s.spec.border } else { 1 };
    if pers_slider(u, c.x, y(top + 198, s), half - 80, b"Border", &mut v, 1, 4) { s.spec.border = v; pers_commit(s); }
    let mut g = s.spec.guard;
    if ui::toggle(u, Rect::new(x2, y(top + 204, s), half, 26), b"Keep text readable (contrast guard)", &mut g) {
        s.spec.guard = g;
        pers_commit(s);
    }
    ui::label(u, c.x, y(top + 252, s), b"Taskbar", t.text_dim);
    let tb = if s.taskbar.is_null() { None } else { Some(unsafe { &mut *s.taskbar }) };
    let mut tc = tb.as_ref().map_or(true, |t| t.centered);
    let mut tl = tb.as_ref().map_or(false, |t| t.labels);
    let mut tt = tb.as_ref().map_or(false, |t| t.top);
    let mut tsz = tb.as_ref().map_or(1, |t| t.size);
    let mut tch = false;
    tch |= ui::toggle(u, Rect::new(c.x, y(top + 274, s), half, 26), b"Center icons", &mut tc);
    tch |= ui::toggle(u, Rect::new(x2, y(top + 274, s), half, 26), b"Show window titles", &mut tl);
    tch |= ui::toggle(u, Rect::new(c.x, y(top + 306, s), half, 26), b"Taskbar at the top", &mut tt);
    const SIZES: [&[u8]; 3] = [b"Small\0", b"Medium\0", b"Large\0"];
    for i in 0..3 {
        if ui::radio(u, Rect::new(x2 + i * (half / 3), y(top + 306, s), half / 3 - 4, 26), SIZES[i as usize], &mut tsz, i) {
            crate::backend::cfg_set_int(be(s), b"ui.taskbar.size\0", i);
            tch = true;
        }
    }
    if tch {
        let b = be(s);
        crate::backend::cfg_set_bool(b, b"ui.taskbar.center\0", tc);
        crate::backend::cfg_set_bool(b, b"ui.taskbar.labels\0", tl);
        crate::backend::cfg_set_bool(b, b"ui.taskbar.top\0", tt);
        if let Some(tb) = tb {
            crate::taskbar::configure(tb, tc, tl, tt, tsz);
        }
        s.changes += 1;
    }
    ui::label(u, c.x, y(top + 346, s), b"Panels and icons", t.text_dim);
    v = if s.spec.opacity != 0 { s.spec.opacity } else { 92 };
    if pers_slider(u, c.x, y(top + 366, s), half - 80, b"Panel opacity %", &mut v, 40, 100) { s.spec.opacity = v; pers_commit(s); }
    const SHAPES: [&[u8]; 3] = [b"Rounded\0", b"Circle\0", b"Square\0"];
    const WALLS: [&[u8]; 4] = [b"Glow\0", b"Gradient\0", b"Aurora\0", b"Solid\0"];
    let mut shp = s.spec.icons;
    let mut wal = s.spec.wallpaper;
    for i in 0..3 {
        if ui::radio(u, Rect::new(x2 + i * (half / 3), y(top + 372, s), half / 3 - 4, 26), SHAPES[i as usize], &mut shp, i) { s.spec.icons = i; pers_commit(s); }
    }
    ui::label(u, c.x, y(top + 414, s), b"Wallpaper", t.text_dim);
    for i in 0..4 {
        if ui::radio(u, Rect::new(c.x + i * (c.w / 4), y(top + 436, s), c.w / 4 - 4, 26), WALLS[i as usize], &mut wal, i) { s.spec.wallpaper = i; pers_commit(s); }
    }
    let cy = top + 482;
    ui::label(u, c.x, y(cy, s), b"Colors (click to select, type a hex value to override)", t.text_dim);
    let res = theme::resolve(&s.spec);
    let colors = [res.desktop, res.surface, res.surface_hi, res.border, res.text, res.text_dim, res.accent, res.accent_text, res.danger, res.title_inactive];
    let mut sw = (c.w - 9 * 6) / 10;
    if sw > 40 { sw = 40; }
    for i in 0..10 {
        let r = Rect::new(c.x + i * (sw + 6), y(cy + 22, s), sw, 30);
        unsafe {
            ffi::fb_fill_round(u.fb, u.clip, r, 6, colors[i as usize]);
            ffi::fb_stroke_round(u.fb, u.clip, r, 6, if s.p_token == i { 2 } else { 1 }, if s.p_token == i { t.text } else { t.border });
        }
        if s.spec.ovr_mask & (1u16 << i) != 0 {
            unsafe { ffi::fb_fill_circle(u.fb, u.clip, r.x + r.w - 6, r.y + 6, 3, t.accent) };
        }
        if u.pressed && ui::hovered(u, r) {
            s.p_token = i;
            theme::format_hex(colors[i as usize], &mut hx);
            put(&mut s.p_hex, &hx);
            s.p_hex_caret = bytes0(&s.p_hex).len() as i32;
        }
    }
    let tok_name = theme::TOKEN_NAMES[s.p_token as usize];
    ui::label(u, c.x, y(cy + 62, s), bytes0(tok_name), t.text);
    if ui::textfield(u, Rect::new(c.x + 130, y(cy + 58, s), 110, 26), &mut s.p_hex, &mut s.p_hex_caret, b"#RRGGBB") {
        if let Some(col) = theme::parse_hex(bytes0(&s.p_hex)) {
            s.spec.ovr[s.p_token as usize] = col;
            s.spec.ovr_mask |= 1u16 << s.p_token;
            pers_commit(s);
        }
    }
    if ui::button(u, Rect::new(c.x + 252, y(cy + 58, s), 120, 26), b"Reset colors") {
        s.spec.ovr_mask = 0;
        pers_commit(s);
    }
    let io = cy + 104;
    ui::label(u, c.x, y(io, s), b"Share or restore this theme as text (press Enter in the box to import)", t.text_dim);
    if ui::button(u, Rect::new(c.x, y(io + 22, s), 120, 26), b"Export") {
        let n = theme::export(&s.spec, s.p_io.as_mut_ptr(), s.p_io.len());
        s.p_io_caret = n.min(s.p_io.len() - 1) as i32;
    }
    if ui::textfield(u, Rect::new(c.x + 130, y(io + 22, s), c.w - 130, 26), &mut s.p_io, &mut s.p_io_caret, b"base=nord;accent=#88C0D0;radius=10") {
        if theme::import(&mut s.spec, bytes0(&s.p_io)) == 0 {
            pers_commit(s);
        } else {
            put(&mut s.p_io, b"Some entries were not understood; valid ones were applied.");
            pers_commit(s);
        }
    }
    ui::vscroll(u, b"pers_scroll", c, io + 22 + 26 + 16, &mut s.p_scroll);
    ui::clip_pop(u, saved);
}

fn page_accounts(s: &mut ShellSettings, u: &mut UiCtx, c: Rect) {
    let t = unsafe { *u.theme };
    ui::heading(u, c.x, c.y, b"Accounts");
    let mut list = [0u8; 128];
    if !be(s).cfg_get(b"login.accounts\0", &mut list) {
        put(&mut list, b"owner");
    }
    let mut names = [[0u8; 32]; 8];
    let mut n = 0;
    let mut start = 0;
    for i in 0..list.len() {
        if list[i] == b',' || list[i] == 0 {
            if i > start && n < 8 {
                let len = (i - start).min(31);
                names[n][..len].copy_from_slice(&list[start..start + len]);
                n += 1;
            }
            if list[i] == 0 { break; }
            start = i + 1;
        }
    }
    ui::label(u, c.x, c.y + 48, b"Local accounts only. There is no cloud account and no PIN sync.", t.text_dim);
    ui::list(u, Rect::new(c.x, c.y + 76, 280, 130), b"accounts", n as i32, 26, Some(row_text), names.as_mut_ptr() as *mut c_void, &mut s.account_sel, &mut s.account_scroll);
    cfg_toggle(s, u, Rect::new(c.x, c.y + 224, 420, 28), b"Sign in automatically", b"login.auto\0", false);
}

pub fn frame(s: &mut ShellSettings, u: &mut UiCtx, w: i32, h: i32) {
    let t = unsafe { *u.theme };
    ui::clear(u, t.surface);
    let sbw = 190;
    unsafe {
        ffi::fb_fill(u.fb, u.clip, Rect::new(0, 0, sbw, h), mix(t.surface, t.desktop, 160));
        ffi::fb_vline(u.fb, u.clip, sbw, 0, h, t.border);
    }
    for i in 0..PG_COUNT {
        let r = Rect::new(8, 12 + i as i32 * 38, sbw - 16, 32);
        let sel = s.page == i as i32;
        if sel {
            unsafe { ffi::fb_fill_round(u.fb, u.clip, r, 6, mix(t.surface_hi, t.accent, 70)) };
        }
        if ui::hovered(u, r) && u.pressed {
            s.page = i as i32;
        } else if ui::hovered(u, r) && !sel {
            unsafe { ffi::fb_fill_round(u.fb, u.clip, r, 6, mix(t.surface, t.surface_hi, 140)) };
        }
        text(u, r.x + 12, r.y + 8, PAGE_NAMES[i], if sel { t.text } else { t.text_dim });
    }
    let c = Rect::new(sbw + 28, 24, w - sbw - 56, h - 48);
    let b = be(s);
    s.nconns = b.net_query(s.conns.as_mut_ptr(), 32).max(0).min(32);
    s.napps = b.perm_apps(s.apps.as_mut_ptr() as *mut u8, MEM_MAX_APPS as i32).max(0).min(MEM_MAX_APPS as i32);
    match s.page {
        PG_PRIVACY => page_privacy(s, u, c),
        PG_NETWORK => page_network(s, u, c),
        PG_PERMISSIONS => page_permissions(s, u, c),
        PG_UPDATES => page_updates(s, u, c),
        PG_APPEARANCE => page_appearance(s, u, c),
        PG_ACCESS => page_access(s, u, c),
        PG_ACCOUNTS => page_accounts(s, u, c),
        PG_PERSONALIZE => page_personalize(s, u, c),
        PG_LOOKS => crate::looks_ui::page_looks(s, u, c),
        PG_BEHAVIOR => crate::looks_ui::page_behavior(s, u, c),
        _ => {}
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn settings_init(s: *mut ShellSettings, be: *const ShellBackend, c: *mut crate::compositor::Compositor, tb: *mut crate::taskbar::ShellTaskbar) {
    if let Some(s) = unsafe { s.as_mut() } {
        init(s, be, c, tb);
    }
}
#[no_mangle]
pub unsafe extern "C" fn settings_apply(s: *mut ShellSettings) {
    if let Some(s) = unsafe { s.as_mut() } {
        apply(s);
    }
}
#[no_mangle]
pub unsafe extern "C" fn settings_quick(s: *mut ShellSettings, action: i32, value: u32) {
    if let Some(s) = unsafe { s.as_mut() } {
        quick(s, action, value);
    }
}
#[no_mangle]
pub unsafe extern "C" fn settings_frame(s: *mut ShellSettings, u: *mut UiCtx, w: i32, h: i32) {
    if let (Some(s), Some(u)) = (unsafe { s.as_mut() }, unsafe { u.as_mut() }) {
        frame(s, u, w, h);
    }
}
#[no_mangle]
pub unsafe extern "C" fn settings_net_state(s: *const ShellSettings) -> i32 {
    unsafe { s.as_ref() }.map_or(NET_IDLE, net_state)
}
#[no_mangle]
pub unsafe extern "C" fn settings_page_by_id(id: *const c_char, out: *mut i32) -> bool {
    match page_by_id(unsafe { ffi::c_bytes(id) }) {
        Some(p) => {
            if !out.is_null() {
                unsafe { *out = p };
            }
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ShellSettings>(), 4872);
        assert_eq!(offset_of!(ShellSettings, be), 0);
        assert_eq!(offset_of!(ShellSettings, comp), 8);
        assert_eq!(offset_of!(ShellSettings, taskbar), 16);
        assert_eq!(offset_of!(ShellSettings, page), 24);
        assert_eq!(offset_of!(ShellSettings, conn_sel), 28);
        assert_eq!(offset_of!(ShellSettings, app_sel), 36);
        assert_eq!(offset_of!(ShellSettings, account_sel), 44);
        assert_eq!(offset_of!(ShellSettings, mirror), 52);
        assert_eq!(offset_of!(ShellSettings, theme), 152);
        assert_eq!(offset_of!(ShellSettings, conns), 164);
        assert_eq!(offset_of!(ShellSettings, apps), 3240);
        assert_eq!(offset_of!(ShellSettings, spec), 4268);
        assert_eq!(offset_of!(ShellSettings, p_scroll), 4360);
        assert_eq!(offset_of!(ShellSettings, p_hex), 4368);
        assert_eq!(offset_of!(ShellSettings, p_io), 4384);
        assert_eq!(offset_of!(ShellSettings, changes), 4868);
    }

    #[test]
    fn resolves_launcher_page_ids() {
        assert_eq!(page_by_id(b"settings:privacy"), Some(PG_PRIVACY));
        assert_eq!(page_by_id(b"settings:personalize"), Some(PG_PERSONALIZE));
        assert_eq!(page_by_id(b"settings:nope"), None);
        assert_eq!(page_by_id(b"privacy"), None);
    }

    unsafe extern "C" fn no_cfg(_u: *mut c_void, _k: *const c_char, _o: *mut c_char, _c: usize) -> bool {
        false
    }

    #[test]
    fn defaults_are_local_only() {
        let mut be = ShellBackend { user: core::ptr::null_mut(), funcs: [core::ptr::null_mut(); 24] };
        be.funcs[crate::backend::F_CFG_GET] = no_cfg as *mut c_void;
        let s = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellSettings>() }));
        init(s, &be, core::ptr::null_mut(), core::ptr::null_mut());
        assert_eq!(s.page, PG_PRIVACY);
        assert_eq!(s.spec.base, theme::TB_DARK);
        assert!(s.spec.guard);
        assert_eq!(s.changes, 0);
    }
}

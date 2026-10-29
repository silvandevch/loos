//! Guided local-only installer wizard (C `installer.c`); disk work goes through `shell_inst_backend`.
#![allow(missing_docs)]

use crate::draw::Rect;
use crate::ffi::{self, Fb};
use crate::ui::{self, UiCtx};
use crate::util;
use core::ffi::{c_char, c_void};

pub const INST_MAX_DISKS: usize = 8;
pub const IP_WELCOME: i32 = 0;
pub const IP_DISK: i32 = 1;
pub const IP_ACCOUNT: i32 = 2;
pub const IP_PRIVACY: i32 = 3;
pub const IP_REVIEW: i32 = 4;
pub const IP_INSTALL: i32 = 5;
pub const IP_DONE: i32 = 6;
pub const IP_ERROR: i32 = 7;

const COL_DANGER: u32 = 0xFFEF_4444;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InstDisk {
    pub name: [u8; 32],
    pub model: [u8; 48],
    pub bytes: u64,
    pub boot_media: bool,
    pub too_small: bool,
}
impl InstDisk {
    pub const EMPTY: Self = Self { name: [0; 32], model: [0; 48], bytes: 0, boot_media: false, too_small: false };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InstConfig {
    pub disk: i32,
    pub display: [u8; 48],
    pub username: [u8; 32],
    pub hostname: [u8; 32],
    pub password: [u8; 64],
    pub net_blocked: bool,
    pub updates: bool,
}
impl InstConfig {
    pub const EMPTY: Self = Self { disk: 0, display: [0; 48], username: [0; 32], hostname: [0; 32], password: [0; 64], net_blocked: false, updates: false };
}

pub type ListDisks = unsafe extern "C" fn(*mut c_void, *mut InstDisk, i32) -> i32;
pub type Begin = unsafe extern "C" fn(*mut c_void, *const InstConfig, *mut c_char, usize) -> bool;
pub type Step = unsafe extern "C" fn(*mut c_void, *mut i32, *mut c_char, usize, *mut c_char, usize) -> i32;
pub type Reboot = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
pub struct ShellInstBackend {
    pub list_disks: Option<ListDisks>,
    pub begin: Option<Begin>,
    pub step: Option<Step>,
    pub reboot: Option<Reboot>,
    pub user: *mut c_void,
}

#[repr(C)]
pub struct ShellInstaller {
    pub be: *const ShellInstBackend,
    pub page: i32,
    pub disks: [InstDisk; INST_MAX_DISKS],
    pub ndisks: i32,
    pub sel_disk: i32,
    pub disk_scroll: i32,
    pub display: [u8; 48],
    pub user: [u8; 32],
    pub host: [u8; 32],
    pub pw: [u8; 64],
    pub pw2: [u8; 64],
    pub c_display: i32,
    pub c_user: i32,
    pub c_host: i32,
    pub c_pw: i32,
    pub c_pw2: i32,
    pub user_edited: bool,
    pub host_edited: bool,
    pub net_blocked: bool,
    pub updates: bool,
    pub percent: i32,
    pub msg: [u8; 96],
    pub err: [u8; 160],
    pub live_requested: bool,
    pub frames: u32,
    pub bg_key: u32,
}

fn bytes0(a: &[u8]) -> &[u8] {
    let n = a.iter().position(|&b| b == 0).unwrap_or(a.len());
    &a[..n]
}
fn eq(a: &[u8], b: &[u8]) -> bool { bytes0(a) == bytes0(b) }
fn put(dst: &mut [u8], src: &[u8]) {
    dst.fill(0);
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
}

pub fn valid_username(s: &[u8]) -> bool {
    let s = bytes0(s);
    if s.is_empty() || s.len() > 31 || !s[0].is_ascii_lowercase() {
        return false;
    }
    if !s.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-') {
        return false;
    }
    s != b"root" && s != b"system" && s != b"admin"
}

pub fn valid_hostname(s: &[u8]) -> bool {
    let s = bytes0(s);
    if s.is_empty() || s.len() > 31 || s[0] == b'-' || s[s.len() - 1] == b'-' {
        return false;
    }
    s.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'-')
}

pub fn password_problem(pw: &[u8], pw2: &[u8]) -> Option<&'static [u8]> {
    let pw = bytes0(pw);
    if pw.len() < 8 {
        return Some(b"Use at least 8 characters.");
    }
    if pw != bytes0(pw2) {
        return Some(b"The passwords do not match.");
    }
    let (mut digit, mut alpha, mut other) = (false, false, false);
    for &c in pw {
        if c.is_ascii_digit() { digit = true; }
        else if (c | 32).is_ascii_lowercase() { alpha = true; }
        else { other = true; }
    }
    if (digit as u8 + alpha as u8 + other as u8) < 2 {
        return Some(b"Mix letters with digits or symbols.");
    }
    None
}

pub fn suggest_username(display: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut n = 0;
    for &mut_c in bytes0(display) {
        let mut c = mut_c;
        if c.is_ascii_uppercase() { c += 32; }
        if c.is_ascii_lowercase() || (n > 0 && (c.is_ascii_digit() || c == b'-' || c == b'_')) {
            if n + 1 < out.len() && n < 31 {
                out[n] = c;
                n += 1;
            }
        } else if c == b' ' && n > 0 {
            break;
        }
    }
    out
}

pub fn init(in_: &mut ShellInstaller, be: *const ShellInstBackend) {
    in_.be = be;
    in_.page = IP_WELCOME;
    in_.disks = [InstDisk::EMPTY; INST_MAX_DISKS];
    in_.ndisks = 0;
    in_.sel_disk = -1;
    in_.disk_scroll = 0;
    in_.display = [0; 48];
    in_.user = [0; 32];
    in_.host = [0; 32];
    in_.pw = [0; 64];
    in_.pw2 = [0; 64];
    in_.c_display = 0;
    in_.c_user = 0;
    in_.c_host = 0;
    in_.c_pw = 0;
    in_.c_pw2 = 0;
    in_.user_edited = false;
    in_.host_edited = false;
    in_.net_blocked = true;
    in_.updates = false;
    in_.percent = 0;
    in_.msg = [0; 96];
    in_.err = [0; 160];
    in_.live_requested = false;
    in_.frames = 0;
    in_.bg_key = 0;
    put(&mut in_.host, b"loos-pc");
    in_.c_host = 8;
}

fn refresh_disks(in_: &mut ShellInstaller) {
    let be = unsafe { &*in_.be };
    in_.ndisks = match be.list_disks {
        Some(f) => unsafe { f(be.user, in_.disks.as_mut_ptr(), INST_MAX_DISKS as i32) },
        None => 0,
    };
    if in_.ndisks < 0 {
        in_.ndisks = 0;
    }
    if in_.ndisks > INST_MAX_DISKS as i32 {
        in_.ndisks = INST_MAX_DISKS as i32;
    }
    if in_.sel_disk >= in_.ndisks || (in_.sel_disk >= 0 && (in_.disks[in_.sel_disk as usize].boot_media || in_.disks[in_.sel_disk as usize].too_small)) {
        in_.sel_disk = -1;
    }
    if in_.sel_disk < 0 {
        for i in 0..in_.ndisks as usize {
            if !in_.disks[i].boot_media && !in_.disks[i].too_small {
                in_.sel_disk = i as i32;
                break;
            }
        }
    }
}

fn account_ok(in_: &ShellInstaller) -> bool {
    in_.display[0] != 0 && valid_username(&in_.user) && valid_hostname(&in_.host) && password_problem(&in_.pw, &in_.pw2).is_none()
}

// ---------------------------------------------------------------- drawing

fn t(u: &UiCtx) -> crate::draw::Theme {
    // SAFETY: ui_begin contract.
    unsafe { *u.theme }
}
fn text(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: &[u8], col: u32) {
    let mut b = [0u8; 256];
    let n = bytes0(s).len().min(255);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    unsafe { ffi::fb_text(fb, clip, x, y, b.as_ptr() as *const c_char, col) };
}
fn text_ex(fb: *mut Fb, clip: Rect, x: i32, y: i32, s: &[u8], col: u32, size: i32, semi: bool) {
    let mut b = [0u8; 256];
    let n = bytes0(s).len().min(255);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    let f = crate::font::ShellFont {
        face: if semi { core::ptr::addr_of!(crate::font_data::loos_font_semibold) } else { core::ptr::addr_of!(crate::font_data::loos_font_regular) },
        size,
    };
    unsafe { ffi::fb_text_ex(fb, clip, x, y, b.as_ptr() as *const c_char, col, f) };
}
fn width(s: &[u8]) -> i32 {
    let mut b = [0u8; 256];
    let n = bytes0(s).len().min(255);
    b[..n].copy_from_slice(&bytes0(s)[..n]);
    unsafe { ffi::shell_text_width(b.as_ptr() as *const c_char) }
}

unsafe extern "C" fn disk_row(u: *mut UiCtx, row: Rect, i: i32, selected: bool, user: *mut c_void) {
    let (Some(u), Some(in_)) = (unsafe { u.as_mut() }, unsafe { (user as *mut ShellInstaller).as_mut() }) else { return };
    let d = in_.disks[i as usize];
    let th = t(u);
    let usable = !d.boot_media && !d.too_small;
    let fg = if usable { if selected { th.accent_text } else { th.text } } else { th.text_dim };
    let mut size = [0u8; 24];
    util::size(d.bytes, &mut size);
    ui::label(u, row.x + 14, row.y + 6, bytes0(&d.name), fg);
    ui::label(u, row.x + 14, row.y + 26, if d.model[0] != 0 { bytes0(&d.model) } else { b"Unknown model" }, if selected && usable { th.accent_text } else { th.text_dim });
    ui::label_right(u, row.x + row.w - 14, row.y + 6, bytes0(&size), fg);
    let note: &[u8] = if d.boot_media { b"Installation media" } else if d.too_small { b"Too small (needs 2 GB)" } else { b"" };
    if !note.is_empty() {
        ui::label_right(u, row.x + row.w - 14, row.y + 26, note, if d.boot_media { th.text_dim } else { COL_DANGER });
    }
}

fn button_disabled(u: &mut UiCtx, r: Rect, label: &[u8]) {
    unsafe { ffi::fb_fill_round(u.fb, u.clip, r, 6, 0x28FF_FFFF) };
    text(u.fb, u.clip, r.x + (r.w - width(label)) / 2, r.y + (r.h - 16) / 2, label, t(u).text_dim);
}

fn field(u: &mut UiCtx, x: i32, y: i32, w: i32, label: &[u8], buf: &mut [u8], caret: &mut i32, ph: &[u8], password: bool, problem: Option<&[u8]>) {
    ui::label(u, x, y, label, t(u).text_dim);
    let r = Rect::new(x, y + 20, w, 30);
    if password { ui::textfield_password(u, r, buf, caret, ph); } else { ui::textfield(u, r, buf, caret, ph); }
    if let Some(p) = problem {
        ui::label(u, x + 4, y + 54, p, COL_DANGER);
    }
}

const STEP_NAMES: [&[u8]; 5] = [b"Welcome\0", b"Disk\0", b"Account\0", b"Privacy\0", b"Review\0"];

pub fn frame(in_: &mut ShellInstaller, u: &mut UiCtx, w: i32, h: i32) {
    let th = t(u);
    in_.frames = in_.frames.wrapping_add(1);
    let cw = if w - 48 < 860 { w - 48 } else { 860 };
    let ch = if h - 48 < 580 { h - 48 } else { 580 };
    let card = Rect::new((w - cw) / 2, (h - ch) / 2, cw, ch);
    let scale = if !u.fb.is_null() && unsafe { (*u.fb).scale } != 0 { unsafe { (*u.fb).scale } } else { 100 };
    let key = (w as u32).wrapping_mul(7919) ^ (h as u32).wrapping_mul(104729) ^ (scale as u32).wrapping_mul(31) ^ 0x5eed_0000;
    if in_.bg_key != key {
        ui::backdrop(u, w, h);
        in_.bg_key = key;
    } else {
        let saved = ui::clip_push(u, Rect::new(card.x - 70, card.y - 70, card.w + 140, card.h + 170));
        ui::backdrop(u, w, h);
        ui::clip_pop(u, saved);
    }
    unsafe {
        ffi::fb_shadow(u.fb, u.clip, card, 14, 36, 10, 0x8000_0000);
        ffi::fb_fill_round(u.fb, u.clip, card, 14, 0xFF11_1826);
        ffi::fb_stroke_round(u.fb, u.clip, card, 14, 1, 0xFF2A_354A);
    }
    let side_w = 200;
    let wizard = in_.page <= IP_REVIEW;
    let body = Rect::new(card.x + if wizard { side_w } else { 0 }, card.y, card.w - if wizard { side_w } else { 0 }, card.h);
    if wizard {
        let side = Rect::new(card.x, card.y, side_w, card.h);
        unsafe {
            ffi::fb_fill_round(u.fb, u.clip, side, 14, 0xFF0E_1420);
            ffi::fb_fill(u.fb, u.clip, Rect::new(side.x + side.w - 20, side.y, 20, side.h), 0xFF0E_1420);
            ffi::fb_fill(u.fb, u.clip, Rect::new(side.x + side.w - 1, side.y + 14, 1, side.h - 28), 0xFF1F_2940);
        }
        text_ex(u.fb, u.clip, side.x + 24, side.y + 26, b"LoOS", 0xFFF3_F4F6, 22, true);
        let lw = crate::text::width(&crate::font_data::semibold(), 22, b"LoOS");
        unsafe { ffi::fb_fill_circle(u.fb, u.clip, side.x + 24 + lw + 7, side.y + 46, 3, 0xFF3B_82F6) };
        text(u.fb, u.clip, side.x + 24, side.y + 56, b"Setup", th.text_dim);
        for i in 0..5 {
            let y = side.y + 110 + i * 40;
            let cur = in_.page == i;
            let done = in_.page > i;
            unsafe { ffi::fb_fill_circle(u.fb, u.clip, side.x + 34, y + 8, 11, if cur { th.accent } else if done { 0xFF1E_3A8A } else { 0xFF1F_2940 }) };
            let num = [b'1' + i as u8, 0];
            let label: &[u8] = if done { b"\xE2\x9C\x93" } else { &num };
            text(u.fb, u.clip, side.x + 34 - width(label) / 2, y, label, if cur || done { 0xFFFF_FFFF } else { th.text_dim });
            text_ex(u.fb, u.clip, side.x + 58, y, STEP_NAMES[i as usize], if cur { 0xFFF3_F4F6 } else if done { 0xFFC8_D4F0 } else { th.text_dim }, if cur { 14 } else { 14 }, cur);
        }
    }
    let bx = body.x + 32;
    let by = body.y + 28;
    let bw = body.w - 64;
    let foot_y = card.y + card.h - 64;
    let mut next = false;
    let mut back = false;
    match in_.page {
        IP_WELCOME => {
            ui::heading(u, bx, by, b"Welcome to LoOS");
            ui::label(u, bx, by + 52, b"A private, local-only operating system.", th.text);
            const POINTS: [&[u8]; 4] = [
                b"No telemetry, ever. Nothing is collected, nothing phones home.",
                b"No cloud account. Your user lives on this computer only.",
                b"The network is off until you allow it, and the tray LED always tells the truth.",
                b"Windows and Linux programs run natively; your files stay yours.",
            ];
            for (i, p) in POINTS.iter().enumerate() {
                unsafe { ffi::fb_fill_circle(u.fb, u.clip, bx + 5, by + 106 + i as i32 * 32 + 8, 4, 0xFF3B_82F6) };
                ui::label(u, bx + 20, by + 106 + i as i32 * 32, p, th.text_dim);
            }
            ui::label(u, bx, by + 260, b"This installer erases the disk you choose and sets LoOS up on it.", th.text_dim);
            if ui::button_accent(u, Rect::new(bx, foot_y, 170, 38), b"Install LoOS") {
                refresh_disks(in_);
                in_.page = IP_DISK;
            }
            if ui::button(u, Rect::new(bx + 186, foot_y, 230, 38), b"Try LoOS without installing") {
                in_.live_requested = true;
            }
        }
        IP_DISK => {
            ui::heading(u, bx, by, b"Choose a disk");
            ui::label(u, bx, by + 50, b"LoOS will be installed on the selected disk.", th.text_dim);
            let rows = if in_.ndisks > 4 { 4 } else if in_.ndisks > 0 { in_.ndisks } else { 1 };
            let list = Rect::new(bx, by + 84, bw, 56 * rows + 4);
            unsafe { ffi::fb_fill_round(u.fb, u.clip, list, 8, 0xFF0E_1420) };
            if in_.ndisks == 0 {
                ui::label(u, list.x + 16, list.y + 18, b"No disks found. Attach a disk (IDE/SATA in legacy mode) and restart.", COL_DANGER);
            } else {
                let mut pick = in_.sel_disk;
                ui::list(u, list, b"installer-disks", in_.ndisks, 56, Some(disk_row), in_ as *mut ShellInstaller as *mut c_void, &mut pick, &mut in_.disk_scroll);
                if pick != in_.sel_disk && pick >= 0 && pick < in_.ndisks && !in_.disks[pick as usize].boot_media && !in_.disks[pick as usize].too_small {
                    in_.sel_disk = pick;
                }
            }
            ui::label(u, bx, list.y + list.h + 16, b"Everything on the selected disk will be erased.", COL_DANGER);
            if ui::button(u, Rect::new(bx, foot_y, 100, 38), b"Back") { back = true; }
            let ok = in_.sel_disk >= 0;
            if ok {
                if ui::button_accent(u, Rect::new(bx + bw - 130, foot_y, 130, 38), b"Continue") { next = true; }
            } else {
                button_disabled(u, Rect::new(bx + bw - 130, foot_y, 130, 38), b"Continue");
            }
        }
        IP_ACCOUNT => {
            ui::heading(u, bx, by, b"Create your account");
            let half = (bw - 20) / 2;
            let p_user = if in_.user[0] != 0 && !valid_username(&in_.user) { Some(&b"Lowercase letters, digits, - and _."[..]) } else { None };
            let p_host = if in_.host[0] != 0 && !valid_hostname(&in_.host) { Some(&b"Letters, digits and hyphens only."[..]) } else { None };
            let p_pw = if in_.pw[0] != 0 { password_problem(&in_.pw, &in_.pw2) } else { None };
            let y0 = by + 48;
            let before = in_.display;
            field(u, bx, y0, half, b"Your name", &mut in_.display, &mut in_.c_display, b"Ada Lovelace", false, None);
            if !eq(&before, &in_.display) && !in_.user_edited {
                in_.user = suggest_username(&in_.display);
                in_.c_user = bytes0(&in_.user).len() as i32;
            }
            let ub = in_.user;
            field(u, bx + half + 20, y0, half, b"Username", &mut in_.user, &mut in_.c_user, b"ada", false, p_user);
            if !eq(&ub, &in_.user) { in_.user_edited = true; }
            let hb = in_.host;
            field(u, bx, y0 + 84, half, b"Computer name", &mut in_.host, &mut in_.c_host, b"loos-pc", false, p_host);
            if !eq(&hb, &in_.host) { in_.host_edited = true; }
            field(u, bx, y0 + 168, half, b"Password", &mut in_.pw, &mut in_.c_pw, b"At least 8 characters", true, None);
            let show_pw = p_pw.is_some() && in_.pw2[0] != 0;
            field(u, bx + half + 20, y0 + 168, half, b"Confirm password", &mut in_.pw2, &mut in_.c_pw2, b"Repeat the password", true, if show_pw { p_pw } else { None });
            if p_pw.is_some() && in_.pw2[0] == 0 {
                ui::label(u, bx + 4, y0 + 222, p_pw.unwrap(), th.text_dim);
            }
            ui::label(u, bx, y0 + 252, b"The password never leaves this computer. It is stored only as a salted PBKDF2 hash.", th.text_dim);
            if ui::button(u, Rect::new(bx, foot_y, 100, 38), b"Back") { back = true; }
            if account_ok(in_) {
                if ui::button_accent(u, Rect::new(bx + bw - 130, foot_y, 130, 38), b"Continue") { next = true; }
            } else {
                button_disabled(u, Rect::new(bx + bw - 130, foot_y, 130, 38), b"Continue");
            }
        }
        IP_PRIVACY => {
            ui::heading(u, bx, by, b"Privacy");
            ui::label(u, bx, by + 50, b"LoOS collects nothing. These two choices are yours:", th.text_dim);
            ui::toggle(u, Rect::new(bx, by + 100, bw, 28), b"Keep the network blocked until I allow it", &mut in_.net_blocked);
            ui::label(u, bx + 52, by + 130, b"A hard switch in the taskbar turns it on whenever you want.", th.text_dim);
            ui::toggle(u, Rect::new(bx, by + 176, bw, 28), b"Check for updates automatically", &mut in_.updates);
            ui::label(u, bx + 52, by + 206, b"Off by default. Updates come from a mirror you choose, never from us.", th.text_dim);
            ui::separator(u, bx, by + 252, bw);
            ui::label(u, bx, by + 268, b"Telemetry, crash uploads and advertising identifiers do not exist in LoOS.", th.text_dim);
            if ui::button(u, Rect::new(bx, foot_y, 100, 38), b"Back") { back = true; }
            if ui::button_accent(u, Rect::new(bx + bw - 130, foot_y, 130, 38), b"Continue") { next = true; }
        }
        IP_REVIEW => {
            ui::heading(u, bx, by, b"Ready to install");
            let d = if in_.sel_disk >= 0 { Some(in_.disks[in_.sel_disk as usize]) } else { None };
            let mut size = [0u8; 24];
            if let Some(d) = d { util::size(d.bytes, &mut size); }
            let boxr = Rect::new(bx, by + 54, bw, 188);
            unsafe { ffi::fb_fill_round(u.fb, u.clip, boxr, 8, 0xFF0E_1420) };
            let mut rows = [[0u8; 120]; 5];
            let mut n = 0;
            if let Some(d) = d {
                for &c in bytes0(&d.name) { rows[0][n] = c; n += 1; }
            } else { rows[0][0] = b'-'; n = 1; }
            for &c in b"  " { rows[0][n] = c; n += 1; }
            for &c in bytes0(&size) { rows[0][n] = c; n += 1; }
            let mut n = 0;
            for &c in bytes0(&in_.display) { rows[1][n] = c; n += 1; }
            for &c in b" (" { rows[1][n] = c; n += 1; }
            for &c in bytes0(&in_.user) { rows[1][n] = c; n += 1; }
            rows[1][n] = b')';
            rows[2][..32].copy_from_slice(&in_.host);
            let s3: &[u8] = if in_.net_blocked { b"Blocked until you allow it" } else { b"Allowed" };
            rows[3][..s3.len()].copy_from_slice(s3);
            let s4: &[u8] = if in_.updates { b"Automatic checks on" } else { b"Off (opt-in)" };
            rows[4][..s4.len()].copy_from_slice(s4);
            const KEYS: [&[u8]; 5] = [b"Install on\0", b"Account\0", b"Computer name\0", b"Network\0", b"Updates\0"];
            for i in 0..5 {
                ui::label(u, boxr.x + 18, boxr.y + 16 + i as i32 * 32, KEYS[i], th.text_dim);
                ui::label(u, boxr.x + 170, boxr.y + 16 + i as i32 * 32, bytes0(&rows[i]), th.text);
            }
            let mut line = [0u8; 160];
            let mut ln = 0;
            for &c in b"All data on " { line[ln] = c; ln += 1; }
            if let Some(d) = d { for &c in bytes0(&d.name) { line[ln] = c; ln += 1; } }
            else { for &c in b"the disk" { line[ln] = c; ln += 1; } }
            for &c in b" will be permanently erased." { line[ln] = c; ln += 1; }
            ui::label(u, bx, boxr.y + boxr.h + 18, bytes0(&line[..ln]), COL_DANGER);
            if ui::button(u, Rect::new(bx, foot_y, 100, 38), b"Back") { back = true; }
            if ui::button_accent(u, Rect::new(bx + bw - 200, foot_y, 200, 38), b"Erase disk and install") {
                let mut cfg = InstConfig::EMPTY;
                cfg.disk = in_.sel_disk;
                cfg.net_blocked = in_.net_blocked;
                cfg.updates = in_.updates;
                put(&mut cfg.display, &in_.display);
                put(&mut cfg.username, &in_.user);
                put(&mut cfg.hostname, &in_.host);
                put(&mut cfg.password, &in_.pw);
                in_.err[0] = 0;
                in_.percent = 0;
                put(&mut in_.msg, b"Preparing");
                let be = unsafe { &*in_.be };
                let ok = match be.begin {
                    // SAFETY: backend contract.
                    Some(f) => unsafe { f(be.user, &cfg, in_.err.as_mut_ptr() as *mut c_char, in_.err.len()) },
                    None => false,
                };
                if ok { in_.page = IP_INSTALL; }
                else {
                    if in_.err[0] == 0 { put(&mut in_.err, b"The installer could not start."); }
                    in_.page = IP_ERROR;
                }
                // wipe the transient config copy
                cfg = InstConfig::EMPTY;
                let _ = cfg;
            }
        }
        IP_INSTALL => {
            ui::heading(u, bx, by, b"Installing LoOS");
            ui::label(u, bx, by + 50, b"Please keep the computer powered on.", th.text_dim);
            ui::progress(u, Rect::new(bx, by + 120, bw, 14), in_.percent);
            let mut pct = [0u8; 16];
            let mut pn = 0;
            let mut v = in_.percent.max(0) as u32;
            let mut tmp = [0u8; 12];
            let mut d = 0;
            if v == 0 { tmp[0] = b'0'; d = 1; }
            while v != 0 { tmp[d] = b'0' + (v % 10) as u8; d += 1; v /= 10; }
            for i in (0..d).rev() { pct[pn] = tmp[i]; pn += 1; }
            pct[pn] = b'%'; pn += 1;
            ui::label_right(u, bx + bw, by + 144, bytes0(&pct[..pn]), th.text);
            ui::label(u, bx, by + 144, bytes0(&in_.msg), th.text_dim);
            let be = unsafe { &*in_.be };
            let rc = match be.step {
                Some(f) => unsafe {
                    f(be.user, &mut in_.percent, in_.msg.as_mut_ptr() as *mut c_char, in_.msg.len(), in_.err.as_mut_ptr() as *mut c_char, in_.err.len())
                },
                None => -1,
            };
            if rc > 0 { in_.percent = 100; in_.page = IP_DONE; }
            else if rc < 0 {
                if in_.err[0] == 0 { put(&mut in_.err, b"The installation failed."); }
                in_.page = IP_ERROR;
            }
        }
        IP_DONE => {
            ui::heading(u, bx, by, b"LoOS is installed");
            ui::label(u, bx, by + 54, b"Remove the installation media, then restart to start LoOS.", th.text);
            let mut line = [0u8; 120];
            let mut n = 0;
            for &c in b"Sign in as " { line[n] = c; n += 1; }
            for &c in bytes0(&in_.user) { line[n] = c; n += 1; }
            for &c in b" with the password you just chose." { line[n] = c; n += 1; }
            ui::label(u, bx, by + 86, bytes0(&line[..n]), th.text_dim);
            if ui::button_accent(u, Rect::new(bx, foot_y, 170, 38), b"Restart now") {
                let be = unsafe { &*in_.be };
                if let Some(f) = be.reboot {
                    // SAFETY: backend contract.
                    unsafe { f(be.user) };
                }
            }
        }
        _ => {
            ui::heading(u, bx, by, b"Installation failed");
            ui::label(u, bx, by + 54, bytes0(&in_.err), COL_DANGER);
            ui::label(u, bx, by + 86, b"Nothing else was changed. You can go back and try again.", th.text_dim);
            if ui::button_accent(u, Rect::new(bx, foot_y, 170, 38), b"Back to review") { in_.page = IP_REVIEW; }
        }
    }
    if next && in_.page < IP_REVIEW {
        in_.page += 1;
    } else if back && in_.page > IP_WELCOME {
        in_.page -= 1;
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn installer_init(in_: *mut ShellInstaller, be: *const ShellInstBackend) {
    if let Some(in_) = unsafe { in_.as_mut() } {
        init(in_, be);
    }
}
#[no_mangle]
pub unsafe extern "C" fn installer_frame(in_: *mut ShellInstaller, u: *mut UiCtx, w: i32, h: i32) {
    if let (Some(in_), Some(u)) = (unsafe { in_.as_mut() }, unsafe { u.as_mut() }) {
        frame(in_, u, w, h);
    }
}
#[no_mangle]
pub unsafe extern "C" fn installer_valid_username(s: *const c_char) -> bool {
    valid_username(unsafe { ffi::c_bytes(s) })
}
#[no_mangle]
pub unsafe extern "C" fn installer_valid_hostname(s: *const c_char) -> bool {
    valid_hostname(unsafe { ffi::c_bytes(s) })
}
#[no_mangle]
pub unsafe extern "C" fn installer_password_problem(pw: *const c_char, pw2: *const c_char) -> *const c_char {
    match password_problem(unsafe { ffi::c_bytes(pw) }, unsafe { ffi::c_bytes(pw2) }) {
        Some(p) => p.as_ptr() as *const c_char,
        None => core::ptr::null(),
    }
}
#[no_mangle]
pub unsafe extern "C" fn installer_suggest_username(display: *const c_char, out: *mut c_char, cap: usize) {
    let u = suggest_username(unsafe { ffi::c_bytes(display) });
    if !out.is_null() && cap > 0 {
        // SAFETY: caller buffer.
        let o = unsafe { core::slice::from_raw_parts_mut(out as *mut u8, cap) };
        put(o, bytes0(&u));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<InstDisk>(), 96);
        assert_eq!(size_of::<InstConfig>(), 184);
        assert_eq!(size_of::<ShellInstBackend>(), 40);
        assert_eq!(size_of::<ShellInstaller>(), 1336);
        assert_eq!(offset_of!(ShellInstaller, page), 8);
        assert_eq!(offset_of!(ShellInstaller, disks), 16);
        assert_eq!(offset_of!(ShellInstaller, ndisks), 784);
        assert_eq!(offset_of!(ShellInstaller, sel_disk), 788);
        assert_eq!(offset_of!(ShellInstaller, display), 796);
        assert_eq!(offset_of!(ShellInstaller, user), 844);
        assert_eq!(offset_of!(ShellInstaller, host), 876);
        assert_eq!(offset_of!(ShellInstaller, pw), 908);
        assert_eq!(offset_of!(ShellInstaller, pw2), 972);
        assert_eq!(offset_of!(ShellInstaller, percent), 1060);
        assert_eq!(offset_of!(ShellInstaller, msg), 1064);
        assert_eq!(offset_of!(ShellInstaller, err), 1160);
        assert_eq!(offset_of!(ShellInstaller, frames), 1324);
        assert_eq!(offset_of!(ShellInstaller, bg_key), 1328);
    }

    #[test]
    fn validation_rules() {
        assert!(valid_username(b"ada"));
        assert!(valid_username(b"ada-1_2"));
        assert!(!valid_username(b"Ada"));
        assert!(!valid_username(b"1ada"));
        assert!(!valid_username(b"root"));
        assert!(valid_hostname(b"loos-pc"));
        assert!(!valid_hostname(b"-pc"));
        assert!(!valid_hostname(b"pc-"));
        assert_eq!(password_problem(b"short", b"short"), Some(&b"Use at least 8 characters."[..]));
        assert_eq!(password_problem(b"longenough1", b"other"), Some(&b"The passwords do not match."[..]));
        assert_eq!(password_problem(b"aaaaaaaa", b"aaaaaaaa"), Some(&b"Mix letters with digits or symbols."[..]));
        assert!(password_problem(b"longenough1", b"longenough1").is_none());
    }

    #[test]
    fn suggests_a_login_name() {
        let u = suggest_username(b"Ada Lovelace");
        assert_eq!(bytes0(&u), b"ada");
        let u = suggest_username(b"  Grace M. Hopper");
        assert_eq!(bytes0(&u), b"grace");
        assert!(valid_username(bytes0(&u)));
    }

    #[test]
    fn init_defaults_private_and_hostname() {
        let in_ = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellInstaller>() }));
        init(in_, core::ptr::null());
        assert_eq!(in_.page, IP_WELCOME);
        assert_eq!(in_.sel_disk, -1);
        assert!(in_.net_blocked);
        assert_eq!(bytes0(&in_.host), b"loos-pc");
        assert_eq!(in_.c_host, 8);
    }
}

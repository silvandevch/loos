//! File manager (C `filemgr.c`): local volumes through the Fs backend, multi-select, copy/cut/paste.
#![allow(missing_docs)]

use crate::backend::{FsEntry, FsVolume, ShellBackend};
use crate::compositor::{KEY_BACKSPACE, KEY_DELETE, KEY_ESC, KEY_F1, MOD_CTRL, MOD_SHIFT};
use crate::draw::{mix, Rect};
use crate::ffi;
use crate::ui::{self, UiCtx};
use crate::util;
use core::ffi::{c_char, c_void};

pub const FM_MAX_ENTRIES: usize = 256;
pub const FM_HISTORY: usize = 32;
pub const FM_PATH: usize = 160;
pub const FM_SORT_NAME: i32 = 0;
pub const FM_SORT_SIZE: i32 = 1;
pub const FM_SORT_DATE: i32 = 2;
pub const FM_MODE_BROWSE: i32 = 0;
pub const FM_MODE_RENAME: i32 = 1;
pub const FM_MODE_NEWDIR: i32 = 2;
pub const FM_MODE_CONFIRM_DELETE: i32 = 3;

const FSA_HIDDEN: u32 = 1;

pub type OpenFn = unsafe extern "C" fn(*const c_char, *mut c_void);

#[repr(C)]
pub struct ShellFilemgr {
    pub be: *const ShellBackend,
    pub cwd: [u8; FM_PATH],
    pub ent: [FsEntry; FM_MAX_ENTRIES],
    pub n: i32,
    pub selected: [u8; FM_MAX_ENTRIES],
    pub cursor: i32,
    pub scroll: i32,
    pub vol_sel: i32,
    pub vol_scroll: i32,
    pub vols: [FsVolume; 4],
    pub nvols: i32,
    pub addr: [u8; FM_PATH],
    pub addr_caret: i32,
    pub filter: [u8; 40],
    pub filter_caret: i32,
    pub edit: [u8; 64],
    pub edit_caret: i32,
    pub mode: i32,
    pub show_hidden: bool,
    pub sort: i32,
    pub sort_desc: bool,
    pub hist: [[u8; FM_PATH]; FM_HISTORY],
    pub hist_n: i32,
    pub hist_pos: i32,
    pub clip: [[u8; FM_PATH]; 16],
    pub nclip: i32,
    pub clip_cut: bool,
    pub status: [u8; 96],
    pub view: [i32; FM_MAX_ENTRIES],
    pub nview: i32,
    pub open_file: Option<OpenFn>,
    pub user: *mut c_void,
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
fn lc(c: u8) -> u8 { if c.is_ascii_uppercase() { c + 32 } else { c } }
fn ci_cmp(a: &[u8], b: &[u8]) -> i32 {
    let n = a.len().min(b.len());
    for i in 0..n {
        if lc(a[i]) != lc(b[i]) {
            return lc(a[i]) as i32 - lc(b[i]) as i32;
        }
    }
    if a.len() == b.len() { 0 } else if a.len() < b.len() { -1 } else { 1 }
}
fn ci_contains(h: &[u8], n: &[u8]) -> bool {
    if n.is_empty() { return true; }
    if n.len() > h.len() { return false; }
    for s in 0..=h.len() - n.len() {
        if (0..n.len()).all(|i| lc(h[s + i]) == lc(n[i])) { return true; }
    }
    false
}
fn cstr_join(dir: &[u8], name: &[u8]) -> [u8; FM_PATH] {
    let mut out = [0u8; FM_PATH];
    let mut n = 0;
    if bytes0(dir) == b"/" {
        out[n] = b'/';
        n += 1;
    } else {
        for &c in bytes0(dir) {
            if n < FM_PATH - 1 { out[n] = c; n += 1; }
        }
        if n < FM_PATH - 1 { out[n] = b'/'; n += 1; }
    }
    for &c in bytes0(name) {
        if n < FM_PATH - 1 { out[n] = c; n += 1; }
    }
    out
}
fn fmt_num(out: &mut [u8], mut v: u32) -> usize {
    let mut tmp = [0u8; 12];
    let mut n = 0;
    if v == 0 { tmp[0] = b'0'; n = 1; }
    while v != 0 { tmp[n] = b'0' + (v % 10) as u8; n += 1; v /= 10; }
    for i in (0..n).rev() { out[n - 1 - i] = tmp[i]; }
    n
}

pub fn join(dir: &[u8], name: &[u8]) -> [u8; FM_PATH] { cstr_join(dir, name) }

fn sort_view(f: &mut ShellFilemgr) {
    for i in 1..f.nview as usize {
        let v = f.view[i];
        let mut j = i;
        while j > 0 {
            let a = f.ent[f.view[j - 1] as usize];
            let b = f.ent[v as usize];
            let mut c;
            if a.is_dir != b.is_dir {
                c = if a.is_dir { -1 } else { 1 };
            } else {
                c = match f.sort {
                    FM_SORT_SIZE => if a.size < b.size { -1 } else { (a.size > b.size) as i32 },
                    FM_SORT_DATE => if a.mtime < b.mtime { -1 } else { (a.mtime > b.mtime) as i32 },
                    _ => ci_cmp(bytes0(&a.name), bytes0(&b.name)),
                };
                if c == 0 { c = ci_cmp(bytes0(&a.name), bytes0(&b.name)); }
                if f.sort_desc { c = -c; }
            }
            if c <= 0 { break; }
            f.view[j] = f.view[j - 1];
            j -= 1;
        }
        f.view[j] = v;
    }
}

pub fn refresh(f: &mut ShellFilemgr) {
    let mut keep = [[0u8; 64]; 8];
    let mut nkeep = 0;
    for i in 0..f.n as usize {
        if nkeep >= 8 { break; }
        if f.selected[i] != 0 {
            keep[nkeep] = f.ent[i].name;
            nkeep += 1;
        }
    }
    let be = unsafe { &*f.be };
    // SAFETY: caller-owned entries; refresh rewrites them.
    let n = be.fs_list(f.cwd.as_ptr() as *const c_char, f.ent.as_mut_ptr() as *mut c_void, FM_MAX_ENTRIES as i32);
    f.n = if n < 0 { 0 } else { n.min(FM_MAX_ENTRIES as i32) };
    f.selected = [0; FM_MAX_ENTRIES];
    for i in 0..f.n as usize {
        for k in 0..nkeep {
            if f.ent[i].name == keep[k] {
                f.selected[i] = 1;
            }
        }
    }
    f.nview = 0;
    for i in 0..f.n as usize {
        let name = bytes0(&f.ent[i].name);
        if !f.show_hidden && ((f.ent[i].attrs & FSA_HIDDEN) != 0 || name.first() == Some(&b'.')) {
            continue;
        }
        if !ci_contains(name, bytes0(&f.filter)) {
            continue;
        }
        f.view[f.nview as usize] = i as i32;
        f.nview += 1;
    }
    sort_view(f);
    if f.cursor >= f.nview {
        f.cursor = if f.nview != 0 { f.nview - 1 } else { 0 };
    }
    f.nvols = be.fs_volumes(f.vols.as_mut_ptr(), 4).max(0).min(4);
    let mut tot = 0u64;
    let mut sel = 0;
    for i in 0..f.n as usize {
        if f.selected[i] != 0 {
            sel += 1;
            tot += f.ent[i].size;
        }
    }
    if sel != 0 {
        let mut sz = [0u8; 24];
        util::size(tot, &mut sz);
        let mut b = [0u8; 96];
        let mut k = fmt_num(&mut b, sel);
        for &c in b" of " { b[k] = c; k += 1; }
        k += fmt_num(&mut b[k..], f.nview as u32);
        for &c in b" selected (" { b[k] = c; k += 1; }
        for &c in bytes0(&sz) { b[k] = c; k += 1; }
        b[k] = b')'; k += 1;
        put(&mut f.status, &b[..k]);
    } else {
        let mut b = [0u8; 96];
        let k = fmt_num(&mut b, f.nview as u32);
        for &c in b" items" { b[k] = c; }
        put(&mut f.status, &b[..k + 6]);
    }
}

fn set_status(f: &mut ShellFilemgr, m: &[u8]) {
    put(&mut f.status, m);
}

pub fn navigate(f: &mut ShellFilemgr, path: &[u8], record: bool) -> bool {
    let path = bytes0(path);
    if path.len() >= FM_PATH {
        return false;
    }
    let mut p = [0u8; FM_PATH];
    let mut n = path.len();
    p[..n].copy_from_slice(path);
    while n > 1 && p[n - 1] == b'/' {
        n -= 1;
        p[n] = 0;
    }
    let mut e = FsEntry::EMPTY;
    let be = unsafe { &*f.be };
    if p[0] != b'/' || !be.fs_stat(p.as_ptr() as *const c_char, &mut e) || !e.is_dir {
        set_status(f, b"Not a folder");
        return false;
    }
    if record {
        if f.hist_n != 0 && f.hist_pos + 1 < f.hist_n {
            f.hist_n = f.hist_pos + 1;
        }
        if f.hist_n as usize == FM_HISTORY {
            f.hist.copy_within(1..FM_HISTORY, 0);
            f.hist_n -= 1;
        }
        f.hist[f.hist_n as usize] = p;
        f.hist_n += 1;
        f.hist_pos = f.hist_n - 1;
    }
    f.cwd = p;
    f.addr = p;
    f.addr_caret = bytes0(&f.cwd).len() as i32;
    f.cursor = 0;
    f.scroll = 0;
    f.selected = [0; FM_MAX_ENTRIES];
    f.filter[0] = 0;
    f.filter_caret = 0;
    f.mode = FM_MODE_BROWSE;
    refresh(f);
    true
}

pub fn back(f: &mut ShellFilemgr) -> bool {
    if f.hist_pos <= 0 { return false; }
    f.hist_pos -= 1;
    let p = f.hist[f.hist_pos as usize];
    navigate(f, bytes0(&p), false)
}
pub fn forward(f: &mut ShellFilemgr) -> bool {
    if f.hist_pos + 1 >= f.hist_n { return false; }
    f.hist_pos += 1;
    let p = f.hist[f.hist_pos as usize];
    navigate(f, bytes0(&p), false)
}
pub fn up(f: &mut ShellFilemgr) -> bool {
    if bytes0(&f.cwd) == b"/" { return false; }
    let cwd = f.cwd;
    let s = bytes0(&cwd);
    let pos = s.iter().rposition(|&c| c == b'/').unwrap_or(0);
    let mut p = [0u8; FM_PATH];
    if pos == 0 {
        p[0] = b'/';
    } else {
        p[..pos].copy_from_slice(&s[..pos]);
    }
    navigate(f, bytes0(&p), true)
}

pub fn init(f: &mut ShellFilemgr, be: *const ShellBackend, start: &[u8]) {
    f.be = be;
    f.cwd = [0; FM_PATH];
    f.ent = [FsEntry::EMPTY; FM_MAX_ENTRIES];
    f.n = 0;
    f.selected = [0; FM_MAX_ENTRIES];
    f.cursor = 0;
    f.scroll = 0;
    f.vol_sel = 0;
    f.vol_scroll = 0;
    f.vols = [FsVolume::EMPTY; 4];
    f.nvols = 0;
    f.addr = [0; FM_PATH];
    f.addr_caret = 0;
    f.filter = [0; 40];
    f.filter_caret = 0;
    f.edit = [0; 64];
    f.edit_caret = 0;
    f.mode = FM_MODE_BROWSE;
    f.show_hidden = false;
    f.sort = FM_SORT_NAME;
    f.sort_desc = false;
    f.hist = [[0; FM_PATH]; FM_HISTORY];
    f.hist_n = 0;
    f.hist_pos = 0;
    f.clip = [[0; FM_PATH]; 16];
    f.nclip = 0;
    f.clip_cut = false;
    f.status = [0; 96];
    f.view = [0; FM_MAX_ENTRIES];
    f.nview = 0;
    f.open_file = None;
    f.user = core::ptr::null_mut();
    if !navigate(f, if start.is_empty() { b"/" } else { start }, true) {
        navigate(f, b"/", true);
    }
}

pub fn set_sort(f: &mut ShellFilemgr, s: i32) {
    if f.sort == s { f.sort_desc = !f.sort_desc; } else { f.sort = s; f.sort_desc = false; }
    refresh(f);
}
pub fn selected_count(f: &ShellFilemgr) -> i32 {
    f.selected[..f.n as usize].iter().filter(|&&b| b != 0).count() as i32
}
pub fn select_all(f: &mut ShellFilemgr) {
    f.selected = [0; FM_MAX_ENTRIES];
    for i in 0..f.nview as usize {
        f.selected[f.view[i] as usize] = 1;
    }
    refresh(f);
}

fn valid_name(n: &[u8]) -> bool {
    !n.is_empty() && n != b"." && n != b".." && !n.contains(&b'/') && n.len() < 60
}

pub fn new_folder(f: &mut ShellFilemgr, name: &[u8]) -> bool {
    if !valid_name(name) { set_status(f, b"Invalid name"); return false; }
    let p = cstr_join(&f.cwd, name);
    let be = unsafe { &*f.be };
    if !be.fs_mkdir(p.as_ptr() as *const c_char) {
        set_status(f, b"Could not create folder (name in use?)");
        return false;
    }
    refresh(f);
    true
}

pub fn rename_selected(f: &mut ShellFilemgr, name: &[u8]) -> bool {
    let mut sel = -1i32;
    for i in 0..f.n as usize {
        if f.selected[i] != 0 {
            if sel >= 0 { set_status(f, b"Select one item to rename"); return false; }
            sel = i as i32;
        }
    }
    if sel < 0 && f.cursor < f.nview { sel = f.view[f.cursor as usize]; }
    if sel < 0 { return false; }
    if !valid_name(name) { set_status(f, b"Invalid name"); return false; }
    let from = cstr_join(&f.cwd, &f.ent[sel as usize].name);
    let to = cstr_join(&f.cwd, name);
    let be = unsafe { &*f.be };
    if !be.fs_rename(from.as_ptr() as *const c_char, to.as_ptr() as *const c_char) {
        set_status(f, b"Could not rename (name in use?)");
        return false;
    }
    f.selected = [0; FM_MAX_ENTRIES];
    refresh(f);
    for i in 0..f.n as usize {
        if eq(&f.ent[i].name, name) {
            f.selected[i] = 1;
        }
    }
    refresh(f);
    true
}

pub fn delete_selected(f: &mut ShellFilemgr) -> bool {
    let (mut done, mut failed) = (0, 0);
    let be = unsafe { &*f.be };
    for i in 0..f.n as usize {
        if f.selected[i] == 0 { continue; }
        let p = cstr_join(&f.cwd, &f.ent[i].name);
        if be.fs_remove(p.as_ptr() as *const c_char) { done += 1; } else { failed += 1; }
    }
    f.selected = [0; FM_MAX_ENTRIES];
    refresh(f);
    if failed != 0 {
        let mut b = [0u8; 96];
        let mut k = fmt_num(&mut b, done);
        for &c in b" deleted, " { b[k] = c; k += 1; }
        k += fmt_num(&mut b[k..], failed);
        for &c in b" could not be deleted" { b[k] = c; k += 1; }
        put(&mut f.status, &b[..k]);
    }
    done > 0 && failed == 0
}

pub fn copy(f: &mut ShellFilemgr, cut: bool) {
    f.nclip = 0;
    f.clip_cut = cut;
    for i in 0..f.n as usize {
        if f.nclip as usize >= 16 { break; }
        if f.selected[i] != 0 {
            f.clip[f.nclip as usize] = cstr_join(&f.cwd, &f.ent[i].name);
            f.nclip += 1;
        }
    }
    if f.nclip == 0 && f.cursor < f.nview {
        f.clip[0] = cstr_join(&f.cwd, &f.ent[f.view[f.cursor as usize] as usize].name);
        f.nclip = 1;
    }
    let mut b = [0u8; 64];
    let mut k = fmt_num(&mut b, f.nclip as u32);
    for &c in if cut { &b" item(s) cut"[..] } else { &b" item(s) copied"[..] } { b[k] = c; k += 1; }
    put(&mut f.status, &b[..k]);
}

fn unique_dest(f: &mut ShellFilemgr, name: &[u8], out: &mut [u8; FM_PATH]) {
    let be = unsafe { &*f.be };
    let mut e = FsEntry::EMPTY;
    *out = cstr_join(&f.cwd, name);
    if !be.fs_stat(out.as_ptr() as *const c_char, &mut e) { return; }
    let nm = bytes0(name);
    let dot = nm.iter().rposition(|&c| c == b'.');
    let stem = match dot { Some(d) if d != 0 => d, _ => nm.len() };
    for k in 1..1000u32 {
        let mut cand = [0u8; 96];
        let mut n = 0;
        for &c in &nm[..stem] { cand[n] = c; n += 1; }
        for &c in b" (copy" { cand[n] = c; n += 1; }
        if k > 1 {
            cand[n] = b' '; n += 1;
            let mut tmp = [0u8; 12];
            let mut d = 0;
            let mut v = k;
            if v == 0 { tmp[0] = b'0'; d = 1; }
            while v != 0 { tmp[d] = b'0' + (v % 10) as u8; d += 1; v /= 10; }
            for i in (0..d).rev() { cand[n] = tmp[i]; n += 1; }
        }
        cand[n] = b')'; n += 1;
        for &c in &nm[stem..] { cand[n] = c; n += 1; }
        *out = cstr_join(&f.cwd, bytes0(&cand[..n]));
        if !be.fs_stat(out.as_ptr() as *const c_char, &mut e) { return; }
    }
}

pub fn paste(f: &mut ShellFilemgr) -> bool {
    let mut ok = 0;
    let be = unsafe { &*f.be };
    for i in 0..f.nclip as usize {
        let src = f.clip[i];
        let s = bytes0(&src);
        let pos = s.iter().rposition(|&c| c == b'/').map_or(0, |p| p + 1);
        let name = &s[pos..];
        let mut dest = [0u8; FM_PATH];
        if f.clip_cut {
            dest = cstr_join(&f.cwd, name);
            let mut srcdir = [0u8; FM_PATH];
            let dpos = if pos > 0 { pos - 1 } else { 0 };
            if dpos == 0 {
                srcdir[0] = b'/';
            } else {
                srcdir[..dpos].copy_from_slice(&s[..dpos]);
            }
            if bytes0(&srcdir) == bytes0(&f.cwd) { continue; }
            unique_dest(f, name, &mut dest);
            if be.fs_rename(src.as_ptr() as *const c_char, dest.as_ptr() as *const c_char) { ok += 1; }
        } else {
            unique_dest(f, name, &mut dest);
            if be.fs_copy(src.as_ptr() as *const c_char, dest.as_ptr() as *const c_char) { ok += 1; }
        }
    }
    if f.clip_cut && ok != 0 { f.nclip = 0; }
    refresh(f);
    ok > 0
}

// ---------------------------------------------------------------- UI

unsafe extern "C" fn row_entry(u: *mut UiCtx, row: Rect, i: i32, cursor: bool, user: *mut c_void) {
    let (Some(u), Some(f)) = (unsafe { u.as_mut() }, unsafe { (user as *mut ShellFilemgr).as_mut() }) else { return };
    let idx = f.view[i as usize] as usize;
    let e = f.ent[idx];
    let sel = f.selected[idx] != 0;
    let t = unsafe { *u.theme };
    if sel && !cursor {
        unsafe { ffi::fb_fill(u.fb, u.clip, row, mix(t.surface, t.accent, 60)) };
    }
    let tc = if cursor { t.accent_text } else { t.text };
    let icon = Rect::new(row.x + 8, row.y + 4, 14, 14);
    unsafe { ffi::fb_fill_round(u.fb, u.clip, icon, if e.is_dir { 3 } else { 2 }, if e.is_dir { 0xFFF5_9E0B } else { t.text_dim }) };
    let mut name = [0u8; 80];
    crate::text::ellipsize(&mut name, bytes0(&e.name), row.w - 330, &crate::font_data::regular(), 14);
    let hidden = (e.attrs & FSA_HIDDEN) != 0 || e.name[0] == b'.';
    ui::label(u, row.x + 30, row.y + 3, bytes0(&name), if hidden { t.text_dim } else { tc });
    let mut sz = [0u8; 24];
    if !e.is_dir {
        util::size(e.size, &mut sz);
    }
    ui::label_right(u, row.x + row.w - 176, row.y + 3, bytes0(&sz), if cursor { t.accent_text } else { t.text_dim });
    let mut tm = [0u8; 24];
    util::time(e.mtime, &mut tm);
    ui::label_right(u, row.x + row.w - 10, row.y + 3, bytes0(&tm), if cursor { t.accent_text } else { t.text_dim });
}

unsafe extern "C" fn row_vol(u: *mut UiCtx, row: Rect, i: i32, sel: bool, user: *mut c_void) {
    let (Some(u), Some(f)) = (unsafe { u.as_mut() }, unsafe { (user as *mut ShellFilemgr).as_mut() }) else { return };
    let v = f.vols[i as usize];
    let t = unsafe { *u.theme };
    let mut sz = [0u8; 24];
    util::size(v.avail, &mut sz);
    ui::label(u, row.x + 8, row.y + 2, if v.label[0] != 0 { bytes0(&v.label) } else { bytes0(&v.mount) }, if sel { t.accent_text } else { t.text });
    let mut line = [0u8; 48];
    let mut n = 0;
    for &c in bytes0(&v.fstype) { line[n] = c; n += 1; }
    line[n] = b' '; n += 1;
    for &c in bytes0(&sz) { line[n] = c; n += 1; }
    if v.readonly {
        for &c in b" ro" { line[n] = c; n += 1; }
    }
    ui::label(u, row.x + 8, row.y + 18, bytes0(&line[..n]), t.text_dim);
}

fn open_cursor(f: &mut ShellFilemgr) {
    if f.cursor >= f.nview { return; }
    let e = f.ent[f.view[f.cursor as usize] as usize];
    let p = cstr_join(&f.cwd, &e.name);
    if e.is_dir {
        navigate(f, bytes0(&p), true);
    } else if let Some(cb) = f.open_file {
        // SAFETY: host callback with a live user pointer.
        unsafe { cb(p.as_ptr() as *const c_char, f.user) };
    }
}

pub fn frame(f: &mut ShellFilemgr, u: &mut UiCtx, w: i32, h: i32) {
    let t = unsafe { *u.theme };
    ui::clear(u, t.surface);
    let browsing = f.mode == FM_MODE_BROWSE;
    let mut parked = [crate::ui::KeyInput::EMPTY; crate::ui::UI_KEYQ];
    let mut nparked = 0;
    let (mut p_pressed, mut p_released, mut p_wheel) = (false, false, 0);
    if !browsing {
        nparked = u.nkeys as usize;
        parked[..nparked].copy_from_slice(&u.keys[..nparked]);
        p_pressed = u.pressed;
        p_released = u.released;
        p_wheel = u.wheel;
        u.nkeys = 0;
        u.pressed = false;
        u.released = false;
        u.wheel = 0;
    }
    if ui::button(u, Rect::new(8, 8, 32, 28), b"<") { back(f); }
    if ui::button(u, Rect::new(44, 8, 32, 28), b">") { forward(f); }
    if ui::button(u, Rect::new(80, 8, 32, 28), b"^") { up(f); }
    let fw = 200;
    if ui::textfield(u, Rect::new(120, 8, w - 120 - fw - 24, 28), &mut f.addr, &mut f.addr_caret, b"Path") {
        let addr = f.addr;
        if !navigate(f, bytes0(&addr), true) {
            f.addr = f.cwd;
        }
    }
    let before = f.filter;
    ui::textfield(u, Rect::new(w - fw - 8, 8, fw, 28), &mut f.filter, &mut f.filter_caret, b"Filter");
    if before != f.filter {
        refresh(f);
    }
    let side = 170;
    let top = 44;
    let bottom = h - 70;
    ui::label(u, 12, top + 2, b"Places", t.text_dim);
    let vs = f.vol_sel;
    let act = ui::list(u, Rect::new(8, top + 22, side - 8, 4 * 40), b"volumes", f.nvols, 40, Some(row_vol), f as *mut ShellFilemgr as *mut c_void, &mut f.vol_sel, &mut f.vol_scroll);
    if f.vol_sel != vs || act >= 0 {
        if f.vol_sel < f.nvols {
            let m = f.vols[f.vol_sel as usize].mount;
            navigate(f, bytes0(&m), true);
        }
    }
    let lx = side + 8;
    let lw = w - lx - 8;
    let hdr = Rect::new(lx, top, lw, 24);
    let sort_name: &[u8] = if f.sort == FM_SORT_NAME { if f.sort_desc { b"Name v\0" } else { b"Name ^\0" } } else { b"Name\0" };
    let sort_size: &[u8] = if f.sort == FM_SORT_SIZE { if f.sort_desc { b"Size v\0" } else { b"Size ^\0" } } else { b"Size\0" };
    let sort_date: &[u8] = if f.sort == FM_SORT_DATE { if f.sort_desc { b"Modified v\0" } else { b"Modified ^\0" } } else { b"Modified\0" };
    ui::label(u, lx + 30, top + 4, sort_name, t.text_dim);
    ui::label_right(u, lx + lw - 176, top + 4, sort_size, t.text_dim);
    ui::label_right(u, lx + lw - 10, top + 4, sort_date, t.text_dim);
    if u.pressed && ui::hovered(u, hdr) {
        if u.mx > lx + lw - 130 { set_sort(f, FM_SORT_DATE); }
        else if u.mx > lx + lw - 260 { set_sort(f, FM_SORT_SIZE); }
        else { set_sort(f, FM_SORT_NAME); }
    }
    unsafe { ffi::fb_hline(u.fb, u.clip, lx, top + 24, lw, t.border) };
    let focus = u.focus;
    if browsing && (focus == ui::id(b"entries") || focus == 0 || focus == ui::id(b"volumes")) {
        if ui::take_key(u, b'c' as u32, MOD_CTRL) { copy(f, false); }
        if ui::take_key(u, b'x' as u32, MOD_CTRL) { copy(f, true); }
        if ui::take_key(u, b'v' as u32, MOD_CTRL) { paste(f); }
        if ui::take_key(u, b'a' as u32, MOD_CTRL) { select_all(f); }
        if ui::take_key(u, KEY_DELETE, 0) && selected_count(f) != 0 { f.mode = FM_MODE_CONFIRM_DELETE; }
        if ui::take_key(u, KEY_F1 + 1, 0) {
            f.mode = FM_MODE_RENAME;
            f.edit[0] = 0;
            if f.cursor < f.nview { f.edit = f.ent[f.view[f.cursor as usize] as usize].name; }
            f.edit_caret = bytes0(&f.edit).len() as i32;
        }
        if ui::take_key(u, KEY_F1 + 4, 0) { refresh(f); }
        if ui::take_key(u, KEY_BACKSPACE, 0) { up(f); }
        if ui::take_key(u, b'h' as u32, MOD_CTRL) {
            f.show_hidden = !f.show_hidden;
            refresh(f);
        }
    }
    let lr = Rect::new(lx, top + 26, lw, bottom - top - 26);
    let cursor_before = f.cursor;
    let opened = ui::list(u, lr, b"entries", f.nview, 22, Some(row_entry), f as *mut ShellFilemgr as *mut c_void, &mut f.cursor, &mut f.scroll);
    if u.pressed && ui::hovered(u, lr) && f.nview != 0 && f.cursor < f.nview {
        let idx = f.view[f.cursor as usize] as usize;
        if (u.mods & MOD_CTRL) != 0 {
            f.selected[idx] ^= 1;
        } else if (u.mods & MOD_SHIFT) != 0 {
            let (a, b) = if cursor_before < f.cursor { (cursor_before, f.cursor) } else { (f.cursor, cursor_before) };
            let mut i = a;
            while i <= b && i < f.nview {
                f.selected[f.view[i as usize] as usize] = 1;
                i += 1;
            }
        } else {
            f.selected = [0; FM_MAX_ENTRIES];
            f.selected[idx] = 1;
        }
        refresh(f);
    } else if f.cursor != cursor_before && browsing && f.nview != 0 {
        f.selected = [0; FM_MAX_ENTRIES];
        f.selected[f.view[f.cursor as usize] as usize] = 1;
        refresh(f);
    }
    if opened >= 0 && browsing {
        open_cursor(f);
    }
    if f.nview == 0 {
        ui::label(u, lx + 14, top + 40, if f.filter[0] != 0 { b"No matches." } else { b"This folder is empty." }, t.text_dim);
    }
    let by = bottom + 8;
    const LABELS: [&[u8]; 6] = [b"New folder\0", b"Rename\0", b"Delete\0", b"Copy\0", b"Cut\0", b"Paste\0"];
    let mut bx = 8;
    for i in 0..6 {
        let bw = unsafe { ffi::shell_text_width(LABELS[i].as_ptr() as *const c_char) } + 24;
        if browsing && ui::button(u, Rect::new(bx, by, bw, 28), LABELS[i]) {
            match i {
                0 => {
                    f.mode = FM_MODE_NEWDIR;
                    put(&mut f.edit, b"New folder");
                    f.edit_caret = bytes0(&f.edit).len() as i32;
                }
                1 => {
                    if f.nview != 0 {
                        f.mode = FM_MODE_RENAME;
                        f.edit = f.ent[f.view[f.cursor as usize] as usize].name;
                        f.edit_caret = bytes0(&f.edit).len() as i32;
                    }
                }
                2 => if selected_count(f) != 0 { f.mode = FM_MODE_CONFIRM_DELETE; },
                3 => copy(f, false),
                4 => copy(f, true),
                _ => { paste(f); }
            }
        } else if !browsing {
            unsafe { ffi::fb_fill_round(u.fb, u.clip, Rect::new(bx, by, bw, 28), 6, t.surface_hi) };
        }
        bx += bw + 6;
    }
    ui::label(u, 12, by + 36, bytes0(&f.status), t.text_dim);
    ui::label_right(u, w - 10, by + 36, if f.show_hidden { b"hidden files shown" } else { b"" }, t.text_dim);
    if !browsing {
        u.keys[..nparked].copy_from_slice(&parked[..nparked]);
        u.nkeys = nparked as i32;
        u.pressed = p_pressed;
        u.released = p_released;
        u.wheel = p_wheel;
        let m = Rect::new(w / 2 - 230, h / 2 - 50, 460, 100);
        unsafe {
            ffi::fb_fill_round(u.fb, u.clip, Rect::new(m.x - 2, m.y - 2, m.w + 4, m.h + 4), 10, t.border);
            ffi::fb_fill_round(u.fb, u.clip, m, 8, t.surface_hi);
        }
        if f.mode == FM_MODE_CONFIRM_DELETE {
            let mut msg = [0u8; 80];
            let mut n = 0;
            for &c in b"Delete " { msg[n] = c; n += 1; }
            n += fmt_num(&mut msg[n..], selected_count(f) as u32);
            for &c in b" item(s)? This cannot be undone." { msg[n] = c; n += 1; }
            ui::label(u, m.x + 16, m.y + 16, bytes0(&msg[..n]), t.text);
            if ui::button_accent(u, Rect::new(m.x + 16, m.y + 56, 120, 30), b"Delete") {
                delete_selected(f);
                f.mode = FM_MODE_BROWSE;
            }
            if ui::button(u, Rect::new(m.x + 144, m.y + 56, 120, 30), b"Cancel") || ui::take_key(u, KEY_ESC, 0) {
                f.mode = FM_MODE_BROWSE;
            }
        } else {
            ui::label(u, m.x + 16, m.y + 12, if f.mode == FM_MODE_RENAME { b"Rename to:" } else { b"New folder name:" }, t.text);
            u.focus = ui::textfield_id(b"name", Rect::new(m.x + 16, m.y + 34, m.w - 32, 28));
            if ui::textfield(u, Rect::new(m.x + 16, m.y + 34, m.w - 32, 28), &mut f.edit, &mut f.edit_caret, b"name") {
                let edit = f.edit;
                let ok = if f.mode == FM_MODE_RENAME { rename_selected(f, bytes0(&edit)) } else { new_folder(f, bytes0(&edit)) };
                if ok { f.mode = FM_MODE_BROWSE; }
            }
            if ui::take_key(u, KEY_ESC, 0) { f.mode = FM_MODE_BROWSE; }
            ui::label(u, m.x + 16, m.y + 72, b"Enter to confirm, Esc to cancel", t.text_dim);
        }
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn fm_init(f: *mut ShellFilemgr, be: *const ShellBackend, start: *const c_char) {
    if let Some(f) = unsafe { f.as_mut() } {
        init(f, be, unsafe { ffi::c_bytes(start) });
    }
}
#[no_mangle]
pub unsafe extern "C" fn fm_navigate(f: *mut ShellFilemgr, path: *const c_char, record: bool) -> bool {
    unsafe { f.as_mut() }.map_or(false, |f| navigate(f, unsafe { ffi::c_bytes(path) }, record))
}
#[no_mangle]
pub unsafe extern "C" fn fm_refresh(f: *mut ShellFilemgr) {
    if let Some(f) = unsafe { f.as_mut() } {
        refresh(f);
    }
}
#[no_mangle]
pub unsafe extern "C" fn fm_back(f: *mut ShellFilemgr) -> bool {
    unsafe { f.as_mut() }.map_or(false, back)
}
#[no_mangle]
pub unsafe extern "C" fn fm_forward(f: *mut ShellFilemgr) -> bool {
    unsafe { f.as_mut() }.map_or(false, forward)
}
#[no_mangle]
pub unsafe extern "C" fn fm_up(f: *mut ShellFilemgr) -> bool {
    unsafe { f.as_mut() }.map_or(false, up)
}
#[no_mangle]
pub unsafe extern "C" fn fm_set_sort(f: *mut ShellFilemgr, s: i32) {
    if let Some(f) = unsafe { f.as_mut() } {
        set_sort(f, s);
    }
}
#[no_mangle]
pub unsafe extern "C" fn fm_select_all(f: *mut ShellFilemgr) {
    if let Some(f) = unsafe { f.as_mut() } {
        select_all(f);
    }
}
#[no_mangle]
pub unsafe extern "C" fn fm_copy(f: *mut ShellFilemgr, cut: bool) {
    if let Some(f) = unsafe { f.as_mut() } {
        copy(f, cut);
    }
}
#[no_mangle]
pub unsafe extern "C" fn fm_paste(f: *mut ShellFilemgr) -> bool {
    unsafe { f.as_mut() }.map_or(false, paste)
}
#[no_mangle]
pub unsafe extern "C" fn fm_delete_selected(f: *mut ShellFilemgr) -> bool {
    unsafe { f.as_mut() }.map_or(false, delete_selected)
}
#[no_mangle]
pub unsafe extern "C" fn fm_new_folder(f: *mut ShellFilemgr, name: *const c_char) -> bool {
    unsafe { f.as_mut() }.map_or(false, |f| new_folder(f, unsafe { ffi::c_bytes(name) }))
}
#[no_mangle]
pub unsafe extern "C" fn fm_rename_selected(f: *mut ShellFilemgr, name: *const c_char) -> bool {
    unsafe { f.as_mut() }.map_or(false, |f| rename_selected(f, unsafe { ffi::c_bytes(name) }))
}
#[no_mangle]
pub unsafe extern "C" fn fm_selected_count(f: *const ShellFilemgr) -> i32 {
    unsafe { f.as_ref() }.map_or(0, selected_count)
}
#[no_mangle]
pub unsafe extern "C" fn fm_join(dir: *const c_char, name: *const c_char, out: *mut c_char, cap: usize) {
    if out.is_null() || cap == 0 {
        return;
    }
    let p = cstr_join(unsafe { ffi::c_bytes(dir) }, unsafe { ffi::c_bytes(name) });
    // SAFETY: caller provides cap bytes.
    let o = unsafe { core::slice::from_raw_parts_mut(out as *mut u8, cap) };
    put(o, bytes0(&p));
}
#[no_mangle]
pub unsafe extern "C" fn fm_frame(f: *mut ShellFilemgr, u: *mut UiCtx, w: i32, h: i32) {
    if let (Some(f), Some(u)) = (unsafe { f.as_mut() }, unsafe { u.as_mut() }) {
        frame(f, u, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    fn mgr() -> &'static mut ShellFilemgr {
        Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellFilemgr>() }))
    }
    fn entry(name: &[u8], dir: bool, size: u64) -> FsEntry {
        let mut e = FsEntry::EMPTY;
        let n = name.len().min(63);
        e.name[..n].copy_from_slice(&name[..n]);
        e.is_dir = dir;
        e.size = size;
        e
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ShellFilemgr>(), 34544);
        assert_eq!(offset_of!(ShellFilemgr, cwd), 8);
        assert_eq!(offset_of!(ShellFilemgr, ent), 168);
        assert_eq!(offset_of!(ShellFilemgr, n), 24744);
        assert_eq!(offset_of!(ShellFilemgr, selected), 24748);
        assert_eq!(offset_of!(ShellFilemgr, cursor), 25004);
        assert_eq!(offset_of!(ShellFilemgr, vols), 25024);
        assert_eq!(offset_of!(ShellFilemgr, addr), 25412);
        assert_eq!(offset_of!(ShellFilemgr, filter), 25576);
        assert_eq!(offset_of!(ShellFilemgr, edit), 25620);
        assert_eq!(offset_of!(ShellFilemgr, hist), 25701);
        assert_eq!(offset_of!(ShellFilemgr, clip), 30832);
        assert_eq!(offset_of!(ShellFilemgr, status), 33397);
        assert_eq!(offset_of!(ShellFilemgr, view), 33496);
        assert_eq!(offset_of!(ShellFilemgr, open_file), 34528);
    }

    #[test]
    fn joins_paths() {
        assert_eq!(bytes0(&cstr_join(b"/", b"home")), b"/home");
        assert_eq!(bytes0(&cstr_join(b"/home", b"user")), b"/home/user");
        assert!(valid_name(b"notes"));
        assert!(!valid_name(b".."));
        assert!(!valid_name(b"a/b"));
        assert!(!valid_name(b""));
    }

    #[test]
    fn sorts_dirs_first_then_name_or_size() {
        let f = mgr();
        f.ent[0] = entry(b"zeta", false, 10);
        f.ent[1] = entry(b"Alpha", true, 0);
        f.ent[2] = entry(b"Beta", false, 20);
        f.ent[3] = entry(b"alpha", false, 5);
        f.n = 4;
        f.nview = 4;
        f.view[..4].copy_from_slice(&[0, 1, 2, 3]);
        f.sort = FM_SORT_NAME;
        sort_view(f);
        assert_eq!(f.view[0], 1, "directories first");
        assert_eq!(f.view[1], 3, "case-insensitive name");
        assert_eq!(f.view[2], 2);
        assert_eq!(f.view[3], 0);
        f.sort = FM_SORT_SIZE;
        f.sort_desc = false;
        sort_view(f);
        assert_eq!(f.view[0], 1);
        assert_eq!(f.view[1], 3);
        assert_eq!(f.view[2], 0);
        assert_eq!(f.view[3], 2);
        f.sort_desc = true;
        sort_view(f);
        assert_eq!(f.view[0], 1, "directories stay first");
    }

    #[test]
    fn ci_helpers() {
        assert!(ci_contains(b"README.txt", b"readme"));
        assert!(!ci_contains(b"abc", b"abcd"));
        assert_eq!(ci_cmp(b"abc", b"ABC"), 0);
    }
}

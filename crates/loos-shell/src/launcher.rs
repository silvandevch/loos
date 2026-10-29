//! Start menu / launcher (C `launcher.c`): fuzzy local search, pinned grid, results and footer.
#![allow(missing_docs)]

use crate::compositor::{Compositor, KEY_BACKSPACE, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_LEFT, KEY_RIGHT, KEY_TAB, KEY_UP};
use crate::draw::{mix, Rect};
use crate::ffi::{self, Fb};
use crate::font::ShellFont;
use crate::font_data;
use crate::inventory::{ShellInventory, ShellItem, ITEM_APP, ITEM_FILE, ITEM_SETTING};
use core::ffi::{c_char, c_void};

pub const LAUNCHER_QUERY_MAX: usize = 64;
pub const LAUNCHER_RESULTS: usize = 8;
const GRID_COLS: usize = 6;
const GRID_MAX: usize = 18;

pub type LaunchFn = unsafe extern "C" fn(*const ShellItem, *mut c_void);

/// C `shell_launcher`.
#[repr(C)]
#[allow(missing_docs)]
pub struct ShellLauncher {
    pub inv: *const ShellInventory,
    pub query: [u8; LAUNCHER_QUERY_MAX],
    pub nq: i32,
    pub hit: [i32; LAUNCHER_RESULTS],
    pub nhit: i32,
    pub sel: i32,
    pub open: bool,
    pub launch: Option<LaunchFn>,
    pub user: *mut c_void,
}

impl ShellLauncher {
    pub const EMPTY: Self = Self {
        inv: core::ptr::null(),
        query: [0; LAUNCHER_QUERY_MAX],
        nq: 0,
        hit: [0; LAUNCHER_RESULTS],
        nhit: 0,
        sel: 0,
        open: false,
        launch: None,
        user: core::ptr::null_mut(),
    };
}

fn lc(c: u8) -> u8 {
    if c.is_ascii_uppercase() { c + 32 } else { c }
}

fn bytes0(a: &[u8]) -> &[u8] {
    let n = a.iter().position(|&b| b == 0).unwrap_or(a.len());
    &a[..n]
}

fn score_text(q: &[u8], h: &[u8], allow_subseq: bool) -> i32 {
    let (ql, hl) = (q.len(), h.len());
    if ql == 0 {
        return 1;
    }
    if ql > hl {
        return 0;
    }
    if ql == hl {
        let mut i = 0;
        while i < ql && lc(q[i]) == lc(h[i]) {
            i += 1;
        }
        if i == ql {
            return 1000;
        }
    }
    let mut i = 0;
    while i < ql && lc(q[i]) == lc(h[i]) {
        i += 1;
    }
    if i == ql {
        return 800;
    }
    for s in 1..=hl.saturating_sub(ql) {
        if !matches!(h[s - 1], b' ' | b'-' | b'_' | b'.') {
            continue;
        }
        let mut k = 0;
        while k < ql && lc(q[k]) == lc(h[s + k]) {
            k += 1;
        }
        if k == ql {
            return 600 - s as i32;
        }
    }
    for s in 1..=hl.saturating_sub(ql) {
        let mut k = 0;
        while k < ql && lc(q[k]) == lc(h[s + k]) {
            k += 1;
        }
        if k == ql {
            return 400 - s as i32;
        }
    }
    if !allow_subseq {
        return 0;
    }
    let (mut qi, mut first, mut last) = (0usize, 0usize, 0usize);
    for hi in 0..hl {
        if qi < ql && lc(h[hi]) == lc(q[qi]) {
            if qi == 0 {
                first = hi;
            }
            last = hi;
            qi += 1;
        }
    }
    if qi == ql {
        let span = (last - first) as i32 + 1 - ql as i32;
        let v = 200 - span * 4;
        return if v > 20 { v } else { 20 };
    }
    0
}

fn fuzzy(q: &[u8], name: &[u8], kw: &[u8]) -> i32 {
    let s = score_text(bytes0(q), bytes0(name), true);
    let mut k = 0;
    for w in bytes0(kw).split(|&b| b == b' ') {
        if w.is_empty() {
            continue;
        }
        let t = score_text(bytes0(q), w, false) / 2;
        if t > k {
            k = t;
        }
    }
    if s > k { s } else { k }
}

fn inv<'a>(l: &ShellLauncher) -> Option<&'a ShellInventory> {
    if l.inv.is_null() { None } else { Some(unsafe { &*l.inv }) }
}

fn item<'a>(l: &ShellLauncher, i: i32) -> &'a ShellItem {
    unsafe { &(*l.inv).item[i as usize] }
}

fn grid_items(l: &ShellLauncher) -> [i32; GRID_MAX] {
    let mut out = [0i32; GRID_MAX];
    let Some(inv) = inv(l) else { return out };
    let mut n = 0;
    for i in 0..inv.n as usize {
        if n >= GRID_MAX {
            break;
        }
        if inv.item[i].kind == ITEM_APP {
            out[n] = i as i32;
            n += 1;
        }
    }
    out
}

fn grid_count(l: &ShellLauncher) -> i32 {
    let Some(inv) = inv(l) else { return 0 };
    let mut n = 0;
    for i in 0..inv.n as usize {
        if inv.item[i].kind == ITEM_APP {
            n += 1;
        }
    }
    n
}

pub fn init(l: &mut ShellLauncher, inventory: *const ShellInventory, launch: Option<LaunchFn>, user: *mut c_void) {
    *l = ShellLauncher::EMPTY;
    l.inv = inventory;
    l.launch = launch;
    l.user = user;
    search(l);
}

pub fn search(l: &mut ShellLauncher) {
    let mut score = [0i32; LAUNCHER_RESULTS];
    l.nhit = 0;
    l.sel = 0;
    let Some(inventory) = inv(l) else { return };
    for i in 0..inventory.n as usize {
        let it = &inventory.item[i];
        let mut s = fuzzy(bytes0(&l.query), bytes0(&it.name), bytes0(&it.keywords));
        if it.kind == ITEM_SETTING && l.nq != 0 {
            let k = score_text(bytes0(&l.query), b"settings", false) / 2;
            if k > s {
                s = k;
            }
        }
        if s == 0 {
            continue;
        }
        if it.kind == ITEM_APP {
            s += 3;
        }
        let mut pos = l.nhit as usize;
        while pos > 0 && score[pos - 1] < s {
            pos -= 1;
        }
        if pos >= LAUNCHER_RESULTS {
            continue;
        }
        let last = if (l.nhit as usize) < LAUNCHER_RESULTS { l.nhit as usize } else { LAUNCHER_RESULTS - 1 };
        let mut k = last;
        while k > pos {
            score[k] = score[k - 1];
            l.hit[k] = l.hit[k - 1];
            k -= 1;
        }
        score[pos] = s;
        l.hit[pos] = i as i32;
        if (l.nhit as usize) < LAUNCHER_RESULTS {
            l.nhit += 1;
        }
    }
}

pub fn open(l: &mut ShellLauncher) {
    l.open = true;
    l.query.fill(0);
    l.nq = 0;
    search(l);
}
pub fn close(l: &mut ShellLauncher) {
    l.open = false;
}

fn launch_selected(l: &mut ShellLauncher) {
    let g = grid_items(l);
    let ng = grid_count(l);
    if l.nq == 0 {
        if l.sel < ng {
            if let Some(f) = l.launch {
                // SAFETY: inventory index from the caller-owned inventory.
                unsafe { f(&(*l.inv).item[g[l.sel as usize] as usize], l.user) };
            }
        }
    } else if (l.sel as usize) < l.nhit as usize {
        if let Some(f) = l.launch {
            let idx = l.hit[l.sel as usize] as usize;
            // SAFETY: inventory index from the caller-owned inventory.
            unsafe { f(&(*l.inv).item[idx], l.user) };
        }
    }
    close(l);
}

pub fn char_event(l: &mut ShellLauncher, cp: u32) -> bool {
    if !l.open {
        return false;
    }
    if (32..127).contains(&cp) && (l.nq as usize) < LAUNCHER_QUERY_MAX - 1 {
        l.query[l.nq as usize] = cp as u8;
        l.nq += 1;
        l.query[l.nq as usize] = 0;
        search(l);
    }
    true
}

pub fn key(l: &mut ShellLauncher, key: u32, _mods: u32) -> bool {
    if !l.open {
        return false;
    }
    match key {
        KEY_ESC => close(l),
        KEY_BACKSPACE => {
            if l.nq > 0 {
                l.nq -= 1;
                l.query[l.nq as usize] = 0;
                search(l);
            }
        }
        KEY_DOWN | KEY_TAB | KEY_UP | KEY_LEFT | KEY_RIGHT => {
            let n = if l.nq != 0 { l.nhit } else { grid_count(l) };
            if n == 0 {
                return true;
            }
            let mut step = if key == KEY_UP { -1 } else { 1 };
            if l.nq == 0 && (key == KEY_DOWN || key == KEY_UP) {
                step *= GRID_COLS as i32;
            }
            if l.nq != 0 && (key == KEY_LEFT || key == KEY_RIGHT) {
                return true;
            }
            if key == KEY_LEFT {
                step = -1;
            }
            l.sel = ((l.sel + step) % n + n) % n;
        }
        KEY_ENTER => launch_selected(l),
        _ => {}
    }
    true
}

const L_W: i32 = 640;
const L_ROW: i32 = 40;
const L_INPUT: i32 = 64;
const TILE: i32 = 92;

fn launcher_h(c: &Compositor) -> i32 {
    let l = if c.taskbar.is_null() {
        None
    } else {
        // SAFETY: `taskbar` points at the live taskbar installed by taskbar_init.
        let t = unsafe { &*(c.taskbar as *const crate::taskbar::ShellTaskbar) };
        if t.launcher.is_null() { None } else { Some(unsafe { &*t.launcher }) }
    };
    let n = l.map_or(GRID_MAX as i32, grid_count);
    let mut rows = (n + GRID_COLS as i32 - 1) / GRID_COLS as i32;
    if rows < 2 {
        rows = 2;
    }
    let grid = L_INPUT + 40 + rows * (TILE + 8) + 16;
    let list = L_INPUT + 8 + LAUNCHER_RESULTS as i32 * L_ROW + 8;
    (if grid > list { grid } else { list }) + 56
}

pub fn rect(c: &Compositor) -> Rect {
    let t = if c.taskbar.is_null() { None } else { Some(unsafe { &*(c.taskbar as *const crate::taskbar::ShellTaskbar) }) };
    let h = launcher_h(c);
    let x = if t.map_or(false, |t| t.centered) { (c.lw() - L_W) / 2 } else { 8 };
    let y = if t.map_or(false, |t| t.top) {
        crate::taskbar::taskbar_h() + 8
    } else {
        c.lh() - crate::taskbar::taskbar_h() - 8 - h
    };
    Rect::new(if x < 0 { 0 } else { x }, if y < 0 { 0 } else { y }, L_W, h)
}

fn tile_rect(r: Rect, i: i32) -> Rect {
    Rect::new(r.x + 26 + (i % GRID_COLS as i32) * (TILE + 6), r.y + L_INPUT + 40 + (i / GRID_COLS as i32) * (TILE + 8), TILE, TILE)
}
fn row_rect(r: Rect, i: i32) -> Rect {
    Rect::new(r.x + 16, r.y + L_INPUT + 8 + i * L_ROW, r.w - 32, L_ROW - 2)
}

pub fn mouse(l: &mut ShellLauncher, c: &Compositor, x: i32, y: i32, down: bool) -> bool {
    if !l.open {
        return false;
    }
    let r = rect(c);
    if !(x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h) {
        if down {
            close(l);
        }
        return false;
    }
    let n = if l.nq != 0 { l.nhit } else { grid_count(l) };
    for i in 0..n {
        let hit = if l.nq != 0 { row_rect(r, i) } else { tile_rect(r, i) };
        if x >= hit.x && y >= hit.y && x < hit.x + hit.w && y < hit.y + hit.h {
            l.sel = i;
            if down {
                launch_selected(l);
            }
            break;
        }
    }
    true
}

// ---------------------------------------------------------------- drawing

fn body() -> ShellFont {
    ShellFont { face: core::ptr::addr_of!(font_data::loos_font_regular), size: 14 }
}
fn small() -> ShellFont {
    ShellFont { face: core::ptr::addr_of!(font_data::loos_font_regular), size: 12 }
}
fn head() -> ShellFont {
    ShellFont { face: core::ptr::addr_of!(font_data::loos_font_semibold), size: 14 }
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

pub fn draw(l: &ShellLauncher, c: &Compositor, fb: *mut Fb, clip: Rect) {
    if !l.open {
        return;
    }
    let t = &c.theme;
    let r = rect(c);
    unsafe {
        ffi::fb_shadow(fb, clip, r, 12, 28, 8, 0x6000_0000);
        ffi::fb_fill_round(fb, clip, r, 12, c.panel_color());
        ffi::fb_stroke_round(fb, clip, r, 12, 1, t.border);
    }
    let input = Rect::new(r.x + 24, r.y + 20, r.w - 48, 36);
    unsafe {
        ffi::fb_fill_round(fb, clip, input, 18, t.surface_hi);
        ffi::fb_stroke_round(fb, clip, input, 18, 1, if l.nq != 0 { t.accent } else { t.border });
    }
    let ty = input.y + (input.h - 16) / 2;
    if l.nq != 0 {
        text(fb, clip, input.x + 16, ty, &l.query[..l.nq as usize], t.text);
    } else {
        text(fb, clip, input.x + 16, ty, b"Type to search apps, files and settings\0", t.text_dim);
    }
    let caret_x = input.x + 16 + if l.nq != 0 { width(body(), &l.query[..l.nq as usize]) } else { 0 };
    unsafe { ffi::fb_fill(fb, clip, Rect::new(caret_x, ty, 2, 16), t.accent) };
    let name_f = small();
    if l.nq == 0 {
        text_ex(fb, clip, r.x + 32, r.y + L_INPUT + 12, b"Pinned\0", t.text, head());
        let g = grid_items(l);
        let n = grid_count(l);
        for i in 0..n {
            let it = item(l, g[i as usize]);
            let tr = tile_rect(r, i);
            if i == l.sel {
                unsafe { ffi::fb_fill_round(fb, clip, tr, 8, (t.text & 0x00FF_FFFF) | 0x1600_0000) };
            }
            let icon = Rect::new(tr.x + (TILE - 40) / 2, tr.y + 12, 40, 40);
            // SAFETY: the taskbar tile helper only reads the name.
            unsafe { crate::taskbar::shell_draw_app_tile(fb, clip, icon, it.name.as_ptr() as *const c_char, c.tile_radius(40)) };
            let mut nm = [0u8; 48];
            crate::text::ellipsize(&mut nm, bytes0(&it.name), TILE - 8, &font_data::regular(), 14);
            let nw = width(name_f, &nm[..nm.iter().position(|&b| b == 0).unwrap_or(nm.len())]);
            text_ex(fb, clip, tr.x + (TILE - nw) / 2, tr.y + 62, &nm, t.text, name_f);
        }
        if n == 0 {
            text(fb, clip, r.x + 32, r.y + L_INPUT + 44, b"No apps installed yet\0", t.text_dim);
        }
    } else {
        for i in 0..l.nhit {
            let it = item(l, l.hit[i as usize]);
            let row = row_rect(r, i);
            if i == l.sel {
                unsafe { ffi::fb_fill_round(fb, clip, row, 6, (t.accent & 0x00FF_FFFF) | 0x4000_0000) };
            }
            let icon = Rect::new(row.x + 8, row.y + 7, 24, 24);
            unsafe {
                if it.kind == ITEM_APP {
                    crate::taskbar::shell_draw_app_tile(fb, clip, icon, it.name.as_ptr() as *const c_char, c.tile_radius(24));
                } else {
                    ffi::fb_fill_round(fb, clip, icon, if it.kind == ITEM_FILE { 4 } else { 12 }, if it.color != 0 { it.color } else { t.text_dim });
                }
            }
            text(fb, clip, row.x + 44, row.y + (row.h - 16) / 2, bytes0(&it.name), t.text);
            let tag: &[u8] = if it.kind == ITEM_APP {
                b"App\0"
            } else if it.kind == ITEM_FILE {
                b"File\0"
            } else {
                b"Setting\0"
            };
            text_ex(fb, clip, row.x + row.w - width(name_f, tag) - 10, row.y + (row.h - 12) / 2, tag, t.text_dim, name_f);
        }
        if l.nhit == 0 {
            text(fb, clip, r.x + 32, r.y + L_INPUT + 16, b"No local results\0", t.text_dim);
        }
    }
    let ft = Rect::new(r.x + 1, r.y + r.h - 56, r.w - 2, 55);
    unsafe {
        ffi::fb_fill(fb, clip, Rect::new(ft.x, ft.y, ft.w, 1), t.border);
        ffi::fb_fill_round(fb, clip, Rect::new(r.x + 28, ft.y + 14, 28, 28), 14, mix(t.accent, t.surface, 80));
    }
    text(fb, clip, r.x + 66, ft.y + 20, b"Local account\0", t.text);
    text_ex(fb, clip, r.x + r.w - 220, ft.y + 22, b"Enter to open, Esc to close\0", t.text_dim, name_f);
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn launcher_init(l: *mut ShellLauncher, inventory: *const ShellInventory, launch: Option<LaunchFn>, user: *mut c_void) {
    if let Some(l) = unsafe { l.as_mut() } {
        init(l, inventory, launch, user);
    }
}

#[no_mangle]
pub unsafe extern "C" fn launcher_open(l: *mut ShellLauncher) {
    if let Some(l) = unsafe { l.as_mut() } {
        open(l);
    }
}

#[no_mangle]
pub unsafe extern "C" fn launcher_close(l: *mut ShellLauncher) {
    if let Some(l) = unsafe { l.as_mut() } {
        close(l);
    }
}

#[no_mangle]
pub unsafe extern "C" fn launcher_search(l: *mut ShellLauncher) {
    if let Some(l) = unsafe { l.as_mut() } {
        search(l);
    }
}

#[no_mangle]
pub unsafe extern "C" fn launcher_key(l: *mut ShellLauncher, key: u32, mods: u32) -> bool {
    unsafe { l.as_mut() }.map_or(false, |l| self::key(l, key, mods))
}

#[no_mangle]
pub unsafe extern "C" fn launcher_char(l: *mut ShellLauncher, cp: u32) -> bool {
    unsafe { l.as_mut() }.map_or(false, |l| char_event(l, cp))
}

#[no_mangle]
pub unsafe extern "C" fn launcher_rect(c: *const Compositor) -> Rect {
    unsafe { c.as_ref() }.map_or(Rect::new(0, 0, 0, 0), rect)
}

#[no_mangle]
pub unsafe extern "C" fn launcher_draw(l: *const ShellLauncher, c: *const Compositor, fb: *mut Fb, clip: Rect) {
    if let (Some(l), Some(c)) = (unsafe { l.as_ref() }, unsafe { c.as_ref() }) {
        draw(l, c, fb, clip);
    }
}

#[no_mangle]
pub unsafe extern "C" fn launcher_mouse(l: *mut ShellLauncher, c: *const Compositor, x: i32, y: i32, down: bool) -> bool {
    match (unsafe { l.as_mut() }, unsafe { c.as_ref() }) {
        (Some(l), Some(c)) => mouse(l, c, x, y, down),
        _ => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn fuzzy_score(q: *const c_char, name: *const c_char, kw: *const c_char) -> i32 {
    fuzzy(unsafe { ffi::c_bytes(q) }, unsafe { ffi::c_bytes(name) }, unsafe { ffi::c_bytes(kw) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    fn setup() -> (&'static mut ShellInventory, &'static mut ShellLauncher) {
        let inv = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellInventory>() }));
        inventory::init(inv);
        inventory::defaults(inv);
        let l = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellLauncher>() }));
        init(l, inv, None, core::ptr::null_mut());
        (inv, l)
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ShellLauncher>(), 136);
        assert_eq!(offset_of!(ShellLauncher, query), 8);
        assert_eq!(offset_of!(ShellLauncher, nq), 72);
        assert_eq!(offset_of!(ShellLauncher, hit), 76);
        assert_eq!(offset_of!(ShellLauncher, nhit), 108);
        assert_eq!(offset_of!(ShellLauncher, open), 116);
        assert_eq!(offset_of!(ShellLauncher, launch), 120);
        assert_eq!(offset_of!(ShellLauncher, user), 128);
    }

    #[test]
    fn fuzzy_scoring_order() {
        assert_eq!(fuzzy(b"LoCmd", b"LoCmd", b"terminal"), 1000);
        assert_eq!(fuzzy(b"calc", b"LoOS Calculator", b"math"), 595);
        assert!(fuzzy(b"set", b"Privacy", b"telemetry") == 0);
        assert!(fuzzy(b"set", b"Settings", b"") > 0);
        assert!(score_text(b"set", b"Settings", false) == 800);
        assert!(fuzzy(b"note", b"LoOS Notepad", b"") > fuzzy(b"note", b"Privacy", b""));
    }

    #[test]
    fn search_ranks_settings_for_set() {
        let (_inv, l) = setup();
        open(l);
        for &c in b"set" {
            char_event(l, c as u32);
        }
        assert!(l.nhit > 0);
        for i in 0..l.nhit as usize {
            assert_eq!(item(l, l.hit[i]).kind, ITEM_SETTING, "hit {i} should be a settings page");
        }
    }

    #[test]
    fn keyboard_navigation_wraps() {
        let (_inv, l) = setup();
        open(l);
        assert_eq!(l.sel, 0);
        key(l, KEY_RIGHT, 0);
        assert_eq!(l.sel, 1);
        key(l, KEY_LEFT, 0);
        assert_eq!(l.sel, 0);
        key(l, KEY_UP, 0);
        assert_eq!(l.sel, 4);
        key(l, KEY_ENTER, 0);
        assert!(!l.open);
    }
}

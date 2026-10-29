//! Task monitor (C `taskmon.c`): process tree, sockets, CPU/memory and kill confirmation.
#![allow(missing_docs)]

use crate::backend::{NetConn, PsProc, ShellBackend};
use crate::compositor::KEY_ESC;
use crate::draw::Rect;
use crate::ffi::{self, Fb};
use crate::ui::{self, UiCtx};
use crate::util;
use core::ffi::c_void;

pub const TM_MAX: usize = 64;
pub const TM_SORT_NAME: i32 = 0;
pub const TM_SORT_PID: i32 = 1;
pub const TM_SORT_CPU: i32 = 2;
pub const TM_SORT_MEM: i32 = 3;
pub const TM_SORT_THREADS: i32 = 4;
pub const TM_SORT_SOCKETS: i32 = 5;

#[repr(C)]
pub struct ShellTaskmon {
    pub be: *const ShellBackend,
    pub p: [PsProc; TM_MAX],
    pub n: i32,
    pub view: [i32; TM_MAX],
    pub depth: [i32; TM_MAX],
    pub nview: i32,
    pub conns: [NetConn; 32],
    pub nconns: i32,
    pub sel: i32,
    pub scroll: i32,
    pub selected_pid: u32,
    pub sort: i32,
    pub desc: bool,
    pub tree: bool,
    pub filter: [u8; 32],
    pub filter_caret: i32,
    pub confirm_kill: bool,
    pub status: [u8; 96],
    pub total_cpu_x10: u32,
    pub total_mem: u64,
    pub total_sockets: u32,
}

const EMPTY_PROC: PsProc = PsProc { pid: 0, ppid: 0, name: [0; 32], threads: 0, handles: 0, sockets: 0, mem: 0, cpu_x10: 0, io_read: 0, io_write: 0 };

fn bytes0(a: &[u8]) -> &[u8] {
    let n = a.iter().position(|&b| b == 0).unwrap_or(a.len());
    &a[..n]
}
fn lc(c: u8) -> u8 {
    if c.is_ascii_uppercase() { c + 32 } else { c }
}
fn ci_cmp(a: &[u8], b: &[u8]) -> i32 {
    let n = a.len().min(b.len());
    for i in 0..n {
        let (x, y) = (lc(a[i]), lc(b[i]));
        if x != y {
            return x as i32 - y as i32;
        }
    }
    if a.len() == b.len() { 0 } else if a.len() < b.len() { -1 } else { 1 }
}
fn ci_contains(h: &[u8], n: &[u8]) -> bool {
    if n.is_empty() {
        return true;
    }
    if n.len() > h.len() {
        return false;
    }
    for s in 0..=h.len() - n.len() {
        if (0..n.len()).all(|i| lc(h[s + i]) == lc(n[i])) {
            return true;
        }
    }
    false
}

fn cmp_proc(t: &ShellTaskmon, a: &PsProc, b: &PsProc) -> i32 {
    let mut c = match t.sort {
        TM_SORT_PID => if a.pid < b.pid { -1 } else { (a.pid > b.pid) as i32 },
        TM_SORT_CPU => if a.cpu_x10 < b.cpu_x10 { -1 } else { (a.cpu_x10 > b.cpu_x10) as i32 },
        TM_SORT_MEM => if a.mem < b.mem { -1 } else { (a.mem > b.mem) as i32 },
        TM_SORT_THREADS => if a.threads < b.threads { -1 } else { (a.threads > b.threads) as i32 },
        TM_SORT_SOCKETS => if a.sockets < b.sockets { -1 } else { (a.sockets > b.sockets) as i32 },
        _ => ci_cmp(bytes0(&a.name), bytes0(&b.name)),
    };
    if c == 0 {
        c = if a.pid < b.pid { -1 } else { (a.pid > b.pid) as i32 };
    }
    if t.desc { -c } else { c }
}

fn visible(t: &ShellTaskmon, i: usize) -> bool {
    ci_contains(bytes0(&t.p[i].name), bytes0(&t.filter))
}

fn sort_indices(t: &ShellTaskmon, idx: &mut [i32], n: usize) {
    for i in 1..n {
        let v = idx[i];
        let mut j = i;
        while j > 0 && cmp_proc(t, &t.p[idx[j - 1] as usize], &t.p[v as usize]) > 0 {
            idx[j] = idx[j - 1];
            j -= 1;
        }
        idx[j] = v;
    }
}

fn add_subtree(t: &mut ShellTaskmon, root: i32, depth: i32, done: &mut [bool; TM_MAX]) {
    let r = root as usize;
    if done[r] || t.nview as usize >= TM_MAX {
        return;
    }
    done[r] = true;
    t.view[t.nview as usize] = root;
    t.depth[t.nview as usize] = depth;
    t.nview += 1;
    let mut kids = [0i32; TM_MAX];
    let mut nk = 0usize;
    for i in 0..t.n as usize {
        if !done[i] && t.p[i].ppid == t.p[r].pid && t.p[i].pid != t.p[r].pid {
            kids[nk] = i as i32;
            nk += 1;
        }
    }
    sort_indices(t, &mut kids, nk);
    for k in 0..nk {
        add_subtree(t, kids[k], depth + 1, done);
    }
}

fn build_view(t: &mut ShellTaskmon) {
    t.nview = 0;
    if t.filter[0] != 0 || !t.tree {
        let mut idx = [0i32; TM_MAX];
        let mut n = 0usize;
        for i in 0..t.n as usize {
            if visible(t, i) {
                idx[n] = i as i32;
                n += 1;
            }
        }
        sort_indices(t, &mut idx, n);
        for i in 0..n {
            t.view[i] = idx[i];
            t.depth[i] = 0;
        }
        t.nview = n as i32;
    } else {
        let mut done = [false; TM_MAX];
        let mut roots = [0i32; TM_MAX];
        let mut nr = 0usize;
        for i in 0..t.n as usize {
            let mut has_parent = false;
            for k in 0..t.n as usize {
                if t.p[k].pid == t.p[i].ppid && k != i {
                    has_parent = true;
                }
            }
            if !has_parent {
                roots[nr] = i as i32;
                nr += 1;
            }
        }
        sort_indices(t, &mut roots, nr);
        for r in 0..nr {
            add_subtree(t, roots[r], 0, &mut done);
        }
        for i in 0..t.n as usize {
            if !done[i] {
                add_subtree(t, i as i32, 0, &mut done);
            }
        }
    }
    t.sel = 0;
    for i in 0..t.nview as usize {
        if t.p[t.view[i] as usize].pid == t.selected_pid {
            t.sel = i as i32;
        }
    }
    if t.nview != 0 {
        t.selected_pid = t.p[t.view[t.sel as usize] as usize].pid;
    } else {
        t.selected_pid = 0;
    }
}

pub fn refresh(t: &mut ShellTaskmon) {
    let be = unsafe { &*t.be };
    t.n = be.ps_snapshot(t.p.as_mut_ptr(), TM_MAX as i32);
    if t.n < 0 {
        t.n = 0;
    }
    t.nconns = be.net_query(t.conns.as_mut_ptr(), 32);
    if t.nconns < 0 {
        t.nconns = 0;
    }
    t.total_cpu_x10 = 0;
    t.total_mem = 0;
    t.total_sockets = 0;
    for i in 0..t.n as usize {
        let mut s = 0;
        for k in 0..t.nconns as usize {
            if t.conns[k].pid == t.p[i].pid {
                s += 1;
            }
        }
        t.p[i].sockets = s;
        t.total_cpu_x10 += t.p[i].cpu_x10;
        t.total_mem += t.p[i].mem;
        t.total_sockets += s;
    }
    build_view(t);
}

pub fn init(t: &mut ShellTaskmon, be: *const ShellBackend) {
    t.be = be;
    t.p = [EMPTY_PROC; TM_MAX];
    t.n = 0;
    t.view = [0; TM_MAX];
    t.depth = [0; TM_MAX];
    t.nview = 0;
    t.conns = [NetConn::EMPTY; 32];
    t.nconns = 0;
    t.sel = 0;
    t.scroll = 0;
    t.selected_pid = 0;
    t.sort = TM_SORT_NAME;
    t.desc = false;
    t.tree = true;
    t.filter = [0; 32];
    t.filter_caret = 0;
    t.confirm_kill = false;
    t.status = [0; 96];
    t.total_cpu_x10 = 0;
    t.total_mem = 0;
    t.total_sockets = 0;
    refresh(t);
}

pub fn set_sort(t: &mut ShellTaskmon, s: i32) {
    if t.sort == s {
        t.desc = !t.desc;
    } else {
        t.sort = s;
        t.desc = s != TM_SORT_NAME && s != TM_SORT_PID;
    }
    build_view(t);
}

pub fn selected(t: &ShellTaskmon) -> Option<usize> {
    if t.nview != 0 && t.sel >= 0 && t.sel < t.nview {
        Some(t.view[t.sel as usize] as usize)
    } else {
        None
    }
}

fn put(dst: &mut [u8], src: &[u8]) {
    dst.fill(0);
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
}

pub fn kill_selected(t: &mut ShellTaskmon) -> bool {
    let Some(i) = selected(t) else { return false };
    let name = t.p[i].name;
    let pid = t.p[i].pid;
    let be = unsafe { &*t.be };
    if !be.ps_kill(pid) {
        let mut b = [0u8; 96];
        let mut k = 0;
        for &c in b"Cannot end " { b[k] = c; k += 1; }
        for &c in bytes0(&name) { b[k] = c; k += 1; }
        for &c in b" (" { b[k] = c; k += 1; }
        let mut tmp = [0u8; 12]; let mut d = 0; let mut v = pid;
        if v == 0 { tmp[0] = b'0'; d = 1; }
        while v != 0 { tmp[d] = b'0' + (v % 10) as u8; d += 1; v /= 10; }
        for i in (0..d).rev() { b[k] = tmp[i]; k += 1; }
        for &c in b"): protected or already gone" { b[k] = c; k += 1; }
        put(&mut t.status, &b[..k]);
        return false;
    }
    let mut b = [0u8; 96];
    let mut k = 0;
    for &c in b"Ended " { b[k] = c; k += 1; }
    for &c in bytes0(&name) { b[k] = c; k += 1; }
    for &c in b" (" { b[k] = c; k += 1; }
    let mut tmp = [0u8; 12]; let mut d = 0; let mut v = pid;
    if v == 0 { tmp[0] = b'0'; d = 1; }
    while v != 0 { tmp[d] = b'0' + (v % 10) as u8; d += 1; v /= 10; }
    for i in (0..d).rev() { b[k] = tmp[i]; k += 1; }
    b[k] = b')'; k += 1;
    put(&mut t.status, &b[..k]);
    refresh(t);
    true
}

pub fn proc_sockets(t: &ShellTaskmon, pid: u32, out: &mut [NetConn]) -> i32 {
    let mut n = 0;
    for i in 0..t.nconns as usize {
        if n >= out.len() { break; }
        if t.conns[i].pid == pid {
            out[n] = t.conns[i];
            n += 1;
        }
    }
    n as i32
}

fn num_str(out: &mut [u8], mut v: u32) {
    let mut tmp = [0u8; 12];
    let mut n = 0;
    if v == 0 { tmp[0] = b'0'; n = 1; }
    while v != 0 { tmp[n] = b'0' + (v % 10) as u8; n += 1; v /= 10; }
    for i in (0..n).rev() { out[n - 1 - i] = tmp[i]; }
    out[n] = 0;
}

unsafe extern "C" fn row_proc(u: *mut UiCtx, row: Rect, i: i32, sel: bool, user: *mut c_void) {
    let (Some(u), Some(t)) = (unsafe { u.as_mut() }, unsafe { (user as *mut ShellTaskmon).as_mut() }) else { return };
    let p = t.p[t.view[i as usize] as usize];
    let th = unsafe { &*u.theme };
    let tc = if sel { th.accent_text } else { th.text };
    let dc = if sel { th.accent_text } else { th.text_dim };
    let mut b = [0u8; 32];
    let indent = t.depth[i as usize] * 16;
    if t.tree && t.filter[0] == 0 {
        let mut has_kids = false;
        for k in 0..t.n as usize {
            if t.p[k].ppid == p.pid && t.p[k].pid != p.pid {
                has_kids = true;
            }
        }
        unsafe {
            if indent != 0 {
                ffi::fb_fill(user_of(u), u.clip, Rect::new(row.x + 8 + indent - 8, row.y + 10, 6, 1), th.border);
            }
            if has_kids {
                ffi::fb_fill(user_of(u), u.clip, Rect::new(row.x + 8 + indent - 2, row.y + 6, 5, 5), th.text_dim);
            }
        }
    }
    ui::label(u, row.x + 14 + indent, row.y + 3, bytes0(&p.name), tc);
    num_str(&mut b, p.pid);
    ui::label_right(u, row.x + row.w - 400, row.y + 3, bytes0(&b), dc);
    let mut c = [0u8; 24];
    util::pct10(p.cpu_x10, &mut c);
    ui::label_right(u, row.x + row.w - 320, row.y + 3, bytes0(&c), if p.cpu_x10 >= 500 { th.danger } else { dc });
    util::size(p.mem, &mut b);
    ui::label_right(u, row.x + row.w - 220, row.y + 3, bytes0(&b), dc);
    num_str(&mut b, p.threads);
    ui::label_right(u, row.x + row.w - 130, row.y + 3, bytes0(&b), dc);
    num_str(&mut b, p.sockets);
    ui::label_right(u, row.x + row.w - 20, row.y + 3, bytes0(&b), if p.sockets != 0 { 0xFFF5_9E0B } else { dc });
}

fn user_of(u: &UiCtx) -> *mut Fb {
    u.fb
}

pub fn frame(t: &mut ShellTaskmon, u: &mut UiCtx, w: i32, h: i32) {
    let th = unsafe { *u.theme };
    ui::clear(u, th.surface);
    let modal = t.confirm_kill;
    let mut parked = [crate::ui::KeyInput::EMPTY; crate::ui::UI_KEYQ];
    let mut np = 0;
    let (mut pp, mut pr, mut pw) = (false, false, 0);
    if modal {
        np = u.nkeys as usize;
        parked[..np].copy_from_slice(&u.keys[..np]);
        pp = u.pressed;
        pr = u.released;
        pw = u.wheel;
        u.nkeys = 0;
        u.pressed = false;
        u.released = false;
        u.wheel = 0;
    }
    let mut b = [0u8; 192];
    let mut c1 = [0u8; 16];
    let mut c2 = [0u8; 24];
    util::pct10(t.total_cpu_x10.min(1000), &mut c1);
    util::size(t.total_mem, &mut c2);
    let mut k = 0;
    for &c in b"Processes " { b[k] = c; k += 1; }
    let mut tmp = [0u8; 12];
    let mut v = t.n.max(0) as u32;
    let mut d = 0;
    if v == 0 { tmp[0] = b'0'; d = 1; }
    while v != 0 { tmp[d] = b'0' + (v % 10) as u8; d += 1; v /= 10; }
    for i in (0..d).rev() { b[k] = tmp[i]; k += 1; }
    for &c in b"    CPU " { b[k] = c; k += 1; }
    for &c in bytes0(&c1) { b[k] = c; k += 1; }
    for &c in b"    Memory " { b[k] = c; k += 1; }
    for &c in bytes0(&c2) { b[k] = c; k += 1; }
    for &c in b"    Sockets " { b[k] = c; k += 1; }
    let mut sv = t.total_sockets;
    let mut sd = 0;
    if sv == 0 { tmp[0] = b'0'; sd = 1; }
    while sv != 0 { tmp[sd] = b'0' + (sv % 10) as u8; sd += 1; sv /= 10; }
    for i in (0..sd).rev() { b[k] = tmp[i]; k += 1; }
    ui::label(u, 12, 12, bytes0(&b), th.text);
    let tree_before = t.tree;
    ui::toggle(u, Rect::new(w - 360, 8, 150, 26), b"Tree", &mut t.tree);
    if tree_before != t.tree {
        build_view(t);
    }
    let before = t.filter;
    ui::textfield(u, Rect::new(w - 200, 6, 190, 28), &mut t.filter, &mut t.filter_caret, b"Search");
    if before != t.filter {
        build_view(t);
    }
    let top = 44;
    let bottom = h - 170;
    let hdr = Rect::new(8, top, w - 16, 24);
    const NAMES: [&[u8]; 6] = [b"Name\0", b"PID\0", b"CPU\0", b"Memory\0", b"Threads\0", b"Sockets\0"];
    let rx = [22, w - 16 - 400, w - 16 - 320, w - 16 - 220, w - 16 - 130, w - 16 - 20];
    for i in 0..6 {
        let mut lb = [0u8; 24];
        let mut n = 0;
        for &c in bytes0(NAMES[i]) { lb[n] = c; n += 1; }
        if t.sort == i as i32 {
            for &c in if t.desc { b" v" } else { b" ^" } { lb[n] = c; n += 1; }
        }
        lb[n] = 0;
        if i == 0 { ui::label(u, 8 + rx[i], top + 4, bytes0(&lb), th.text_dim); }
        else { ui::label_right(u, 8 + rx[i], top + 4, bytes0(&lb), th.text_dim); }
    }
    if u.pressed && ui::hovered(u, hdr) {
        let x = u.mx - 8;
        let mut pick = 0;
        for i in 1..6 {
            if x > rx[i] - 70 {
                pick = i;
            }
        }
        set_sort(t, pick as i32);
    }
    unsafe { ffi::fb_hline(u.fb, u.clip, 8, top + 24, w - 16, th.border) };
    ui::list(u, Rect::new(8, top + 26, w - 16, bottom - top - 26), b"procs", t.nview, 22, Some(row_proc), t as *mut ShellTaskmon as *mut c_void, &mut t.sel, &mut t.scroll);
    if t.nview != 0 && t.sel < t.nview {
        t.selected_pid = t.p[t.view[t.sel as usize] as usize].pid;
    }
    let sel = selected(t);
    let dy = bottom + 10;
    unsafe { ffi::fb_hline(u.fb, u.clip, 8, dy - 4, w - 16, th.border) };
    if let Some(si) = sel {
        let p = t.p[si];
        let mut io1 = [0u8; 24];
        let mut io2 = [0u8; 24];
        util::size(p.io_read, &mut io1);
        util::size(p.io_write, &mut io2);
        let mut line = [0u8; 192];
        let mut n = 0;
        let push = |s: &[u8], line: &mut [u8; 192], n: &mut usize| {
            for &c in s { if *n < 191 { line[*n] = c; *n += 1; } }
        };
        push(bytes0(&p.name), &mut line, &mut n);
        push(b"  pid ", &mut line, &mut n);
        let mut t2 = [0u8; 12]; num_str(&mut t2, p.pid); push(bytes0(&t2), &mut line, &mut n);
        push(b"  parent ", &mut line, &mut n); num_str(&mut t2, p.ppid); push(bytes0(&t2), &mut line, &mut n);
        push(b"  handles ", &mut line, &mut n); num_str(&mut t2, p.handles); push(bytes0(&t2), &mut line, &mut n);
        push(b"  disk read ", &mut line, &mut n); push(bytes0(&io1), &mut line, &mut n);
        push(b"  write ", &mut line, &mut n); push(bytes0(&io2), &mut line, &mut n);
        ui::label(u, 12, dy, bytes0(&line), th.text);
        let mut sk = [NetConn::EMPTY; 8];
        let ns = proc_sockets(t, p.pid, &mut sk);
        if ns == 0 {
            ui::label(u, 12, dy + 24, b"No open sockets.", th.text_dim);
        }
        for i in 0..ns.min(4) as usize {
            let mut s = [0u8; 96];
            let mut n = 0;
            let push = |x: &[u8], s: &mut [u8; 96], n: &mut usize| { for &c in x { if *n < 95 { s[*n] = c; *n += 1; } } };
            push(bytes0(&sk[i].proto), &mut s, &mut n);
            push(b" ", &mut s, &mut n);
            push(bytes0(&sk[i].remote), &mut s, &mut n);
            push(b":", &mut s, &mut n);
            let mut t2 = [0u8; 12]; num_str(&mut t2, sk[i].port as u32); push(bytes0(&t2), &mut s, &mut n);
            push(b"  ", &mut s, &mut n);
            push(bytes0(&sk[i].state), &mut s, &mut n);
            ui::label(u, 12, dy + 24 + i as i32 * 18, bytes0(&s), 0xFFF5_9E0B);
        }
        if ui::button_accent(u, Rect::new(w - 140, dy + 4, 128, 30), b"End task") {
            t.confirm_kill = true;
        }
    } else {
        ui::label(u, 12, dy, b"Select a process.", th.text_dim);
    }
    ui::label(u, 12, h - 24, bytes0(&t.status), th.text_dim);
    if modal {
        u.keys[..np].copy_from_slice(&parked[..np]);
        u.nkeys = np as i32;
        u.pressed = pp;
        u.released = pr;
        u.wheel = pw;
        let m = Rect::new(w / 2 - 220, h / 2 - 50, 440, 100);
        unsafe {
            ffi::fb_fill_round(u.fb, u.clip, Rect::new(m.x - 2, m.y - 2, m.w + 4, m.h + 4), 10, th.border);
            ffi::fb_fill_round(u.fb, u.clip, m, 8, th.surface_hi);
        }
        let k = selected(t);
        let mut line = [0u8; 192];
        let mut n = 0;
        let push = |s: &[u8], line: &mut [u8; 192], n: &mut usize| { for &c in s { if *n < 191 { line[*n] = c; *n += 1; } } };
        push(b"End ", &mut line, &mut n);
        if let Some(i) = k { push(bytes0(&t.p[i].name), &mut line, &mut n); } else { push(b"?", &mut line, &mut n); }
        push(b" (", &mut line, &mut n);
        let mut t2 = [0u8; 12]; num_str(&mut t2, k.map_or(0, |i| t.p[i].pid)); push(bytes0(&t2), &mut line, &mut n);
        push(b")? Unsaved data in it is lost.", &mut line, &mut n);
        ui::label(u, m.x + 16, m.y + 16, bytes0(&line), th.text);
        if ui::button_accent(u, Rect::new(m.x + 16, m.y + 56, 120, 30), b"End task") {
            kill_selected(t);
            t.confirm_kill = false;
        }
        if ui::button(u, Rect::new(m.x + 144, m.y + 56, 120, 30), b"Cancel") || ui::take_key(u, KEY_ESC, 0) {
            t.confirm_kill = false;
        }
    }
}

// ---------------------------------------------------------------- C ABI

#[no_mangle]
pub unsafe extern "C" fn tm_init(t: *mut ShellTaskmon, be: *const ShellBackend) {
    if let Some(t) = unsafe { t.as_mut() } {
        init(t, be);
    }
}
#[no_mangle]
pub unsafe extern "C" fn tm_refresh(t: *mut ShellTaskmon) {
    if let Some(t) = unsafe { t.as_mut() } {
        refresh(t);
    }
}
#[no_mangle]
pub unsafe extern "C" fn tm_set_sort(t: *mut ShellTaskmon, s: i32) {
    if let Some(t) = unsafe { t.as_mut() } {
        set_sort(t, s);
    }
}
#[no_mangle]
pub unsafe extern "C" fn tm_selected(t: *const ShellTaskmon) -> *const PsProc {
    match unsafe { t.as_ref() } {
        Some(t) => match selected(t) {
            Some(i) => &t.p[i],
            None => core::ptr::null(),
        },
        None => core::ptr::null(),
    }
}
#[no_mangle]
pub unsafe extern "C" fn tm_kill_selected(t: *mut ShellTaskmon) -> bool {
    unsafe { t.as_mut() }.map_or(false, kill_selected)
}
#[no_mangle]
pub unsafe extern "C" fn tm_proc_sockets(t: *const ShellTaskmon, pid: u32, out: *mut NetConn, max: i32) -> i32 {
    let Some(t) = (unsafe { t.as_ref() }) else { return 0 };
    if out.is_null() || max <= 0 {
        return 0;
    }
    // SAFETY: caller provides max entries.
    let o = unsafe { core::slice::from_raw_parts_mut(out, max as usize) };
    proc_sockets(t, pid, o)
}
#[no_mangle]
pub unsafe extern "C" fn tm_frame(t: *mut ShellTaskmon, u: *mut UiCtx, w: i32, h: i32) {
    if let (Some(t), Some(u)) = (unsafe { t.as_mut() }, unsafe { u.as_mut() }) {
        frame(t, u, w, h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::PsProc;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    fn proc(pid: u32, ppid: u32, name: &[u8], cpu: u32) -> PsProc {
        let mut p = EMPTY_PROC;
        p.pid = pid;
        p.ppid = ppid;
        p.cpu_x10 = cpu;
        let n = name.len().min(31);
        p.name[..n].copy_from_slice(&name[..n]);
        p
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ShellTaskmon>(), 9416);
        assert_eq!(offset_of!(ShellTaskmon, p), 8);
        assert_eq!(offset_of!(ShellTaskmon, n), 5640);
        assert_eq!(offset_of!(ShellTaskmon, view), 5644);
        assert_eq!(offset_of!(ShellTaskmon, nview), 6156);
        assert_eq!(offset_of!(ShellTaskmon, conns), 6160);
        assert_eq!(offset_of!(ShellTaskmon, sel), 9236);
        assert_eq!(offset_of!(ShellTaskmon, selected_pid), 9244);
        assert_eq!(offset_of!(ShellTaskmon, sort), 9248);
        assert_eq!(offset_of!(ShellTaskmon, status), 9293);
    }

    fn t() -> &'static mut ShellTaskmon {
        let t = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellTaskmon>() }));
        t.be = core::ptr::null();
        t.p = [EMPTY_PROC; TM_MAX];
        t.tree = true;
        t.sort = TM_SORT_NAME;
        t
    }

    #[test]
    fn sorts_flat_and_builds_the_tree() {
        let t = t();
        t.p[0] = proc(1, 0, b"init", 0);
        t.p[1] = proc(2, 1, b"shell", 30);
        t.p[2] = proc(3, 1, b"notepad", 10);
        t.p[3] = proc(4, 2, b"worker", 0);
        t.n = 4;
        build_view(t);
        assert_eq!(t.nview, 4);
        // Tree: init > notepad, shell > worker (children sorted by name).
        assert_eq!(t.p[t.view[0] as usize].pid, 1);
        assert_eq!(t.depth[0], 0);
        assert_eq!(t.p[t.view[1] as usize].pid, 3);
        assert_eq!(t.depth[1], 1);
        assert_eq!(t.p[t.view[2] as usize].pid, 2);
        assert_eq!(t.depth[2], 1);
        assert_eq!(t.p[t.view[3] as usize].pid, 4);
        assert_eq!(t.depth[3], 2);
        // Flat CPU sort: shell first.
        t.tree = false;
        t.sort = TM_SORT_CPU;
        t.desc = true;
        build_view(t);
        assert_eq!(t.p[t.view[0] as usize].pid, 2);
    }

    #[test]
    fn filter_and_selection() {
        let t = t();
        t.p[0] = proc(1, 0, b"system", 0);
        t.p[1] = proc(2, 1, b"notepad", 0);
        t.n = 2;
        t.selected_pid = 2;
        t.filter[..4].copy_from_slice(b"note");
        build_view(t);
        assert_eq!(t.nview, 1);
        assert_eq!(selected(t), Some(1));
        assert_eq!(t.selected_pid, 2);
    }

    #[test]
    fn ci_helpers() {
        assert!(ci_contains(b"LoOS Browser", b"browser"));
        assert!(!ci_contains(b"abc", b"abcd"));
        assert_eq!(ci_cmp(b"abc", b"ABC"), 0);
        assert!(ci_cmp(b"abc", b"abd") < 0);
    }
}

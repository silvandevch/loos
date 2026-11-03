//! In-process reference backend (was `src/shell/backend_mem.c`): config store, in-memory filesystem, process list,
//! firewall and permission tables behind the 24-entry [`ShellBackend`] vtable. Used by the desktop until the real
//! services exist, and by every host test. Nothing here touches the network or the disk.
#![allow(missing_docs)]

use crate::backend::{FsEntry, FsVolume, FwRule, NetConn, PsProc, ShellBackend};
use crate::ffi::c_bytes;
use alloc::vec::Vec;
use core::ffi::{c_char, c_void};

pub const MAX_CONN: usize = 64;
pub const MAX_RULES: usize = 64;
pub const MAX_APPS: usize = 32;
pub const MAX_CFG: usize = 256;
pub const MAX_NODES: usize = 1024;
pub const MAX_FILES: usize = 64;
pub const FILE_BYTES: usize = 16384;
pub const MAX_PROCS: usize = 64;
pub const RES_COUNT: i32 = 5;
pub const FSA_READONLY: u32 = 2;

pub const PERM_RES_NAMES: [&str; 5] = ["Camera", "Microphone", "Files", "Notifications", "Location"];

struct Node {
    path: Vec<u8>,
    dir: bool,
    size: u64,
    mtime: i64,
    attrs: u32,
}
struct File {
    path: Vec<u8>,
    data: Vec<u8>,
}

/// The reference backend state. `vtable()` hands the shell a [`ShellBackend`] that points back at it.
pub struct MemBackend {
    conn: Vec<NetConn>,
    rule: Vec<FwRule>,
    hard_off: bool,
    app: Vec<[u8; 32]>,
    perm: Vec<u8>,
    decided: Vec<u8>,
    cfg: Vec<([u8; 40], [u8; 96])>,
    node: Vec<Node>,
    vol: Vec<FsVolume>,
    proc_: Vec<PsProc>,
    file: Vec<File>,
}

fn copy_str<const N: usize>(dst: &mut [u8; N], src: &[u8]) {
    dst.fill(0);
    let n = src.len().min(N - 1);
    dst[..n].copy_from_slice(&src[..n]);
}
fn nul(a: &[u8]) -> &[u8] {
    &a[..a.iter().position(|&b| b == 0).unwrap_or(a.len())]
}
fn norm(p: &[u8]) -> Vec<u8> {
    let mut n = p.len();
    while n > 1 && p[n - 1] == b'/' {
        n -= 1;
    }
    p[..n.min(159)].to_vec()
}
fn base(p: &[u8]) -> &[u8] {
    match p.iter().rposition(|&c| c == b'/') {
        Some(i) => &p[i + 1..],
        None => p,
    }
}
fn is_child(dir: &[u8], p: &[u8]) -> bool {
    if dir == b"/" {
        return p.len() > 1 && p[0] == b'/' && !p[1..].contains(&b'/');
    }
    p.len() > dir.len() + 1 && p.starts_with(dir) && p[dir.len()] == b'/' && !p[dir.len() + 1..].contains(&b'/')
}
fn under(p: &[u8], root: &[u8]) -> bool {
    p == root || (p.starts_with(root) && p.get(root.len()) == Some(&b'/'))
}

impl Default for MemBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MemBackend {
    pub const fn new() -> Self {
        Self {
            conn: Vec::new(),
            rule: Vec::new(),
            hard_off: false,
            app: Vec::new(),
            perm: Vec::new(),
            decided: Vec::new(),
            cfg: Vec::new(),
            node: Vec::new(),
            vol: Vec::new(),
            proc_: Vec::new(),
            file: Vec::new(),
        }
    }

    /// The vtable the shell talks to. `self` must stay at this address while the vtable is in use.
    pub fn vtable(&mut self) -> ShellBackend {
        macro_rules! fp {
            ($f:expr) => {
                $f as usize as *mut c_void
            };
        }
        ShellBackend {
            user: self as *mut Self as *mut c_void,
            funcs: [
                fp!(net_query), fp!(fw_list), fp!(fw_set), fp!(fw_check), fp!(hard_get), fp!(hard_set),
                fp!(perm_apps), fp!(perm_get), fp!(perm_set), fp!(perm_state), fp!(perm_reset),
                fp!(fs_list), fp!(fs_stat), fp!(fs_mkdir), fp!(fs_remove), fp!(fs_rename), fp!(fs_copy),
                fp!(fs_read), fp!(fs_write), fp!(fs_volumes), fp!(ps_snapshot), fp!(ps_kill),
                fp!(cfg_get), fp!(cfg_set),
            ],
        }
    }

    // ---- config
    fn cfg_find(&self, k: &[u8]) -> Option<usize> {
        self.cfg.iter().position(|(key, _)| nul(key) == k)
    }
    pub fn set_cfg(&mut self, k: &[u8], v: &[u8]) -> bool {
        if k.len() >= 40 || v.len() >= 96 {
            return false;
        }
        let i = match self.cfg_find(k) {
            Some(i) => i,
            None => {
                if self.cfg.len() >= MAX_CFG {
                    return false;
                }
                let mut key = [0u8; 40];
                copy_str(&mut key, k);
                self.cfg.push((key, [0; 96]));
                self.cfg.len() - 1
            }
        };
        copy_str(&mut self.cfg[i].1, v);
        true
    }
    pub fn get_cfg(&self, k: &[u8]) -> Option<&[u8]> {
        self.cfg_find(k).map(|i| nul(&self.cfg[i].1))
    }

    // ---- population helpers (also used by tests)
    pub fn add_conn(&mut self, proc_: &str, pid: u32, remote: &str, port: u16, proto: &str, state: &str) {
        if self.conn.len() >= MAX_CONN {
            return;
        }
        let mut c = NetConn::EMPTY;
        copy_str(&mut c.proc, proc_.as_bytes());
        copy_str(&mut c.remote, remote.as_bytes());
        copy_str(&mut c.proto, proto.as_bytes());
        copy_str(&mut c.state, state.as_bytes());
        c.pid = pid;
        c.port = port;
        self.conn.push(c);
    }
    pub fn add_app(&mut self, name: &str) -> bool {
        if self.app.len() >= MAX_APPS || self.app_index(name.as_bytes()).is_some() {
            return false;
        }
        let mut a = [0u8; 32];
        copy_str(&mut a, name.as_bytes());
        self.app.push(a);
        self.perm.push(0); // default deny: every grant is an explicit user action
        self.decided.push(0);
        true
    }
    pub fn add_node(&mut self, path: &str, dir: bool, size: u64, mtime: i64, attrs: u32) -> bool {
        self.add_node_b(&norm(path.as_bytes()), dir, size, mtime, attrs)
    }
    fn add_node_b(&mut self, p: &[u8], dir: bool, size: u64, mtime: i64, attrs: u32) -> bool {
        if self.node.len() >= MAX_NODES || self.node_find(p).is_some() {
            return false;
        }
        self.node.push(Node { path: p.to_vec(), dir, size: if dir { 0 } else { size }, mtime, attrs });
        true
    }
    pub fn add_volume(&mut self, mount: &str, label: &str, fstype: &str, total: u64, avail: u64, ro: bool) {
        if self.vol.len() >= 4 {
            return;
        }
        let mut v = FsVolume::EMPTY;
        copy_str(&mut v.mount, mount.as_bytes());
        copy_str(&mut v.label, label.as_bytes());
        copy_str(&mut v.fstype, fstype.as_bytes());
        v.total = total;
        v.avail = avail;
        v.readonly = ro;
        self.vol.push(v);
    }
    pub fn add_proc(&mut self, pid: u32, ppid: u32, name: &str, threads: u32, mem: u64, cpu_x10: u32) {
        if self.proc_.len() >= MAX_PROCS {
            return;
        }
        let mut p = PsProc { pid, ppid, name: [0; 32], threads, handles: threads * 12, sockets: 0, mem, cpu_x10, io_read: 0, io_write: 0 };
        copy_str(&mut p.name, name.as_bytes());
        self.proc_.push(p);
    }
    /// Convenience used by the seed: create or replace a file with text contents.
    pub fn put_file(&mut self, path: &str, data: &[u8]) -> bool {
        self.write(&norm(path.as_bytes()), data)
    }

    // ---- filesystem
    fn node_find(&self, p: &[u8]) -> Option<usize> {
        self.node.iter().position(|n| n.path == p)
    }
    fn file_find(&self, p: &[u8]) -> Option<usize> {
        self.file.iter().position(|f| f.path == p)
    }
    fn parent_is_dir(&self, p: &[u8]) -> bool {
        match p.iter().rposition(|&c| c == b'/') {
            None => false,
            Some(0) => true,
            Some(i) => self.node_find(&p[..i]).map_or(false, |n| self.node[n].dir),
        }
    }
    fn entry(&self, i: usize) -> FsEntry {
        let n = &self.node[i];
        let mut e = FsEntry::EMPTY;
        copy_str(&mut e.name, base(&n.path));
        e.is_dir = n.dir;
        e.size = n.size;
        e.mtime = n.mtime;
        e.attrs = n.attrs;
        e
    }
    fn write(&mut self, p: &[u8], data: &[u8]) -> bool {
        if data.len() > FILE_BYTES || !self.parent_is_dir(p) {
            return false;
        }
        let n = self.node_find(p);
        if let Some(n) = n {
            if self.node[n].dir || self.node[n].attrs & FSA_READONLY != 0 {
                return false;
            }
        }
        let f = match self.file_find(p) {
            Some(f) => f,
            None => {
                if self.file.len() >= MAX_FILES {
                    return false;
                }
                self.file.push(File { path: p.to_vec(), data: Vec::new() });
                self.file.len() - 1
            }
        };
        if n.is_none() && !self.add_node_b(p, false, 0, 1_700_000_000, 0) {
            self.file.pop();
            return false;
        }
        let n = self.node_find(p).unwrap_or(0);
        self.file[f].data = data.to_vec();
        self.node[n].size = data.len() as u64;
        true
    }
    fn app_index(&self, a: &[u8]) -> Option<usize> {
        self.app.iter().position(|n| nul(n) == a)
    }

    // ---- persistence: plain text, local only
    pub fn save(&self) -> Vec<u8> {
        let mut o: Vec<u8> = Vec::new();
        let line = |o: &mut Vec<u8>, k: &[u8], v: &[u8]| {
            o.extend_from_slice(k);
            o.push(b'=');
            o.extend_from_slice(v);
            o.push(b'\n');
        };
        o.extend_from_slice(b"[config]\n");
        for (k, v) in &self.cfg {
            line(&mut o, nul(k), nul(v));
        }
        o.extend_from_slice(b"[firewall]\n");
        for r in &self.rule {
            line(&mut o, nul(&r.proc), if r.action == 1 { b"allow" } else { b"deny" });
        }
        o.extend_from_slice(b"[perms]\n");
        for i in 0..self.app.len() {
            let v = self.perm[i] as u32 | (self.decided[i] as u32) << 8;
            let mut b = [0u8; 12];
            let n = fmt_u32(&mut b, v);
            line(&mut o, nul(&self.app[i]), &b[..n]);
        }
        o
    }
    /// Returns the number of entries read.
    pub fn load(&mut self, text: &[u8]) -> i32 {
        let (mut sect, mut n) = (0, 0);
        for ln in text.split(|&c| c == b'\n') {
            if ln.is_empty() || ln[0] == b'#' {
                continue;
            }
            if ln[0] == b'[' {
                sect = if ln.starts_with(b"[config]") { 1 } else if ln.starts_with(b"[firewall]") { 2 } else if ln.starts_with(b"[perms]") { 3 } else { 0 };
                continue;
            }
            let Some(eq) = ln.iter().position(|&c| c == b'=') else { continue };
            if sect == 0 {
                continue;
            }
            let (k, v) = (&ln[..eq], &ln[eq + 1..]);
            match sect {
                1 => {
                    if self.set_cfg(k, v) {
                        n += 1;
                    }
                }
                2 => {
                    if self.rule_set(k, if v == b"allow" { 1 } else { 2 }) {
                        n += 1;
                    }
                }
                _ => {
                    if let Ok(ks) = core::str::from_utf8(k) {
                        self.add_app(ks);
                    }
                    if let Some(i) = self.app_index(k) {
                        let val = crate::backend::atoi(v) as u32;
                        self.perm[i] = (val & 0xff) as u8;
                        self.decided[i] = ((val >> 8) & 0xff) as u8;
                        if self.decided[i] == 0 {
                            self.decided[i] = self.perm[i];
                        }
                        n += 1;
                    }
                }
            }
        }
        n
    }
    fn rule_set(&mut self, proc_: &[u8], action: i32) -> bool {
        if let Some(i) = self.rule.iter().position(|r| nul(&r.proc) == proc_) {
            if action == 0 {
                self.rule.swap_remove(i);
            } else {
                self.rule[i].action = action;
            }
            return true;
        }
        if action == 0 {
            return true;
        }
        if self.rule.len() >= MAX_RULES {
            return false;
        }
        let mut r = FwRule::EMPTY;
        copy_str(&mut r.proc, proc_);
        r.action = action;
        self.rule.push(r);
        true
    }
}

fn fmt_u32(out: &mut [u8; 12], mut v: u32) -> usize {
    let mut tmp = [0u8; 10];
    let mut d = 0;
    if v == 0 {
        tmp[0] = b'0';
        d = 1;
    }
    while v != 0 {
        tmp[d] = b'0' + (v % 10) as u8;
        d += 1;
        v /= 10;
    }
    for i in 0..d {
        out[i] = tmp[d - 1 - i];
    }
    d
}

// ---------------------------------------------------------------- vtable entries
unsafe fn m<'a>(u: *mut c_void) -> &'a mut MemBackend {
    // SAFETY: `user` is the MemBackend that built the vtable (see `vtable`).
    unsafe { &mut *(u as *mut MemBackend) }
}
unsafe fn out_slice<'a, T>(p: *mut T, max: i32) -> &'a mut [T] {
    if p.is_null() || max <= 0 {
        &mut []
    } else {
        // SAFETY: the shell passes a buffer of `max` elements.
        unsafe { core::slice::from_raw_parts_mut(p, max as usize) }
    }
}

unsafe extern "C" fn net_query(u: *mut c_void, out: *mut NetConn, max: i32) -> i32 {
    let (m, o) = unsafe { (m(u), out_slice(out, max)) };
    let n = m.conn.len().min(o.len());
    o[..n].copy_from_slice(&m.conn[..n]);
    n as i32
}
unsafe extern "C" fn fw_list(u: *mut c_void, out: *mut FwRule, max: i32) -> i32 {
    let (m, o) = unsafe { (m(u), out_slice(out, max)) };
    let n = m.rule.len().min(o.len());
    o[..n].copy_from_slice(&m.rule[..n]);
    n as i32
}
unsafe extern "C" fn fw_set(u: *mut c_void, proc_: *const c_char, a: i32) -> bool {
    unsafe { m(u) }.rule_set(unsafe { c_bytes(proc_) }, a)
}
unsafe extern "C" fn fw_check(u: *mut c_void, proc_: *const c_char) -> i32 {
    let m = unsafe { m(u) };
    if m.hard_off {
        return 2;
    }
    let p = unsafe { c_bytes(proc_) };
    m.rule.iter().find(|r| nul(&r.proc) == p).map_or(0, |r| r.action)
}
unsafe extern "C" fn hard_get(u: *mut c_void) -> bool {
    unsafe { m(u) }.hard_off
}
unsafe extern "C" fn hard_set(u: *mut c_void, off: bool) {
    unsafe { m(u) }.hard_off = off;
}
unsafe extern "C" fn perm_apps(u: *mut c_void, names: *mut u8, max: i32) -> i32 {
    let m = unsafe { m(u) };
    let n = m.app.len().min(max.max(0) as usize);
    for i in 0..n {
        // SAFETY: names holds `max` rows of 32 bytes.
        unsafe { core::ptr::copy_nonoverlapping(m.app[i].as_ptr(), names.add(i * 32), 32) };
    }
    n as i32
}
unsafe extern "C" fn perm_get(u: *mut c_void, app: *const c_char, r: i32) -> bool {
    let m = unsafe { m(u) };
    m.app_index(unsafe { c_bytes(app) }).map_or(false, |i| (0..RES_COUNT).contains(&r) && m.perm[i] >> r & 1 != 0)
}
unsafe extern "C" fn perm_set(u: *mut c_void, app: *const c_char, r: i32, ok: bool) -> bool {
    let m = unsafe { m(u) };
    let Some(i) = m.app_index(unsafe { c_bytes(app) }) else { return false };
    if !(0..RES_COUNT).contains(&r) {
        return false;
    }
    if ok { m.perm[i] |= 1 << r } else { m.perm[i] &= !(1 << r) }
    m.decided[i] |= 1 << r;
    true
}
unsafe extern "C" fn perm_state(u: *mut c_void, app: *const c_char, r: i32) -> i32 {
    let m = unsafe { m(u) };
    match m.app_index(unsafe { c_bytes(app) }) {
        Some(i) if (0..RES_COUNT).contains(&r) && m.decided[i] >> r & 1 != 0 => {
            if m.perm[i] >> r & 1 != 0 { 1 } else { 2 }
        }
        _ => 0,
    }
}
unsafe extern "C" fn perm_reset(u: *mut c_void, app: *const c_char, r: i32) -> bool {
    let m = unsafe { m(u) };
    let Some(i) = m.app_index(unsafe { c_bytes(app) }) else { return false };
    if !(0..RES_COUNT).contains(&r) {
        return false;
    }
    m.perm[i] &= !(1 << r);
    m.decided[i] &= !(1 << r);
    true
}
unsafe extern "C" fn fs_list(u: *mut c_void, path: *const c_char, out: *mut c_void, max: i32) -> i32 {
    let m = unsafe { m(u) };
    let p = norm(unsafe { c_bytes(path) });
    if p != b"/" && !m.node_find(&p).map_or(false, |i| m.node[i].dir) {
        return -1;
    }
    let o = unsafe { out_slice(out as *mut FsEntry, max) };
    let mut n = 0;
    for i in 0..m.node.len() {
        if n >= o.len() {
            break;
        }
        if is_child(&p, &m.node[i].path) {
            o[n] = m.entry(i);
            n += 1;
        }
    }
    n as i32
}
unsafe extern "C" fn fs_stat(u: *mut c_void, path: *const c_char, out: *mut FsEntry) -> bool {
    let m = unsafe { m(u) };
    let p = norm(unsafe { c_bytes(path) });
    let Some(o) = (unsafe { out.as_mut() }) else { return false };
    match m.node_find(&p) {
        Some(i) => {
            *o = m.entry(i);
            true
        }
        None if p == b"/" => {
            *o = FsEntry::EMPTY;
            o.name[0] = b'/';
            o.is_dir = true;
            true
        }
        None => false,
    }
}
unsafe extern "C" fn fs_mkdir(u: *mut c_void, path: *const c_char) -> bool {
    let m = unsafe { m(u) };
    let p = norm(unsafe { c_bytes(path) });
    m.parent_is_dir(&p) && m.add_node_b(&p, true, 0, 1_700_000_000, 0)
}
unsafe extern "C" fn fs_read(u: *mut c_void, path: *const c_char, out: *mut c_char, cap: usize) -> i64 {
    let m = unsafe { m(u) };
    let p = norm(unsafe { c_bytes(path) });
    match m.node_find(&p) {
        Some(n) if !m.node[n].dir => {
            let data: &[u8] = m.file_find(&p).map_or(&[], |f| &m.file[f].data);
            let len = data.len().min(cap);
            if len > 0 {
                // SAFETY: caller provides `cap` bytes.
                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), out as *mut u8, len) };
            }
            len as i64
        }
        _ => -1,
    }
}
unsafe extern "C" fn fs_write(u: *mut c_void, path: *const c_char, data: *const c_char, len: usize) -> bool {
    let m = unsafe { m(u) };
    let p = norm(unsafe { c_bytes(path) });
    let d: &[u8] = if len == 0 || data.is_null() { &[] } else { unsafe { core::slice::from_raw_parts(data as *const u8, len) } };
    m.write(&p, d)
}
unsafe extern "C" fn fs_remove(u: *mut c_void, path: *const c_char) -> bool {
    let m = unsafe { m(u) };
    let p = norm(unsafe { c_bytes(path) });
    match m.node_find(&p) {
        Some(i) if m.node[i].attrs & FSA_READONLY == 0 => {
            m.node.retain(|n| !under(&n.path, &p));
            m.file.retain(|f| !under(&f.path, &p));
            true
        }
        _ => false,
    }
}
fn rebase(path: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut v = to.to_vec();
    v.extend_from_slice(&path[from.len()..]);
    v
}
unsafe extern "C" fn fs_rename(u: *mut c_void, from: *const c_char, to: *const c_char) -> bool {
    let m = unsafe { m(u) };
    let (f, t) = (norm(unsafe { c_bytes(from) }), norm(unsafe { c_bytes(to) }));
    let Some(i) = m.node_find(&f) else { return false };
    if m.node_find(&t).is_some() || !m.parent_is_dir(&t) {
        return false;
    }
    if m.node[i].dir && under(&t, &f) {
        return false; // into itself
    }
    for n in m.node.iter_mut().filter(|n| under(&n.path, &f)) {
        n.path = rebase(&n.path, &f, &t);
    }
    for fl in m.file.iter_mut().filter(|n| under(&n.path, &f)) {
        fl.path = rebase(&fl.path, &f, &t); // contents follow the file
    }
    true
}
unsafe extern "C" fn fs_copy(u: *mut c_void, from: *const c_char, to: *const c_char) -> bool {
    let m = unsafe { m(u) };
    let (f, t) = (norm(unsafe { c_bytes(from) }), norm(unsafe { c_bytes(to) }));
    let Some(i) = m.node_find(&f) else { return false };
    if m.node_find(&t).is_some() || !m.parent_is_dir(&t) || (m.node[i].dir && under(&t, &f)) {
        return false;
    }
    let src: Vec<(Vec<u8>, bool, u64, i64, u32)> =
        m.node.iter().filter(|n| under(&n.path, &f)).map(|n| (n.path.clone(), n.dir, n.size, n.mtime, n.attrs)).collect();
    for (p, dir, size, mtime, attrs) in src {
        if m.node.len() >= MAX_NODES {
            return false;
        }
        let np = rebase(&p, &f, &t);
        m.add_node_b(&np, dir, size, mtime, attrs & !FSA_READONLY);
        if !dir {
            if let Some(fi) = m.file_find(&p) {
                if m.file.len() < MAX_FILES {
                    let data = m.file[fi].data.clone();
                    m.file.push(File { path: np, data });
                }
            }
        }
    }
    true
}
unsafe extern "C" fn fs_volumes(u: *mut c_void, out: *mut FsVolume, max: i32) -> i32 {
    let (m, o) = unsafe { (m(u), out_slice(out, max)) };
    let n = m.vol.len().min(o.len());
    o[..n].copy_from_slice(&m.vol[..n]);
    n as i32
}
unsafe extern "C" fn ps_snapshot(u: *mut c_void, out: *mut PsProc, max: i32) -> i32 {
    let (m, o) = unsafe { (m(u), out_slice(out, max)) };
    let n = m.proc_.len().min(o.len());
    o[..n].copy_from_slice(&m.proc_[..n]);
    n as i32
}
unsafe extern "C" fn ps_kill(u: *mut c_void, pid: u32) -> bool {
    let m = unsafe { m(u) };
    if pid <= 4 {
        return false; // the system process tree is not killable from the UI
    }
    let Some(i) = m.proc_.iter().position(|p| p.pid == pid) else { return false };
    m.conn.retain(|c| c.pid != pid);
    m.proc_.swap_remove(i);
    for p in m.proc_.iter_mut().filter(|p| p.ppid == pid) {
        p.ppid = 0; // orphans re-parent to the root
    }
    true
}
unsafe extern "C" fn cfg_get(u: *mut c_void, key: *const c_char, out: *mut c_char, cap: usize) -> bool {
    let m = unsafe { m(u) };
    match m.get_cfg(unsafe { c_bytes(key) }) {
        Some(v) if cap > 0 => {
            let n = v.len().min(cap - 1);
            // SAFETY: caller provides `cap` bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(v.as_ptr(), out as *mut u8, n);
                *out.add(n) = 0;
            }
            true
        }
        _ => false,
    }
}
unsafe extern "C" fn cfg_set(u: *mut c_void, key: *const c_char, val: *const c_char) -> bool {
    unsafe { m(u) }.set_cfg(unsafe { c_bytes(key) }, unsafe { c_bytes(val) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;

    fn setup() -> (Box<MemBackend>, ShellBackend) {
        let mut m = Box::new(MemBackend::new());
        let be = m.vtable();
        (m, be)
    }

    #[test]
    fn files_round_trip_and_rename_follow_contents() {
        let (mut m, be) = setup();
        m.add_node("/d", true, 0, 1, 0);
        assert!(be.fs_write(c"/d/a.txt".as_ptr(), b"hello".as_ptr() as *const c_char, 5));
        assert!(be.fs_rename(c"/d/a.txt".as_ptr(), c"/d/b.txt".as_ptr()));
        let mut buf = [0u8; 16];
        assert_eq!(be.fs_read(c"/d/b.txt".as_ptr(), buf.as_mut_ptr() as *mut c_char, 16), 5);
        assert_eq!(&buf[..5], b"hello");
        assert_eq!(be.fs_read(c"/d/a.txt".as_ptr(), buf.as_mut_ptr() as *mut c_char, 16), -1);
        assert!(!be.fs_write(c"/nope/x".as_ptr(), b"x".as_ptr() as *const c_char, 1));
    }

    #[test]
    fn config_perm_and_save_load() {
        let (mut m, be) = setup();
        be.cfg_set(b"ui.base\0", b"nord\0");
        m.add_app("calc.exe");
        assert!(be.perm_set(b"calc.exe\0", 2, true));
        assert_eq!(be.perm_state_get(b"calc.exe\0", 2), 1);
        let text = m.save();
        let (mut m2, be2) = setup();
        assert!(m2.load(&text) >= 2);
        let mut v = [0u8; 16];
        assert!(be2.cfg_get(b"ui.base\0", &mut v));
        assert_eq!(nul(&v), b"nord");
        assert_eq!(be2.perm_state_get(b"calc.exe\0", 2), 1);
    }

    #[test]
    fn kill_protects_system_and_reparents() {
        let (mut m, be) = setup();
        m.add_proc(1, 0, "system", 1, 1, 0);
        m.add_proc(9, 1, "a", 1, 1, 0);
        m.add_proc(10, 9, "b", 1, 1, 0);
        assert!(!be.ps_kill(1));
        assert!(be.ps_kill(9));
        let mut p = [PsProc { pid: 0, ppid: 0, name: [0; 32], threads: 0, handles: 0, sockets: 0, mem: 0, cpu_x10: 0, io_read: 0, io_write: 0 }; 4];
        assert_eq!(be.ps_snapshot(p.as_mut_ptr(), 4), 2);
        assert_eq!(p.iter().find(|x| x.pid == 10).unwrap().ppid, 0);
    }
}

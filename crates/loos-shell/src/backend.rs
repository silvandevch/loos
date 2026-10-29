//! C `shell_backend` layout and the helper wrappers the shell modules need (C `backend.h`).
#![allow(missing_docs)]
#![allow(dead_code)]

use core::ffi::{c_char, c_void};

pub type CfgGet = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_char, usize) -> bool;
pub type CfgSet = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> bool;
pub type PermSet = unsafe extern "C" fn(*mut c_void, *const c_char, i32, bool) -> bool;
pub type PermStateGet = unsafe extern "C" fn(*mut c_void, *const c_char, i32) -> i32;
pub type PermReset = unsafe extern "C" fn(*mut c_void, *const c_char, i32) -> bool;

/// C `shell_backend`: `user` followed by 24 function pointers.
#[repr(C)]
pub struct ShellBackend {
    pub user: *mut c_void,
    pub funcs: [*mut c_void; 24],
}

pub const F_PERM_SET: usize = 8;
pub const F_PERM_STATE_GET: usize = 9;
pub const F_PERM_RESET: usize = 10;
pub const F_CFG_GET: usize = 22;
pub const F_CFG_SET: usize = 23;

impl ShellBackend {
    fn func<T: Copy>(&self, i: usize) -> Option<T> {
        let p = self.funcs[i];
        if p.is_null() {
            None
        } else {
            // SAFETY: the backend installs the C function pointer with this exact signature.
            Some(unsafe { core::mem::transmute_copy::<*mut c_void, T>(&p) })
        }
    }
    pub fn cfg_get(&self, key: &[u8], out: &mut [u8]) -> bool {
        match self.func::<CfgGet>(F_CFG_GET) {
            Some(f) if !out.is_empty() => {
                // SAFETY: key is NUL-terminated by the caller helper; out has `out.len()` bytes.
                unsafe { f(self.user, key.as_ptr() as *const c_char, out.as_mut_ptr() as *mut c_char, out.len()) }
            }
            _ => false,
        }
    }
    pub fn cfg_set(&self, key: &[u8], value: &[u8]) {
        if let Some(f) = self.func::<CfgSet>(F_CFG_SET) {
            // SAFETY: key/value are NUL-terminated by the caller.
            unsafe { f(self.user, key.as_ptr() as *const c_char, value.as_ptr() as *const c_char) };
        }
    }
    pub fn perm_set(&self, app: &[u8], res: i32, allowed: bool) -> bool {
        match self.func::<PermSet>(F_PERM_SET) {
            Some(f) => {
                // SAFETY: app is NUL-terminated.
                unsafe { f(self.user, app.as_ptr() as *const c_char, res, allowed) }
            }
            None => false,
        }
    }
    pub fn perm_state_get(&self, app: &[u8], res: i32) -> i32 {
        match self.func::<PermStateGet>(F_PERM_STATE_GET) {
            Some(f) => {
                // SAFETY: app is NUL-terminated.
                unsafe { f(self.user, app.as_ptr() as *const c_char, res) }
            }
            None => 0,
        }
    }
    pub fn perm_reset(&self, app: &[u8], res: i32) -> bool {
        match self.func::<PermReset>(F_PERM_RESET) {
            Some(f) => {
                // SAFETY: app is NUL-terminated.
                unsafe { f(self.user, app.as_ptr() as *const c_char, res) }
            }
            None => false,
        }
    }
}

/// C `atoi`-like parse for config values (leading spaces, optional sign, digits).
pub fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = (v * 10 + (s[i] - b'0') as i64).min(2_147_483_647);
        i += 1;
    }
    if neg { -(v as i32) } else { v as i32 }
}

pub fn cfg_int(be: &ShellBackend, key: &[u8], def: i32) -> i32 {
    let mut v = [0u8; 32];
    if be.cfg_get(key, &mut v) { atoi(&v) } else { def }
}

pub fn cfg_bool(be: &ShellBackend, key: &[u8], def: bool) -> bool {
    let mut v = [0u8; 32];
    if !be.cfg_get(key, &mut v) {
        return def;
    }
    v[0] == b'1' || v[0] == b't'
}

fn put_i32(out: &mut [u8], v: i32) {
    let mut tmp = [0u8; 12];
    let mut n = 0;
    let neg = v < 0;
    let mut u = v.unsigned_abs();
    if u == 0 {
        tmp[0] = b'0';
        n = 1;
    }
    while u != 0 {
        tmp[n] = b'0' + (u % 10) as u8;
        n += 1;
        u /= 10;
    }
    let mut k = 0;
    if neg && k < out.len() - 1 {
        out[k] = b'-';
        k += 1;
    }
    for i in (0..n).rev() {
        if k < out.len() - 1 {
            out[k] = tmp[i];
            k += 1;
        }
    }
    if !out.is_empty() {
        out[k.min(out.len() - 1)] = 0;
    }
}

pub fn cfg_set_int(be: &ShellBackend, key: &[u8], v: i32) {
    let mut b = [0u8; 16];
    put_i32(&mut b, v);
    be.cfg_set(key, &b);
}

pub fn cfg_set_bool(be: &ShellBackend, key: &[u8], v: bool) {
    be.cfg_set(key, if v { b"1\0" } else { b"0\0" });
}

// ---------------------------------------------------------------- plain-function wrappers (C `backend.h`)

/// C `ps_proc` (only pid/name are read here); full layout for the snapshot buffer.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PsProc {
    pub pid: u32,
    pub ppid: u32,
    pub name: [u8; 32],
    pub threads: u32,
    pub handles: u32,
    pub sockets: u32,
    pub mem: u64,
    pub cpu_x10: u32,
    pub io_read: u64,
    pub io_write: u64,
}

pub const F_FS_LIST: usize = 11;
pub const F_FS_MKDIR: usize = 13;
pub const F_FS_REMOVE: usize = 14;
pub const F_FS_READ: usize = 17;
pub const F_FS_WRITE: usize = 18;
pub const F_PS_SNAPSHOT: usize = 20;

type FsList = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, i32) -> i32;
type FsRead = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_char, usize) -> i64;
type FsWrite = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, usize) -> bool;
type FsPath = unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
type PsSnapshot = unsafe extern "C" fn(*mut c_void, *mut PsProc, i32) -> i32;

impl ShellBackend {
    pub(crate) fn fs_list(&self, path: *const c_char, out: *mut c_void, max: i32) -> i32 {
        match self.func::<FsList>(F_FS_LIST) {
            // SAFETY: backend contract from backend.h.
            Some(f) => unsafe { f(self.user, path, out, max) },
            None => 0,
        }
    }
    pub(crate) fn fs_read(&self, path: *const c_char, out: *mut c_char, cap: usize) -> i64 {
        match self.func::<FsRead>(F_FS_READ) {
            Some(f) => unsafe { f(self.user, path, out, cap) },
            None => -1,
        }
    }
    pub(crate) fn fs_write(&self, path: *const c_char, data: *const c_char, n: usize) -> bool {
        match self.func::<FsWrite>(F_FS_WRITE) {
            Some(f) => unsafe { f(self.user, path, data, n) },
            None => false,
        }
    }
    pub(crate) fn fs_mkdir(&self, path: *const c_char) -> bool {
        match self.func::<FsPath>(F_FS_MKDIR) {
            Some(f) => unsafe { f(self.user, path) },
            None => false,
        }
    }
    pub(crate) fn fs_remove(&self, path: *const c_char) -> bool {
        match self.func::<FsPath>(F_FS_REMOVE) {
            Some(f) => unsafe { f(self.user, path) },
            None => false,
        }
    }
    pub(crate) fn ps_snapshot(&self, out: *mut PsProc, max: i32) -> i32 {
        match self.func::<PsSnapshot>(F_PS_SNAPSHOT) {
            Some(f) => unsafe { f(self.user, out, max) },
            None => 0,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn be_fs_list(b: *const ShellBackend, path: *const c_char, out: *mut c_void, max: i32) -> i32 {
    unsafe { b.as_ref() }.map_or(0, |b| b.fs_list(path, out, max))
}
#[no_mangle]
pub unsafe extern "C" fn be_fs_read(b: *const ShellBackend, path: *const c_char, out: *mut c_char, cap: usize) -> i64 {
    unsafe { b.as_ref() }.map_or(-1, |b| b.fs_read(path, out, cap))
}
#[no_mangle]
pub unsafe extern "C" fn be_fs_write(b: *const ShellBackend, path: *const c_char, data: *const c_char, n: usize) -> bool {
    unsafe { b.as_ref() }.map_or(false, |b| b.fs_write(path, data, n))
}
#[no_mangle]
pub unsafe extern "C" fn be_fs_mkdir(b: *const ShellBackend, path: *const c_char) -> bool {
    unsafe { b.as_ref() }.map_or(false, |b| b.fs_mkdir(path))
}
#[no_mangle]
pub unsafe extern "C" fn be_fs_remove(b: *const ShellBackend, path: *const c_char) -> bool {
    unsafe { b.as_ref() }.map_or(false, |b| b.fs_remove(path))
}

static mut PS_BUF: [PsProc; 64] = [PsProc { pid: 0, ppid: 0, name: [0; 32], threads: 0, handles: 0, sockets: 0, mem: 0, cpu_x10: 0, io_read: 0, io_write: 0 }; 64];

/// C `be_ps_list`: "pid name\n" lines; returns the process count.
#[no_mangle]
pub unsafe extern "C" fn be_ps_list(b: *const ShellBackend, out: *mut c_char, cap: usize) -> i32 {
    let Some(b) = (unsafe { b.as_ref() }) else { return 0 };
    // SAFETY: single-threaded shell; snapshot buffer is scratch.
    let ps = unsafe { &mut *core::ptr::addr_of_mut!(PS_BUF) };
    let n = b.ps_snapshot(ps.as_mut_ptr(), 64);
    let mut o = 0usize;
    for p in ps.iter().take(n.max(0) as usize) {
        if o >= cap {
            break;
        }
        let name = {
            let k = p.name.iter().position(|&c| c == 0).unwrap_or(32);
            &p.name[..k]
        };
        let mut tmp = [0u8; 44];
        let mut k = 0;
        let mut v = p.pid;
        let mut digits = [0u8; 10];
        let mut d = 0;
        if v == 0 {
            digits[0] = b'0';
            d = 1;
        }
        while v != 0 {
            digits[d] = b'0' + (v % 10) as u8;
            d += 1;
            v /= 10;
        }
        for i in (0..d).rev() {
            tmp[k] = digits[i];
            k += 1;
        }
        tmp[k] = b' ';
        k += 1;
        for &c in name {
            tmp[k] = c;
            k += 1;
        }
        tmp[k] = b'\n';
        k += 1;
        let room = cap.saturating_sub(o);
        let m = k.min(room.saturating_sub(1));
        // SAFETY: caller provides cap bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(tmp.as_ptr(), out as *mut u8, m);
            *out.add(o + m) = 0;
        }
        o += k;
    }
    n
}

// ---------------------------------------------------------------- data types shared with C
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetConn {
    pub proc: [u8; 32],
    pub pid: u32,
    pub remote: [u8; 40],
    pub port: u16,
    pub proto: [u8; 4],
    pub state: [u8; 12],
}
impl NetConn {
    pub const EMPTY: Self = Self { proc: [0; 32], pid: 0, remote: [0; 40], port: 0, proto: [0; 4], state: [0; 12] };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FsEntry {
    pub name: [u8; 64],
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
    pub attrs: u32,
}
impl FsEntry {
    pub const EMPTY: Self = Self { name: [0; 64], is_dir: false, size: 0, mtime: 0, attrs: 0 };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FsVolume {
    pub mount: [u8; 32],
    pub label: [u8; 24],
    pub fstype: [u8; 12],
    pub total: u64,
    pub avail: u64,
    pub readonly: bool,
}
impl FsVolume {
    pub const EMPTY: Self = Self { mount: [0; 32], label: [0; 24], fstype: [0; 12], total: 0, avail: 0, readonly: false };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FwRule {
    pub proc: [u8; 32],
    pub action: i32,
}
impl FwRule {
    pub const EMPTY: Self = Self { proc: [0; 32], action: 0 };
}

pub const F_NET_QUERY: usize = 0;
pub const F_FW_LIST: usize = 1;
pub const F_FW_SET: usize = 2;
pub const F_FW_CHECK: usize = 3;
pub const F_NET_OFF_GET: usize = 4;
pub const F_NET_OFF_SET: usize = 5;
pub const F_PERM_APPS: usize = 6;
pub const F_PERM_GET: usize = 7;
pub const F_FS_STAT: usize = 12;
pub const F_FS_RENAME: usize = 15;
pub const F_FS_COPY: usize = 16;
pub const F_FS_VOLUMES: usize = 19;
pub const F_PS_KILL: usize = 21;

type NetQuery = unsafe extern "C" fn(*mut c_void, *mut NetConn, i32) -> i32;
type FwList = unsafe extern "C" fn(*mut c_void, *mut FwRule, i32) -> i32;
type FwSet = unsafe extern "C" fn(*mut c_void, *const c_char, i32) -> bool;
type FwCheck = unsafe extern "C" fn(*mut c_void, *const c_char) -> i32;
type BoolGet = unsafe extern "C" fn(*mut c_void) -> bool;
type BoolSet = unsafe extern "C" fn(*mut c_void, bool);
type PermApps = unsafe extern "C" fn(*mut c_void, *mut u8, i32) -> i32;
type PermGet = unsafe extern "C" fn(*mut c_void, *const c_char, i32) -> bool;
type FsStat = unsafe extern "C" fn(*mut c_void, *const c_char, *mut FsEntry) -> bool;
type FsRename = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> bool;
type FsCopy = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> bool;
type FsVolumes = unsafe extern "C" fn(*mut c_void, *mut FsVolume, i32) -> i32;
type PsKill = unsafe extern "C" fn(*mut c_void, u32) -> bool;

impl ShellBackend {
    pub(crate) fn net_query(&self, out: *mut NetConn, max: i32) -> i32 {
        match self.func::<NetQuery>(F_NET_QUERY) { Some(f) => unsafe { f(self.user, out, max) }, None => 0 }
    }
    pub(crate) fn fw_list(&self, out: *mut FwRule, max: i32) -> i32 {
        match self.func::<FwList>(F_FW_LIST) { Some(f) => unsafe { f(self.user, out, max) }, None => 0 }
    }
    pub(crate) fn fw_set(&self, proc_: *const c_char, a: i32) -> bool {
        match self.func::<FwSet>(F_FW_SET) { Some(f) => unsafe { f(self.user, proc_, a) }, None => false }
    }
    pub(crate) fn fw_check(&self, proc_: *const c_char) -> i32 {
        match self.func::<FwCheck>(F_FW_CHECK) { Some(f) => unsafe { f(self.user, proc_) }, None => 0 }
    }
    pub(crate) fn net_off_get(&self) -> bool {
        match self.func::<BoolGet>(F_NET_OFF_GET) { Some(f) => unsafe { f(self.user) }, None => false }
    }
    pub(crate) fn net_off_set(&self, off: bool) {
        if let Some(f) = self.func::<BoolSet>(F_NET_OFF_SET) { unsafe { f(self.user, off) }; }
    }
    pub(crate) fn perm_apps(&self, out: *mut u8, max: i32) -> i32 {
        match self.func::<PermApps>(F_PERM_APPS) { Some(f) => unsafe { f(self.user, out, max) }, None => 0 }
    }
    pub(crate) fn perm_get(&self, app: *const c_char, r: i32) -> bool {
        match self.func::<PermGet>(F_PERM_GET) { Some(f) => unsafe { f(self.user, app, r) }, None => false }
    }
    pub(crate) fn fs_stat(&self, path: *const c_char, out: *mut FsEntry) -> bool {
        match self.func::<FsStat>(F_FS_STAT) { Some(f) => unsafe { f(self.user, path, out) }, None => false }
    }
    pub(crate) fn fs_rename(&self, from: *const c_char, to: *const c_char) -> bool {
        match self.func::<FsRename>(F_FS_RENAME) { Some(f) => unsafe { f(self.user, from, to) }, None => false }
    }
    pub(crate) fn fs_copy(&self, from: *const c_char, to: *const c_char) -> bool {
        match self.func::<FsCopy>(F_FS_COPY) { Some(f) => unsafe { f(self.user, from, to) }, None => false }
    }
    pub(crate) fn fs_volumes(&self, out: *mut FsVolume, max: i32) -> i32 {
        match self.func::<FsVolumes>(F_FS_VOLUMES) { Some(f) => unsafe { f(self.user, out, max) }, None => 0 }
    }
    pub(crate) fn ps_kill(&self, pid: u32) -> bool {
        match self.func::<PsKill>(F_PS_KILL) { Some(f) => unsafe { f(self.user, pid) }, None => false }
    }
}

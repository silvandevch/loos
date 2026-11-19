//! The compat runtime as part of the desktop: one root file system (Linux root, Wine's C: drive in
//! /home/.wine, macOS bundles in /Applications) that lives for the whole session, shown in Files as a volume
//! and used by every program the user starts from there (`loos_compat_run`). What programs write stays.
//! The kernel is single-threaded here (the desktop loop), so the state is a plain static.

use crate::lin::{rootfs, say, session};
use alloc::{string::String, vec::Vec};
use core::ffi::{c_char, c_void};
use loos_lin::fs::{normalize, Kind, LinFs, MemFs};
use loos_shell::backend::{FsEntry, ShellBackend, FSA_EXEC};

static mut FS: Option<MemFs> = None;
static mut BACKEND: ShellBackend = ShellBackend { user: core::ptr::null_mut(), funcs: [core::ptr::null_mut(); 25] };

fn fs() -> Option<&'static mut MemFs> {
    // SAFETY: single-threaded desktop loop; no reference outlives one backend call or session.
    unsafe { (*core::ptr::addr_of_mut!(FS)).as_mut() }
}

/// SAFETY: `p` is NUL-terminated (or null).
unsafe fn path(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: caller contract.
    let b = unsafe { core::ffi::CStr::from_ptr(p) }.to_bytes();
    Some(normalize("/", core::str::from_utf8(b).ok()?))
}

fn entry(name: &str, st: &loos_lin::fs::Stat) -> FsEntry {
    let mut e = FsEntry::EMPTY;
    let n = name.len().min(63);
    e.name[..n].copy_from_slice(&name.as_bytes()[..n]);
    e.is_dir = st.kind == Kind::Dir;
    e.size = st.size;
    e.mtime = st.mtime;
    e.attrs = if st.kind == Kind::File && st.mode & 0o111 != 0 { FSA_EXEC } else { 0 };
    e
}

/// Follow a symlink at the end of `p` (Files shows what a link points at, e.g. dosdevices/c:).
fn stat_follow(fs: &mut MemFs, p: &str) -> Option<loos_lin::fs::Stat> {
    let abs = loos_lin::fs::resolve(fs, "/", p, true).ok()?;
    fs.lstat(&abs).ok()
}

unsafe extern "C" fn fs_list(_: *mut c_void, p: *const c_char, out: *mut c_void, max: i32) -> i32 {
    let (Some(fs), Some(p)) = (fs(), unsafe { path(p) }) else { return -1 };
    let Ok(abs) = loos_lin::fs::resolve(fs, "/", &p, true) else { return -1 };
    let Ok(list) = fs.list(&abs) else { return -1 };
    // SAFETY: caller provides `max` entries.
    let o = unsafe { core::slice::from_raw_parts_mut(out as *mut FsEntry, max.max(0) as usize) };
    let mut n = 0;
    for d in list {
        if n >= o.len() {
            break;
        }
        let child = if abs == "/" { alloc::format!("/{}", d.name) } else { alloc::format!("{abs}/{}", d.name) };
        if let Some(st) = stat_follow(fs, &child) {
            o[n] = entry(&d.name, &st);
            n += 1;
        }
    }
    n as i32
}
unsafe extern "C" fn fs_stat(_: *mut c_void, p: *const c_char, out: *mut FsEntry) -> bool {
    let (Some(fs), Some(p), Some(o)) = (fs(), unsafe { path(p) }, unsafe { out.as_mut() }) else { return false };
    let Some(st) = stat_follow(fs, &p) else { return false };
    *o = entry(p.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("/"), &st);
    true
}
unsafe extern "C" fn fs_mkdir(_: *mut c_void, p: *const c_char) -> bool {
    let (Some(fs), Some(p)) = (fs(), unsafe { path(p) }) else { return false };
    fs.mkdir(&p, 0o755).is_ok()
}
unsafe extern "C" fn fs_remove(_: *mut c_void, p: *const c_char) -> bool {
    let (Some(fs), Some(p)) = (fs(), unsafe { path(p) }) else { return false };
    fn rm(fs: &mut MemFs, p: &str) -> bool {
        match fs.lstat(p) {
            Ok(st) if st.kind == Kind::Dir => {
                let kids: Vec<String> = fs.list(p).map(|l| l.into_iter().map(|d| alloc::format!("{p}/{}", d.name)).collect()).unwrap_or_default();
                kids.iter().all(|k| rm(fs, k)) && fs.remove(p, true).is_ok()
            }
            Ok(_) => fs.remove(p, false).is_ok(),
            Err(_) => false,
        }
    }
    p != "/" && rm(fs, &p)
}
unsafe extern "C" fn fs_rename(_: *mut c_void, a: *const c_char, b: *const c_char) -> bool {
    let (Some(fs), Some(a), Some(b)) = (fs(), unsafe { path(a) }, unsafe { path(b) }) else { return false };
    fs.lstat(&b).is_err() && fs.rename(&a, &b).is_ok()
}
unsafe extern "C" fn fs_copy(_: *mut c_void, a: *const c_char, b: *const c_char) -> bool {
    let (Some(fs), Some(a), Some(b)) = (fs(), unsafe { path(a) }, unsafe { path(b) }) else { return false };
    fn cp(fs: &mut MemFs, a: &str, b: &str) -> bool {
        let Ok(st) = fs.lstat(a) else { return false };
        match st.kind {
            Kind::Dir => {
                if fs.mkdir(b, st.mode).is_err() {
                    return false;
                }
                let kids: Vec<String> = fs.list(a).map(|l| l.into_iter().map(|d| d.name).collect()).unwrap_or_default();
                kids.iter().all(|k| cp(fs, &alloc::format!("{a}/{k}"), &alloc::format!("{b}/{k}")))
            }
            Kind::Symlink => fs.readlink(a).is_ok_and(|t| fs.symlink(&t, b).is_ok()),
            _ => {
                let data = fs.file(a).map(|d| d.to_vec()).unwrap_or_default();
                fs.create(b, st.mode).is_ok() && fs.write(b, 0, &data).is_ok()
            }
        }
    }
    (b != a && !b.starts_with(&alloc::format!("{a}/"))) && fs.lstat(&b).is_err() && cp(fs, &a, &b)
}
unsafe extern "C" fn fs_read(_: *mut c_void, p: *const c_char, out: *mut c_char, cap: usize) -> i64 {
    let (Some(fs), Some(p)) = (fs(), unsafe { path(p) }) else { return -1 };
    let Ok(abs) = loos_lin::fs::resolve(fs, "/", &p, true) else { return -1 };
    // SAFETY: caller provides `cap` bytes.
    let buf = unsafe { core::slice::from_raw_parts_mut(out as *mut u8, cap) };
    fs.read(&abs, 0, buf).map_or(-1, |n| n as i64)
}
unsafe extern "C" fn fs_write(_: *mut c_void, p: *const c_char, data: *const c_char, n: usize) -> bool {
    let (Some(fs), Some(p)) = (fs(), unsafe { path(p) }) else { return false };
    // SAFETY: caller provides `n` bytes.
    let d = if n == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(data as *const u8, n) } };
    if fs.lstat(&p).is_err() && fs.create(&p, 0o644).is_err() {
        return false;
    }
    fs.truncate(&p, 0).is_ok() && fs.write(&p, 0, d).is_ok()
}
unsafe extern "C" fn fs_set_exec(_: *mut c_void, p: *const c_char, on: bool) -> bool {
    let (Some(fs), Some(p)) = (fs(), unsafe { path(p) }) else { return false };
    let Ok(abs) = loos_lin::fs::resolve(fs, "/", &p, true) else { return false };
    let Ok(st) = fs.lstat(&abs) else { return false };
    if st.kind != Kind::File {
        return false;
    }
    // like `chmod +x`: everyone who may read it may run it
    let mode = if on { st.mode | (st.mode & 0o444) >> 2 } else { st.mode & !0o111 };
    fs.chmod(&abs, mode).is_ok()
}

/// Build the compat file system from the boot module (once) and return the desktop backend for it, or null
/// when there is no runtime.
#[no_mangle]
pub unsafe extern "C" fn loos_compat_init(initrd: *const u8, len: usize) -> *const ShellBackend {
    if initrd.is_null() || len == 0 {
        return core::ptr::null();
    }
    // SAFETY: the boot module is never freed.
    let tar = unsafe { core::slice::from_raw_parts::<'static, u8>(initrd, len) };
    // SAFETY: single-threaded init; see `fs()`.
    unsafe {
        if (*core::ptr::addr_of!(FS)).is_none() {
            FS = Some(rootfs(tar));
        }
        let b = &mut *core::ptr::addr_of_mut!(BACKEND);
        use loos_shell::backend::*;
        macro_rules! fp {
            ($f:expr) => {
                $f as *const () as *mut c_void
            };
        }
        b.funcs[F_FS_LIST] = fp!(fs_list);
        b.funcs[F_FS_STAT] = fp!(fs_stat);
        b.funcs[F_FS_MKDIR] = fp!(fs_mkdir);
        b.funcs[F_FS_REMOVE] = fp!(fs_remove);
        b.funcs[F_FS_RENAME] = fp!(fs_rename);
        b.funcs[F_FS_COPY] = fp!(fs_copy);
        b.funcs[F_FS_READ] = fp!(fs_read);
        b.funcs[F_FS_WRITE] = fp!(fs_write);
        b.funcs[F_FS_SET_EXEC] = fp!(fs_set_exec);
        b as *const ShellBackend
    }
}

/// Run a program the desktop asked for (`desktop_take_launch` text: one command entry per line) on the
/// compat file system, full screen, until it exits. Returns its wait status (negative: could not start).
#[no_mangle]
pub unsafe extern "C" fn loos_compat_run(cmd: *const c_char, back: *mut u32, width: i32, height: i32) -> i32 {
    // SAFETY: cmd is NUL-terminated.
    let text = unsafe { core::ffi::CStr::from_ptr(cmd) }.to_string_lossy().into_owned();
    let argv: Vec<String> = text.lines().filter(|l| !l.is_empty()).map(String::from).collect();
    // SAFETY: single-threaded; the file system is moved into the session and back.
    let Some(fs) = (unsafe { (*core::ptr::addr_of_mut!(FS)).take() }) else {
        say("compat: no runtime (boot without the initrd module)\n");
        return -1;
    };
    say("compat: starting ");
    say(argv.iter().rev().next().map_or("", |s| s.as_str()));
    say("\n");
    let (st, fs) = session(fs, argv, back, width, height);
    // SAFETY: as above.
    unsafe { FS = Some(fs) };
    st
}

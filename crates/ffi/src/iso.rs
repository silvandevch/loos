//! ISO 9660 / Joliet / Rock Ridge reader (loos-fs) behind the `src/fs/iso9660/iso9660.h` C API.

use crate::{bytes_mut, cstr};
use loos_fs::iso9660::Iso;
use loos_fs::{BlockDev, FileKind, FsError};
use std::ffi::{c_char, c_void};

const MAX_NAME: usize = 255;
const MAX_PATH: usize = 1024;

type ReadAt = unsafe extern "C" fn(*mut c_void, u64, *mut c_void, usize) -> i32;

#[repr(C)]
pub struct IoHooks {
    read_at: Option<ReadAt>,
    opaque: *mut c_void,
}

struct CDev {
    io: IoHooks,
}

impl BlockDev for CDev {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> loos_fs::Result<()> {
        let f = self.io.read_at.ok_or(FsError::Io)?;
        // SAFETY: the caller of loos_iso9660_mount promised the hook is valid for the lifetime of the mount.
        let rc = unsafe { f(self.io.opaque, offset, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if rc == 0 { Ok(()) } else { Err(FsError::Io) }
    }
    fn write_at(&mut self, _: u64, _: &[u8]) -> loos_fs::Result<()> {
        Err(FsError::ReadOnly)
    }
    fn flush(&mut self) -> loos_fs::Result<()> {
        Ok(())
    }
    fn len(&self) -> u64 {
        1 << 40 // unknown to the C API; the volume descriptor carries the real size
    }
}

#[repr(C)]
pub struct Entry {
    extent: u64,
    size: u64,
    flags: u8,
    name_length: u8,
    name: [u8; MAX_NAME + 1],
    path: [u8; MAX_PATH],
}

#[repr(C)]
pub struct Fs {
    io: IoHooks,
    volume_bytes: u64,
    volume_blocks: u32,
    volume_id: [u8; 33],
    root: Entry,
    mounted: bool,
    rust: *mut c_void,
}

fn code(e: FsError) -> i32 {
    match e {
        FsError::NotRecognized => -3,
        FsError::Io => -2,
        FsError::NotFound => -4,
        FsError::NotDir => -5,
        FsError::IsDir => -6,
        FsError::Unsupported => -7,
        FsError::Corrupt => -8,
        _ => -1,
    }
}

unsafe fn iso<'a>(fs: *const Fs) -> Option<&'a mut Iso<CDev>> {
    // SAFETY: `rust` is either null or the Box<Iso<CDev>> leaked by mount; the C API is single-threaded per mount.
    unsafe { ((*fs).rust as *mut Iso<CDev>).as_mut() }
}

#[no_mangle]
pub unsafe extern "C" fn loos_iso9660_mount(fs: *mut Fs, io: *const IoHooks) -> i32 {
    if fs.is_null() || io.is_null() {
        return -1;
    }
    // SAFETY: both pointers valid per the header contract.
    unsafe {
        if (*io).read_at.is_none() {
            return -1;
        }
        if !(*fs).rust.is_null() {
            drop(Box::from_raw((*fs).rust as *mut Iso<CDev>));
        }
        (*fs).rust = std::ptr::null_mut();
        (*fs).mounted = false;
        (*fs).io = IoHooks { read_at: (*io).read_at, opaque: (*io).opaque };
        let dev = CDev { io: IoHooks { read_at: (*io).read_at, opaque: (*io).opaque } };
        // Level-1 plain names are what the C API promised; richer flavours are still reachable by path.
        match Iso::mount(dev) {
            Ok(i) => {
                let label = i.label().as_bytes();
                let n = label.len().min(32);
                let f = &mut *fs;
                f.volume_id = [0; 33];
                f.volume_id[..n].copy_from_slice(&label[..n]);
                (*fs).volume_blocks = i.sectors().min(u32::MAX as u64) as u32;
                (*fs).volume_bytes = i.sectors() * 2048;
                (*fs).root.flags = 2;
                (*fs).root.size = 0;
                (*fs).root.extent = 0;
                (*fs).root.name_length = 0;
                (*fs).root.path[0] = b'/';
                (*fs).root.path[1] = 0;
                (*fs).rust = Box::into_raw(Box::new(i)) as *mut c_void;
                (*fs).mounted = true;
                0
            }
            Err(e) => code(e),
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn loos_iso9660_unmount(fs: *mut Fs) {
    if fs.is_null() {
        return;
    }
    // SAFETY: see iso().
    unsafe {
        if !(*fs).rust.is_null() {
            drop(Box::from_raw((*fs).rust as *mut Iso<CDev>));
        }
        (*fs).rust = std::ptr::null_mut();
        (*fs).mounted = false;
    }
}

#[no_mangle]
pub unsafe extern "C" fn loos_iso9660_lookup(fs: *const Fs, path: *const c_char, entry: *mut Entry) -> i32 {
    // SAFETY: pointers valid per the header contract.
    unsafe {
        let (Some(p), Some(i)) = (cstr(path), if fs.is_null() { None } else { iso(fs) }) else { return -1 };
        if entry.is_null() || p.len() >= MAX_PATH {
            return -1;
        }
        // ISO 9660 version suffix ("NAME.TXT;1") is optional in the C API; the Rust reader lists names without it.
        let p = match p.rsplit_once(';') {
            Some((head, ver)) if !ver.is_empty() && ver.bytes().all(|b| b.is_ascii_digit()) => head,
            _ => p,
        };
        let abs = if p.starts_with('/') { p.to_string() } else { format!("/{p}") };
        match i.stat(&abs) {
            Ok(st) => {
                let e = &mut *entry;
                e.extent = 0;
                e.size = st.size;
                e.flags = if st.kind == FileKind::Dir { 2 } else { 0 };
                let nb = st.name.as_bytes();
                let n = nb.len().min(MAX_NAME);
                e.name = [0; MAX_NAME + 1];
                e.name[..n].copy_from_slice(&nb[..n]);
                e.name_length = n as u8;
                e.path = [0; MAX_PATH];
                e.path[..abs.len()].copy_from_slice(abs.as_bytes());
                0
            }
            Err(e) => code(e),
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn loos_iso9660_read(fs: *const Fs, entry: *const Entry, offset: u64, buf: *mut u8, len: usize) -> i32 {
    // SAFETY: pointers valid per the header contract.
    unsafe {
        let Some(i) = (if fs.is_null() { None } else { iso(fs) }) else { return -1 };
        if entry.is_null() || (buf.is_null() && len != 0) {
            return -1;
        }
        if (*entry).flags & 2 != 0 {
            return -6;
        }
        let end = offset.checked_add(len as u64);
        if end.is_none_or(|e| e > (*entry).size) {
            return -1;
        }
        let e = &*entry;
        let pl = e.path.iter().position(|&b| b == 0).unwrap_or(MAX_PATH);
        let Ok(path) = std::str::from_utf8(&e.path[..pl]) else { return -1 };
        let out = bytes_mut(buf, len);
        match i.read_file(path, offset, out) {
            Ok(n) if n == len => 0,
            Ok(_) => -8,
            Err(e) => code(e),
        }
    }
}

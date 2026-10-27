//! Storage: mount manager over FAT/ISO/NTFS volumes (loos-vfs + loos-fs) and the btrfs reader (loos-btrfs-ops).

use crate::{bytes_mut, cstr, put_str};
use loos_fs::{BlockDev, FsError};
use loos_vfs::{adapters, MountManager, VolumeId};
use std::ffi::{c_char, c_void};

type ReadAt = unsafe extern "C" fn(*mut c_void, u64, *mut c_void, usize) -> i32;
type WriteAt = unsafe extern "C" fn(*mut c_void, u64, *const c_void, usize) -> i32;

#[derive(Clone, Copy)]
struct Hooks {
    read: ReadAt,
    write: Option<WriteAt>,
    opaque: *mut c_void,
    len: u64,
}

struct CBlock(Hooks);

impl BlockDev for CBlock {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> loos_fs::Result<()> {
        // SAFETY: the C caller guarantees the hook and opaque pointer outlive the mount.
        let rc = unsafe { (self.0.read)(self.0.opaque, offset, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if rc == 0 { Ok(()) } else { Err(FsError::Io) }
    }
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> loos_fs::Result<()> {
        match self.0.write {
            // SAFETY: as above.
            Some(w) => if unsafe { w(self.0.opaque, offset, buf.as_ptr() as *const c_void, buf.len()) } == 0 { Ok(()) } else { Err(FsError::Io) },
            None => Err(FsError::ReadOnly),
        }
    }
    fn flush(&mut self) -> loos_fs::Result<()> {
        Ok(())
    }
    fn len(&self) -> u64 {
        self.0.len
    }
}

/// Mount manager handle.
pub struct Vfs {
    mgr: MountManager,
    next: u64,
}

#[no_mangle]
pub extern "C" fn loos_vfs_new() -> *mut Vfs {
    Box::into_raw(Box::new(Vfs { mgr: MountManager::new(), next: 1 }))
}
#[no_mangle]
pub unsafe extern "C" fn loos_vfs_free(v: *mut Vfs) {
    if !v.is_null() {
        // SAFETY: allocated by loos_vfs_new.
        drop(unsafe { Box::from_raw(v) });
    }
}
/// Mounts a volume image at `mountpoint` (optionally with a drive-letter `alias` such as "C:", may be NULL).
/// kind: 0 FAT, 1 ISO 9660/Joliet/Rock Ridge, 2 NTFS (read-only). `write_at` may be NULL. Returns a mount id (> 0) or
/// a negative code (-1 args, -2 not recognized/corrupt, -3 mount conflict).
#[no_mangle]
pub unsafe extern "C" fn loos_vfs_mount(v: *mut Vfs, kind: u32, read_at: Option<ReadAt>, write_at: Option<WriteAt>, opaque: *mut c_void, len: u64, mountpoint: *const c_char, alias: *const c_char) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(v), Some(read), Some(mp)) = (v.as_mut(), read_at, cstr(mountpoint)) else { return -1 };
        let dev = CBlock(Hooks { read, write: write_at, opaque, len });
        let mut guid = [0u8; 16];
        guid[..8].copy_from_slice(&v.next.to_le_bytes());
        v.next += 1;
        let id = VolumeId::from(guid);
        let reg = match kind {
            0 => adapters::FatAdapter::mount(dev, 0).map(|a| v.mgr.register_volume(id, a)),
            1 => adapters::IsoAdapter::mount(dev).map(|a| v.mgr.register_volume(id, a)),
            2 => adapters::NtfsAdapter::mount(dev, 0).map(|a| v.mgr.register_volume(id, a)),
            _ => return -1,
        };
        match reg {
            Ok(Ok(())) => {}
            _ => return -2,
        }
        let aliases: Vec<&str> = cstr(alias).filter(|a| !a.is_empty()).into_iter().collect();
        match v.mgr.mount(id, mp, &aliases, write_at.is_none() || kind != 0) {
            Ok(m) => m as i64,
            Err(_) => -3,
        }
    }
}
/// Size of a file/directory at `path` (u64 in `*size`), `*is_dir` set; 0 ok, -1 not found / error.
#[no_mangle]
pub unsafe extern "C" fn loos_vfs_stat(v: *mut Vfs, path: *const c_char, size: *mut u64, is_dir: *mut bool) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(v), Some(p)) = (v.as_mut(), cstr(path)) else { return -1 };
        match v.mgr.stat(p) {
            Ok(e) => {
                *size = e.size;
                *is_dir = e.kind == loos_fs::FileKind::Dir;
                0
            }
            Err(_) => -1,
        }
    }
}
/// Directory listing, one name per line (directories end with '/'). Returns text length or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_vfs_list(v: *mut Vfs, path: *const c_char, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(v), Some(p)) = (v.as_mut(), cstr(path)) else { return -1 };
        match v.mgr.read_dir(p) {
            Ok(es) => {
                let mut t = String::new();
                for e in es {
                    t.push_str(&e.name);
                    if e.kind == loos_fs::FileKind::Dir {
                        t.push('/');
                    }
                    t.push('\n');
                }
                put_str(&t, out, cap) as i64
            }
            Err(_) => -1,
        }
    }
}
/// Reads file bytes; returns the count read or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_vfs_read(v: *mut Vfs, path: *const c_char, offset: u64, buf: *mut u8, len: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(v), Some(p)) = (v.as_mut(), cstr(path)) else { return -1 };
        v.mgr.read(p, offset, bytes_mut(buf, len)).map_or(-1, |n| n as i64)
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_vfs_write(v: *mut Vfs, path: *const c_char, offset: u64, data: *const u8, len: usize) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(v), Some(p)) = (v.as_mut(), cstr(path)) else { return -1 };
        v.mgr.write(p, offset, crate::bytes(data, len)).map_or(-1, |_| 0)
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_vfs_unmount(v: *mut Vfs, mountpoint: *const c_char) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(v), Some(p)) = (v.as_mut(), cstr(mountpoint)) else { return -1 };
        v.mgr.unmount(p).map_or(-1, |_| 0)
    }
}

// ------------------------------------------------------------------ btrfs (read side)
use loos_btrfs_ops::dev::{Dev, DevError};
use loos_btrfs_ops::fs::{Fs, Kind};
use loos_btrfs_ops::volume::Volume;

struct CDev(Hooks);

impl Dev for CDev {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), DevError> {
        // SAFETY: hooks outlive the handle (header contract).
        if unsafe { (self.0.read)(self.0.opaque, offset, buf.as_mut_ptr() as *mut c_void, buf.len()) } == 0 { Ok(()) } else { Err(DevError::Io) }
    }
    fn write_at(&mut self, _: u64, _: &[u8]) -> Result<(), DevError> {
        Err(DevError::ReadOnly)
    }
    fn flush(&mut self) -> Result<(), DevError> {
        Ok(())
    }
    fn len(&self) -> u64 {
        self.0.len
    }
}

pub struct Btrfs(Fs<CDev>);

/// Opens a btrfs image (single device) through read hooks; NULL if it is not a valid btrfs filesystem.
#[no_mangle]
pub unsafe extern "C" fn loos_btrfs_open(read_at: Option<ReadAt>, opaque: *mut c_void, len: u64) -> *mut Btrfs {
    let Some(read) = read_at else { return std::ptr::null_mut() };
    let dev = CDev(Hooks { read, write: None, opaque, len });
    match Volume::open(vec![dev]).and_then(Fs::open) {
        Ok(fs) => Box::into_raw(Box::new(Btrfs(fs))),
        Err(_) => std::ptr::null_mut(),
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_btrfs_close(b: *mut Btrfs) {
    if !b.is_null() {
        // SAFETY: allocated by loos_btrfs_open.
        drop(unsafe { Box::from_raw(b) });
    }
}
/// Size in bytes of the file at `path`; `*is_dir` set. 0 ok, -1 not found / error.
#[no_mangle]
pub unsafe extern "C" fn loos_btrfs_stat(b: *mut Btrfs, path: *const c_char, size: *mut u64, is_dir: *mut bool) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(b), Some(p)) = (b.as_mut(), cstr(path)) else { return -1 };
        match b.0.resolve(p).and_then(|n| b.0.stat(n)) {
            Ok(i) => {
                *size = i.size;
                *is_dir = i.kind() == Kind::Dir;
                0
            }
            Err(_) => -1,
        }
    }
}
/// Directory listing, one name per line, directories end with '/'. Returns text length or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_btrfs_list(b: *mut Btrfs, path: *const c_char, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(b), Some(p)) = (b.as_mut(), cstr(path)) else { return -1 };
        match b.0.resolve(p).and_then(|n| b.0.readdir(n)) {
            Ok(es) => {
                let mut t = String::new();
                for e in es {
                    t.push_str(&String::from_utf8_lossy(&e.name));
                    if e.kind == Kind::Dir {
                        t.push('/');
                    }
                    t.push('\n');
                }
                put_str(&t, out, cap) as i64
            }
            Err(_) => -1,
        }
    }
}
/// Reads file bytes (compressed extents are inflated); returns the count read or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_btrfs_read(b: *mut Btrfs, path: *const c_char, offset: u64, buf: *mut u8, len: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(b), Some(p)) = (b.as_mut(), cstr(path)) else { return -1 };
        match b.0.resolve(p).and_then(|n| b.0.read(n, offset, bytes_mut(buf, len))) {
            Ok(n) => n as i64,
            Err(_) => -1,
        }
    }
}

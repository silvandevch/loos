//! Linux filesystem adapter over the native LoOS VFS mount manager.
//!
//! [`VfsLinFs`] keeps Linux paths absolute while delegating storage to the mounted
//! filesystem selected by [`loos_vfs::MountManager`].  Generated `/proc` and
//! device nodes remain owned by [`crate::sys::LinSystem`], so this adapter only
//! handles persistent filesystem paths such as `/etc`, `/usr`, and `/home`.

use crate::errno::*;
use crate::fs::{DirEnt, Kind, LinFs, Stat};
use alloc::string::String;
use alloc::vec::Vec;
use loos_fs::FileKind;
use loos_vfs::{MountManager, VfsError};

/// A Linux filesystem view backed by the native LoOS VFS.
pub struct VfsLinFs {
    manager: MountManager,
}

impl VfsLinFs {
    /// Wraps an already configured mount manager.
    pub const fn new(manager: MountManager) -> Self {
        Self { manager }
    }

    /// Returns the underlying mount manager.
    pub fn manager(&self) -> &MountManager {
        &self.manager
    }

    /// Mutably accesses the underlying mount manager for mount/unmount operations.
    pub fn manager_mut(&mut self) -> &mut MountManager {
        &mut self.manager
    }

    /// Consumes the adapter and returns its mount manager.
    pub fn into_manager(self) -> MountManager {
        self.manager
    }
}

fn errno(error: VfsError) -> i64 {
    match error {
        VfsError::Fs(e) => match e {
            loos_fs::FsError::Io => EIO,
            loos_fs::FsError::NotRecognized | loos_fs::FsError::Corrupt => EIO,
            loos_fs::FsError::NotFound => ENOENT,
            loos_fs::FsError::NotDir => ENOTDIR,
            loos_fs::FsError::IsDir => EISDIR,
            loos_fs::FsError::Exists => EEXIST,
            loos_fs::FsError::NotEmpty => ENOTEMPTY,
            loos_fs::FsError::NoSpace => 28,
            loos_fs::FsError::InvalidName => EINVAL,
            loos_fs::FsError::Unsupported => ENOSYS,
            loos_fs::FsError::ReadOnly => 30,
        },
        VfsError::InvalidPath => EINVAL,
        VfsError::InvalidGuid => EINVAL,
        VfsError::DuplicateVolume | VfsError::AlreadyMounted | VfsError::MountConflict => EEXIST,
        VfsError::VolumeNotFound | VfsError::MountNotFound => ENOENT,
        VfsError::Busy => 16,
        VfsError::Unsupported => ENOSYS,
        VfsError::HandleClosed => EBADF,
    }
}

fn kind(k: FileKind) -> Kind {
    match k {
        FileKind::File => Kind::File,
        FileKind::Dir => Kind::Dir,
        FileKind::Symlink => Kind::Symlink,
    }
}

fn stat_from(e: loos_fs::Entry) -> Stat {
    Stat {
        ino: e.id,
        kind: kind(e.kind),
        mode: e.mode,
        size: e.size,
        nlink: 1,
        uid: e.uid,
        gid: e.gid,
        mtime: e.mtime.min(i64::MAX as u64) as i64,
    }
}

impl LinFs for VfsLinFs {
    fn lstat(&mut self, path: &str) -> Result<Stat, i64> {
        self.manager.stat(path).map(stat_from).map_err(errno)
    }

    fn read(&mut self, path: &str, off: u64, buf: &mut [u8]) -> Result<usize, i64> {
        self.manager.read(path, off, buf).map_err(errno)
    }

    fn write(&mut self, path: &str, off: u64, data: &[u8]) -> Result<usize, i64> {
        self.manager.write(path, off, data).map(|()| data.len()).map_err(errno)
    }

    fn create(&mut self, path: &str, _mode: u32) -> Result<(), i64> {
        self.manager.create_file(path).map_err(errno)
    }

    fn truncate(&mut self, _path: &str, _len: u64) -> Result<(), i64> {
        self.manager.truncate(_path, _len).map_err(errno)
    }

    fn mkdir(&mut self, path: &str, mode: u32) -> Result<(), i64> {
        self.manager.mkdir(path, mode).map_err(errno)
    }

    fn remove(&mut self, path: &str, dir: bool) -> Result<(), i64> {
        self.manager.remove(path, dir).map_err(errno)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), i64> {
        self.manager.rename(from, to).map_err(errno)
    }

    fn readlink(&mut self, path: &str) -> Result<String, i64> {
        let e = self.manager.stat(path).map_err(errno)?;
        e.target.ok_or(EINVAL)
    }

    fn symlink(&mut self, target: &str, path: &str) -> Result<(), i64> {
        self.manager.symlink(target, path).map_err(errno)
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEnt>, i64> {
        self.manager
            .read_dir(path)
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|e| DirEnt { name: e.name, ino: e.id, kind: kind(e.kind) })
                    .collect()
            })
            .map_err(errno)
    }
}

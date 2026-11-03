//! [`HiveStorage`] over the LoOS VFS: hive files live on the system btrfs volume
//! (e.g. `/System/config/SYSTEM`) and are replaced with write-tmp + flush + rename.

use crate::service::HiveStorage;
use crate::Error;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use loos_fs::FsError;
use loos_vfs::{MountManager, VfsError};

/// Hive files below `root` (e.g. `/System`) on a mounted volume.
pub struct VfsStorage<'a> {
    vfs: &'a mut MountManager,
    root: String,
}

impl<'a> VfsStorage<'a> {
    /// Use `root` as the directory that holds hive files.
    pub fn new(vfs: &'a mut MountManager, root: &str) -> Self {
        Self { vfs, root: root.trim_end_matches('/').into() }
    }
}

fn io(_: VfsError) -> Error {
    Error::Io
}

impl HiveStorage for VfsStorage<'_> {
    fn load(&mut self, file: &str) -> Result<Option<Vec<u8>>, Error> {
        let path = format!("{}/{}", self.root, file);
        let size = match self.vfs.stat(&path) {
            Ok(e) => e.size,
            Err(VfsError::Fs(FsError::NotFound)) => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        if size > 64 * 1024 * 1024 {
            return Err(Error::TooLarge);
        }
        let mut buf = vec![0u8; size as usize];
        let mut off = 0usize;
        while off < buf.len() {
            let n = self.vfs.read(&path, off as u64, &mut buf[off..]).map_err(io)?;
            if n == 0 {
                return Err(Error::Corrupt);
            }
            off += n;
        }
        Ok(Some(buf))
    }

    fn store(&mut self, file: &str, data: &[u8]) -> Result<(), Error> {
        let path = format!("{}/{}", self.root, file);
        let tmp = format!("{path}.new");
        self.vfs.create_file(&tmp).map_err(io)?;
        self.vfs.write(&tmp, 0, data).map_err(io)?;
        self.vfs.flush_path(&tmp).map_err(io)?;
        self.vfs.rename(&tmp, &path).map_err(io)
    }
}

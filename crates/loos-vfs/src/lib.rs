//! A small, allocation-only virtual filesystem mount manager.
//!
//! The manager owns registered filesystem backends, resolves normalized absolute paths to the
//! longest matching mountpoint, and provides common stat, directory, read, and write operations.
//! The concrete adapters in this crate bridge the filesystems implemented by [`loos_fs`].
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use loos_fs::{Entry, FsError};

/// A 128-bit volume identity, suitable for a filesystem GUID or a generated volume identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VolumeId([u8; 16]);

impl VolumeId {
    /// Creates an identifier from its raw bytes.
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the raw identifier bytes.
    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }

    /// Parses a GUID in either `00112233-4455-6677-8899-aabbccddeeff` or compact form.
    ///
    /// The textual byte order is retained. This is intentional: the VFS treats the value as an
    /// identity and does not reinterpret a platform-specific on-disk GUID layout.
    pub fn from_guid(text: &str) -> Result<Self, VfsError> {
        let mut bytes = [0u8; 16];
        let mut nibbles = 0usize;
        let mut pending = None;
        for c in text.chars() {
            if c == '-' {
                continue;
            }
            let v = c.to_digit(16).ok_or(VfsError::InvalidGuid)? as u8;
            if let Some(hi) = pending.take() {
                if nibbles >= 32 {
                    return Err(VfsError::InvalidGuid);
                }
                bytes[nibbles / 2] = (hi << 4) | v;
                nibbles += 2;
            } else {
                pending = Some(v);
            }
        }
        if pending.is_some() || nibbles != 32 {
            return Err(VfsError::InvalidGuid);
        }
        Ok(Self(bytes))
    }
}

impl From<[u8; 16]> for VolumeId {
    fn from(bytes: [u8; 16]) -> Self {
        Self::new(bytes)
    }
}

/// Errors produced by the VFS manager and adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VfsError {
    /// An operation failed in the underlying filesystem.
    Fs(FsError),
    /// A path was not absolute, contained an invalid drive form, or contained NUL.
    InvalidPath,
    /// A GUID string was malformed.
    InvalidGuid,
    /// The volume identifier is already registered.
    DuplicateVolume,
    /// The requested volume is not registered.
    VolumeNotFound,
    /// A mountpoint or alias is already in use.
    MountConflict,
    /// The requested mountpoint has no mounted filesystem.
    MountNotFound,
    /// A volume is already mounted.
    AlreadyMounted,
    /// An unmount was blocked by open handles.
    Busy,
    /// The requested operation is not supported by the backend.
    Unsupported,
    /// A handle was already closed.
    HandleClosed,
}

impl From<FsError> for VfsError {
    fn from(error: FsError) -> Self {
        Self::Fs(error)
    }
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fs(e) => e.fmt(f),
            Self::InvalidPath => f.write_str("invalid absolute path"),
            Self::InvalidGuid => f.write_str("invalid volume GUID"),
            Self::DuplicateVolume => f.write_str("volume already registered"),
            Self::VolumeNotFound => f.write_str("volume not found"),
            Self::MountConflict => f.write_str("mountpoint or alias already in use"),
            Self::MountNotFound => f.write_str("mount not found"),
            Self::AlreadyMounted => f.write_str("volume already mounted"),
            Self::Busy => f.write_str("mount is busy"),
            Self::Unsupported => f.write_str("unsupported operation"),
            Self::HandleClosed => f.write_str("handle is closed"),
        }
    }
}

/// A mounted filesystem backend.
pub trait FileSystem {
    /// Returns metadata for a path relative to the backend root.
    fn stat(&mut self, path: &str) -> Result<Entry, VfsError>;
    /// Lists a directory relative to the backend root.
    fn read_dir(&mut self, path: &str) -> Result<Vec<Entry>, VfsError>;
    /// Reads at most `buf.len()` bytes from a file.
    fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError>;
    /// Writes bytes at an offset when the backend supports mutation.
    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<(), VfsError> {
        let _ = (path, offset, data);
        Err(VfsError::Fs(FsError::ReadOnly))
    }
    /// Flushes completed writes.
    fn flush(&mut self) -> Result<(), VfsError> {
        Ok(())
    }
    /// Whether the backend itself is intrinsically read-only.
    fn is_read_only(&self) -> bool {
        false
    }
}

/// Normalizes an absolute path using `/` separators.
///
/// Backslashes are accepted as separators. Empty and `.` components are removed, while `..`
/// removes one component and is clamped at the root. This function never lets `..` escape `/` and
/// never uses string-prefix matching for component boundaries.
pub fn normalize_absolute(path: &str) -> Result<String, VfsError> {
    if path.is_empty() || !path.starts_with('/') || path.bytes().any(|b| b == 0) {
        return Err(VfsError::InvalidPath);
    }
    let mut components: Vec<&str> = Vec::new();
    for component in path.split(|c| c == '/' || c == '\\') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            value if value.bytes().any(|b| b == 0) => return Err(VfsError::InvalidPath),
            value => components.push(value),
        }
    }
    let mut out = String::from("/");
    for (i, component) in components.iter().enumerate() {
        if i != 0 {
            out.push('/');
        }
        out.push_str(component);
    }
    Ok(out)
}

/// Alias for [`normalize_absolute`].
pub fn normalize_path(path: &str) -> Result<String, VfsError> {
    normalize_absolute(path)
}

/// Public information about a mounted volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountInfo {
    /// Stable mount identifier.
    pub id: u64,
    /// Registered volume identity.
    pub volume: VolumeId,
    /// Canonical mountpoint.
    pub mountpoint: String,
    /// Effective read-only state.
    pub read_only: bool,
    /// Aliases resolving to this mountpoint.
    pub aliases: Vec<String>,
    /// Number of currently open handles.
    pub open_handles: usize,
}

struct Volume {
    id: VolumeId,
    backend: Box<dyn FileSystem>,
    mounted: bool,
}

struct Mount {
    info: MountInfo,
    volume_index: usize,
}

/// A registered-volume and mountpoint manager.
pub struct MountManager {
    volumes: Vec<Volume>,
    mounts: Vec<Mount>,
    next_mount_id: u64,
}

impl Default for MountManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MountManager {
    /// Creates an empty manager.
    pub const fn new() -> Self {
        Self {
            volumes: Vec::new(),
            mounts: Vec::new(),
            next_mount_id: 1,
        }
    }

    /// Registers a backend under a stable volume identity.
    pub fn register_volume<F>(&mut self, id: VolumeId, backend: F) -> Result<(), VfsError>
    where
        F: FileSystem + 'static,
    {
        self.register_boxed(id, Box::new(backend))
    }

    /// Registers an already boxed backend.
    pub fn register_boxed(
        &mut self,
        id: VolumeId,
        backend: Box<dyn FileSystem>,
    ) -> Result<(), VfsError> {
        if self.volumes.iter().any(|v| v.id == id) {
            return Err(VfsError::DuplicateVolume);
        }
        self.volumes.push(Volume {
            id,
            backend,
            mounted: false,
        });
        Ok(())
    }

    /// Removes an unmounted volume registration.
    pub fn unregister_volume(&mut self, id: VolumeId) -> Result<(), VfsError> {
        let index = self.volume_index(id)?;
        if self.volumes[index].mounted {
            return Err(VfsError::Busy);
        }
        self.volumes.remove(index);
        for mount in &mut self.mounts {
            if mount.volume_index > index {
                mount.volume_index -= 1;
            }
        }
        Ok(())
    }

    /// Mounts a registered volume at an absolute mountpoint.
    ///
    /// Aliases may be Windows drive names such as `C:` or absolute paths. Drive names are matched
    /// case-insensitively and only as complete drive prefixes (`C:` / `C:/...`).
    pub fn mount(
        &mut self,
        volume: VolumeId,
        mountpoint: &str,
        aliases: &[&str],
        read_only: bool,
    ) -> Result<u64, VfsError> {
        let volume_index = self.volume_index(volume)?;
        if self.volumes[volume_index].mounted {
            return Err(VfsError::AlreadyMounted);
        }
        let mountpoint = normalize_absolute(mountpoint)?;
        let mut normalized_aliases = Vec::with_capacity(aliases.len());
        for alias in aliases {
            let normalized = normalize_alias(alias)?;
            if normalized == mountpoint {
                return Err(VfsError::MountConflict);
            }
            if normalized_aliases.iter().any(|x| x == &normalized) {
                return Err(VfsError::MountConflict);
            }
            normalized_aliases.push(normalized);
        }
        if self.mounts.iter().any(|m| {
            m.info.mountpoint == mountpoint
                || aliases_conflict(&m.info.mountpoint, &normalized_aliases)
                || aliases_conflict_any(&m.info.aliases, &mountpoint, &normalized_aliases)
        }) {
            return Err(VfsError::MountConflict);
        }
        let effective_read_only = read_only || self.volumes[volume_index].backend.is_read_only();
        let id = self.next_mount_id;
        self.next_mount_id = self.next_mount_id.checked_add(1).unwrap_or(1);
        self.volumes[volume_index].mounted = true;
        self.mounts.push(Mount {
            info: MountInfo {
                id,
                volume,
                mountpoint,
                read_only: effective_read_only,
                aliases: normalized_aliases,
                open_handles: 0,
            },
            volume_index,
        });
        Ok(id)
    }

    /// Returns a snapshot of mounted volumes.
    pub fn mounts(&self) -> Vec<MountInfo> {
        self.mounts.iter().map(|m| m.info.clone()).collect()
    }

    /// Unmounts by canonical mountpoint or one of its aliases.
    pub fn unmount(&mut self, path: &str) -> Result<(), VfsError> {
        let index = self.mount_index_for(path)?;
        if self.mounts[index].info.open_handles != 0 {
            return Err(VfsError::Busy);
        }
        let volume_index = self.mounts[index].volume_index;
        self.volumes[volume_index].backend.flush()?;
        self.volumes[volume_index].mounted = false;
        self.mounts.remove(index);
        Ok(())
    }

    /// Returns metadata for a path.
    pub fn stat(&mut self, path: &str) -> Result<Entry, VfsError> {
        let (index, subpath) = self.resolve(path)?;
        let volume_index = self.mounts[index].volume_index;
        self.volumes[volume_index].backend.stat(&subpath)
    }

    /// Lists a directory.
    pub fn read_dir(&mut self, path: &str) -> Result<Vec<Entry>, VfsError> {
        let (index, subpath) = self.resolve(path)?;
        let volume_index = self.mounts[index].volume_index;
        self.volumes[volume_index].backend.read_dir(&subpath)
    }

    /// Reads a file.
    pub fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
        let (index, subpath) = self.resolve(path)?;
        let volume_index = self.mounts[index].volume_index;
        self.volumes[volume_index]
            .backend
            .read(&subpath, offset, buf)
    }

    /// Writes a file, enforcing the mount's effective read-only state first.
    pub fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<(), VfsError> {
        let (index, subpath) = self.resolve(path)?;
        if self.mounts[index].info.read_only {
            return Err(VfsError::Fs(FsError::ReadOnly));
        }
        let volume_index = self.mounts[index].volume_index;
        self.volumes[volume_index]
            .backend
            .write(&subpath, offset, data)
    }

    /// Opens a path and records a busy handle against its mount.
    pub fn open(&mut self, path: &str) -> Result<Handle, VfsError> {
        let (index, subpath) = self.resolve(path)?;
        let volume_index = self.mounts[index].volume_index;
        self.volumes[volume_index].backend.stat(&subpath)?;
        let mount_id = self.mounts[index].info.id;
        self.mounts[index].info.open_handles += 1;
        Ok(Handle {
            mount_id,
            path: subpath,
        })
    }

    fn volume_index(&self, id: VolumeId) -> Result<usize, VfsError> {
        self.volumes
            .iter()
            .position(|v| v.id == id)
            .ok_or(VfsError::VolumeNotFound)
    }

    fn resolve(&self, path: &str) -> Result<(usize, String), VfsError> {
        let canonical = self.canonicalize(path)?;
        let index = self
            .longest_mount(&canonical)
            .ok_or(VfsError::MountNotFound)?;
        let mountpoint = &self.mounts[index].info.mountpoint;
        let suffix = if mountpoint == "/" {
            canonical.clone()
        } else if canonical == *mountpoint {
            String::from("/")
        } else {
            canonical[mountpoint.len()..].to_string()
        };
        Ok((index, suffix))
    }

    fn mount_index_for(&self, path: &str) -> Result<usize, VfsError> {
        let canonical = self.canonicalize(path)?;
        self.mounts
            .iter()
            .position(|m| {
                m.info.mountpoint == canonical
                    || m.info.aliases.iter().any(|a| a == path || a == &canonical)
            })
            .ok_or(VfsError::MountNotFound)
    }

    fn longest_mount(&self, canonical: &str) -> Option<usize> {
        self.mounts
            .iter()
            .enumerate()
            .filter(|(_, m)| component_prefix(&m.info.mountpoint, canonical))
            .max_by_key(|(_, m)| m.info.mountpoint.len())
            .map(|(i, _)| i)
    }

    fn canonicalize(&self, path: &str) -> Result<String, VfsError> {
        if let Some((drive, rest)) = split_drive(path) {
            let alias = self
                .mounts
                .iter()
                .flat_map(|m| m.info.aliases.iter().map(move |a| (m, a)))
                .find(|(_, a)| a.eq_ignore_ascii_case(&drive))
                .map(|(m, _)| m.info.mountpoint.clone())
                .ok_or(VfsError::MountNotFound)?;
            let tail = if rest.is_empty() {
                String::from("/")
            } else {
                normalize_absolute(&format!(
                    "/{}",
                    rest.trim_start_matches(|c| c == '/' || c == '\\')
                ))?
            };
            return join_paths(&alias, &tail);
        }
        let normalized = normalize_absolute(path)?;
        let mut best: Option<(&Mount, &str)> = None;
        for mount in &self.mounts {
            for alias in &mount.info.aliases {
                if !alias.starts_with('/') || !component_prefix(alias, &normalized) {
                    continue;
                }
                if best.map_or(true, |(_, old)| alias.len() > old.len()) {
                    best = Some((mount, alias));
                }
            }
        }
        if let Some((mount, alias)) = best {
            let suffix = if normalized == alias {
                "/"
            } else {
                &normalized[alias.len()..]
            };
            return join_paths(&mount.info.mountpoint, suffix);
        }
        Ok(normalized)
    }
}

/// A busy path handle. Call [`Handle::close`] to release its mount reference.
pub struct Handle {
    mount_id: u64,
    path: String,
}

impl Handle {
    /// Reads through this handle.
    pub fn read(
        &self,
        manager: &mut MountManager,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, VfsError> {
        let index = manager
            .mounts
            .iter()
            .position(|m| m.info.id == self.mount_id)
            .ok_or(VfsError::MountNotFound)?;
        let volume_index = manager.mounts[index].volume_index;
        manager.volumes[volume_index]
            .backend
            .read(&self.path, offset, buf)
    }

    /// Returns metadata through this handle.
    pub fn stat(&self, manager: &mut MountManager) -> Result<Entry, VfsError> {
        let index = manager
            .mounts
            .iter()
            .position(|m| m.info.id == self.mount_id)
            .ok_or(VfsError::MountNotFound)?;
        let volume_index = manager.mounts[index].volume_index;
        manager.volumes[volume_index].backend.stat(&self.path)
    }

    /// Releases this handle, allowing its mount to be unmounted.
    pub fn close(self, manager: &mut MountManager) -> Result<(), VfsError> {
        let index = manager
            .mounts
            .iter()
            .position(|m| m.info.id == self.mount_id)
            .ok_or(VfsError::MountNotFound)?;
        if manager.mounts[index].info.open_handles == 0 {
            return Err(VfsError::HandleClosed);
        }
        manager.mounts[index].info.open_handles -= 1;
        Ok(())
    }
}

fn split_drive(path: &str) -> Option<(String, &str)> {
    let mut chars = path.chars();
    let letter = chars.next()?;
    if !letter.is_ascii_alphabetic() || chars.next()? != ':' {
        return None;
    }
    let rest = &path[2..];
    if !rest.is_empty() && !rest.starts_with('/') && !rest.starts_with('\\') {
        return None;
    }
    Some((format!("{}:", letter.to_ascii_uppercase()), rest))
}

fn normalize_alias(alias: &str) -> Result<String, VfsError> {
    if let Some((drive, rest)) = split_drive(alias) {
        if !rest.is_empty() && rest != "/" && rest != "\\" {
            return Err(VfsError::InvalidPath);
        }
        return Ok(drive);
    }
    normalize_absolute(alias)
}

fn component_prefix(prefix: &str, path: &str) -> bool {
    prefix == "/"
        || prefix == path
        || match path.strip_prefix(prefix) {
            Some(rest) => rest.starts_with('/'),
            None => false,
        }
}

fn join_paths(base: &str, suffix: &str) -> Result<String, VfsError> {
    if base == "/" {
        return normalize_absolute(suffix);
    }
    let mut joined = String::from(base);
    if !suffix.starts_with('/') {
        joined.push('/');
    }
    joined.push_str(suffix);
    normalize_absolute(&joined)
}

fn aliases_conflict(mountpoint: &str, aliases: &[String]) -> bool {
    aliases.iter().any(|alias| {
        alias == mountpoint
            || component_prefix(alias, mountpoint)
            || component_prefix(mountpoint, alias)
    })
}

fn aliases_conflict_any(existing: &[String], mountpoint: &str, aliases: &[String]) -> bool {
    existing.iter().any(|old| {
        old == mountpoint
            || aliases.iter().any(|new| {
                old.eq_ignore_ascii_case(new)
                    || component_prefix(old, new)
                    || component_prefix(new, old)
            })
    })
}

/// Adapter implementations for filesystems from [`loos_fs`].
pub mod winpath;

pub mod adapters {
    use super::{FileSystem, VfsError};
    use alloc::vec::Vec;
    use loos_fs::fat::Fat;
    use loos_fs::iso9660::Iso;
    use loos_fs::ntfs::Ntfs;
    use loos_fs::{BlockDev, Entry};

    /// FAT12/16/32 adapter with read/write support.
    pub struct FatAdapter<D: BlockDev> {
        fs: Fat<D>,
    }

    impl<D: BlockDev> FatAdapter<D> {
        /// Mounts a FAT volume at a device byte offset.
        pub fn mount(dev: D, base: u64) -> Result<Self, VfsError> {
            Ok(Self {
                fs: Fat::mount(dev, base)?,
            })
        }

        /// Wraps an already-mounted FAT filesystem.
        pub fn new(fs: Fat<D>) -> Self {
            Self { fs }
        }

        /// Returns the wrapped filesystem.
        pub fn into_inner(self) -> Fat<D> {
            self.fs
        }
    }

    impl<D: BlockDev> FileSystem for FatAdapter<D> {
        fn stat(&mut self, path: &str) -> Result<Entry, VfsError> {
            let e = self.fs.stat(path)?;
            Ok(Entry {
                name: e.name,
                kind: e.kind,
                size: e.size as u64,
                mtime: e.mtime,
                mode: 0,
                uid: 0,
                gid: 0,
                target: None,
                id: e.first_cluster as u64,
            })
        }

        fn read_dir(&mut self, path: &str) -> Result<Vec<Entry>, VfsError> {
            self.fs
                .read_dir(path)?
                .into_iter()
                .map(|e| {
                    Ok(Entry {
                        name: e.name,
                        kind: e.kind,
                        size: e.size as u64,
                        mtime: e.mtime,
                        mode: 0,
                        uid: 0,
                        gid: 0,
                        target: None,
                        id: e.first_cluster as u64,
                    })
                })
                .collect()
        }

        fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
            Ok(self.fs.read_file(path, offset, buf)?)
        }

        fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<(), VfsError> {
            Ok(self.fs.write_at(path, offset, data)?)
        }

        fn flush(&mut self) -> Result<(), VfsError> {
            Ok(self.fs.sync()?)
        }
    }

    /// Read-only ISO 9660/Joliet/Rock Ridge adapter.
    pub struct IsoAdapter<D: BlockDev> {
        fs: Iso<D>,
    }

    impl<D: BlockDev> IsoAdapter<D> {
        /// Mounts an ISO image.
        pub fn mount(dev: D) -> Result<Self, VfsError> {
            Ok(Self {
                fs: Iso::mount(dev)?,
            })
        }

        /// Wraps an already-mounted ISO filesystem.
        pub fn new(fs: Iso<D>) -> Self {
            Self { fs }
        }
    }

    impl<D: BlockDev> FileSystem for IsoAdapter<D> {
        fn stat(&mut self, path: &str) -> Result<Entry, VfsError> {
            Ok(self.fs.stat(path)?)
        }
        fn read_dir(&mut self, path: &str) -> Result<Vec<Entry>, VfsError> {
            Ok(self.fs.read_dir(path)?)
        }
        fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
            Ok(self.fs.read_file(path, offset, buf)?)
        }
        fn is_read_only(&self) -> bool {
            true
        }
    }

    /// Read-only NTFS adapter.
    pub struct NtfsAdapter<D: BlockDev> {
        fs: Ntfs<D>,
    }

    impl<D: BlockDev> NtfsAdapter<D> {
        /// Mounts an NTFS volume at a device byte offset.
        pub fn mount(dev: D, base: u64) -> Result<Self, VfsError> {
            Ok(Self {
                fs: Ntfs::mount(dev, base)?,
            })
        }

        /// Wraps an already-mounted NTFS filesystem.
        pub fn new(fs: Ntfs<D>) -> Self {
            Self { fs }
        }
    }

    impl<D: BlockDev> FileSystem for NtfsAdapter<D> {
        fn stat(&mut self, path: &str) -> Result<Entry, VfsError> {
            Ok(self.fs.stat(path)?)
        }
        fn read_dir(&mut self, path: &str) -> Result<Vec<Entry>, VfsError> {
            Ok(self.fs.read_dir(path)?)
        }
        fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
            Ok(self.fs.read_file(path, offset, buf)?)
        }
        fn is_read_only(&self) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::adapters::IsoAdapter;
    use super::*;
    use alloc::vec;
    use loos_fs::FileKind;
    use loos_fs::MemDev;

    struct TestFs;

    impl FileSystem for TestFs {
        fn stat(&mut self, path: &str) -> Result<Entry, VfsError> {
            Ok(Entry {
                name: path.into(),
                kind: FileKind::File,
                size: 3,
                mtime: 0,
                mode: 0,
                uid: 0,
                gid: 0,
                target: None,
                id: 1,
            })
        }
        fn read_dir(&mut self, _path: &str) -> Result<Vec<Entry>, VfsError> {
            Ok(Vec::new())
        }
        fn read(&mut self, _path: &str, _offset: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
            let n = buf.len().min(3);
            buf[..n].copy_from_slice(&b"vfs"[..n]);
            Ok(n)
        }
        fn write(&mut self, _path: &str, _offset: u64, _data: &[u8]) -> Result<(), VfsError> {
            Ok(())
        }
    }

    #[test]
    fn normalization_clamps_parent_and_rejects_relative() {
        assert_eq!(normalize_absolute("/a/./b/../../c").unwrap(), "/c");
        assert_eq!(normalize_absolute("/../../").unwrap(), "/");
        assert_eq!(
            normalize_absolute("relative").unwrap_err(),
            VfsError::InvalidPath
        );
    }

    #[test]
    fn longest_mount_uses_component_boundaries() {
        let mut vfs = MountManager::new();
        let one = VolumeId::new([1; 16]);
        let two = VolumeId::new([2; 16]);
        vfs.register_volume(one, TestFs).unwrap();
        vfs.register_volume(two, TestFs).unwrap();
        vfs.mount(one, "/mnt", &[], false).unwrap();
        vfs.mount(two, "/mnt/data", &["D:"], false).unwrap();
        assert_eq!(vfs.stat("/mnt/data/file").unwrap().name, "/file");
        assert_eq!(vfs.stat("/mnt/data2/file").unwrap().name, "/data2/file");
        assert_eq!(vfs.stat("d:\\file").unwrap().name, "/file");
    }

    #[test]
    fn read_only_mount_and_busy_handle_block_mutation_and_unmount() {
        let mut vfs = MountManager::new();
        let id = VolumeId::new([3; 16]);
        vfs.register_volume(id, TestFs).unwrap();
        vfs.mount(id, "/", &[], true).unwrap();
        assert_eq!(
            vfs.write("/x", 0, b"x").unwrap_err(),
            VfsError::Fs(FsError::ReadOnly)
        );
        let handle = vfs.open("/x").unwrap();
        assert_eq!(vfs.unmount("/").unwrap_err(), VfsError::Busy);
        handle.close(&mut vfs).unwrap();
        vfs.unmount("/").unwrap();
    }

    #[test]
    fn unregistering_an_earlier_volume_preserves_mount_indices() {
        let mut vfs = MountManager::new();
        let first = VolumeId::new([4; 16]);
        let mounted = VolumeId::new([5; 16]);
        vfs.register_volume(first, TestFs).unwrap();
        vfs.register_volume(mounted, TestFs).unwrap();
        vfs.mount(mounted, "/data", &[], false).unwrap();
        vfs.unregister_volume(first).unwrap();
        assert_eq!(vfs.stat("/data/file").unwrap().name, "/file");
        vfs.unmount("/data").unwrap();
    }

    #[test]
    fn guid_parser_requires_exactly_sixteen_bytes() {
        let id = VolumeId::from_guid("00112233-4455-6677-8899-aabbccddeeff").unwrap();
        assert_eq!(id.bytes()[0], 0);
        assert!(VolumeId::from_guid("bad").is_err());
    }

    fn iso_record(out: &mut [u8], extent: u32, length: u32, flags: u8, name: &[u8]) -> usize {
        let len = 33 + name.len() + usize::from(name.len() % 2 == 0);
        out[0] = len as u8;
        out[1] = 0;
        out[2..6].copy_from_slice(&extent.to_le_bytes());
        out[6..10].copy_from_slice(&extent.to_be_bytes());
        out[10..14].copy_from_slice(&length.to_le_bytes());
        out[14..18].copy_from_slice(&length.to_be_bytes());
        out[18..25].copy_from_slice(&[124, 1, 1, 0, 0, 0, 0]);
        out[25] = flags;
        out[26] = 0;
        out[28..30].copy_from_slice(&1u16.to_le_bytes());
        out[30..32].copy_from_slice(&1u16.to_be_bytes());
        out[32] = name.len() as u8;
        out[33..33 + name.len()].copy_from_slice(name);
        len
    }

    #[test]
    fn iso_adapter_reads_a_real_in_memory_image() {
        let sector = 2048usize;
        let mut image = vec![0u8; sector * 24];
        let pvd = &mut image[16 * sector..17 * sector];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[6] = 1;
        pvd[40..48].copy_from_slice(b"VFS TEST");
        pvd[80..84].copy_from_slice(&(24u32).to_le_bytes());
        pvd[84..88].copy_from_slice(&(24u32).to_be_bytes());
        pvd[128..130].copy_from_slice(&(2048u16).to_le_bytes());
        pvd[130..132].copy_from_slice(&(2048u16).to_be_bytes());
        iso_record(&mut pvd[156..190], 20, sector as u32, 2, &[0]);
        let term = &mut image[17 * sector..18 * sector];
        term[0] = 255;
        term[1..6].copy_from_slice(b"CD001");
        term[6] = 1;
        let root = &mut image[20 * sector..21 * sector];
        let mut at = 0;
        at += iso_record(&mut root[at..], 20, sector as u32, 2, &[0]);
        at += iso_record(&mut root[at..], 20, sector as u32, 2, &[1]);
        let _ = iso_record(&mut root[at..], 21, 5, 0, b"HELLO.TXT;1");
        image[21 * sector..21 * sector + 5].copy_from_slice(b"hello");

        let backend = IsoAdapter::mount(MemDev::from_image(image)).unwrap();
        let id = VolumeId::new([9; 16]);
        let mut vfs = MountManager::new();
        vfs.register_volume(id, backend).unwrap();
        vfs.mount(id, "/cd", &["E:"], false).unwrap();
        let mut buf = [0u8; 5];
        assert_eq!(vfs.read("e:\\hello.txt", 0, &mut buf).unwrap(), 5);
        assert_eq!(&buf, b"hello");
        assert_eq!(
            vfs.write("/cd/HELLO.TXT", 0, b"x").unwrap_err(),
            VfsError::Fs(FsError::ReadOnly)
        );
    }
}

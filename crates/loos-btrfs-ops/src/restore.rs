//! `btrfs restore` equivalent: copy files out of a (possibly damaged) filesystem through a
//! [`RestoreSink`], without ever writing to the source.
use crate::consts::*;
use crate::dev::Dev;
use crate::error::{Error, Result};
use crate::fs::{DirEnt, Fs, Kind, Node};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Where restored objects go.
pub trait RestoreSink {
    /// Creates a directory (parents exist already).
    fn mkdir(&mut self, path: &str) -> Result<()>;
    /// Creates an empty regular file of final size `size` (sparse).
    fn create_file(&mut self, path: &str, size: u64) -> Result<()>;
    /// Writes file data.
    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<()>;
    /// Creates a symlink.
    fn symlink(&mut self, path: &str, target: &[u8]) -> Result<()>;
    /// Sets an extended attribute (sinks without xattr support return `Unsupported`).
    fn set_xattr(&mut self, path: &str, name: &[u8], value: &[u8]) -> Result<()>;
    /// Applies mode/owner/times after the object is complete.
    fn set_meta(&mut self, path: &str, mode: u32, uid: u32, gid: u32, mtime_sec: u64) -> Result<()>;
}

/// What to restore.
#[derive(Clone, Copy, Debug)]
pub struct RestoreOptions {
    /// Descend into subvolumes/snapshots found on the way.
    pub snapshots: bool,
    /// Restore extended attributes.
    pub xattrs: bool,
    /// Restore symlinks.
    pub symlinks: bool,
    /// Restore mode/owner/times.
    pub metadata: bool,
    /// Keep going after read/checksum errors (the affected file is truncated/zeroed there).
    pub ignore_errors: bool,
    /// Verify data checksums while reading.
    pub verify: bool,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        RestoreOptions { snapshots: false, xattrs: true, symlinks: true, metadata: true, ignore_errors: false, verify: true }
    }
}

/// Totals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreStats {
    /// Directories created.
    pub dirs: u64,
    /// Files created.
    pub files: u64,
    /// Symlinks created.
    pub symlinks: u64,
    /// Bytes of file data written.
    pub bytes: u64,
    /// Subvolumes skipped because `snapshots` was off.
    pub subvolumes_skipped: u64,
    /// Device nodes, FIFOs, sockets skipped.
    pub special_skipped: u64,
    /// xattrs the sink refused.
    pub xattrs_skipped: u64,
    /// Paths that could not be restored completely.
    pub errors: Vec<String>,
}

fn join(base: &str, name: &[u8]) -> String {
    let n = String::from_utf8_lossy(name);
    if base.is_empty() {
        n.into_owned()
    } else {
        let mut s = String::from(base);
        s.push('/');
        s.push_str(&n);
        s
    }
}

/// Restores the tree below `path` (use `""` for everything) into `sink`.
pub fn restore<D: Dev, S: RestoreSink>(fs: &mut Fs<D>, path: &str, sink: &mut S, opts: &RestoreOptions) -> Result<RestoreStats> {
    let mut stats = RestoreStats::default();
    let was = fs.verify_data;
    fs.verify_data = opts.verify;
    let start = fs.resolve(path)?;
    let st = fs.stat(start)?;
    let rel = String::from(path.trim_matches('/'));
    let r = match st.kind() {
        Kind::Dir => {
            if !rel.is_empty() {
                sink.mkdir(&rel)?;
                stats.dirs += 1;
            }
            restore_dir(fs, start, &rel, sink, opts, &mut stats)
        }
        _ => restore_entry(fs, start, &rel, st.kind(), sink, opts, &mut stats),
    };
    fs.verify_data = was;
    r?;
    Ok(stats)
}

fn restore_dir<D: Dev, S: RestoreSink>(fs: &mut Fs<D>, dir: Node, rel: &str, sink: &mut S, opts: &RestoreOptions, stats: &mut RestoreStats) -> Result<()> {
    let entries = match fs.readdir(dir) {
        Ok(e) => e,
        Err(e) => {
            if opts.ignore_errors {
                stats.errors.push(alloc::format!("{}: {}", rel, e));
                return Ok(());
            }
            return Err(e);
        }
    };
    for e in entries {
        let child = join(rel, &e.name);
        match restore_one(fs, &e, &child, sink, opts, stats) {
            Ok(()) => {}
            Err(err) => {
                if opts.ignore_errors {
                    stats.errors.push(alloc::format!("{}: {}", child, err));
                } else {
                    return Err(err);
                }
            }
        }
    }
    if opts.metadata {
        if let Ok(st) = fs.stat(dir) {
            if !rel.is_empty() {
                let _ = sink.set_meta(rel, st.mode & 0o7777, st.uid, st.gid, st.mtime.sec);
            }
        }
    }
    Ok(())
}

fn restore_one<D: Dev, S: RestoreSink>(fs: &mut Fs<D>, e: &DirEnt, path: &str, sink: &mut S, opts: &RestoreOptions, stats: &mut RestoreStats) -> Result<()> {
    match e.kind {
        Kind::Subvolume => {
            if !opts.snapshots {
                stats.subvolumes_skipped += 1;
                return Ok(());
            }
            let ri = fs.vol.root_item(e.node.subvol)?;
            let node = Node { subvol: e.node.subvol, ino: ri.root_dirid };
            sink.mkdir(path)?;
            stats.dirs += 1;
            restore_dir(fs, node, path, sink, opts, stats)
        }
        Kind::Dir => {
            sink.mkdir(path)?;
            stats.dirs += 1;
            restore_dir(fs, e.node, path, sink, opts, stats)
        }
        k => restore_entry(fs, e.node, path, k, sink, opts, stats),
    }
}

fn restore_entry<D: Dev, S: RestoreSink>(fs: &mut Fs<D>, node: Node, path: &str, kind: Kind, sink: &mut S, opts: &RestoreOptions, stats: &mut RestoreStats) -> Result<()> {
    match kind {
        Kind::File => {
            let st = fs.stat(node)?;
            sink.create_file(path, st.size)?;
            stats.files += 1;
            let mut off = 0u64;
            let mut buf = vec![0u8; 1 << 20];
            while off < st.size {
                let want = core::cmp::min(buf.len() as u64, st.size - off) as usize;
                match fs.read(node, off, &mut buf[..want]) {
                    Ok(n) if n > 0 => {
                        // skip all-zero runs so sparse files stay sparse
                        for (i, chunk) in buf[..n].chunks(4096).enumerate() {
                            if chunk.iter().any(|&b| b != 0) {
                                sink.write(path, off + (i * 4096) as u64, chunk)?;
                            }
                        }
                        stats.bytes += n as u64;
                        off += n as u64;
                    }
                    Ok(_) => break,
                    Err(e) => {
                        if !opts.ignore_errors {
                            return Err(e);
                        }
                        // retry sector by sector so only the damaged sectors become holes
                        let mut bad_first: Option<u64> = None;
                        let mut pos = 0usize;
                        while pos < want {
                            let n = core::cmp::min(4096, want - pos);
                            match fs.read(node, off + pos as u64, &mut buf[pos..pos + n]) {
                                Ok(got) if got == n => {
                                    if buf[pos..pos + n].iter().any(|&b| b != 0) {
                                        sink.write(path, off + pos as u64, &buf[pos..pos + n])?;
                                    }
                                    stats.bytes += n as u64;
                                }
                                _ => {
                                    bad_first.get_or_insert(off + pos as u64);
                                }
                            }
                            pos += n;
                        }
                        stats.errors.push(alloc::format!("{}: unreadable data starting at offset {}: {}", path, bad_first.unwrap_or(off), e));
                        off += want as u64;
                    }
                }
            }
        }
        Kind::Symlink => {
            if opts.symlinks {
                let t = fs.readlink(node)?;
                sink.symlink(path, &t)?;
                stats.symlinks += 1;
            }
        }
        Kind::Dir => unreachable!(),
        Kind::Subvolume => unreachable!(),
        _ => {
            stats.special_skipped += 1;
            return Ok(());
        }
    }
    if opts.xattrs {
        if let Ok(list) = fs.xattrs(node) {
            for (n, v) in list {
                if sink.set_xattr(path, &n, &v).is_err() {
                    stats.xattrs_skipped += 1;
                }
            }
        }
    }
    if opts.metadata && kind == Kind::File {
        if let Ok(st) = fs.stat(node) {
            let _ = sink.set_meta(path, st.mode & 0o7777, st.uid, st.gid, st.mtime.sec);
        }
    }
    Ok(())
}

/// Host-directory sink (tools and tests).
#[cfg(any(feature = "std", test))]
pub struct DirSink {
    /// Destination root.
    pub root: std::path::PathBuf,
}

#[cfg(any(feature = "std", test))]
impl DirSink {
    fn p(&self, rel: &str) -> std::path::PathBuf {
        self.root.join(rel)
    }
}

#[cfg(any(feature = "std", test))]
impl RestoreSink for DirSink {
    fn mkdir(&mut self, path: &str) -> Result<()> {
        std::fs::create_dir_all(self.p(path)).map_err(|_| Error::Io)
    }
    fn create_file(&mut self, path: &str, size: u64) -> Result<()> {
        let f = std::fs::File::create(self.p(path)).map_err(|_| Error::Io)?;
        f.set_len(size).map_err(|_| Error::Io)
    }
    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new().write(true).open(self.p(path)).map_err(|_| Error::Io)?;
        f.seek(SeekFrom::Start(offset)).map_err(|_| Error::Io)?;
        f.write_all(data).map_err(|_| Error::Io)
    }
    fn symlink(&mut self, path: &str, target: &[u8]) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), self.p(path)).map_err(|_| Error::Io)
        }
        #[cfg(not(unix))]
        {
            let _ = (path, target);
            Err(Error::Unsupported("symlinks on this host"))
        }
    }
    fn set_xattr(&mut self, _path: &str, _name: &[u8], _value: &[u8]) -> Result<()> {
        Err(Error::Unsupported("xattrs"))
    }
    fn set_meta(&mut self, path: &str, mode: u32, _uid: u32, _gid: u32, _mtime_sec: u64) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(self.p(path), std::fs::Permissions::from_mode(mode));
        }
        let _ = mode;
        // Rust 1.63 has no stable portable file-mtime setter.  Preserve mode
        // here; a platform-specific sink can apply timestamps after restore.
        Ok(())
    }
}

#[allow(dead_code)]
const _UNUSED: u64 = FIRST_FREE_OBJECTID;

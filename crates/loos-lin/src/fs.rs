//! Linux view of the filesystem. `LinFs` is implemented over the LoOS VFS/btrfs
//! (`\Lin` root); `MemFs` is the in-memory implementation used by tests and the
//! initial ramdisk.

#[allow(unused_imports)]
use alloc::borrow::Cow;
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::errno::*;
use alloc::collections::BTreeMap;

/// File kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Regular file.
    File,
    /// Directory.
    Dir,
    /// Symlink.
    Symlink,
    /// Character device (virtual /dev nodes).
    Char,
    /// Pipe.
    Fifo,
}

/// `stat` result.
#[derive(Clone, Copy, Debug)]
pub struct Stat {
    /// Inode number.
    pub ino: u64,
    /// Kind.
    pub kind: Kind,
    /// Permission bits.
    pub mode: u32,
    /// Size in bytes.
    pub size: u64,
    /// Link count.
    pub nlink: u64,
    /// Owner.
    pub uid: u32,
    /// Group.
    pub gid: u32,
    /// Modification time (seconds).
    pub mtime: i64,
}

impl Stat {
    /// st_mode including S_IFMT.
    pub fn st_mode(&self) -> u32 {
        let t = match self.kind {
            Kind::File => 0o100000,
            Kind::Dir => 0o040000,
            Kind::Symlink => 0o120000,
            Kind::Char => 0o020000,
            Kind::Fifo => 0o010000,
        };
        t | (self.mode & 0o7777)
    }
}

/// Directory entry.
#[derive(Clone, Debug)]
pub struct DirEnt {
    /// Name.
    pub name: String,
    /// Inode.
    pub ino: u64,
    /// Kind.
    pub kind: Kind,
}

/// Filesystem operations needed by the Linux layer. Paths are absolute, normalised, with no
/// symlink components except possibly the last (resolution is done by [`resolve`]).
pub trait LinFs {
    /// `lstat`.
    fn lstat(&mut self, path: &str) -> Result<Stat, i64>;
    /// Read file bytes.
    fn read(&mut self, path: &str, off: u64, buf: &mut [u8]) -> Result<usize, i64>;
    /// Write file bytes (extends, zero-filling holes).
    fn write(&mut self, path: &str, off: u64, data: &[u8]) -> Result<usize, i64>;
    /// Create a regular file (fails with `EEXIST`).
    fn create(&mut self, path: &str, mode: u32) -> Result<(), i64>;
    /// Truncate to `len`.
    fn truncate(&mut self, path: &str, len: u64) -> Result<(), i64>;
    /// `mkdir`.
    fn mkdir(&mut self, path: &str, mode: u32) -> Result<(), i64>;
    /// `unlink` / `rmdir`.
    fn remove(&mut self, path: &str, dir: bool) -> Result<(), i64>;
    /// `rename`.
    fn rename(&mut self, from: &str, to: &str) -> Result<(), i64>;
    /// `readlink`.
    fn readlink(&mut self, path: &str) -> Result<String, i64>;
    /// `symlink`.
    fn symlink(&mut self, target: &str, path: &str) -> Result<(), i64>;
    /// Directory listing (without `.` / `..`).
    fn list(&mut self, path: &str) -> Result<Vec<DirEnt>, i64>;
    /// `link`: the backends store files by path, so a hard link is a snapshot copy (enough for the
    /// create-then-link-into-place pattern of sem_open/lockfiles; later writes are not shared).
    fn link(&mut self, from: &str, to: &str) -> Result<(), i64> {
        let st = self.lstat(from)?;
        if st.kind == Kind::Dir {
            return Err(crate::errno::EPERM);
        }
        if self.lstat(to).is_ok() {
            return Err(crate::errno::EEXIST);
        }
        let mut data = alloc::vec![0u8; st.size as usize];
        let n = self.read(from, 0, &mut data)?;
        self.create(to, st.mode)?;
        self.write(to, 0, &data[..n])?;
        Ok(())
    }
}

/// Normalise `path` against `cwd` (handles `.`/`..`/repeated slashes); never escapes `/`.
pub fn normalize(cwd: &str, path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let full = if path.starts_with('/') { path.to_string() } else { format!("{cwd}/{path}") };
    for c in full.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    format!("/{}", parts.join("/"))
}

/// Resolve symlinks (up to 40 hops) in every component; `follow_last` controls the final one.
pub fn resolve(fs: &mut dyn LinFs, cwd: &str, path: &str, follow_last: bool) -> Result<String, i64> {
    if path.is_empty() {
        return Err(ENOENT);
    }
    if path.len() > 4096 {
        return Err(ENAMETOOLONG);
    }
    let mut hops = 0;
    let mut pending: Vec<String> = normalize_keep(cwd, path);
    let mut cur: Vec<String> = Vec::new();
    pending.reverse();
    while let Some(c) = pending.pop() {
        match c.as_str() {
            "" | "." => continue,
            ".." => {
                cur.pop();
                continue;
            }
            _ => {}
        }
        cur.push(c);
        let p = format!("/{}", cur.join("/"));
        let last = pending.iter().all(|x| x.is_empty() || x == ".");
        if last && !follow_last {
            break;
        }
        match fs.lstat(&p) {
            Ok(st) if st.kind == Kind::Symlink => {
                hops += 1;
                if hops > 40 {
                    return Err(ELOOP);
                }
                let t = fs.readlink(&p)?;
                cur.pop();
                if t.starts_with('/') {
                    cur.clear();
                }
                let mut comps: Vec<String> = t.split('/').map(String::from).collect();
                comps.reverse();
                pending.extend(comps);
            }
            Ok(_) => {}
            Err(e) if e == ENOENT && last => {}
            Err(e) => return Err(e),
        }
    }
    Ok(format!("/{}", cur.join("/")))
}

fn normalize_keep(cwd: &str, path: &str) -> Vec<String> {
    let full = if path.starts_with('/') { path.to_string() } else { format!("{cwd}/{path}") };
    full.split('/').map(String::from).collect()
}

enum Node {
    /// Contents: owned, or borrowed from the boot image (copied on the first write).
    File(Cow<'static, [u8]>),
    Dir,
    Link(String),
}

struct Ent {
    node: Node,
    mode: u32,
    ino: u64,
}

/// In-memory filesystem.
pub struct MemFs {
    ents: BTreeMap<String, Ent>,
    next_ino: u64,
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

impl MemFs {
    /// New fs containing `/`.
    pub fn new() -> Self {
        let mut ents = BTreeMap::new();
        ents.insert("/".to_string(), Ent { node: Node::Dir, mode: 0o755, ino: 1 });
        Self { ents, next_ino: 2 }
    }
    fn parent(p: &str) -> String {
        match p.rfind('/') {
            Some(0) | None => "/".into(),
            Some(i) => p[..i].to_string(),
        }
    }
    fn add(&mut self, p: &str, node: Node, mode: u32) {
        let mut cur = Self::parent(p);
        let mut missing = Vec::new();
        while !self.ents.contains_key(&cur) {
            missing.push(cur.clone());
            cur = Self::parent(&cur);
        }
        for d in missing.into_iter().rev() {
            self.insert(&d, Node::Dir, 0o755);
        }
        self.insert(p, node, mode);
    }
    fn insert(&mut self, p: &str, node: Node, mode: u32) {
        let ino = self.next_ino;
        self.next_ino += 1;
        self.ents.insert(p.to_string(), Ent { node, mode, ino });
    }
    /// Add (or replace) a file, creating parent directories.
    pub fn add_file(&mut self, path: &str, data: &[u8], mode: u32) {
        self.add(path, Node::File(Cow::Owned(data.to_vec())), mode);
    }
    /// Add a symlink.
    pub fn add_symlink(&mut self, path: &str, target: &str) {
        self.add(path, Node::Link(target.to_string()), 0o777);
    }
    /// Add a directory.
    pub fn add_dir(&mut self, path: &str) {
        self.add(path, Node::Dir, 0o755);
    }
    /// Every regular file as `(absolute path, contents)`.
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.ents.iter().filter_map(|(p, e)| match &e.node {
            Node::File(d) => Some((p.as_str(), &d[..])),
            _ => None,
        })
    }
    /// Whole-file read helper.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        match &self.ents.get(path)?.node {
            Node::File(d) => Some(d),
            _ => None,
        }
    }
}

impl MemFs {
    /// Populate from a ustar archive (initrd rootfs). Returns the number of entries added.
    /// Unknown entry types are skipped; malformed headers end the archive.
    pub fn load_tar(&mut self, tar: &[u8]) -> usize {
        self.load_tar_with(tar, |data| Cow::Owned(data.to_vec()))
    }

    /// [`Self::load_tar`] for an archive that stays in memory for good (the boot module): file
    /// contents are not copied, only borrowed until something writes to them.
    pub fn load_tar_static(&mut self, tar: &'static [u8]) -> usize {
        // SAFETY of the borrow: every slice handed out is a sub-slice of `tar` ('static).
        let base = tar.as_ptr() as usize;
        self.load_tar_with(tar, |data| {
            let off = data.as_ptr() as usize - base;
            Cow::Borrowed(&tar[off..off + data.len()])
        })
    }

    fn load_tar_with(&mut self, tar: &[u8], mut keep: impl FnMut(&[u8]) -> Cow<'static, [u8]>) -> usize {
        let mut off = 0;
        let mut n = 0;
        while off + 512 <= tar.len() {
            let h = &tar[off..off + 512];
            if h.iter().all(|&b| b == 0) {
                break;
            }
            let cstr = |a: usize, b: usize| {
                let f = &h[a..b];
                let e = f.iter().position(|&c| c == 0).unwrap_or(f.len());
                String::from_utf8_lossy(&f[..e]).into_owned()
            };
            let oct = |a: usize, b: usize| u64::from_str_radix(cstr(a, b).trim(), 8).ok();
            let (Some(size), Some(mode)) = (oct(124, 136), oct(100, 108)) else { break };
            let prefix = cstr(345, 500);
            let name = if prefix.is_empty() { cstr(0, 100) } else { format!("{prefix}/{}", cstr(0, 100)) };
            let path = normalize("/", &name);
            let body = off + 512;
            let Some(end) = body.checked_add(size as usize).filter(|&e| e <= tar.len()) else { break };
            match h[156] {
                b'0' | 0 => {
                    self.add(&path, Node::File(keep(&tar[body..end])), (mode & 0o7777) as u32);
                    n += 1;
                }
                b'5' => {
                    self.add_dir(&path);
                    n += 1;
                }
                b'2' => {
                    self.add_symlink(&path, &cstr(157, 257));
                    n += 1;
                }
                _ => {}
            }
            off = body + (size as usize).div_ceil(512) * 512;
        }
        n
    }
}

impl LinFs for MemFs {
    fn lstat(&mut self, path: &str) -> Result<Stat, i64> {
        let e = self.ents.get(path).ok_or(ENOENT)?;
        let (kind, size) = match &e.node {
            Node::File(d) => (Kind::File, d.len() as u64),
            Node::Dir => (Kind::Dir, 4096),
            Node::Link(t) => (Kind::Symlink, t.len() as u64),
        };
        Ok(Stat { ino: e.ino, kind, mode: e.mode, size, nlink: 1, uid: 0, gid: 0, mtime: 1_700_000_000 })
    }
    fn read(&mut self, path: &str, off: u64, buf: &mut [u8]) -> Result<usize, i64> {
        match &self.ents.get(path).ok_or(ENOENT)?.node {
            Node::File(d) => {
                let off = off.min(d.len() as u64) as usize;
                let n = buf.len().min(d.len() - off);
                buf[..n].copy_from_slice(&d[off..off + n]);
                Ok(n)
            }
            Node::Dir => Err(EISDIR),
            Node::Link(_) => Err(EINVAL),
        }
    }
    fn write(&mut self, path: &str, off: u64, data: &[u8]) -> Result<usize, i64> {
        match &mut self.ents.get_mut(path).ok_or(ENOENT)?.node {
            Node::File(d) => {
                let end = (off as usize).checked_add(data.len()).filter(|&e| e <= 1 << 30).ok_or(EINVAL)?;
                let d = d.to_mut();
                if d.len() < end {
                    d.resize(end, 0);
                }
                d[off as usize..end].copy_from_slice(data);
                Ok(data.len())
            }
            Node::Dir => Err(EISDIR),
            Node::Link(_) => Err(EINVAL),
        }
    }
    fn create(&mut self, path: &str, mode: u32) -> Result<(), i64> {
        if self.ents.contains_key(path) {
            return Err(EEXIST);
        }
        match self.ents.get(&Self::parent(path)) {
            Some(Ent { node: Node::Dir, .. }) => {}
            Some(_) => return Err(ENOTDIR),
            None => return Err(ENOENT),
        }
        self.insert(path, Node::File(Cow::Owned(Vec::new())), mode);
        Ok(())
    }
    fn truncate(&mut self, path: &str, len: u64) -> Result<(), i64> {
        match &mut self.ents.get_mut(path).ok_or(ENOENT)?.node {
            Node::File(d) => {
                d.to_mut().resize(len.min(1 << 30) as usize, 0);
                Ok(())
            }
            _ => Err(EINVAL),
        }
    }
    fn mkdir(&mut self, path: &str, mode: u32) -> Result<(), i64> {
        if self.ents.contains_key(path) {
            return Err(EEXIST);
        }
        match self.ents.get(&Self::parent(path)) {
            Some(Ent { node: Node::Dir, .. }) => {}
            Some(_) => return Err(ENOTDIR),
            None => return Err(ENOENT),
        }
        self.insert(path, Node::Dir, mode);
        Ok(())
    }
    fn remove(&mut self, path: &str, dir: bool) -> Result<(), i64> {
        let e = self.ents.get(path).ok_or(ENOENT)?;
        match (&e.node, dir) {
            (Node::Dir, false) => return Err(EISDIR),
            (Node::Dir, true) => {
                let pre = format!("{}/", path.trim_end_matches('/'));
                if self.ents.keys().any(|k| k.starts_with(&pre)) {
                    return Err(ENOTEMPTY);
                }
            }
            (_, true) => return Err(ENOTDIR),
            _ => {}
        }
        if path == "/" {
            return Err(EPERM);
        }
        self.ents.remove(path);
        Ok(())
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), i64> {
        if !self.ents.contains_key(from) {
            return Err(ENOENT);
        }
        let pre = format!("{from}/");
        let moved: Vec<String> = self.ents.keys().filter(|k| *k == from || k.starts_with(&pre)).cloned().collect();
        if to.starts_with(&pre) {
            return Err(EINVAL);
        }
        if !self.ents.contains_key(&Self::parent(to)) {
            return Err(ENOENT);
        }
        for k in moved {
            let e = self.ents.remove(&k).unwrap();
            self.ents.insert(format!("{to}{}", &k[from.len()..]), e);
        }
        Ok(())
    }
    fn readlink(&mut self, path: &str) -> Result<String, i64> {
        match &self.ents.get(path).ok_or(ENOENT)?.node {
            Node::Link(t) => Ok(t.clone()),
            _ => Err(EINVAL),
        }
    }
    fn symlink(&mut self, target: &str, path: &str) -> Result<(), i64> {
        if self.ents.contains_key(path) {
            return Err(EEXIST);
        }
        self.insert(path, Node::Link(target.into()), 0o777);
        Ok(())
    }
    fn list(&mut self, path: &str) -> Result<Vec<DirEnt>, i64> {
        match self.ents.get(path).ok_or(ENOENT)?.node {
            Node::Dir => {}
            _ => return Err(ENOTDIR),
        }
        let pre = if path == "/" { "/".to_string() } else { format!("{path}/") };
        Ok(self
            .ents
            .range(pre.clone()..)
            .take_while(|(k, _)| k.starts_with(&pre))
            .filter(|(k, _)| k.len() > pre.len() && !k[pre.len()..].contains('/'))
            .map(|(k, e)| DirEnt {
                name: k[pre.len()..].to_string(),
                ino: e.ino,
                kind: match e.node {
                    Node::File(_) => Kind::File,
                    Node::Dir => Kind::Dir,
                    Node::Link(_) => Kind::Symlink,
                },
            })
            .collect())
    }
}

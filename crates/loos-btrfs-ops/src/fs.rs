//! Read-only filesystem view over a [`Volume`]: inodes, directories, extents (inline, regular,
//! prealloc, holes, all compression types), data checksums with mirror/parity fallback, xattrs,
//! symlinks, subvolumes and snapshots.
use crate::compress;
use crate::consts::*;
use crate::error::{Error, Result};
use crate::volume::*;
use crate::dev::Dev;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use loos_btrfs::crc32c_raw;

/// `btrfs_name_hash`: the key offset of `DIR_ITEM`/`XATTR_ITEM` for a name.
pub fn name_hash(name: &[u8]) -> u64 {
    crc32c_raw(0xffff_fffe, name) as u64
}

/// A filesystem object: (subvolume id, inode number).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Node {
    /// Subvolume (tree) id.
    pub subvol: u64,
    /// Inode number inside the subvolume.
    pub ino: u64,
}

/// Kind of a directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Regular file.
    File,
    /// Directory.
    Dir,
    /// Symlink.
    Symlink,
    /// Character device.
    CharDev,
    /// Block device.
    BlockDev,
    /// FIFO.
    Fifo,
    /// Socket.
    Socket,
    /// A subvolume mount point (entry points at another tree).
    Subvolume,
}

impl Kind {
    fn from_ft(t: u8) -> Kind {
        match t {
            FT_DIR => Kind::Dir,
            FT_SYMLINK => Kind::Symlink,
            FT_CHRDEV => Kind::CharDev,
            FT_BLKDEV => Kind::BlockDev,
            FT_FIFO => Kind::Fifo,
            FT_SOCK => Kind::Socket,
            _ => Kind::File,
        }
    }
}

/// Timestamp (seconds, nanoseconds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timespec {
    /// Seconds since the Unix epoch.
    pub sec: u64,
    /// Nanoseconds.
    pub nsec: u32,
}

/// `btrfs_inode_item`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inode {
    /// Inode number.
    pub ino: u64,
    /// Creation generation.
    pub generation: u64,
    /// Last-modified transaction.
    pub transid: u64,
    /// Size in bytes.
    pub size: u64,
    /// Allocated bytes.
    pub nbytes: u64,
    /// Link count.
    pub nlink: u32,
    /// Owner.
    pub uid: u32,
    /// Group.
    pub gid: u32,
    /// POSIX mode (type + permissions).
    pub mode: u32,
    /// Device number.
    pub rdev: u64,
    /// `INODE_*` flags.
    pub flags: u64,
    /// Access time.
    pub atime: Timespec,
    /// Change time.
    pub ctime: Timespec,
    /// Modification time.
    pub mtime: Timespec,
    /// Creation time.
    pub otime: Timespec,
}

impl Inode {
    /// Parses a 160-byte inode item.
    pub fn parse(ino: u64, b: &[u8]) -> Result<Inode> {
        if b.len() < 160 {
            return Err(Error::Corrupt("inode item size"));
        }
        let ts = |o: usize| Timespec { sec: le64(b, o), nsec: le32(b, o + 8) };
        Ok(Inode {
            ino,
            generation: le64(b, 0),
            transid: le64(b, 8),
            size: le64(b, 16),
            nbytes: le64(b, 24),
            nlink: le32(b, 40),
            uid: le32(b, 44),
            gid: le32(b, 48),
            mode: le32(b, 52),
            rdev: le64(b, 56),
            flags: le64(b, 64),
            atime: ts(112),
            ctime: ts(124),
            mtime: ts(136),
            otime: ts(148),
        })
    }
    /// Kind derived from the mode bits.
    pub fn kind(&self) -> Kind {
        match self.mode & 0o170000 {
            0o040000 => Kind::Dir,
            0o120000 => Kind::Symlink,
            0o020000 => Kind::CharDev,
            0o060000 => Kind::BlockDev,
            0o010000 => Kind::Fifo,
            0o140000 => Kind::Socket,
            _ => Kind::File,
        }
    }
}

/// One directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEnt {
    /// Name bytes (UTF-8 in practice).
    pub name: Vec<u8>,
    /// Target object (for subvolume entries `ino` is 256, the subvolume's root dir).
    pub node: Node,
    /// Entry kind.
    pub kind: Kind,
    /// `DIR_INDEX` position.
    pub index: u64,
}

/// A subvolume or snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubvolInfo {
    /// Tree id.
    pub id: u64,
    /// Parent subvolume id (0 for the top-level).
    pub parent: u64,
    /// Name inside the parent directory.
    pub name: String,
    /// Directory inode inside the parent that holds the entry.
    pub dirid: u64,
    /// Root item.
    pub root: RootItem,
    /// Full path from the top-level subvolume.
    pub path: String,
}

/// A decoded file extent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileExtent {
    /// File offset the extent starts at.
    pub file_offset: u64,
    /// Inline payload / on-disk reference.
    pub kind: ExtentKind,
}

/// What a file extent item points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtentKind {
    /// Data stored in the leaf.
    Inline {
        /// Compression type.
        compression: u8,
        /// Uncompressed length.
        ram_bytes: u64,
        /// Stored bytes.
        data: Vec<u8>,
    },
    /// Regular or preallocated extent.
    Regular {
        /// Preallocated (reads as zeros)?
        prealloc: bool,
        /// Compression type.
        compression: u8,
        /// Logical address of the on-disk extent (0 = hole).
        disk_bytenr: u64,
        /// On-disk (compressed) size.
        disk_num_bytes: u64,
        /// Offset into the (uncompressed) extent.
        offset: u64,
        /// Bytes of file covered.
        num_bytes: u64,
        /// Uncompressed extent size.
        ram_bytes: u64,
    },
}

impl FileExtent {
    /// Number of file bytes this extent covers.
    pub fn len(&self) -> u64 {
        match &self.kind {
            ExtentKind::Inline { ram_bytes, .. } => *ram_bytes,
            ExtentKind::Regular { num_bytes, .. } => *num_bytes,
        }
    }
    /// True if it covers no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Decodes a file extent item.
pub fn parse_file_extent(file_offset: u64, b: &[u8]) -> Result<FileExtent> {
    if b.len() < 21 {
        return Err(Error::Corrupt("file extent size"));
    }
    let compression = b[16];
    let ram_bytes = le64(b, 8);
    match b[20] {
        FILE_EXTENT_INLINE => Ok(FileExtent { file_offset, kind: ExtentKind::Inline { compression, ram_bytes, data: b[21..].to_vec() } }),
        FILE_EXTENT_REG | FILE_EXTENT_PREALLOC => {
            if b.len() < 53 {
                return Err(Error::Corrupt("file extent size"));
            }
            if b[17] != 0 || le16(b, 18) != 0 {
                return Err(Error::Unsupported("encrypted/encoded extent"));
            }
            Ok(FileExtent {
                file_offset,
                kind: ExtentKind::Regular {
                    prealloc: b[20] == FILE_EXTENT_PREALLOC,
                    compression,
                    disk_bytenr: le64(b, 21),
                    disk_num_bytes: le64(b, 29),
                    offset: le64(b, 37),
                    num_bytes: le64(b, 45),
                    ram_bytes,
                },
            })
        }
        _ => Err(Error::Corrupt("file extent type")),
    }
}

/// Parses the concatenated `btrfs_dir_item` records of a `DIR_ITEM`/`DIR_INDEX`/`XATTR_ITEM` payload.
pub fn parse_dir_items(b: &[u8]) -> Result<Vec<(Key, u8, Vec<u8>, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if i + 30 > b.len() {
            return Err(Error::Corrupt("dir item header"));
        }
        let loc = Key::parse(&b[i..]);
        let data_len = le16(b, i + 25) as usize;
        let name_len = le16(b, i + 27) as usize;
        let ty = b[i + 29];
        let end = i + 30 + name_len + data_len;
        if end > b.len() {
            return Err(Error::Corrupt("dir item bounds"));
        }
        out.push((loc, ty, b[i + 30..i + 30 + name_len].to_vec(), b[i + 30 + name_len..end].to_vec()));
        i = end;
    }
    Ok(out)
}

/// Read-only filesystem handle.
pub struct Fs<D: Dev> {
    /// Underlying volume.
    pub vol: Volume<D>,
    roots: BTreeMap<u64, TreeRef>,
    /// Verify data checksums on read (default on).
    pub verify_data: bool,
    /// Case-insensitive name matching (Win32 layer); exact match is always preferred.
    pub case_insensitive: bool,
    csum_root: Option<TreeRef>,
    extent_cache: Option<(u64, u8, Vec<u8>)>,
    /// Subvolume that path `/` refers to.
    pub default_subvol: u64,
}

impl<D: Dev> Fs<D> {
    /// Opens the filesystem on the volume's default subvolume.
    pub fn open(mut vol: Volume<D>) -> Result<Fs<D>> {
        let default = Self::find_default_subvol(&mut vol)?;
        let csum_root = vol.tree_root(CSUM_TREE_OBJECTID).ok();
        Ok(Fs { vol, roots: BTreeMap::new(), verify_data: true, case_insensitive: false, csum_root, extent_cache: None, default_subvol: default })
    }

    /// Opens with explicitly supplied tree roots (root tree unreadable): subvolume 5 is `/`, its root
    /// directory is assumed to be inode 256, the checksum tree root is taken from `roots` when present.
    pub fn open_with_roots(vol: Volume<D>, roots: BTreeMap<u64, TreeRef>) -> Fs<D> {
        let csum_root = roots.get(&CSUM_TREE_OBJECTID).copied();
        Fs { vol, roots, verify_data: csum_root.is_some(), case_insensitive: false, csum_root, extent_cache: None, default_subvol: FS_TREE_OBJECTID }
    }

    /// Opens a specific subvolume as `/`.
    pub fn open_subvol(vol: Volume<D>, subvol: u64) -> Result<Fs<D>> {
        let mut fs = Fs::open(vol)?;
        fs.tree(subvol)?;
        fs.default_subvol = subvol;
        Ok(fs)
    }

    fn find_default_subvol(vol: &mut Volume<D>) -> Result<u64> {
        let root = vol.sb.root;
        let key = Key::new(ROOT_TREE_DIR_OBJECTID, DIR_ITEM_KEY, name_hash(b"default"));
        match vol.tree_lookup(root, key)? {
            Some(d) => {
                let items = parse_dir_items(&d)?;
                Ok(items.first().map_or(FS_TREE_OBJECTID, |i| i.0.objectid))
            }
            None => Ok(FS_TREE_OBJECTID),
        }
    }

    /// Overrides the root of a tree (used with `rescue::find_tree_roots` when the root tree is damaged).
    pub fn set_tree_root(&mut self, subvol: u64, root: TreeRef) {
        self.roots.insert(subvol, root);
    }

    /// Root block of a subvolume tree (cached).
    pub fn tree(&mut self, subvol: u64) -> Result<TreeRef> {
        if let Some(t) = self.roots.get(&subvol) {
            return Ok(*t);
        }
        let t = self.vol.tree_root(subvol)?;
        self.roots.insert(subvol, t);
        Ok(t)
    }

    /// Root node of the mounted subvolume.
    pub fn root_node(&mut self) -> Result<Node> {
        let sv = self.default_subvol;
        match self.vol.root_item(sv) {
            Ok(ri) => Ok(Node { subvol: sv, ino: ri.root_dirid }),
            Err(e) if self.roots.contains_key(&sv) => {
                let _ = e;
                Ok(Node { subvol: sv, ino: FIRST_FREE_OBJECTID })
            }
            Err(e) => Err(e),
        }
    }

    // ------------------------------------------------------------------------- inodes

    /// Reads an inode.
    pub fn stat(&mut self, n: Node) -> Result<Inode> {
        let t = self.tree(n.subvol)?;
        let d = self.vol.tree_lookup(t, Key::new(n.ino, INODE_ITEM_KEY, 0))?.ok_or(Error::NotFound)?;
        Inode::parse(n.ino, &d)
    }

    /// Looks a name up in a directory.
    pub fn lookup(&mut self, dir: Node, name: &[u8]) -> Result<DirEnt> {
        let t = self.tree(dir.subvol)?;
        let key = Key::new(dir.ino, DIR_ITEM_KEY, name_hash(name));
        if let Some(d) = self.vol.tree_lookup(t, key)? {
            for (loc, ty, nm, _) in parse_dir_items(&d)? {
                if nm == name {
                    return Ok(self.dirent(dir, loc, ty, nm, 0));
                }
            }
        }
        if self.case_insensitive {
            let want = fold(name);
            for e in self.readdir(dir)? {
                if fold(&e.name) == want {
                    return Ok(e);
                }
            }
        }
        Err(Error::NotFound)
    }

    fn dirent(&self, dir: Node, loc: Key, ty: u8, name: Vec<u8>, index: u64) -> DirEnt {
        if loc.ty == ROOT_ITEM_KEY {
            DirEnt { name, node: Node { subvol: loc.objectid, ino: FIRST_FREE_OBJECTID }, kind: Kind::Subvolume, index }
        } else {
            DirEnt { name, node: Node { subvol: dir.subvol, ino: loc.objectid }, kind: Kind::from_ft(ty), index }
        }
    }

    /// Lists a directory in `DIR_INDEX` (creation) order.
    pub fn readdir(&mut self, dir: Node) -> Result<Vec<DirEnt>> {
        let t = self.tree(dir.subvol)?;
        let items = self.vol.tree_items(t, Key::new(dir.ino, DIR_INDEX_KEY, 0), Key::new(dir.ino, DIR_INDEX_KEY, u64::MAX))?;
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            for (loc, ty, nm, _) in parse_dir_items(&it.data)? {
                out.push(self.dirent(dir, loc, ty, nm, it.key.offset));
            }
        }
        Ok(out)
    }

    /// Resolves an absolute path (crossing subvolume boundaries); `.`/`..`/empty components handled.
    pub fn resolve(&mut self, path: &str) -> Result<Node> {
        let mut stack: Vec<Node> = Vec::new();
        let mut cur = self.root_node()?;
        for comp in path.split('/') {
            match comp {
                "" | "." => continue,
                ".." => {
                    cur = stack.pop().unwrap_or(cur);
                    continue;
                }
                _ => {}
            }
            let st = self.stat(cur)?;
            if st.kind() != Kind::Dir {
                return Err(Error::NotDir);
            }
            let e = self.lookup(cur, comp.as_bytes())?;
            stack.push(cur);
            cur = if e.kind == Kind::Subvolume {
                let ri = self.vol.root_item(e.node.subvol)?;
                Node { subvol: e.node.subvol, ino: ri.root_dirid }
            } else {
                e.node
            };
        }
        Ok(cur)
    }

    /// Reads a symlink target.
    pub fn readlink(&mut self, n: Node) -> Result<Vec<u8>> {
        let st = self.stat(n)?;
        if st.kind() != Kind::Symlink {
            return Err(Error::Invalid("not a symlink"));
        }
        let mut buf = vec![0u8; st.size as usize];
        let got = self.read(n, 0, &mut buf)?;
        buf.truncate(got);
        Ok(buf)
    }

    // ------------------------------------------------------------------------- xattrs

    /// All extended attributes of an inode as (name, value).
    pub fn xattrs(&mut self, n: Node) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let t = self.tree(n.subvol)?;
        let items = self.vol.tree_items(t, Key::new(n.ino, XATTR_ITEM_KEY, 0), Key::new(n.ino, XATTR_ITEM_KEY, u64::MAX))?;
        let mut out = Vec::new();
        for it in items {
            for (_, _, nm, val) in parse_dir_items(&it.data)? {
                out.push((nm, val));
            }
        }
        Ok(out)
    }

    /// One extended attribute.
    pub fn get_xattr(&mut self, n: Node, name: &[u8]) -> Result<Option<Vec<u8>>> {
        let t = self.tree(n.subvol)?;
        match self.vol.tree_lookup(t, Key::new(n.ino, XATTR_ITEM_KEY, name_hash(name)))? {
            Some(d) => Ok(parse_dir_items(&d)?.into_iter().find(|i| i.2 == name).map(|i| i.3)),
            None => Ok(None),
        }
    }

    // ------------------------------------------------------------------------- file data

    /// File extents of an inode within `[from, ..)` (starting with the extent that covers `from`).
    pub fn extents(&mut self, n: Node, from: u64, to: u64) -> Result<Vec<FileExtent>> {
        let t = self.tree(n.subvol)?;
        let mut start = from;
        if let Some(prev) = self.vol.tree_floor(t, Key::new(n.ino, EXTENT_DATA_KEY, from))? {
            if prev.key.objectid == n.ino && prev.key.ty == EXTENT_DATA_KEY {
                start = prev.key.offset;
            }
        }
        let items = self.vol.tree_items(t, Key::new(n.ino, EXTENT_DATA_KEY, start), Key::new(n.ino, EXTENT_DATA_KEY, to))?;
        items.into_iter().map(|it| parse_file_extent(it.key.offset, &it.data)).collect()
    }

    fn csum_lookup(&mut self, logical: u64, len: u64) -> Result<BTreeMap<u64, Vec<u8>>> {
        // sector logical address -> stored checksum
        let mut map = BTreeMap::new();
        let root = match self.csum_root {
            Some(r) => r,
            None => return Ok(map),
        };
        let cs = self.vol.csum_size() as u64;
        let ss = self.vol.sectorsize() as u64;
        // an item starting before `logical` may cover it
        let lo = match self.vol.tree_floor(root, Key::new(EXTENT_CSUM_OBJECTID, EXTENT_CSUM_KEY, logical))? {
            Some(it) if it.key.objectid == EXTENT_CSUM_OBJECTID && it.key.ty == EXTENT_CSUM_KEY => it.key.offset,
            _ => logical,
        };
        let items = self.vol.tree_items(root, Key::new(EXTENT_CSUM_OBJECTID, EXTENT_CSUM_KEY, lo), Key::new(EXTENT_CSUM_OBJECTID, EXTENT_CSUM_KEY, logical + len))?;
        for it in items {
            let n = it.data.len() as u64 / cs;
            for i in 0..n {
                let sector = it.key.offset + i * ss;
                if sector + ss > logical && sector < logical + len {
                    map.insert(sector, it.data[(i * cs) as usize..((i + 1) * cs) as usize].to_vec());
                }
            }
        }
        Ok(map)
    }

    /// Reads on-disk bytes `[logical, logical+len)` with checksum verification and mirror fallback.
    pub fn read_disk(&mut self, logical: u64, buf: &mut [u8], nodatasum: bool) -> Result<()> {
        let ss = self.vol.sectorsize() as u64;
        if !self.verify_data || nodatasum {
            return self.vol.read_logical(logical, buf);
        }
        let csums = self.csum_lookup(logical, buf.len() as u64)?;
        let alg = self.vol.alg();
        let mut verify = |lg: u64, seg: &[u8]| -> bool {
            for (i, sec) in seg.chunks(ss as usize).enumerate() {
                let addr = lg + i as u64 * ss;
                if let Some(c) = csums.get(&addr) {
                    if sec.len() as u64 == ss && !loos_btrfs::verify(alg, sec, c) {
                        return false;
                    }
                }
            }
            true
        };
        self.vol.read_logical_verified(logical, buf, &mut verify).map(|_| ())
    }

    fn read_extent_bytes(&mut self, e: &FileExtent, ino_flags: u64, rel: u64, out: &mut [u8]) -> Result<()> {
        // `rel`: offset of the first wanted byte inside the extent's file range
        let nodatasum = ino_flags & INODE_NODATASUM != 0;
        match &e.kind {
            ExtentKind::Inline { compression, ram_bytes, data } => {
                let full: Vec<u8> = if *compression == COMPRESS_NONE { data.clone() } else { compress::decompress(*compression, data, *ram_bytes as usize, self.vol.sectorsize() as usize)? };
                for (i, b) in out.iter_mut().enumerate() {
                    *b = full.get(rel as usize + i).copied().unwrap_or(0);
                }
                Ok(())
            }
            ExtentKind::Regular { prealloc, compression, disk_bytenr, disk_num_bytes, offset, ram_bytes, .. } => {
                if *prealloc || *disk_bytenr == 0 {
                    for b in out.iter_mut() {
                        *b = 0;
                    }
                    return Ok(());
                }
                if *compression == COMPRESS_NONE {
                    let ss = self.vol.sectorsize() as u64;
                    let start = disk_bytenr + offset + rel;
                    // read whole sectors so each can be verified, then copy the wanted part
                    let a0 = start / ss * ss;
                    let a1 = (start + out.len() as u64 + ss - 1) / ss * ss;
                    let ext_end = disk_bytenr + disk_num_bytes;
                    let a1 = core::cmp::min(a1, (ext_end + ss - 1) / ss * ss);
                    let mut tmp = vec![0u8; (a1 - a0) as usize];
                    self.read_disk(a0, &mut tmp, nodatasum)?;
                    let s = (start - a0) as usize;
                    out.copy_from_slice(&tmp[s..s + out.len()]);
                    Ok(())
                } else {
                    let key = (*disk_bytenr, *compression);
                    let hit = matches!(&self.extent_cache, Some((b, c, _)) if (*b, *c) == key);
                    if !hit {
                        let ss = self.vol.sectorsize() as u64;
                        let padded = (disk_num_bytes + ss - 1) / ss * ss;
                        let mut raw = vec![0u8; padded as usize];
                        self.read_disk(*disk_bytenr, &mut raw, nodatasum)?;
                        raw.truncate(*disk_num_bytes as usize);
                        let plain = compress::decompress(*compression, &raw, *ram_bytes as usize, ss as usize)?;
                        self.extent_cache = Some((key.0, key.1, plain));
                    }
                    let plain = &self.extent_cache.as_ref().unwrap().2;
                    let s = (*offset + rel) as usize;
                    if s + out.len() > plain.len() {
                        return Err(Error::Corrupt("extent range beyond decompressed data"));
                    }
                    out.copy_from_slice(&plain[s..s + out.len()]);
                    Ok(())
                }
            }
        }
    }

    /// Reads file data; holes and preallocated ranges read as zeros. Returns bytes read.
    pub fn read(&mut self, n: Node, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let st = self.stat(n)?;
        if st.kind() == Kind::Dir {
            return Err(Error::IsDir);
        }
        if offset >= st.size {
            return Ok(0);
        }
        let want = core::cmp::min(buf.len() as u64, st.size - offset) as usize;
        for b in &mut buf[..want] {
            *b = 0;
        }
        let end = offset + want as u64;
        for e in self.extents(n, offset, end)? {
            let (e_start, e_len) = (e.file_offset, e.len());
            let e_end = e_start + e_len;
            if e_end <= offset || e_start >= end {
                continue;
            }
            let lo = core::cmp::max(offset, e_start);
            let hi = core::cmp::min(end, e_end);
            let dst = &mut buf[(lo - offset) as usize..(hi - offset) as usize];
            self.read_extent_bytes(&e, st.flags, lo - e_start, dst)?;
        }
        Ok(want)
    }

    /// Reads a whole file.
    pub fn read_all(&mut self, n: Node) -> Result<Vec<u8>> {
        let st = self.stat(n)?;
        let mut v = vec![0u8; st.size as usize];
        let got = self.read(n, 0, &mut v)?;
        v.truncate(got);
        Ok(v)
    }

    // ------------------------------------------------------------------------- subvolumes

    /// Lists every subvolume/snapshot (excluding the top-level tree itself).
    pub fn subvolumes(&mut self) -> Result<Vec<SubvolInfo>> {
        let root = self.vol.sb.root;
        let items = self.vol.tree_items(root, Key::new(0, ROOT_REF_KEY, 0), Key::new(u64::MAX, ROOT_REF_KEY, u64::MAX))?;
        let mut raw: Vec<(u64, u64, u64, String)> = Vec::new(); // parent, child, dirid, name
        for it in items {
            if it.key.ty != ROOT_REF_KEY || it.data.len() < 18 {
                continue;
            }
            let name_len = le16(&it.data, 16) as usize;
            if 18 + name_len > it.data.len() {
                return Err(Error::Corrupt("root ref name"));
            }
            raw.push((it.key.objectid, it.key.offset, le64(&it.data, 0), String::from_utf8_lossy(&it.data[18..18 + name_len]).into_owned()));
        }
        let mut out = Vec::new();
        for &(parent, child, dirid, ref name) in &raw {
            let ri = self.vol.root_item(child)?;
            // path: walk parents, each segment = parent's dir path inside its parent is not tracked here;
            // btrfs `ROOT_REF.name` is the leaf name, intermediate directories come from the parent tree.
            let dir_path = self.dir_path(parent, dirid)?;
            let mut path = self.subvol_path(parent, &raw)?;
            if !dir_path.is_empty() {
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(&dir_path);
            }
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(name);
            out.push(SubvolInfo { id: child, parent, name: name.clone(), dirid, root: ri, path });
        }
        Ok(out)
    }

    fn subvol_path(&mut self, subvol: u64, raw: &[(u64, u64, u64, String)]) -> Result<String> {
        // path of `subvol` from the top-level tree
        let mut parts: Vec<String> = Vec::new();
        let mut cur = subvol;
        let mut guard = 0;
        while let Some(&(parent, _, dirid, ref name)) = raw.iter().find(|r| r.1 == cur) {
            guard += 1;
            if guard > 256 {
                return Err(Error::Corrupt("subvolume loop"));
            }
            let mut seg = String::new();
            let dp = self.dir_path(parent, dirid)?;
            if !dp.is_empty() {
                seg.push_str(&dp);
                seg.push('/');
            }
            seg.push_str(name);
            parts.push(seg);
            cur = parent;
        }
        parts.reverse();
        Ok(parts.join("/"))
    }

    /// Path of directory inode `dirid` inside tree `subvol` relative to that tree's root (via `INODE_REF`).
    pub fn dir_path(&mut self, subvol: u64, dirid: u64) -> Result<String> {
        let t = match self.tree(subvol) {
            Ok(t) => t,
            Err(_) => return Ok(String::new()),
        };
        let top = match self.vol.root_item(subvol) {
            Ok(r) => r.root_dirid,
            Err(_) => FIRST_FREE_OBJECTID,
        };
        let mut parts: Vec<String> = Vec::new();
        let mut cur = dirid;
        let mut guard = 0;
        while cur != top {
            guard += 1;
            if guard > 1024 {
                return Err(Error::Corrupt("directory loop"));
            }
            let it = self.vol.tree_first_ge(t, Key::new(cur, INODE_REF_KEY, 0), Key::new(cur, INODE_REF_KEY, u64::MAX))?.ok_or(Error::NotFound)?;
            if it.data.len() < 10 {
                return Err(Error::Corrupt("inode ref"));
            }
            let nl = le16(&it.data, 8) as usize;
            if 10 + nl > it.data.len() {
                return Err(Error::Corrupt("inode ref name"));
            }
            parts.push(String::from_utf8_lossy(&it.data[10..10 + nl]).into_owned());
            cur = it.key.offset;
        }
        parts.reverse();
        Ok(parts.join("/"))
    }
}

fn fold(b: &[u8]) -> Vec<u8> {
    match core::str::from_utf8(b) {
        Ok(s) => s.chars().flat_map(|c| c.to_uppercase()).collect::<String>().into_bytes(),
        Err(_) => b.to_ascii_uppercase(),
    }
}

//! A read-only [`Backend`] over [`Fs`]: answers every query ioctl from the on-disk trees and
//! refuses the mutating ones with `EROFS`. Mutating backends (the write engine) implement the
//! same trait and reuse [`crate::ioctl::dispatch`] unchanged.
use crate::consts::*;
use crate::dev::Dev;
use crate::fs::{ExtentKind, Fs, Node};
use crate::ioctl::*;
use crate::volume::{le64, Key};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Query-only ioctl backend for the file `node` the ioctl was issued on.
pub struct ReadOnlyBackend<'a, D: Dev> {
    /// Filesystem.
    pub fs: &'a mut Fs<D>,
    /// The inode the caller holds open (per-inode ioctls act on it).
    pub node: Node,
}

const FIEMAP_EXTENT_LAST: u32 = 0x1;
const FIEMAP_EXTENT_ENCODED: u32 = 0x8;
const FIEMAP_EXTENT_NOT_ALIGNED: u32 = 0x100;
const FIEMAP_EXTENT_DATA_INLINE: u32 = 0x200;
const FIEMAP_EXTENT_UNWRITTEN: u32 = 0x800;

impl<'a, D: Dev> ReadOnlyBackend<'a, D> {
    /// Creates the backend.
    pub fn new(fs: &'a mut Fs<D>, node: Node) -> Self {
        ReadOnlyBackend { fs, node }
    }

    fn tree_id(&self, id: u64) -> u64 {
        if id == 0 {
            self.node.subvol
        } else {
            id
        }
    }

    fn space_info(&mut self) -> Result<Vec<(u64, u64, u64)>, Errno> {
        let vol = &mut self.fs.vol;
        let root = if vol.sb.compat_ro & COMPAT_RO_BLOCK_GROUP_TREE != 0 {
            vol.tree_root(BLOCK_GROUP_TREE_OBJECTID)?
        } else {
            vol.tree_root(EXTENT_TREE_OBJECTID)?
        };
        let mut by_flags: BTreeMap<u64, (u64, u64)> = BTreeMap::new();
        vol.walk_tree(root, Key::min(0), Key::max(u64::MAX), &mut |k, d| {
            if k.ty == BLOCK_GROUP_ITEM_KEY && d.len() >= 24 {
                let e = by_flags.entry(le64(d, 16)).or_insert((0, 0));
                e.0 += k.offset;
                e.1 += le64(d, 0);
            }
            true
        })?;
        Ok(by_flags.into_iter().map(|(f, (t, u))| (f, t, u)).collect())
    }

    fn tree_search(&mut self, k: &SearchKey, buf_size: usize) -> Result<Vec<SearchItem>, Errno> {
        let tree = self.tree_id(k.tree_id);
        let root = self.fs.vol.tree_root(tree)?;
        let lo = Key::new(k.min_objectid, k.min_type.min(255) as u8, k.min_offset);
        let hi = Key::new(k.max_objectid, 255, u64::MAX);
        let mut out = Vec::new();
        let mut used = 0usize;
        let limit = k.nr_items as usize;
        self.fs.vol.walk_tree_gen(root, lo, hi, &mut |generation, key, data| {
            if out.len() >= limit {
                return false;
            }
            // the kernel filters on each component independently (a box, not a lexicographic range)
            if key.objectid < k.min_objectid
                || key.objectid > k.max_objectid
                || (key.ty as u32) < k.min_type
                || (key.ty as u32) > k.max_type
                || key.offset < k.min_offset && key.objectid == k.min_objectid && (key.ty as u32) == k.min_type
                || key.offset > k.max_offset
                || generation < k.min_transid
                || generation > k.max_transid
            {
                return true;
            }
            let need = 32 + data.len();
            if used + need > buf_size {
                if out.is_empty() {
                    // an item bigger than the buffer: kernel returns it truncated? no: EOVERFLOW; keep and let encode fail
                    out.push(SearchItem { transid: generation, objectid: key.objectid, offset: key.offset, ty: key.ty as u32, data: data.to_vec() });
                }
                return false;
            }
            used += need;
            out.push(SearchItem { transid: generation, objectid: key.objectid, offset: key.offset, ty: key.ty as u32, data: data.to_vec() });
            true
        })?;
        Ok(out)
    }

    fn fiemap(&mut self, rq: &FiemapReq) -> Result<Reply, Errno> {
        let st = self.fs.stat(self.node)?;
        let end = rq.start.saturating_add(rq.length).min(st.size);
        let exts = if rq.start >= st.size { Vec::new() } else { self.fs.extents(self.node, rq.start, end.saturating_sub(1))? };
        let mut out: Vec<FiemapExtent> = Vec::new();
        for e in exts {
            let (len, phys, mut flags) = match &e.kind {
                ExtentKind::Inline { ram_bytes, .. } => (*ram_bytes, 0, FIEMAP_EXTENT_DATA_INLINE | FIEMAP_EXTENT_NOT_ALIGNED),
                ExtentKind::Regular { prealloc, compression, disk_bytenr, offset, num_bytes, .. } => {
                    if *disk_bytenr == 0 && !*prealloc {
                        continue; // hole
                    }
                    let mut f = 0;
                    if *prealloc {
                        f |= FIEMAP_EXTENT_UNWRITTEN;
                    }
                    if *compression != COMPRESS_NONE {
                        f |= FIEMAP_EXTENT_ENCODED;
                    }
                    (*num_bytes, disk_bytenr + offset, f)
                }
            };
            if e.file_offset + len <= rq.start || e.file_offset >= end {
                continue;
            }
            // merge physically adjacent plain extents like the kernel's fiemap does
            if let Some(last) = out.last_mut() {
                if last.flags == flags && last.logical + last.length == e.file_offset && last.physical + last.length == phys && flags == 0 {
                    last.length += len;
                    continue;
                }
            }
            if flags & FIEMAP_EXTENT_DATA_INLINE != 0 {
                flags |= FIEMAP_EXTENT_LAST;
            }
            out.push(FiemapExtent { logical: e.file_offset, physical: phys, length: len, flags });
        }
        if let Some(l) = out.last_mut() {
            if l.logical + l.length >= st.size {
                l.flags |= FIEMAP_EXTENT_LAST;
            }
        }
        let mapped = out.len() as u32;
        out.truncate(rq.extent_count as usize);
        Ok(Reply::Fiemap { mapped, extents: out })
    }
}

impl<'a, D: Dev> Backend for ReadOnlyBackend<'a, D> {
    fn ioctl(&mut self, req: Request) -> Result<Reply, Errno> {
        match req {
            Request::FsInfo { .. } => {
                let sb = self.fs.vol.sb.clone();
                let max_id = self.fs.vol.dev_items.keys().next_back().copied().unwrap_or(sb.dev_item.devid);
                Ok(Reply::FsInfo(FsInfo {
                    max_id,
                    num_devices: sb.num_devices,
                    fsid: sb.fsid,
                    nodesize: sb.nodesize,
                    sectorsize: sb.sectorsize,
                    clone_alignment: sb.sectorsize,
                    csum_type: sb.csum_type,
                    csum_size: self.fs.vol.csum_size() as u16,
                    generation: sb.generation,
                    metadata_uuid: sb.metadata_uuid,
                }))
            }
            Request::DevInfo { devid, .. } => {
                let d = *self.fs.vol.dev_items.get(&devid).ok_or(Errno::ENODEV)?;
                Ok(Reply::DevInfo(DevInfo { devid, uuid: d.uuid, bytes_used: d.bytes_used, total_bytes: d.total_bytes, fsid: d.fsid, path: alloc::format!("devid{}", devid).into_bytes() }))
            }
            Request::SpaceInfo { .. } => Ok(Reply::SpaceInfo(self.space_info()?)),
            Request::TreeSearch { key, buf_size } => Ok(Reply::TreeSearch(self.tree_search(&key, buf_size)?)),
            Request::InoLookup { treeid, objectid } => {
                let tree = self.tree_id(treeid);
                let objectid = if objectid == 0 { self.node.ino } else { objectid };
                if objectid == FIRST_FREE_OBJECTID {
                    return Ok(Reply::Path { treeid: tree, path: Vec::new() });
                }
                let mut p = self.fs.dir_path(tree, objectid)?.into_bytes();
                if !p.is_empty() {
                    p.push(b'/');
                }
                Ok(Reply::Path { treeid: tree, path: p })
            }
            Request::GetSubvolInfo => {
                let id = self.node.subvol;
                let ri = self.fs.vol.root_item(id)?;
                let (name, parent, dirid) = match self.fs.subvolumes()?.into_iter().find(|s| s.id == id) {
                    Some(s) => (s.name.into_bytes(), s.parent, s.dirid),
                    None => (Vec::new(), 0, 0),
                };
                Ok(Reply::SubvolInfo(SubvolInfoReply {
                    treeid: id,
                    name,
                    parent_id: parent,
                    dirid,
                    generation: ri.generation,
                    flags: ri.flags,
                    uuid: ri.uuid,
                    parent_uuid: ri.parent_uuid,
                    received_uuid: ri.received_uuid,
                    transids: [ri.ctransid, ri.otransid, ri.stransid, ri.rtransid],
                }))
            }
            Request::SubvolGetFlags => {
                let ri = self.fs.vol.root_item(self.node.subvol)?;
                Ok(Reply::Value(if ri.flags & ROOT_SUBVOL_RDONLY != 0 { SUBVOL_RDONLY } else { 0 }))
            }
            Request::GetSubvolRootref { min_treeid } => {
                let root = self.fs.vol.sb.root;
                let subvol = self.node.subvol;
                let items = self.fs.vol.tree_items(root, Key::new(subvol, ROOT_REF_KEY, min_treeid), Key::new(subvol, ROOT_REF_KEY, u64::MAX))?;
                Ok(Reply::RootRefs(items.into_iter().filter(|i| i.data.len() >= 8).map(|i| (i.key.offset, le64(&i.data, 0))).take(255).collect()))
            }
            Request::GetLabel => Ok(Reply::Label(self.fs.vol.sb.label.clone().into_bytes())),
            Request::GetFeatures => Ok(Reply::Features([0, self.fs.vol.sb.compat_ro, self.fs.vol.sb.incompat])),
            Request::GetSupportedFeatures => Ok(Reply::Features([0, COMPAT_RO_FREE_SPACE_TREE | COMPAT_RO_FREE_SPACE_TREE_VALID | COMPAT_RO_BLOCK_GROUP_TREE, INCOMPAT_KNOWN])),
            Request::GetDevStats { devid, flags } => {
                if flags & !1 != 0 {
                    return Err(Errno::EINVAL);
                }
                let root = self.fs.vol.sb.root;
                let it = self.fs.vol.tree_lookup(root, Key::new(DEV_STATS_OBJECTID, DEV_STATS_KEY, devid))?;
                let mut values = [0u64; 5];
                if let Some(d) = it {
                    for (i, v) in values.iter_mut().enumerate() {
                        if d.len() >= (i + 1) * 8 {
                            *v = le64(&d, i * 8);
                        }
                    }
                }
                Ok(Reply::DevStats { values, nr_items: 5 })
            }
            Request::GetFlags => {
                let st = self.fs.stat(self.node)?;
                Ok(Reply::Value(inode_flags_to_fs_flags(st.flags, st.kind() == crate::fs::Kind::Dir) as u64))
            }
            Request::Fiemap(rq) => self.fiemap(&rq),
            Request::Sync | Request::StartSync | Request::WaitSync { .. } | Request::SubvolSyncWait { .. } | Request::ScrubCancel | Request::BalanceProgress | Request::QuotaRescanStatus | Request::QuotaRescanWait => {
                if matches!(req, Request::StartSync) {
                    Ok(Reply::Value(self.fs.vol.sb.generation))
                } else if matches!(req, Request::QuotaRescanStatus) {
                    Ok(Reply::Rescan { flags: 0, progress: 0 })
                } else {
                    Ok(Reply::Done)
                }
            }
            Request::DevicesReady { .. } => Ok(Reply::Done),
            Request::Other { .. } => Err(Errno::ENOTTY),
            // everything else mutates or needs the write engine
            _ => Err(Errno::EROFS),
        }
    }
}

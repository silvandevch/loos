//! Defragmentation planning (manual `BTRFS_IOC_DEFRAG_RANGE` and `autodefrag`), mirroring the
//! kernel's target collection: which file ranges are worth rewriting, honouring the extent-size
//! threshold, adjacency, holes/prealloc/inline extents, shared extents and (re)compression requests.
//! The write engine executes the plan (rewrite the ranges through the page cache).
use crate::consts::*;
use crate::dev::Dev;
use crate::error::Result;
use crate::fs::{ExtentKind, FileExtent, Fs, Node};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

/// Default `extent_thresh` (the kernel's 32 MiB).
pub const DEFAULT_EXTENT_THRESH: u64 = 32 << 20;
/// Largest uncompressed extent the allocator writes.
pub const MAX_EXTENT: u64 = 128 << 20;
/// Largest compressed extent (uncompressed size).
pub const MAX_COMPRESSED_EXTENT: u64 = 128 << 10;

/// Options (`btrfs_ioctl_defrag_range_args`).
#[derive(Clone, Copy, Debug)]
pub struct DefragOptions {
    /// First byte of the range.
    pub start: u64,
    /// Length (`u64::MAX` = to EOF).
    pub len: u64,
    /// Extents at least this large are left alone (0 = default).
    pub extent_thresh: u64,
    /// Recompress with this type (1 zlib, 2 lzo, 3 zstd).
    pub compress: Option<u8>,
    /// Rewrite compressed extents uncompressed.
    pub nocompress: bool,
}

impl Default for DefragOptions {
    fn default() -> Self {
        DefragOptions { start: 0, len: u64::MAX, extent_thresh: 0, compress: None, nocompress: false }
    }
}

/// What to rewrite.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefragPlan {
    /// File ranges `[start, end)` to rewrite, sorted and merged.
    pub ranges: Vec<(u64, u64)>,
    /// Extents in the requested range before defragmenting.
    pub extents_before: u64,
    /// Estimated extents covering the same bytes afterwards.
    pub extents_after: u64,
    /// Bytes that will be rewritten.
    pub bytes: u64,
}

fn compression_of(e: &FileExtent) -> Option<u8> {
    match &e.kind {
        ExtentKind::Regular { compression, .. } => Some(*compression),
        ExtentKind::Inline { compression, .. } => Some(*compression),
    }
}

/// Extent is real data (not hole, not prealloc, not inline).
fn plain(e: &FileExtent) -> bool {
    matches!(&e.kind, ExtentKind::Regular { prealloc: false, disk_bytenr, .. } if *disk_bytenr != 0)
}

/// Plans a defrag of one file's extents. `shared` holds the `disk_bytenr`s of extents referenced more than
/// once (reflinks/snapshots): they are skipped unless a (re)compression rewrite was requested.
pub fn plan_extents(extents: &[FileExtent], shared: &BTreeSet<u64>, opts: &DefragOptions) -> DefragPlan {
    let thresh = if opts.extent_thresh == 0 { DEFAULT_EXTENT_THRESH } else { opts.extent_thresh };
    let end_req = opts.start.saturating_add(opts.len);
    let mut plan = DefragPlan::default();
    let mut targets: Vec<(u64, u64)> = Vec::new();
    let in_range: Vec<&FileExtent> = extents.iter().filter(|e| e.file_offset + e.len() > opts.start && e.file_offset < end_req).collect();
    plan.extents_before = in_range.len() as u64;
    for (i, e) in in_range.iter().enumerate() {
        if !plain(e) {
            continue;
        }
        let (disk, len) = match &e.kind {
            ExtentKind::Regular { disk_bytenr, num_bytes, .. } => (*disk_bytenr, *num_bytes),
            _ => continue,
        };
        let comp = compression_of(e).unwrap_or(0);
        let wrong_compression = match (opts.compress, opts.nocompress) {
            (Some(t), _) => comp != t,
            (None, true) => comp != COMPRESS_NONE,
            _ => false,
        };
        let is_shared = shared.contains(&disk);
        let mut take = false;
        if opts.compress.is_some() || opts.nocompress {
            // An explicit compressor request targets only extents whose
            // current encoding differs; already-correct extents are not
            // pulled into an adjacent rewrite range.
            take = wrong_compression;
        } else if wrong_compression {
            take = true; // nocompress rewrites compressed extents regardless of size or sharing
        } else if !is_shared && len < thresh {
            // worth merging only if the neighbour is also rewritable data that is not already contiguous
            take = match in_range.get(i + 1) {
                None => false,
                Some(next) => {
                    if !plain(next) {
                        false
                    } else {
                        let (nd, nlen) = match &next.kind {
                            ExtentKind::Regular { disk_bytenr, num_bytes, .. } => (*disk_bytenr, *num_bytes),
                            _ => (0, 0),
                        };
                        let adjacent_on_disk = match &e.kind {
                            ExtentKind::Regular { disk_bytenr, disk_num_bytes, offset, num_bytes, .. } => *disk_bytenr + *disk_num_bytes == nd && *offset + *num_bytes == *disk_num_bytes,
                            _ => false,
                        };
                        e.file_offset + len == next.file_offset && !adjacent_on_disk && nlen < thresh && !shared.contains(&nd)
                    }
                }
            };
            // the last extent of a run joins its predecessor's range when that was a target
            if !take && !(opts.nocompress && comp == COMPRESS_NONE) {
                if let Some((_, pe)) = targets.last() {
                    if *pe == e.file_offset && i > 0 {
                        take = prev_wants_merge(in_range[i - 1], e, thresh, shared);
                    }
                }
            }
        }
        if take {
            let s = e.file_offset.max(opts.start);
            let t = (e.file_offset + len).min(end_req);
            match targets.last_mut() {
                Some(last) if last.1 == s && opts.compress.is_none() => last.1 = t,
                _ => targets.push((s, t)),
            }
        }
    }
    plan.bytes = targets.iter().map(|r| r.1 - r.0).sum();
    // estimate: rewritten ranges become ceil(len / max extent) extents; untouched extents stay
    let max = match (opts.compress, opts.nocompress) {
        (Some(_), _) => MAX_COMPRESSED_EXTENT,
        _ => MAX_EXTENT,
    };
    let rewritten_extents: u64 = in_range.iter().filter(|e| targets.iter().any(|r| e.file_offset >= r.0 && e.file_offset < r.1)).count() as u64;
    let new_extents: u64 = targets.iter().map(|r| (r.1 - r.0 + max - 1) / max).sum();
    plan.extents_after = plan.extents_before - rewritten_extents + new_extents;
    plan.ranges = targets;
    plan
}

fn prev_wants_merge(prev: &FileExtent, cur: &FileExtent, thresh: u64, shared: &BTreeSet<u64>) -> bool {
    if !plain(prev) || !plain(cur) {
        return false;
    }
    let (pd, plen, pdn, poff) = match &prev.kind {
        ExtentKind::Regular { disk_bytenr, num_bytes, disk_num_bytes, offset, .. } => (*disk_bytenr, *num_bytes, *disk_num_bytes, *offset),
        _ => return false,
    };
    let cd = match &cur.kind {
        ExtentKind::Regular { disk_bytenr, .. } => *disk_bytenr,
        _ => return false,
    };
    let adjacent = pd + pdn == cd && poff + plen == pdn;
    prev.file_offset + plen == cur.file_offset && !adjacent && plen < thresh && !shared.contains(&pd) && !shared.contains(&cd)
}

/// Plans a defrag for a file on a live filesystem (looks up which extents are shared).
pub fn plan_file<D: Dev>(fs: &mut Fs<D>, node: Node, opts: &DefragOptions) -> Result<DefragPlan> {
    let st = fs.stat(node)?;
    if st.size == 0 {
        return Ok(DefragPlan::default());
    }
    let list = fs.extents(node, opts.start.min(st.size), opts.start.saturating_add(opts.len).min(st.size).saturating_sub(1).max(opts.start.min(st.size)))?;
    let inv = crate::extents::inventory(&mut fs.vol)?;
    let mut shared = BTreeSet::new();
    for e in &list {
        if let ExtentKind::Regular { disk_bytenr, .. } = &e.kind {
            let idx = inv.extents.partition_point(|x| x.start < *disk_bytenr);
            if let Some(x) = inv.extents.get(idx) {
                if x.start == *disk_bytenr && x.refs > 1 {
                    shared.insert(*disk_bytenr);
                }
            }
        }
    }
    Ok(plan_extents(&list, &shared, opts))
}

// ---------------------------------------------------------------------------------------------
// autodefrag

/// Writes smaller than this to a fragmented file queue the inode for background defrag.
pub const AUTODEFRAG_WRITE_THRESHOLD: u64 = 64 << 10;

/// The `autodefrag` queue: inodes touched by small random writes, defragmented in the background.
#[derive(Debug, Default)]
pub struct AutoDefrag {
    queue: Vec<(Node, u64)>, // node, next offset to resume from
    queued: BTreeSet<(u64, u64)>,
}

impl AutoDefrag {
    /// New empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reports a completed write. Returns true when the inode was newly queued. `extent_len` is the
    /// size of the extent the write landed in (a small write into an already large extent changes nothing).
    pub fn on_write(&mut self, node: Node, offset: u64, write_len: u64, extent_len: u64, thresh: u64) -> bool {
        let thresh = if thresh == 0 { DEFAULT_EXTENT_THRESH } else { thresh };
        if write_len >= AUTODEFRAG_WRITE_THRESHOLD || extent_len >= thresh {
            return false;
        }
        if !self.queued.insert((node.subvol, node.ino)) {
            return false;
        }
        self.queue.push((node, offset / (64 << 10) * (64 << 10)));
        true
    }

    /// Pops the next inode to defragment, with the offset to start from.
    pub fn next(&mut self) -> Option<(Node, u64)> {
        if self.queue.is_empty() {
            return None;
        }
        let (n, off) = self.queue.remove(0);
        self.queued.remove(&(n.subvol, n.ino));
        Some((n, off))
    }

    /// Queue length.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// True if nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Drops an inode (file deleted).
    pub fn forget(&mut self, node: Node) {
        self.queue.retain(|(n, _)| *n != node);
        self.queued.remove(&(node.subvol, node.ino));
    }
}

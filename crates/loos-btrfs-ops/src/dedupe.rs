//! Block-level duplicate detection for `FIDEDUPERANGE` / `BTRFS_IOC_FILE_EXTENT_SAME`, plus the
//! byte-compare primitive the ioctl itself needs. The scanner reads uncompressed regular files,
//! hashes aligned blocks (SHA-256), groups equal blocks, **verifies them byte for byte**, skips
//! ranges that already share an extent, and merges neighbouring blocks into runs.
use crate::consts::*;
use crate::dev::Dev;
use crate::error::Result;
use crate::fs::{ExtentKind, Fs, Kind, Node};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use loos_btrfs::{digest, ChecksumAlgorithm};

/// Scan parameters.
#[derive(Clone, Copy, Debug)]
pub struct ScanOptions {
    /// Block size (multiple of the sector size) compared at aligned file offsets.
    pub block_size: u64,
    /// Ignore runs shorter than this many bytes.
    pub min_run: u64,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions { block_size: 128 * 1024, min_run: 128 * 1024 }
    }
}

/// A file range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Loc {
    /// File.
    pub node: NodeKey,
    /// Byte offset in the file.
    pub offset: u64,
}

/// `Node` as an orderable key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeKey {
    /// Subvolume id.
    pub subvol: u64,
    /// Inode.
    pub ino: u64,
}

impl From<Node> for NodeKey {
    fn from(n: Node) -> Self {
        NodeKey { subvol: n.subvol, ino: n.ino }
    }
}

impl NodeKey {
    /// Back to a filesystem node.
    pub fn node(self) -> Node {
        Node { subvol: self.subvol, ino: self.ino }
    }
}

/// One deduplication opportunity: every `dsts` range holds the same bytes as `src`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupRun {
    /// The range to keep (source of the clone).
    pub src: Loc,
    /// Ranges that can be replaced by references to `src`.
    pub dsts: Vec<Loc>,
    /// Length of each range.
    pub len: u64,
}

/// Scan result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Opportunities, largest saving first.
    pub runs: Vec<DupRun>,
    /// Bytes hashed.
    pub bytes_scanned: u64,
    /// Bytes that would be freed if every run were applied.
    pub saved_bytes: u64,
    /// Files skipped (compressed, inline, too small, special).
    pub files_skipped: u64,
}

fn collect_files<D: Dev>(fs: &mut Fs<D>, dir: Node, out: &mut Vec<Node>, depth: u32) -> Result<()> {
    if depth > 64 {
        return Ok(());
    }
    for e in fs.readdir(dir)? {
        match e.kind {
            Kind::File => out.push(e.node),
            Kind::Dir => collect_files(fs, e.node, out, depth + 1)?,
            Kind::Subvolume => {
                let ri = fs.vol.root_item(e.node.subvol)?;
                collect_files(fs, Node { subvol: e.node.subvol, ino: ri.root_dirid }, out, depth + 1)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Physical identity of a file range's first sector: (extent bytenr + offset) if it is a plain regular extent.
fn phys_key<D: Dev>(fs: &mut Fs<D>, n: Node, off: u64) -> Option<u64> {
    let exts = fs.extents(n, off, off).ok()?;
    for e in exts {
        if let ExtentKind::Regular { disk_bytenr, offset, num_bytes, compression, prealloc, .. } = e.kind {
            if e.file_offset <= off && off < e.file_offset + num_bytes && compression == COMPRESS_NONE && !prealloc && disk_bytenr != 0 {
                return Some(disk_bytenr + offset + (off - e.file_offset));
            }
        }
    }
    None
}

/// True if the file has only plain uncompressed regular extents (candidates for dedupe).
fn dedupable<D: Dev>(fs: &mut Fs<D>, n: Node, size: u64) -> bool {
    if size == 0 {
        return false;
    }
    match fs.extents(n, 0, size - 1) {
        Ok(list) => !list.is_empty() && list.iter().all(|e| matches!(&e.kind, ExtentKind::Regular { compression, prealloc, disk_bytenr, .. } if *compression == COMPRESS_NONE && !*prealloc && *disk_bytenr != 0)),
        Err(_) => false,
    }
}

/// Scans every regular file below the default subvolume root.
pub fn scan<D: Dev>(fs: &mut Fs<D>, opts: &ScanOptions) -> Result<ScanReport> {
    let root = fs.root_node()?;
    let mut files = Vec::new();
    collect_files(fs, root, &mut files, 0)?;
    let ss = fs.vol.sectorsize() as u64;
    let bs = (opts.block_size.max(ss) + ss - 1) / ss * ss;
    let mut rep = ScanReport::default();
    // hash -> locations
    let mut groups: BTreeMap<[u8; 32], Vec<Loc>> = BTreeMap::new();
    let mut seen_nodes: Vec<Node> = Vec::new();
    for n in files {
        if seen_nodes.contains(&n) {
            continue; // hard link
        }
        seen_nodes.push(n);
        let st = fs.stat(n)?;
        if st.size < bs || !dedupable(fs, n, st.size) {
            rep.files_skipped += 1;
            continue;
        }
        let mut buf = vec![0u8; bs as usize];
        let mut off = 0u64;
        while off + bs <= st.size {
            if fs.read(n, off, &mut buf)? as u64 != bs {
                break;
            }
            rep.bytes_scanned += bs;
            let h = digest(ChecksumAlgorithm::Sha256, &buf);
            let mut key = [0u8; 32];
            key.copy_from_slice(h.as_bytes());
            groups.entry(key).or_default().push(Loc { node: n.into(), offset: off });
            off += bs;
        }
    }
    // candidate pairs: group head is the source; verify bytes; skip already shared
    let mut pairs: Vec<(Loc, Loc)> = Vec::new(); // (src, dst)
    for (_, locs) in groups.iter().filter(|(_, l)| l.len() > 1) {
        let src = locs[0];
        let mut srcbuf = vec![0u8; bs as usize];
        fs.read(src.node.node(), src.offset, &mut srcbuf)?;
        let src_phys = phys_key(fs, src.node.node(), src.offset);
        for dst in &locs[1..] {
            let mut b = vec![0u8; bs as usize];
            fs.read(dst.node.node(), dst.offset, &mut b)?;
            if b != srcbuf {
                continue; // hash collision (or racing writer): never dedupe unequal data
            }
            if src_phys.is_some() && src_phys == phys_key(fs, dst.node.node(), dst.offset) {
                continue; // already the same physical extent
            }
            pairs.push((src, *dst));
        }
    }
    // merge neighbouring blocks that are neighbours on both sides
    pairs.sort();
    let mut runs: Vec<(Loc, Loc, u64)> = Vec::new();
    for (s, d) in pairs {
        match runs.last_mut() {
            Some((ls, ld, len)) if ls.node == s.node && ld.node == d.node && ls.offset + *len == s.offset && ld.offset + *len == d.offset => *len += bs,
            _ => runs.push((s, d, bs)),
        }
    }
    // collapse identical sources into multi-destination runs
    let mut by_src: BTreeMap<(Loc, u64), Vec<Loc>> = BTreeMap::new();
    for (s, d, len) in runs {
        if len >= opts.min_run {
            by_src.entry((s, len)).or_default().push(d);
        }
    }
    for ((src, len), dsts) in by_src {
        rep.saved_bytes += len * dsts.len() as u64;
        rep.runs.push(DupRun { src, dsts, len });
    }
    rep.runs.sort_by(|a, b| (b.len * b.dsts.len() as u64).cmp(&(a.len * a.dsts.len() as u64)).then(a.src.cmp(&b.src)));
    Ok(rep)
}

/// Result of comparing two ranges for `FIDEDUPERANGE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeCmp {
    /// Equal: the dedupe may proceed.
    Same,
    /// Different (`FILE_DEDUPE_RANGE_DIFFERS`).
    Differs,
}

/// Compares `len` bytes of two files (the check `FIDEDUPERANGE` performs before sharing extents).
/// Ranges must be sector aligned except when they end at EOF.
pub fn compare_ranges<D: Dev>(fs: &mut Fs<D>, a: Node, a_off: u64, b: Node, b_off: u64, len: u64) -> Result<RangeCmp> {
    let ss = fs.vol.sectorsize() as u64;
    if a_off % ss != 0 || b_off % ss != 0 {
        return Err(crate::error::Error::Invalid("dedupe offsets must be sector aligned"));
    }
    let (sa, sb) = (fs.stat(a)?, fs.stat(b)?);
    let end_a = a_off.checked_add(len).ok_or(crate::error::Error::Invalid("range overflow"))?;
    let end_b = b_off.checked_add(len).ok_or(crate::error::Error::Invalid("range overflow"))?;
    if end_a > sa.size || end_b > sb.size {
        return Err(crate::error::Error::Invalid("range extends past EOF"));
    }
    if len % ss != 0 && !(end_a == sa.size && end_b == sb.size) {
        return Err(crate::error::Error::Invalid("unaligned length only allowed up to EOF of both files"));
    }
    let mut done = 0u64;
    let mut x = vec![0u8; 1 << 20];
    let mut y = vec![0u8; 1 << 20];
    while done < len {
        let n = core::cmp::min(x.len() as u64, len - done) as usize;
        fs.read(a, a_off + done, &mut x[..n])?;
        fs.read(b, b_off + done, &mut y[..n])?;
        if x[..n] != y[..n] {
            return Ok(RangeCmp::Differs);
        }
        done += n as u64;
    }
    Ok(RangeCmp::Same)
}

/// Human-readable path-less summary line (tools print this).
pub fn describe(r: &DupRun) -> String {
    alloc::format!("{}:{}@{} == {} other range(s), {} bytes each", r.src.node.subvol, r.src.node.ino, r.src.offset, r.dsts.len(), r.len)
}

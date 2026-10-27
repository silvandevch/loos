//! Offline structural + semantic checker (`btrfs check --readonly` equivalent).
//!
//! Phases: superblocks and devices → chunk/dev-extent/block-group agreement → every tree block
//! (checksum, identity, level, generation, key order, item packing, parent keys) → extent-tree
//! accounting (tree blocks, data extents, reference counts, block-group `used`) → free-space tree
//! → every filesystem tree (inodes, links, directory items/indexes, sizes, file extents, data
//! checksum coverage). Problems are returned as [`Finding`]s; the checker never writes.
use crate::consts::*;
use crate::dev::Dev;
use crate::error::Error;
use crate::fs::{name_hash, parse_dir_items};
use crate::raid::Profile;
use crate::volume::*;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Severity of a finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The filesystem is inconsistent.
    Error,
    /// Suspicious but not provably wrong.
    Warning,
}

/// One problem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// How bad.
    pub severity: Severity,
    /// Stable machine-readable class (`bad-csum`, `key-order`, `dir-index-missing`, ...).
    pub kind: &'static str,
    /// Tree the problem was found in (0 = not tree specific).
    pub tree: u64,
    /// Logical address of the tree block involved (0 = none).
    pub bytenr: u64,
    /// Key involved.
    pub key: Option<Key>,
    /// Human-readable detail.
    pub msg: String,
}

/// Counters collected while checking.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Trees walked.
    pub trees: u64,
    /// Tree blocks read (unique).
    pub tree_blocks: u64,
    /// Leaves among them.
    pub leaves: u64,
    /// Items seen (unique leaves).
    pub items: u64,
    /// Inodes in all filesystem trees.
    pub inodes: u64,
    /// Subvolumes (including the top level).
    pub subvolumes: u64,
    /// Distinct data extents referenced.
    pub data_extents: u64,
    /// Bytes in those extents.
    pub data_bytes: u64,
    /// Checksum items.
    pub csum_items: u64,
    /// Block groups.
    pub block_groups: u64,
}

/// Result of a check.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Everything that was found.
    pub findings: Vec<Finding>,
    /// Counters.
    pub stats: Stats,
    /// True if `max_findings` was hit and later problems were dropped.
    pub truncated: bool,
}

impl Report {
    /// Number of errors.
    pub fn errors(&self) -> usize {
        self.findings.iter().filter(|f| f.severity == Severity::Error).count()
    }
    /// No errors (warnings are allowed).
    pub fn is_clean(&self) -> bool {
        self.errors() == 0
    }
    /// True if any finding has this kind.
    pub fn has(&self, kind: &str) -> bool {
        self.findings.iter().any(|f| f.kind == kind)
    }
}

/// Checker options.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Read all data and verify checksums (slow).
    pub check_data_csums: bool,
    /// Stop recording after this many findings.
    pub max_findings: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { check_data_csums: false, max_findings: 10_000 }
    }
}

struct Ctx {
    rep: Report,
    max: usize,
}

impl Ctx {
    fn add(&mut self, severity: Severity, kind: &'static str, tree: u64, bytenr: u64, key: Option<Key>, msg: String) {
        if self.rep.findings.len() >= self.max {
            self.rep.truncated = true;
            return;
        }
        self.rep.findings.push(Finding { severity, kind, tree, bytenr, key, msg });
    }
    fn err(&mut self, kind: &'static str, tree: u64, bytenr: u64, key: Option<Key>, msg: String) {
        self.add(Severity::Error, kind, tree, bytenr, key, msg);
    }
    fn warn(&mut self, kind: &'static str, tree: u64, bytenr: u64, key: Option<Key>, msg: String) {
        self.add(Severity::Warning, kind, tree, bytenr, key, msg);
    }
}

/// Checks devices; open failures (no superblock, bad chunk tree) become findings.
pub fn check_devices<D: Dev>(devs: Vec<D>, opts: &Options) -> Report {
    match Volume::open(devs) {
        Ok(mut v) => check_volume(&mut v, opts),
        Err(e) => {
            let mut c = Ctx { rep: Report::default(), max: opts.max_findings };
            let kind = match e {
                Error::NotBtrfs => "no-valid-superblock",
                Error::Csum => "bad-csum",
                _ => "open-failed",
            };
            c.err(kind, 0, 0, None, format!("cannot open the filesystem: {}", e));
            c.rep
        }
    }
}

#[derive(Clone, Debug)]
struct MetaRec {
    refs: u64,
    flags: u64,
}

#[derive(Clone, Debug)]
struct DataRec {
    len: u64,
    refs: u64,
}

/// Everything collected from the extent tree.
#[derive(Default)]
struct ExtentIndex {
    meta: BTreeMap<u64, MetaRec>,
    data: BTreeMap<u64, DataRec>,
    block_groups: Vec<(u64, u64, u64, u64)>, // start, len, used, flags
}

/// Per-tree block bookkeeping shared across all trees.
#[derive(Default)]
struct Blocks {
    seen: BTreeMap<u64, (u8, u64)>, // bytenr -> (level, first tree)
}

fn inline_ref_count(d: &[u8], hdr_end: usize) -> u64 {
    // sum of reference counts of the inline refs following the extent item header
    let mut total = 0u64;
    let mut i = hdr_end;
    while i < d.len() {
        match d[i] {
            TREE_BLOCK_REF_KEY | SHARED_BLOCK_REF_KEY => {
                total += 1;
                i += 9;
            }
            EXTENT_DATA_REF_KEY => {
                if i + 29 > d.len() {
                    break;
                }
                total += le32(d, i + 1 + 24) as u64;
                i += 29;
            }
            SHARED_DATA_REF_KEY => {
                if i + 13 > d.len() {
                    break;
                }
                total += le32(d, i + 9) as u64;
                i += 13;
            }
            _ => break,
        }
    }
    total
}

/// Runs every check on an opened volume.
pub fn check_volume<D: Dev>(vol: &mut Volume<D>, opts: &Options) -> Report {
    let mut c = Ctx { rep: Report::default(), max: opts.max_findings };
    check_supers(vol, &mut c);
    check_chunks(vol, &mut c);
    let mut blocks = Blocks::default();

    // ---- root tree + chunk tree
    let root_ref = vol.sb.root;
    let mut roots: Vec<(Key, Vec<u8>)> = Vec::new();
    let mut root_refs: Vec<(u64, u64, u64, Vec<u8>)> = Vec::new();
    let mut root_backrefs: Vec<(u64, u64, Vec<u8>)> = Vec::new();
    walk_checked(vol, &mut c, &mut blocks, ROOT_TREE_OBJECTID, root_ref, &mut |k, d| {
        match k.ty {
            ROOT_ITEM_KEY => roots.push((*k, d.to_vec())),
            ROOT_REF_KEY => root_refs.push((k.objectid, k.offset, 0, d.to_vec())),
            ROOT_BACKREF_KEY => root_backrefs.push((k.objectid, k.offset, d.to_vec())),
            _ => {}
        }
    });
    c.rep.stats.trees += 1;
    let chunk_ref = vol.sb.chunk_root;
    let mut dev_items_seen: BTreeSet<u64> = BTreeSet::new();
    walk_checked(vol, &mut c, &mut blocks, CHUNK_TREE_OBJECTID, chunk_ref, &mut |k, _| {
        if k.ty == DEV_ITEM_KEY {
            dev_items_seen.insert(k.offset);
        }
    });
    c.rep.stats.trees += 1;
    for d in vol.devids() {
        if !dev_items_seen.contains(&d) {
            c.err("dev-item-missing", CHUNK_TREE_OBJECTID, 0, None, format!("device {} has no DEV_ITEM in the chunk tree", d));
        }
    }

    // ---- every other tree
    let mut ext_idx = ExtentIndex::default();
    let mut dev_extents: Vec<(u64, u64, u64, u64, u64)> = Vec::new(); // devid, physical, chunk_offset, length, chunk_objectid
    let mut csum_ranges: BTreeMap<u64, u64> = BTreeMap::new();
    let mut fs_roots: Vec<(u64, TreeRef, RootItem)> = Vec::new();
    let mut free_space: Vec<(u64, u64, u64, Vec<u8>)> = Vec::new(); // key.objectid, key.ty, key.offset ... flattened below
    let _ = &mut free_space;
    let mut fst_items: Vec<(Key, Vec<u8>)> = Vec::new();
    let mut have_fst = false;
    let mut bg_tree_items: Vec<(Key, Vec<u8>)> = Vec::new();
    let mut have_bg_tree = false;
    for (key, data) in roots.iter() {
        let ri = match RootItem::parse(data) {
            Ok(r) => r,
            Err(_) => {
                c.err("root-item", ROOT_TREE_OBJECTID, 0, Some(*key), "malformed root item".into());
                continue;
            }
        };
        let id = key.objectid;
        let tref = TreeRef { bytenr: ri.bytenr, level: ri.level };
        if ri.generation > vol.sb.generation + 1 {
            c.err("root-generation", id, ri.bytenr, Some(*key), format!("root generation {} newer than the superblock's {}", ri.generation, vol.sb.generation));
        }
        match id {
            EXTENT_TREE_OBJECTID => {
                walk_checked(vol, &mut c, &mut blocks, id, tref, &mut |k, d| collect_extent_item(&mut ext_idx, k, d, vol_nodesize(d, k)));
                c.rep.stats.trees += 1;
            }
            DEV_TREE_OBJECTID => {
                walk_checked(vol, &mut c, &mut blocks, id, tref, &mut |k, d| {
                    if k.ty == DEV_EXTENT_KEY && d.len() >= 48 {
                        dev_extents.push((k.objectid, k.offset, le64(d, 16), le64(d, 24), le64(d, 8)));
                    }
                });
                c.rep.stats.trees += 1;
            }
            CSUM_TREE_OBJECTID => {
                let cs = vol.csum_size() as u64;
                let ss = vol.sectorsize() as u64;
                let mut items = 0u64;
                walk_checked(vol, &mut c, &mut blocks, id, tref, &mut |k, d| {
                    if k.ty == EXTENT_CSUM_KEY && k.objectid == EXTENT_CSUM_OBJECTID {
                        items += 1;
                        let len = d.len() as u64 / cs * ss;
                        csum_ranges.insert(k.offset, len);
                    }
                });
                c.rep.stats.csum_items = items;
                c.rep.stats.trees += 1;
            }
            FREE_SPACE_TREE_OBJECTID => {
                have_fst = true;
                walk_checked(vol, &mut c, &mut blocks, id, tref, &mut |k, d| fst_items.push((*k, d.to_vec())));
                c.rep.stats.trees += 1;
            }
            BLOCK_GROUP_TREE_OBJECTID => {
                have_bg_tree = true;
                walk_checked(vol, &mut c, &mut blocks, id, tref, &mut |k, d| bg_tree_items.push((*k, d.to_vec())));
                c.rep.stats.trees += 1;
            }
            QUOTA_TREE_OBJECTID | UUID_TREE_OBJECTID | DATA_RELOC_TREE_OBJECTID => {
                walk_checked(vol, &mut c, &mut blocks, id, tref, &mut |_, _| {});
                c.rep.stats.trees += 1;
            }
            _ if id == FS_TREE_OBJECTID || (FIRST_FREE_OBJECTID..=LAST_FREE_OBJECTID).contains(&id) => {
                fs_roots.push((id, tref, ri));
            }
            TREE_LOG_OBJECTID | TREE_RELOC_OBJECTID => {
                c.warn("log-or-reloc-tree", id, ri.bytenr, Some(*key), "log/relocation tree present (needs replay)".into());
            }
            _ => {}
        }
    }
    if vol.sb.log_root != 0 {
        c.warn("log-tree", 0, vol.sb.log_root, None, "superblock has a log tree that has not been replayed".into());
    }
    // block-group items live in the extent tree or in the dedicated block-group tree
    if have_bg_tree {
        for (k, d) in &bg_tree_items {
            if k.ty == BLOCK_GROUP_ITEM_KEY && d.len() >= 24 {
                ext_idx.block_groups.push((k.objectid, k.offset, le64(d, 0), le64(d, 16)));
            }
        }
    }

    // ---- filesystem trees
    let mut fs_state = FsAgg::default();
    for (id, tref, ri) in &fs_roots {
        check_fs_tree(vol, &mut c, &mut blocks, *id, *tref, ri, &mut fs_state);
    }
    c.rep.stats.subvolumes = fs_roots.len() as u64;
    c.rep.stats.inodes = fs_state.inodes;

    // ---- root refs
    check_root_refs(&mut c, &roots, &root_refs, &root_backrefs);

    // ---- extent accounting
    check_extents(vol, &mut c, &blocks, &ext_idx, &fs_state);
    check_block_groups(vol, &mut c, &ext_idx, &dev_extents);
    if have_fst {
        check_free_space(vol, &mut c, &ext_idx, &fst_items);
    }
    check_csum_ranges(vol, &mut c, &csum_ranges, &fs_state);
    if opts.check_data_csums {
        let rep = crate::scrub::scrub_data_only(vol);
        for e in rep {
            c.err("data-csum", 0, e, None, format!("data checksum mismatch at logical {}", e));
        }
    }
    c.rep
}

fn vol_nodesize(_d: &[u8], _k: &Key) -> u64 {
    0
}

fn collect_extent_item(idx: &mut ExtentIndex, k: &Key, d: &[u8], _ns: u64) {
    match k.ty {
        EXTENT_ITEM_KEY if d.len() >= 24 => {
            let flags = le64(d, 16);
            if flags & 1 != 0 {
                idx.data.insert(k.objectid, DataRec { len: k.offset, refs: le64(d, 0) });
            } else {
                idx.meta.insert(k.objectid, MetaRec { refs: le64(d, 0), flags });
            }
            let _ = inline_ref_count(d, 24);
        }
        METADATA_ITEM_KEY if d.len() >= 24 => {
            idx.meta.insert(k.objectid, MetaRec { refs: le64(d, 0), flags: le64(d, 16) });
        }
        BLOCK_GROUP_ITEM_KEY if d.len() >= 24 => {
            idx.block_groups.push((k.objectid, k.offset, le64(d, 0), le64(d, 16)));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// superblocks / devices / chunks

fn check_supers<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx) {
    let main_gen = vol.sb.generation;
    let ids = vol.devids();
    let nodev = ids.len() as u64;
    if nodev < vol.sb.num_devices {
        c.warn("missing-devices", 0, 0, None, format!("{} of {} devices present", nodev, vol.sb.num_devices));
    }
    for id in ids {
        let dev_len = vol.dev_mut(id).map_or(0, |d| d.len());
        if let Some(item) = vol.dev_items.get(&id) {
            if item.total_bytes > dev_len {
                c.err("device-size", 0, 0, None, format!("device {} is {} bytes but the filesystem expects {}", id, dev_len, item.total_bytes));
            }
        }
        let mut valid = 0;
        for off in Volume::<D>::super_offsets(dev_len) {
            let mut raw = alloc::vec![0u8; SUPER_INFO_SIZE];
            if let Some(d) = vol.dev_mut(id) {
                if d.read_at(off, &mut raw).is_err() {
                    continue;
                }
            }
            match Super::parse(&raw, off) {
                Ok(s) => {
                    valid += 1;
                    if s.fsid != vol.sb.fsid {
                        c.err("super-fsid", 0, off, None, format!("superblock at {} on device {} has a different fsid", off, id));
                    }
                    if s.generation != main_gen && off != SUPER_MIRROR_OFFSETS[0] {
                        c.warn("super-mirror-generation", 0, off, None, format!("superblock mirror at {} has generation {} (primary {})", off, s.generation, main_gen));
                    }
                }
                Err(Error::NotBtrfs) if off != SUPER_MIRROR_OFFSETS[0] => {} // an unused mirror slot is fine
                Err(e) => c.warn("super-mirror", 0, off, None, format!("superblock at {} on device {}: {}", off, id, e)),
            }
        }
        if valid == 0 {
            c.err("no-valid-superblock", 0, 0, None, format!("device {} has no valid superblock", id));
        }
    }
}

fn check_chunks<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx) {
    let chunks = vol.chunks().to_vec();
    for ch in &chunks {
        if ch.validate().is_err() {
            c.err("chunk-geometry", CHUNK_TREE_OBJECTID, ch.logical, None, format!("chunk {} has invalid geometry", ch.logical));
            continue;
        }
        if let Some(p) = ch.profile() {
            if p.parity() > 0 && vol.sb.incompat & INCOMPAT_RAID56 == 0 {
                c.warn("raid56-flag", 0, ch.logical, None, "RAID5/6 chunk but the RAID56 incompat flag is not set".into());
            }
            if matches!(p, Profile::Raid1c3 | Profile::Raid1c4) && vol.sb.incompat & INCOMPAT_RAID1C34 == 0 {
                c.warn("raid1c34-flag", 0, ch.logical, None, "RAID1C3/C4 chunk but the RAID1C34 incompat flag is not set".into());
            }
        }
        let dev_len = ch.stripe_dev_len();
        for s in &ch.stripes {
            if let Some(it) = vol.dev_items.get(&s.devid) {
                if s.physical + dev_len > it.total_bytes {
                    c.err("chunk-beyond-device", CHUNK_TREE_OBJECTID, ch.logical, None, format!("chunk {} stripe on device {} ends at {} beyond its {} bytes", ch.logical, s.devid, s.physical + dev_len, it.total_bytes));
                }
            } else {
                c.err("chunk-unknown-device", CHUNK_TREE_OBJECTID, ch.logical, None, format!("chunk {} uses device {} which has no device item", ch.logical, s.devid));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// tree walking

fn walk_checked<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx, blocks: &mut Blocks, tree: u64, root: TreeRef, f: &mut dyn FnMut(&Key, &[u8])) {
    walk_block(vol, c, blocks, tree, root.bytenr, root.level, None, None, f);
}

#[allow(clippy::too_many_arguments)]
fn walk_block<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx, blocks: &mut Blocks, tree: u64, bytenr: u64, level: u8, parent_key: Option<Key>, parent_gen: Option<u64>, f: &mut dyn FnMut(&Key, &[u8])) {
    let ns = vol.sb.nodesize as u64;
    if bytenr % vol.sb.sectorsize as u64 != 0 || vol.chunk_for(bytenr).is_none() {
        c.err("bad-block-pointer", tree, bytenr, parent_key, format!("pointer {} is unaligned or not inside any chunk", bytenr));
        return;
    }
    let first_visit = !blocks.seen.contains_key(&bytenr);
    let (raw, st) = match vol.read_tree_block_raw(bytenr, Some(level)) {
        Ok(x) => x,
        Err(e) => {
            if first_visit {
                blocks.seen.insert(bytenr, (level, tree));
                c.err("block-unreadable", tree, bytenr, parent_key, format!("cannot read tree block: {}", e));
            }
            return;
        }
    };
    if first_visit {
        blocks.seen.insert(bytenr, (level, tree));
        c.rep.stats.tree_blocks += 1;
        if !st.csum_ok {
            c.err("bad-csum", tree, bytenr, parent_key, "tree block checksum mismatch".into());
        }
        if !st.fsid_ok {
            c.err("bad-fsid", tree, bytenr, parent_key, "tree block belongs to another filesystem".into());
        }
        if !st.bytenr_ok {
            c.err("bad-bytenr", tree, bytenr, parent_key, format!("header says bytenr {}", le64(&raw, 48)));
        }
        if !st.level_ok {
            c.err("bad-level", tree, bytenr, parent_key, format!("expected level {}, header says {}", level, raw[100]));
        }
        if !st.generation_ok {
            c.err("bad-generation", tree, bytenr, parent_key, format!("block generation {} is newer than the superblock's {}", le64(&raw, 80), vol.sb.generation));
        }
    }
    if !(st.csum_ok && st.fsid_ok && st.bytenr_ok && st.level_ok) {
        return; // do not trust the content
    }
    if let Some(pg) = parent_gen {
        if first_visit && le64(&raw, 80) != pg {
            c.err("generation-mismatch", tree, bytenr, parent_key, format!("parent expects generation {} but the block has {}", pg, le64(&raw, 80)));
        }
    }
    if first_visit && tree < FIRST_FREE_OBJECTID && tree != 0 && le64(&raw, 88) != tree && tree != FREE_SPACE_TREE_OBJECTID {
        c.warn("owner-mismatch", tree, bytenr, parent_key, format!("header owner {} differs from tree {}", le64(&raw, 88), tree));
    }
    let n = le32(&raw, 96) as usize;
    if level == 0 {
        if HEADER_SIZE + n * ITEM_SIZE > raw.len() {
            c.err("bad-item-count", tree, bytenr, None, format!("{} items do not fit in the leaf", n));
            return;
        }
        let mut prev: Option<Key> = None;
        let mut expect_end = ns as usize - HEADER_SIZE;
        let table_end = HEADER_SIZE + n * ITEM_SIZE;
        let mut ok = true;
        let mut items: Vec<(Key, usize, usize)> = Vec::with_capacity(n);
        for i in 0..n {
            let o = HEADER_SIZE + i * ITEM_SIZE;
            let key = Key::parse(&raw[o..]);
            let off = le32(&raw, o + 17) as usize;
            let size = le32(&raw, o + 21) as usize;
            if let Some(p) = prev {
                if key <= p {
                    if first_visit {
                        c.err("key-order", tree, bytenr, Some(key), format!("item {} key {:?} is not greater than the previous {:?}", i, key, p));
                    }
                    ok = false;
                }
            }
            prev = Some(key);
            if off.checked_add(size) != Some(expect_end) || HEADER_SIZE + off < table_end && size > 0 {
                if first_visit {
                    c.err("item-bounds", tree, bytenr, Some(key), format!("item {} data [{}, +{}) does not pack against the previous item (expected end {})", i, off, size, expect_end));
                }
                ok = false;
            }
            expect_end = off;
            items.push((key, off, size));
        }
        if let (Some(pk), Some(first)) = (parent_key, items.first()) {
            if first_visit && first.0 != pk {
                c.err("parent-key", tree, bytenr, Some(first.0), format!("first key {:?} does not match the parent's pointer key {:?}", first.0, pk));
            }
        }
        if first_visit {
            c.rep.stats.leaves += 1;
            c.rep.stats.items += n as u64;
        }
        if !ok {
            return;
        }
        for (key, off, size) in items {
            let s = HEADER_SIZE + off;
            f(&key, &raw[s..s + size]);
        }
    } else {
        if HEADER_SIZE + n * KEY_PTR_SIZE > raw.len() || n == 0 {
            c.err("bad-item-count", tree, bytenr, None, format!("node with {} pointers", n));
            return;
        }
        let mut prev: Option<Key> = None;
        let mut ptrs = Vec::with_capacity(n);
        for i in 0..n {
            let o = HEADER_SIZE + i * KEY_PTR_SIZE;
            let key = Key::parse(&raw[o..]);
            if let Some(p) = prev {
                if key <= p && first_visit {
                    c.err("key-order", tree, bytenr, Some(key), format!("node pointer {} key {:?} is not greater than the previous {:?}", i, key, p));
                }
            }
            prev = Some(key);
            ptrs.push((key, le64(&raw, o + 17), le64(&raw, o + 25)));
        }
        if let (Some(pk), Some(first)) = (parent_key, ptrs.first()) {
            if first_visit && first.0 != pk {
                c.err("parent-key", tree, bytenr, Some(first.0), format!("first key {:?} does not match the parent's pointer key {:?}", first.0, pk));
            }
        }
        for (key, ptr, gen) in ptrs {
            walk_block(vol, c, blocks, tree, ptr, level - 1, Some(key), Some(gen), f);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// filesystem trees

#[derive(Default)]
struct FsAgg {
    inodes: u64,
    /// data extent bytenr -> (disk_num_bytes, refs found, first referencing tree)
    data_refs: BTreeMap<u64, (u64, u64)>,
    /// checksummed byte ranges that some extent needs: (start, len, tree, ino)
    need_csum: Vec<(u64, u64, u64, u64)>,
    /// ranges referenced by any extent (for orphan csum detection): start -> end
    extent_ranges: BTreeMap<u64, u64>,
    /// unique leaves already counted for reference accounting
    counted_leaves: BTreeSet<u64>,
}

#[derive(Default, Clone)]
struct InodeInfo {
    exists: bool,
    nlink: u32,
    mode: u32,
    size: u64,
    nbytes: u64,
    flags: u64,
    refs_found: u32,
    dir_name_bytes: u64,
    first_extent_end: u64,
}

struct ExtentRec {
    offset: u64,
    len: u64,
    inline: bool,
    prealloc: bool,
    compressed: bool,
    disk_bytenr: u64,
    disk_num_bytes: u64,
    ext_offset: u64,
    ram_bytes: u64,
    inline_len: u64,
}

fn check_fs_tree<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx, blocks: &mut Blocks, tree: u64, root: TreeRef, ri: &RootItem, agg: &mut FsAgg) {
    // Collect all items of the tree; leaf sharing across snapshots is handled by the walker
    // revisiting shared leaves (so each tree gets a complete picture) while reference counting
    // only happens on the first visit of a leaf block.
    let mut items: Vec<(Key, Vec<u8>)> = Vec::new();
    {
        let mut visit = |k: &Key, d: &[u8]| items.push((*k, d.to_vec()));
        walk_checked(vol, c, blocks, tree, root, &mut visit);
    }
    c.rep.stats.trees += 1;
    let ss = vol.sectorsize() as u64;
    let no_holes = vol.sb.incompat & INCOMPAT_NO_HOLES != 0;
    let mut inodes: BTreeMap<u64, InodeInfo> = BTreeMap::new();
    let mut dir_items: Vec<(u64, u64, Vec<u8>, Key, u8)> = Vec::new(); // dir, key.offset(hash), name, location, type
    let mut dir_index: Vec<(u64, u64, Vec<u8>, Key, u8)> = Vec::new();
    let mut inode_refs: Vec<(u64, u64, u64, Vec<u8>)> = Vec::new(); // ino, parent, index, name
    let mut extents: BTreeMap<u64, Vec<ExtentRec>> = BTreeMap::new();

    for (k, d) in &items {
        match k.ty {
            INODE_ITEM_KEY => {
                if d.len() < 160 {
                    c.err("inode-item-size", tree, 0, Some(*k), "inode item shorter than 160 bytes".into());
                    continue;
                }
                let e = inodes.entry(k.objectid).or_default();
                e.exists = true;
                e.nlink = le32(d, 40);
                e.mode = le32(d, 52);
                e.size = le64(d, 16);
                e.nbytes = le64(d, 24);
                e.flags = le64(d, 64);
            }
            INODE_REF_KEY => {
                let mut i = 0;
                while i + 10 <= d.len() {
                    let nl = le16(d, i + 8) as usize;
                    if i + 10 + nl > d.len() {
                        c.err("inode-ref", tree, 0, Some(*k), "inode ref name overruns the item".into());
                        break;
                    }
                    inode_refs.push((k.objectid, k.offset, le64(d, i), d[i + 10..i + 10 + nl].to_vec()));
                    i += 10 + nl;
                }
            }
            INODE_EXTREF_KEY => {
                let mut i = 0;
                while i + 18 <= d.len() {
                    let nl = le16(d, i + 16) as usize;
                    if i + 18 + nl > d.len() {
                        c.err("inode-extref", tree, 0, Some(*k), "inode extref name overruns the item".into());
                        break;
                    }
                    inode_refs.push((k.objectid, le64(d, i), le64(d, i + 8), d[i + 18..i + 18 + nl].to_vec()));
                    i += 18 + nl;
                }
            }
            DIR_ITEM_KEY | DIR_INDEX_KEY => match parse_dir_items(d) {
                Ok(list) => {
                    for (loc, ty, name, _) in list {
                        if k.ty == DIR_ITEM_KEY {
                            if name_hash(&name) != k.offset {
                                c.err("dir-item-hash", tree, 0, Some(*k), format!("name {:?} does not hash to the key offset", String::from_utf8_lossy(&name)));
                            }
                            dir_items.push((k.objectid, k.offset, name, loc, ty));
                        } else {
                            dir_index.push((k.objectid, k.offset, name, loc, ty));
                        }
                    }
                }
                Err(_) => c.err("dir-item", tree, 0, Some(*k), "malformed directory item".into()),
            },
            XATTR_ITEM_KEY => match parse_dir_items(d) {
                Ok(list) => {
                    for (_, _, name, _) in list {
                        if name_hash(&name) != k.offset {
                            c.err("xattr-hash", tree, 0, Some(*k), "xattr name does not hash to the key offset".into());
                        }
                    }
                }
                Err(_) => c.err("xattr-item", tree, 0, Some(*k), "malformed xattr item".into()),
            },
            EXTENT_DATA_KEY => {
                if d.len() < 21 {
                    c.err("file-extent-size", tree, 0, Some(*k), "file extent item too short".into());
                    continue;
                }
                let ty = d[20];
                let comp = d[16] != 0;
                let ram = le64(d, 8);
                let rec = match ty {
                    FILE_EXTENT_INLINE => ExtentRec { offset: k.offset, len: ram, inline: true, prealloc: false, compressed: comp, disk_bytenr: 0, disk_num_bytes: 0, ext_offset: 0, ram_bytes: ram, inline_len: (d.len() - 21) as u64 },
                    FILE_EXTENT_REG | FILE_EXTENT_PREALLOC if d.len() >= 53 => ExtentRec {
                        offset: k.offset,
                        len: le64(d, 45),
                        inline: false,
                        prealloc: ty == FILE_EXTENT_PREALLOC,
                        compressed: comp,
                        disk_bytenr: le64(d, 21),
                        disk_num_bytes: le64(d, 29),
                        ext_offset: le64(d, 37),
                        ram_bytes: ram,
                        inline_len: 0,
                    },
                    _ => {
                        c.err("file-extent-type", tree, 0, Some(*k), format!("bad file extent type {} or size {}", ty, d.len()));
                        continue;
                    }
                };
                extents.entry(k.objectid).or_default().push(rec);
            }
            _ => {}
        }
    }

    // ---- reference accounting for data extents: count each *leaf* once across all trees.
    // (Leaf identity is not tracked per item here; shared leaves are rare, so count per first tree
    // that sees the extent through an unshared path — see `leaf_sharing` below.)
    count_data_refs(vol, c, blocks, tree, root, agg);

    // ---- inode checks
    if !inodes.contains_key(&FIRST_FREE_OBJECTID) && ri.root_dirid == FIRST_FREE_OBJECTID {
        c.err("root-dir-missing", tree, 0, None, "subvolume has no root directory inode".into());
    }
    let mut ref_count: BTreeMap<u64, u32> = BTreeMap::new();
    for (ino, _, _, _) in &inode_refs {
        *ref_count.entry(*ino).or_insert(0) += 1;
    }
    for (ino, n) in &ref_count {
        inodes.entry(*ino).or_default().refs_found = *n;
    }
    for (dir, _, name, _, _) in dir_index.iter() {
        inodes.entry(*dir).or_default().dir_name_bytes += name.len() as u64 * 2;
    }
    agg.inodes += inodes.values().filter(|i| i.exists).count() as u64;
    let idx_by_name: BTreeMap<(u64, Vec<u8>), (u64, Key)> = dir_index.iter().map(|(d, idx, n, l, _)| ((*d, n.clone()), (*idx, *l))).collect();
    let item_by_name: BTreeSet<(u64, Vec<u8>)> = dir_items.iter().map(|(d, _, n, _, _)| (*d, n.clone())).collect();
    for (dir, _, name, loc, ty) in &dir_items {
        match idx_by_name.get(&(*dir, name.clone())) {
            None => c.err("dir-index-missing", tree, 0, Some(Key::new(*dir, DIR_ITEM_KEY, name_hash(name))), format!("DIR_ITEM {:?} has no DIR_INDEX", String::from_utf8_lossy(name))),
            Some((_, l2)) if l2 != loc => c.err("dir-index-mismatch", tree, 0, Some(*loc), format!("DIR_ITEM and DIR_INDEX for {:?} point at different targets", String::from_utf8_lossy(name))),
            _ => {}
        }
        let _ = ty;
    }
    for (dir, idx, name, loc, ty) in &dir_index {
        if !item_by_name.contains(&(*dir, name.clone())) {
            c.err("dir-item-missing", tree, 0, Some(Key::new(*dir, DIR_INDEX_KEY, *idx)), format!("DIR_INDEX {:?} has no DIR_ITEM", String::from_utf8_lossy(name)));
        }
        if loc.ty == ROOT_ITEM_KEY {
            continue; // subvolume entry: checked through root refs
        }
        match inodes.get(&loc.objectid) {
            Some(t) if t.exists => {
                let kind_ok = match *ty {
                    FT_DIR => t.mode & 0o170000 == 0o040000,
                    FT_REG_FILE => t.mode & 0o170000 == 0o100000,
                    FT_SYMLINK => t.mode & 0o170000 == 0o120000,
                    _ => true,
                };
                if !kind_ok {
                    c.err("dir-item-type", tree, 0, Some(Key::new(*dir, DIR_INDEX_KEY, *idx)), format!("entry {:?} claims type {} but inode {} has mode {:o}", String::from_utf8_lossy(name), ty, loc.objectid, t.mode));
                }
                let backref = inode_refs.iter().any(|(i, p, ix, n)| *i == loc.objectid && *p == *dir && *ix == *idx && n == name);
                if !backref {
                    c.err("backref-missing", tree, 0, Some(Key::new(loc.objectid, INODE_REF_KEY, *dir)), format!("inode {} has no INODE_REF for {:?} in directory {} (index {})", loc.objectid, String::from_utf8_lossy(name), dir, idx));
                }
            }
            _ => c.err("dir-item-orphan", tree, 0, Some(Key::new(*dir, DIR_INDEX_KEY, *idx)), format!("entry {:?} points at missing inode {}", String::from_utf8_lossy(name), loc.objectid)),
        }
    }
    // every INODE_REF must be backed by a directory entry
    for (ino, parent, idx, name) in &inode_refs {
        if *ino == *parent && *ino == FIRST_FREE_OBJECTID {
            continue; // subvolume root refers to itself
        }
        match idx_by_name.get(&(*parent, name.clone())) {
            Some((i2, loc)) if *i2 == *idx && loc.objectid == *ino => {}
            _ => c.err("inode-ref-orphan", tree, 0, Some(Key::new(*ino, INODE_REF_KEY, *parent)), format!("INODE_REF {:?} (parent {}, index {}) has no matching directory entry", String::from_utf8_lossy(name), parent, idx)),
        }
    }
    for (ino, info) in &inodes {
        if !info.exists {
            // items for an inode without INODE_ITEM
            if *ino >= FIRST_FREE_OBJECTID && *ino <= LAST_FREE_OBJECTID && (info.refs_found > 0 || info.dir_name_bytes > 0) {
                c.err("inode-missing", tree, 0, Some(Key::new(*ino, INODE_ITEM_KEY, 0)), format!("inode {} has references but no INODE_ITEM", ino));
            }
            continue;
        }
        let expected_links = if *ino == FIRST_FREE_OBJECTID { info.refs_found.max(1) } else { info.refs_found };
        if info.nlink != expected_links && info.nlink != 0 {
            c.err("nlink", tree, 0, Some(Key::new(*ino, INODE_ITEM_KEY, 0)), format!("inode {} nlink {} but {} back references", ino, info.nlink, info.refs_found));
        }
        if info.nlink == 0 && info.refs_found == 0 && *ino != FIRST_FREE_OBJECTID {
            // orphan awaiting reclamation: reported as a warning
            c.warn("orphan-inode", tree, 0, Some(Key::new(*ino, INODE_ITEM_KEY, 0)), format!("inode {} is unlinked (orphan item pending)", ino));
        }
        let is_dir = info.mode & 0o170000 == 0o040000;
        if is_dir && info.size != info.dir_name_bytes {
            c.err("dir-size", tree, 0, Some(Key::new(*ino, INODE_ITEM_KEY, 0)), format!("directory {} has i_size {} but its names add up to {}", ino, info.size, info.dir_name_bytes));
        }
    }
    // ---- file extents
    for (ino, list) in extents.iter_mut() {
        list.sort_by_key(|e| e.offset);
        let info = inodes.get(ino).cloned().unwrap_or_default();
        let nodatasum = info.flags & INODE_NODATASUM != 0;
        let mut end = 0u64;
        let mut nbytes = 0u64;
        let mut all_plain = true;
        let mut saw_regular = false;
        for e in list.iter() {
            let key = Some(Key::new(*ino, EXTENT_DATA_KEY, e.offset));
            if e.offset < end {
                c.err("extent-overlap", tree, 0, key, format!("inode {} extent at {} overlaps the previous one (ends {})", ino, e.offset, end));
            } else if e.offset > end && !no_holes && !(e.inline) {
                c.err("extent-hole", tree, 0, key, format!("inode {} has an unexplained hole [{}, {}) (NO_HOLES is off)", ino, end, e.offset));
            }
            if e.inline {
                if e.offset != 0 {
                    c.err("inline-extent-offset", tree, 0, key, format!("inline extent of inode {} at offset {}", ino, e.offset));
                }
                if saw_regular {
                    c.err("inline-with-regular", tree, 0, key, format!("inode {} mixes inline and regular extents", ino));
                }
                if !e.compressed && e.inline_len != e.ram_bytes {
                    c.err("inline-size", tree, 0, key, format!("inode {} inline extent stores {} bytes but ram_bytes is {}", ino, e.inline_len, e.ram_bytes));
                }
                end = e.offset + e.len;
                nbytes += e.ram_bytes;
                continue;
            }
            saw_regular = true;
            if e.offset % ss != 0 || e.len % ss != 0 {
                c.err("extent-alignment", tree, 0, key, format!("inode {} extent [{}, +{}) is not sector aligned", ino, e.offset, e.len));
            }
            end = e.offset + e.len;
            if e.compressed {
                all_plain = false;
            }
            if e.disk_bytenr != 0 {
                if !e.prealloc {
                    nbytes += e.len;
                } else {
                    nbytes += e.len;
                }
                if e.ext_offset + e.len > e.ram_bytes && !e.compressed {
                    c.err("extent-range", tree, 0, key, format!("inode {} extent uses [{}, +{}) of a {} byte extent", ino, e.ext_offset, e.len, e.ram_bytes));
                }
                agg.extent_ranges.entry(e.disk_bytenr).and_modify(|x| *x = (*x).max(e.disk_bytenr + e.disk_num_bytes)).or_insert(e.disk_bytenr + e.disk_num_bytes);
                if !e.prealloc && !nodatasum {
                    let (s, l) = if e.compressed { (e.disk_bytenr, e.disk_num_bytes) } else { (e.disk_bytenr + e.ext_offset, e.len) };
                    agg.need_csum.push((s, l, tree, *ino));
                }
            } else {
                // explicit hole extent
                if e.len == 0 {
                    c.err("extent-zero", tree, 0, key, format!("inode {} has a zero-length hole extent", ino));
                }
            }
        }
        if info.exists && info.mode & 0o170000 == 0o100000 {
            let aligned = (info.size + ss - 1) / ss * ss;
            let last_real = list.iter().rev().find(|e| !e.prealloc);
            if let Some(l) = last_real {
                if !l.inline && l.offset + l.len > aligned {
                    c.err("extent-beyond-size", tree, 0, Some(Key::new(*ino, EXTENT_DATA_KEY, l.offset)), format!("inode {} has data past i_size {}", ino, info.size));
                }
                if l.inline && info.size != l.ram_bytes {
                    c.err("inline-file-size", tree, 0, Some(Key::new(*ino, INODE_ITEM_KEY, 0)), format!("inode {} i_size {} but its inline data is {} bytes", ino, info.size, l.ram_bytes));
                }
            }
            if all_plain && info.nbytes != nbytes {
                c.err("nbytes", tree, 0, Some(Key::new(*ino, INODE_ITEM_KEY, 0)), format!("inode {} nbytes {} but its extents add up to {}", ino, info.nbytes, nbytes));
            }
        }
    }
    // inodes that claim size but have no extents are sparse files: nothing to check
}

/// Counts data-extent references of one filesystem tree on leaves not seen before.
fn count_data_refs<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx, _blocks: &mut Blocks, tree: u64, root: TreeRef, agg: &mut FsAgg) {
    // second light-weight walk that only touches leaves not yet counted
    fn rec<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx, agg: &mut FsAgg, tree: u64, bytenr: u64, level: u8) {
        let raw = match vol.read_tree_block_raw(bytenr, Some(level)) {
            Ok((r, st)) if st.csum_ok && st.bytenr_ok => r,
            _ => return,
        };
        if level == 0 {
            if !agg.counted_leaves.insert(bytenr) {
                return;
            }
            let n = le32(&raw, 96) as usize;
            if HEADER_SIZE + n * ITEM_SIZE > raw.len() {
                return;
            }
            for i in 0..n {
                let o = HEADER_SIZE + i * ITEM_SIZE;
                let key = Key::parse(&raw[o..]);
                if key.ty != EXTENT_DATA_KEY {
                    continue;
                }
                let off = HEADER_SIZE + le32(&raw, o + 17) as usize;
                let size = le32(&raw, o + 21) as usize;
                if off + size > raw.len() || size < 53 || raw[off + 20] == FILE_EXTENT_INLINE {
                    continue;
                }
                let disk = le64(&raw, off + 21);
                if disk == 0 {
                    continue;
                }
                let e = agg.data_refs.entry(disk).or_insert((le64(&raw, off + 29), 0));
                e.1 += 1;
                if e.0 != le64(&raw, off + 29) {
                    c.err("extent-size-mismatch", tree, bytenr, Some(key), format!("extent at {} referenced with sizes {} and {}", disk, e.0, le64(&raw, off + 29)));
                }
            }
            return;
        }
        let n = le32(&raw, 96) as usize;
        if HEADER_SIZE + n * KEY_PTR_SIZE > raw.len() {
            return;
        }
        for i in 0..n {
            let ptr = le64(&raw, HEADER_SIZE + i * KEY_PTR_SIZE + 17);
            rec(vol, c, agg, tree, ptr, level - 1);
        }
    }
    rec(vol, c, agg, tree, root.bytenr, root.level);
}

fn check_root_refs(c: &mut Ctx, roots: &[(Key, Vec<u8>)], refs: &[(u64, u64, u64, Vec<u8>)], backrefs: &[(u64, u64, Vec<u8>)]) {
    let known: BTreeSet<u64> = roots.iter().map(|(k, _)| k.objectid).collect();
    for (parent, child, _, d) in refs {
        if d.len() < 18 {
            c.err("root-ref", ROOT_TREE_OBJECTID, 0, Some(Key::new(*parent, ROOT_REF_KEY, *child)), "root ref too short".into());
            continue;
        }
        let nl = le16(d, 16) as usize;
        let name = if 18 + nl <= d.len() { d[18..18 + nl].to_vec() } else { Vec::new() };
        if !known.contains(child) {
            c.err("root-ref-orphan", ROOT_TREE_OBJECTID, 0, Some(Key::new(*parent, ROOT_REF_KEY, *child)), format!("ROOT_REF points at missing subvolume {}", child));
        }
        let back = backrefs.iter().find(|(c2, p2, _)| c2 == child && p2 == parent);
        match back {
            None => c.err("root-backref-missing", ROOT_TREE_OBJECTID, 0, Some(Key::new(*child, ROOT_BACKREF_KEY, *parent)), format!("subvolume {} lacks ROOT_BACKREF to {}", child, parent)),
            Some((_, _, bd)) => {
                if bd.len() >= 18 {
                    let bl = le16(bd, 16) as usize;
                    if 18 + bl > bd.len() || bd[18..18 + bl] != name[..] || le64(bd, 0) != le64(d, 0) {
                        c.err("root-backref-mismatch", ROOT_TREE_OBJECTID, 0, Some(Key::new(*child, ROOT_BACKREF_KEY, *parent)), format!("ROOT_REF/ROOT_BACKREF of subvolume {} disagree", child));
                    }
                }
            }
        }
    }
    for (child, parent, _) in backrefs {
        if !refs.iter().any(|(p2, c2, _, _)| p2 == parent && c2 == child) {
            c.err("root-ref-missing", ROOT_TREE_OBJECTID, 0, Some(Key::new(*parent, ROOT_REF_KEY, *child)), format!("ROOT_BACKREF of {} has no ROOT_REF in {}", child, parent));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// extent accounting

fn check_extents<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx, blocks: &Blocks, idx: &ExtentIndex, fs: &FsAgg) {
    let ns = vol.sb.nodesize as u64;
    for (bytenr, (level, tree)) in &blocks.seen {
        match idx.meta.get(bytenr) {
            None => c.err("extent-missing", *tree, *bytenr, None, format!("tree block {} (level {}) has no extent item", bytenr, level)),
            Some(m) => {
                if m.refs == 0 {
                    c.err("extent-refs", *tree, *bytenr, None, format!("tree block {} has refs 0", bytenr));
                }
                if m.flags & 2 == 0 {
                    c.err("extent-flags", *tree, *bytenr, None, format!("tree block {} extent lacks the TREE_BLOCK flag", bytenr));
                }
            }
        }
    }
    for bytenr in idx.meta.keys() {
        if !blocks.seen.contains_key(bytenr) {
            c.err("extent-unreferenced", 0, *bytenr, None, format!("extent item for tree block {} but no tree references it", bytenr));
        }
    }
    let mut data_bytes = 0u64;
    for (bytenr, (len, refs)) in &fs.data_refs {
        data_bytes += *len;
        match idx.data.get(bytenr) {
            None => c.err("extent-missing", 0, *bytenr, None, format!("file extent at {} ({} bytes) has no extent item", bytenr, len)),
            Some(d) => {
                if d.len != *len {
                    c.err("extent-length", 0, *bytenr, None, format!("extent item at {} is {} bytes but file extents use {}", bytenr, d.len, len));
                }
                if d.refs != *refs {
                    c.err("extent-refs", 0, *bytenr, None, format!("extent at {} has refs {} but {} file extent items reference it", bytenr, d.refs, refs));
                }
            }
        }
    }
    for (bytenr, d) in &idx.data {
        if !fs.data_refs.contains_key(bytenr) {
            c.err("extent-unreferenced", 0, *bytenr, None, format!("data extent {} ({} bytes, refs {}) is not referenced by any file", bytenr, d.len, d.refs));
        }
    }
    c.rep.stats.data_extents = fs.data_refs.len() as u64;
    c.rep.stats.data_bytes = data_bytes;
    // overlap between extent records
    let mut all: Vec<(u64, u64)> = idx.meta.keys().map(|b| (*b, ns)).chain(idx.data.iter().map(|(b, d)| (*b, d.len))).collect();
    all.sort();
    for w in all.windows(2) {
        if w[0].0 + w[0].1 > w[1].0 {
            c.err("extent-overlap", 0, w[0].0, None, format!("extents [{}, +{}) and [{}, +{}) overlap", w[0].0, w[0].1, w[1].0, w[1].1));
        }
    }
}

fn check_block_groups<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx, idx: &ExtentIndex, dev_extents: &[(u64, u64, u64, u64, u64)]) {
    let ns = vol.sb.nodesize as u64;
    let chunks = vol.chunks().to_vec();
    c.rep.stats.block_groups = idx.block_groups.len() as u64;
    let mut total_used = 0u64;
    for (start, len, used, flags) in &idx.block_groups {
        total_used += *used;
        match chunks.iter().find(|ch| ch.logical == *start) {
            None => c.err("bg-no-chunk", 0, *start, None, format!("block group {} has no chunk", start)),
            Some(ch) => {
                if ch.length != *len || (ch.flags ^ *flags) & (BLOCK_GROUP_TYPE_MASK | BLOCK_GROUP_PROFILE_MASK) != 0 {
                    c.err("bg-chunk-mismatch", 0, *start, None, format!("block group {} (len {}, flags {:x}) disagrees with its chunk (len {}, flags {:x})", start, len, flags, ch.length, ch.flags));
                }
            }
        }
        let meta: u64 = idx.meta.range(*start..start + len).count() as u64 * ns;
        let data: u64 = idx.data.range(*start..start + len).map(|(_, d)| d.len).sum();
        let counted = meta + data;
        if counted != *used {
            c.err("bg-used", 0, *start, None, format!("block group {} says used {} but its extents add up to {}", start, used, counted));
        }
    }
    for ch in &chunks {
        if !idx.block_groups.iter().any(|b| b.0 == ch.logical) {
            c.err("chunk-no-bg", 0, ch.logical, None, format!("chunk {} has no block group item", ch.logical));
        }
    }
    if vol.sb.bytes_used != total_used {
        c.err("super-bytes-used", 0, 0, None, format!("superblock bytes_used {} but block groups add up to {}", vol.sb.bytes_used, total_used));
    }
    // chunk stripes <-> dev extents
    let mut seen_ext: BTreeSet<(u64, u64)> = BTreeSet::new();
    for ch in &chunks {
        let dl = ch.stripe_dev_len();
        for s in &ch.stripes {
            match dev_extents.iter().find(|e| e.0 == s.devid && e.1 == s.physical) {
                None => c.err("dev-extent-missing", DEV_TREE_OBJECTID, ch.logical, None, format!("chunk {} stripe at device {} offset {} has no dev extent", ch.logical, s.devid, s.physical)),
                Some(e) => {
                    seen_ext.insert((s.devid, s.physical));
                    if e.2 != ch.logical || e.3 != dl {
                        c.err("dev-extent-mismatch", DEV_TREE_OBJECTID, ch.logical, None, format!("dev extent on device {} at {} says chunk {} length {} (chunk {} stripe length {})", s.devid, s.physical, e.2, e.3, ch.logical, dl));
                    }
                }
            }
        }
    }
    for e in dev_extents {
        if !seen_ext.contains(&(e.0, e.1)) {
            c.err("dev-extent-orphan", DEV_TREE_OBJECTID, e.2, None, format!("dev extent on device {} at {} (chunk {}) belongs to no chunk stripe", e.0, e.1, e.2));
        }
    }
    // device bytes_used == sum of its dev extents
    let items: Vec<DevItem> = vol.dev_items.values().copied().collect();
    for it in items {
        let sum: u64 = dev_extents.iter().filter(|e| e.0 == it.devid).map(|e| e.3).sum();
        if sum != it.bytes_used {
            c.err("dev-bytes-used", CHUNK_TREE_OBJECTID, 0, None, format!("device {} bytes_used {} but its dev extents add up to {}", it.devid, it.bytes_used, sum));
        }
    }
}

fn check_free_space<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx, idx: &ExtentIndex, fst: &[(Key, Vec<u8>)]) {
    let ns = vol.sb.nodesize as u64;
    let ss = vol.sb.sectorsize as u64;
    for (start, len, used, _) in &idx.block_groups {
        // used ranges inside the group
        let mut used_ranges: Vec<(u64, u64)> = Vec::new();
        for (b, _) in idx.meta.range(*start..start + len) {
            used_ranges.push((*b, b + ns));
        }
        for (b, d) in idx.data.range(*start..start + len) {
            used_ranges.push((*b, b + d.len));
        }
        used_ranges.sort();
        // free ranges recorded in the free space tree
        let mut free: Vec<(u64, u64)> = Vec::new();
        let mut info_found = false;
        for (k, d) in fst {
            if k.objectid < *start || k.objectid >= start + len {
                continue;
            }
            match k.ty {
                FREE_SPACE_INFO_KEY if k.objectid == *start => info_found = true,
                FREE_SPACE_EXTENT_KEY => free.push((k.objectid, k.objectid + k.offset)),
                FREE_SPACE_BITMAP_KEY => {
                    let bits = d.len() as u64 * 8;
                    let mut run: Option<u64> = None;
                    for b in 0..bits {
                        let set = d[(b / 8) as usize] & (1 << (b % 8)) != 0;
                        match (set, run) {
                            (true, None) => run = Some(b),
                            (false, Some(r)) => {
                                free.push((k.objectid + r * ss, k.objectid + b * ss));
                                run = None;
                            }
                            _ => {}
                        }
                    }
                    if let Some(r) = run {
                        free.push((k.objectid + r * ss, k.objectid + bits * ss));
                    }
                }
                _ => {}
            }
        }
        if !info_found {
            c.err("fst-info-missing", FREE_SPACE_TREE_OBJECTID, *start, None, format!("block group {} has no FREE_SPACE_INFO", start));
            continue;
        }
        free.sort();
        // merge adjacent free ranges
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for r in free {
            match merged.last_mut() {
                Some(l) if l.1 == r.0 => l.1 = r.1,
                Some(l) if l.1 > r.0 => c.err("fst-overlap", FREE_SPACE_TREE_OBJECTID, *start, None, format!("free ranges overlap at {}", r.0)),
                _ => merged.push(r),
            }
        }
        let free_total: u64 = merged.iter().map(|r| r.1 - r.0).sum();
        if free_total + used != *len {
            c.err("fst-total", FREE_SPACE_TREE_OBJECTID, *start, None, format!("block group {}: used {} + free {} != length {}", start, used, free_total, len));
        }
        // no used range may intersect free space
        for u in &used_ranges {
            if merged.iter().any(|f| u.0 < f.1 && f.0 < u.1) {
                c.err("fst-used-overlap", FREE_SPACE_TREE_OBJECTID, u.0, None, format!("extent [{}, {}) lies inside recorded free space", u.0, u.1));
            }
        }
    }
}

fn check_csum_ranges<D: Dev>(vol: &mut Volume<D>, c: &mut Ctx, ranges: &BTreeMap<u64, u64>, fs: &FsAgg) {
    let ss = vol.sectorsize() as u64;
    // coverage: build merged csum coverage
    let mut covered: Vec<(u64, u64)> = Vec::new();
    for (s, l) in ranges {
        match covered.last_mut() {
            Some(last) if last.1 >= *s => last.1 = last.1.max(s + l),
            _ => covered.push((*s, s + l)),
        }
    }
    let covers = |s: u64, e: u64| -> bool { covered.iter().any(|(a, b)| *a <= s && e <= *b) };
    for (s, l, tree, ino) in &fs.need_csum {
        let start = s / ss * ss;
        let end = (s + l + ss - 1) / ss * ss;
        if !covers(start, end) {
            c.err("csum-missing", *tree, 0, Some(Key::new(*ino, EXTENT_DATA_KEY, 0)), format!("inode {} extent [{}, {}) lacks data checksums", ino, start, end));
        }
    }
    // orphan checksum ranges: not inside any referenced extent
    for (s, e) in &covered {
        let mut pos = *s;
        while pos < *e {
            let hit = fs.extent_ranges.range(..=pos).next_back().filter(|(_, end)| **end > pos);
            match hit {
                Some((_, end)) => pos = *end,
                None => {
                    let next = fs.extent_ranges.range(pos..).next().map(|(s2, _)| *s2).unwrap_or(*e);
                    c.err("csum-orphan", CSUM_TREE_OBJECTID, pos, None, format!("checksums for [{}, {}) belong to no extent", pos, next.min(*e)));
                    pos = next.min(*e).max(pos + ss);
                }
            }
        }
    }
}

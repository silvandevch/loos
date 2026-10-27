//! Extent-tree and block-group inventory used by the maintenance planners (trim, balance, defrag,
//! dedupe, qgroups, resize).
use crate::consts::*;
use crate::dev::Dev;
use crate::error::Result;
use crate::volume::*;
use alloc::vec::Vec;

/// One extent item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtentRecord {
    /// Logical start.
    pub start: u64,
    /// Length in bytes (nodesize for tree blocks).
    pub len: u64,
    /// Tree block (true) or data extent.
    pub meta: bool,
    /// Reference count.
    pub refs: u64,
    /// Extent flags (`1` data, `2` tree block, `0x80` full backref).
    pub flags: u64,
    /// Level of a tree block extent (skinny metadata).
    pub level: u8,
}

/// One block group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockGroup {
    /// Logical start.
    pub start: u64,
    /// Length.
    pub len: u64,
    /// Bytes used (recorded).
    pub used: u64,
    /// Type + profile flags.
    pub flags: u64,
}

/// Inventory of the extent tree.
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    /// Extents sorted by start.
    pub extents: Vec<ExtentRecord>,
    /// Block groups sorted by start.
    pub groups: Vec<BlockGroup>,
}

/// Reads every extent item and block group item.
pub fn inventory<D: Dev>(vol: &mut Volume<D>) -> Result<Inventory> {
    let ns = vol.nodesize() as u64;
    let mut inv = Inventory::default();
    let ext_root = vol.tree_root(EXTENT_TREE_OBJECTID)?;
    vol.walk_tree(ext_root, Key::min(0), Key::max(u64::MAX), &mut |k, d| {
        match k.ty {
            EXTENT_ITEM_KEY if d.len() >= 24 => {
                let flags = le64(d, 16);
                inv.extents.push(ExtentRecord { start: k.objectid, len: if flags & 1 != 0 { k.offset } else { ns }, meta: flags & 1 == 0, refs: le64(d, 0), flags, level: 0 });
            }
            METADATA_ITEM_KEY if d.len() >= 24 => {
                inv.extents.push(ExtentRecord { start: k.objectid, len: ns, meta: true, refs: le64(d, 0), flags: le64(d, 16), level: k.offset as u8 });
            }
            BLOCK_GROUP_ITEM_KEY if d.len() >= 24 => {
                inv.groups.push(BlockGroup { start: k.objectid, len: k.offset, used: le64(d, 0), flags: le64(d, 16) });
            }
            _ => {}
        }
        true
    })?;
    if vol.sb.compat_ro & COMPAT_RO_BLOCK_GROUP_TREE != 0 {
        let bg_root = vol.tree_root(BLOCK_GROUP_TREE_OBJECTID)?;
        vol.walk_tree(bg_root, Key::min(0), Key::max(u64::MAX), &mut |k, d| {
            if k.ty == BLOCK_GROUP_ITEM_KEY && d.len() >= 24 {
                inv.groups.push(BlockGroup { start: k.objectid, len: k.offset, used: le64(d, 0), flags: le64(d, 16) });
            }
            true
        })?;
    }
    inv.extents.sort_by_key(|e| e.start);
    inv.groups.sort_by_key(|g| g.start);
    Ok(inv)
}

impl Inventory {
    /// Extents inside `[start, start+len)`.
    pub fn in_range(&self, start: u64, len: u64) -> impl Iterator<Item = &ExtentRecord> {
        let lo = self.extents.partition_point(|e| e.start + e.len <= start);
        self.extents[lo..].iter().take_while(move |e| e.start < start + len)
    }

    /// Free logical ranges of a block group (complement of its extents).
    pub fn free_ranges(&self, g: &BlockGroup) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut pos = g.start;
        for e in self.in_range(g.start, g.len) {
            if e.start > pos {
                out.push((pos, e.start));
            }
            pos = pos.max(e.start + e.len);
        }
        if pos < g.start + g.len {
            out.push((pos, g.start + g.len));
        }
        out
    }
}

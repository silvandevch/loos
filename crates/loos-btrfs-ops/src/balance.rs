//! Balance planning: which block groups to relocate for `btrfs balance` filters and profile
//! conversion. Pure planning over the chunk map plus block-group usage read from the extent
//! (or block-group) tree; the relocation itself is performed by the write path.
use crate::consts::*;
use crate::dev::Dev;
use crate::error::Result;
use crate::raid::{ChunkMap, Profile};
use crate::volume::{Key, Volume};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// A block group's usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupUsage {
    /// Logical start.
    pub logical: u64,
    /// Length.
    pub length: u64,
    /// Used bytes.
    pub used: u64,
    /// Type + profile flags.
    pub flags: u64,
}

impl GroupUsage {
    /// Usage in percent (0-100).
    pub fn percent(&self) -> u64 {
        if self.length == 0 {
            0
        } else {
            (self.used as u128 * 100 / self.length as u128) as u64
        }
    }
}

/// Reads every block group's usage.
pub fn read_usage<D: Dev>(vol: &mut Volume<D>) -> Result<Vec<GroupUsage>> {
    let objid = if vol.sb.compat_ro & COMPAT_RO_BLOCK_GROUP_TREE != 0 { BLOCK_GROUP_TREE_OBJECTID } else { EXTENT_TREE_OBJECTID };
    let root = vol.tree_root(objid)?;
    let mut out = Vec::new();
    let flags_of: BTreeMap<u64, u64> = vol.chunks().iter().map(|c| (c.logical, c.flags)).collect();
    vol.walk_tree(root, Key::min(0), Key::max(u64::MAX), &mut |k, d| {
        if k.ty == BLOCK_GROUP_ITEM_KEY && d.len() >= 24 {
            let used = u64::from_le_bytes(d[0..8].try_into().unwrap());
            let flags = flags_of.get(&k.objectid).copied().unwrap_or_else(|| u64::from_le_bytes(d[16..24].try_into().unwrap()));
            out.push(GroupUsage { logical: k.objectid, length: k.offset, used, flags });
        }
        true
    })?;
    Ok(out)
}

/// Which block-group types a filter applies to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Only these type bits (`BLOCK_GROUP_DATA|METADATA|SYSTEM`); 0 = any.
    pub types: u64,
    /// Only groups with usage in `[usage_min, usage_max]` percent.
    pub usage_min: u64,
    /// Upper usage bound (inclusive); 100 = no bound.
    pub usage_max: u64,
    /// Only groups with a stripe on this device (0 = any).
    pub devid: u64,
    /// Only groups whose profile differs from `convert` (set by [`Plan::convert`]).
    pub profile_not: Option<Profile>,
}

impl Filter {
    /// Matches everything.
    pub fn all() -> Filter {
        Filter { usage_max: 100, ..Default::default() }
    }
    /// `-dusage=N` style: groups at most N% full.
    pub fn usage_at_most(types: u64, pct: u64) -> Filter {
        Filter { types, usage_max: pct, ..Default::default() }
    }

    fn matches(&self, g: &GroupUsage, c: Option<&ChunkMap>) -> bool {
        if self.types != 0 && g.flags & self.types & BLOCK_GROUP_TYPE_MASK == 0 {
            return false;
        }
        let p = g.percent();
        if p < self.usage_min || p > self.usage_max {
            return false;
        }
        if self.devid != 0 && !c.map_or(false, |c| c.stripes.iter().any(|s| s.devid == self.devid)) {
            return false;
        }
        if let Some(pn) = self.profile_not {
            if Profile::from_flags(g.flags) == Some(pn) {
                return false;
            }
        }
        true
    }
}

/// A relocation plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Block groups to relocate, emptiest first (cheapest and most space-freeing first).
    pub groups: Vec<GroupUsage>,
    /// Target profile for new chunks, if converting.
    pub convert: Option<Profile>,
    /// Bytes of live data that must be copied.
    pub bytes_to_move: u64,
}

/// Plans a balance: selects matching groups, emptiest first. With `convert`, groups already in
/// the target profile are skipped.
pub fn plan(vol: &Volume<impl Dev>, usage: &[GroupUsage], mut f: Filter, convert: Option<Profile>) -> Plan {
    if convert.is_some() {
        f.profile_not = convert;
    }
    let mut groups: Vec<GroupUsage> = usage.iter().copied().filter(|g| f.matches(g, vol.chunk_for(g.logical))).collect();
    groups.sort_by(|a, b| a.used.cmp(&b.used).then(a.logical.cmp(&b.logical)));
    let bytes_to_move = groups.iter().map(|g| g.used).sum();
    Plan { groups, convert, bytes_to_move }
}

/// Free-space check: relocating `p` needs room for its live data plus one fresh chunk's worth of
/// slack in the destination class; `free_unallocated` is raw unallocated device bytes.
pub fn feasible(p: &Plan, free_unallocated: u64, chunk_size: u64) -> bool {
    p.groups.is_empty() || free_unallocated >= chunk_size.max(p.groups.iter().map(|g| g.used).max().unwrap_or(0))
}

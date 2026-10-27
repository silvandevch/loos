//! Quota groups: reads the quota tree (`QGROUP_STATUS/INFO/LIMIT/RELATION` items) and evaluates
//! limits including inheritance through parent groups.
use crate::consts::*;
use crate::dev::Dev;
use crate::error::{Error, Result};
use crate::volume::{Key, Volume};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// `qgroupid >> 48`.
pub fn level(qgroupid: u64) -> u16 {
    (qgroupid >> 48) as u16
}
/// Lower 48 bits (subvolume id for level 0).
pub fn subvol_id(qgroupid: u64) -> u64 {
    qgroupid & ((1 << 48) - 1)
}
/// Builds a qgroup id.
pub fn make_id(level: u16, id: u64) -> u64 {
    ((level as u64) << 48) | (id & ((1 << 48) - 1))
}

/// Limit flag bits.
pub mod limit {
    /// `max_rfer` is set.
    pub const MAX_RFER: u64 = 1 << 0;
    /// `max_excl` is set.
    pub const MAX_EXCL: u64 = 1 << 1;
}

/// Status item flags.
pub mod status {
    /// Quotas enabled.
    pub const ON: u64 = 1 << 0;
    /// Rescan in progress.
    pub const RESCAN: u64 = 1 << 1;
    /// Counts are inconsistent and need a rescan.
    pub const INCONSISTENT: u64 = 1 << 2;
    /// Simple (extent-owner based) accounting.
    pub const SIMPLE_MODE: u64 = 1 << 3;
}

/// One quota group.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Qgroup {
    /// Referenced bytes.
    pub rfer: u64,
    /// Exclusive bytes.
    pub excl: u64,
    /// Limit flags ([`limit`]).
    pub limit_flags: u64,
    /// Referenced limit.
    pub max_rfer: u64,
    /// Exclusive limit.
    pub max_excl: u64,
    /// Parent groups.
    pub parents: Vec<u64>,
    /// Child groups.
    pub children: Vec<u64>,
}

/// The whole quota tree.
#[derive(Clone, Debug, Default)]
pub struct Quota {
    /// Status flags (0 if no status item).
    pub flags: u64,
    /// Accounting generation.
    pub generation: u64,
    /// Groups by id.
    pub groups: BTreeMap<u64, Qgroup>,
}

/// A violated limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Violation {
    /// Referenced bytes over `max_rfer`.
    Referenced {
        /// Group.
        qgroup: u64,
        /// Used.
        used: u64,
        /// Limit.
        limit: u64,
    },
    /// Exclusive bytes over `max_excl`.
    Exclusive {
        /// Group.
        qgroup: u64,
        /// Used.
        used: u64,
        /// Limit.
        limit: u64,
    },
}

fn le64(b: &[u8], o: usize) -> Result<u64> {
    b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap())).ok_or(Error::Corrupt("qgroup item size"))
}

impl Quota {
    /// Reads the quota tree; `Ok(None)` if quotas were never enabled.
    pub fn read<D: Dev>(vol: &mut Volume<D>) -> Result<Option<Quota>> {
        let root = match vol.tree_root(QUOTA_TREE_OBJECTID) {
            Ok(r) => r,
            Err(Error::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut items = Vec::new();
        vol.walk_tree(root, Key::min(0), Key::max(u64::MAX), &mut |k, d| {
            items.push((*k, d.to_vec()));
            true
        })?;
        Self::from_items(&items).map(Some)
    }

    /// Builds from raw `(key, data)` items (also used by tests).
    pub fn from_items(items: &[(Key, Vec<u8>)]) -> Result<Quota> {
        let mut q = Quota::default();
        for (k, d) in items {
            match k.ty {
                QGROUP_STATUS_KEY => {
                    q.flags = le64(d, 16)?;
                    q.generation = le64(d, 8)?;
                }
                QGROUP_INFO_KEY if k.objectid == 0 => {
                    let g = q.groups.entry(k.offset).or_default();
                    g.rfer = le64(d, 8)?;
                    g.excl = le64(d, 24)?;
                }
                QGROUP_LIMIT_KEY if k.objectid == 0 => {
                    let g = q.groups.entry(k.offset).or_default();
                    g.limit_flags = le64(d, 0)?;
                    g.max_rfer = le64(d, 8)?;
                    g.max_excl = le64(d, 16)?;
                }
                _ => {}
            }
        }
        // Relations are stored twice (child->parent and parent->child); keep the child->parent
        // direction (objectid is the lower level) and derive the other side.
        for (k, _) in items.iter().filter(|(k, _)| k.ty == QGROUP_RELATION_KEY) {
            let (a, b) = (k.objectid, k.offset);
            if level(a) < level(b) {
                q.groups.entry(a).or_default().parents.push(b);
                q.groups.entry(b).or_default().children.push(a);
            }
        }
        for g in q.groups.values_mut() {
            g.parents.sort_unstable();
            g.parents.dedup();
            g.children.sort_unstable();
            g.children.dedup();
        }
        Ok(q)
    }

    /// True if counts may be stale.
    pub fn inconsistent(&self) -> bool {
        self.flags & status::INCONSISTENT != 0
    }

    /// Every limit currently exceeded.
    pub fn violations(&self) -> Vec<Violation> {
        let mut v = Vec::new();
        for (&id, g) in &self.groups {
            if g.limit_flags & limit::MAX_RFER != 0 && g.rfer > g.max_rfer {
                v.push(Violation::Referenced { qgroup: id, used: g.rfer, limit: g.max_rfer });
            }
            if g.limit_flags & limit::MAX_EXCL != 0 && g.excl > g.max_excl {
                v.push(Violation::Exclusive { qgroup: id, used: g.excl, limit: g.max_excl });
            }
        }
        v
    }

    /// Would writing `extra` more exclusive bytes in `qgroup` break its own or an ancestor's
    /// limit? Returns the first group that would be exceeded. Cycles in relations are tolerated.
    pub fn would_exceed(&self, qgroup: u64, extra: u64) -> Option<Violation> {
        let mut seen = Vec::new();
        let mut stack = alloc::vec![qgroup];
        while let Some(id) = stack.pop() {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let Some(g) = self.groups.get(&id) else { continue };
            if g.limit_flags & limit::MAX_RFER != 0 && g.rfer.saturating_add(extra) > g.max_rfer {
                return Some(Violation::Referenced { qgroup: id, used: g.rfer.saturating_add(extra), limit: g.max_rfer });
            }
            if g.limit_flags & limit::MAX_EXCL != 0 && g.excl.saturating_add(extra) > g.max_excl {
                return Some(Violation::Exclusive { qgroup: id, used: g.excl.saturating_add(extra), limit: g.max_excl });
            }
            stack.extend(g.parents.iter().copied());
        }
        None
    }
}

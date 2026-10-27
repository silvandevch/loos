//! `fstrim` planning: which physical byte ranges of which device may be discarded, plus the zoned
//! variant (which whole zones can be reset).
//!
//! Discardable space is (a) unallocated device space and (b) free space inside block groups,
//! mapped to every physical copy. RAID5/6 block groups are not discarded inside (parity must stay
//! consistent), and superblock mirrors plus the first MiB are never touched.
use crate::consts::*;
use crate::dev::Dev;
use crate::error::Result;
use crate::extents::inventory;
use crate::raid::Profile;
use crate::volume::*;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// A physical range to discard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrimRange {
    /// Device id.
    pub devid: u64,
    /// Physical byte offset.
    pub physical: u64,
    /// Length.
    pub len: u64,
}

/// `FITRIM`-style options (offsets refer to device byte offsets).
#[derive(Clone, Copy, Debug)]
pub struct TrimOptions {
    /// Only ranges at or after this device offset.
    pub start: u64,
    /// Only ranges before `start + len`.
    pub len: u64,
    /// Skip ranges shorter than this.
    pub minlen: u64,
    /// Device discard granularity: ranges are shrunk inward to this alignment.
    pub granularity: u64,
}

impl Default for TrimOptions {
    fn default() -> Self {
        TrimOptions { start: 0, len: u64::MAX, minlen: 0, granularity: 1 }
    }
}

/// Result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrimPlan {
    /// Sorted, merged, non-overlapping ranges.
    pub ranges: Vec<TrimRange>,
    /// Sum of range lengths.
    pub bytes: u64,
}

/// First MiB plus each superblock mirror slot (rounded to whole sectors) are off limits.
pub fn reserved_ranges(dev_len: u64) -> Vec<(u64, u64)> {
    let mut v = alloc::vec![(0u64, 1 << 20)];
    for &off in SUPER_MIRROR_OFFSETS.iter().skip(1) {
        if off + SUPER_INFO_SIZE as u64 <= dev_len {
            v.push((off, off + 64 * 1024));
        }
    }
    v
}

fn subtract(ranges: Vec<(u64, u64)>, holes: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut holes: Vec<(u64, u64)> = holes.to_vec();
    holes.sort();
    let mut out = Vec::new();
    for (mut s, e) in ranges {
        for h in holes.iter() {
            if h.1 <= s || h.0 >= e {
                continue;
            }
            if h.0 > s {
                out.push((s, h.0));
            }
            s = s.max(h.1);
            if s >= e {
                break;
            }
        }
        if s < e {
            out.push((s, e));
        }
    }
    out
}

/// Plans a trim of the whole filesystem.
pub fn plan_trim<D: Dev>(vol: &mut Volume<D>, opts: &TrimOptions) -> Result<TrimPlan> {
    let inv = inventory(vol)?;
    let chunks = vol.chunks().to_vec();
    let mut per_dev: BTreeMap<u64, Vec<(u64, u64)>> = BTreeMap::new();

    // (a) unallocated device space = device minus every dev extent
    for id in vol.devids() {
        let item = vol.dev_items.get(&id).copied();
        let size = item.map_or_else(|| vol.dev_mut(id).map_or(0, |d| d.len()), |i| i.total_bytes);
        let mut used: Vec<(u64, u64)> = Vec::new();
        for ch in &chunks {
            let dl = ch.stripe_dev_len();
            for s in ch.stripes.iter().filter(|s| s.devid == id) {
                used.push((s.physical, s.physical + dl));
            }
        }
        let mut reserved = reserved_ranges(size);
        used.append(&mut reserved);
        per_dev.entry(id).or_default().extend(subtract(alloc::vec![(0, size)], &used));
    }
    // (b) free space inside block groups
    for g in &inv.groups {
        let ch = match chunks.iter().find(|c| c.logical == g.start) {
            Some(c) => c,
            None => continue,
        };
        if matches!(ch.profile(), Some(Profile::Raid5) | Some(Profile::Raid6)) {
            continue;
        }
        for (s, e) in inv.free_ranges(g) {
            let mut pos = s;
            while pos < e {
                let m = match ch.map(pos) {
                    Ok(m) => m,
                    Err(_) => break,
                };
                let n = m.len.min(e - pos);
                for c in &m.copies {
                    per_dev.entry(c.devid).or_default().push((c.physical, c.physical + n));
                }
                pos += n;
            }
        }
    }
    let mut plan = TrimPlan::default();
    let g = opts.granularity.max(1);
    for (devid, mut ranges) in per_dev {
        // superblock mirrors may sit inside allocated chunks (excluded from extent allocation): never trim them
        let size = vol.dev_items.get(&devid).map_or(u64::MAX, |i| i.total_bytes);
        let ranges = subtract(ranges, &reserved_ranges(size));
        let mut ranges = ranges;
        ranges.sort();
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for r in ranges {
            match merged.last_mut() {
                Some(l) if l.1 >= r.0 => l.1 = l.1.max(r.1),
                _ => merged.push(r),
            }
        }
        for (mut s, mut e) in merged {
            // user range
            let hi = opts.start.saturating_add(opts.len);
            s = s.max(opts.start);
            e = e.min(hi);
            if s >= e {
                continue;
            }
            // align inward
            s = (s + g - 1) / g * g;
            e = e / g * g;
            if s >= e || e - s < opts.minlen.max(1) {
                continue;
            }
            plan.bytes += e - s;
            plan.ranges.push(TrimRange { devid, physical: s, len: e - s });
        }
    }
    Ok(plan)
}

// ---------------------------------------------------------------------------------------------
// zoned devices

/// State of one zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoneState {
    /// First byte of the zone.
    pub start: u64,
    /// Write pointer (absolute byte offset; `start` = empty, `start + size` = full).
    pub write_pointer: u64,
}

/// Zones that hold no live data and are not already empty: they can be reset instead of discarded.
/// `live` are the physical ranges still holding data or metadata.
pub fn zones_to_reset(zone_size: u64, zones: &[ZoneState], live: &[(u64, u64)]) -> Vec<u64> {
    zones
        .iter()
        .filter(|z| z.write_pointer > z.start)
        .filter(|z| !live.iter().any(|&(s, e)| s < z.start + zone_size && z.start < e))
        .map(|z| z.start)
        .collect()
}

/// Validates that a chunk's stripes fit a zoned device: start and size must be zone aligned.
pub fn zoned_chunk_ok(zone_size: u64, physical: u64, stripe_len: u64) -> bool {
    zone_size.is_power_of_two() && physical % zone_size == 0 && stripe_len % zone_size == 0 && stripe_len > 0
}

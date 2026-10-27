//! Rescue operations: superblock recovery, log clearing, device-size repair and tree-root
//! discovery when the root tree is unreadable (`btrfs rescue` / `btrfs restore -l/-t`).
use crate::consts::*;
use crate::dev::Dev;
use crate::error::{Error, Result};
use crate::volume::*;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use loos_btrfs::{digest, ChecksumAlgorithm};

fn reseal(raw: &mut [u8], alg: ChecksumAlgorithm) {
    let d = digest(alg, &raw[32..]);
    raw[..32].fill(0);
    raw[..d.as_bytes().len()].copy_from_slice(d.as_bytes());
}

/// What `super_recover` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SuperRecovery {
    /// `(devid, offset)` of every superblock slot rewritten.
    pub rewritten: Vec<(u64, u64)>,
    /// Generation of the superblock everything was restored from.
    pub from_generation: u64,
}

/// Restores damaged superblock copies from the valid copy with the highest generation (per device the
/// device's own `dev_item` is preserved). Works with any checksum type. Writes to the devices.
pub fn super_recover<D: Dev>(devs: &mut [D]) -> Result<SuperRecovery> {
    // find the best valid superblock across all devices
    let mut best: Option<(u64, Vec<u8>)> = None; // generation, raw
    for d in devs.iter_mut() {
        for &off in SUPER_MIRROR_OFFSETS.iter() {
            if off + SUPER_INFO_SIZE as u64 > d.len() {
                continue;
            }
            let mut raw = vec![0u8; SUPER_INFO_SIZE];
            if d.read_at(off, &mut raw).is_ok() {
                if let Ok(s) = Super::parse(&raw, off) {
                    if best.as_ref().map_or(true, |b| s.generation > b.0) {
                        best = Some((s.generation, raw));
                    }
                }
            }
        }
    }
    let (generation, good) = best.ok_or(Error::NotBtrfs)?;
    let alg = ChecksumAlgorithm::from_btrfs_type(le16(&good, 196)).ok_or(Error::Unsupported("checksum type"))?;
    let mut out = SuperRecovery { rewritten: Vec::new(), from_generation: generation };
    for d in devs.iter_mut() {
        // this device's own dev_item: take it from any valid copy on this device if one exists
        let mut own: Option<Vec<u8>> = None;
        for &off in SUPER_MIRROR_OFFSETS.iter() {
            if off + SUPER_INFO_SIZE as u64 > d.len() {
                continue;
            }
            let mut raw = vec![0u8; SUPER_INFO_SIZE];
            if d.read_at(off, &mut raw).is_ok() && Super::parse(&raw, off).is_ok() {
                own = Some(raw[201..299].to_vec());
                break;
            }
        }
        for &off in SUPER_MIRROR_OFFSETS.iter() {
            if off + SUPER_INFO_SIZE as u64 > d.len() {
                continue;
            }
            let mut cur = vec![0u8; SUPER_INFO_SIZE];
            let _ = d.read_at(off, &mut cur);
            let valid_current = Super::parse(&cur, off).map_or(false, |s| s.generation == generation);
            if valid_current {
                continue;
            }
            let mut img = good.clone();
            if let Some(di) = &own {
                img[201..299].copy_from_slice(di);
            }
            put64(&mut img, 48, off);
            reseal(&mut img, alg);
            d.write_at(off, &img)?;
            out.rewritten.push((le64(&img, 201), off));
        }
        d.flush()?;
    }
    Ok(out)
}

/// Clears the log-tree pointer in every superblock copy (`btrfs rescue zero-log`).
pub fn zero_log<D: Dev>(devs: &mut [D]) -> Result<u32> {
    let mut n = 0;
    for d in devs.iter_mut() {
        for &off in SUPER_MIRROR_OFFSETS.iter() {
            if off + SUPER_INFO_SIZE as u64 > d.len() {
                continue;
            }
            let mut raw = vec![0u8; SUPER_INFO_SIZE];
            if d.read_at(off, &mut raw).is_err() {
                continue;
            }
            let s = match Super::parse(&raw, off) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let alg = ChecksumAlgorithm::from_btrfs_type(s.csum_type).unwrap();
            put64(&mut raw, 96, 0); // log_root
            put64(&mut raw, 104, 0); // log_root_transid
            raw[200] = 0; // log_root_level
            reseal(&mut raw, alg);
            d.write_at(off, &raw)?;
            n += 1;
        }
        d.flush()?;
    }
    Ok(n)
}

/// Makes the recorded device sizes match the real device sizes (`btrfs rescue fix-device-size`).
/// Only the superblocks are patched (the dev item in the chunk tree is a separate, later fix).
pub fn fix_device_size<D: Dev>(devs: &mut [D]) -> Result<u32> {
    let mut n = 0;
    for d in devs.iter_mut() {
        let real = d.len() / 4096 * 4096;
        for &off in SUPER_MIRROR_OFFSETS.iter() {
            if off + SUPER_INFO_SIZE as u64 > d.len() {
                continue;
            }
            let mut raw = vec![0u8; SUPER_INFO_SIZE];
            if d.read_at(off, &mut raw).is_err() {
                continue;
            }
            let s = match Super::parse(&raw, off) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if s.dev_item.total_bytes == real {
                continue;
            }
            let alg = ChecksumAlgorithm::from_btrfs_type(s.csum_type).unwrap();
            let old = s.dev_item.total_bytes;
            put64(&mut raw, 201 + 8, real);
            // total_bytes of the filesystem moves by the same delta on single-device filesystems
            if s.num_devices == 1 {
                put64(&mut raw, 112, real);
            } else {
                put64(&mut raw, 112, s.total_bytes.wrapping_sub(old).wrapping_add(real));
            }
            reseal(&mut raw, alg);
            d.write_at(off, &raw)?;
            n += 1;
        }
        d.flush()?;
    }
    Ok(n)
}

/// A tree root candidate found by scanning metadata chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootCandidate {
    /// Logical address.
    pub bytenr: u64,
    /// Tree level.
    pub level: u8,
    /// Generation.
    pub generation: u64,
}

/// Scans every metadata/system chunk for valid tree blocks and returns, per owner tree, the block with
/// the highest level (then highest generation) — the root when the root tree cannot be read.
pub fn find_tree_roots<D: Dev>(vol: &mut Volume<D>) -> Result<BTreeMap<u64, RootCandidate>> {
    let ns = vol.nodesize() as u64;
    let chunks: Vec<_> = vol.chunks().iter().filter(|c| c.flags & (BLOCK_GROUP_METADATA | BLOCK_GROUP_SYSTEM) != 0).cloned().collect();
    let mut best: BTreeMap<u64, RootCandidate> = BTreeMap::new();
    for ch in chunks {
        let mut pos = ch.logical;
        let end = ch.logical + ch.length;
        while pos + ns <= end {
            if let Ok((raw, st)) = vol.read_tree_block_raw(pos, None) {
                if st.csum_ok && st.fsid_ok && st.bytenr_ok {
                    let owner = le64(&raw, 88);
                    let cand = RootCandidate { bytenr: pos, level: raw[100], generation: le64(&raw, 80) };
                    let better = best.get(&owner).map_or(true, |b| (cand.level, cand.generation) > (b.level, b.generation));
                    if better {
                        best.insert(owner, cand);
                    }
                }
            }
            pos += ns;
        }
    }
    Ok(best)
}

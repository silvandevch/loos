//! Scrub: read **every copy** of every metadata block and data sector, verify checksums, and (with
//! `repair`) overwrite bad copies from good ones; verify and rebuild RAID5/6 parity; verify and
//! restore superblock mirrors.
use crate::consts::*;
use crate::dev::Dev;
use crate::error::Result;
use crate::ioctl::ScrubProgress;
use crate::raid::{self, Copy as RaidCopy, Profile};
use crate::volume::*;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use loos_btrfs::verify as verify_digest;

/// Scrub options.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScrubOptions {
    /// Rewrite bad copies/parity (otherwise read-only, like `BTRFS_SCRUB_READONLY`).
    pub repair: bool,
}

/// Counters, same meaning as `btrfs_scrub_progress`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrubStats {
    /// Data extents examined.
    pub data_extents_scrubbed: u64,
    /// Tree blocks examined.
    pub tree_extents_scrubbed: u64,
    /// Data bytes examined (per extent, not per copy).
    pub data_bytes_scrubbed: u64,
    /// Tree bytes examined.
    pub tree_bytes_scrubbed: u64,
    /// Reads that failed outright.
    pub read_errors: u64,
    /// Data checksum mismatches (per bad copy and sector).
    pub csum_errors: u64,
    /// Tree blocks whose header/checksum did not verify (per bad copy).
    pub verify_errors: u64,
    /// Sectors without a checksum (nodatasum).
    pub no_csum: u64,
    /// Checksums with no data.
    pub csum_discards: u64,
    /// Bad superblocks.
    pub super_errors: u64,
    /// Errors with no intact copy or failed write-back.
    pub uncorrectable_errors: u64,
    /// Errors repaired from another copy/parity.
    pub corrected_errors: u64,
    /// RAID5/6 parity stripes that were wrong.
    pub parity_errors: u64,
}

impl ScrubStats {
    /// In `btrfs_scrub_progress` field order.
    pub fn to_progress(&self) -> ScrubProgress {
        ScrubProgress {
            counters: [
                self.data_extents_scrubbed,
                self.tree_extents_scrubbed,
                self.data_bytes_scrubbed,
                self.tree_bytes_scrubbed,
                self.read_errors,
                self.csum_errors,
                self.verify_errors,
                self.no_csum,
                self.csum_discards,
                self.super_errors,
                0,
                self.uncorrectable_errors,
                self.corrected_errors,
                0,
                0,
            ],
        }
    }
}

/// One damaged copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadCopy {
    /// Logical address (0 for superblocks).
    pub logical: u64,
    /// Device holding the bad copy.
    pub devid: u64,
    /// Physical offset of the bad copy.
    pub physical: u64,
    /// Metadata (true) or data/parity/super (false).
    pub metadata: bool,
    /// Whether it was rewritten successfully.
    pub corrected: bool,
}

/// Scrub outcome.
#[derive(Clone, Debug, Default)]
pub struct ScrubReport {
    /// Counters.
    pub stats: ScrubStats,
    /// Every bad copy found.
    pub bad: Vec<BadCopy>,
}

fn csums_for<D: Dev>(vol: &mut Volume<D>, root: Option<TreeRef>, logical: u64, len: u64) -> BTreeMap<u64, Vec<u8>> {
    let mut map = BTreeMap::new();
    let root = match root {
        Some(r) => r,
        None => return map,
    };
    let cs = vol.csum_size() as u64;
    let ss = vol.sectorsize() as u64;
    let lo = match vol.tree_floor(root, Key::new(EXTENT_CSUM_OBJECTID, EXTENT_CSUM_KEY, logical)) {
        Ok(Some(it)) if it.key.objectid == EXTENT_CSUM_OBJECTID && it.key.ty == EXTENT_CSUM_KEY => it.key.offset,
        _ => logical,
    };
    if let Ok(items) = vol.tree_items(root, Key::new(EXTENT_CSUM_OBJECTID, EXTENT_CSUM_KEY, lo), Key::new(EXTENT_CSUM_OBJECTID, EXTENT_CSUM_KEY, logical + len)) {
        for it in items {
            for i in 0..(it.data.len() as u64 / cs) {
                let sector = it.key.offset + i * ss;
                if sector + ss > logical && sector < logical + len {
                    map.insert(sector, it.data[(i * cs) as usize..((i + 1) * cs) as usize].to_vec());
                }
            }
        }
    }
    map
}

/// Scrubs the whole volume.
pub fn scrub<D: Dev>(vol: &mut Volume<D>, opts: &ScrubOptions) -> Result<ScrubReport> {
    let mut rep = ScrubReport::default();
    scrub_supers(vol, opts, &mut rep);

    // gather extents from the extent tree
    let ext_root = vol.tree_root(EXTENT_TREE_OBJECTID)?;
    let csum_root = vol.tree_root(CSUM_TREE_OBJECTID).ok();
    let ns = vol.nodesize() as u64;
    let mut meta: Vec<u64> = Vec::new();
    let mut data: Vec<(u64, u64)> = Vec::new();
    vol.walk_tree(ext_root, Key::min(0), Key::max(u64::MAX), &mut |k, d| {
        match k.ty {
            METADATA_ITEM_KEY => meta.push(k.objectid),
            EXTENT_ITEM_KEY if d.len() >= 24 => {
                if le64(d, 16) & 1 != 0 {
                    data.push((k.objectid, k.offset));
                } else {
                    meta.push(k.objectid);
                }
            }
            _ => {}
        }
        true
    })?;

    for b in meta {
        scrub_tree_block(vol, b, ns, opts, &mut rep);
    }
    for (b, len) in data {
        scrub_data_extent(vol, csum_root, b, len, opts, &mut rep);
    }
    // parity of RAID5/6 chunks
    let chunks: Vec<raid::ChunkMap> = vol.chunks().iter().filter(|c| matches!(c.profile(), Some(Profile::Raid5) | Some(Profile::Raid6))).cloned().collect();
    for ch in chunks {
        scrub_parity(vol, &ch, opts, &mut rep);
    }
    Ok(rep)
}

fn scrub_supers<D: Dev>(vol: &mut Volume<D>, opts: &ScrubOptions, rep: &mut ScrubReport) {
    let good_raw: Vec<u8> = {
        // rebuild the authoritative superblock image from the first valid copy we can find
        let mut found: Option<Vec<u8>> = None;
        for id in vol.devids() {
            let len = vol.dev_mut(id).map_or(0, |d| d.len());
            for off in Volume::<D>::super_offsets(len) {
                let mut raw = vec![0u8; SUPER_INFO_SIZE];
                if let Some(d) = vol.dev_mut(id) {
                    if d.read_at(off, &mut raw).is_ok() {
                        if let Ok(s) = Super::parse(&raw, off) {
                            if s.generation == vol.sb.generation && s.dev_item.devid == id {
                                found = Some(raw);
                                break;
                            }
                        }
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }
        match found {
            Some(f) => f,
            None => return,
        }
    };
    for id in vol.devids() {
        let len = vol.dev_mut(id).map_or(0, |d| d.len());
        for off in Volume::<D>::super_offsets(len) {
            let mut raw = vec![0u8; SUPER_INFO_SIZE];
            let ok = match vol.dev_mut(id) {
                Some(d) => d.read_at(off, &mut raw).is_ok() && Super::parse(&raw, off).is_ok(),
                None => false,
            };
            // an all-zero mirror slot has simply never been written (small devices)
            if !ok && raw.iter().any(|&b| b != 0) {
                rep.stats.super_errors += 1;
                let mut bad = BadCopy { logical: 0, devid: id, physical: off, metadata: false, corrected: false };
                if opts.repair {
                    // only the device's own dev_item differs between devices; rewrite this device's primary image
                    let mut img = good_raw.clone();
                    // patch bytenr and checksum for this slot
                    put64(&mut img, 48, off);
                    let alg = vol.alg();
                    let d = loos_btrfs::digest(alg, &img[32..]);
                    img[..32].fill(0);
                    img[..d.as_bytes().len()].copy_from_slice(d.as_bytes());
                    if let Some(dv) = vol.dev_mut(id) {
                        if dv.write_at(off, &img).is_ok() {
                            bad.corrected = true;
                            rep.stats.corrected_errors += 1;
                        }
                    }
                }
                if opts.repair && !bad.corrected {
                    rep.stats.uncorrectable_errors += 1;
                }
                rep.bad.push(bad);
            }
        }
    }
}

fn scrub_tree_block<D: Dev>(vol: &mut Volume<D>, bytenr: u64, ns: u64, opts: &ScrubOptions, rep: &mut ScrubReport) {
    rep.stats.tree_extents_scrubbed += 1;
    rep.stats.tree_bytes_scrubbed += ns;
    let chunk = match vol.chunk_for(bytenr) {
        Some(c) => c.clone(),
        None => {
            rep.stats.verify_errors += 1;
            rep.stats.uncorrectable_errors += 1;
            return;
        }
    };
    let m = match chunk.map(bytenr) {
        Ok(m) => m,
        Err(_) => return,
    };
    let copies = m.copies.len();
    let mut images: Vec<Option<Vec<u8>>> = Vec::new();
    let mut good_idx: Option<usize> = None;
    for mirror in 0..copies {
        let mut raw = vec![0u8; ns as usize];
        match vol.read_mapped(&m, &mut raw, mirror) {
            Ok(()) => {
                let st = vol.block_status(&raw, bytenr, None);
                if st.csum_ok && st.fsid_ok && st.bytenr_ok && st.generation_ok {
                    if good_idx.is_none() {
                        good_idx = Some(mirror);
                    }
                    images.push(Some(raw));
                } else {
                    rep.stats.verify_errors += 1;
                    images.push(None);
                    rep.bad.push(BadCopy { logical: bytenr, devid: m.copies[mirror].devid, physical: m.copies[mirror].physical, metadata: true, corrected: false });
                }
            }
            Err(_) => {
                rep.stats.read_errors += 1;
                images.push(None);
                rep.bad.push(BadCopy { logical: bytenr, devid: m.copies[mirror].devid, physical: m.copies[mirror].physical, metadata: true, corrected: false });
            }
        }
    }
    repair_copies(vol, &m.copies, &images, good_idx, bytenr, true, opts, rep);
    if m.parity.is_some() && good_idx.is_none() {
        // RAID5/6: try the parity rebuild path
        rep.stats.uncorrectable_errors += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn repair_copies<D: Dev>(vol: &mut Volume<D>, copies: &[RaidCopy], images: &[Option<Vec<u8>>], good_idx: Option<usize>, logical: u64, metadata: bool, opts: &ScrubOptions, rep: &mut ScrubReport) {
    let bad_now: Vec<usize> = (0..images.len()).filter(|&i| images[i].is_none()).collect();
    if bad_now.is_empty() {
        return;
    }
    match good_idx {
        None => {
            rep.stats.uncorrectable_errors += bad_now.len() as u64;
        }
        Some(g) => {
            for i in bad_now {
                let mut ok = false;
                if opts.repair {
                    if let Some(img) = images[g].as_ref() {
                        ok = vol.write_copy(copies[i], img).is_ok();
                    }
                }
                if ok {
                    rep.stats.corrected_errors += 1;
                    // mark the matching record corrected
                    if let Some(b) = rep.bad.iter_mut().rev().find(|b| b.logical == logical && b.devid == copies[i].devid && b.physical == copies[i].physical && b.metadata == metadata) {
                        b.corrected = true;
                    }
                } else if opts.repair {
                    rep.stats.uncorrectable_errors += 1;
                }
            }
        }
    }
}

fn scrub_data_extent<D: Dev>(vol: &mut Volume<D>, csum_root: Option<TreeRef>, logical: u64, len: u64, opts: &ScrubOptions, rep: &mut ScrubReport) {
    rep.stats.data_extents_scrubbed += 1;
    rep.stats.data_bytes_scrubbed += len;
    let ss = vol.sectorsize() as u64;
    let alg = vol.alg();
    let csums = csums_for(vol, csum_root, logical, len);
    if csums.is_empty() {
        rep.stats.no_csum += len / ss;
        return;
    }
    let mut pos = 0u64;
    while pos < len {
        let lg = logical + pos;
        let chunk = match vol.chunk_for(lg) {
            Some(c) => c.clone(),
            None => return,
        };
        let m = match chunk.map(lg) {
            Ok(m) => m,
            Err(_) => return,
        };
        let seg_len = core::cmp::min(m.len, len - pos);
        let copies = m.copies.len();
        let mut images: Vec<Option<Vec<u8>>> = Vec::new();
        let mut good_idx = None;
        for mirror in 0..copies {
            let mut buf = vec![0u8; seg_len as usize];
            match vol.read_mapped(&m, &mut buf, mirror) {
                Ok(()) => {
                    let mut bad_sectors = 0u64;
                    for (i, sec) in buf.chunks(ss as usize).enumerate() {
                        if let Some(c) = csums.get(&(lg + i as u64 * ss)) {
                            if sec.len() as u64 == ss && !verify_digest(alg, sec, c) {
                                bad_sectors += 1;
                            }
                        }
                    }
                    if bad_sectors == 0 {
                        if good_idx.is_none() {
                            good_idx = Some(mirror);
                        }
                        images.push(Some(buf));
                    } else {
                        rep.stats.csum_errors += bad_sectors;
                        images.push(None);
                        rep.bad.push(BadCopy { logical: lg, devid: m.copies[mirror].devid, physical: m.copies[mirror].physical, metadata: false, corrected: false });
                    }
                }
                Err(_) => {
                    rep.stats.read_errors += 1;
                    images.push(None);
                    rep.bad.push(BadCopy { logical: lg, devid: m.copies[mirror].devid, physical: m.copies[mirror].physical, metadata: false, corrected: false });
                }
            }
        }
        if m.parity.is_some() {
            // single data copy: on a bad sector try rebuilding from parity
            if images[0].is_none() {
                let mut buf = vec![0u8; seg_len as usize];
                let rebuilt = vol.read_mapped(&m, &mut buf, 1).is_ok() && buf.chunks(ss as usize).enumerate().all(|(i, s)| csums.get(&(lg + i as u64 * ss)).map_or(true, |c| s.len() as u64 != ss || verify_digest(alg, s, c)));
                if rebuilt {
                    if opts.repair && vol.write_copy(m.copies[0], &buf).is_ok() {
                        rep.stats.corrected_errors += 1;
                        if let Some(b) = rep.bad.iter_mut().rev().find(|b| b.logical == lg && !b.metadata) {
                            b.corrected = true;
                        }
                    } else if opts.repair {
                        rep.stats.uncorrectable_errors += 1;
                    }
                } else {
                    rep.stats.uncorrectable_errors += 1;
                }
            }
        } else {
            repair_copies(vol, &m.copies, &images, good_idx, lg, false, opts, rep);
        }
        pos += seg_len;
    }
}

fn scrub_parity<D: Dev>(vol: &mut Volume<D>, ch: &raid::ChunkMap, opts: &ScrubOptions, rep: &mut ScrubReport) {
    let nr_data = ch.data_stripes();
    let np = ch.profile().map_or(0, |p| p.parity());
    let sl = ch.stripe_len;
    let full_len = nr_data as u64 * sl;
    let full_stripes = ch.length / full_len;
    for f in 0..full_stripes {
        let base = ch.logical + f * full_len;
        let m = match ch.map(base) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let p = m.parity.unwrap();
        let mut stripes: Vec<Option<Vec<u8>>> = Vec::new();
        let mut unreadable = 0;
        for s in &p.stripes {
            let mut v = vec![0u8; sl as usize];
            match vol.dev_mut(s.devid).map(|d| d.read_at(s.physical, &mut v)) {
                Some(Ok(())) => stripes.push(Some(v)),
                _ => {
                    stripes.push(None);
                    unreadable += 1;
                }
            }
        }
        if unreadable > 0 {
            rep.stats.read_errors += unreadable;
            continue;
        }
        let data: Vec<&[u8]> = (0..nr_data).map(|i| stripes[i].as_deref().unwrap()).collect();
        let mut pp = vec![0u8; sl as usize];
        let mut qq = vec![0u8; sl as usize];
        raid::compute_parity(&data, &mut pp, if np == 2 { Some(&mut qq) } else { None });
        let p_ok = stripes[nr_data].as_deref() == Some(&pp[..]);
        let q_ok = np < 2 || stripes[nr_data + 1].as_deref() == Some(&qq[..]);
        if p_ok && q_ok {
            continue;
        }
        rep.stats.parity_errors += 1;
        // data stripes are trusted (checksums were verified per extent): rewrite the parity
        let mut fixed = true;
        if opts.repair {
            if !p_ok {
                fixed &= vol.write_copy(p.stripes[nr_data], &pp).is_ok();
            }
            if !q_ok {
                fixed &= vol.write_copy(p.stripes[nr_data + 1], &qq).is_ok();
            }
            if fixed {
                rep.stats.corrected_errors += 1;
            } else {
                rep.stats.uncorrectable_errors += 1;
            }
        }
        rep.bad.push(BadCopy { logical: base, devid: p.stripes[nr_data].devid, physical: p.stripes[nr_data].physical, metadata: false, corrected: opts.repair && fixed });
    }
}

/// Verifies all data checksums without repair; returns the logical sector addresses that fail on
/// **every** copy (what `btrfs check --check-data-csum` reports).
pub fn scrub_data_only<D: Dev>(vol: &mut Volume<D>) -> Vec<u64> {
    let mut out = Vec::new();
    let ext_root = match vol.tree_root(EXTENT_TREE_OBJECTID) {
        Ok(r) => r,
        Err(_) => return out,
    };
    let csum_root = vol.tree_root(CSUM_TREE_OBJECTID).ok();
    let mut data: Vec<(u64, u64)> = Vec::new();
    let _ = vol.walk_tree(ext_root, Key::min(0), Key::max(u64::MAX), &mut |k, d| {
        if k.ty == EXTENT_ITEM_KEY && d.len() >= 24 && le64(d, 16) & 1 != 0 {
            data.push((k.objectid, k.offset));
        }
        true
    });
    let ss = vol.sectorsize() as u64;
    let alg = vol.alg();
    for (b, len) in data {
        let csums = csums_for(vol, csum_root, b, len);
        let mut pos = 0;
        while pos < len {
            let lg = b + pos;
            let n = core::cmp::min(ss, len - pos) as usize;
            if let Some(c) = csums.get(&lg) {
                let mut buf = vec![0u8; n];
                let mut verified = false;
                let mut v = |_: u64, s: &[u8]| s.len() == ss as usize && verify_digest(alg, s, c);
                if vol.read_logical_verified(lg, &mut buf, &mut v).is_ok() {
                    verified = true;
                }
                if !verified && n == ss as usize {
                    out.push(lg);
                }
            }
            pos += ss;
        }
    }
    out
}

/// Parity-only scrub of every RAID5/6 chunk (used by tools and tests on volumes without trees).
pub fn scrub_parity_only<D: Dev>(vol: &mut Volume<D>, opts: &ScrubOptions) -> ScrubReport {
    let mut rep = ScrubReport::default();
    let chunks: Vec<raid::ChunkMap> = vol.chunks().iter().filter(|c| matches!(c.profile(), Some(Profile::Raid5) | Some(Profile::Raid6))).cloned().collect();
    for ch in chunks {
        scrub_parity(vol, &ch, opts, &mut rep);
    }
    rep
}

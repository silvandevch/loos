mod common;
use common::*;
use loos_btrfs_ops::check::{check_devices, Options};
use loos_btrfs_ops::consts::*;
use loos_btrfs_ops::dev::{FileDev, MemDev};
use loos_btrfs_ops::raid::{self, ChunkMap, Stripe};
use loos_btrfs_ops::scrub::*;
use loos_btrfs_ops::volume::*;
use std::collections::BTreeMap;
use std::io::{Seek, SeekFrom, Write};

fn corrupt(img: &std::path::Path, physical: u64, n: usize) {
    let mut f = std::fs::OpenOptions::new().write(true).open(img).unwrap();
    f.seek(SeekFrom::Start(physical)).unwrap();
    f.write_all(&vec![0xa5; n]).unwrap();
}

#[test]
fn dup_scrub_detects_then_repairs_metadata_data_and_superblock_mirror() {
    if !have("mkfs.btrfs") || !have("btrfs") {
        return;
    }
    let d = scratch("scrub-dup");
    let t = d.join("t");
    std::fs::create_dir_all(&t).unwrap();
    std::fs::write(t.join("f.bin"), payload(5, 1_500_000)).unwrap();
    for i in 0..50 {
        std::fs::write(t.join(format!("s{}", i)), payload(i, 3000)).unwrap();
    }
    let img = d.join("s.img");
    mkfs_btrfs(&img, 160, Some(&t), &["-d", "dup", "-m", "dup", "--nodiscard"]);
    let pristine = std::fs::read(&img).unwrap();

    // baseline: nothing wrong
    let mut vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let rep = scrub(&mut vol, &ScrubOptions { repair: false }).unwrap();
    assert_eq!(rep.stats.csum_errors + rep.stats.verify_errors + rep.stats.read_errors + rep.stats.super_errors, 0, "{:?}", rep);
    assert!(rep.stats.tree_extents_scrubbed > 3 && rep.stats.data_extents_scrubbed >= 2, "{:?}", rep.stats);
    drop(vol);

    // damage: one copy of the root tree block, the *other* copy of a data extent, and a superblock mirror
    let (meta_phys, data_phys, ext_len) = {
        let mut vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
        let root = vol.sb.root.bytenr;
        let m = vol.chunk_for(root).unwrap().map(root).unwrap();
        let ext_root = vol.tree_root(EXTENT_TREE_OBJECTID).unwrap();
        let mut data_ext = None;
        vol.walk_tree(ext_root, Key::min(0), Key::max(u64::MAX), &mut |k, dd| {
            if k.ty == EXTENT_ITEM_KEY && dd[16] & 1 == 1 && data_ext.map_or(true, |(_, l): (u64, u64)| k.offset > l) {
                data_ext = Some((k.objectid, k.offset));
            }
            true
        })
        .unwrap();
        let (dl, len) = data_ext.unwrap();
        let dm = vol.chunk_for(dl).unwrap().map(dl).unwrap();
        (m.copies[0].physical, dm.copies[1].physical + 4096 * 3, len)
    };
    corrupt(&img, meta_phys + 200, 32);
    corrupt(&img, data_phys, 64);
    corrupt(&img, SUPER_MIRROR_OFFSETS[1] + 100, 16);
    assert!(ext_len > 4096 * 4);
    let damaged = std::fs::read(&img).unwrap();

    // read-only scrub: finds everything, repairs nothing, leaves the image byte-identical
    let mut vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let rep = scrub(&mut vol, &ScrubOptions { repair: false }).unwrap();
    assert!(rep.stats.verify_errors >= 1, "{:?}", rep.stats);
    assert!(rep.stats.csum_errors >= 1, "{:?}", rep.stats);
    assert_eq!(rep.stats.super_errors, 1, "{:?}", rep.stats);
    assert_eq!(rep.stats.corrected_errors, 0);
    assert_eq!(rep.stats.uncorrectable_errors, 0, "an intact DUP copy exists for everything");
    assert!(rep.bad.iter().any(|b| b.metadata) && rep.bad.iter().any(|b| !b.metadata));
    drop(vol);
    assert_eq!(std::fs::read(&img).unwrap(), damaged);

    // repair scrub
    let mut vol = Volume::open(vec![FileDev::open_rw(&img).unwrap()]).unwrap();
    let rep = scrub(&mut vol, &ScrubOptions { repair: true }).unwrap();
    assert!(rep.stats.corrected_errors >= 3, "{:?}", rep.stats);
    assert_eq!(rep.stats.uncorrectable_errors, 0, "{:?}", rep.stats);
    assert!(rep.bad.iter().all(|b| b.corrected), "{:?}", rep.bad);
    vol.flush().unwrap();
    drop(vol);

    // second scrub is clean, the checker and btrfs-progs agree, and the data + metadata bytes are pristine again
    let mut vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let rep = scrub(&mut vol, &ScrubOptions { repair: false }).unwrap();
    assert_eq!(rep.stats.csum_errors + rep.stats.verify_errors + rep.stats.super_errors + rep.stats.read_errors, 0, "{:?}", rep);
    assert!(run("btrfs", &["check", "--readonly", img.to_str().unwrap()]).0);
    assert!(check_devices(vec![FileDev::open_ro(&img).unwrap()], &Options { check_data_csums: true, max_findings: 100 }).is_clean());
    let fixed = std::fs::read(&img).unwrap();
    assert_eq!(&fixed[meta_phys as usize..meta_phys as usize + 16384], &pristine[meta_phys as usize..meta_phys as usize + 16384]);
    assert_eq!(&fixed[data_phys as usize..data_phys as usize + 64], &pristine[data_phys as usize..data_phys as usize + 64]);
}

#[test]
fn single_profile_corruption_is_uncorrectable() {
    if !have("mkfs.btrfs") {
        return;
    }
    let d = scratch("scrub-single");
    let t = d.join("t");
    std::fs::create_dir_all(&t).unwrap();
    std::fs::write(t.join("f"), payload(6, 600_000)).unwrap();
    let img = d.join("s.img");
    mkfs_btrfs(&img, 160, Some(&t), &["-d", "single", "-m", "single", "--nodiscard"]);
    let (phys, _) = {
        let mut vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
        let ext_root = vol.tree_root(EXTENT_TREE_OBJECTID).unwrap();
        let mut e = None;
        vol.walk_tree(ext_root, Key::min(0), Key::max(u64::MAX), &mut |k, dd| {
            if k.ty == EXTENT_ITEM_KEY && dd[16] & 1 == 1 {
                e = Some(k.objectid);
                return false;
            }
            true
        })
        .unwrap();
        let l = e.unwrap();
        (vol.chunk_for(l).unwrap().map(l).unwrap().copies[0].physical, l)
    };
    corrupt(&img, phys + 8192, 100);
    let mut vol = Volume::open(vec![FileDev::open_rw(&img).unwrap()]).unwrap();
    let rep = scrub(&mut vol, &ScrubOptions { repair: true }).unwrap();
    assert!(rep.stats.csum_errors >= 1);
    assert!(rep.stats.uncorrectable_errors >= 1);
    assert_eq!(rep.stats.corrected_errors, 0);
}

// ------------------------------------------------------------------ RAID5/6 on in-memory devices

fn raid_volume(profile: u64, ndev: usize) -> Volume<MemDev> {
    let dev_size = 4 << 20;
    let mut devs = BTreeMap::new();
    for i in 0..ndev {
        devs.insert(i as u64 + 1, MemDev::new(dev_size));
    }
    let chunk = ChunkMap {
        logical: 1 << 30,
        length: (ndev as u64 - if profile == BLOCK_GROUP_RAID6 { 2 } else { 1 }) * raid::ChunkMap { logical: 0, length: 0, flags: 0, stripe_len: STRIPE_LEN, stripes: vec![], sub_stripes: 0 }.stripe_len * 8,
        flags: BLOCK_GROUP_DATA | profile,
        stripe_len: STRIPE_LEN,
        stripes: (0..ndev).map(|i| Stripe { devid: i as u64 + 1, physical: 1 << 20 }).collect(),
        sub_stripes: 0,
    };
    Volume::synthetic(devs, vec![chunk], 16384, 4096, loos_btrfs::ChecksumAlgorithm::Crc32c)
}

/// Writes `data` at `logical` with correct parity on every full stripe touched (full-stripe writes only).
fn write_full_stripes(vol: &mut Volume<MemDev>, logical: u64, data: &[u8]) {
    let ch = vol.chunk_for(logical).unwrap().clone();
    let nr_data = ch.data_stripes();
    let sl = ch.stripe_len as usize;
    let full = nr_data * sl;
    assert_eq!(data.len() % full, 0);
    for (f, block) in data.chunks(full).enumerate() {
        let base = logical + (f * full) as u64;
        let m = ch.map(base).unwrap();
        let p = m.parity.unwrap();
        let stripes: Vec<&[u8]> = block.chunks(sl).collect();
        let mut pp = vec![0u8; sl];
        let mut qq = vec![0u8; sl];
        let np = ch.profile().unwrap().parity();
        raid::compute_parity(&stripes, &mut pp, if np == 2 { Some(&mut qq) } else { None });
        for (i, s) in stripes.iter().enumerate() {
            vol.write_copy(p.stripes[i], s).unwrap();
        }
        vol.write_copy(p.stripes[nr_data], &pp).unwrap();
        if np == 2 {
            vol.write_copy(p.stripes[nr_data + 1], &qq).unwrap();
        }
    }
}

#[test]
fn raid56_reads_survive_missing_devices_and_scrub_rebuilds_parity() {
    for (profile, ndev, tolerate) in [(BLOCK_GROUP_RAID5, 4usize, 1usize), (BLOCK_GROUP_RAID6, 5, 2), (BLOCK_GROUP_RAID5, 3, 1)] {
        let mut vol = raid_volume(profile, ndev);
        let ch = vol.chunk_for(1 << 30).unwrap().clone();
        let total = ch.data_stripes() * ch.stripe_len as usize * 8;
        let data = payload(profile + ndev as u64, total);
        write_full_stripes(&mut vol, 1 << 30, &data);
        let mut back = vec![0u8; total];
        vol.read_logical(1 << 30, &mut back).unwrap();
        assert_eq!(back, data, "healthy read");

        let truth = data.clone();
        let base = 1u64 << 30;

        // (1) silent corruption of one member: only checksums notice; parity rebuilds the stripe
        {
            let mut v1 = raid_volume(profile, ndev);
            write_full_stripes(&mut v1, base, &data);
            for b in v1.dev_mut(1).unwrap().data.iter_mut() {
                *b = 0xcc;
            }
            let mut out = vec![0u8; total];
            let mut verify = |lg: u64, seg: &[u8]| seg == &truth[(lg - base) as usize..(lg - base) as usize + seg.len()];
            let fallbacks = v1.read_logical_verified(base, &mut out, &mut verify).unwrap();
            assert_eq!(out, data, "{} devices: silent corruption rebuilt from parity", ndev);
            assert!(fallbacks > 0);
        }

        // (2) lost devices: reads fail with an error, parity covers up to `tolerate` of them
        for victim in 1..=tolerate as u64 {
            vol.drop_device(victim).unwrap();
        }
        let mut out = vec![0u8; total];
        vol.read_logical(base, &mut out).unwrap();
        assert_eq!(out, data, "{} devices, {} lost: degraded read", ndev, tolerate);

        // one more loss than the profile tolerates cannot be rebuilt
        vol.drop_device(tolerate as u64 + 1).unwrap();
        let mut out = vec![0u8; total];
        assert!(vol.read_logical(base, &mut out).is_err());
    }
}

#[test]
fn raid5_parity_scrub_fixes_a_stale_parity_stripe() {
    let mut vol = raid_volume(BLOCK_GROUP_RAID5, 4);
    let ch = vol.chunk_for(1 << 30).unwrap().clone();
    let total = ch.data_stripes() * ch.stripe_len as usize * 8;
    write_full_stripes(&mut vol, 1 << 30, &payload(77, total));
    // trash the parity of full stripe 2
    let m = ch.map((1 << 30) + 2 * 3 * STRIPE_LEN).unwrap();
    let p = m.parity.unwrap();
    vol.write_copy(p.stripes[3], &vec![0x11u8; STRIPE_LEN as usize]).unwrap();
    // scrub_parity is exercised through the public entry point on a volume with no trees: use the
    // dedicated helper exposed for synthetic volumes
    let rep = scrub_parity_only(&mut vol, &ScrubOptions { repair: false });
    assert_eq!(rep.stats.parity_errors, 1);
    let rep = scrub_parity_only(&mut vol, &ScrubOptions { repair: true });
    assert_eq!((rep.stats.parity_errors, rep.stats.corrected_errors), (1, 1));
    let rep = scrub_parity_only(&mut vol, &ScrubOptions { repair: false });
    assert_eq!(rep.stats.parity_errors, 0);
}

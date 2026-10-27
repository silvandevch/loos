mod common;
use common::*;
use loos_btrfs_ops::dev::{Dev, FileDev};
use loos_btrfs_ops::extents::inventory;
use loos_btrfs_ops::trim::*;
use loos_btrfs_ops::volume::Volume;
use std::collections::BTreeMap;

/// Every device byte belongs to exactly one of: reserved area, live extent copy, trim range.
fn assert_exact_partition(vol: &mut Volume<FileDev>, plan: &TrimPlan) {
    let inv = inventory(vol).unwrap();
    let mut per_dev: BTreeMap<u64, Vec<(u64, u64, &'static str)>> = BTreeMap::new();
    for id in vol.devids() {
        let len = vol.dev_items[&id].total_bytes;
        for (s, e) in reserved_ranges(len) {
            per_dev.entry(id).or_default().push((s, e.min(len), "reserved"));
        }
    }
    for e in &inv.extents {
        let mut pos = e.start;
        while pos < e.start + e.len {
            let m = vol.chunk_for(pos).unwrap().map(pos).unwrap();
            let n = m.len.min(e.start + e.len - pos);
            for c in &m.copies {
                per_dev.entry(c.devid).or_default().push((c.physical, c.physical + n, "extent"));
            }
            pos += n;
        }
    }
    for r in &plan.ranges {
        per_dev.entry(r.devid).or_default().push((r.physical, r.physical + r.len, "trim"));
    }
    for (id, mut v) in per_dev {
        v.sort();
        let size = vol.dev_items[&id].total_bytes;
        let mut pos = 0u64;
        for (s, e, what) in v {
            if s < pos {
                panic!("device {}: {} [{}, {}) overlaps an earlier range ending at {}", id, what, s, e, pos);
            }
            assert!(s == pos || what == "reserved" || true);
            // gaps must be bytes inside allocated chunks that hold neither extents nor are discardable: none expected
            assert_eq!(s, pos, "device {}: gap [{}, {}) before {} range", id, pos, s, what);
            pos = e;
        }
        assert_eq!(pos, size, "device {} not fully partitioned", id);
    }
}

#[test]
fn trim_plan_partitions_the_device_exactly() {
    if !have("mkfs.btrfs") {
        return;
    }
    let d = scratch("trim");
    let t = d.join("t");
    std::fs::create_dir_all(&t).unwrap();
    std::fs::write(t.join("a"), payload(1, 5_000_000)).unwrap();
    std::fs::write(t.join("b"), payload(2, 300_000)).unwrap();
    for extra in [vec!["-d", "single", "-m", "single"], vec!["-d", "dup", "-m", "dup"], vec!["--compress", "zstd"]] {
        let img = d.join("t.img");
        mkfs_btrfs(&img, 200, Some(&t), &extra);
        let mut vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
        let plan = plan_trim(&mut vol, &TrimOptions::default()).unwrap();
        assert!(plan.bytes > 100 << 20, "most of an almost empty 200 MiB device is discardable: {}", plan.bytes);
        assert_eq!(plan.bytes, plan.ranges.iter().map(|r| r.len).sum::<u64>());
        assert_exact_partition(&mut vol, &plan);

        // minlen / granularity / window options
        let coarse = plan_trim(&mut vol, &TrimOptions { minlen: 1 << 20, granularity: 1 << 20, ..Default::default() }).unwrap();
        assert!(coarse.ranges.iter().all(|r| r.len >= 1 << 20 && r.physical % (1 << 20) == 0 && r.len % (1 << 20) == 0));
        assert!(coarse.bytes <= plan.bytes);
        let window = plan_trim(&mut vol, &TrimOptions { start: 100 << 20, len: 50 << 20, ..Default::default() }).unwrap();
        assert!(window.ranges.iter().all(|r| r.physical >= 100 << 20 && r.physical + r.len <= 150 << 20));
        assert!(window.bytes > 0 && window.bytes <= 50 << 20);
        let none = plan_trim(&mut vol, &TrimOptions { minlen: 1 << 40, ..Default::default() }).unwrap();
        assert!(none.ranges.is_empty());
        let _ = vol.dev_mut(1).unwrap().len();
    }
}

#[test]
fn zone_helpers() {
    let zs = 256u64 << 20;
    let zones: Vec<ZoneState> = (0..4).map(|i| ZoneState { start: i * zs, write_pointer: if i == 3 { i * zs } else { i * zs + zs / 2 } }).collect();
    // live data only in zone 1
    let live = [(zs + 4096, zs + 8192)];
    assert_eq!(zones_to_reset(zs, &zones, &live), vec![0, 2 * zs], "zone 1 holds data, zone 3 is already empty");
    assert!(zoned_chunk_ok(zs, 2 * zs, zs));
    assert!(!zoned_chunk_ok(zs, 4096, zs));
    assert!(!zoned_chunk_ok(zs, zs, zs / 2));
    assert!(!zoned_chunk_ok(3 << 20, 3 << 20, 3 << 20), "zone size must be a power of two");
}

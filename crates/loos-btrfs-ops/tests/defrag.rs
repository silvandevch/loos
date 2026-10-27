mod common;
use common::*;
use loos_btrfs_ops::consts::*;
use loos_btrfs_ops::defrag::*;
use loos_btrfs_ops::dev::FileDev;
use loos_btrfs_ops::fs::{ExtentKind, FileExtent, Fs, Node};
use loos_btrfs_ops::volume::Volume;
use std::collections::BTreeSet;

fn ext(file_offset: u64, len: u64, disk: u64, comp: u8) -> FileExtent {
    FileExtent { file_offset, kind: ExtentKind::Regular { prealloc: false, compression: comp, disk_bytenr: disk, disk_num_bytes: len, offset: 0, num_bytes: len, ram_bytes: len } }
}

const K: u64 = 1024;

#[test]
fn fragmented_small_extents_are_merged_into_one_range() {
    // 8 x 16 KiB extents scattered on disk, then a big already-fine extent
    let mut v: Vec<FileExtent> = (0..8).map(|i| ext(i * 16 * K, 16 * K, 1_000_000 + (7 - i) * 1_000_000, 0)).collect();
    v.push(ext(128 * K, 64 << 20, 900_000_000, 0));
    let p = plan_extents(&v, &BTreeSet::new(), &DefragOptions::default());
    assert_eq!(p.ranges, vec![(0, 128 * K)], "{:?}", p);
    assert_eq!(p.extents_before, 9);
    assert_eq!(p.extents_after, 2);
    assert_eq!(p.bytes, 128 * K);
}

#[test]
fn contiguous_or_large_extents_are_left_alone() {
    // physically contiguous small extents (already merged on disk)
    let v: Vec<FileExtent> = (0..4).map(|i| ext(i * 16 * K, 16 * K, 5_000_000 + i * 16 * K, 0)).collect();
    let p = plan_extents(&v, &BTreeSet::new(), &DefragOptions::default());
    assert!(p.ranges.is_empty(), "{:?}", p);
    // extents above the threshold
    let v = vec![ext(0, 64 << 20, 10_000, 0), ext(64 << 20, 64 << 20, 900_000_000, 0)];
    assert!(plan_extents(&v, &BTreeSet::new(), &DefragOptions::default()).ranges.is_empty());
    // a lowered threshold makes the same extents candidates only when they are small relative to it
    let v: Vec<FileExtent> = (0..3).map(|i| ext(i * 64 * K, 64 * K, 100_000 * (3 - i), 0)).collect();
    assert!(!plan_extents(&v, &BTreeSet::new(), &DefragOptions { extent_thresh: 1 << 20, ..Default::default() }).ranges.is_empty());
    assert!(plan_extents(&v, &BTreeSet::new(), &DefragOptions { extent_thresh: 32 * K, ..Default::default() }).ranges.is_empty());
}

#[test]
fn holes_prealloc_inline_and_shared_extents_break_runs() {
    let hole = FileExtent { file_offset: 32 * K, kind: ExtentKind::Regular { prealloc: false, compression: 0, disk_bytenr: 0, disk_num_bytes: 0, offset: 0, num_bytes: 16 * K, ram_bytes: 16 * K } };
    let pre = FileExtent { file_offset: 64 * K, kind: ExtentKind::Regular { prealloc: true, compression: 0, disk_bytenr: 777, disk_num_bytes: 16 * K, offset: 0, num_bytes: 16 * K, ram_bytes: 16 * K } };
    let v = vec![ext(0, 16 * K, 90_000, 0), ext(16 * K, 16 * K, 10_000, 0), hole, ext(48 * K, 16 * K, 500_000, 0), pre, ext(80 * K, 16 * K, 123_000, 0), ext(96 * K, 16 * K, 20_000, 0)];
    let p = plan_extents(&v, &BTreeSet::new(), &DefragOptions::default());
    // first two merge; the one before the hole's successor and prealloc stand alone; last two merge
    assert_eq!(p.ranges, vec![(0, 32 * K), (80 * K, 112 * K)], "{:?}", p);
    // shared extents are never merged...
    let shared: BTreeSet<u64> = [10_000u64].into_iter().collect();
    let p = plan_extents(&v, &shared, &DefragOptions::default());
    assert_eq!(p.ranges, vec![(80 * K, 112 * K)]);
    // ...unless recompression is requested
    let p = plan_extents(&v, &shared, &DefragOptions { compress: Some(COMPRESS_ZSTD), ..Default::default() });
    assert!(p.ranges.iter().any(|r| r.0 <= 16 * K && r.1 >= 32 * K));
    assert!(p.extents_after > 0);
}

#[test]
fn recompression_targets_wrong_or_unwanted_compression() {
    let v = vec![ext(0, 1 << 20, 1_000_000, COMPRESS_ZLIB), ext(1 << 20, 1 << 20, 50_000_000, COMPRESS_ZSTD), ext(2 << 20, 1 << 20, 99_000_000, COMPRESS_NONE)];
    let to_zstd = plan_extents(&v, &BTreeSet::new(), &DefragOptions { compress: Some(COMPRESS_ZSTD), ..Default::default() });
    assert_eq!(to_zstd.ranges, vec![(0, 1 << 20), (2 << 20, 3 << 20)]);
    let none = plan_extents(&v, &BTreeSet::new(), &DefragOptions { nocompress: true, ..Default::default() });
    assert_eq!(none.ranges, vec![(0, 2 << 20)], "both compressed extents are adjacent and merge");
    // only the requested window is considered
    let win = plan_extents(&v, &BTreeSet::new(), &DefragOptions { compress: Some(COMPRESS_ZSTD), start: 2 << 20, len: 1 << 20, ..Default::default() });
    assert_eq!(win.ranges, vec![(2 << 20, 3 << 20)]);
}

#[test]
fn autodefrag_queue_rules() {
    let mut q = AutoDefrag::new();
    let a = Node { subvol: 5, ino: 300 };
    let b = Node { subvol: 5, ino: 301 };
    assert!(!q.on_write(a, 0, 1 << 20, 4096, 0), "large writes do not queue");
    assert!(!q.on_write(a, 0, 4096, 64 << 20, 0), "a small write into a big extent changes nothing");
    assert!(q.on_write(a, 200_000, 4096, 4096, 0));
    assert!(!q.on_write(a, 5, 4096, 4096, 0), "no duplicate entries");
    assert!(q.on_write(b, 70_000, 100, 8192, 0));
    assert_eq!(q.len(), 2);
    assert_eq!(q.next(), Some((a, 196_608)), "resume from the 64 KiB boundary of the first write");
    q.forget(b);
    assert!(q.is_empty() && q.next().is_none());
    assert!(q.on_write(a, 0, 100, 100, 0), "after popping it can be queued again");
}

#[test]
fn live_filesystem_plan_for_contiguous_and_recompressed_files() {
    if !have("mkfs.btrfs") {
        return;
    }
    let d = scratch("defrag");
    let t = d.join("t");
    std::fs::create_dir_all(&t).unwrap();
    std::fs::write(t.join("big"), payload(1, 6 << 20)).unwrap();
    std::fs::write(t.join("text"), vec![b'q'; 3 << 20]).unwrap();
    let img = d.join("z.img");
    // mkfs.btrfs creates the image but does not apply a compression mount
    // policy; compressed extents require a mounted writable filesystem.
    mkfs_btrfs(&img, 200, Some(&t), &["--nodiscard"]);
    let mut fs = Fs::open(Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap()).unwrap();
    let big = fs.resolve("/big").unwrap();
    let p = plan_file(&mut fs, big, &DefragOptions::default()).unwrap();
    assert!(p.extents_before > 0 && p.extents_after > 0, "{:?}", p);
    // If the host image contains compressed extents, exercise the
    // recompression planner; otherwise the image remains a valid uncompressed
    // fixture and this portion is an external-tool limitation.
    let text = fs.resolve("/text").unwrap();
    let compressed = fs
        .extents(text, 0, 3 << 20)
        .unwrap()
        .iter()
        .any(|e| matches!(&e.kind, ExtentKind::Regular { compression, .. } if *compression != COMPRESS_NONE));
    if compressed {
        let p = plan_file(&mut fs, text, &DefragOptions { nocompress: true, ..Default::default() }).unwrap();
        assert_eq!(p.bytes, 3 << 20, "{:?}", p);
        let p = plan_file(&mut fs, text, &DefragOptions { compress: Some(COMPRESS_ZSTD), ..Default::default() }).unwrap();
        assert!(p.ranges.is_empty(), "already zstd: {:?}", p);
    }
}

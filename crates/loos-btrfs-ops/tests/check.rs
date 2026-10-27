mod common;
use common::*;
use loos_btrfs_ops::check::*;
use loos_btrfs_ops::consts::*;
use loos_btrfs_ops::dev::FileDev;
use loos_btrfs_ops::volume::*;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

fn tree(root: &Path) {
    let w = |p: &str, data: Vec<u8>| {
        let f = root.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, data).unwrap();
    };
    w("a.txt", b"hello".to_vec());
    w("big.bin", payload(1, 2_000_000));
    w("text.txt", vec![b'x'; 500_000]);
    w("dir/b.txt", payload(2, 9000));
    w("dir/sub/c", payload(3, 100));
    w("empty", vec![]);
    for i in 0..300 {
        w(&format!("many/f{:03}", i), format!("v{}", i).into_bytes());
    }
    std::os::unix::fs::symlink("a.txt", root.join("link")).unwrap();
}

fn check_img(img: &Path) -> Report {
    check_devices(vec![FileDev::open_ro(img).unwrap()], &Options { check_data_csums: true, max_findings: 1000 })
}

fn progs_check(img: &Path) -> bool {
    run("btrfs", &["check", "--readonly", img.to_str().unwrap()]).0
}

fn dump(r: &Report) -> String {
    r.findings.iter().map(|f| format!("{:?} {} tree {} @{} {:?}: {}", f.severity, f.kind, f.tree, f.bytenr, f.key, f.msg)).collect::<Vec<_>>().join("\n")
}

#[test]
fn fresh_filesystems_check_clean() {
    if !have("mkfs.btrfs") || !have("btrfs") {
        eprintln!("skip: btrfs-progs missing");
        return;
    }
    let d = scratch("check-clean");
    let t = d.join("t");
    tree(&t);
    let cases: Vec<Vec<&str>> = vec![
        vec![],
        vec!["-d", "dup", "-m", "dup", "--checksum", "xxhash"],
        vec!["--checksum", "sha256"],
        vec!["-n", "4096"],
        vec!["-M"],
    ];
    for (i, extra) in cases.iter().enumerate() {
        let img = d.join(format!("c{}.img", i));
        mkfs_btrfs(&img, 160, Some(&t), extra);
        assert!(progs_check(&img), "btrfs-progs itself rejects {:?}", extra);
        let rep = check_img(&img);
        assert!(rep.is_clean(), "{:?}\n{}", extra, dump(&rep));
        assert!(rep.stats.tree_blocks > 5 && rep.stats.inodes > 300 && rep.stats.data_extents > 3, "{:?}", rep.stats);
    }
}

// ------------------------------------------------------------------------------------ corruption

struct Located {
    leaf: u64,
    data_off: usize,
    size: usize,
    item_idx: usize,
}

fn find_item(vol: &mut Volume<FileDev>, root: TreeRef, key: Key) -> Option<Located> {
    let mut bytenr = root.bytenr;
    let mut level = root.level;
    loop {
        let raw = vol.read_tree_block(bytenr, Some(level)).unwrap();
        if level == 0 {
            let n = u32::from_le_bytes(raw[96..100].try_into().unwrap()) as usize;
            for i in 0..n {
                let o = 101 + i * 25;
                if Key::parse(&raw[o..]) == key {
                    return Some(Located { leaf: bytenr, data_off: 101 + u32::from_le_bytes(raw[o + 17..o + 21].try_into().unwrap()) as usize, size: u32::from_le_bytes(raw[o + 21..o + 25].try_into().unwrap()) as usize, item_idx: i });
                }
            }
            return None;
        }
        let ptrs = vol.node_ptrs(&raw).unwrap();
        let mut pick = 0;
        for (i, p) in ptrs.iter().enumerate() {
            if p.0 <= key {
                pick = i;
            }
        }
        bytenr = ptrs[pick].1;
        level -= 1;
    }
}

/// Rewrites one tree block on every copy, recomputing the checksum, so only the semantic
/// corruption remains.
fn patch_block(img: &Path, bytenr: u64, fix_csum: bool, f: &dyn Fn(&mut Vec<u8>)) {
    let mut vol = Volume::open(vec![FileDev::open_ro(img).unwrap()]).unwrap();
    let mut raw = vol.read_tree_block(bytenr, None).unwrap();
    let alg = vol.alg();
    let m = vol.chunk_for(bytenr).unwrap().map(bytenr).unwrap();
    f(&mut raw);
    if fix_csum {
        let c = loos_btrfs::digest(alg, &raw[32..]);
        for b in raw[..32].iter_mut() {
            *b = 0;
        }
        raw[..c.as_bytes().len()].copy_from_slice(c.as_bytes());
    }
    let mut file = std::fs::OpenOptions::new().write(true).open(img).unwrap();
    for cp in &m.copies {
        file.seek(SeekFrom::Start(cp.physical)).unwrap();
        file.write_all(&raw).unwrap();
    }
}

fn fresh(d: &Path, name: &str) -> std::path::PathBuf {
    let t = d.join("t");
    if !t.exists() {
        tree(&t);
    }
    let img = d.join(name);
    mkfs_btrfs(&img, 160, Some(&t), &["--nodiscard"]);
    img
}

fn locate(img: &Path, tree_id: u64, key: Key) -> Located {
    let mut vol = Volume::open(vec![FileDev::open_ro(img).unwrap()]).unwrap();
    let root = vol.tree_root(tree_id).unwrap();
    find_item(&mut vol, root, key).unwrap_or_else(|| panic!("item {:?} not found", key))
}

fn inode_of(img: &Path, path: &str) -> u64 {
    use loos_btrfs_ops::fs::Fs;
    let vol = Volume::open(vec![FileDev::open_ro(img).unwrap()]).unwrap();
    let mut fs = Fs::open(vol).unwrap();
    fs.resolve(path).unwrap().ino
}

#[test]
fn semantic_corruptions_are_detected_like_btrfs_check() {
    if !have("mkfs.btrfs") || !have("btrfs") {
        return;
    }
    let d = scratch("check-corrupt");
    // (name, mutation) pairs: each must be flagged by us with the expected kind and rejected by btrfs-progs
    struct Case {
        name: &'static str,
        kind: &'static str,
        apply: Box<dyn Fn(&Path)>,
    }
    let cases: Vec<Case> = vec![
        Case {
            name: "nlink",
            kind: "nlink",
            apply: Box::new(|img| {
                let ino = inode_of(img, "/a.txt");
                let l = locate(img, FS_TREE_OBJECTID, Key::new(ino, INODE_ITEM_KEY, 0));
                patch_block(img, l.leaf, true, &|b| b[l.data_off + 40] = 7);
            }),
        },
        Case {
            name: "dirsize",
            kind: "dir-size",
            apply: Box::new(|img| {
                let ino = inode_of(img, "/dir");
                let l = locate(img, FS_TREE_OBJECTID, Key::new(ino, INODE_ITEM_KEY, 0));
                patch_block(img, l.leaf, true, &|b| b[l.data_off + 16] = b[l.data_off + 16].wrapping_add(2));
            }),
        },
        Case {
            name: "bgused",
            kind: "bg-used",
            apply: Box::new(|img| {
                let mut vol = Volume::open(vec![FileDev::open_ro(img).unwrap()]).unwrap();
                let bg = vol.chunks().iter().find(|c| c.flags & BLOCK_GROUP_DATA != 0).unwrap().clone();
                let key = Key::new(bg.logical, BLOCK_GROUP_ITEM_KEY, bg.length);
                // block groups live in the block-group tree on modern filesystems, else in the extent tree
                let l = [BLOCK_GROUP_TREE_OBJECTID, EXTENT_TREE_OBJECTID]
                    .iter()
                    .find_map(|t| {
                        let r = vol.tree_root(*t).ok()?;
                        find_item(&mut vol, r, key)
                    })
                    .unwrap();
                drop(vol);
                patch_block(img, l.leaf, true, &|b| b[l.data_off] = b[l.data_off].wrapping_add(1));
            }),
        },
        Case {
            name: "extentrefs",
            kind: "extent-refs",
            apply: Box::new(|img| {
                let mut vol = Volume::open(vec![FileDev::open_ro(img).unwrap()]).unwrap();
                let root = vol.tree_root(EXTENT_TREE_OBJECTID).unwrap();
                // first data extent item
                let mut key = None;
                vol.walk_tree(root, Key::min(0), Key::max(u64::MAX), &mut |k, dd| {
                    if k.ty == EXTENT_ITEM_KEY && dd.len() >= 24 && dd[16] & 1 == 1 {
                        key = Some(*k);
                        return false;
                    }
                    true
                })
                .unwrap();
                let l = find_item(&mut vol, root, key.unwrap()).unwrap();
                drop(vol);
                patch_block(img, l.leaf, true, &|b| b[l.data_off] = b[l.data_off].wrapping_add(1));
            }),
        },
        Case {
            name: "keyorder",
            kind: "key-order",
            apply: Box::new(|img| {
                let ino = inode_of(img, "/a.txt");
                let l = locate(img, FS_TREE_OBJECTID, Key::new(ino, INODE_ITEM_KEY, 0));
                // make this item's key larger than its successor's
                patch_block(img, l.leaf, true, &|b| {
                    let o = 101 + l.item_idx * 25;
                    b[o + 8] = 250;
                });
            }),
        },
        Case {
            name: "badcsum",
            kind: "bad-csum",
            apply: Box::new(|img| {
                let ino = inode_of(img, "/a.txt");
                let l = locate(img, FS_TREE_OBJECTID, Key::new(ino, INODE_ITEM_KEY, 0));
                patch_block(img, l.leaf, false, &|b| b[3000] ^= 0x55);
            }),
        },
        Case {
            name: "nbytes",
            kind: "nbytes",
            apply: Box::new(|img| {
                let ino = inode_of(img, "/big.bin");
                let l = locate(img, FS_TREE_OBJECTID, Key::new(ino, INODE_ITEM_KEY, 0));
                patch_block(img, l.leaf, true, &|b| b[l.data_off + 24 + 1] = b[l.data_off + 24 + 1].wrapping_add(1));
            }),
        },
    ];
    for c in cases {
        let img = fresh(&d, &format!("{}.img", c.name));
        assert!(check_img(&img).is_clean(), "{}: baseline must be clean", c.name);
        (c.apply)(&img);
        let rep = check_img(&img);
        assert!(rep.has(c.kind), "{}: expected a {:?} finding, got:\n{}", c.name, c.kind, dump(&rep));
        assert!(!rep.is_clean());
        assert!(!progs_check(&img), "{}: btrfs-progs accepted what we rejected", c.name);
    }
}

#[test]
fn data_csum_corruption_is_reported_and_missing_devices_do_not_panic() {
    if !have("mkfs.btrfs") {
        return;
    }
    let d = scratch("check-data");
    let img = fresh(&d, "data.img");
    let ino = inode_of(&img, "/big.bin");
    let vol_img = img.clone();
    let (logical, phys) = {
        let mut vol = Volume::open(vec![FileDev::open_ro(&vol_img).unwrap()]).unwrap();
        let root = vol.tree_root(FS_TREE_OBJECTID).unwrap();
        let it = vol.tree_first_ge(root, Key::new(ino, EXTENT_DATA_KEY, 0), Key::new(ino, EXTENT_DATA_KEY, u64::MAX)).unwrap().unwrap();
        let logical = u64::from_le_bytes(it.data[21..29].try_into().unwrap());
        let m = vol.chunk_for(logical).unwrap().map(logical).unwrap();
        (logical, m.copies[0].physical)
    };
    {
        let mut f = std::fs::OpenOptions::new().write(true).open(&img).unwrap();
        f.seek(SeekFrom::Start(phys + 10)).unwrap();
        f.write_all(&[0xff, 0xee, 0xdd]).unwrap();
    }
    let rep = check_img(&img);
    assert!(rep.has("data-csum"), "{}", dump(&rep));
    let bad = rep.findings.iter().find(|f| f.kind == "data-csum").unwrap();
    assert_eq!(bad.bytenr / 4096, logical / 4096);
    // no device at all
    let rep = check_devices::<loos_btrfs_ops::dev::MemDev>(vec![], &Options::default());
    assert!(rep.has("no-valid-superblock") || rep.has("open-failed"));
    // garbage device
    let rep = check_devices(vec![loos_btrfs_ops::dev::MemDev::new(1 << 20)], &Options::default());
    assert!(!rep.is_clean());
}

mod common;
use common::*;
use loos_btrfs_ops::consts::*;
use loos_btrfs_ops::dev::FileDev;
use loos_btrfs_ops::fs::Fs;
use loos_btrfs_ops::ioctl::*;
use loos_btrfs_ops::ioctl_abi::layout::*;
use loos_btrfs_ops::ioctl_fs::ReadOnlyBackend;
use loos_btrfs_ops::volume::Volume;

fn get64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn get32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn field(dump: &str, name: &str) -> String {
    dump.lines().find(|l| l.starts_with(name)).map(|l| l[name.len()..].trim().to_string()).unwrap_or_default()
}

#[test]
fn query_ioctls_agree_with_btrfs_progs() {
    if !have("mkfs.btrfs") || !have("btrfs") {
        eprintln!("skip: btrfs-progs missing");
        return;
    }
    let d = scratch("ioctl-fs");
    let tree = d.join("tree");
    std::fs::create_dir_all(tree.join("sub/deeper")).unwrap();
    std::fs::write(tree.join("a.txt"), b"hi").unwrap();
    std::fs::write(tree.join("sub/deeper/f"), payload(1, 200_000)).unwrap();
    std::fs::write(tree.join("hole"), b"x").unwrap();
    let img = d.join("i.btrfs");
    mkfs_btrfs(&img, 160, Some(&tree), &["-L", "IOCTLFS"]);
    let sup = must("btrfs", &["inspect-internal", "dump-super", img.to_str().unwrap()]);

    let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let mut fs = Fs::open(vol).unwrap();
    let root = fs.root_node().unwrap();
    let mut be = ReadOnlyBackend::new(&mut fs, root);

    // FS_INFO vs dump-super
    let c = code("BTRFS_IOC_FS_INFO").unwrap();
    let mut a = vec![0u8; ioc_size(c)];
    dispatch(&mut be, c, &mut a).unwrap();
    assert_eq!(get32(&a, btrfs_ioctl_fs_info_args::NODESIZE).to_string(), field(&sup, "nodesize"));
    assert_eq!(get32(&a, btrfs_ioctl_fs_info_args::SECTORSIZE).to_string(), field(&sup, "sectorsize"));
    assert_eq!(get64(&a, btrfs_ioctl_fs_info_args::GENERATION).to_string(), field(&sup, "generation"));
    assert_eq!(get64(&a, btrfs_ioctl_fs_info_args::NUM_DEVICES).to_string(), field(&sup, "num_devices"));
    let fsid: String = a[btrfs_ioctl_fs_info_args::FSID..btrfs_ioctl_fs_info_args::FSID + 16].iter().map(|b| format!("{:02x}", b)).collect();
    assert_eq!(fsid, field(&sup, "fsid").replace('-', ""));

    // DEV_INFO vs dump-super dev_item
    let c = code("BTRFS_IOC_DEV_INFO").unwrap();
    let mut a = vec![0u8; ioc_size(c)];
    put64(&mut a, btrfs_ioctl_dev_info_args::DEVID, 1);
    dispatch(&mut be, c, &mut a).unwrap();
    assert_eq!(get64(&a, btrfs_ioctl_dev_info_args::TOTAL_BYTES).to_string(), field(&sup, "total_bytes"));
    put64(&mut a, btrfs_ioctl_dev_info_args::DEVID, 9);
    assert_eq!(dispatch(&mut be, c, &mut a), Err(Errno::ENODEV));

    // SPACE_INFO: data + metadata + system groups, totals add up to the device chunk allocation
    let c = code("BTRFS_IOC_SPACE_INFO").unwrap();
    let mut a = vec![0u8; btrfs_ioctl_space_args::SIZE_OF + 8 * btrfs_ioctl_space_info::SIZE_OF];
    put64(&mut a, btrfs_ioctl_space_args::SPACE_SLOTS, 8);
    dispatch(&mut be, c, &mut a).unwrap();
    let n = get64(&a, btrfs_ioctl_space_args::TOTAL_SPACES) as usize;
    assert!(n >= 3, "data, metadata and system spaces expected, got {}", n);
    let mut flags_seen = 0u64;
    for i in 0..n {
        let o = btrfs_ioctl_space_args::SIZE_OF + i * btrfs_ioctl_space_info::SIZE_OF;
        let (f, total, used) = (get64(&a, o), get64(&a, o + 8), get64(&a, o + 16));
        assert!(used <= total && total > 0, "space {:x}: {}/{}", f, used, total);
        flags_seen |= f;
    }
    assert_eq!(flags_seen & 7, 7);

    // TREE_SEARCH over the root tree: same key sequence as `btrfs inspect-internal dump-tree -t 1`
    let c = code("BTRFS_IOC_TREE_SEARCH").unwrap();
    let mut mine: Vec<(u64, u32, u64)> = Vec::new();
    let (mut min_obj, mut min_type, mut min_off) = (0u64, 0u32, 0u64);
    let mut calls = 0;
    loop {
        let mut a = vec![0u8; ioc_size(c)];
        put64(&mut a, btrfs_ioctl_search_key::TREE_ID, 1);
        put64(&mut a, btrfs_ioctl_search_key::MIN_OBJECTID, min_obj);
        put32(&mut a, btrfs_ioctl_search_key::MIN_TYPE, min_type);
        put64(&mut a, btrfs_ioctl_search_key::MIN_OFFSET, min_off);
        put64(&mut a, btrfs_ioctl_search_key::MAX_OBJECTID, u64::MAX);
        put64(&mut a, btrfs_ioctl_search_key::MAX_OFFSET, u64::MAX);
        put64(&mut a, btrfs_ioctl_search_key::MAX_TRANSID, u64::MAX);
        put32(&mut a, btrfs_ioctl_search_key::MAX_TYPE, 255);
        put32(&mut a, btrfs_ioctl_search_key::NR_ITEMS, 4096);
        dispatch(&mut be, c, &mut a).unwrap();
        calls += 1;
        let nr = get32(&a, btrfs_ioctl_search_key::NR_ITEMS) as usize;
        if nr == 0 {
            break;
        }
        let mut pos = btrfs_ioctl_search_key::SIZE_OF;
        for _ in 0..nr {
            let it = (get64(&a, pos + btrfs_ioctl_search_header::OBJECTID), get32(&a, pos + btrfs_ioctl_search_header::TYPE), get64(&a, pos + btrfs_ioctl_search_header::OFFSET));
            mine.push(it);
            (min_obj, min_type, min_off) = it;
            pos += btrfs_ioctl_search_header::SIZE_OF + get32(&a, pos + btrfs_ioctl_search_header::LEN) as usize;
        }
        // resume strictly after the last key, exactly like btrfs-progs' search loop
        if min_off < u64::MAX {
            min_off += 1;
        } else if min_type < 255 {
            min_type += 1;
            min_off = 0;
        } else {
            min_obj += 1;
            min_type = 0;
            min_off = 0;
        }
    }
    assert!(calls >= 2, "the 4 KiB buffer forces at least two round trips");
    let dump = must("btrfs", &["inspect-internal", "dump-tree", "-t", "1", img.to_str().unwrap()]);
    let ids = |s: &str| -> Option<u64> {
        Some(match s {
            "EXTENT_TREE" => 2, "CHUNK_TREE" => 3, "DEV_TREE" => 4, "FS_TREE" => 5, "ROOT_TREE_DIR" => 6, "CSUM_TREE" => 7, "QUOTA_TREE" => 8, "UUID_TREE" => 9,
            "FREE_SPACE_TREE" => 10, "BLOCK_GROUP_TREE" => 11, "DATA_RELOC_TREE" => (-9i64) as u64, "TREE_LOG" => (-6i64) as u64, "ORPHAN" => (-5i64) as u64, "BALANCE" => (-4i64) as u64, other => other.parse().ok()?,
        })
    };
    let tys = |s: &str| -> Option<u32> {
        Some(match s {
            "INODE_ITEM" => 1, "INODE_REF" => 12, "DIR_ITEM" => 84, "DIR_INDEX" => 96, "ROOT_ITEM" => 132, "ROOT_BACKREF" => 144, "ROOT_REF" => 156,
            "UUID_KEY_SUBVOL" | "UUID_SUBVOL" => 251, "UUID_KEY_RECEIVED_SUBVOL" => 252, other => other.parse().ok()?,
        })
    };
    let mut theirs: Vec<(u64, u32, u64)> = Vec::new();
    for l in dump.lines().filter(|l| l.trim_start().starts_with("item ")) {
        let k = &l[l.find("key (").unwrap() + 5..];
        let k = &k[..k.find(')').unwrap()];
        let p: Vec<&str> = k.split_whitespace().collect();
        theirs.push((ids(p[0]).unwrap_or_else(|| panic!("unmapped objectid {}", p[0])), tys(p[1]).unwrap_or_else(|| panic!("unmapped type {}", p[1])), p[2].parse().unwrap()));
    }
    assert_eq!(mine, theirs, "TREE_SEARCH and dump-tree disagree");
    // box filter: only ROOT_ITEMs, through TREE_SEARCH_V2 with a 64 KiB buffer
    let c2 = code("BTRFS_IOC_TREE_SEARCH_V2").unwrap();
    let mut b = vec![0u8; btrfs_ioctl_search_args_v2::SIZE_OF + 65536];
    put64(&mut b, btrfs_ioctl_search_args_v2::BUF_SIZE, 65536);
    put64(&mut b, btrfs_ioctl_search_key::TREE_ID, 1);
    put64(&mut b, btrfs_ioctl_search_key::MAX_OBJECTID, u64::MAX);
    put64(&mut b, btrfs_ioctl_search_key::MAX_OFFSET, u64::MAX);
    put64(&mut b, btrfs_ioctl_search_key::MAX_TRANSID, u64::MAX);
    put32(&mut b, btrfs_ioctl_search_key::MIN_TYPE, 132);
    put32(&mut b, btrfs_ioctl_search_key::MAX_TYPE, 132);
    put32(&mut b, btrfs_ioctl_search_key::NR_ITEMS, 4096);
    dispatch(&mut be, c2, &mut b).unwrap();
    assert_eq!(get32(&b, btrfs_ioctl_search_key::NR_ITEMS) as usize, mine.iter().filter(|m| m.1 == 132).count());

    // GET_LABEL, features, supported features
    let c = code("BTRFS_IOC_GET_FSLABEL").unwrap();
    let mut a = vec![0u8; 256];
    dispatch(&mut be, c, &mut a).unwrap();
    assert_eq!(&a[..7], b"IOCTLFS");
    let c = code("BTRFS_IOC_GET_FEATURES").unwrap();
    let mut a = vec![0u8; ioc_size(c)];
    dispatch(&mut be, c, &mut a).unwrap();
    let incompat = get64(&a, 16);
    assert_eq!(incompat & (INCOMPAT_SKINNY_METADATA | INCOMPAT_EXTENDED_IREF | INCOMPAT_NO_HOLES), INCOMPAT_SKINNY_METADATA | INCOMPAT_EXTENDED_IREF | INCOMPAT_NO_HOLES, "mkfs defaults: {:x}", incompat);
    assert!(field(&sup, "incompat_flags").starts_with(&format!("0x{:x}", incompat)));

    // mutating ioctls are refused
    let c = code("BTRFS_IOC_SNAP_CREATE").unwrap();
    let mut a = vec![0u8; ioc_size(c)];
    a[btrfs_ioctl_vol_args::NAME] = b's';
    assert_eq!(dispatch(&mut be, c, &mut a), Err(Errno::EROFS));
    drop(be);

    // per-inode ioctls on the read-only subvolume
    let sub = fs.resolve("/sub/deeper/f").unwrap();
    let has_nested_subvolume = sub.subvol != 5;
    let mut be = ReadOnlyBackend::new(&mut fs, sub);
    let c = code("BTRFS_IOC_SUBVOL_GETFLAGS").unwrap();
    let mut a = vec![0u8; 8];
    dispatch(&mut be, c, &mut a).unwrap();
    assert_eq!(get64(&a, 0), if has_nested_subvolume { SUBVOL_RDONLY } else { 0 }, "subvolume flags");
    let c = code("BTRFS_IOC_GET_SUBVOL_INFO").unwrap();
    let mut a = vec![0u8; ioc_size(c)];
    dispatch(&mut be, c, &mut a).unwrap();
    if has_nested_subvolume {
        assert_ne!(get64(&a, btrfs_ioctl_get_subvol_info_args::TREEID), 5);
        assert_eq!(get64(&a, btrfs_ioctl_get_subvol_info_args::PARENT_ID), 5);
        assert_eq!(&a[btrfs_ioctl_get_subvol_info_args::NAME..btrfs_ioctl_get_subvol_info_args::NAME + 3], b"sub");
        assert_eq!(get64(&a, btrfs_ioctl_get_subvol_info_args::FLAGS) & 1, 1);
    } else {
        assert_eq!(get64(&a, btrfs_ioctl_get_subvol_info_args::TREEID), 5);
    }
    // FIEMAP of a compressed multi-extent file
    let c = code("FS_IOC_FIEMAP").unwrap();
    let mut a = vec![0u8; fiemap::SIZE_OF + 8 * fiemap_extent::SIZE_OF];
    put64(&mut a, fiemap::FM_LENGTH, u64::MAX);
    put32(&mut a, fiemap::FM_EXTENT_COUNT, 8);
    dispatch(&mut be, c, &mut a).unwrap();
    let mapped = get32(&a, fiemap::FM_MAPPED_EXTENTS);
    assert!(mapped >= 1);
    let last = fiemap::SIZE_OF + (mapped as usize - 1) * fiemap_extent::SIZE_OF;
    assert_eq!(get32(&a, last + fiemap_extent::FE_FLAGS) & 1, 1, "FIEMAP_EXTENT_LAST on the final extent");
    // INO_LOOKUP of the directory
    let dir = be.fs.resolve("/sub/deeper").unwrap();
    let c = code("BTRFS_IOC_INO_LOOKUP").unwrap();
    let mut a = vec![0u8; ioc_size(c)];
    put64(&mut a, btrfs_ioctl_ino_lookup_args::TREEID, dir.subvol);
    put64(&mut a, btrfs_ioctl_ino_lookup_args::OBJECTID, dir.ino);
    dispatch(&mut be, c, &mut a).unwrap();
    let name = &a[btrfs_ioctl_ino_lookup_args::NAME..btrfs_ioctl_ino_lookup_args::NAME + 8];
    if has_nested_subvolume {
        assert_eq!(name, b"deeper/\0");
    } else {
        assert_eq!(&name[..4], b"sub/");
    }
}

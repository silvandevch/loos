mod common;
use common::*;
use loos_btrfs_ops::ioctl::*;
use loos_btrfs_ops::ioctl_abi::{layout::*, IOCTLS};

fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn get64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn get32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

#[test]
fn ioctl_abi_matches_kernel_headers() {
    if !have("python3") || !have("gcc") || !std::path::Path::new("/usr/include/linux/btrfs.h").exists() {
        eprintln!("skip: no gcc/python3/uapi headers");
        return;
    }
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/gen/gen_ioctl_abi.py");
    let fresh = must("python3", &[script]);
    let checked_in = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ioctl_abi.rs")).unwrap();
    assert_eq!(fresh.trim(), checked_in.trim(), "src/ioctl_abi.rs drifted: re-run `python3 gen/gen_ioctl_abi.py > src/ioctl_abi.rs`");
}

#[test]
fn well_known_numbers() {
    assert_eq!(code("BTRFS_IOC_SNAP_CREATE"), Some(0x5000_9401));
    assert_eq!(code("BTRFS_IOC_SNAP_CREATE"), Some(ioc(IOC_WRITE, BTRFS_IOCTL_MAGIC, 1, 4096)));
    assert_eq!(code("FICLONE"), Some(0x4004_9409));
    assert_eq!(code("FIDEDUPERANGE"), Some(0xc018_9436));
    assert_eq!(code("BTRFS_IOC_TREE_SEARCH"), Some(0xd000_9411));
    assert_eq!(code("BTRFS_IOC_FS_INFO"), Some(ioc(IOC_READ, BTRFS_IOCTL_MAGIC, 31, 1024)));
    assert_eq!(code("BTRFS_IOC_NOPE"), None);
    assert!(matches!(name_of(0x4004_9409), Some("FICLONE") | Some("BTRFS_IOC_CLONE")), "aliases share the number");
    // every table entry decodes into consistent direction/size/type fields
    for (n, c) in IOCTLS {
        assert_eq!(ioc(ioc_dir(*c), ioc_type(*c), ioc_nr(*c), ioc_size(*c) as u32), *c, "{}", n);
    }
    assert!(IOCTLS.len() >= 70, "table has only {} entries", IOCTLS.len());
}

#[test]
fn resize_strings() {
    assert_eq!(parse_resize("max"), Some(ResizeSpec { devid: 1, kind: ResizeKind::Max }));
    assert_eq!(parse_resize("2:+1g"), Some(ResizeSpec { devid: 2, kind: ResizeKind::Grow(1 << 30) }));
    assert_eq!(parse_resize("-512m"), Some(ResizeSpec { devid: 1, kind: ResizeKind::Shrink(512 << 20) }));
    assert_eq!(parse_resize("4096"), Some(ResizeSpec { devid: 1, kind: ResizeKind::Absolute(4096) }));
    assert_eq!(parse_resize("3:max").unwrap().devid, 3);
    for bad in ["", "+", "1x", "abc", "1:", "+-1", "99999999999999999999g", "18446744073709551615e"] {
        assert_eq!(parse_resize(bad), None, "{:?}", bad);
    }
}

#[derive(Default)]
struct Mock {
    seen: Vec<Request>,
}

impl Backend for Mock {
    fn ioctl(&mut self, req: Request) -> Result<Reply, Errno> {
        self.seen.push(req.clone());
        Ok(match req {
            Request::FsInfo { .. } => Reply::FsInfo(FsInfo { max_id: 3, num_devices: 3, fsid: [7; 16], nodesize: 16384, sectorsize: 4096, clone_alignment: 4096, csum_type: 1, csum_size: 8, generation: 99, metadata_uuid: [9; 16] }),
            Request::DevInfo { devid, .. } => Reply::DevInfo(DevInfo { devid, uuid: [1; 16], bytes_used: 5, total_bytes: 10, fsid: [7; 16], path: b"/dev/loop0".to_vec() }),
            Request::SpaceInfo { .. } => Reply::SpaceInfo(vec![(1, 100, 10), (4, 200, 20), (2, 300, 30)]),
            Request::TreeSearch { .. } => Reply::TreeSearch(vec![
                SearchItem { transid: 1, objectid: 256, offset: 0, ty: 1, data: vec![1; 160] },
                SearchItem { transid: 1, objectid: 256, offset: 0, ty: 12, data: vec![2; 20] },
                SearchItem { transid: 2, objectid: 257, offset: 5, ty: 1, data: vec![3; 160] },
            ]),
            Request::Dedupe { dests, .. } => Reply::Dedupe(dests.iter().enumerate().map(|(i, _)| DedupeResult { bytes: 4096 * (i as u64 + 1), status: if i == 1 { 1 } else { 0 } }).collect()),
            Request::InoLookup { objectid, .. } => Reply::Path { treeid: 5, path: format!("dir/file{}/", objectid).into_bytes() },
            Request::GetLabel => Reply::Label(b"LOOS".to_vec()),
            Request::SubvolGetFlags => Reply::Value(2),
            Request::Fiemap(_) => Reply::Fiemap { mapped: 2, extents: vec![FiemapExtent { logical: 0, physical: 1 << 20, length: 4096, flags: 0 }, FiemapExtent { logical: 4096, physical: 2 << 20, length: 8192, flags: 1 }] },
            Request::GetSubvolRootref { .. } => Reply::RootRefs(vec![(256, 256), (260, 258)]),
            Request::Scrub { .. } => Reply::Scrub(ScrubProgress { counters: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] }),
            Request::Balance { flags, .. } => Reply::Balance { flags, state: 1, data: BalanceArgs { profiles: 8, ..Default::default() }, meta: BalanceArgs::default(), sys: BalanceArgs::default(), stat: [10, 5, 2] },
            _ => Reply::Done,
        })
    }
}

fn buf(cmd_name: &str) -> (u32, Vec<u8>) {
    let c = code(cmd_name).unwrap();
    (c, vec![0u8; ioc_size(c)])
}

#[test]
fn fs_info_dev_info_space_info_roundtrip() {
    let mut m = Mock::default();
    let (c, mut a) = buf("BTRFS_IOC_FS_INFO");
    put64(&mut a, btrfs_ioctl_fs_info_args::FLAGS, 7);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(m.seen[0], Request::FsInfo { flags: 7 });
    assert_eq!(get64(&a, btrfs_ioctl_fs_info_args::MAX_ID), 3);
    assert_eq!(get32(&a, btrfs_ioctl_fs_info_args::NODESIZE), 16384);
    assert_eq!(&a[btrfs_ioctl_fs_info_args::FSID..btrfs_ioctl_fs_info_args::FSID + 16], &[7u8; 16]);
    assert_eq!(get64(&a, btrfs_ioctl_fs_info_args::GENERATION), 99);
    assert_eq!(u16::from_le_bytes(a[btrfs_ioctl_fs_info_args::CSUM_SIZE..][..2].try_into().unwrap()), 8);

    let (c, mut a) = buf("BTRFS_IOC_DEV_INFO");
    put64(&mut a, btrfs_ioctl_dev_info_args::DEVID, 2);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(&a[btrfs_ioctl_dev_info_args::PATH..btrfs_ioctl_dev_info_args::PATH + 10], b"/dev/loop0");
    assert_eq!(get64(&a, btrfs_ioctl_dev_info_args::TOTAL_BYTES), 10);

    // space info with room for only 2 of the 3 spaces: total_spaces still reports 3
    let (c, _) = buf("BTRFS_IOC_SPACE_INFO");
    let mut a = vec![0u8; btrfs_ioctl_space_args::SIZE_OF + 2 * btrfs_ioctl_space_info::SIZE_OF];
    put64(&mut a, btrfs_ioctl_space_args::SPACE_SLOTS, 2);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(get64(&a, btrfs_ioctl_space_args::TOTAL_SPACES), 3);
    let o = btrfs_ioctl_space_args::SIZE_OF + btrfs_ioctl_space_info::SIZE_OF;
    assert_eq!((get64(&a, o), get64(&a, o + 8), get64(&a, o + 16)), (4, 200, 20));
}

#[test]
fn tree_search_v1_and_v2_buffers() {
    let mut m = Mock::default();
    // v1: fixed 4096 byte argument
    let (c, mut a) = buf("BTRFS_IOC_TREE_SEARCH");
    assert_eq!(a.len(), 4096);
    put64(&mut a, btrfs_ioctl_search_key::TREE_ID, 5);
    put64(&mut a, btrfs_ioctl_search_key::MAX_OBJECTID, u64::MAX);
    put32(&mut a, btrfs_ioctl_search_key::MAX_TYPE, 255);
    put32(&mut a, btrfs_ioctl_search_key::NR_ITEMS, 2);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(get32(&a, btrfs_ioctl_search_key::NR_ITEMS), 2, "nr_items clamps to the request");
    let h = btrfs_ioctl_search_key::SIZE_OF;
    assert_eq!(get64(&a, h + btrfs_ioctl_search_header::OBJECTID), 256);
    assert_eq!(get32(&a, h + btrfs_ioctl_search_header::LEN), 160);
    let second = h + btrfs_ioctl_search_header::SIZE_OF + 160;
    assert_eq!(get32(&a, second + btrfs_ioctl_search_header::TYPE), 12);
    assert_eq!(a[second + btrfs_ioctl_search_header::SIZE_OF], 2);

    // v2 with a buffer that fits exactly the first item
    let (c, _) = buf("BTRFS_IOC_TREE_SEARCH_V2");
    let cap = btrfs_ioctl_search_header::SIZE_OF + 160;
    let mut a = vec![0u8; btrfs_ioctl_search_args_v2::SIZE_OF + cap];
    put32(&mut a, btrfs_ioctl_search_key::NR_ITEMS, 10);
    put64(&mut a, btrfs_ioctl_search_args_v2::BUF_SIZE, cap as u64);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(get32(&a, btrfs_ioctl_search_key::NR_ITEMS), 1);
    // a buffer that cannot hold even one item fails with EOVERFLOW (75)
    let mut a = vec![0u8; btrfs_ioctl_search_args_v2::SIZE_OF + 100];
    put32(&mut a, btrfs_ioctl_search_key::NR_ITEMS, 10);
    put64(&mut a, btrfs_ioctl_search_args_v2::BUF_SIZE, 100);
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno(75)));
    // buf_size larger than the argument is rejected before the backend is called
    let before = m.seen.len();
    let mut a = vec![0u8; btrfs_ioctl_search_args_v2::SIZE_OF + 10];
    put64(&mut a, btrfs_ioctl_search_args_v2::BUF_SIZE, 4096);
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EFAULT));
    assert_eq!(m.seen.len(), before);
}

#[test]
fn volume_args_validation() {
    let mut m = Mock::default();
    let mk = |name: &[u8]| {
        let (c, mut a) = buf("BTRFS_IOC_SNAP_CREATE");
        put64(&mut a, btrfs_ioctl_vol_args::FD, 9);
        a[btrfs_ioctl_vol_args::NAME..btrfs_ioctl_vol_args::NAME + name.len()].copy_from_slice(name);
        (c, a)
    };
    let (c, mut a) = mk(b"snap-2026");
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(m.seen.last().unwrap(), &Request::SnapCreate { source_fd: 9, name: b"snap-2026".to_vec(), read_only: false, qgroups: None });
    for bad in [&b"a/b"[..], b".", b"..", b""] {
        let (c, mut a) = mk(bad);
        assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EINVAL), "{:?}", bad);
    }
    let (c, mut a) = mk(&[b'x'; 300]);
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EINVAL), "name longer than 255");
    // unterminated name
    let (c, mut a) = buf("BTRFS_IOC_SNAP_CREATE");
    for b in &mut a[btrfs_ioctl_vol_args::NAME..] {
        *b = b'q';
    }
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::ENAMETOOLONG));

    // v2: read-only flag, unsupported flag, by-id destroy
    let (c, mut a) = buf("BTRFS_IOC_SNAP_CREATE_V2");
    put64(&mut a, btrfs_ioctl_vol_args_v2::FLAGS, SUBVOL_RDONLY);
    a[btrfs_ioctl_vol_args_v2::NAME..btrfs_ioctl_vol_args_v2::NAME + 3].copy_from_slice(b"ro1");
    dispatch(&mut m, c, &mut a).unwrap();
    assert!(matches!(m.seen.last().unwrap(), Request::SnapCreate { read_only: true, .. }));
    put64(&mut a, btrfs_ioctl_vol_args_v2::FLAGS, 1 << 40);
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EOPNOTSUPP));
    let (c, mut a) = buf("BTRFS_IOC_SNAP_DESTROY_V2");
    put64(&mut a, btrfs_ioctl_vol_args_v2::FLAGS, SUBVOL_SPEC_BY_ID);
    put64(&mut a, btrfs_ioctl_vol_args_v2::SUBVOLID, 258);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(m.seen.last().unwrap(), &Request::SnapDestroy { target: Spec::Id(258) });

    // resize through vol_args
    let (c, mut a) = buf("BTRFS_IOC_RESIZE");
    a[btrfs_ioctl_vol_args::NAME..btrfs_ioctl_vol_args::NAME + 6].copy_from_slice(b"1:+10m");
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(m.seen.last().unwrap(), &Request::Resize { spec: ResizeSpec { devid: 1, kind: ResizeKind::Grow(10 << 20) } });

    // subvolume flags
    let (c, mut a) = buf("BTRFS_IOC_SUBVOL_SETFLAGS");
    put64(&mut a, 0, 4);
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EOPNOTSUPP));
    put64(&mut a, 0, SUBVOL_RDONLY);
    dispatch(&mut m, c, &mut a).unwrap();
    let (c, mut a) = buf("BTRFS_IOC_SUBVOL_GETFLAGS");
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(get64(&a, 0), 2);
}

#[test]
fn clone_dedupe_defrag() {
    let mut m = Mock::default();
    let (c, mut a) = buf("FICLONE");
    put32(&mut a, 0, 12);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(m.seen.last().unwrap(), &Request::CloneRange { src_fd: 12, src_offset: 0, src_length: 0, dest_offset: 0 });
    let (c, mut a) = buf("FICLONERANGE");
    put64(&mut a, file_clone_range::SRC_FD, 5);
    put64(&mut a, file_clone_range::SRC_OFFSET, 4096);
    put64(&mut a, file_clone_range::SRC_LENGTH, 8192);
    put64(&mut a, file_clone_range::DEST_OFFSET, 65536);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(m.seen.last().unwrap(), &Request::CloneRange { src_fd: 5, src_offset: 4096, src_length: 8192, dest_offset: 65536 });

    // dedupe with 2 destinations: argument = header + 2 info records
    let c = code("FIDEDUPERANGE").unwrap();
    let hdr = file_dedupe_range::SIZE_OF;
    let info = file_dedupe_range_info::SIZE_OF;
    let mut a = vec![0u8; hdr + 2 * info];
    put64(&mut a, file_dedupe_range::SRC_LENGTH, 65536);
    a[file_dedupe_range::DEST_COUNT..file_dedupe_range::DEST_COUNT + 2].copy_from_slice(&2u16.to_le_bytes());
    for i in 0..2 {
        put64(&mut a, hdr + i * info + file_dedupe_range_info::DEST_FD, 20 + i as u64);
        put64(&mut a, hdr + i * info + file_dedupe_range_info::DEST_OFFSET, 4096 * i as u64);
    }
    dispatch(&mut m, c, &mut a).unwrap();
    assert!(matches!(m.seen.last().unwrap(), Request::Dedupe { dests, .. } if dests.len() == 2 && dests[1].fd == 21));
    assert_eq!(get64(&a, hdr + file_dedupe_range_info::BYTES_DEDUPED), 4096);
    assert_eq!(get32(&a, hdr + info + file_dedupe_range_info::STATUS), 1);
    // zero destinations / short buffer
    let mut z = vec![0u8; hdr];
    assert_eq!(dispatch(&mut m, c, &mut z), Err(Errno::EINVAL));
    let mut short = vec![0u8; hdr + info];
    short[file_dedupe_range::DEST_COUNT] = 2;
    assert_eq!(dispatch(&mut m, c, &mut short), Err(Errno::EFAULT));

    let (c, mut a) = buf("BTRFS_IOC_DEFRAG_RANGE");
    put64(&mut a, btrfs_ioctl_defrag_range_args::LEN, u64::MAX);
    put64(&mut a, btrfs_ioctl_defrag_range_args::FLAGS, 1 | 2);
    put32(&mut a, btrfs_ioctl_defrag_range_args::COMPRESS_TYPE, 3 | (3 << 8));
    dispatch(&mut m, c, &mut a).unwrap();
    assert!(matches!(m.seen.last().unwrap(), Request::Defrag { range: Some(r) } if r.compress_type == 3 && r.level == 3 && r.len == u64::MAX));
    put64(&mut a, btrfs_ioctl_defrag_range_args::FLAGS, 0x100);
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EOPNOTSUPP));
}

#[test]
fn misc_roundtrips() {
    let mut m = Mock::default();
    let (c, mut a) = buf("BTRFS_IOC_INO_LOOKUP");
    put64(&mut a, btrfs_ioctl_ino_lookup_args::OBJECTID, 257);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(get64(&a, btrfs_ioctl_ino_lookup_args::TREEID), 5);
    assert_eq!(&a[btrfs_ioctl_ino_lookup_args::NAME..btrfs_ioctl_ino_lookup_args::NAME + 16], b"dir/file257/\0\0\0\0");

    let (c, mut a) = buf("FS_IOC_GETFSLABEL");
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(&a[..5], b"LOOS\0");
    assert_eq!(code("BTRFS_IOC_GET_FSLABEL"), code("FS_IOC_GETFSLABEL"));

    let c = code("FS_IOC_FIEMAP").unwrap();
    let mut a = vec![0u8; fiemap::SIZE_OF + 2 * fiemap_extent::SIZE_OF];
    a[fiemap::FM_EXTENT_COUNT..fiemap::FM_EXTENT_COUNT + 4].copy_from_slice(&2u32.to_le_bytes());
    put64(&mut a, fiemap::FM_LENGTH, u64::MAX);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(get32(&a, fiemap::FM_MAPPED_EXTENTS), 2);
    assert_eq!(get64(&a, fiemap::SIZE_OF + fiemap_extent::SIZE_OF + fiemap_extent::FE_LENGTH), 8192);
    assert_eq!(get32(&a, fiemap::SIZE_OF + fiemap_extent::SIZE_OF + fiemap_extent::FE_FLAGS), 1);

    let (c, mut a) = buf("BTRFS_IOC_GET_SUBVOL_ROOTREF");
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(a[btrfs_ioctl_get_subvol_rootref_args::NUM_ITEMS], 2);
    assert_eq!(get64(&a, 8 + 16), 260);

    let (c, mut a) = buf("BTRFS_IOC_SCRUB");
    put64(&mut a, btrfs_ioctl_scrub_args::DEVID, 1);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(get64(&a, btrfs_ioctl_scrub_args::PROGRESS + 14 * 8), 15);
    put64(&mut a, btrfs_ioctl_scrub_args::FLAGS, 2);
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EOPNOTSUPP));

    let (c, mut a) = buf("BTRFS_IOC_BALANCE_V2");
    put64(&mut a, btrfs_ioctl_balance_args::FLAGS, 7);
    dispatch(&mut m, c, &mut a).unwrap();
    assert_eq!(get64(&a, btrfs_ioctl_balance_args::STATE), 1);
    assert_eq!(get64(&a, btrfs_ioctl_balance_args::DATA + btrfs_balance_args::PROFILES), 8);
    assert_eq!(get64(&a, btrfs_ioctl_balance_args::STAT + 16), 2);

    // quota / qgroup validation
    let (c, mut a) = buf("BTRFS_IOC_QUOTA_CTL");
    put64(&mut a, btrfs_ioctl_quota_ctl_args::CMD, 1);
    dispatch(&mut m, c, &mut a).unwrap();
    put64(&mut a, btrfs_ioctl_quota_ctl_args::CMD, 3);
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EINVAL));
    let (c, mut a) = buf("BTRFS_IOC_QGROUP_ASSIGN");
    put64(&mut a, btrfs_ioctl_qgroup_assign_args::ASSIGN, 1);
    put64(&mut a, btrfs_ioctl_qgroup_assign_args::SRC, 257);
    put64(&mut a, btrfs_ioctl_qgroup_assign_args::DST, 257);
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EINVAL));
    put64(&mut a, btrfs_ioctl_qgroup_assign_args::DST, 1u64 << 48 | 1);
    dispatch(&mut m, c, &mut a).unwrap();

    // flags mapping
    use loos_btrfs_ops::consts::*;
    let f = inode_flags_to_fs_flags(INODE_IMMUTABLE | INODE_NODATACOW | INODE_COMPRESS, false);
    assert_eq!(f, 0x10 | 0x80_0000 | 0x4);
    assert_eq!(fs_flags_to_inode_flags(f), INODE_IMMUTABLE | INODE_NODATACOW | INODE_COMPRESS);
}

#[test]
fn unknown_and_foreign_ioctls() {
    let mut m = Mock::default();
    let mut a = vec![0u8; 64];
    assert_eq!(dispatch(&mut m, ioc(IOC_READ, b'x', 1, 64), &mut a), Err(Errno::ENOTTY));
    assert_eq!(dispatch(&mut m, ioc(IOC_READ, BTRFS_IOCTL_MAGIC, 200, 64), &mut a), Err(Errno::ENOTTY));
    // too-short buffer for a known command
    let (c, _) = buf("BTRFS_IOC_FS_INFO");
    assert_eq!(dispatch(&mut m, c, &mut a), Err(Errno::EFAULT));
    // numbers with a typed decoder never fall through to Other
    let typed = [
        "BTRFS_IOC_FS_INFO", "BTRFS_IOC_DEV_INFO", "BTRFS_IOC_SPACE_INFO", "BTRFS_IOC_TREE_SEARCH", "BTRFS_IOC_INO_LOOKUP", "BTRFS_IOC_SCRUB", "BTRFS_IOC_BALANCE_V2",
        "BTRFS_IOC_QUOTA_CTL", "BTRFS_IOC_GET_SUBVOL_INFO", "FS_IOC_FIEMAP", "BTRFS_IOC_SEND", "BTRFS_IOC_DEV_REPLACE",
    ];
    for t in typed {
        let (c, mut a) = buf(t);
        if let Ok(Request::Other { .. }) = decode(c, &mut a) {
            panic!("{} has no typed decoder", t);
        }
    }
}

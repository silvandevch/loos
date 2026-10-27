mod common;
use common::*;
use loos_btrfs_ops::dev::FileDev;
use loos_btrfs_ops::fs::Fs;
use loos_btrfs_ops::swapfile::*;
use loos_btrfs_ops::volume::Volume;

fn open(img: &std::path::Path) -> Fs<FileDev> {
    Fs::open(Volume::open(vec![FileDev::open_ro(img).unwrap()]).unwrap()).unwrap()
}

#[test]
fn swapfile_rules_and_physical_mapping() {
    if !have("mkfs.btrfs") {
        return;
    }
    let help = run("mkfs.btrfs", &["--help"]).1;
    if !help.contains("--inode-flags") || !help.contains("--compress") {
        eprintln!("skip: installed mkfs.btrfs lacks image-time inode/compression flags");
        return;
    }
    let d = scratch("swap");
    let t = d.join("t");
    std::fs::create_dir_all(&t).unwrap();
    std::fs::write(t.join("swap"), payload(1, 8 << 20)).unwrap(); // nodatacow via --inode-flags
    std::fs::write(t.join("cow"), payload(2, 1 << 20)).unwrap();
    std::fs::write(t.join("odd"), payload(3, 4096 * 10 + 7)).unwrap();
    std::fs::write(t.join("tiny"), b"x").unwrap();
    std::fs::write(t.join("empty"), b"").unwrap();
    std::fs::write(t.join("zeros"), vec![b'a'; 4 << 20]).unwrap();
    let img = d.join("s.img");
    mkfs_btrfs(&img, 200, Some(&t), &["--nodiscard", "-d", "single", "-m", "single", "--inode-flags", "nodatacow:swap", "--inode-flags", "nodatacow:odd", "--inode-flags", "nodatacow:tiny", "--inode-flags", "nodatacow:empty"]);
    let mut fs = open(&img);
    let swap = fs.resolve("/swap").unwrap();
    let map = map_swapfile(&mut fs, swap).unwrap();
    assert_eq!(map.pages, (8 << 20) / 4096);
    assert_eq!(map.extents.iter().map(|e| e.pages).sum::<u64>(), map.pages);
    // runs are ordered, gap free in file space, and the first sector matches the chunk mapping of the first extent
    let mut p = 0;
    for e in &map.extents {
        assert_eq!(e.start_page, p);
        p += e.pages;
    }
    // independent check: the bytes at the mapped physical location are the file's bytes
    let raw = std::fs::read(&img).unwrap();
    let data = std::fs::read(t.join("swap")).unwrap();
    for e in &map.extents {
        let (off, len) = ((e.sector * 512) as usize, (e.pages * 4096) as usize);
        let file_off = (e.start_page * 4096) as usize;
        assert_eq!(&raw[off..off + len], &data[file_off..file_off + len], "physical run does not hold the file data");
    }

    // rejections
    let cow = fs.resolve("/cow").unwrap();
    assert_eq!(map_swapfile(&mut fs, cow), Err(SwapError::Cow));
    let odd = fs.resolve("/odd").unwrap();
    assert_eq!(map_swapfile(&mut fs, odd), Err(SwapError::Unaligned));
    let tiny = fs.resolve("/tiny").unwrap();
    assert!(matches!(map_swapfile(&mut fs, tiny), Err(SwapError::Unaligned) | Err(SwapError::Inline)));
    let empty = fs.resolve("/empty").unwrap();
    assert_eq!(map_swapfile(&mut fs, empty), Err(SwapError::Unaligned));
    let root = fs.root_node().unwrap();
    assert_eq!(map_swapfile(&mut fs, root), Err(SwapError::NotRegular));

    // DUP data is refused, compression is refused
    let img2 = d.join("dup.img");
    mkfs_btrfs(&img2, 200, Some(&t), &["--nodiscard", "-d", "dup", "-m", "dup", "--inode-flags", "nodatacow:swap"]);
    let mut fs2 = open(&img2);
    let s2 = fs2.resolve("/swap").unwrap();
    assert_eq!(map_swapfile(&mut fs2, s2), Err(SwapError::UnsupportedProfile));
    let img3 = d.join("z.img");
    mkfs_btrfs(&img3, 200, Some(&t), &["--nodiscard", "--compress", "zstd", "-d", "single", "-m", "single"]);
    let mut fs3 = open(&img3);
    let z = fs3.resolve("/zeros").unwrap();
    // compressible + COW file: rejected as COW before compression is even looked at; the flag order is part of the contract
    assert_eq!(map_swapfile(&mut fs3, z), Err(SwapError::Cow));
}

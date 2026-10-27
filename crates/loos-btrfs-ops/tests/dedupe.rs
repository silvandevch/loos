mod common;
use common::*;
use loos_btrfs_ops::dedupe::*;
use loos_btrfs_ops::dev::FileDev;
use loos_btrfs_ops::fs::Fs;
use loos_btrfs_ops::volume::Volume;

fn open(img: &std::path::Path) -> Fs<FileDev> {
    Fs::open(Volume::open(vec![FileDev::open_ro(img).unwrap()]).unwrap()).unwrap()
}

#[test]
fn finds_whole_file_and_partial_duplicates_without_false_positives() {
    if !have("mkfs.btrfs") {
        return;
    }
    let d = scratch("dedupe");
    let t = d.join("t");
    std::fs::create_dir_all(t.join("sub")).unwrap();
    let a = payload(1, 1 << 20);
    std::fs::write(t.join("a"), &a).unwrap();
    std::fs::write(t.join("b"), &a).unwrap(); // identical
    std::fs::write(t.join("sub/b2"), &a).unwrap(); // identical, other directory
    let mut c = a[..256 * 1024].to_vec(); // first 256 KiB shared, rest different
    c.extend(payload(2, 768 * 1024));
    std::fs::write(t.join("c"), &c).unwrap();
    std::fs::write(t.join("d"), payload(3, 1 << 20)).unwrap(); // unrelated
    std::fs::write(t.join("small"), &a[..4096]).unwrap(); // below the block size: ignored
    let mut e = a.clone();
    e[512 * 1024 + 5] ^= 1; // differs inside one block of the otherwise identical file
    std::fs::write(t.join("e"), &e).unwrap();
    let img = d.join("d.img");
    mkfs_btrfs(&img, 200, Some(&t), &["--nodiscard"]);
    let mut fs = open(&img);
    let rep = scan(&mut fs, &ScanOptions { block_size: 128 * 1024, min_run: 128 * 1024 }).unwrap();
    assert!(rep.saved_bytes > 0 && rep.bytes_scanned >= 5 << 20, "{:?}", rep);
    assert!(rep.files_skipped >= 1, "the small file is skipped");

    // every reported range is byte-identical to its source (re-verified here, independently)
    for run in &rep.runs {
        let sdata = {
            let mut v = vec![0u8; run.len as usize];
            assert_eq!(fs.read(run.src.node.node(), run.src.offset, &mut v).unwrap() as u64, run.len);
            v
        };
        for dst in &run.dsts {
            let mut v = vec![0u8; run.len as usize];
            assert_eq!(fs.read(dst.node.node(), dst.offset, &mut v).unwrap() as u64, run.len);
            assert_eq!(v, sdata, "{}", describe(run));
            assert_eq!(compare_ranges(&mut fs, run.src.node.node(), run.src.offset, dst.node.node(), dst.offset, run.len).unwrap(), RangeCmp::Same);
        }
    }
    // d is unrelated: no run may involve it
    let d_node = fs.resolve("/d").unwrap();
    assert!(rep.runs.iter().all(|r| r.src.node.node() != d_node && r.dsts.iter().all(|x| x.node.node() != d_node)));
    // the big identical triple a/b/b2 is found as a 1 MiB-class opportunity (maybe split around e/c)
    let total: u64 = rep.runs.iter().map(|r| r.len * r.dsts.len() as u64).sum();
    assert_eq!(total, rep.saved_bytes);
    assert!(rep.saved_bytes >= 2 * (1 << 20), "two full duplicates of a: {}", rep.saved_bytes);
    // compare_ranges notices the one flipped byte in e and the argument rules
    let (an, en) = (fs.resolve("/a").unwrap(), fs.resolve("/e").unwrap());
    assert_eq!(compare_ranges(&mut fs, an, 0, en, 0, 1 << 20).unwrap(), RangeCmp::Differs);
    assert_eq!(compare_ranges(&mut fs, an, 0, en, 0, 512 * 1024).unwrap(), RangeCmp::Same);
    assert!(compare_ranges(&mut fs, an, 1, en, 0, 4096).is_err());
    assert!(compare_ranges(&mut fs, an, 0, en, 0, (1 << 20) + 4096).is_err());
    assert!(compare_ranges(&mut fs, an, 0, en, 0, 100).is_err());
}

#[test]
fn reflinked_files_are_not_reported_twice() {
    if !have("mkfs.btrfs") {
        return;
    }
    // mkfs --reflink needs a reflink-capable source filesystem; skip quietly where unavailable
    let d = scratch("dedupe-reflink");
    let t = d.join("t");
    std::fs::create_dir_all(&t).unwrap();
    std::fs::write(t.join("a"), payload(1, 1 << 20)).unwrap();
    std::fs::copy(t.join("a"), t.join("b")).unwrap();
    let img = d.join("r.img");
    blank(&img, 200 << 20);
    let (ok, _) = run("mkfs.btrfs", &["-q", "-f", "--reflink", "-r", t.to_str().unwrap(), img.to_str().unwrap()]);
    if !ok {
        eprintln!("skip: no reflink support on the scratch filesystem");
        return;
    }
    let mut fs = open(&img);
    let rep = scan(&mut fs, &ScanOptions::default()).unwrap();
    assert!(rep.runs.is_empty() || rep.saved_bytes == 0, "already shared extents need no dedupe: {:?}", rep);
}

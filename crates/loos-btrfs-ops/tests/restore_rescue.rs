mod common;
use common::*;
use loos_btrfs_ops::consts::*;
use loos_btrfs_ops::dev::FileDev;
use loos_btrfs_ops::fs::Fs;
use loos_btrfs_ops::rescue::*;
use loos_btrfs_ops::restore::*;
use loos_btrfs_ops::volume::*;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

fn tree(root: &Path) {
    let w = |p: &str, data: Vec<u8>| {
        let f = root.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, data).unwrap();
    };
    w("a.txt", b"hello".to_vec());
    w("big.bin", payload(1, 2_500_000));
    w("text.txt", vec![b'z'; 400_000]);
    w("dir/b.txt", payload(2, 9000));
    w("dir/deep/er/c", payload(3, 100));
    w("sub/inner", payload(4, 5000));
    w("empty", vec![]);
    std::os::unix::fs::symlink("a.txt", root.join("link")).unwrap();
    std::fs::set_permissions(root.join("a.txt"), std::fs::Permissions::from_mode(0o640)).unwrap();
    std::fs::set_permissions(root.join("dir"), std::fs::Permissions::from_mode(0o750)).unwrap();
    // a sparse file with a hole in the middle
    let sp = root.join("sparse");
    let f = std::fs::File::create(&sp).unwrap();
    f.set_len(3_000_000).unwrap();
    drop(f);
    let mut f = std::fs::OpenOptions::new().write(true).open(&sp).unwrap();
    f.write_all(b"head").unwrap();
    f.seek(SeekFrom::Start(2_900_000)).unwrap();
    f.write_all(b"tail").unwrap();
}

fn same_tree(a: &Path, b: &Path, skip_subvol: Option<&str>) {
    let mut names: Vec<String> = std::fs::read_dir(a).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    for n in names {
        if Some(n.as_str()) == skip_subvol {
            continue;
        }
        let (pa, pb) = (a.join(&n), b.join(&n));
        let ma = std::fs::symlink_metadata(&pa).unwrap();
        let mb = std::fs::symlink_metadata(&pb).unwrap_or_else(|_| panic!("{} missing in restore", pb.display()));
        assert_eq!(ma.file_type().is_symlink(), mb.file_type().is_symlink(), "{}", n);
        if ma.file_type().is_symlink() {
            assert_eq!(std::fs::read_link(&pa).unwrap(), std::fs::read_link(&pb).unwrap());
        } else if ma.is_dir() {
            assert_eq!(ma.mode() & 0o7777, mb.mode() & 0o7777, "dir mode {}", n);
            same_tree(&pa, &pb, None);
        } else {
            assert_eq!(std::fs::read(&pa).unwrap(), std::fs::read(&pb).unwrap(), "{}", pa.display());
            assert_eq!(ma.mode() & 0o7777, mb.mode() & 0o7777, "mode {}", n);
        }
    }
}

#[test]
fn restore_reproduces_the_tree_and_agrees_with_btrfs_restore() {
    if !have("mkfs.btrfs") || !have("btrfs") {
        return;
    }
    let d = scratch("restore");
    let t = d.join("t");
    tree(&t);
    for (i, extra) in [vec![], vec!["--checksum", "xxhash"], vec!["-n", "4096"]].into_iter().enumerate() {
        let img = d.join(format!("r{}.img", i));
        mkfs_btrfs(&img, 160, Some(&t), &extra);
        let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
        let mut fs = Fs::open(vol).unwrap();
        let out = d.join(format!("out{}", i));
        std::fs::create_dir_all(&out).unwrap();
        let mut sink = DirSink { root: out.clone() };
        let has_subvol = extra.contains(&"rw:sub");
        let stats = restore(&mut fs, "", &mut sink, &RestoreOptions { snapshots: false, ..Default::default() }).unwrap();
        assert!(stats.files >= 7 && stats.symlinks == 1 && stats.dirs >= 3, "{:?}", stats);
        assert_eq!(stats.subvolumes_skipped, if has_subvol { 1 } else { 0 });
        same_tree(&t, &out, if has_subvol { Some("sub") } else { None });
        // sparse file stays sparse on the host
        let ms = std::fs::metadata(out.join("sparse")).unwrap();
        assert_eq!(ms.len(), 3_000_000);
        assert!(ms.blocks() * 512 < 2_000_000, "hole preserved ({} blocks)", ms.blocks());
        if has_subvol {
            let out2 = d.join(format!("out{}s", i));
            std::fs::create_dir_all(&out2).unwrap();
            let mut s2 = DirSink { root: out2.clone() };
            let st2 = restore(&mut fs, "", &mut s2, &RestoreOptions { snapshots: true, ..Default::default() }).unwrap();
            assert_eq!(st2.subvolumes_skipped, 0);
            same_tree(&t, &out2, None);
        }
        // partial restore of one directory
        let out3 = d.join(format!("out{}p", i));
        std::fs::create_dir_all(&out3).unwrap();
        let mut s3 = DirSink { root: out3.clone() };
        restore(&mut fs, "/dir", &mut s3, &RestoreOptions::default()).unwrap();
        assert_eq!(std::fs::read(out3.join("dir/b.txt")).unwrap(), std::fs::read(t.join("dir/b.txt")).unwrap());
        assert!(!out3.join("a.txt").exists());
        // the reference implementation restores the same bytes
        let refout = d.join(format!("ref{}", i));
        std::fs::create_dir_all(&refout).unwrap();
        let (ok, msg) = run("btrfs", &["restore", "-S", "-m", if has_subvol { "-s" } else { "-D" }, img.to_str().unwrap(), refout.to_str().unwrap()]);
        if ok && !has_subvol {
            // -D is a dry run: nothing written. Re-run for real.
            let _ = run("btrfs", &["restore", "-S", "-m", img.to_str().unwrap(), refout.to_str().unwrap()]);
        }
        let _ = msg;
        if refout.join("a.txt").exists() {
            assert_eq!(std::fs::read(refout.join("big.bin")).unwrap(), std::fs::read(out.join("big.bin")).unwrap());
            assert_eq!(std::fs::read(refout.join("text.txt")).unwrap(), std::fs::read(out.join("text.txt")).unwrap());
        }
    }
}

#[test]
fn restore_tolerates_corrupted_data_with_ignore_errors() {
    if !have("mkfs.btrfs") {
        return;
    }
    let d = scratch("restore-bad");
    let t = d.join("t");
    std::fs::create_dir_all(&t).unwrap();
    let data = payload(9, 1_000_000);
    std::fs::write(t.join("f"), &data).unwrap();
    std::fs::write(t.join("g"), b"intact").unwrap();
    let img = d.join("b.img");
    mkfs_btrfs(&img, 160, Some(&t), &["--nodiscard"]);
    let phys = {
        let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
        let mut fs = Fs::open(vol).unwrap();
        let n = fs.resolve("/f").unwrap();
        let e = fs.extents(n, 0, 1).unwrap();
        let logical = match &e[0].kind {
            loos_btrfs_ops::fs::ExtentKind::Regular { disk_bytenr, .. } => *disk_bytenr,
            _ => panic!(),
        };
        fs.vol.chunk_for(logical).unwrap().map(logical).unwrap().copies[0].physical
    };
    let mut f = std::fs::OpenOptions::new().write(true).open(&img).unwrap();
    f.seek(SeekFrom::Start(phys + 4096 * 5 + 7)).unwrap();
    f.write_all(&[1, 2, 3]).unwrap();
    drop(f);
    let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let mut fs = Fs::open(vol).unwrap();
    let out = d.join("o");
    std::fs::create_dir_all(&out).unwrap();
    let mut sink = DirSink { root: out.clone() };
    // strict: fails
    assert!(restore(&mut fs, "", &mut sink, &RestoreOptions::default()).is_err());
    // tolerant: reports the error, restores the rest, leaves a hole where the damaged bytes were
    std::fs::remove_dir_all(&out).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    let st = restore(&mut fs, "", &mut sink, &RestoreOptions { ignore_errors: true, ..Default::default() }).unwrap();
    assert!(st.errors.iter().any(|e| e.starts_with("f:")), "{:?}", st.errors);
    assert!(st.errors[0].contains("offset 20480"), "{:?}", st.errors);
    assert_eq!(std::fs::read(out.join("g")).unwrap(), b"intact");
    let got = std::fs::read(out.join("f")).unwrap();
    assert_eq!(got.len(), data.len());
    assert_eq!(&got[..4096 * 5], &data[..4096 * 5]);
    assert_eq!(&got[4096 * 6..], &data[4096 * 6..], "everything outside the damaged sector survives");
    assert!(got[4096 * 5..4096 * 6].iter().all(|&b| b == 0), "damaged sector is a hole");
    // verify=false hands back the corrupted bytes instead
    std::fs::remove_dir_all(&out).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    restore(&mut fs, "", &mut sink, &RestoreOptions { verify: false, ..Default::default() }).unwrap();
    let raw = std::fs::read(out.join("f")).unwrap();
    assert_eq!(&raw[4096 * 5 + 7..4096 * 5 + 10], &[1, 2, 3]);
}

#[test]
fn rescue_super_recover_zero_log_and_root_discovery() {
    if !have("mkfs.btrfs") || !have("btrfs") {
        return;
    }
    let d = scratch("rescue");
    let t = d.join("t");
    tree(&t);
    let img = d.join("x.img");
    mkfs_btrfs(&img, 160, Some(&t), &["--nodiscard", "-d", "dup", "-m", "dup"]);

    // 1. primary superblock destroyed: btrfs-progs falls back to a mirror, we restore the primary
    {
        let mut f = std::fs::OpenOptions::new().write(true).open(&img).unwrap();
        f.seek(SeekFrom::Start(65536)).unwrap();
        f.write_all(&[0u8; 4096]).unwrap();
    }
    let mut devs = vec![FileDev::open_rw(&img).unwrap()];
    let rec = super_recover(&mut devs).unwrap();
    assert!(rec.rewritten.iter().any(|(_, off)| *off == 65536), "{:?}", rec);
    assert!(rec.from_generation > 0);
    drop(devs);
    assert!(run("btrfs", &["check", "--readonly", img.to_str().unwrap()]).0);
    let mut raw = vec![0u8; 4096];
    FileDev::open_ro(&img).unwrap().read_at_for_test(65536, &mut raw);
    assert!(Super::parse(&raw, 65536).is_ok());

    // 2. a log tree pointer is dropped by zero_log; the superblocks stay valid
    {
        let mut f = std::fs::OpenOptions::new().read(true).write(true).open(&img).unwrap();
        let mut sb = vec![0u8; 4096];
        use std::io::Read;
        f.seek(SeekFrom::Start(65536)).unwrap();
        f.read_exact(&mut sb).unwrap();
        sb[96..104].copy_from_slice(&0x1234_5000u64.to_le_bytes());
        let c = loos_btrfs::digest(loos_btrfs::ChecksumAlgorithm::Crc32c, &sb[32..]);
        sb[..32].fill(0);
        sb[..4].copy_from_slice(c.as_bytes());
        f.seek(SeekFrom::Start(65536)).unwrap();
        f.write_all(&sb).unwrap();
    }
    {
        let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
        assert_eq!(vol.sb.log_root, 0x1234_5000);
    }
    let mut devs = vec![FileDev::open_rw(&img).unwrap()];
    assert!(zero_log(&mut devs).unwrap() >= 1);
    drop(devs);
    let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    assert_eq!(vol.sb.log_root, 0);
    drop(vol);
    assert!(run("btrfs", &["check", "--readonly", img.to_str().unwrap()]).0);

    // 3. the root tree block is destroyed in every copy: roots are rediscovered by scanning
    let (root_bytenr, copies) = {
        let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
        let rb = vol.sb.root.bytenr;
        (rb, vol.chunk_for(rb).unwrap().map(rb).unwrap().copies)
    };
    {
        let mut f = std::fs::OpenOptions::new().write(true).open(&img).unwrap();
        for c in &copies {
            f.seek(SeekFrom::Start(c.physical)).unwrap();
            f.write_all(&vec![0xeeu8; 16384]).unwrap();
        }
    }
    let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let mut fs = Fs::open(vol);
    assert!(fs.is_err(), "the default-subvolume lookup needs the root tree");
    let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let mut vol = vol;
    let found = find_tree_roots(&mut vol).unwrap();
    let fs_root = found.get(&FS_TREE_OBJECTID).expect("FS tree root rediscovered");
    assert!(fs_root.level <= 3 && fs_root.generation > 0);
    let roots: std::collections::BTreeMap<u64, TreeRef> = found.iter().filter(|(k, _)| **k == FS_TREE_OBJECTID || **k == CSUM_TREE_OBJECTID).map(|(k, v)| (*k, TreeRef { bytenr: v.bytenr, level: v.level })).collect();
    let mut fs = Fs::open_with_roots(vol, roots);
    let out = d.join("rescued");
    std::fs::create_dir_all(&out).unwrap();
    let mut sink = DirSink { root: out.clone() };
    let st = restore(&mut fs, "", &mut sink, &RestoreOptions::default()).unwrap();
    assert!(st.files >= 8, "{:?}", st);
    assert_eq!(std::fs::read(out.join("big.bin")).unwrap(), std::fs::read(t.join("big.bin")).unwrap());
    assert_eq!(std::fs::read(out.join("dir/deep/er/c")).unwrap(), std::fs::read(t.join("dir/deep/er/c")).unwrap());
    assert!(root_bytenr > 0);
}

trait ReadForTest {
    fn read_at_for_test(&mut self, off: u64, buf: &mut [u8]);
}
impl ReadForTest for FileDev {
    fn read_at_for_test(&mut self, off: u64, buf: &mut [u8]) {
        use loos_btrfs_ops::dev::Dev;
        self.read_at(off, buf).unwrap();
    }
}

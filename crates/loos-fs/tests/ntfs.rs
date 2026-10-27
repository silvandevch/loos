mod common;
use common::*;
use loos_fs::ntfs::Ntfs;
use loos_fs::{FileDev, FileKind, FsError};
use std::path::Path;

fn tools() -> bool {
    if !(have("mkntfs") && have("ntfscp") && have("ntfscat") && have("ntfsls")) {
        eprintln!("skip: ntfs-3g tools missing");
        return false;
    }
    true
}

fn mk(dir: &Path, mb: u64, extra: &[&str]) -> std::path::PathBuf {
    let img = dir.join("ntfs.img");
    blank(&img, mb << 20);
    let mut args = vec!["-F", "-Q", "-L", "LOOSNT"];
    args.extend_from_slice(extra);
    let p = img.to_str().unwrap().to_string();
    args.push(&p);
    must("mkntfs", &args);
    img
}

fn cp(img: &Path, data: &[u8], dest: &str, stream: Option<&str>) {
    let tmp = img.with_extension("src");
    std::fs::write(&tmp, data).unwrap();
    let mut args = vec!["-f"];
    if let Some(s) = stream {
        args.push("-N");
        args.push(s);
    }
    args.push(img.to_str().unwrap());
    args.push(tmp.to_str().unwrap());
    args.push(dest);
    must("ntfscp", &args);
}

fn cat(img: &Path, path: &str) -> Vec<u8> {
    let out = std::process::Command::new("ntfscat").args(["-f", img.to_str().unwrap(), path]).output().unwrap();
    assert!(out.status.success(), "ntfscat {} failed", path);
    out.stdout
}

fn text(seed: u64, len: usize) -> Vec<u8> {
    // compressible text with an incompressible tail
    let mut v = Vec::new();
    while v.len() < len / 2 {
        v.extend_from_slice(b"The quick brown fox jumps over the lazy dog. ");
    }
    v.extend(payload(seed, len - v.len()));
    v.truncate(len);
    v
}

#[test]
fn reads_plain_volumes_of_every_cluster_size() {
    if !tools() {
        return;
    }
    for (mb, extra) in [(64u64, vec![]), (64, vec!["-c", "512"]), (64, vec!["-c", "1024"]), (128, vec!["-c", "65536"]), (64, vec!["-s", "4096"])] {
        let d = scratch(&format!("ntfs-plain-{}", extra.join("")));
        let img = mk(&d, mb, &extra);
        let small = b"resident data".to_vec();
        let big = payload(7, 1_234_567);
        cp(&img, &small, "/small.txt", None);
        cp(&img, &big, "/Big File.bin", None);
        cp(&img, &payload(8, 5000), "/ünï — name.dat", None);
        cp(&img, b"", "/empty", None);
        cp(&img, b"stream-body", "/small.txt", Some("meta"));
        cp(&img, &payload(9, 100_000), "/big-ads.bin", None);
        cp(&img, &payload(10, 20_000), "/big-ads.bin", Some("Zone.Identifier"));

        let mut fs = Ntfs::mount(FileDev::open_ro(&img).unwrap(), 0).unwrap();
        assert_eq!(fs.label(), "LOOSNT");
        assert_eq!(fs.version().0, 3);
        let root = fs.read_dir("/").unwrap();
        let names: Vec<&str> = root.iter().map(|e| e.name.as_str()).collect();
        for want in ["small.txt", "Big File.bin", "ünï — name.dat", "empty", "$MFT", "$Extend", "$Volume"] {
            assert!(names.contains(&want), "{} missing from {:?}", want, names);
        }
        assert!(!names.iter().any(|n| n.contains('~') && n.len() == 12), "DOS aliases hidden: {:?}", names);
        assert_eq!(fs.read_all("/small.txt").unwrap(), small);
        assert_eq!(fs.read_all("/big file.bin").unwrap(), big, "case-insensitive");
        assert_eq!(fs.read_all("\\ÜNÏ — NAME.DAT").unwrap(), payload(8, 5000));
        assert_eq!(fs.read_all("/empty").unwrap(), b"");
        assert_eq!(fs.read_all("/small.txt:meta").unwrap(), b"stream-body");
        assert_eq!(fs.read_all("/big-ads.bin:Zone.Identifier:$DATA").unwrap(), payload(10, 20_000));
        assert_eq!(fs.streams("/small.txt").unwrap(), vec!["meta".to_string()]);
        assert_eq!(fs.stat("/Big File.bin").unwrap().size, 1_234_567);
        assert_eq!(fs.read_all("/Big File.bin").unwrap(), cat(&img, "/Big File.bin"));
        // partial reads across cluster boundaries
        let mut buf = vec![0u8; 10_000];
        for off in [0u64, 511, 4095, 4096, 65535, 1_230_000] {
            let n = fs.read_file("/Big File.bin", off, &mut buf).unwrap();
            let end = (off as usize + n).min(big.len());
            assert_eq!(&buf[..n], &big[off as usize..end]);
        }
        assert_eq!(fs.read_file("/Big File.bin", 5_000_000, &mut buf).unwrap(), 0);
        // metadata directories, nested path
        let ext: Vec<String> = fs.read_dir("/$Extend").unwrap().into_iter().map(|e| e.name).collect();
        assert!(ext.iter().any(|n| n == "$ObjId" || n == "$Quota" || n == "$Reparse" || n == "$UsnJrnl"), "{:?}", ext);
        assert_eq!(fs.stat("/$Extend").unwrap().kind, FileKind::Dir);
        assert_eq!(fs.read_all("/$Extend").unwrap_err(), FsError::IsDir);
        assert_eq!(fs.read_all("/nope").unwrap_err(), FsError::NotFound);
        assert_eq!(fs.read_all("/small.txt/x").unwrap_err(), FsError::NotDir);
        let free = fs.free_clusters().unwrap();
        assert!(free > 0 && free * fs.cluster_bytes() < (mb << 20));
    }
}

#[test]
fn large_directory_exercises_the_index_btree() {
    if !tools() {
        return;
    }
    let d = scratch("ntfs-bigdir");
    let img = mk(&d, 64, &[]);
    let n = 500usize;
    let names: Vec<String> = (0..n).map(|i| format!("file-{:05}-{}", (i * 7919) % 100_000, "x".repeat(60 + i % 50))).collect();
    for (i, nm) in names.iter().enumerate() {
        cp(&img, format!("body {}", i).as_bytes(), &format!("/{}", nm), None);
    }
    let mut fs = Ntfs::mount(FileDev::open_ro(&img).unwrap(), 0).unwrap();
    let listed: Vec<String> = fs.read_dir("/").unwrap().into_iter().map(|e| e.name).collect();
    for nm in &names {
        assert!(listed.contains(nm), "missing {}", nm);
    }
    // indexed lookups of every name (and a case-folded variant)
    for (i, nm) in names.iter().enumerate() {
        assert_eq!(fs.read_all(&format!("/{}", nm)).unwrap(), format!("body {}", i).as_bytes());
        assert_eq!(fs.read_all(&format!("/{}", nm.to_uppercase())).unwrap(), format!("body {}", i).as_bytes());
    }
    assert_eq!(fs.read_all("/file-99999-zzz").unwrap_err(), FsError::NotFound);
    // listing is in NTFS collation order (case-insensitive code-unit order)
    let mut sorted = listed.clone();
    sorted.sort_by_key(|s| s.to_uppercase());
    assert_eq!(listed, sorted);
}

#[test]
fn lznt1_compressed_files() {
    if !tools() {
        return;
    }
    for extra in [vec!["-C"], vec!["-C", "-c", "4096"], vec!["-C", "-c", "512"]] {
        let d = scratch(&format!("ntfs-comp-{}", extra.join("")));
        let img = mk(&d, 64, &extra);
        let a = text(11, 960_000);
        let b = vec![0u8; 300_000]; // all zeros -> sparse/compressed
        let c = text(12, 70_000);
        cp(&img, &a, "/comp.txt", None);
        cp(&img, &b, "/zeros.bin", None);
        cp(&img, &c, "/mixed.dat", None);
        let mut fs = Ntfs::mount(FileDev::open_ro(&img).unwrap(), 0).unwrap();
        assert_eq!(fs.read_all("/comp.txt").unwrap(), a, "{:?}", extra);
        assert_eq!(fs.read_all("/zeros.bin").unwrap(), b);
        assert_eq!(fs.read_all("/mixed.dat").unwrap(), c);
        assert_eq!(fs.attributes("/comp.txt").unwrap() & 0x800, 0x800, "COMPRESSED attribute");
        // random access crossing compression-unit edges
        let mut buf = vec![0u8; 50_000];
        for off in [0u64, 1, 65535, 65536, 65537, 131071, 500_000, 959_000] {
            let n = fs.read_file("/comp.txt", off, &mut buf).unwrap();
            assert_eq!(&buf[..n], &a[off as usize..off as usize + n], "off {}", off);
        }
        assert_eq!(fs.read_all("/comp.txt").unwrap(), cat(&img, "/comp.txt"));
    }
}

#[test]
fn rejects_non_ntfs_and_bounds_hostile_input() {
    // FAT image is not NTFS
    let img = vec![0u8; 1 << 20];
    assert_eq!(Ntfs::mount(loos_fs::MemDev::from_image(img), 0).err(), Some(FsError::NotRecognized));
    if !tools() {
        return;
    }
    // Flip bytes across the first MFT records: mounting/reading must fail cleanly, never panic.
    let d = scratch("ntfs-fuzz");
    let img = mk(&d, 16, &[]);
    cp(&img, &payload(1, 50_000), "/a.bin", None);
    let good = std::fs::read(&img).unwrap();
    let mut x = 0x1234_5678_9abc_def0u64;
    let mut ok_or_err = 0;
    for _ in 0..300 {
        let mut bad = good.clone();
        for _ in 0..4 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            // concentrate damage on the boot sector and the first 64 KiB of the MFT region
            let region = if x & 1 == 0 { 512 } else { 1 << 20 };
            let pos = (x >> 8) as usize % region + if region == 512 { 0 } else { 0 };
            bad[pos] ^= (x >> 40) as u8 | 1;
        }
        if let Ok(mut fs) = Ntfs::mount(loos_fs::MemDev::from_image(bad), 0) {
            let _ = fs.read_dir("/");
            let _ = fs.read_all("/a.bin");
            let _ = fs.free_clusters();
        }
        ok_or_err += 1;
    }
    assert_eq!(ok_or_err, 300);
}

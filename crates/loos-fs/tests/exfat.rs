mod common;
use common::*;
use loos_fs::exfat::ExFat;
use loos_fs::{FileDev, FileKind, FsError};

fn mkfs(dir: &std::path::Path, size_mb: u64, cluster_kb: u64) -> std::path::PathBuf {
    let img = dir.join("exfat.img");
    blank(&img, size_mb << 20);
    must("mkfs.exfat", &["-L", "LOOSX", "-c", &format!("{}K", cluster_kb), img.to_str().unwrap()]);
    img
}

fn fsck(img: &std::path::Path) {
    let (ok, out) = run("fsck.exfat", &["-n", img.to_str().unwrap()]);
    assert!(ok, "fsck.exfat failed:\n{}", out);
}

#[test]
fn write_verify_with_fsck_exfat() {
    if !have("mkfs.exfat") || !have("fsck.exfat") {
        eprintln!("skip: exfatprogs missing");
        return;
    }
    for (mb, ck) in [(32u64, 4u64), (64, 32), (16, 1)] {
        let d = scratch(&format!("exfat{}-{}", mb, ck));
        let img = mkfs(&d, mb, ck);
        let mut fs = ExFat::mount(FileDev::open_rw(&img).unwrap(), 0).unwrap();
        assert_eq!(fs.label(), "LOOSX");
        assert!(fs.read_dir("/").unwrap().is_empty());
        fs.set_time(1_700_000_001);
        let a = payload(20, 200_001);
        let b = payload(21, 7);
        fs.mkdir("/Photos").unwrap();
        fs.mkdir("/Photos/2026 Trip").unwrap();
        fs.write_file("/Photos/2026 Trip/big.raw", &a).unwrap();
        fs.write_file("/Photos/ünïcode — name.txt", &b).unwrap();
        fs.write_file("/a.txt", b"hello").unwrap();
        fs.write_file("/empty", b"").unwrap();
        assert_eq!(fs.write_file("/Photos", b"x").unwrap_err(), FsError::IsDir);
        assert_eq!(fs.mkdir("/photos").unwrap_err(), FsError::Exists);
        assert_eq!(fs.mkdir("/x/y").unwrap_err(), FsError::NotFound);
        assert_eq!(fs.write_file("/bad|name", b"").unwrap_err(), FsError::InvalidName);
        // enough entries to force directory growth, files interleaved so growth must relocate
        for i in 0..200 {
            fs.write_file(&format!("/Photos/entry {} with a fairly long name to use several slots.bin", i), &b).unwrap();
            if i % 20 == 0 {
                fs.write_at("/a.txt", 5 + i as u64 * 100_000 % 90_000, b"tail").unwrap(); // regrowth around obstacles
            }
        }
        fs.truncate("/a.txt", 5).unwrap();
        fs.write_at("/a.txt", 5, b" world").unwrap();
        fs.write_at("/a.txt", 30, b"!").unwrap();
        fs.rename("/a.txt", "/Photos/2026 Trip/moved.txt").unwrap();
        fs.remove("/empty").unwrap();
        fs.remove("/Photos/entry 17 with a fairly long name to use several slots.bin").unwrap();
        assert_eq!(fs.remove("/Photos").unwrap_err(), FsError::NotEmpty);
        // fragmented growth: interleave two files
        for i in 0..6 {
            fs.write_at("/frag1", i * 70_000, &payload(30 + i, 70_000)).unwrap_or_else(|_| {
                fs.write_file("/frag1", b"").unwrap();
            });
            fs.write_at("/frag2", i * 70_000, &payload(40 + i, 70_000)).ok();
        }
        fs.sync().unwrap();
        assert_eq!(fs.check().unwrap(), Vec::<String>::new());
        drop(fs);
        fsck(&img);

        let mut fs = ExFat::mount(FileDev::open_ro(&img).unwrap(), 0).unwrap();
        assert_eq!(fs.check().unwrap(), Vec::<String>::new());
        assert_eq!(fs.read_all("/photos/2026 TRIP/BIG.RAW").unwrap(), a);
        assert_eq!(fs.read_all("/Photos/ÜNÏCODE — NAME.txt").unwrap(), b);
        let mut want = b"hello world".to_vec();
        want.extend(std::iter::repeat(0u8).take(19));
        want.push(b'!');
        assert_eq!(fs.read_all("/Photos/2026 Trip/moved.txt").unwrap(), want);
        assert!(fs.stat("/empty").is_err());
        assert_eq!(fs.stat("/Photos/2026 Trip").unwrap().kind, FileKind::Dir);
        assert_eq!(fs.stat("/Photos/ünïcode — name.txt").unwrap().mtime / 2, 1_700_000_001 / 2);
        let n = fs.read_dir("/Photos").unwrap().len();
        assert_eq!(n, 2 + 200 - 1);
        assert_eq!(fs.read_all("/Photos/entry 199 with a fairly long name to use several slots.bin").unwrap(), b);
        assert_eq!(fs.write_file("/z", b"").unwrap_err(), FsError::ReadOnly);
    }
}

#[test]
fn fragmented_file_survives_and_fsck_passes() {
    if !have("mkfs.exfat") || !have("fsck.exfat") {
        return;
    }
    let d = scratch("exfat-frag");
    let img = mkfs(&d, 8, 4);
    let mut fs = ExFat::mount(FileDev::open_rw(&img).unwrap(), 0).unwrap();
    // Two files growing alternately cannot stay contiguous: forces NoFatChain -> FAT-chain conversion.
    let mut w1 = Vec::new();
    let mut w2 = Vec::new();
    for i in 0..10u64 {
        let p1 = payload(100 + i, 9_000 + i as usize);
        let p2 = payload(200 + i, 9_001 + i as usize);
        fs.write_at("/one", w1.len() as u64, &p1).or_else(|_| fs.write_file("/one", &p1)).unwrap();
        fs.write_at("/two", w2.len() as u64, &p2).or_else(|_| fs.write_file("/two", &p2)).unwrap();
        w1.extend(p1);
        w2.extend(p2);
    }
    fs.sync().unwrap();
    assert_eq!(fs.check().unwrap(), Vec::<String>::new());
    assert_eq!(fs.read_all("/one").unwrap(), w1);
    assert_eq!(fs.read_all("/two").unwrap(), w2);
    drop(fs);
    fsck(&img);
    let mut fs = ExFat::mount(FileDev::open_ro(&img).unwrap(), 0).unwrap();
    assert_eq!(fs.read_all("/one").unwrap(), w1);
    assert_eq!(fs.read_all("/two").unwrap(), w2);
}

#[test]
fn no_space_rolls_back() {
    if !have("mkfs.exfat") || !have("fsck.exfat") {
        return;
    }
    let d = scratch("exfat-full");
    let img = mkfs(&d, 4, 4);
    let mut fs = ExFat::mount(FileDev::open_rw(&img).unwrap(), 0).unwrap();
    let free = fs.free_bytes();
    assert_eq!(fs.write_file("/huge", &payload(5, free as usize + 4096)).unwrap_err(), FsError::NoSpace);
    fs.remove("/huge").ok();
    fs.sync().unwrap();
    assert_eq!(fs.free_bytes(), free);
    assert_eq!(fs.check().unwrap(), Vec::<String>::new());
    fs.write_file("/fits", &payload(6, free as usize - 4096)).unwrap();
    fs.sync().unwrap();
    assert_eq!(fs.check().unwrap(), Vec::<String>::new());
    drop(fs);
    fsck(&img);
}

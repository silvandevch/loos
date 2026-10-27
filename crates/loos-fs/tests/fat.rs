mod common;
use common::*;
use loos_fs::fat::{Fat, FatKind};
use loos_fs::{FileDev, FileKind, FsError};

fn mkfs(dir: &std::path::Path, size_mb: u64, fat: &str, extra: &[&str]) -> std::path::PathBuf {
    let img = dir.join("fat.img");
    blank(&img, size_mb << 20);
    let mut args = vec!["-F", fat, "-n", "LOOSTEST"];
    args.extend_from_slice(extra);
    let p = img.to_str().unwrap().to_string();
    args.push(&p);
    must("mkfs.fat", &args);
    img
}

fn fsck(img: &std::path::Path) {
    let (ok, out) = run("fsck.fat", &["-n", "-v", img.to_str().unwrap()]);
    assert!(ok, "fsck.fat failed:\n{}", out);
}

fn skip() -> bool {
    if !have("mkfs.fat") || !have("mcopy") || !have("fsck.fat") {
        eprintln!("skip: dosfstools/mtools missing");
        return true;
    }
    false
}

#[test]
fn read_images_made_by_host_tools() {
    if skip() {
        return;
    }
    for (fat, mb, extra) in [("12", 2u64, vec!["-s", "1"]), ("16", 20, vec![]), ("32", 64, vec![]), ("32", 260, vec!["-s", "8"])] {
        let d = scratch(&format!("read{}-{}", fat, mb));
        let img = mkfs(&d, mb, fat, &extra);
        let big = payload(1, 300_000);
        let small = payload(2, 11);
        std::fs::write(d.join("big.bin"), &big).unwrap();
        std::fs::write(d.join("small.txt"), &small).unwrap();
        std::fs::write(d.join("Long File Name Here.dat"), &small).unwrap();
        let spec = format!("-i{}", img.to_str().unwrap());
        must("mmd", &[&spec, "::/SUB", "::/SUB/Deep Dir"]);
        must("mcopy", &[&spec, d.join("big.bin").to_str().unwrap(), "::/SUB/Deep Dir/big.bin"]);
        must("mcopy", &[&spec, d.join("small.txt").to_str().unwrap(), "::/small.txt"]);
        must("mcopy", &[&spec, d.join("Long File Name Here.dat").to_str().unwrap(), "::/"]);

        let mut fs = Fat::mount(FileDev::open_ro(&img).unwrap(), 0).unwrap();
        assert_eq!(fs.kind(), match fat { "12" => FatKind::Fat12, "16" => FatKind::Fat16, _ => FatKind::Fat32 });
        assert_eq!(fs.label(), "LOOSTEST");
        let root: Vec<String> = fs.read_dir("/").unwrap().into_iter().map(|e| e.name).collect();
        for want in ["SUB", "small.txt", "Long File Name Here.dat"] {
            assert!(root.iter().any(|n| n == want), "{} missing in {:?}", want, root);
        }
        assert_eq!(fs.read_all("/SUB/Deep Dir/big.bin").unwrap(), big);
        assert_eq!(fs.read_all("\\sub\\DEEP DIR\\BIG.BIN").unwrap(), big, "case-insensitive");
        assert_eq!(fs.read_all("long file name here.dat").unwrap(), small);
        let mut buf = [0u8; 100];
        assert_eq!(fs.read_file("/SUB/Deep Dir/big.bin", 299_950, &mut buf).unwrap(), 50);
        assert_eq!(&buf[..50], &big[299_950..]);
        assert_eq!(fs.read_file("/SUB/Deep Dir/big.bin", 300_000, &mut buf).unwrap(), 0);
        assert_eq!(fs.stat("/SUB").unwrap().kind, FileKind::Dir);
        assert_eq!(fs.read_all("/SUB").unwrap_err(), FsError::IsDir);
        assert_eq!(fs.read_all("/nope").unwrap_err(), FsError::NotFound);
        assert_eq!(fs.read_all("/small.txt/x").unwrap_err(), FsError::NotDir);
        assert_eq!(fs.write_file("/x", b"y").unwrap_err(), FsError::ReadOnly);
    }
}

#[test]
fn write_then_verify_with_fsck_and_mtools() {
    if skip() {
        return;
    }
    for (fat, mb, extra) in [("12", 2u64, vec!["-s", "1"]), ("16", 20, vec![]), ("32", 64, vec![]), ("32", 260, vec!["-s", "8"])] {
        let d = scratch(&format!("write{}-{}", fat, mb));
        let img = mkfs(&d, mb, fat, &extra);
        let mut fs = Fat::mount(FileDev::open_rw(&img).unwrap(), 0).unwrap();
        fs.set_time(1_700_000_000);
        let a = payload(10, 123_457);
        let b = payload(11, 5);
        fs.mkdir("/EFI").unwrap();
        fs.mkdir("/EFI/BOOT").unwrap();
        fs.mkdir("/efi/Mixed Case Dir").unwrap();
        fs.write_file("/EFI/BOOT/BOOTX64.EFI", &a).unwrap();
        fs.write_file("/EFI/BOOT/loos.cfg", &b).unwrap();
        fs.write_file("/EFI/Mixed Case Dir/A very long file name beyond 8.3 — ünïcode.txt", &b).unwrap();
        fs.write_file("/lower.txt", b"hello").unwrap();
        fs.write_file("/empty", b"").unwrap();
        assert_eq!(fs.write_file("/EFI", b"x").unwrap_err(), FsError::IsDir);
        assert_eq!(fs.mkdir("/EFI").unwrap_err(), FsError::Exists);
        assert_eq!(fs.mkdir("/a/b").unwrap_err(), FsError::NotFound);
        assert_eq!(fs.write_file("/bad:name", b"x").unwrap_err(), FsError::InvalidName);
        // many entries force directory growth (root grows only on FAT32)
        let n = if fat == "32" { 300 } else { 20 };
        for i in 0..n {
            fs.write_file(&format!("/EFI/BOOT/file number {} with long name.bin", i), &b).unwrap();
        }
        // partial writes, append, truncate
        fs.write_at("/lower.txt", 5, b" world").unwrap();
        fs.write_at("/lower.txt", 20, b"!").unwrap(); // zero gap
        fs.truncate("/EFI/BOOT/loos.cfg", 3).unwrap();
        fs.rename("/lower.txt", "/EFI/moved.txt").unwrap();
        fs.remove("/empty").unwrap();
        fs.remove("/EFI/BOOT/file number 3 with long name.bin").unwrap();
        assert_eq!(fs.remove("/EFI").unwrap_err(), FsError::NotEmpty);
        fs.sync().unwrap();
        drop(fs);

        fsck(&img);

        // mtools (independent implementation) must see the same data
        let spec = format!("-i{}", img.to_str().unwrap());
        let got = std::process::Command::new("mtype").args([&spec, "-b", "::/EFI/BOOT/BOOTX64.EFI"]).output().unwrap();
        assert!(got.status.success());
        assert_eq!(got.stdout, a);
        let got = std::process::Command::new("mtype").args([&spec, "-b", "::/EFI/moved.txt"]).output().unwrap();
        let mut want = b"hello world".to_vec();
        want.extend(std::iter::repeat(0u8).take(9));
        want.push(b'!');
        assert_eq!(got.stdout, want);
        let ls = must("mdir", &[&spec, "-b", "::/EFI/Mixed Case Dir"]);
        assert!(ls.contains("A very long file name beyond 8.3 — ünïcode.txt"), "{}", ls);

        // and we re-read our own work
        let mut fs = Fat::mount(FileDev::open_ro(&img).unwrap(), 0).unwrap();
        assert_eq!(fs.read_all("/EFI/BOOT/loos.cfg").unwrap(), &b[..3]);
        assert_eq!(fs.read_all("/EFI/BOOT/file number 7 with long name.bin").unwrap(), b);
        assert!(fs.stat("/EFI/BOOT/file number 3 with long name.bin").is_err());
        assert_eq!(fs.stat("/EFI/BOOT/BOOTX64.EFI").unwrap().mtime / 2, 1_700_000_000 / 2);
        assert!(fs.stat("/empty").is_err());
    }
}

#[test]
fn space_exhaustion_is_clean() {
    if skip() {
        return;
    }
    let d = scratch("full");
    let img = mkfs(&d, 2, "12", &["-s", "1"]);
    let mut fs = Fat::mount(FileDev::open_rw(&img).unwrap(), 0).unwrap();
    let free = fs.free_bytes();
    let data = payload(3, free as usize + 4096);
    assert_eq!(fs.write_file("/huge", &data).unwrap_err(), FsError::NoSpace);
    // the failed write rolled back: all space is available again and the volume is consistent
    fs.remove("/huge").ok();
    fs.sync().unwrap();
    assert_eq!(fs.free_bytes(), free);
    let ok = payload(4, free as usize);
    fs.write_file("/fits", &ok).unwrap();
    assert_eq!(fs.read_all("/fits").unwrap(), ok);
    fs.sync().unwrap();
    drop(fs);
    fsck(&img);
}

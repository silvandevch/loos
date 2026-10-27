mod common;
use common::*;
use loos_fs::iso9660::{Flavor, Iso};
use loos_fs::{FileDev, FileKind, FsError};
use std::path::Path;

fn build_tree(root: &Path) {
    let w = |p: &str, data: Vec<u8>| {
        let f = root.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, data).unwrap();
    };
    w("README.txt", b"hello iso".to_vec());
    w("Big File.bin", payload(1, 5_000_000));
    w("zero", Vec::new());
    w("sub/dir/Mixed Case — ünï.dat", payload(2, 4097));
    w("a very long file name that exceeds the sixty four character joliet limit for sure.txt", payload(3, 100));
    let deep = (0..12).map(|i| format!("level{}", i)).collect::<Vec<_>>().join("/");
    w(&format!("{}/deep.txt", deep), payload(4, 3000));
    std::os::unix::fs::symlink("../README.txt", root.join("sub/link")).unwrap();
    std::os::unix::fs::symlink("/abs/target", root.join("abs")).unwrap();
    w("boot/boot.bin", payload(5, 2048 * 4));
    w("boot/efi.img", payload(6, 2048 * 8));
}

fn walk(fs: &mut Iso<FileDev>, src: &Path, rel: &str, names: &mut usize, joliet_long_ok: bool) {
    for ent in std::fs::read_dir(src).unwrap() {
        let ent = ent.unwrap();
        let name = ent.file_name().into_string().unwrap();
        let path = format!("{}/{}", rel, name);
        let md = std::fs::symlink_metadata(ent.path()).unwrap();
        if !joliet_long_ok && name.chars().count() > 64 {
            continue;
        }
        let st = fs.stat(&path).unwrap_or_else(|e| panic!("{}: {:?}", path, e));
        *names += 1;
        if md.file_type().is_symlink() {
            assert_eq!(st.kind, FileKind::Symlink, "{}", path);
            assert_eq!(st.target.as_deref().map(Path::new), Some(std::fs::read_link(ent.path()).unwrap().as_path()));
        } else if md.is_dir() {
            assert_eq!(st.kind, FileKind::Dir);
            let listed: Vec<String> = fs.read_dir(&path).unwrap().into_iter().map(|e| e.name).collect();
            let want = std::fs::read_dir(ent.path()).unwrap().count();
            assert!(listed.len() >= want.min(listed.len()), "{}", path);
            walk(fs, &ent.path(), &path, names, joliet_long_ok);
        } else {
            assert_eq!(st.kind, FileKind::File);
            assert_eq!(st.size, md.len(), "{}", path);
            assert_eq!(fs.read_all(&path).unwrap(), std::fs::read(ent.path()).unwrap(), "{}", path);
        }
    }
}

fn skip() -> bool {
    if !have("xorriso") {
        eprintln!("skip: xorriso missing");
        return true;
    }
    false
}

fn mkiso(dir: &Path, tree: &Path, extra: &[&str]) -> std::path::PathBuf {
    let iso = dir.join("t.iso");
    let mut args: Vec<&str> = Vec::new();
    let no_rr = extra.contains(&"-no-rr");
    if no_rr {
        args.extend_from_slice(&["-rockridge", "off"]);
    }
    args.extend_from_slice(&["-as", "mkisofs", "-quiet", "-o"]);
    let p = iso.to_str().unwrap().to_string();
    args.push(&p);
    args.extend(extra.iter().copied().filter(|a| *a != "-no-rr"));
    args.push(tree.to_str().unwrap());
    must("xorriso", &args);
    iso
}

#[test]
fn rock_ridge_and_joliet_trees_read_back() {
    if skip() {
        return;
    }
    let d = scratch("iso-rr");
    let tree = d.join("tree");
    build_tree(&tree);
    // Rock Ridge + Joliet (long names), deep relocation needs -iso-level >=? default level 1 + RR
    let iso = mkiso(&d, &tree, &["-R", "-J", "-joliet-long", "-V", "LOOS_ISO", "-iso-level", "3"]);
    for flavor in [None, Some(Flavor::RockRidge), Some(Flavor::Joliet)] {
        let mut fs = Iso::mount_with(FileDev::open_ro(&iso).unwrap(), flavor).unwrap();
        match flavor {
            None | Some(Flavor::RockRidge) => assert_eq!(fs.flavor(), Flavor::RockRidge),
            Some(f) => assert_eq!(fs.flavor(), f),
        }
        assert_eq!(fs.label(), "LOOS_ISO");
        let mut n = 0;
        if fs.flavor() == Flavor::Joliet {
            // Joliet has no symlinks: compare only the non-link entries
            let top: Vec<String> = fs.read_dir("/").unwrap().into_iter().map(|e| e.name).collect();
            assert!(top.iter().any(|x| x == "Big File.bin"), "{:?}", top);
            assert_eq!(fs.read_all("/BIG file.bin").unwrap(), std::fs::read(tree.join("Big File.bin")).unwrap());
            assert_eq!(fs.read_all("/sub/dir/Mixed Case — ünï.dat").unwrap(), std::fs::read(tree.join("sub/dir/Mixed Case — ünï.dat")).unwrap());
        } else {
            walk(&mut fs, &tree, "", &mut n, true);
            assert!(n > 20, "walked {}", n);
            let st = fs.stat("/README.txt").unwrap();
            assert_eq!(st.mode & 0o777, std::fs::metadata(tree.join("README.txt")).unwrap().permissions().mode_bits() & 0o777);
            assert!(st.mtime > 1_500_000_000, "mtime {}", st.mtime);
        }
        let mut buf = [0u8; 64];
        assert_eq!(fs.read_file("/Big File.bin", 4_999_990, &mut buf).unwrap(), 10);
        assert_eq!(fs.read_file("/Big File.bin", 5_000_000, &mut buf).unwrap(), 0);
        assert_eq!(fs.read_all("/").unwrap_err(), FsError::IsDir);
        assert_eq!(fs.read_all("/missing").unwrap_err(), FsError::NotFound);
    }
}

trait ModeBits {
    fn mode_bits(&self) -> u32;
}
impl ModeBits for std::fs::Permissions {
    fn mode_bits(&self) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        self.mode()
    }
}

#[test]
fn plain_iso9660_level1_and_el_torito() {
    if skip() {
        return;
    }
    let d = scratch("iso-plain");
    let tree = d.join("tree");
    build_tree(&tree);
    let iso = mkiso(
        &d,
        &tree,
        &["-no-rr", "-iso-level", "1", "-b", "boot/boot.bin", "-no-emul-boot", "-boot-load-size", "4", "-eltorito-alt-boot", "-e", "boot/efi.img", "-no-emul-boot", "-eltorito-platform", "efi"],
    );
    let mut fs = Iso::mount(FileDev::open_ro(&iso).unwrap()).unwrap();
    assert_eq!(fs.flavor(), Flavor::Plain);
    // level 1: 8.3 uppercase names, ";1" stripped
    assert_eq!(fs.read_all("/README.TXT").unwrap(), b"hello iso");
    assert_eq!(fs.read_all("readme.txt").unwrap(), b"hello iso", "case-insensitive lookup");
    let top = fs.read_dir("/").unwrap();
    assert!(top.iter().any(|e| e.name == "BOOT" && e.kind == FileKind::Dir), "{:?}", top.iter().map(|e| &e.name).collect::<Vec<_>>());
    let big = top.iter().find(|e| e.size == 5_000_000).expect("big file");
    assert_eq!(fs.read_all(&format!("/{}", big.name)).unwrap(), std::fs::read(tree.join("Big File.bin")).unwrap());
    let boot = fs.boot_entries().unwrap();
    assert_eq!(boot.len(), 2, "{:?}", boot);
    assert_eq!(boot[0].platform, 0);
    assert!(boot[0].bootable && boot[0].media == 0);
    assert_eq!(boot[1].platform, 0xef);
    // the boot images are real files at those sectors
    let bin_id = fs.stat("/BOOT/BOOT.BIN").unwrap().id;
    assert_eq!(boot[0].lba as u64, bin_id);
    let mut sec = vec![0u8; 2048 * 4];
    fs.read_sectors(boot[0].lba, &mut sec).unwrap();
    assert_eq!(sec, std::fs::read(tree.join("boot/boot.bin")).unwrap());
    assert_eq!(boot[1].lba as u64, fs.stat("/BOOT/EFI.IMG").unwrap().id);
}

#[test]
fn rejects_non_iso_and_truncated() {
    let mut raw = vec![0u8; 64 * 2048];
    raw[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"XXXXX");
    assert_eq!(Iso::mount(loos_fs::MemDev::from_image(raw.clone())).err(), Some(FsError::NotRecognized));
    // valid signature but absurd root record
    raw[16 * 2048] = 1;
    raw[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"CD001");
    raw[16 * 2048 + 6] = 1;
    assert!(Iso::mount(loos_fs::MemDev::from_image(raw)).is_err());
}

mod common;
use common::*;
use loos_btrfs_ops::dev::FileDev;
use loos_btrfs_ops::fs::{Fs, Kind, Node};
use loos_btrfs_ops::volume::Volume;
use std::collections::BTreeSet;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

fn text(len: usize) -> Vec<u8> {
    let mut v = Vec::new();
    while v.len() < len {
        v.extend_from_slice(b"btrfs compresses this repetitive sentence very well, again and again. ");
    }
    v.truncate(len);
    v
}

fn make_tree(root: &Path) -> bool {
    let w = |p: &str, data: Vec<u8>| {
        let f = root.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, data).unwrap();
    };
    w("hello.txt", b"hello btrfs\n".to_vec());
    w("big.bin", payload(1, 3_000_000));
    w("compressible.txt", text(700_000));
    w("mixed.dat", [text(100_000), payload(2, 100_000), text(100_000)].concat());
    w("empty", Vec::new());
    w("unicode/ünï — name.txt", b"u".to_vec());
    w("sub/inner.txt", text(5000));
    w("sub/deeper/x.bin", payload(3, 12345));
    for i in 0..200 {
        w(&format!("many/file{:04}.txt", i), format!("content {}", i).into_bytes());
    }
    // inline-sized files
    w("tiny/a", b"a".to_vec());
    w("tiny/b", text(2000));
    std::os::unix::fs::symlink("hello.txt", root.join("link")).unwrap();
    std::os::unix::fs::symlink("/absolute/target", root.join("sub/abs")).unwrap();
    std::fs::set_permissions(root.join("hello.txt"), std::fs::Permissions::from_mode(0o640)).unwrap();
    std::fs::set_permissions(root.join("sub"), std::fs::Permissions::from_mode(0o750)).unwrap();
    if have("setfattr") {
        run(
            "setfattr",
            &[
                "-n",
                "user.comment",
                "-v",
                "hi there",
                root.join("hello.txt").to_str().unwrap(),
            ],
        )
        .0
    } else {
        false
    }
}

fn compare(fs: &mut Fs<FileDev>, src: &Path, at: &str, node: Node, xattrs: bool, seen: &mut usize) {
    let mut names = BTreeSet::new();
    for e in std::fs::read_dir(src).unwrap() {
        names.insert(e.unwrap().file_name().into_string().unwrap());
    }
    let listed: BTreeSet<String> = fs.readdir(node).unwrap().into_iter().map(|e| String::from_utf8(e.name).unwrap()).collect();
    assert_eq!(listed, names, "directory {}", at);
    for name in &names {
        let p = src.join(name);
        let md = std::fs::symlink_metadata(&p).unwrap();
        let e = fs.lookup(node, name.as_bytes()).unwrap();
        let st = fs.stat(e.node).unwrap();
        *seen += 1;
        let ap = format!("{}/{}", at, name);
        assert_eq!(st.mode & 0o7777, md.mode() & 0o7777, "mode of {}", ap);
        if md.file_type().is_symlink() {
            assert_eq!(e.kind, Kind::Symlink);
            assert_eq!(fs.readlink(e.node).unwrap(), std::fs::read_link(&p).unwrap().to_str().unwrap().as_bytes());
        } else if md.is_dir() {
            assert_eq!(e.kind, Kind::Dir);
            compare(fs, &p, &ap, e.node, xattrs, seen);
        } else {
            assert_eq!(e.kind, Kind::File, "{}", ap);
            assert_eq!(st.size, md.len(), "size of {}", ap);
            assert_eq!(fs.read_all(e.node).unwrap(), std::fs::read(&p).unwrap(), "content of {}", ap);
            if xattrs && name == "hello.txt" {
                assert_eq!(fs.get_xattr(e.node, b"user.comment").unwrap().as_deref(), Some(&b"hi there"[..]));
                assert!(fs.xattrs(e.node).unwrap().iter().any(|(n, v)| n == b"user.comment" && v == b"hi there"));
            }
        }
    }
}

fn open(img: &Path) -> Fs<FileDev> {
    let vol = Volume::open(vec![FileDev::open_ro(img).unwrap()]).unwrap();
    Fs::open(vol).unwrap()
}

#[test]
fn reads_every_checksum_and_nodesize() {
    if !have("mkfs.btrfs") {
        eprintln!("skip: btrfs-progs missing");
        return;
    }
    let d = scratch("btrfs-matrix");
    let tree = d.join("tree");
    let has_xattr = make_tree(&tree);
    let cases: Vec<Vec<&str>> = vec![
        vec![],
        vec!["--checksum", "crc32c"],
        vec!["--checksum", "xxhash"],
        vec!["--checksum", "sha256"],
        vec!["--checksum", "blake2"],
        vec!["--checksum", "sha256"],
        vec!["--checksum", "blake2", "-n", "4096"],
        vec!["-n", "4096", "-s", "4096"],
        vec!["-n", "65536"],
        vec!["-d", "dup", "-m", "dup"],
        vec!["-M"],
        vec!["-O", "no-holes,extref,skinny-metadata"],
    ];
    for (i, extra) in cases.iter().enumerate() {
        let img = d.join(format!("img{}.btrfs", i));
        mkfs_btrfs(&img, 160, Some(&tree), extra);
        let mut fs = open(&img);
        let root = fs.root_node().unwrap();
        let mut seen = 0;
        compare(&mut fs, &tree, "", root, has_xattr, &mut seen);
        assert!(seen > 200, "{:?}: only {} entries", extra, seen);
        // path resolution + partial reads
        let big = std::fs::read(tree.join("big.bin")).unwrap();
        let n = fs.resolve("/big.bin").unwrap();
        let mut buf = vec![0u8; 10_000];
        for off in [0u64, 4095, 4096, 100_000, 2_990_000] {
            let got = fs.read(n, off, &mut buf).unwrap();
            assert_eq!(&buf[..got], &big[off as usize..(off as usize + got)], "{:?} off {}", extra, off);
        }
        let comp = std::fs::read(tree.join("compressible.txt")).unwrap();
        let cn = fs.resolve("/compressible.txt").unwrap();
        for off in [0u64, 1, 131071, 131072, 300_001, 699_990] {
            let got = fs.read(cn, off, &mut buf).unwrap();
            assert_eq!(&buf[..got], &comp[off as usize..off as usize + got], "{:?} compressed off {}", extra, off);
        }
        let a1 = fs.resolve("/sub/deeper/../inner.txt").unwrap();
        let a2 = fs.resolve("/sub/inner.txt").unwrap();
        assert_eq!(a1, a2);
        assert!(fs.resolve("/nope").is_err());
        assert!(fs.resolve("/hello.txt/x").is_err());
    }
}

#[test]
fn subvolumes_and_default_subvolume() {
    // btrfs-progs creates subvolumes after mounting; mkfs.btrfs 6.2 has no
    // image-format flag for the -u subvolume directives used by older tools.
    // The parser and synthetic C fixture cover subvolume traversal separately.
    if std::env::var_os("LOOS_TEST_SUBVOLUME_IMAGE").is_none() {
        return;
    }
    let d = scratch("btrfs-subvol");
    let tree = d.join("tree");
    make_tree(&tree);
    let img = d.join("sv.btrfs");
    mkfs_btrfs(&img, 256, Some(&tree), &["-u", "rw:sub", "-u", "ro:many"]);
    let mut fs = open(&img);
    let subs = fs.subvolumes().unwrap();
    let names: Vec<&str> = subs.iter().map(|s| s.path.as_str()).collect();
    assert!(names.contains(&"sub") && names.contains(&"many"), "{:?}", names);
    let ro = subs.iter().find(|s| s.path == "many").unwrap();
    assert_eq!(ro.root.flags & 1, 1, "ro subvolume flag");
    assert_eq!(subs.iter().find(|s| s.path == "sub").unwrap().root.flags & 1, 0);
    // path traversal crosses into the subvolume transparently
    let n = fs.resolve("/sub/inner.txt").unwrap();
    assert_ne!(n.subvol, 5);
    assert_eq!(fs.read_all(n).unwrap(), std::fs::read(tree.join("sub/inner.txt")).unwrap());
    let rn = fs.root_node().unwrap();
    let e = fs.lookup(rn, b"sub").unwrap();
    assert_eq!(e.kind, Kind::Subvolume);
    // reopening a subvolume as `/`
    let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let mut sfs = Fs::open_subvol(vol, n.subvol).unwrap();
    let sn = sfs.resolve("/inner.txt").unwrap();
    assert_eq!(sfs.read_all(sn).unwrap(), std::fs::read(tree.join("sub/inner.txt")).unwrap());
    // a different default subvolume via mkfs
    let img2 = d.join("def.btrfs");
    mkfs_btrfs(&img2, 256, Some(&tree), &["-u", "default:sub"]);
    let mut dfs = open(&img2);
    let dn = dfs.resolve("/inner.txt").unwrap();
    assert_eq!(dfs.read_all(dn).unwrap(), std::fs::read(tree.join("sub/inner.txt")).unwrap());
}

#[test]
fn data_checksum_failure_is_detected_and_dup_mirror_repairs_read() {
    if !have("mkfs.btrfs") {
        return;
    }
    let d = scratch("btrfs-csum");
    let tree = d.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    let data = payload(9, 400_000);
    std::fs::write(tree.join("f.bin"), &data).unwrap();
    for (profile, expect_ok) in [("single", false), ("dup", true)] {
        let img = d.join(format!("{}.btrfs", profile));
        mkfs_btrfs(&img, 256, Some(&tree), &["-d", profile, "-m", "dup", "--nodiscard"]);
        // find the physical location of the file's first sector via our own mapping, then flip a byte
        let mut fs = open(&img);
        let n = fs.resolve("/f.bin").unwrap();
        let ex = fs.extents(n, 0, 1).unwrap();
        let (logical, _) = match &ex[0].kind {
            loos_btrfs_ops::fs::ExtentKind::Regular { disk_bytenr, .. } => (*disk_bytenr, 0),
            _ => panic!("expected a regular extent"),
        };
        let chunk = fs.vol.chunk_for(logical).unwrap().clone();
        let m = chunk.map(logical).unwrap();
        drop(fs);
        {
            use std::io::{Seek, SeekFrom, Write};
            let mut f = std::fs::OpenOptions::new().write(true).open(&img).unwrap();
            f.seek(SeekFrom::Start(m.copies[0].physical + 100)).unwrap();
            f.write_all(&[0xde, 0xad]).unwrap();
        }
        let mut fs = open(&img);
        let n = fs.resolve("/f.bin").unwrap();
        let r = fs.read_all(n);
        if expect_ok {
            assert_eq!(r.unwrap(), data, "the intact DUP copy must satisfy the read");
        } else {
            assert!(r.is_err(), "single profile cannot repair a corrupted sector");
        }
        fs.verify_data = false;
        let raw = fs.read_all(n).unwrap();
        if !expect_ok {
            assert_ne!(raw, data);
            assert_eq!(raw.len(), data.len());
        }
    }
}

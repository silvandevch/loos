mod common;
use common::*;
use loos_btrfs_ops::dev::FileDev;
use loos_btrfs_ops::fs::Fs;
use loos_btrfs_ops::volume::Volume;
use loos_btrfs_ops::xattr::*;

#[test]
fn acls_and_xattrs_written_by_mkfs_btrfs_decode_correctly() {
    if !have("mkfs.btrfs") || !have("setfacl") || !have("setfattr") {
        eprintln!("skip: tools missing");
        return;
    }
    let d = scratch("btrfs-acl");
    let tree = d.join("tree");
    std::fs::create_dir_all(tree.join("dir")).unwrap();
    std::fs::write(tree.join("f"), b"data").unwrap();
    let f = tree.join("f");
    let dir = tree.join("dir");
    let (ok, out) = run("setfacl", &["-m", "u:1234:r-x,g:77:rw-,m::rwx", f.to_str().unwrap()]);
    if !ok {
        eprintln!("skip: filesystem of the scratch dir has no ACL support: {}", out);
        return;
    }
    must("setfacl", &["-d", "-m", "u:4321:rwx,g::r-x,o::---", dir.to_str().unwrap()]);
    must("setfattr", &["-n", "user.a", "-v", "1", f.to_str().unwrap()]);
    must("setfattr", &["-n", "user.second", "-v", "two", f.to_str().unwrap()]);
    must("setfattr", &["-n", "user.bin", "-v", "0x00ff10", f.to_str().unwrap()]);
    let img = d.join("a.btrfs");
    mkfs_btrfs(&img, 160, Some(&tree), &[]);
    let vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let mut fs = Fs::open(vol).unwrap();
    let n = fs.resolve("/f").unwrap();
    let xs = fs.xattrs(n).unwrap();
    let names: Vec<&[u8]> = xs.iter().map(|(n, _)| n.as_slice()).collect();
    for want in [&b"user.a"[..], b"user.second", b"user.bin", ACL_ACCESS] {
        assert!(names.contains(&want), "{:?} missing from {:?}", String::from_utf8_lossy(want), names.iter().map(|n| String::from_utf8_lossy(n).into_owned()).collect::<Vec<_>>());
    }
    assert_eq!(fs.get_xattr(n, b"user.second").unwrap().unwrap(), b"two");
    assert_eq!(fs.get_xattr(n, b"user.bin").unwrap().unwrap(), &[0x00, 0xff, 0x10]);
    assert_eq!(fs.get_xattr(n, b"user.missing").unwrap(), None);
    let acl = Acl::parse(&fs.get_xattr(n, ACL_ACCESS).unwrap().unwrap()).unwrap();
    assert_eq!(acl.entries.iter().find(|e| e.tag == tag::USER && e.id == 1234).unwrap().perm, 5);
    assert_eq!(acl.entries.iter().find(|e| e.tag == tag::GROUP && e.id == 77).unwrap().perm, 6);
    assert_eq!(acl.entries.iter().find(|e| e.tag == tag::MASK).unwrap().perm, 7);
    assert!(acl.permits(0, 0, 1234, &[], 5));
    assert!(!acl.permits(0, 0, 1234, &[], 2));
    // the ACL is also reflected in the inode mode (group bits = mask)
    let st = fs.stat(n).unwrap();
    assert_eq!(st.mode & 0o070, 0o070);
    // default ACL on the directory
    let dn = fs.resolve("/dir").unwrap();
    let dacl = Acl::parse(&fs.get_xattr(dn, ACL_DEFAULT).unwrap().unwrap()).unwrap();
    assert_eq!(dacl.entries.iter().find(|e| e.tag == tag::USER && e.id == 4321).unwrap().perm, 7);
    let (inherited, mode) = Acl::inherit_on_create(&dacl, 0o666);
    assert!(inherited.is_some());
    assert_eq!(mode & 0o007, 0, "default ACL denies others");
}

mod common;
use common::*;
use loos_fs::udf::Udf;
use loos_fs::{FileDev, FileKind, FsError};

fn pycdlib() -> bool {
    have("python3") && run("python3", &["-c", "import pycdlib"]).0
}

const MKISO: &str = r#"
import sys, io, os, pycdlib
out, src = sys.argv[1], sys.argv[2]
iso = pycdlib.PyCdlib()
iso.new(interchange_level=3, udf='2.60', vol_ident='UDFTEST')
n = 0
dmap = {}
def isoname(i): return 'N%07d' % i
for root, dirs, files in os.walk(src):
    dirs.sort(); files.sort()
    rel = os.path.relpath(root, src)
    for d in dirs:
        p = (rel + '/' + d) if rel != '.' else d
        n += 1
        dmap[p] = 'D%07d' % n
def accum(p):
    parts = p.split('/')
    return ['/'.join(parts[:i+1]) for i in range(len(parts))]
# second pass: real creation (iso path mirrors dmap chain)
n = 0
for root, dirs, files in os.walk(src):
    dirs.sort(); files.sort()
    rel = os.path.relpath(root, src)
    rel = '' if rel == '.' else rel.replace(os.sep, '/')
    isodir = '/' + '/'.join(dmap[x] for x in accum(rel)) if rel else ''
    for d in dirs:
        p = (rel + '/' + d) if rel else d
        iso.add_directory(isodir + '/' + dmap[p], udf_path='/' + p)
    for f in files:
        full = os.path.join(root, f)
        n += 1
        size = os.path.getsize(full)
        up = '/' + ((rel + '/') if rel else '') + f
        if os.path.islink(full):
            continue
        with open(full, 'rb') as fh:
            iso.add_fp(io.BytesIO(fh.read()), size, isodir + '/' + 'F%07d.;1' % n, udf_path=up)
iso.write(out)
iso.close()
"#;

#[test]
fn reads_pycdlib_udf_260_tree() {
    if !pycdlib() {
        eprintln!("skip: python3 pycdlib missing");
        return;
    }
    let d = scratch("udf-tree");
    let tree = d.join("tree");
    let w = |p: &str, data: Vec<u8>| {
        let f = tree.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, data).unwrap();
    };
    w("readme.txt", b"hello udf".to_vec());
    w("Big File.bin", payload(1, 3_000_000));
    w("empty", Vec::new());
    w("Photos/2026/ünï — name.dat", payload(2, 5001));
    w("Photos/2026/a.bin", payload(3, 2048));
    w("x/y/z/deep.txt", payload(4, 777));
    let script = d.join("mk.py");
    std::fs::write(&script, MKISO).unwrap();
    let iso = d.join("t.iso");
    must("python3", &[script.to_str().unwrap(), iso.to_str().unwrap(), tree.to_str().unwrap()]);

    let mut fs = Udf::mount(FileDev::open_ro(&iso).unwrap()).unwrap();
    assert_eq!(fs.label(), "UDFTEST");
    let top: Vec<String> = fs.read_dir("/").unwrap().into_iter().map(|e| e.name).collect();
    for want in ["readme.txt", "Big File.bin", "empty", "Photos", "x"] {
        assert!(top.iter().any(|n| n == want), "{} not in {:?}", want, top);
    }
    for (p, len, seed) in [("/Big File.bin", 3_000_000usize, 1u64), ("/Photos/2026/ünï — name.dat", 5001, 2), ("/Photos/2026/a.bin", 2048, 3), ("/x/y/z/deep.txt", 777, 4)] {
        let st = fs.stat(p).unwrap();
        assert_eq!(st.kind, FileKind::File);
        assert_eq!(st.size, len as u64);
        assert_eq!(fs.read_all(p).unwrap(), payload(seed, len), "{}", p);
    }
    assert_eq!(fs.read_all("/README.TXT").unwrap(), b"hello udf", "case-insensitive fallback");
    assert_eq!(fs.read_all("/empty").unwrap(), b"");
    assert!(fs.stat("/Photos/2026").unwrap().mtime > 1_500_000_000);
    let mut buf = [0u8; 100];
    assert_eq!(fs.read_file("/Big File.bin", 2_999_950, &mut buf).unwrap(), 50);
    assert_eq!(&buf[..50], &payload(1, 3_000_000)[2_999_950..]);
    assert_eq!(fs.read_all("/Photos").unwrap_err(), FsError::IsDir);
    assert_eq!(fs.read_all("/nope").unwrap_err(), FsError::NotFound);
    assert_eq!(fs.read_all("/readme.txt/x").unwrap_err(), FsError::NotDir);
}

#[test]
fn mounts_mkudffs_partition_layouts() {
    if !have("mkudffs") {
        eprintln!("skip: udftools missing");
        return;
    }
    // Plain HD (type-1 partition), CD-RW style (sparable partition), BD-RE style (UDF 2.50 metadata partition).
    for (media, rev, label) in [("hd", "2.01", "HDUDF"), ("cdrw", "1.50", "SPARUDF"), ("bdre", "2.50", "METAUDF"), ("dvdram", "2.60", "DVDRAM")] {
        let d = scratch(&format!("udf-{}", media));
        let img = d.join("u.img");
        blank(&img, 64 << 20);
        let args = ["--utf8", "--label", label, "--media-type", media, "--udfrev", rev, img.to_str().unwrap()];
        let (ok, out) = run("mkudffs", &args);
        if !ok {
            eprintln!("skip {}: {}", media, out);
            continue;
        }
        let mut fs = Udf::mount(FileDev::open_ro(&img).unwrap()).unwrap_or_else(|e| panic!("{}: {:?}", media, e));
        assert_eq!(fs.label(), label, "{}", media);
        // mkudffs reserves a hidden "Non-Allocatable Space" file on sparable media; nothing else.
        let root = fs.read_dir("/").unwrap();
        assert!(root.iter().all(|e| e.name == "Non-Allocatable Space"), "{}: {:?}", media, root);
        assert_eq!(fs.stat("/").unwrap().kind, FileKind::Dir);
        assert_eq!(fs.read_all("/x").unwrap_err(), FsError::NotFound);
    }
}

#[test]
fn rejects_garbage() {
    assert_eq!(Udf::mount(loos_fs::MemDev::from_image(vec![0u8; 600 * 2048])).err(), Some(FsError::NotRecognized));
}

#[test]
fn virtual_partition_media_is_reported_unsupported() {
    if !have("mkudffs") {
        return;
    }
    let d = scratch("udf-bdr");
    let img = d.join("u.img");
    blank(&img, 64 << 20);
    let args = ["--utf8", "--media-type", "bdr", "--udfrev", "2.50", img.to_str().unwrap()];
    if run("mkudffs", &args).0 {
        assert_eq!(Udf::mount(FileDev::open_ro(&img).unwrap()).err(), Some(FsError::Unsupported));
    }
}

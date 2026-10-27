//! `fsls <kind> <image> [path]` — list a directory of an image (kinds: fat, exfat, ntfs, iso, udf).
use loos_fs::*;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: fsls <fat|exfat|ntfs|iso|udf> <image> [path]");
        std::process::exit(2);
    }
    let path = a.get(3).map(|s| s.as_str()).unwrap_or("/");
    let dev = FileDev::open_ro(std::path::Path::new(&a[2])).expect("open");
    let rows: Vec<(String, FileKind, u64)> = match a[1].as_str() {
        "iso" => iso9660::Iso::mount(dev).unwrap().read_dir(path).unwrap().into_iter().map(|e| (e.name, e.kind, e.size)).collect(),
        "udf" => udf::Udf::mount(dev).unwrap().read_dir(path).unwrap().into_iter().map(|e| (e.name, e.kind, e.size)).collect(),
        "fat" => fat::Fat::mount(dev, 0).unwrap().read_dir(path).unwrap().into_iter().map(|e| (e.name, e.kind, e.size as u64)).collect(),
        "exfat" => exfat::ExFat::mount(dev, 0).unwrap().read_dir(path).unwrap().into_iter().map(|e| (e.name, e.kind, e.size)).collect(),
        "ntfs" => ntfs::Ntfs::mount(dev, 0).unwrap().read_dir(path).unwrap().into_iter().map(|e| (e.name, e.kind, e.size)).collect(),
        k => panic!("unknown kind {}", k),
    };
    for (n, k, s) in rows {
        println!("{:?}\t{}\t{}", k, s, n);
    }
}

use loos_btrfs::{BtrfsReader, ReaderError, SliceReader};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "loos-btrfs-reader-{}-{}-{}",
        name,
        std::process::id(),
        nonce
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn real_image() -> (PathBuf, Vec<u8>) {
    let dir = scratch("real");
    let root = dir.join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("hello.txt"), b"hello from btrfs\n").unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(
        root.join("nested/data.bin"),
        (0..100_000u32)
            .map(|n| (n as u8).wrapping_mul(31))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let image = dir.join("image.btrfs");
    fs::File::create(&image)
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    let output = Command::new("mkfs.btrfs")
        .args([
            "--rootdir",
            root.to_str().unwrap(),
            "-f",
            "-s",
            "4096",
            "-n",
            "16384",
            image.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mkfs.btrfs failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(&image).unwrap();
    (image, bytes)
}

#[test]
fn reads_real_mkfs_rootdir_image() {
    let (_image, bytes) = real_image();
    let reader = BtrfsReader::open(SliceReader::new(&bytes)).unwrap();
    let stat = reader.stat_path(b"/hello.txt").unwrap();
    assert_eq!(stat.size, 17);
    assert_eq!(
        reader.read_file(b"/hello.txt", 0, 100).unwrap(),
        b"hello from btrfs\n"
    );
    let listed = reader
        .list_dir(reader.stat_path(b"/").unwrap().inode)
        .unwrap();
    assert!(listed.iter().any(|entry| entry.name == b"hello.txt"));
    assert_eq!(
        reader
            .read_file(b"/nested/data.bin", 4090, 40)
            .unwrap()
            .len(),
        40
    );
}

#[test]
fn rejects_corrupted_superblock_without_returning_bytes() {
    let (_image, mut bytes) = real_image();
    let reader = BtrfsReader::open(SliceReader::new(&bytes));
    assert!(reader.is_ok());
    // Corrupting the primary superblock is detected before any tree read.
    bytes[65536 + 80] ^= 1;
    assert!(matches!(
        BtrfsReader::open(SliceReader::new(&bytes)),
        Err(ReaderError::Checksum)
    ));
}

#[test]
fn rejects_truncated_image_before_tree_traversal() {
    let (_image, bytes) = real_image();
    let truncated = &bytes[..65_536];
    assert!(matches!(
        BtrfsReader::open(SliceReader::new(truncated)),
        Err(ReaderError::Read(loos_btrfs::ReadError::OutOfBounds { .. }))
    ));
}

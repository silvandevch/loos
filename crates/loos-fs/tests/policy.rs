//! Policy guards (`docs/BTRFS.md`, `docs/INSTALL.md`): btrfs is the only filesystem LoOS lets a user
//! pick, NTFS is read-only, FAT is boot-only. These tests read the repository sources, so a change that
//! quietly adds a filesystem choice to the installer or a write path to the NTFS reader fails CI.
use std::path::PathBuf;

fn repo(path: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(path);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {}", p.display(), e))
}

#[test]
fn installer_offers_no_filesystem_choice() {
    let ui = repo("src/shell/installer.c").to_lowercase();
    for fs in ["ntfs", "ext4", "ext3", "exfat", "xfs", "zfs", "reiser", "refs", "f2fs", "fat32"] {
        assert!(!ui.contains(fs), "the installer UI mentions {:?}: filesystem choices are forbidden (docs/BTRFS.md)", fs);
    }
    assert!(ui.contains("choose a disk"), "installer UI changed shape; update this test deliberately");
    // the steps are Welcome / Disk / Account / Privacy / Review: no "Filesystem"/"Partition" page
    assert!(!ui.contains("\"partition") && !ui.contains("\"filesystem") && !ui.contains("file system"));
}

#[test]
fn installer_backend_formats_only_btrfs_and_the_esp() {
    let be = repo("src/kernel/boot/install.c");
    assert!(be.contains("loos_btrfs_mkfs"), "the data partition must be formatted with the native btrfs formatter");
    let lower = be.to_lowercase();
    for fs in ["ntfs", "ext4", "exfat", "xfs"] {
        assert!(!lower.contains(fs), "installer backend references {:?}", fs);
    }
    // the only other filesystem it writes is the FAT32 ESP
    assert!(lower.contains("fat32_format"));
    assert_eq!(lower.matches("_format(").count() + lower.matches("_mkfs(").count(), lower.matches("fat32_format(").count() + lower.matches("loos_btrfs_mkfs(").count());
}

#[test]
fn install_doc_states_the_policy() {
    let doc = repo("docs/INSTALL.md");
    assert!(doc.contains("No NTFS/ext4/exFAT choice"), "docs/INSTALL.md must keep the btrfs-only statement");
    let btrfs = repo("docs/BTRFS.md");
    assert!(btrfs.contains("btrfs is the only filesystem you choose"));
}

#[test]
fn ntfs_reader_has_no_write_path() {
    let src = repo("crates/loos-fs/src/ntfs.rs");
    // a write path would need a mutable device method; the reader never calls write_at
    assert!(!src.contains("write_at"), "NTFS must stay read-only (docs/BTRFS.md)");
    for banned in ["pub fn write", "pub fn create", "pub fn mkdir", "pub fn remove", "pub fn truncate", "pub fn rename"] {
        assert!(!src.contains(banned), "ntfs.rs gained {:?}", banned);
    }
    for ro in ["iso9660", "udf"] {
        let s = repo(&format!("crates/loos-fs/src/{}.rs", ro));
        assert!(!s.contains("write_at"), "{} must stay read-only", ro);
    }
}

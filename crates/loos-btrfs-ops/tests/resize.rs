mod common;
use common::*;
use loos_btrfs_ops::check::{check_devices, Options};
use loos_btrfs_ops::dev::{Dev, FileDev};
use loos_btrfs_ops::error::Error;
use loos_btrfs_ops::ioctl::{ResizeKind, ResizeSpec};
use loos_btrfs_ops::resize::*;
use loos_btrfs_ops::volume::Volume;

fn field(dump: &str, name: &str) -> String {
    dump.lines().find(|l| l.starts_with(name)).map(|l| l[name.len()..].trim().to_string()).unwrap_or_default()
}

fn total_bytes(img: &std::path::Path) -> u64 {
    field(&must("btrfs", &["inspect-internal", "dump-super", img.to_str().unwrap()]), "total_bytes").parse().unwrap()
}

#[test]
fn offline_grow_and_shrink_match_btrfs_progs_view() {
    if !have("mkfs.btrfs") || !have("btrfs") {
        return;
    }
    let d = scratch("resize");
    let t = d.join("t");
    std::fs::create_dir_all(&t).unwrap();
    std::fs::write(t.join("f"), payload(1, 3_000_000)).unwrap();
    let img = d.join("r.img");
    mkfs_btrfs(&img, 200, Some(&t), &["--nodiscard", "-d", "dup", "-m", "dup"]);
    // mkfs --rootdir may extend the image to fit DUP data: use whatever size it ended up with
    let total0 = total_bytes(&img);
    assert!(total0 >= 200 << 20);

    // ---- grow to the new device size
    let f = std::fs::OpenOptions::new().write(true).open(&img).unwrap();
    f.set_len(total0 + (121 << 20) + 1234).unwrap(); // not sector aligned on purpose
    drop(f);
    let mut vol = Volume::open(vec![FileDev::open_rw(&img).unwrap()]).unwrap();
    let dev_len = vol.dev_mut(1).unwrap().len();
    let plan = plan_resize(&mut vol, &ResizeSpec { devid: 1, kind: ResizeKind::Max }, dev_len).unwrap();
    assert!(plan.is_direct());
    assert_eq!(plan.old_size, total0);
    assert_eq!(plan.new_size, total0 + (121 << 20), "aligned down to the sector size");
    apply_offline(&mut vol, &plan).unwrap();
    drop(vol);
    assert_eq!(total_bytes(&img), total0 + (121 << 20));
    assert!(run("btrfs", &["check", "--readonly", img.to_str().unwrap()]).0, "btrfs-progs must accept the grown filesystem");
    let rep = check_devices(vec![FileDev::open_ro(&img).unwrap()], &Options::default());
    assert!(rep.is_clean(), "{:?}", rep.findings);

    // ---- shrink by a relative amount that still holds every chunk
    let mut vol = Volume::open(vec![FileDev::open_rw(&img).unwrap()]).unwrap();
    let dev_len = vol.dev_mut(1).unwrap().len();
    let plan = plan_resize(&mut vol, &ResizeSpec { devid: 1, kind: ResizeKind::Shrink(100 << 20) }, dev_len).unwrap();
    assert_eq!(plan.new_size, total0 + (21 << 20));
    assert!(plan.is_direct(), "chunks live at the start of the device: {:?}", plan.relocate);
    assert!(plan.used_end <= plan.new_size);
    apply_offline(&mut vol, &plan).unwrap();
    drop(vol);
    assert_eq!(total_bytes(&img), total0 + (21 << 20));
    assert!(run("btrfs", &["check", "--readonly", img.to_str().unwrap()]).0);

    // ---- a shrink below the allocated area needs relocation and is refused by the offline applier
    let mut vol = Volume::open(vec![FileDev::open_rw(&img).unwrap()]).unwrap();
    let dev_len = vol.dev_mut(1).unwrap().len();
    let used_end = plan.used_end;
    let plan = plan_resize(&mut vol, &ResizeSpec { devid: 1, kind: ResizeKind::Absolute(used_end - (4 << 20)) }, dev_len).unwrap();
    assert!(!plan.relocate.is_empty());
    assert!(plan.relocate.iter().all(|r| r.physical + r.length > plan.new_size));
    assert_eq!(apply_offline(&mut vol, &plan), Err(Error::Invalid("shrink needs relocation of chunks beyond the new end")));

    // ---- argument validation
    assert!(plan_resize(&mut vol, &ResizeSpec { devid: 1, kind: ResizeKind::Absolute(4096) }, dev_len).is_err());
    assert!(plan_resize(&mut vol, &ResizeSpec { devid: 1, kind: ResizeKind::Grow(u64::MAX) }, dev_len).is_err());
    assert!(plan_resize(&mut vol, &ResizeSpec { devid: 1, kind: ResizeKind::Absolute(dev_len + (1 << 20)) }, dev_len).is_err());
    assert!(plan_resize(&mut vol, &ResizeSpec { devid: 9, kind: ResizeKind::Max }, dev_len).is_err());
    assert!(plan_resize(&mut vol, &ResizeSpec { devid: 1, kind: ResizeKind::Shrink(u64::MAX) }, dev_len).is_err());
}

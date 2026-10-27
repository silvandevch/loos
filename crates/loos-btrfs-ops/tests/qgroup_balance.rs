mod common;
use common::*;
use loos_btrfs_ops::balance::*;
use loos_btrfs_ops::consts::*;
use loos_btrfs_ops::dev::FileDev;
use loos_btrfs_ops::qgroup::*;
use loos_btrfs_ops::volume::{Key, Volume};

fn w(vals: &[u64]) -> Vec<u8> {
    vals.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn qgroup_limits_and_inheritance() {
    let sv = make_id(0, 256);
    let top = make_id(1, 0);
    let items = vec![
        (Key::new(0, QGROUP_STATUS_KEY, 0), w(&[1, 7, status::ON | status::INCONSISTENT, 0, 7])),
        (Key::new(0, QGROUP_INFO_KEY, sv), w(&[7, 900, 900, 400, 400])),
        (Key::new(0, QGROUP_INFO_KEY, top), w(&[7, 900, 900, 400, 400])),
        (Key::new(0, QGROUP_LIMIT_KEY, sv), w(&[limit::MAX_EXCL, 0, 600, 0, 0])),
        (Key::new(0, QGROUP_LIMIT_KEY, top), w(&[limit::MAX_RFER, 1000, 0, 0, 0])),
        (Key::new(sv, QGROUP_RELATION_KEY, top), vec![]),
        (Key::new(top, QGROUP_RELATION_KEY, sv), vec![]),
    ];
    let q = Quota::from_items(&items).unwrap();
    assert!(q.inconsistent());
    assert_eq!(q.groups[&sv].parents, vec![top]);
    assert_eq!(q.groups[&top].children, vec![sv]);
    assert!(q.violations().is_empty());
    // 50 more bytes fits the child's exclusive limit (450 <= 500) but 150 pushes the parent over rfer.
    assert_eq!(q.would_exceed(sv, 50), None);
    assert_eq!(q.would_exceed(sv, 101), Some(Violation::Referenced { qgroup: top, used: 1001, limit: 1000 }));
    assert!(matches!(q.would_exceed(sv, 250), Some(Violation::Exclusive { qgroup, .. }) if qgroup == sv));
    assert_eq!((level(top), subvol_id(sv)), (1, 256));
}

#[test]
fn balance_plan_orders_and_filters() {
    if !have("mkfs.btrfs") {
        return;
    }
    let d = scratch("balance");
    let t = d.join("t");
    std::fs::create_dir_all(&t).unwrap();
    std::fs::write(t.join("f"), payload(1, 4 << 20)).unwrap();
    let img = d.join("d.img");
    mkfs_btrfs(&img, 300, Some(&t), &["--nodiscard"]);
    let mut vol = Volume::open(vec![FileDev::open_ro(&img).unwrap()]).unwrap();
    let usage = read_usage(&mut vol).unwrap();
    assert!(usage.iter().filter(|g| g.flags & BLOCK_GROUP_DATA != 0).map(|g| g.used).sum::<u64>() >= 4 << 20, "{usage:?}");
    assert!(usage.iter().all(|g| g.used <= g.length));
    let all = plan(&vol, &usage, Filter::all(), None);
    assert_eq!(all.groups.len(), usage.len());
    assert!(all.groups.windows(2).all(|w| w[0].used <= w[1].used));
    let data_empty = plan(&vol, &usage, Filter::usage_at_most(BLOCK_GROUP_DATA, 1), None);
    assert!(data_empty.groups.iter().all(|g| g.flags & BLOCK_GROUP_DATA != 0 && g.percent() <= 1));
    // converting to the profile everything already has plans nothing
    let cur = loos_btrfs_ops::raid::Profile::from_flags(usage.iter().find(|g| g.flags & BLOCK_GROUP_DATA != 0).unwrap().flags);
    if let Some(p) = cur {
        let only_data = Filter { types: BLOCK_GROUP_DATA, usage_max: 100, ..Default::default() };
        assert!(plan(&vol, &usage, only_data, Some(p)).groups.is_empty());
    }
    assert!(feasible(&all, 1 << 30, 256 << 20));
    assert!(!feasible(&all, 1, 256 << 20));
}

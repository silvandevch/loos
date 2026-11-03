use loos_btrfs_ops::snapshot::{plan_snapshot, SnapshotMode, SnapshotRequest};
use loos_btrfs_ops::volume::RootItem;

fn source() -> RootItem {
    RootItem {
        generation: 7,
        root_dirid: 256,
        bytenr: 0x5000,
        byte_limit: 0,
        bytes_used: 0x4000,
        last_snapshot: 3,
        flags: 0,
        refs: 2,
        level: 0,
        uuid: [1; 16],
        parent_uuid: [0; 16],
        received_uuid: [2; 16],
        ctransid: 7,
        otransid: 1,
        stransid: 0,
        rtransid: 0,
    }
}

#[test]
fn plans_read_only_snapshot_with_parent_uuid() {
    let plan = plan_snapshot(
        source(),
        SnapshotRequest {
            source_id: 256,
            destination_id: 257,
            generation: 8,
            uuid: [9; 16],
            mode: SnapshotMode::ReadOnly,
        },
    )
    .unwrap();
    assert_eq!(plan.source_id, 256);
    assert_eq!(plan.root.parent_uuid, [1; 16]);
    assert_eq!(plan.root.uuid, [9; 16]);
    assert_eq!(plan.root.generation, 8);
    assert_eq!(plan.root.otransid, 8);
    assert_eq!(plan.root.flags & 1, 1);
    assert_eq!(plan.root.refs, 1);
    assert_eq!(plan.root.received_uuid, [0; 16]);
}

#[test]
fn writable_snapshot_clears_read_only_but_rejects_bad_identity() {
    let mut parent = source();
    parent.flags = 1;
    let plan = plan_snapshot(
        parent,
        SnapshotRequest {
            source_id: 256,
            destination_id: 257,
            generation: 8,
            uuid: [9; 16],
            mode: SnapshotMode::Writable,
        },
    )
    .unwrap();
    assert_eq!(plan.root.flags & 1, 0);
    assert!(plan_snapshot(
        source(),
        SnapshotRequest {
            source_id: 256,
            destination_id: 256,
            generation: 8,
            uuid: [9; 16],
            mode: SnapshotMode::Writable,
        },
    )
    .is_err());
    assert!(plan_snapshot(
        source(),
        SnapshotRequest {
            source_id: 256,
            destination_id: 257,
            generation: 7,
            uuid: [9; 16],
            mode: SnapshotMode::Writable,
        },
    )
    .is_err());
}

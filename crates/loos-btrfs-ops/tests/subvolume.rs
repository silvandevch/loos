use loos_btrfs_ops::subvolume::{validate_subvolume, SubvolumeRequest};

fn request<'a>(name: &'a [u8]) -> SubvolumeRequest<'a> {
    SubvolumeRequest {
        id: 256,
        parent_id: 5,
        dirid: 257,
        name,
        uuid: [7; 16],
    }
}

#[test]
fn validates_nested_subvolume_identity() {
    let id = validate_subvolume(request(b"home")).unwrap();
    assert_eq!(id.id, 256);
    assert_eq!(id.parent_id, 5);
    assert_eq!(id.dirid, 257);
    assert_eq!(id.name, b"home");
}

#[test]
fn rejects_unsafe_or_reserved_subvolume_inputs() {
    assert!(validate_subvolume(request(b"")).is_err());
    assert!(validate_subvolume(request(b"a/b")).is_err());
    assert!(validate_subvolume(request(b"a\0b")).is_err());
    let mut bad = request(b"reserved");
    bad.id = 255;
    assert!(validate_subvolume(bad).is_err());
    bad = request(b"zero-parent");
    bad.parent_id = 0;
    assert!(validate_subvolume(bad).is_err());
    bad = request(b"zero-uuid");
    bad.uuid = [0; 16];
    assert!(validate_subvolume(bad).is_err());
}

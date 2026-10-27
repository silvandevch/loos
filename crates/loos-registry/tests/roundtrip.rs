use loos_registry::{Error, Hive, ValueType};

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn canonical_hive_roundtrips_keys_values_and_unicode() {
    let mut hive = Hive::new();
    hive.create_key("Software\\LoOS\\Shell").unwrap();
    hive.set_value(
        "Software\\LoOS\\Shell",
        "Enabled",
        ValueType::Dword,
        &7u32.to_le_bytes(),
    )
    .unwrap();
    hive.set_value(
        "Software\\LoOS\\Shell",
        "Title",
        ValueType::Sz,
        &utf16("Lócal"),
    )
    .unwrap();
    hive.set_value(
        "Software\\LoOS\\Shell",
        "Blob",
        ValueType::Binary,
        &[1, 2, 3, 4, 5],
    )
    .unwrap();

    let image = hive.save().unwrap();
    assert_eq!(&image[..4], b"regf");
    let loaded = Hive::load(&image).unwrap();
    assert_eq!(loaded.key("software/loos/shell").unwrap().name, "Shell");
    assert_eq!(
        loaded
            .value("Software\\LoOS\\Shell", "enabled")
            .unwrap()
            .data,
        7u32.to_le_bytes()
    );
    assert_eq!(
        loaded.value("Software\\LoOS\\Shell", "TITLE").unwrap().data,
        utf16("Lócal")
    );
    assert_eq!(
        loaded.value("Software\\LoOS\\Shell", "Blob").unwrap().kind,
        ValueType::Binary
    );
}

#[test]
fn updates_and_deletes_are_persisted() {
    let mut hive = Hive::new();
    hive.set_value(
        "Software\\LoOS",
        "Value",
        ValueType::Dword,
        &1u32.to_le_bytes(),
    )
    .unwrap();
    hive.set_value(
        "Software\\LoOS",
        "Value",
        ValueType::Dword,
        &2u32.to_le_bytes(),
    )
    .unwrap();
    assert_eq!(
        hive.value("Software\\LoOS", "Value").unwrap().data,
        2u32.to_le_bytes()
    );
    hive.delete_value("Software\\LoOS", "value").unwrap();
    assert_eq!(hive.value("Software\\LoOS", "Value"), Err(Error::NotFound));
}

#[test]
fn rejects_bad_signature_and_truncated_cells() {
    assert_eq!(Hive::load(b"not a hive"), Err(Error::Corrupt));
    let mut image = Hive::new().save().unwrap();
    image[0x1000 + 4..0x1000 + 8].copy_from_slice(&(-4i32).to_le_bytes());
    assert_eq!(Hive::load(&image), Err(Error::Corrupt));
}

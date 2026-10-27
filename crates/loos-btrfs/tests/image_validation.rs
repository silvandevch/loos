use loos_btrfs::{
    crc32c, validate_image, write_crc32c_block, BtrfsSuperblock, ImageValidationError,
    BTRFS_IMAGE_MIN_SIZE, BTRFS_MAGIC, BTRFS_SUPERBLOCK_OFFSET, BTRFS_SUPERBLOCK_SIZE,
    BTRFS_SYSTEM_CHUNK_ARRAY_OFFSET,
};

const SUPER_FSID: usize = 32;
const SUPER_BYTENR: usize = 48;
const SUPER_MAGIC: usize = 64;
const SUPER_ROOT: usize = 80;
const SUPER_CHUNK_ROOT: usize = 88;
const SUPER_TOTAL_BYTES: usize = 112;
const SUPER_SECTORSIZE: usize = 144;
const SUPER_NODESIZE: usize = 148;
const SUPER_SYS_CHUNK_ARRAY_SIZE: usize = 160;

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn valid_image() -> Vec<u8> {
    let mut image = vec![0u8; BTRFS_IMAGE_MIN_SIZE];
    let image_len = image.len() as u64;
    let superblock = &mut image[BTRFS_SUPERBLOCK_OFFSET as usize
        ..BTRFS_SUPERBLOCK_OFFSET as usize + BTRFS_SUPERBLOCK_SIZE];
    superblock[SUPER_FSID..SUPER_FSID + 16].fill(1);
    put64(superblock, SUPER_BYTENR, BTRFS_SUPERBLOCK_OFFSET);
    put64(superblock, SUPER_MAGIC, BTRFS_MAGIC);
    put64(superblock, SUPER_ROOT, 0x1000);
    put64(superblock, SUPER_CHUNK_ROOT, 0x2000);
    put64(superblock, SUPER_TOTAL_BYTES, image_len);
    put32(superblock, SUPER_SECTORSIZE, 4096);
    put32(superblock, SUPER_NODESIZE, 16_384);

    let chunk = BTRFS_SYSTEM_CHUNK_ARRAY_OFFSET;
    put32(superblock, SUPER_SYS_CHUNK_ARRAY_SIZE, 97);
    put64(superblock, chunk, 0x100);
    superblock[chunk + 8] = 228;
    put64(superblock, chunk + 9, 0);
    put64(superblock, chunk + 17, 0x1000);
    put64(superblock, chunk + 17 + 24, 0);
    superblock[chunk + 17 + 44..chunk + 17 + 46].copy_from_slice(&1u16.to_le_bytes());
    put64(superblock, chunk + 17 + 48, 1);
    put64(superblock, chunk + 17 + 56, 0x1000);
    write_crc32c_block(superblock).unwrap();
    image
}

#[test]
fn validates_complete_image_and_reports_chunks() {
    let image = valid_image();
    let result = validate_image(&image).unwrap();
    assert_eq!(result.image_len, image.len());
    assert_eq!(result.system_chunk_count, 1);
    assert_eq!(result.superblock.total_bytes, image.len() as u64);
}

#[test]
fn rejects_image_without_primary_superblock() {
    assert_eq!(
        validate_image(&[0; BTRFS_IMAGE_MIN_SIZE - 1]),
        Err(ImageValidationError::ImageTooShort {
            needed: BTRFS_IMAGE_MIN_SIZE,
            available: BTRFS_IMAGE_MIN_SIZE - 1,
        })
    );
}

#[test]
fn rejects_corrupted_superblock_checksum() {
    let mut image = valid_image();
    image[BTRFS_SUPERBLOCK_OFFSET as usize + SUPER_ROOT] ^= 1;
    assert!(matches!(
        validate_image(&image),
        Err(ImageValidationError::Superblock(
            loos_btrfs::ParseError::Checksum(_)
        ))
    ));
}

#[test]
fn rejects_declared_image_larger_than_supplied_bytes() {
    let mut image = valid_image();
    let image_len = image.len() as u64;
    let superblock = &mut image[BTRFS_SUPERBLOCK_OFFSET as usize
        ..BTRFS_SUPERBLOCK_OFFSET as usize + BTRFS_SUPERBLOCK_SIZE];
    put64(superblock, SUPER_TOTAL_BYTES, image_len + 1);
    write_crc32c_block(superblock).unwrap();
    assert!(matches!(
        validate_image(&image),
        Err(ImageValidationError::DeclaredSizeExceedsImage { .. })
    ));
}

#[test]
fn rejects_chunk_stripe_outside_declared_device() {
    let mut image = valid_image();
    let image_len = image.len() as u64;
    let superblock = &mut image[BTRFS_SUPERBLOCK_OFFSET as usize
        ..BTRFS_SUPERBLOCK_OFFSET as usize + BTRFS_SUPERBLOCK_SIZE];
    let chunk = BTRFS_SYSTEM_CHUNK_ARRAY_OFFSET + 17;
    put64(superblock, chunk + 56, image_len - 4096 + 1);
    write_crc32c_block(superblock).unwrap();
    assert!(matches!(
        validate_image(&image),
        Err(ImageValidationError::ChunkOutOfBounds { .. })
    ));
}

#[test]
fn rejects_malformed_system_chunk_array_without_panicking() {
    let mut image = valid_image();
    let superblock = &mut image[BTRFS_SUPERBLOCK_OFFSET as usize
        ..BTRFS_SUPERBLOCK_OFFSET as usize + BTRFS_SUPERBLOCK_SIZE];
    put32(superblock, SUPER_SYS_CHUNK_ARRAY_SIZE, 18);
    write_crc32c_block(superblock).unwrap();
    assert!(matches!(
        validate_image(&image),
        Err(ImageValidationError::SystemChunk(
            loos_btrfs::ParseError::Truncated { .. }
        ))
    ));
}

#[test]
fn rejects_chunk_without_stripes() {
    let mut image = valid_image();
    let superblock = &mut image[BTRFS_SUPERBLOCK_OFFSET as usize
        ..BTRFS_SUPERBLOCK_OFFSET as usize + BTRFS_SUPERBLOCK_SIZE];
    let chunk = BTRFS_SYSTEM_CHUNK_ARRAY_OFFSET + 17;
    put32(superblock, SUPER_SYS_CHUNK_ARRAY_SIZE, 65);
    superblock[chunk + 44..chunk + 46].fill(0);
    write_crc32c_block(superblock).unwrap();
    assert_eq!(
        validate_image(&image),
        Err(ImageValidationError::SystemChunk(
            loos_btrfs::ParseError::InvalidValue {
                field: "stripe count"
            }
        ))
    );
}

#[test]
fn crc_vector_used_by_image_builder_is_stable() {
    assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    assert!(BtrfsSuperblock::parse(&[0; BTRFS_SUPERBLOCK_SIZE]).is_err());
}

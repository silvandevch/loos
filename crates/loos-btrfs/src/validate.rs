//! Safe validation of a complete byte-slice Btrfs image.

use crate::ondisk::{
    BtrfsSuperblock, ParseError, BTRFS_SUPERBLOCK_OFFSET, BTRFS_SUPERBLOCK_SIZE,
};

/// The smallest image that can contain the primary Btrfs superblock.
pub const BTRFS_IMAGE_MIN_SIZE: usize = BTRFS_SUPERBLOCK_OFFSET as usize + BTRFS_SUPERBLOCK_SIZE;

/// Errors returned while validating a byte-slice image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageValidationError {
    /// The image does not reach the primary superblock.
    ImageTooShort { needed: usize, available: usize },
    /// The superblock declares a device smaller than the primary superblock.
    DeclaredSizeTooSmall { minimum: u64, declared: u64 },
    /// The superblock declares more device bytes than the supplied image.
    DeclaredSizeExceedsImage { declared: u64, available: u64 },
    /// The primary superblock could not be parsed or its checksum failed.
    Superblock(ParseError),
    /// A system chunk-array entry could not be parsed.
    SystemChunk(ParseError),
    /// A chunk stripe points beyond the declared device image.
    ChunkOutOfBounds {
        stripe: usize,
        offset: u64,
        length: u64,
        device_bytes: u64,
    },
}

/// The result of validating one complete image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageValidation<'a> {
    /// Parsed primary superblock metadata.
    pub superblock: BtrfsSuperblock<'a>,
    /// Number of system chunk entries parsed from the superblock.
    pub system_chunk_count: usize,
    /// Number of bytes supplied by the caller.
    pub image_len: usize,
}

/// Parse and validate the primary superblock and system chunk array in `image`.
///
/// This API only borrows the caller's bytes. It never casts a byte slice to a
/// packed structure and performs all offset, length, and checksum checks before
/// exposing parsed metadata. Tree blocks are intentionally deferred until a
/// caller has a chunk map and can provide their logical bytes.
pub fn validate_image(image: &[u8]) -> Result<ImageValidation<'_>, ImageValidationError> {
    if image.len() < BTRFS_IMAGE_MIN_SIZE {
        return Err(ImageValidationError::ImageTooShort {
            needed: BTRFS_IMAGE_MIN_SIZE,
            available: image.len(),
        });
    }

    let superblock_start = BTRFS_SUPERBLOCK_OFFSET as usize;
    let superblock_end = superblock_start + BTRFS_SUPERBLOCK_SIZE;
    let superblock = BtrfsSuperblock::parse(&image[superblock_start..superblock_end])
        .map_err(ImageValidationError::Superblock)?;
    superblock
        .verify_checksum(&image[superblock_start..superblock_end])
        .map_err(ImageValidationError::Superblock)?;

    let image_len = u64::try_from(image.len()).map_err(|_| ParseError::ArithmeticOverflow);
    let image_len = image_len.map_err(ImageValidationError::Superblock)?;
    let minimum = BTRFS_IMAGE_MIN_SIZE as u64;
    if superblock.total_bytes < minimum {
        return Err(ImageValidationError::DeclaredSizeTooSmall {
            minimum,
            declared: superblock.total_bytes,
        });
    }
    if superblock.total_bytes > image_len {
        return Err(ImageValidationError::DeclaredSizeExceedsImage {
            declared: superblock.total_bytes,
            available: image_len,
        });
    }

    let mut chunks = superblock.system_chunks();
    let mut system_chunk_count = 0usize;
    while let Some(chunk) = chunks.next().map_err(ImageValidationError::SystemChunk)? {
        if chunk.item.length == 0 || chunk.item.stripe_count == 0 {
            return Err(ImageValidationError::SystemChunk(ParseError::InvalidValue {
                field: if chunk.item.length == 0 {
                    "chunk length"
                } else {
                    "stripe count"
                },
            }));
        }
        for stripe_index in 0..usize::from(chunk.item.stripe_count) {
            let stripe = chunk
                .item
                .stripe(stripe_index)
                .map_err(ImageValidationError::SystemChunk)?;
            let end = stripe.offset.checked_add(chunk.item.length).ok_or(
                ImageValidationError::ChunkOutOfBounds {
                    stripe: stripe_index,
                    offset: stripe.offset,
                    length: chunk.item.length,
                    device_bytes: superblock.total_bytes,
                },
            )?;
            if end > superblock.total_bytes {
                return Err(ImageValidationError::ChunkOutOfBounds {
                    stripe: stripe_index,
                    offset: stripe.offset,
                    length: chunk.item.length,
                    device_bytes: superblock.total_bytes,
                });
            }
        }
        system_chunk_count = system_chunk_count
            .checked_add(1)
            .ok_or(ImageValidationError::SystemChunk(ParseError::ArithmeticOverflow))?;
    }

    Ok(ImageValidation {
        superblock,
        system_chunk_count,
        image_len: image.len(),
    })
}

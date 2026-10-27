//! Checksums used by the Btrfs on-disk format.

/// Number of bytes reserved for a Btrfs block checksum.
pub const BTRFS_CSUM_SIZE: usize = 32;

/// Errors returned while checking a stored block checksum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksumError {
    /// The input does not contain the checksum area and at least one payload byte.
    TooShort,
    /// The stored and calculated checksums differ.
    Mismatch { expected: u32, actual: u32 },
}

/// Calculate the reflected CRC32C recurrence from an arbitrary seed.
///
/// Btrfs directory hashes use this raw form with a format-defined seed.  For
/// stored block checksums, use [`crc32c`] instead.
pub fn crc32c_raw(mut crc: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0x82f6_3b78 & mask);
        }
    }
    crc
}

/// Calculate the finalized CRC32C value used for Btrfs metadata blocks.
pub fn crc32c(bytes: &[u8]) -> u32 {
    !crc32c_raw(!0, bytes)
}

/// Verify a Btrfs CRC32C block checksum.
///
/// The first four bytes contain the little-endian CRC32C and the remaining
/// bytes of the 32-byte checksum area are reserved.  They are excluded from
/// the calculation, as required by the format.
pub fn verify_crc32c_block(block: &[u8]) -> Result<(), ChecksumError> {
    if block.len() <= BTRFS_CSUM_SIZE {
        return Err(ChecksumError::TooShort);
    }

    let expected = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
    let actual = crc32c(&block[BTRFS_CSUM_SIZE..]);
    if expected == actual {
        Ok(())
    } else {
        Err(ChecksumError::Mismatch { expected, actual })
    }
}

/// Write a Btrfs CRC32C block checksum into `block`.
///
/// This clears the reserved checksum bytes before writing the little-endian
/// value.  The slice must contain the complete 32-byte checksum area and at
/// least one payload byte.
pub fn write_crc32c_block(block: &mut [u8]) -> Result<(), ChecksumError> {
    if block.len() <= BTRFS_CSUM_SIZE {
        return Err(ChecksumError::TooShort);
    }

    let actual = crc32c(&block[BTRFS_CSUM_SIZE..]);
    block[..BTRFS_CSUM_SIZE].fill(0);
    block[..4].copy_from_slice(&actual.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_known_vector() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }

    #[test]
    fn raw_crc_supports_btrfs_directory_seed() {
        assert_eq!(crc32c_raw(!1, b"hello"), 0x5d9f_2b1f);
    }

    #[test]
    fn block_checksum_round_trip_and_reserved_bytes() {
        let mut block = [0xa5u8; 64];
        write_crc32c_block(&mut block).unwrap();
        assert!(block[4..BTRFS_CSUM_SIZE].iter().all(|&byte| byte == 0));
        verify_crc32c_block(&block).unwrap();

        block[40] ^= 1;
        assert!(matches!(
            verify_crc32c_block(&block),
            Err(ChecksumError::Mismatch { .. })
        ));
    }

    #[test]
    fn checksum_rejects_missing_payload() {
        assert_eq!(
            verify_crc32c_block(&[0; BTRFS_CSUM_SIZE]),
            Err(ChecksumError::TooShort)
        );
    }
}

//! Allocation-free Btrfs checksum algorithms.
//!
//! Btrfs stores CRC32C and XXH64 in little-endian form, while SHA-256 and
//! BLAKE2b-256 are stored as their 32-byte digest.  The implementation keeps
//! the API `no_std` and uses only bounded stack scratch space.

use crate::checksum::crc32c;

/// Checksum types defined by the Btrfs superblock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum ChecksumAlgorithm {
    /// CRC32C, stored as a four-byte little-endian value.
    Crc32c = 0,
    /// XXH64, stored as an eight-byte little-endian value.
    XxHash64 = 1,
    /// SHA-256, stored as a 32-byte digest.
    Sha256 = 2,
    /// BLAKE2b with a 256-bit output, stored as a 32-byte digest.
    Blake2b256 = 3,
}

impl ChecksumAlgorithm {
    /// Converts an on-disk Btrfs checksum type to an algorithm.
    pub fn from_btrfs_type(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::Crc32c),
            1 => Some(Self::XxHash64),
            2 => Some(Self::Sha256),
            3 => Some(Self::Blake2b256),
            _ => None,
        }
    }

    /// Returns the number of meaningful bytes in a stored checksum.
    pub const fn digest_len(self) -> usize {
        match self {
            Self::Crc32c => 4,
            Self::XxHash64 => 8,
            Self::Sha256 | Self::Blake2b256 => 32,
        }
    }
}

/// A checksum with room for the largest Btrfs digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Digest {
    bytes: [u8; 32],
    len: usize,
}

impl Digest {
    /// Returns the meaningful encoded checksum bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Computes a checksum in the exact byte order used by Btrfs checksum items.
pub fn digest(algorithm: ChecksumAlgorithm, data: &[u8]) -> Digest {
    let mut out = Digest { bytes: [0; 32], len: algorithm.digest_len() };
    match algorithm {
        ChecksumAlgorithm::Crc32c => out.bytes[..4].copy_from_slice(&crc32c(data).to_le_bytes()),
        ChecksumAlgorithm::XxHash64 => out.bytes[..8].copy_from_slice(&xxhash64(data).to_le_bytes()),
        ChecksumAlgorithm::Sha256 => sha256(data, &mut out.bytes),
        ChecksumAlgorithm::Blake2b256 => blake2b256(data, &mut out.bytes),
    }
    out
}

/// Compares a stored checksum with data without early-exit timing differences.
pub fn verify(algorithm: ChecksumAlgorithm, data: &[u8], expected: &[u8]) -> bool {
    let actual = digest(algorithm, data);
    if expected.len() != actual.len {
        return false;
    }
    let mut different = 0u8;
    for i in 0..actual.len {
        different |= actual.bytes[i] ^ expected[i];
    }
    different == 0
}

const XX_P1: u64 = 11_400_714_785_074_694_791;
const XX_P2: u64 = 14_029_467_366_897_019_727;
const XX_P3: u64 = 1_609_587_929_392_839_161;
const XX_P4: u64 = 9_650_029_242_287_828_579;
const XX_P5: u64 = 2_870_177_450_012_600_261;

fn read_u64(data: &[u8]) -> u64 {
    u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]])
}

fn read_u32(data: &[u8]) -> u32 {
    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
}

fn xx_round(mut acc: u64, input: u64) -> u64 {
    acc = acc.wrapping_add(input.wrapping_mul(XX_P2));
    acc.rotate_left(31).wrapping_mul(XX_P1)
}

fn xx_merge(mut acc: u64, value: u64) -> u64 {
    acc ^= xx_round(0, value);
    acc = acc.wrapping_mul(XX_P1).wrapping_add(XX_P4);
    acc
}

fn xxhash64(data: &[u8]) -> u64 {
    let mut p = 0usize;
    let mut hash;
    if data.len() >= 32 {
        let mut v1 = XX_P1.wrapping_add(XX_P2);
        let mut v2 = XX_P2;
        let mut v3 = 0;
        let mut v4 = 0u64.wrapping_sub(XX_P1);
        while p + 32 <= data.len() {
            v1 = xx_round(v1, read_u64(&data[p..]));
            v2 = xx_round(v2, read_u64(&data[p + 8..]));
            v3 = xx_round(v3, read_u64(&data[p + 16..]));
            v4 = xx_round(v4, read_u64(&data[p + 24..]));
            p += 32;
        }
        hash = v1.rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        hash = xx_merge(hash, v1);
        hash = xx_merge(hash, v2);
        hash = xx_merge(hash, v3);
        hash = xx_merge(hash, v4);
    } else {
        hash = XX_P5;
    }
    hash = hash.wrapping_add(data.len() as u64);
    while p + 8 <= data.len() {
        let k = xx_round(0, read_u64(&data[p..]));
        hash ^= k;
        hash = hash.rotate_left(27).wrapping_mul(XX_P1).wrapping_add(XX_P4);
        p += 8;
    }
    if p + 4 <= data.len() {
        hash ^= (read_u32(&data[p..]) as u64).wrapping_mul(XX_P1);
        hash = hash.rotate_left(23).wrapping_mul(XX_P2).wrapping_add(XX_P3);
        p += 4;
    }
    while p < data.len() {
        hash ^= (data[p] as u64).wrapping_mul(XX_P5);
        hash = hash.rotate_left(11).wrapping_mul(XX_P1);
        p += 1;
    }
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(XX_P2);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(XX_P3);
    hash ^ (hash >> 32)
}

const SHA256_K: [u32; 64] = [
    0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1,
    0x923f_82a4, 0xab1c_5ed5, 0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3,
    0x72be_5d74, 0x80de_b1fe, 0x9bdc_06a7, 0xc19b_f174, 0xe49b_69c1, 0xefbe_4786,
    0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f, 0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da,
    0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7, 0xc6e0_0bf3, 0xd5a7_9147,
    0x06ca_6351, 0x1429_2967, 0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc, 0x5338_0d13,
    0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85, 0xa2bf_e8a1, 0xa81a_664b,
    0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070,
    0x19a4_c116, 0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a,
    0x5b9c_ca4f, 0x682e_6ff3, 0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208,
    0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7, 0xc671_78f2,
];

#[inline]
fn sha_rotr(value: u32, bits: u32) -> u32 {
    value.rotate_right(bits)
}

fn sha256_compress(state: &mut [u32; 8], block: &[u8]) {
    let mut schedule = [0u32; 64];
    for i in 0..16 {
        schedule[i] = u32::from_be_bytes([
            block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let x = schedule[i - 15];
        let y = schedule[i - 2];
        let small0 = sha_rotr(x, 7) ^ sha_rotr(x, 18) ^ (x >> 3);
        let small1 = sha_rotr(y, 17) ^ sha_rotr(y, 19) ^ (y >> 10);
        schedule[i] = schedule[i - 16]
            .wrapping_add(small0)
            .wrapping_add(schedule[i - 7])
            .wrapping_add(small1);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];
    for i in 0..64 {
        let big1 = sha_rotr(e, 6) ^ sha_rotr(e, 11) ^ sha_rotr(e, 25);
        let choose = (e & f) ^ ((!e) & g);
        let t1 = h
            .wrapping_add(big1)
            .wrapping_add(choose)
            .wrapping_add(SHA256_K[i])
            .wrapping_add(schedule[i]);
        let big0 = sha_rotr(a, 2) ^ sha_rotr(a, 13) ^ sha_rotr(a, 22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let t2 = big0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

fn sha256(data: &[u8], output: &mut [u8; 32]) {
    let mut state = [
        0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a,
        0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19,
    ];
    let mut offset = 0;
    while offset + 64 <= data.len() {
        sha256_compress(&mut state, &data[offset..offset + 64]);
        offset += 64;
    }
    let remaining = data.len() - offset;
    let mut padded = [0u8; 128];
    padded[..remaining].copy_from_slice(&data[offset..]);
    padded[remaining] = 0x80;
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let pad_len = if remaining < 56 { 64 } else { 128 };
    padded[pad_len - 8..pad_len].copy_from_slice(&bit_len.to_be_bytes());
    sha256_compress(&mut state, &padded[..64]);
    if pad_len == 128 {
        sha256_compress(&mut state, &padded[64..]);
    }
    for i in 0..8 {
        output[i * 4..i * 4 + 4].copy_from_slice(&state[i].to_be_bytes());
    }
}

const BLAKE2B_IV: [u64; 8] = [
    0x6a09_e667_f3bc_c908,
    0xbb67_ae85_84ca_a73b,
    0x3c6e_f372_fe94_f82b,
    0xa54f_f53a_5f1d_36f1,
    0x510e_527f_ade6_82d1,
    0x9b05_688c_2b3e_6c1f,
    0x1f83_d9ab_fb41_bd6b,
    0x5be0_cd19_137e_2179,
];

const BLAKE2B_SIGMA: [[usize; 16]; 12] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
];

#[inline]
fn blake_g(v: &mut [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

fn blake2b_compress(state: &mut [u64; 8], block: &[u8], counter: u128, last: bool) {
    let mut message = [0u64; 16];
    for i in 0..16 {
        message[i] = u64::from_le_bytes([
            block[i * 8], block[i * 8 + 1], block[i * 8 + 2], block[i * 8 + 3],
            block[i * 8 + 4], block[i * 8 + 5], block[i * 8 + 6], block[i * 8 + 7],
        ]);
    }
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(state);
    v[8..].copy_from_slice(&BLAKE2B_IV);
    v[12] ^= counter as u64;
    v[13] ^= (counter >> 64) as u64;
    if last {
        v[14] = !v[14];
    }
    for round in 0..12 {
        let s = &BLAKE2B_SIGMA[round];
        blake_g(&mut v, 0, 4, 8, 12, message[s[0]], message[s[1]]);
        blake_g(&mut v, 1, 5, 9, 13, message[s[2]], message[s[3]]);
        blake_g(&mut v, 2, 6, 10, 14, message[s[4]], message[s[5]]);
        blake_g(&mut v, 3, 7, 11, 15, message[s[6]], message[s[7]]);
        blake_g(&mut v, 0, 5, 10, 15, message[s[8]], message[s[9]]);
        blake_g(&mut v, 1, 6, 11, 12, message[s[10]], message[s[11]]);
        blake_g(&mut v, 2, 7, 8, 13, message[s[12]], message[s[13]]);
        blake_g(&mut v, 3, 4, 9, 14, message[s[14]], message[s[15]]);
    }
    for i in 0..8 {
        state[i] ^= v[i] ^ v[i + 8];
    }
}

fn blake2b256(data: &[u8], output: &mut [u8; 32]) {
    let mut state = BLAKE2B_IV;
    // digest length 32, fanout 1, depth 1 (the unkeyed sequential parameter block)
    state[0] ^= 0x0101_0020;
    let mut offset = 0usize;
    let mut counter = 0u128;
    while data.len() - offset > 128 {
        blake2b_compress(&mut state, &data[offset..offset + 128], counter + 128, false);
        counter += 128;
        offset += 128;
    }
    let remaining = data.len() - offset;
    let mut final_block = [0u8; 128];
    final_block[..remaining].copy_from_slice(&data[offset..]);
    counter += remaining as u128;
    blake2b_compress(&mut state, &final_block, counter, true);
    for i in 0..4 {
        output[i * 8..i * 8 + 8].copy_from_slice(&state[i].to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithm_ids_and_lengths() {
        assert_eq!(ChecksumAlgorithm::from_btrfs_type(0), Some(ChecksumAlgorithm::Crc32c));
        assert_eq!(ChecksumAlgorithm::from_btrfs_type(9), None);
        assert_eq!(ChecksumAlgorithm::Crc32c.digest_len(), 4);
        assert_eq!(ChecksumAlgorithm::XxHash64.digest_len(), 8);
        assert_eq!(ChecksumAlgorithm::Sha256.digest_len(), 32);
    }

    #[test]
    fn xxhash_vectors() {
        assert_eq!(digest(ChecksumAlgorithm::XxHash64, b"").as_bytes(), &[0x99, 0xe9, 0xd8, 0x51, 0x37, 0xdb, 0x46, 0xef]);
        assert_eq!(digest(ChecksumAlgorithm::XxHash64, b"hello").as_bytes(), &[0xa3, 0x6d, 0x9f, 0x88, 0x7d, 0x82, 0xc7, 0x26]);
    }

    #[test]
    fn sha256_vectors() {
        assert_eq!(
            digest(ChecksumAlgorithm::Sha256, b"hello").as_bytes(),
            &[
                0x2c, 0xf2, 0x4d, 0xba, 0x5f, 0xb0, 0xa3, 0x0e, 0x26, 0xe8, 0x3b, 0x2a,
                0xc5, 0xb9, 0xe2, 0x9e, 0x1b, 0x16, 0x1e, 0x5c, 0x1f, 0xa7, 0x42, 0x5e,
                0x73, 0x04, 0x33, 0x62, 0x93, 0x8b, 0x98, 0x24,
            ]
        );
    }

    #[test]
    fn blake2b_vectors() {
        assert_eq!(
            digest(ChecksumAlgorithm::Blake2b256, b"").as_bytes(),
            &[
                0x0e, 0x57, 0x51, 0xc0, 0x26, 0xe5, 0x43, 0xb2, 0xe8, 0xab, 0x2e, 0xb0,
                0x60, 0x99, 0xda, 0xa1, 0xd1, 0xe5, 0xdf, 0x47, 0x77, 0x8f, 0x77, 0x87,
                0xfa, 0xab, 0x45, 0xcd, 0xf1, 0x2f, 0xe3, 0xa8,
            ]
        );
    }

    #[test]
    fn verification_is_length_sensitive() {
        let d = digest(ChecksumAlgorithm::Sha256, b"hello");
        assert!(verify(ChecksumAlgorithm::Sha256, b"hello", d.as_bytes()));
        assert!(!verify(ChecksumAlgorithm::Sha256, b"Hello", d.as_bytes()));
        assert!(!verify(ChecksumAlgorithm::Sha256, b"hello", &d.as_bytes()[..31]));
    }
}

//! Logical → physical mapping for every btrfs profile, plus RAID5/6 parity math.
//!
//! Mirrors the kernel's `btrfs_map_block` for reads: single/DUP/RAID1/RAID1C3/RAID1C4 are plain
//! mirrors, RAID0 and RAID10 stripe in `stripe_len` units, RAID5/6 rotate the parity stripe by one
//! slot per full stripe (data slot `(full_stripe_nr + data_index) % num_stripes`, P at
//! `(full_stripe_nr + nr_data) % num_stripes`, Q right after P).
use crate::consts::*;
use alloc::vec;
use alloc::vec::Vec;

/// Allocation profile of a chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// One copy.
    Single,
    /// Two copies on the same device(s).
    Dup,
    /// Striped, no redundancy.
    Raid0,
    /// Two mirrors.
    Raid1,
    /// Three mirrors.
    Raid1c3,
    /// Four mirrors.
    Raid1c4,
    /// Striped mirrors (`sub_stripes` copies).
    Raid10,
    /// Single parity.
    Raid5,
    /// Double parity.
    Raid6,
}

impl Profile {
    /// Decodes the profile bits of a chunk/block-group type.
    pub fn from_flags(flags: u64) -> Option<Profile> {
        let p = flags & BLOCK_GROUP_PROFILE_MASK;
        Some(match p {
            0 => Profile::Single,
            BLOCK_GROUP_DUP => Profile::Dup,
            BLOCK_GROUP_RAID0 => Profile::Raid0,
            BLOCK_GROUP_RAID1 => Profile::Raid1,
            BLOCK_GROUP_RAID1C3 => Profile::Raid1c3,
            BLOCK_GROUP_RAID1C4 => Profile::Raid1c4,
            BLOCK_GROUP_RAID10 => Profile::Raid10,
            BLOCK_GROUP_RAID5 => Profile::Raid5,
            BLOCK_GROUP_RAID6 => Profile::Raid6,
            _ => return None,
        })
    }

    /// Number of parity stripes per full stripe.
    pub fn parity(self) -> usize {
        match self {
            Profile::Raid5 => 1,
            Profile::Raid6 => 2,
            _ => 0,
        }
    }

    /// Profile bit(s) for a chunk type word.
    pub fn flag(self) -> u64 {
        match self {
            Profile::Single => 0,
            Profile::Dup => BLOCK_GROUP_DUP,
            Profile::Raid0 => BLOCK_GROUP_RAID0,
            Profile::Raid1 => BLOCK_GROUP_RAID1,
            Profile::Raid1c3 => BLOCK_GROUP_RAID1C3,
            Profile::Raid1c4 => BLOCK_GROUP_RAID1C4,
            Profile::Raid10 => BLOCK_GROUP_RAID10,
            Profile::Raid5 => BLOCK_GROUP_RAID5,
            Profile::Raid6 => BLOCK_GROUP_RAID6,
        }
    }

    /// Fewest devices the profile can be created on.
    pub fn min_devices(self) -> usize {
        match self {
            Profile::Single | Profile::Dup => 1,
            Profile::Raid0 | Profile::Raid1 => 2,
            Profile::Raid1c3 => 3,
            Profile::Raid1c4 => 4,
            Profile::Raid10 => 2,
            Profile::Raid5 => 2,
            Profile::Raid6 => 3,
        }
    }

    /// Device failures tolerated per stripe set.
    pub fn tolerated_failures(self) -> usize {
        match self {
            Profile::Single | Profile::Raid0 => 0,
            Profile::Dup | Profile::Raid1 | Profile::Raid5 | Profile::Raid10 => 1,
            Profile::Raid6 | Profile::Raid1c3 => 2,
            Profile::Raid1c4 => 3,
        }
    }
}

/// One stripe of a chunk: where it lives on which device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stripe {
    /// Device id.
    pub devid: u64,
    /// Physical byte offset of the stripe's first byte.
    pub physical: u64,
}

/// A chunk's mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkMap {
    /// Logical start.
    pub logical: u64,
    /// Logical length.
    pub length: u64,
    /// Type + profile flags.
    pub flags: u64,
    /// Stripe unit (usually 64 KiB).
    pub stripe_len: u64,
    /// Stripe count.
    pub stripes: Vec<Stripe>,
    /// RAID10 sub-stripe count.
    pub sub_stripes: u16,
}

/// One mirror of a contiguous mapped range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Copy {
    /// Device id.
    pub devid: u64,
    /// Physical byte offset.
    pub physical: u64,
}

/// RAID5/6 reconstruction context for a mapped range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParityInfo {
    /// Index of this range's data stripe within the full stripe.
    pub data_index: usize,
    /// Number of data stripes.
    pub nr_data: usize,
    /// `[data0 .. dataN-1, P, (Q)]` physical start of each stripe of this full stripe.
    pub stripes: Vec<Copy>,
    /// Offset of the range inside the stripe.
    pub offset_in_stripe: u64,
    /// Stripe unit.
    pub stripe_len: u64,
    /// Full stripe number inside the chunk.
    pub full_stripe_nr: u64,
}

/// Result of mapping one logical position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mapped {
    /// Contiguous bytes available from the requested position.
    pub len: u64,
    /// Redundant copies (RAID5/6 and single have exactly one entry: the data stripe).
    pub copies: Vec<Copy>,
    /// Parity context for RAID5/6.
    pub parity: Option<ParityInfo>,
}

/// Mapping errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// Logical address outside the chunk.
    OutOfRange,
    /// Structurally invalid chunk (zero stripes, bad stripe length, unknown profile).
    BadChunk,
}

impl ChunkMap {
    /// The chunk's profile.
    pub fn profile(&self) -> Option<Profile> {
        Profile::from_flags(self.flags)
    }

    /// Validates stripe counts against the profile (what the kernel checks at mount).
    pub fn validate(&self) -> Result<(), MapError> {
        let p = self.profile().ok_or(MapError::BadChunk)?;
        let n = self.stripes.len();
        if n == 0 || self.length == 0 || self.stripe_len == 0 || !self.stripe_len.is_power_of_two() {
            return Err(MapError::BadChunk);
        }
        let ok = match p {
            Profile::Single => n == 1,
            Profile::Dup => n == 2,
            Profile::Raid1 => n == 2,
            Profile::Raid1c3 => n == 3,
            Profile::Raid1c4 => n == 4,
            Profile::Raid0 => n >= 1,
            Profile::Raid10 => self.sub_stripes >= 1 && n % self.sub_stripes as usize == 0,
            Profile::Raid5 => n >= 2,
            Profile::Raid6 => n >= 3,
        };
        if ok {
            Ok(())
        } else {
            Err(MapError::BadChunk)
        }
    }

    /// Number of data stripes in a full stripe.
    pub fn data_stripes(&self) -> usize {
        match self.profile() {
            Some(Profile::Raid0) => self.stripes.len(),
            Some(Profile::Raid10) => self.stripes.len() / (self.sub_stripes.max(1) as usize),
            Some(Profile::Raid5) => self.stripes.len() - 1,
            Some(Profile::Raid6) => self.stripes.len() - 2,
            _ => 1,
        }
    }

    /// Bytes each member stripe occupies on its device.
    pub fn stripe_dev_len(&self) -> u64 {
        match self.profile() {
            Some(Profile::Raid0) | Some(Profile::Raid10) | Some(Profile::Raid5) | Some(Profile::Raid6) => {
                let ds = self.data_stripes() as u64;
                // length is a multiple of ds*stripe_len for chunks btrfs creates; round up for safety
                let per = (self.length + ds - 1) / ds;
                (per + self.stripe_len - 1) / self.stripe_len * self.stripe_len
            }
            _ => self.length,
        }
    }

    /// Maps a logical address inside this chunk.
    pub fn map(&self, logical: u64) -> Result<Mapped, MapError> {
        self.validate()?;
        if logical < self.logical || logical >= self.logical + self.length {
            return Err(MapError::OutOfRange);
        }
        let off = logical - self.logical;
        let left_in_chunk = self.length - off;
        let sl = self.stripe_len;
        let n = self.stripes.len();
        match self.profile().unwrap() {
            Profile::Single | Profile::Dup | Profile::Raid1 | Profile::Raid1c3 | Profile::Raid1c4 => Ok(Mapped {
                len: left_in_chunk,
                copies: self.stripes.iter().map(|s| Copy { devid: s.devid, physical: s.physical + off }).collect(),
                parity: None,
            }),
            Profile::Raid0 => {
                let stripe_nr = off / sl;
                let so = off % sl;
                let idx = (stripe_nr % n as u64) as usize;
                let row = stripe_nr / n as u64;
                let s = self.stripes[idx];
                Ok(Mapped {
                    len: core::cmp::min(sl - so, left_in_chunk),
                    copies: vec![Copy { devid: s.devid, physical: s.physical + row * sl + so }],
                    parity: None,
                })
            }
            Profile::Raid10 => {
                let sub = self.sub_stripes as usize;
                let factor = n / sub;
                let stripe_nr = off / sl;
                let so = off % sl;
                let base = (stripe_nr % factor as u64) as usize * sub;
                let row = stripe_nr / factor as u64;
                let copies = (0..sub)
                    .map(|k| {
                        let s = self.stripes[base + k];
                        Copy { devid: s.devid, physical: s.physical + row * sl + so }
                    })
                    .collect();
                Ok(Mapped { len: core::cmp::min(sl - so, left_in_chunk), copies, parity: None })
            }
            Profile::Raid5 | Profile::Raid6 => {
                let np = if self.profile() == Some(Profile::Raid5) { 1 } else { 2 };
                let nr_data = n - np;
                let stripe_nr = off / sl;
                let so = off % sl;
                let data_index = (stripe_nr % nr_data as u64) as usize;
                let full = stripe_nr / nr_data as u64;
                let slot = |i: usize| -> Copy {
                    let s = self.stripes[((full + i as u64) % n as u64) as usize];
                    Copy { devid: s.devid, physical: s.physical + full * sl }
                };
                let stripes: Vec<Copy> = (0..n).map(slot).collect();
                let d = stripes[data_index];
                Ok(Mapped {
                    len: core::cmp::min(sl - so, left_in_chunk),
                    copies: vec![Copy { devid: d.devid, physical: d.physical + so }],
                    parity: Some(ParityInfo { data_index, nr_data, stripes, offset_in_stripe: so, stripe_len: sl, full_stripe_nr: full }),
                })
            }
        }
    }
}

// --------------------------------------------------------------------------------------------
// GF(2^8) with the RAID6 polynomial 0x11d, generator 2.

const fn gen_tables() -> ([u8; 512], [u8; 256]) {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11d;
        }
        i += 1;
    }
    let mut j = 255;
    while j < 512 {
        exp[j] = exp[j - 255];
        j += 1;
    }
    (exp, log)
}

const TABLES: ([u8; 512], [u8; 256]) = gen_tables();

/// Multiplication in GF(2^8)/0x11d.
pub fn gf_mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        TABLES.0[TABLES.1[a as usize] as usize + TABLES.1[b as usize] as usize]
    }
}

/// Multiplicative inverse (`a != 0`).
pub fn gf_inv(a: u8) -> u8 {
    debug_assert!(a != 0);
    TABLES.0[255 - TABLES.1[a as usize] as usize]
}

/// `2^i`.
pub fn gf_pow2(i: usize) -> u8 {
    TABLES.0[i % 255]
}

/// Computes P (and Q when `q` is given) over equally sized data stripes.
pub fn compute_parity(data: &[&[u8]], p: &mut [u8], mut q: Option<&mut [u8]>) {
    for b in p.iter_mut() {
        *b = 0;
    }
    if let Some(q) = q.as_deref_mut() {
        for b in q.iter_mut() {
            *b = 0;
        }
    }
    for (i, d) in data.iter().enumerate() {
        let coef = gf_pow2(i);
        for (j, &v) in d.iter().enumerate() {
            p[j] ^= v;
            if let Some(q) = q.as_deref_mut() {
                q[j] ^= gf_mul(coef, v);
            }
        }
    }
}

/// Failure to reconstruct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconError {
    /// More stripes are missing than the parity can cover.
    TooManyMissing,
    /// Buffers have inconsistent sizes.
    BadShape,
}

/// Reconstructs missing stripes in place. `stripes` is `[D0..Dn-1, P, (Q)]`; `None` marks a lost
/// stripe, which is replaced by the recovered content (lost parity is recomputed too).
pub fn reconstruct(stripes: &mut [Option<Vec<u8>>], nr_data: usize) -> Result<(), ReconError> {
    let np = stripes.len().checked_sub(nr_data).ok_or(ReconError::BadShape)?;
    if np == 0 || np > 2 {
        return Err(ReconError::BadShape);
    }
    let len = match stripes.iter().flatten().next() {
        Some(v) => v.len(),
        None => return Err(ReconError::TooManyMissing),
    };
    if stripes.iter().flatten().any(|v| v.len() != len) {
        return Err(ReconError::BadShape);
    }
    let missing: Vec<usize> = (0..stripes.len()).filter(|&i| stripes[i].is_none()).collect();
    if missing.is_empty() {
        return Ok(());
    }
    if missing.len() > np {
        return Err(ReconError::TooManyMissing);
    }
    let missing_data: Vec<usize> = missing.iter().copied().filter(|&i| i < nr_data).collect();
    let p_idx = nr_data;
    let q_idx = nr_data + 1;
    let have_p = |s: &[Option<Vec<u8>>]| s[p_idx].is_some();
    let have_q = |s: &[Option<Vec<u8>>]| np == 2 && s[q_idx].is_some();

    // Partial sums over the surviving data stripes.
    let partial = |s: &[Option<Vec<u8>>], skip: &[usize]| -> (Vec<u8>, Vec<u8>) {
        let mut pp = vec![0u8; len];
        let mut qq = vec![0u8; len];
        for i in 0..nr_data {
            if skip.contains(&i) {
                continue;
            }
            if let Some(d) = &s[i] {
                let c = gf_pow2(i);
                for j in 0..len {
                    pp[j] ^= d[j];
                    qq[j] ^= gf_mul(c, d[j]);
                }
            }
        }
        (pp, qq)
    };

    match missing_data.len() {
        0 => {}
        1 => {
            let x = missing_data[0];
            if have_p(stripes) {
                let (pp, _) = partial(stripes, &[x]);
                let mut d = stripes[p_idx].clone().unwrap();
                for j in 0..len {
                    d[j] ^= pp[j];
                }
                stripes[x] = Some(d);
            } else if have_q(stripes) {
                let (_, qq) = partial(stripes, &[x]);
                let inv = gf_inv(gf_pow2(x));
                let mut d = stripes[q_idx].clone().unwrap();
                for j in 0..len {
                    d[j] = gf_mul(inv, d[j] ^ qq[j]);
                }
                stripes[x] = Some(d);
            } else {
                return Err(ReconError::TooManyMissing);
            }
        }
        2 => {
            if !(have_p(stripes) && have_q(stripes)) {
                return Err(ReconError::TooManyMissing);
            }
            let (x, y) = (missing_data[0], missing_data[1]);
            let (pp, qq) = partial(stripes, &[x, y]);
            let pxy: Vec<u8> = (0..len).map(|j| stripes[p_idx].as_ref().unwrap()[j] ^ pp[j]).collect();
            let qxy: Vec<u8> = (0..len).map(|j| stripes[q_idx].as_ref().unwrap()[j] ^ qq[j]).collect();
            let (gx, gy) = (gf_pow2(x), gf_pow2(y));
            let denom_inv = gf_inv(gx ^ gy);
            let mut dx = vec![0u8; len];
            let mut dy = vec![0u8; len];
            for j in 0..len {
                let a = qxy[j] ^ gf_mul(gy, pxy[j]);
                dx[j] = gf_mul(a, denom_inv);
                dy[j] = pxy[j] ^ dx[j];
            }
            stripes[x] = Some(dx);
            stripes[y] = Some(dy);
        }
        _ => return Err(ReconError::TooManyMissing),
    }
    // Recompute lost parity from the (now complete) data.
    if missing.iter().any(|&i| i >= nr_data) {
        let data: Vec<&[u8]> = (0..nr_data).map(|i| stripes[i].as_deref().unwrap()).collect();
        let mut p = vec![0u8; len];
        let mut q = vec![0u8; len];
        compute_parity(&data, &mut p, if np == 2 { Some(&mut q) } else { None });
        if stripes[p_idx].is_none() {
            stripes[p_idx] = Some(p);
        }
        if np == 2 && stripes[q_idx].is_none() {
            stripes[q_idx] = Some(q);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeSet;

    fn chunk(flags: u64, n: usize, sub: u16, length: u64) -> ChunkMap {
        ChunkMap {
            logical: 1 << 30,
            length,
            flags,
            stripe_len: STRIPE_LEN,
            stripes: (0..n).map(|i| Stripe { devid: i as u64 + 1, physical: (i as u64 + 1) * (1 << 28) }).collect(),
            sub_stripes: sub,
        }
    }

    /// Every data byte maps to a distinct location inside its member stripe, copies are consistent.
    fn check_bijection(c: &ChunkMap) {
        let dev_len = c.stripe_dev_len();
        let mut seen: BTreeSet<(u64, u64)> = BTreeSet::new();
        let step = 4096u64;
        let mut pos = 0;
        while pos < c.length {
            let m = c.map(c.logical + pos).unwrap();
            assert!(m.len >= step.min(c.length - pos), "mapping returns at least a sector at {}", pos);
            for cp in &m.copies {
                let s = c.stripes.iter().find(|s| s.devid == cp.devid && cp.physical >= s.physical && cp.physical < s.physical + dev_len);
                assert!(s.is_some(), "copy outside its stripe: {:?} at {}", cp, pos);
            }
            if m.parity.is_none() && matches!(c.profile(), Some(Profile::Raid0) | Some(Profile::Raid10)) || m.parity.is_some() {
                for cp in &m.copies {
                    assert!(seen.insert((cp.devid, cp.physical)), "{:?}: duplicate physical {:?} at {}", c.profile(), cp, pos);
                }
            }
            pos += step;
        }
    }

    #[test]
    fn striping_profiles_are_injective() {
        check_bijection(&chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID0, 3, 0, 3 * 4 * STRIPE_LEN * 2));
        check_bijection(&chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID10, 4, 2, 2 * 4 * STRIPE_LEN * 3));
        check_bijection(&chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID5, 4, 0, 3 * STRIPE_LEN * 5));
        check_bijection(&chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID6, 5, 0, 3 * STRIPE_LEN * 7));
        check_bijection(&chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID5, 2, 0, STRIPE_LEN * 6));
    }

    #[test]
    fn raid0_matches_hand_computed_layout() {
        let c = chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID0, 3, 0, 3 * STRIPE_LEN * 4);
        // stripe_nr 0 -> dev1 row0, 1 -> dev2 row0, 2 -> dev3 row0, 3 -> dev1 row1
        for (stripe_nr, devid, row) in [(0u64, 1u64, 0u64), (1, 2, 0), (2, 3, 0), (3, 1, 1), (7, 2, 2)] {
            let m = c.map(c.logical + stripe_nr * STRIPE_LEN + 100).unwrap();
            assert_eq!(m.copies.len(), 1);
            assert_eq!(m.copies[0].devid, devid);
            assert_eq!(m.copies[0].physical, devid * (1 << 28) + row * STRIPE_LEN + 100);
            assert_eq!(m.len, STRIPE_LEN - 100);
        }
    }

    #[test]
    fn raid10_pairs_and_rows() {
        let c = chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID10, 4, 2, 2 * STRIPE_LEN * 4);
        // factor = 2 pairs: stripe_nr 0 -> (dev1,dev2) row0, 1 -> (dev3,dev4) row0, 2 -> (dev1,dev2) row1
        let m = c.map(c.logical + 2 * STRIPE_LEN).unwrap();
        assert_eq!(m.copies.iter().map(|x| x.devid).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(m.copies[0].physical, (1 << 28) + STRIPE_LEN);
        let m = c.map(c.logical + STRIPE_LEN).unwrap();
        assert_eq!(m.copies.iter().map(|x| x.devid).collect::<Vec<_>>(), vec![3, 4]);
    }

    #[test]
    fn raid5_parity_rotates_one_slot_per_full_stripe() {
        let c = chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID5, 4, 0, 3 * STRIPE_LEN * 6);
        // 3 data stripes, 4 devices. full stripe f: data i on slot (f+i)%4, parity on slot (f+3)%4.
        for f in 0..6u64 {
            for i in 0..3u64 {
                let m = c.map(c.logical + (f * 3 + i) * STRIPE_LEN).unwrap();
                let slot = (f + i) % 4;
                assert_eq!(m.copies[0].devid, slot + 1);
                assert_eq!(m.copies[0].physical, (slot + 1) * (1 << 28) + f * STRIPE_LEN);
                let par = m.parity.unwrap();
                assert_eq!(par.data_index as u64, i);
                assert_eq!(par.stripes[3].devid, (f + 3) % 4 + 1, "P slot for full stripe {}", f);
            }
        }
    }

    #[test]
    fn raid6_q_follows_p() {
        let c = chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID6, 5, 0, 3 * STRIPE_LEN * 4);
        let m = c.map(c.logical + 3 * STRIPE_LEN).unwrap(); // full stripe 1, data 0
        let par = m.parity.unwrap();
        assert_eq!(par.nr_data, 3);
        assert_eq!(par.stripes[3].devid, (1 + 3) % 5 + 1);
        assert_eq!(par.stripes[4].devid, (1 + 4) % 5 + 1);
    }

    #[test]
    fn bad_chunks_are_rejected() {
        assert_eq!(chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID1, 3, 0, 1 << 20).map(1 << 30), Err(MapError::BadChunk));
        assert_eq!(chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID10, 3, 2, 1 << 20).map(1 << 30), Err(MapError::BadChunk));
        assert_eq!(chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID6, 2, 0, 1 << 20).map(1 << 30), Err(MapError::BadChunk));
        assert_eq!(chunk(BLOCK_GROUP_DATA, 1, 0, 1 << 20).map(0), Err(MapError::OutOfRange));
        assert_eq!(chunk(BLOCK_GROUP_DATA, 1, 0, 1 << 20).map((1 << 30) + (1 << 20)), Err(MapError::OutOfRange));
        assert!(chunk(BLOCK_GROUP_DATA | BLOCK_GROUP_RAID1 | BLOCK_GROUP_RAID0, 2, 0, 1 << 20).validate().is_err());
    }

    #[test]
    fn gf_field_axioms() {
        for a in 1..=255u8 {
            assert_eq!(gf_mul(a, gf_inv(a)), 1);
            assert_eq!(gf_mul(a, 1), a);
            for b in [3u8, 7, 0x1d, 0x80, 0xff] {
                assert_eq!(gf_mul(a, b), gf_mul(b, a));
            }
        }
        assert_eq!(gf_mul(2, 0x80), 0x1d); // x * x^7 reduces by 0x11d
        assert_eq!(gf_pow2(8), 0x1d);
    }

    fn pseudo(seed: u64, len: usize) -> Vec<u8> {
        let mut x = seed | 1;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 3) as u8
            })
            .collect()
    }

    #[test]
    fn raid5_and_raid6_recover_every_loss_pattern() {
        for (nr_data, np) in [(2usize, 1usize), (3, 1), (4, 2), (5, 2), (2, 2)] {
            let data: Vec<Vec<u8>> = (0..nr_data).map(|i| pseudo(i as u64 + 1, 4096)).collect();
            let refs: Vec<&[u8]> = data.iter().map(|d| d.as_slice()).collect();
            let mut p = vec![0u8; 4096];
            let mut q = vec![0u8; 4096];
            compute_parity(&refs, &mut p, if np == 2 { Some(&mut q) } else { None });
            let mut full: Vec<Vec<u8>> = data.clone();
            full.push(p);
            if np == 2 {
                full.push(q);
            }
            let total = nr_data + np;
            for mask in 0u32..(1 << total) {
                let lost: Vec<usize> = (0..total).filter(|i| mask & (1 << i) != 0).collect();
                let mut s: Vec<Option<Vec<u8>>> = full.iter().enumerate().map(|(i, v)| if lost.contains(&i) { None } else { Some(v.clone()) }).collect();
                let r = reconstruct(&mut s, nr_data);
                if lost.len() > np {
                    assert_eq!(r, Err(ReconError::TooManyMissing));
                } else {
                    r.unwrap();
                    for i in 0..total {
                        assert_eq!(s[i].as_ref().unwrap(), &full[i], "nr_data {} np {} lost {:?} stripe {}", nr_data, np, lost, i);
                    }
                }
            }
        }
    }
}

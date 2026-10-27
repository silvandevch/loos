//! AES-128/256 (table-free byte implementation), XTS, GCM and RFC 3394 key wrap.
//!
//! Byte-oriented and not constant-time against cache attacks; acceptable for the software
//! fallback (AES-NI backends can replace [`Aes`] behind the same API).

use crate::{Error, Result};
use alloc::vec::Vec;

const fn gen_sbox() -> ([u8; 256], [u8; 256]) {
    let mut s = [0u8; 256];
    let mut p: u8 = 1;
    let mut q: u8 = 1;
    loop {
        p = p ^ (p << 1) ^ if p & 0x80 != 0 { 0x1b } else { 0 };
        q ^= q << 1;
        q ^= q << 2;
        q ^= q << 4;
        if q & 0x80 != 0 {
            q ^= 0x09;
        }
        s[p as usize] = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4) ^ 0x63;
        if p == 1 {
            break;
        }
    }
    s[0] = 0x63;
    let mut inv = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        inv[s[i] as usize] = i as u8;
        i += 1;
    }
    (s, inv)
}
const SBOXES: ([u8; 256], [u8; 256]) = gen_sbox();
const SBOX: [u8; 256] = SBOXES.0;
const INV: [u8; 256] = SBOXES.1;

fn xt(x: u8) -> u8 {
    (x << 1) ^ if x & 0x80 != 0 { 0x1b } else { 0 }
}
fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut r = 0;
    while b != 0 {
        if b & 1 != 0 {
            r ^= a;
        }
        a = xt(a);
        b >>= 1;
    }
    r
}

/// An expanded AES key (128 or 256 bit).
#[derive(Clone)]
pub struct Aes {
    rk: [[u8; 16]; 15],
    rounds: usize,
}

impl Aes {
    /// Expand a 16- or 32-byte key.
    pub fn new(key: &[u8]) -> Result<Self> {
        let nk = match key.len() {
            16 => 4,
            32 => 8,
            _ => return Err(Error::Invalid),
        };
        let rounds = nk + 6;
        let mut w = [[0u8; 4]; 60];
        for i in 0..nk {
            w[i].copy_from_slice(&key[i * 4..i * 4 + 4]);
        }
        let mut rcon = 1u8;
        for i in nk..4 * (rounds + 1) {
            let mut t = w[i - 1];
            if i % nk == 0 {
                t = [SBOX[t[1] as usize] ^ rcon, SBOX[t[2] as usize], SBOX[t[3] as usize], SBOX[t[0] as usize]];
                rcon = xt(rcon);
            } else if nk == 8 && i % nk == 4 {
                t = t.map(|b| SBOX[b as usize]);
            }
            for j in 0..4 {
                w[i][j] = w[i - nk][j] ^ t[j];
            }
        }
        let mut rk = [[0u8; 16]; 15];
        for r in 0..=rounds {
            for c in 0..4 {
                rk[r][c * 4..c * 4 + 4].copy_from_slice(&w[r * 4 + c]);
            }
        }
        Ok(Aes { rk, rounds })
    }

    /// Encrypt one block in place.
    pub fn encrypt(&self, b: &mut [u8; 16]) {
        xor(b, &self.rk[0]);
        for r in 1..=self.rounds {
            for x in b.iter_mut() {
                *x = SBOX[*x as usize];
            }
            let t = *b;
            for c in 0..4 {
                for row in 0..4 {
                    b[c * 4 + row] = t[((c + row) % 4) * 4 + row];
                }
            }
            if r != self.rounds {
                for c in 0..4 {
                    let a = [b[c * 4], b[c * 4 + 1], b[c * 4 + 2], b[c * 4 + 3]];
                    b[c * 4] = xt(a[0]) ^ (xt(a[1]) ^ a[1]) ^ a[2] ^ a[3];
                    b[c * 4 + 1] = a[0] ^ xt(a[1]) ^ (xt(a[2]) ^ a[2]) ^ a[3];
                    b[c * 4 + 2] = a[0] ^ a[1] ^ xt(a[2]) ^ (xt(a[3]) ^ a[3]);
                    b[c * 4 + 3] = (xt(a[0]) ^ a[0]) ^ a[1] ^ a[2] ^ xt(a[3]);
                }
            }
            xor(b, &self.rk[r]);
        }
    }

    /// Decrypt one block in place.
    pub fn decrypt(&self, b: &mut [u8; 16]) {
        xor(b, &self.rk[self.rounds]);
        for r in (0..self.rounds).rev() {
            let t = *b;
            for c in 0..4 {
                for row in 0..4 {
                    b[((c + row) % 4) * 4 + row] = t[c * 4 + row];
                }
            }
            for x in b.iter_mut() {
                *x = INV[*x as usize];
            }
            xor(b, &self.rk[r]);
            if r != 0 {
                for c in 0..4 {
                    let a = [b[c * 4], b[c * 4 + 1], b[c * 4 + 2], b[c * 4 + 3]];
                    b[c * 4] = mul(a[0], 14) ^ mul(a[1], 11) ^ mul(a[2], 13) ^ mul(a[3], 9);
                    b[c * 4 + 1] = mul(a[0], 9) ^ mul(a[1], 14) ^ mul(a[2], 11) ^ mul(a[3], 13);
                    b[c * 4 + 2] = mul(a[0], 13) ^ mul(a[1], 9) ^ mul(a[2], 14) ^ mul(a[3], 11);
                    b[c * 4 + 3] = mul(a[0], 11) ^ mul(a[1], 13) ^ mul(a[2], 9) ^ mul(a[3], 14);
                }
            }
        }
    }
}

fn xor(a: &mut [u8; 16], b: &[u8; 16]) {
    for (x, y) in a.iter_mut().zip(b) {
        *x ^= y;
    }
}

// ---------- XTS ----------

/// AES-XTS with two independent keys (disk sector encryption).
pub struct Xts {
    data: Aes,
    tweak: Aes,
}

fn xts_mul_alpha(t: &mut [u8; 16]) {
    let carry = t[15] >> 7;
    for i in (1..16).rev() {
        t[i] = (t[i] << 1) | (t[i - 1] >> 7);
    }
    t[0] = (t[0] << 1) ^ if carry != 0 { 0x87 } else { 0 };
}

impl Xts {
    /// `key` is data-key || tweak-key (32 or 64 bytes).
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.len() != 32 && key.len() != 64 {
            return Err(Error::Invalid);
        }
        let h = key.len() / 2;
        Ok(Xts { data: Aes::new(&key[..h])?, tweak: Aes::new(&key[h..])? })
    }

    fn run(&self, sector: u64, buf: &mut [u8], enc: bool) -> Result<()> {
        if buf.len() < 16 || buf.len() % 16 != 0 {
            return Err(Error::Invalid); // ciphertext stealing unnecessary for sector multiples
        }
        let mut t = [0u8; 16];
        t[..8].copy_from_slice(&sector.to_le_bytes());
        self.tweak.encrypt(&mut t);
        for blk in buf.chunks_exact_mut(16) {
            let mut b: [u8; 16] = blk.try_into().unwrap();
            xor(&mut b, &t);
            if enc {
                self.data.encrypt(&mut b);
            } else {
                self.data.decrypt(&mut b);
            }
            xor(&mut b, &t);
            blk.copy_from_slice(&b);
            xts_mul_alpha(&mut t);
        }
        Ok(())
    }

    /// Encrypt a sector-aligned buffer in place.
    pub fn encrypt_sector(&self, sector: u64, buf: &mut [u8]) -> Result<()> {
        self.run(sector, buf, true)
    }

    /// Decrypt a sector-aligned buffer in place.
    pub fn decrypt_sector(&self, sector: u64, buf: &mut [u8]) -> Result<()> {
        self.run(sector, buf, false)
    }
}

// ---------- GCM ----------

fn gmul(x: u128, y: u128) -> u128 {
    let mut z = 0u128;
    let mut v = y;
    for i in 0..128 {
        if (x >> (127 - i)) & 1 == 1 {
            z ^= v;
        }
        v = if v & 1 == 1 { (v >> 1) ^ (0xe1u128 << 120) } else { v >> 1 };
    }
    z
}

fn ghash(h: u128, aad: &[u8], ct: &[u8]) -> u128 {
    let mut y = 0u128;
    for data in [aad, ct] {
        for c in data.chunks(16) {
            let mut b = [0u8; 16];
            b[..c.len()].copy_from_slice(c);
            y = gmul(y ^ u128::from_be_bytes(b), h);
        }
    }
    gmul(y ^ ((aad.len() as u128 * 8) << 64 | ct.len() as u128 * 8), h)
}

/// AES-GCM with a 96-bit nonce and 128-bit tag.
pub struct Gcm {
    aes: Aes,
    h: u128,
}

impl Gcm {
    /// New context from a 16/32-byte key.
    pub fn new(key: &[u8]) -> Result<Self> {
        let aes = Aes::new(key)?;
        let mut z = [0u8; 16];
        aes.encrypt(&mut z);
        Ok(Gcm { aes, h: u128::from_be_bytes(z) })
    }

    fn ctr(&self, nonce: &[u8; 12], data: &mut [u8]) {
        for (i, chunk) in data.chunks_mut(16).enumerate() {
            let mut cb = [0u8; 16];
            cb[..12].copy_from_slice(nonce);
            cb[12..].copy_from_slice(&(i as u32 + 2).to_be_bytes());
            self.aes.encrypt(&mut cb);
            for (d, k) in chunk.iter_mut().zip(cb) {
                *d ^= k;
            }
        }
    }

    fn tag(&self, nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> [u8; 16] {
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(nonce);
        j0[15] = 1;
        self.aes.encrypt(&mut j0);
        (ghash(self.h, aad, ct) ^ u128::from_be_bytes(j0)).to_be_bytes()
    }

    /// Encrypt `data` in place and return the tag.
    pub fn seal(&self, nonce: &[u8; 12], aad: &[u8], data: &mut [u8]) -> [u8; 16] {
        self.ctr(nonce, data);
        self.tag(nonce, aad, data)
    }

    /// Verify `tag` then decrypt in place.
    pub fn open(&self, nonce: &[u8; 12], aad: &[u8], data: &mut [u8], tag: &[u8]) -> Result<()> {
        if !crate::hash::ct_eq(&self.tag(nonce, aad, data), tag) {
            return Err(Error::BadSignature);
        }
        self.ctr(nonce, data);
        Ok(())
    }
}

// ---------- RFC 3394 key wrap ----------

const IV: [u8; 8] = [0xa6; 8];

/// Wrap `key` (multiple of 8 bytes, >= 16) under `kek`.
pub fn key_wrap(kek: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    if key.len() < 16 || key.len() % 8 != 0 {
        return Err(Error::Invalid);
    }
    let aes = Aes::new(kek)?;
    let n = key.len() / 8;
    let mut a = IV;
    let mut r: Vec<[u8; 8]> = key.chunks(8).map(|c| c.try_into().unwrap()).collect();
    for j in 0..6 {
        for i in 0..n {
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&a);
            b[8..].copy_from_slice(&r[i]);
            aes.encrypt(&mut b);
            let t = (n * j + i + 1) as u64;
            a = (u64::from_be_bytes(b[..8].try_into().unwrap()) ^ t).to_be_bytes();
            r[i].copy_from_slice(&b[8..]);
        }
    }
    let mut out = a.to_vec();
    for x in r {
        out.extend_from_slice(&x);
    }
    Ok(out)
}

/// Unwrap; fails with [`Error::BadCredential`] if the KEK is wrong or the blob was altered.
pub fn key_unwrap(kek: &[u8], wrapped: &[u8]) -> Result<Vec<u8>> {
    if wrapped.len() < 24 || wrapped.len() % 8 != 0 {
        return Err(Error::Invalid);
    }
    let aes = Aes::new(kek)?;
    let n = wrapped.len() / 8 - 1;
    let mut a: [u8; 8] = wrapped[..8].try_into().unwrap();
    let mut r: Vec<[u8; 8]> = wrapped[8..].chunks(8).map(|c| c.try_into().unwrap()).collect();
    for j in (0..6).rev() {
        for i in (0..n).rev() {
            let t = (n * j + i + 1) as u64;
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&(u64::from_be_bytes(a) ^ t).to_be_bytes());
            b[8..].copy_from_slice(&r[i]);
            aes.decrypt(&mut b);
            a.copy_from_slice(&b[..8]);
            r[i].copy_from_slice(&b[8..]);
        }
    }
    if !crate::hash::ct_eq(&a, &IV) {
        return Err(Error::BadCredential);
    }
    Ok(r.concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hex;

    #[test]
    fn fips197() {
        let pt = hex("00112233445566778899aabbccddeeff");
        for (key, ct) in [
            ("000102030405060708090a0b0c0d0e0f", "69c4e0d86a7b0430d8cdb78070b4c55a"),
            ("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f", "8ea2b7ca516745bfeafc49904b496089"),
        ] {
            let a = Aes::new(&hex(key)).unwrap();
            let mut b: [u8; 16] = pt.clone().try_into().unwrap();
            a.encrypt(&mut b);
            assert_eq!(b.to_vec(), hex(ct));
            a.decrypt(&mut b);
            assert_eq!(b.to_vec(), pt);
        }
        assert!(Aes::new(&[0; 24]).is_err());
    }

    #[test]
    fn gcm_vectors() {
        let g = Gcm::new(&[0; 16]).unwrap();
        let mut d = [0u8; 16];
        let tag = g.seal(&[0; 12], &[], &mut d);
        assert_eq!(d.to_vec(), hex("0388dace60b6a392f328c2b971b2fe78"));
        assert_eq!(tag.to_vec(), hex("ab6e47d42cec13bdf53a67b21257bddf"));
        g.open(&[0; 12], &[], &mut d, &tag).unwrap();
        assert_eq!(d, [0; 16]);
        let mut d2 = [1u8; 40];
        let t = g.seal(&[7; 12], b"hdr", &mut d2);
        let mut bad = d2;
        bad[3] ^= 1;
        assert_eq!(g.open(&[7; 12], b"hdr", &mut bad, &t), Err(Error::BadSignature));
        assert_eq!(g.open(&[7; 12], b"HDR", &mut d2, &t), Err(Error::BadSignature));
        g.open(&[7; 12], b"hdr", &mut d2, &t).unwrap();
        assert_eq!(d2, [1; 40]);
    }

    #[test]
    fn keywrap_rfc3394() {
        let w = key_wrap(&hex("000102030405060708090a0b0c0d0e0f"), &hex("00112233445566778899aabbccddeeff")).unwrap();
        assert_eq!(w, hex("1fa68b0a8112b447aef34bd8fb5a7b829d3e862371d2cfe5"));
        assert_eq!(key_unwrap(&hex("000102030405060708090a0b0c0d0e0f"), &w).unwrap(), hex("00112233445566778899aabbccddeeff"));
        assert_eq!(key_unwrap(&[1; 16], &w), Err(Error::BadCredential));
    }

    #[test]
    fn xts_roundtrip_and_sector_binding() {
        let x = Xts::new(&[0x42; 64]).unwrap();
        let orig: Vec<u8> = (0..512u32).map(|i| i as u8).collect();
        let mut a = orig.clone();
        x.encrypt_sector(7, &mut a).unwrap();
        assert_ne!(a, orig);
        let mut b = orig.clone();
        x.encrypt_sector(8, &mut b).unwrap();
        assert_ne!(a, b);
        x.decrypt_sector(7, &mut a).unwrap();
        assert_eq!(a, orig);
        assert!(x.encrypt_sector(0, &mut [0u8; 17]).is_err());
        // IEEE 1619 vector 1: zero keys, sector 0, 32 zero bytes
        let x = Xts::new(&[0; 32]).unwrap();
        let mut z = [0u8; 32];
        x.encrypt_sector(0, &mut z).unwrap();
        assert_eq!(z.to_vec(), hex("917cf69ebd68b2ec9b9fe9a3eadda692cd43d2f59598ed858c02c2652fbf922e"));
    }
}

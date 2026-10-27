//! Minimal unsigned big integers for RSA public-key verification.

use alloc::vec::Vec;

/// Little-endian base-2^32 unsigned integer (no trailing zero limbs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uint(Vec<u32>);

impl Uint {
    /// From big-endian bytes.
    pub fn from_be(b: &[u8]) -> Self {
        let mut v: Vec<u32> = b.rchunks(4).map(|c| c.iter().fold(0u32, |a, &x| a << 8 | x as u32)).collect();
        while v.last() == Some(&0) {
            v.pop();
        }
        Uint(v)
    }

    /// Small constant.
    pub fn from_u32(x: u32) -> Self {
        Uint(if x == 0 { Vec::new() } else { alloc::vec![x] })
    }

    /// To big-endian bytes left-padded to `len`.
    pub fn to_be(&self, len: usize) -> Vec<u8> {
        let mut out = alloc::vec![0u8; len];
        for (i, l) in self.0.iter().enumerate() {
            for j in 0..4 {
                let pos = i * 4 + j;
                if pos < len {
                    out[len - 1 - pos] = (l >> (8 * j)) as u8;
                }
            }
        }
        out
    }

    /// True for zero.
    pub fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    fn bits(&self) -> usize {
        self.0.last().map_or(0, |l| self.0.len() * 32 - l.leading_zeros() as usize)
    }

    fn bit(&self, i: usize) -> bool {
        self.0.get(i / 32).is_some_and(|l| l >> (i % 32) & 1 == 1)
    }

    fn cmp(&self, o: &Uint) -> core::cmp::Ordering {
        self.0.len().cmp(&o.0.len()).then_with(|| self.0.iter().rev().cmp(o.0.iter().rev()))
    }

    fn sub_assign(&mut self, o: &Uint) {
        let mut borrow = 0i64;
        for i in 0..self.0.len() {
            let d = self.0[i] as i64 - *o.0.get(i).unwrap_or(&0) as i64 - borrow;
            self.0[i] = d as u32;
            borrow = (d < 0) as i64;
        }
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    fn shl1_or(&mut self, bit: bool) {
        let mut carry = bit as u32;
        for l in self.0.iter_mut() {
            let n = *l >> 31;
            *l = *l << 1 | carry;
            carry = n;
        }
        if carry != 0 {
            self.0.push(carry);
        }
    }

    fn mul(&self, o: &Uint) -> Uint {
        let mut r = alloc::vec![0u32; self.0.len() + o.0.len() + 1];
        for (i, &a) in self.0.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in o.0.iter().enumerate() {
                let t = r[i + j] as u64 + a as u64 * b as u64 + carry;
                r[i + j] = t as u32;
                carry = t >> 32;
            }
            r[i + o.0.len()] = carry as u32;
        }
        while r.last() == Some(&0) {
            r.pop();
        }
        Uint(r)
    }

    /// `self mod m` by shift-subtract (m != 0).
    pub fn rem(&self, m: &Uint) -> Uint {
        let mut r = Uint(Vec::new());
        for i in (0..self.bits()).rev() {
            r.shl1_or(self.bit(i));
            if r.cmp(m) != core::cmp::Ordering::Less {
                r.sub_assign(m);
            }
        }
        r
    }

    /// `self^e mod m` (square-and-multiply; public exponents only).
    pub fn modpow(&self, e: &Uint, m: &Uint) -> Uint {
        let mut result = Uint::from_u32(1).rem(m);
        let base = self.rem(m);
        for i in (0..e.bits()).rev() {
            result = result.mul(&result).rem(m);
            if e.bit(i) {
                result = result.mul(&base).rem(m);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modpow_small_and_big() {
        let r = Uint::from_u32(4).modpow(&Uint::from_u32(13), &Uint::from_u32(497));
        assert_eq!(r, Uint::from_u32(445));
        // Fermat: 2^(p-1) mod p == 1 for p = 2^127-1
        let p = Uint::from_be(&[0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        let mut pm1 = p.clone();
        pm1.sub_assign(&Uint::from_u32(1));
        assert_eq!(Uint::from_u32(2).modpow(&pm1, &p), Uint::from_u32(1));
        assert_eq!(Uint::from_be(&[0, 0, 1, 2]).to_be(4), alloc::vec![0, 0, 1, 2]);
        assert!(Uint::from_be(&[0, 0]).is_zero());
    }
}

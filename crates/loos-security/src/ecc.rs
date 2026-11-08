//! ECDSA signature verification on NIST P-256 and P-384 (FIPS 186-4), for certificate chains and TLS CertificateVerify.
//!
//! Verification only: every input is public, so the arithmetic is plain variable-time Montgomery multiplication with
//! Jacobian points and a joint (Shamir) double-and-add. Nothing here ever handles a secret.

use crate::{Error, Result};
use alloc::vec::Vec;

const L: usize = 6;
type U = [u64; L];

/// The two supported curves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    /// secp256r1 / prime256v1.
    P256,
    /// secp384r1.
    P384,
}

struct Params {
    k: usize,
    p: &'static str,
    b: &'static str,
    gx: &'static str,
    gy: &'static str,
    n: &'static str,
}

const P256: Params = Params {
    k: 4,
    p: "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
    b: "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b",
    gx: "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
    gy: "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
    n: "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
};

const P384: Params = Params {
    k: 6,
    p: "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
    b: "b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef",
    gx: "aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7",
    gy: "3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
    n: "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973",
};

fn from_hex(s: &str) -> U {
    let mut out = [0u64; L];
    let bytes: Vec<u8> = (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or(0)).collect();
    let be = &bytes;
    for (i, b) in be.iter().rev().enumerate() {
        out[i / 8] |= u64::from(*b) << (8 * (i % 8));
    }
    out
}

fn from_be(b: &[u8]) -> Option<U> {
    if b.len() > L * 8 {
        return None;
    }
    let mut out = [0u64; L];
    for (i, x) in b.iter().rev().enumerate() {
        out[i / 8] |= u64::from(*x) << (8 * (i % 8));
    }
    Some(out)
}

fn cmp(a: &U, b: &U) -> core::cmp::Ordering {
    for i in (0..L).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    core::cmp::Ordering::Equal
}

fn is_zero(a: &U) -> bool {
    a.iter().all(|&x| x == 0)
}

/// a + b; returns the carry out.
fn add_raw(a: &U, b: &U) -> (U, bool) {
    let mut r = [0u64; L];
    let mut c = false;
    for i in 0..L {
        let (s1, c1) = a[i].overflowing_add(b[i]);
        let (s2, c2) = s1.overflowing_add(u64::from(c));
        r[i] = s2;
        c = c1 || c2;
    }
    (r, c)
}

fn sub_raw(a: &U, b: &U) -> (U, bool) {
    let mut r = [0u64; L];
    let mut br = false;
    for i in 0..L {
        let (s1, b1) = a[i].overflowing_sub(b[i]);
        let (s2, b2) = s1.overflowing_sub(u64::from(br));
        r[i] = s2;
        br = b1 || b2;
    }
    (r, br)
}

/// Arithmetic modulo an odd modulus in Montgomery form.
struct Mont {
    m: U,
    k: usize,
    n0: u64,
    r2: U,
    one: U,
}

impl Mont {
    fn new(m: U, k: usize) -> Mont {
        let mut inv = 1u64;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(m[0].wrapping_mul(inv)));
        }
        let mut z = Mont { m, k, n0: inv.wrapping_neg(), r2: [0; L], one: [0; L] };
        // R mod m by doubling 1 for 64k steps, then R^2 by another 64k.
        let mut r = [0u64; L];
        r[0] = 1;
        for _ in 0..64 * k {
            r = z.add(&r, &r);
        }
        z.one = r;
        for _ in 0..64 * k {
            r = z.add(&r, &r);
        }
        z.r2 = r;
        z
    }

    fn add(&self, a: &U, b: &U) -> U {
        let (s, c) = add_raw(a, b);
        if c || cmp(&s, &self.m) != core::cmp::Ordering::Less {
            sub_raw(&s, &self.m).0
        } else {
            s
        }
    }

    fn sub(&self, a: &U, b: &U) -> U {
        let (d, br) = sub_raw(a, b);
        if br {
            add_raw(&d, &self.m).0
        } else {
            d
        }
    }

    fn mul(&self, a: &U, b: &U) -> U {
        let k = self.k;
        let mut t = [0u64; L + 2];
        for i in 0..k {
            let mut c: u128 = 0;
            for j in 0..k {
                let v = u128::from(t[j]) + u128::from(a[j]) * u128::from(b[i]) + c;
                t[j] = v as u64;
                c = v >> 64;
            }
            let v = u128::from(t[k]) + c;
            t[k] = v as u64;
            t[k + 1] = (v >> 64) as u64;
            let q = t[0].wrapping_mul(self.n0);
            let v = u128::from(t[0]) + u128::from(q) * u128::from(self.m[0]);
            let mut c = v >> 64;
            for j in 1..k {
                let v = u128::from(t[j]) + u128::from(q) * u128::from(self.m[j]) + c;
                t[j - 1] = v as u64;
                c = v >> 64;
            }
            let v = u128::from(t[k]) + c;
            t[k - 1] = v as u64;
            t[k] = t[k + 1] + (v >> 64) as u64;
            t[k + 1] = 0;
        }
        let mut r = [0u64; L];
        r[..k].copy_from_slice(&t[..k]);
        if t[k] != 0 || cmp(&r, &self.m) != core::cmp::Ordering::Less {
            r = sub_raw(&r, &self.m).0;
            for x in r.iter_mut().skip(k) {
                *x = 0; // the subtraction may borrow out of the k live limbs
            }
        }
        r
    }

    fn to_m(&self, a: &U) -> U {
        self.mul(a, &self.r2)
    }

    fn from_m(&self, a: &U) -> U {
        let mut one = [0u64; L];
        one[0] = 1;
        self.mul(a, &one)
    }

    /// a^e where `a` is in Montgomery form and `e` a plain integer.
    fn pow(&self, a: &U, e: &U) -> U {
        let mut r = self.one;
        for i in (0..64 * L).rev() {
            r = self.mul(&r, &r);
            if (e[i / 64] >> (i % 64)) & 1 == 1 {
                r = self.mul(&r, a);
            }
        }
        r
    }

    fn inv(&self, a: &U) -> U {
        let mut e = self.m;
        let two = {
            let mut t = [0u64; L];
            t[0] = 2;
            t
        };
        e = sub_raw(&e, &two).0;
        self.pow(a, &e)
    }
}

#[derive(Clone, Copy)]
struct Pt {
    x: U,
    y: U,
    z: U,
}

struct Ctx {
    f: Mont,
    n: Mont,
    b: U,
    g: Pt,
    three: U,
}

impl Ctx {
    fn new(c: Curve) -> Ctx {
        let pr = if c == Curve::P256 { &P256 } else { &P384 };
        let f = Mont::new(from_hex(pr.p), pr.k);
        let n = Mont::new(from_hex(pr.n), pr.k);
        let b = f.to_m(&from_hex(pr.b));
        let g = Pt { x: f.to_m(&from_hex(pr.gx)), y: f.to_m(&from_hex(pr.gy)), z: f.one };
        let mut t = [0u64; L];
        t[0] = 3;
        let three = f.to_m(&t);
        Ctx { f, n, b, g, three }
    }

    fn inf(&self) -> Pt {
        Pt { x: self.f.one, y: self.f.one, z: [0; L] }
    }

    fn on_curve(&self, x: &U, y: &U) -> bool {
        let f = &self.f;
        let lhs = f.mul(y, y);
        let x2 = f.mul(x, x);
        let x3 = f.mul(&x2, x);
        let rhs = f.add(&f.sub(&x3, &f.mul(&self.three, x)), &self.b);
        lhs == rhs
    }

    fn dbl(&self, p: &Pt) -> Pt {
        if is_zero(&p.z) {
            return *p;
        }
        let f = &self.f;
        let delta = f.mul(&p.z, &p.z);
        let gamma = f.mul(&p.y, &p.y);
        let beta = f.mul(&p.x, &gamma);
        let t = f.mul(&f.sub(&p.x, &delta), &f.add(&p.x, &delta));
        let alpha = f.add(&f.add(&t, &t), &t);
        let b4 = f.add(&f.add(&beta, &beta), &f.add(&beta, &beta));
        let b8 = f.add(&b4, &b4);
        let x3 = f.sub(&f.mul(&alpha, &alpha), &b8);
        let yz = f.add(&p.y, &p.z);
        let z3 = f.sub(&f.sub(&f.mul(&yz, &yz), &gamma), &delta);
        let g2 = f.mul(&gamma, &gamma);
        let g8 = {
            let a = f.add(&g2, &g2);
            let b = f.add(&a, &a);
            f.add(&b, &b)
        };
        let y3 = f.sub(&f.mul(&alpha, &f.sub(&b4, &x3)), &g8);
        Pt { x: x3, y: y3, z: z3 }
    }

    fn add(&self, p: &Pt, q: &Pt) -> Pt {
        if is_zero(&p.z) {
            return *q;
        }
        if is_zero(&q.z) {
            return *p;
        }
        let f = &self.f;
        let z1z1 = f.mul(&p.z, &p.z);
        let z2z2 = f.mul(&q.z, &q.z);
        let u1 = f.mul(&p.x, &z2z2);
        let u2 = f.mul(&q.x, &z1z1);
        let s1 = f.mul(&f.mul(&p.y, &q.z), &z2z2);
        let s2 = f.mul(&f.mul(&q.y, &p.z), &z1z1);
        if u1 == u2 {
            return if s1 == s2 { self.dbl(p) } else { self.inf() };
        }
        let h = f.sub(&u2, &u1);
        let h2 = f.add(&h, &h);
        let i = f.mul(&h2, &h2);
        let j = f.mul(&h, &i);
        let rr = f.sub(&s2, &s1);
        let r = f.add(&rr, &rr);
        let v = f.mul(&u1, &i);
        let x3 = f.sub(&f.sub(&f.mul(&r, &r), &j), &f.add(&v, &v));
        let s1j = f.mul(&s1, &j);
        let y3 = f.sub(&f.mul(&r, &f.sub(&v, &x3)), &f.add(&s1j, &s1j));
        let zs = f.add(&p.z, &q.z);
        let z3 = f.mul(&f.sub(&f.sub(&f.mul(&zs, &zs), &z1z1), &z2z2), &h);
        Pt { x: x3, y: y3, z: z3 }
    }
}

fn der_int(b: &[u8]) -> Result<(&[u8], &[u8])> {
    if b.len() < 2 || b[0] != 0x02 {
        return Err(Error::Parse);
    }
    let l = usize::from(b[1]);
    if l == 0 || l > 0x7f || b.len() < 2 + l {
        return Err(Error::Parse);
    }
    Ok((&b[2..2 + l], &b[2 + l..]))
}

/// Verifies an ASN.1-DER ECDSA signature `sig` over the digest `hash` with the uncompressed public point `q` (`04 || X || Y`).
pub fn ecdsa_verify(curve: Curve, q: &[u8], hash: &[u8], sig: &[u8]) -> Result<()> {
    let ctx = Ctx::new(curve);
    let k = ctx.f.k;
    let cl = k * 8;
    if q.len() != 1 + 2 * cl || q[0] != 4 {
        return Err(Error::Unsupported);
    }
    if sig.len() < 8 || sig[0] != 0x30 {
        return Err(Error::Parse);
    }
    let (body, tail) = match sig[1] {
        l if l < 0x80 => (sig.get(2..2 + usize::from(l)).ok_or(Error::Parse)?, &sig[(2 + usize::from(l)).min(sig.len())..]),
        0x81 => {
            let l = usize::from(*sig.get(2).ok_or(Error::Parse)?);
            (sig.get(3..3 + l).ok_or(Error::Parse)?, &sig[(3 + l).min(sig.len())..])
        }
        _ => return Err(Error::Parse),
    };
    if !tail.is_empty() {
        return Err(Error::Parse);
    }
    let (rb, rest) = der_int(body)?;
    let (sb, rest) = der_int(rest)?;
    if !rest.is_empty() {
        return Err(Error::Parse);
    }
    let strip = |v: &[u8]| -> Vec<u8> { v.iter().skip_while(|&&x| x == 0).copied().collect() };
    let (r, s) = (from_be(&strip(rb)).ok_or(Error::BadSignature)?, from_be(&strip(sb)).ok_or(Error::BadSignature)?);
    let nmod = ctx.n.m;
    if is_zero(&r) || is_zero(&s) || cmp(&r, &nmod) != core::cmp::Ordering::Less || cmp(&s, &nmod) != core::cmp::Ordering::Less {
        return Err(Error::BadSignature);
    }
    let qx = from_be(&q[1..1 + cl]).ok_or(Error::Parse)?;
    let qy = from_be(&q[1 + cl..]).ok_or(Error::Parse)?;
    if cmp(&qx, &ctx.f.m) != core::cmp::Ordering::Less || cmp(&qy, &ctx.f.m) != core::cmp::Ordering::Less {
        return Err(Error::Parse);
    }
    let (qxm, qym) = (ctx.f.to_m(&qx), ctx.f.to_m(&qy));
    if !ctx.on_curve(&qxm, &qym) {
        return Err(Error::Parse);
    }
    // e: leftmost bits of the digest (at most the group order's length), reduced mod n.
    let eb = &hash[..hash.len().min(cl)];
    let mut e = from_be(eb).ok_or(Error::Invalid)?;
    if cmp(&e, &nmod) != core::cmp::Ordering::Less {
        e = sub_raw(&e, &nmod).0;
    }
    let n = &ctx.n;
    let w = n.inv(&n.to_m(&s));
    let u1 = n.from_m(&n.mul(&n.to_m(&e), &w));
    let u2 = n.from_m(&n.mul(&n.to_m(&r), &w));
    // R = u1*G + u2*Q (Shamir's trick).
    let qp = Pt { x: qxm, y: qym, z: ctx.f.one };
    let gq = ctx.add(&ctx.g, &qp);
    let mut acc = ctx.inf();
    for i in (0..cl * 8).rev() {
        acc = ctx.dbl(&acc);
        let (b1, b2) = ((u1[i / 64] >> (i % 64)) & 1, (u2[i / 64] >> (i % 64)) & 1);
        match (b1, b2) {
            (1, 0) => acc = ctx.add(&acc, &ctx.g),
            (0, 1) => acc = ctx.add(&acc, &qp),
            (1, 1) => acc = ctx.add(&acc, &gq),
            _ => {}
        }
    }
    if is_zero(&acc.z) {
        return Err(Error::BadSignature);
    }
    let f = &ctx.f;
    let zi = f.inv(&acc.z);
    let x = f.from_m(&f.mul(&acc.x, &f.mul(&zi, &zi)));
    let mut xr = x;
    if cmp(&xr, &nmod) != core::cmp::Ordering::Less {
        xr = sub_raw(&xr, &nmod).0;
    }
    if xr == r {
        Ok(())
    } else {
        Err(Error::BadSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hex;

    const VECTORS: &[(&str, &str, &str, &str, &str)] = &[
        ("p256", "sha256", "042db811d1dfc0a2b23bcac442edfbabfe83dc9ac2777cd0c2f286d49cd1adfa75136e1e39ee9c4b875269db36e643d95e3d4575a97f7c71e5e68da62f57839f9a", "2af9e066b33335862f8f38dfe6fa4fd9a862985115b0ca002aab472202f7491e", "3045022005a1fbd7a0a4cae72e3ce2a1aad5b466d49ce53a57c2f8e0a79b9b2e02ad83130221008fa8ba366ea02bc186b6125fc87b1447f76b0fb293ac4cb1965fae52cfaefd8b"),
        ("p256", "sha384", "042db811d1dfc0a2b23bcac442edfbabfe83dc9ac2777cd0c2f286d49cd1adfa75136e1e39ee9c4b875269db36e643d95e3d4575a97f7c71e5e68da62f57839f9a", "0757aa64ccda42fbfe7eacce7afb6e6193fc9e6e2735c5bf2b72ed68889efb0a8940fa37a4264378bcb726e64884ccb4", "3044022068a14d4dd5a481c543a3f146ffddfccc0156c96f602b74111b32847e9372a7a0022064bbb324f6e780d734d9ae06bae7de8390f5d92501bcaaf6130f7542f65c5a88"),
        ("p384", "sha256", "0483232f69c89f65570798d7412c999524cb13b4a75e38a33d2be5d2d4727806447a9cb491acfb43337a0152e68b91291529baf18610d72d2274f8ed8b358a0fb9f0ac31ed7c436f10def5180884fde973656e8b4f0f3d1efb1dbbc91bc560f5a4", "b73e987f16f97186f31b66977fba4379251886742ec0bcfd5e8209002ec74d29", "30640230555b5f553ab1c697e64b3db02272544020c218d9d70f9748e47c1cea799be1e5b1a5e3d83a3033ce37f2d6373d61e142023026465fba781faf703430824d51065387376369078b6752a8f3aa1e1fd2bdcfa86b831d06a3cbb481404b87c4d4d204e4"),
        ("p384", "sha384", "0483232f69c89f65570798d7412c999524cb13b4a75e38a33d2be5d2d4727806447a9cb491acfb43337a0152e68b91291529baf18610d72d2274f8ed8b358a0fb9f0ac31ed7c436f10def5180884fde973656e8b4f0f3d1efb1dbbc91bc560f5a4", "4ee6a8dfe2a31cdb68c08beeb36104b95530c4e1e73b84c6bbb238657e9b48890dea59599a3ca3019c7fe48effd8ca73", "3065023009f7f11f5f0588a3327f0d34011855f4a65671d0093045d4c67cce7ea6314133253d5624ab66037c4fec32cb7856257b023100c8650f2afb293907ddcd8be8ead1d7361cd4bdf17a333821c40a80ddf099e7d9695c9f7e9f558529facfd848787714ac"),
    ];

    #[test]
    fn openssl_signatures_verify_and_tampering_fails() {
        for (c, _h, q, d, sig) in VECTORS {
            let curve = if *c == "p256" { Curve::P256 } else { Curve::P384 };
            let (q, d, sig) = (hex(q), hex(d), hex(sig));
            ecdsa_verify(curve, &q, &d, &sig).unwrap();
            let mut bad = d.clone();
            bad[3] ^= 1;
            assert!(ecdsa_verify(curve, &q, &bad, &sig).is_err());
            let mut bq = q.clone();
            bq[40] ^= 1;
            assert!(ecdsa_verify(curve, &bq, &d, &sig).is_err(), "point off the curve");
            let wrong = if curve == Curve::P256 { Curve::P384 } else { Curve::P256 };
            assert!(ecdsa_verify(wrong, &q, &d, &sig).is_err());
        }
    }
}

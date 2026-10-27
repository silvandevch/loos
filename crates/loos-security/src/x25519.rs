//! X25519 (RFC 7748) on 5x51-bit-free simple field arithmetic: 16 limbs of 16 bits (TweetNaCl style).

type Fe = [i64; 16];

fn car(o: &mut Fe) {
    for i in 0..16 {
        o[i] += 1 << 16;
        let c = o[i] >> 16;
        if i < 15 {
            o[i + 1] += c - 1;
        } else {
            o[0] += 38 * (c - 1);
        }
        o[i] -= c << 16;
    }
}
fn sel(p: &mut Fe, q: &mut Fe, b: i64) {
    let c = !(b - 1);
    for i in 0..16 {
        let t = c & (p[i] ^ q[i]);
        p[i] ^= t;
        q[i] ^= t;
    }
}
fn pack(n: &Fe) -> [u8; 32] {
    let mut t = *n;
    car(&mut t);
    car(&mut t);
    car(&mut t);
    for _ in 0..2 {
        let mut m: Fe = [0; 16];
        m[0] = t[0] - 0xffed;
        for i in 1..15 {
            m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
            m[i - 1] &= 0xffff;
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        let b = (m[15] >> 16) & 1;
        m[14] &= 0xffff;
        sel(&mut t, &mut m, 1 - b);
    }
    let mut o = [0u8; 32];
    for i in 0..16 {
        o[2 * i] = t[i] as u8;
        o[2 * i + 1] = (t[i] >> 8) as u8;
    }
    o
}
fn unpack(n: &[u8; 32]) -> Fe {
    let mut o: Fe = [0; 16];
    for i in 0..16 {
        o[i] = n[2 * i] as i64 + ((n[2 * i + 1] as i64) << 8);
    }
    o[15] &= 0x7fff;
    o
}
fn add(a: &Fe, b: &Fe) -> Fe {
    let mut o = [0; 16];
    for i in 0..16 {
        o[i] = a[i] + b[i];
    }
    o
}
fn sub(a: &Fe, b: &Fe) -> Fe {
    let mut o = [0; 16];
    for i in 0..16 {
        o[i] = a[i] - b[i];
    }
    o
}
fn mul(a: &Fe, b: &Fe) -> Fe {
    let mut t = [0i64; 31];
    for i in 0..16 {
        for j in 0..16 {
            t[i + j] += a[i] * b[j];
        }
    }
    for i in 0..15 {
        t[i] += 38 * t[i + 16];
    }
    let mut o: Fe = [0; 16];
    o.copy_from_slice(&t[..16]);
    car(&mut o);
    car(&mut o);
    o
}
fn inv(i: &Fe) -> Fe {
    let mut c = *i;
    for a in (0..=253).rev() {
        c = mul(&c, &c);
        if a != 2 && a != 4 {
            c = mul(&c, i);
        }
    }
    c
}

/// Scalar multiplication: `scalar * u` (clamps the scalar).
pub fn x25519(scalar: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let mut z = *scalar;
    z[31] = (z[31] & 127) | 64;
    z[0] &= 248;
    let x = unpack(u);
    let (mut a, mut b, mut c, mut d): (Fe, Fe, Fe, Fe) = ([0; 16], x, [0; 16], [0; 16]);
    a[0] = 1;
    d[0] = 1;
    let c121665: Fe = {
        let mut f = [0; 16];
        f[0] = 0xdb41;
        f[1] = 1;
        f
    };
    for i in (0..=254).rev() {
        let r = ((z[i >> 3] >> (i & 7)) & 1) as i64;
        sel(&mut a, &mut b, r);
        sel(&mut c, &mut d, r);
        let mut e = add(&a, &c);
        a = sub(&a, &c);
        c = add(&b, &d);
        b = sub(&b, &d);
        d = mul(&e, &e);
        let f = mul(&a, &a);
        a = mul(&c, &a);
        c = mul(&b, &e);
        e = add(&a, &c);
        a = sub(&a, &c);
        b = mul(&a, &a);
        c = sub(&d, &f);
        a = mul(&c, &c121665);
        a = add(&a, &d);
        c = mul(&c, &a);
        a = mul(&d, &f);
        d = mul(&b, &x);
        b = mul(&e, &e);
        sel(&mut a, &mut b, r);
        sel(&mut c, &mut d, r);
    }
    let ci = inv(&c);
    pack(&mul(&a, &ci))
}

/// Public key for a private scalar.
pub fn public_key(private: &[u8; 32]) -> [u8; 32] {
    let mut base = [0u8; 32];
    base[0] = 9;
    x25519(private, &base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hex;

    #[test]
    fn rfc7748_dh() {
        let a: [u8; 32] = hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a").try_into().unwrap();
        let b: [u8; 32] = hex("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb").try_into().unwrap();
        let (pa, pb) = (public_key(&a), public_key(&b));
        assert_eq!(pa.to_vec(), hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"));
        assert_eq!(pb.to_vec(), hex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"));
        let s = hex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        assert_eq!(x25519(&a, &pb).to_vec(), s);
        assert_eq!(x25519(&b, &pa).to_vec(), s);
    }
}

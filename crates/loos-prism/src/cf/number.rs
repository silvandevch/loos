//! CFNumber / NSNumber arithmetic. A number travels as (CFNumberType, bits): `bits` is the
//! sign-extended integer, or the bits of a double for the floating-point types.

use super::{rd, string, wr};
use crate::libsys::{Act, Call, Func};

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

/// Size in bytes and floating-point-ness of a CFNumberType.
fn shape(t: u64) -> (u64, bool) {
    match t {
        1 | 7 => (1, false),
        2 | 8 => (2, false),
        3 | 9 => (4, false),
        5 | 12 => (4, true),
        6 | 13 | 16 => (8, true),
        _ => (8, false), // SInt64, long, long long, CFIndex, NSInteger
    }
}

#[derive(Clone, Copy)]
enum Num {
    I(i64),
    F(f64),
}

fn num(t: u64, bits: u64) -> Num {
    if shape(t).1 { Num::F(f64::from_bits(bits)) } else { Num::I(bits as i64) }
}
fn as_f64(n: Num) -> f64 {
    match n {
        Num::I(i) => i as f64,
        Num::F(f) => f,
    }
}
fn as_i64(n: Num) -> i64 {
    match n {
        Num::I(i) => i,
        Num::F(f) => f as i64, // saturating; NaN is 0
    }
}

fn compare(a: Num, b: Num) -> i64 {
    let o = match (a, b) {
        (Num::I(x), Num::I(y)) => x.cmp(&y),
        // An integer beyond 2^53 does not survive the trip through a double: compare exactly.
        (Num::I(x), Num::F(y)) | (Num::F(y), Num::I(x)) => {
            let flip = matches!(a, Num::F(_));
            let o = if y.is_nan() {
                core::cmp::Ordering::Greater
            } else if y >= 9.3e18 {
                core::cmp::Ordering::Less
            } else if y <= -9.3e18 {
                core::cmp::Ordering::Greater
            } else {
                let t = y as i64; // truncated toward zero
                x.cmp(&t).then(if (t as f64) < y { core::cmp::Ordering::Less } else if (t as f64) > y { core::cmp::Ordering::Greater } else { core::cmp::Ordering::Equal })
            };
            if flip { o.reverse() } else { o }
        }
        (Num::F(x), Num::F(y)) => x.partial_cmp(&y).unwrap_or_else(|| x.is_nan().cmp(&y.is_nan()).reverse()),
    };
    o as i64
}

/// Equal numbers hash equally whatever their type.
fn hash(n: Num) -> u64 {
    match n {
        Num::F(f) if f.is_finite() && (f as i64) as f64 == f => (f as i64).unsigned_abs(),
        Num::F(f) => f.to_bits().wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 1,
        Num::I(i) => i.unsigned_abs(),
    }
}

/// The text of `-[NSNumber description]`.
pub(crate) fn describe(t: u64, bits: u64) -> alloc::string::String {
    use alloc::format;
    match num(t, bits) {
        Num::I(i) => format!("{i}"),
        Num::F(f) if f.is_nan() => "nan".into(),
        Num::F(f) if f.is_infinite() => if f > 0.0 { "inf".into() } else { "-inf".into() },
        Num::F(f) if shape(t).0 == 4 => format!("{}", f as f32),
        Num::F(f) => format!("{f}"),
    }
}

pub static FUNCS: &[(&str, Func)] = &[
    // (type, pointer to a C value of that type) → bits
    f!("___prism_cf_num_read", |c| {
        let (t, p) = (c.arg(), c.arg());
        let (size, float) = shape(t);
        let raw = match size {
            1 => c.mem.read_vec(p, 1).map(|v| v[0] as i8 as i64 as u64).unwrap_or(0),
            2 => c.mem.read_vec(p, 2).map(|v| i16::from_le_bytes([v[0], v[1]]) as i64 as u64).unwrap_or(0),
            4 => c.mem.read_u32(p).map(|v| if float { (f32::from_bits(v) as f64).to_bits() } else { v as i32 as i64 as u64 }).unwrap_or(0),
            _ => rd(c, p),
        };
        c.ret(raw)
    }),
    // (type, bits, wanted type, pointer) → 1 if the value is represented exactly
    f!("___prism_cf_num_write", |c| {
        let (t, bits, to, p) = (c.arg(), c.arg(), c.arg(), c.arg());
        let n = num(t, bits);
        let (size, float) = shape(to);
        let exact = if float {
            let v = as_f64(n);
            if size == 4 {
                let _ = c.mem.write_u32(p, (v as f32).to_bits());
                (v as f32) as f64 == v || v.is_nan()
            } else {
                wr(c, p, v.to_bits());
                !matches!(n, Num::I(i) if (v as i64) != i)
            }
        } else {
            let v = as_i64(n);
            let _ = c.mem.write(p, &v.to_le_bytes()[..size as usize]);
            let fits = size == 8 || (v >> (size * 8 - 1) == 0 || v >> (size * 8 - 1) == -1);
            fits && !matches!(n, Num::F(f) if (v as f64) != f)
        };
        c.ret(exact as u64)
    }),
    f!("___prism_cf_num_compare", |c| {
        let (t1, b1, t2, b2) = (c.arg(), c.arg(), c.arg(), c.arg());
        c.ret(compare(num(t1, b1), num(t2, b2)) as u64)
    }),
    f!("___prism_cf_num_hash", |c| { let (t, b) = (c.arg(), c.arg()); c.ret(hash(num(t, b))) }),
    f!("___prism_cf_num_i64", |c| { let (t, b) = (c.arg(), c.arg()); c.ret(as_i64(num(t, b)) as u64) }),
    f!("___prism_cf_num_f64", |c| {
        let (t, b) = (c.arg(), c.arg());
        c.abi.ret_f64(as_f64(num(t, b)));
        Act::Done
    }),
    // (type, bits, string object): the string becomes the number's description
    f!("___prism_cf_num_str", |c| {
        let (t, b, s) = (c.arg(), c.arg(), c.arg());
        let text: alloc::vec::Vec<u16> = describe(t, b).encode_utf16().collect();
        string::store(c, s, &text);
        c.ret(1)
    }),
];

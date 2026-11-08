use loos_winrt::fmt::{float, format, Args};

struct A {
    v: Vec<u64>,
    at: usize,
    strs: Vec<(u64, Vec<u32>)>,
}
impl Args for A {
    fn next(&mut self) -> u64 {
        self.at += 1;
        self.v[self.at - 1]
    }
    fn string(&mut self, p: u64, _w: bool, _m: usize) -> Vec<u32> {
        self.strs.iter().find(|s| s.0 == p).unwrap().1.clone()
    }
}

fn f(fmt: &str, v: &[u64]) -> String {
    let mut a = A { v: v.to_vec(), at: 0, strs: vec![(1, "hello".chars().map(|c| c as u32).collect()), (2, "wide".chars().map(|c| c as u32).collect())] };
    let o = format(&fmt.chars().map(|c| c as u32).collect::<Vec<_>>(), false, &mut a);
    o.iter().map(|c| char::from_u32(*c).unwrap()).collect()
}
fn d(x: f64) -> u64 {
    x.to_bits()
}

#[test]
fn integers_flags_width_precision() {
    assert_eq!(f("%d|%5d|%-5d|%05d|%+d|% d", &[42, 42, 42, 42, 42, 42]), "42|   42|42   |00042|+42| 42");
    assert_eq!(f("%d %i", &[(-7i64) as u64, (-1i32) as u32 as u64]), "-7 -1");
    assert_eq!(f("%u", &[(-1i32) as u32 as u64]), "4294967295");
    assert_eq!(f("%x %X %#x %#o %o", &[255, 255, 255, 8, 8]), "ff FF 0xff 010 10");
    assert_eq!(f("%lld %llu %I64d %zu", &[i64::MIN as u64, u64::MAX, (-5i64) as u64, 9]), "-9223372036854775808 18446744073709551615 -5 9");
    assert_eq!(f("%hd %hhd %hu", &[65537, 257, 65537]), "1 1 1");
    assert_eq!(f("%.3d|%5.3d|%.0d|", &[7, 7, 0]), "007|  007||");
    assert_eq!(f("%*d|%-*d|", &[5, 3, 4, 2]), "    3|2   |");
    assert_eq!(f("%%|%c|%3c", &[65, 66]), "%|A|  B");
    assert_eq!(f("%p", &[0xdeadbeef]), "00000000DEADBEEF".to_lowercase());
}

#[test]
fn strings() {
    assert_eq!(f("%s|%8s|%-8s|%.2s|", &[1, 1, 1, 1]), "hello|   hello|hello   |he|");
    assert_eq!(f("%s", &[0]), "(null)");
    assert_eq!(f("%ls %S", &[2, 2]), "wide wide");
}

#[test]
fn floats() {
    assert_eq!(f("%f", &[d(3.14159)]), "3.141590");
    assert_eq!(f("%.2f %.0f %.1f", &[d(2.675), d(2.5), d(0.05)]), "2.67 3 0.1".replace("2.67", "2.67"));
    assert_eq!(f("%.3f|%8.2f|%-8.2f|%08.2f|%+.1f", &[d(1.0), d(-3.14159), d(3.14159), d(-3.14159), d(2.0)]), "1.000|   -3.14|3.14    |-0003.14|+2.0");
    assert_eq!(f("%f", &[d(0.0)]), "0.000000");
    assert_eq!(f("%f", &[d(123456789.125)]), "123456789.125000");
    assert_eq!(f("%.10f", &[d(0.1)]), "0.1000000000");
    assert_eq!(f("%e", &[d(12345.678)]), "1.234568e+04");
    assert_eq!(f("%.2e %E", &[d(0.000123), d(5.0)]), "1.23e-04 5.000000E+00");
    assert_eq!(f("%g %g %g %g", &[d(100000.0), d(1000000.0), d(0.0001), d(0.00001)]), "100000 1e+06 0.0001 1e-05");
    assert_eq!(f("%g %.3g %g", &[d(3.14159), d(1234.5), d(0.5)]), "3.14159 1.23e+03 0.5");
    assert_eq!(f("%f %f %f", &[d(f64::INFINITY), d(-f64::INFINITY), d(f64::NAN)]), "inf -inf nan");
    assert_eq!(f("%f", &[d(1e20)]), "100000000000000000000.000000");
    assert_eq!(f("%.1f", &[d(9.96)]), "10.0");
    assert_eq!(f("%e", &[d(9.9999999)]), "1.000000e+01");
    assert_eq!(float(255.0, b'g', Some(6), false, false), "255");
}

#[test]
fn unknown_and_truncated_specs_do_not_panic() {
    assert_eq!(f("100%", &[]), "100");
    assert_eq!(f("%q", &[]), "%q");
    assert_eq!(f("%5", &[]), "");
}

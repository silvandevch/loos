use loos_timesvc::calendar::*;
use loos_timesvc::ntp::*;
use loos_timesvc::tz::Zone;

#[test]
fn calendar() {
    assert_eq!(to_unix(2000, 2, 29, 12, 0, 0), Some(951_825_600));
    assert_eq!(to_unix(2001, 2, 29, 0, 0, 0), None);
    let d = from_unix(1_709_210_096); // 2024-02-29 12:34:56 Thu
    assert_eq!((d.year, d.month, d.day, d.hour, d.min, d.sec, d.weekday), (2024, 2, 29, 12, 34, 56, 4));
    let n = from_unix(-1); // 1969-12-31 23:59:59 Wed
    assert_eq!((n.year, n.month, n.day, n.hour, n.weekday), (1969, 12, 31, 23, 3));
}

#[test]
fn ntp_roundtrip_and_math() {
    let t1_ns: i128 = 1_700_000_000_000_000_000;
    let t1 = Ts::from_unix_ns(t1_ns);
    let req = request(t1);
    // server clock is 2.5 s ahead; one-way latency 40 ms each way; 1 ms processing
    let skew = 2_500_000_000i128;
    let rx = Ts::from_unix_ns(t1_ns + 40_000_000 + skew);
    let tx = Ts::from_unix_ns(t1_ns + 41_000_000 + skew);
    let t4 = t1_ns + 81_000_000;
    let reply = parse_reply(&server_reply(&req, rx, tx, 2), t1).unwrap();
    let s = measure(&reply, t1_ns, t4);
    assert!((s.offset_ns - skew).abs() < 10, "{}", s.offset_ns);
    assert!((s.delay_ns - 80_000_000).abs() < 10, "{}", s.delay_ns);
    assert_eq!(discipline(s.offset_ns, 600), Action::Step(s.offset_ns));
    // era mapping: result near the hint, not 1900 or 2036
    assert!((Ts::from_unix_ns(t1_ns).to_unix_ns(1_700_000_000) - t1_ns).abs() < 2);
}

#[test]
fn ntp_rejections() {
    let t1 = Ts::from_unix_ns(1_700_000_000_000_000_000);
    let req = request(t1);
    let ok = server_reply(&req, Ts(1), Ts(2), 3);
    assert!(parse_reply(&ok, t1).is_ok());
    assert_eq!(parse_reply(&ok[..47], t1), Err(Reject::Length));
    assert_eq!(parse_reply(&ok, Ts(t1.0 + 1)), Err(Reject::OriginMismatch));
    let mut b = ok.clone();
    b[1] = 0;
    assert_eq!(parse_reply(&b, t1), Err(Reject::Unsynchronized)); // kiss-o'-death
    let mut b = ok.clone();
    b[0] = (4 << 3) | 3;
    assert_eq!(parse_reply(&b, t1), Err(Reject::Mode));
    let mut b = ok.clone();
    b[0] = (3 << 6) | (4 << 3) | 4;
    assert_eq!(parse_reply(&b, t1), Err(Reject::Unsynchronized));
    assert_eq!(parse_reply(&server_reply(&req, Ts(1), Ts(0), 3), t1), Err(Reject::NoTransmit));
}

#[test]
fn select_and_discipline() {
    let s = |o, d, st| Sample { offset_ns: o, delay_ns: d, stratum: st };
    let best = select(&[s(5, 50, 2), s(9, 20, 3), s(1, -4, 1), s(2, 20, 2), s(7, 999, 1)], 100).unwrap();
    assert_eq!(best, s(2, 20, 2));
    assert_eq!(select(&[s(0, 500, 1)], 100), None);
    assert_eq!(discipline(500_000, 600), Action::None);
    assert_eq!(discipline(10_000_000, 100), Action::Slew(100));
    assert_eq!(discipline(100_000_000, 10), Action::Slew(500));
    assert_eq!((poll_interval(0), poll_interval(2), poll_interval(9)), (64, 256, 1024));
}

#[test]
fn timezones() {
    let cet = Zone::parse("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
    let t = |y, mo, d, h| to_unix(y, mo, d, h, 0, 0).unwrap();
    // 2024: DST starts Sun 31 Mar 01:00 UTC, ends Sun 27 Oct 01:00 UTC
    assert_eq!(cet.offset_at(t(2024, 3, 31, 0) + 3599), (3600, false));
    assert_eq!(cet.offset_at(t(2024, 3, 31, 1)), (7200, true));
    assert_eq!(cet.offset_at(t(2024, 10, 27, 1) - 1), (7200, true));
    assert_eq!(cet.offset_at(t(2024, 10, 27, 1)), (3600, false));
    assert_eq!(cet.abbrev(t(2024, 7, 1, 0)), "CEST");
    let (lt, off, dst) = cet.local(t(2024, 7, 1, 10));
    assert_eq!((lt.hour, off, dst), (12, 7200, true));
    // southern hemisphere: Sydney
    let syd = Zone::parse("AEST-10AEDT,M10.1.0,M4.1.0/3").unwrap();
    assert_eq!(syd.offset_at(t(2024, 1, 15, 0)).1, true);
    assert_eq!(syd.offset_at(t(2024, 7, 15, 0)).1, false);
    // no-DST, quoted names, explicit dst offset, US style
    assert_eq!(Zone::parse("<+0530>-5:30").unwrap().std_off, 19_800);
    let est = Zone::parse("EST5EDT,M3.2.0,M11.1.0").unwrap();
    assert_eq!(est.offset_at(t(2024, 3, 10, 6)), (-18_000, false));
    assert_eq!(est.offset_at(t(2024, 3, 10, 7)), (-14_400, true));
    assert!(Zone::parse("X").is_none() && Zone::parse("CET-1CEST,J1,J2").is_none());
    assert_eq!(Zone::parse("UTC0").unwrap().offset_at(0), (0, false));
}

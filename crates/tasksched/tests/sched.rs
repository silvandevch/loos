use loos_tasksched::*;

const H: u32 = 3600;

#[test]
fn civil_roundtrip() {
    for d in [-1000i64, 0, 59, 60, 11016, 19_000, 20_000] {
        let (y, m, dd) = civil_from_days(d);
        assert_eq!(days_from_civil(y, m, dd), d);
    }
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    assert_eq!(days_from_civil(2000, 3, 1), 11_017);
}

#[test]
fn daily_and_weekly() {
    let t = Trigger::Daily { start_day: 0, every_days: 2, secs_of_day: 3 * H };
    assert_eq!(t.next_after(0), Some(3 * H as i64));
    assert_eq!(t.next_after(3 * H as i64), Some(2 * DAY + 3 * H as i64));
    // day 3 (odd) -> next is day 4
    assert_eq!(t.next_after(3 * DAY), Some(4 * DAY + 3 * H as i64));
    // 1970-01-01 is Thursday (4). Mondays only (bit 1).
    let w = Trigger::Weekly { mask: 1 << 1, secs_of_day: 0 };
    assert_eq!(w.next_after(0), Some(4 * DAY));
    assert_eq!(Trigger::Weekly { mask: 0, secs_of_day: 0 }.next_after(0), None);
}

#[test]
fn monthly_clamps() {
    // 2024-01-31 12:00 -> day 31 clamps to Feb 29
    let t = Trigger::Monthly { day: 31, secs_of_day: 0 };
    let from = days_from_civil(2024, 1, 31) * DAY + 12 * H as i64;
    assert_eq!(t.next_after(from), Some(days_from_civil(2024, 2, 29) * DAY));
    let dec = days_from_civil(2024, 12, 31) * DAY + 1;
    assert_eq!(t.next_after(dec), Some(days_from_civil(2025, 1, 31) * DAY));
}

#[test]
fn interval_and_once() {
    let t = Trigger::Interval { anchor: 100, secs: 60 };
    assert_eq!(t.next_after(0), Some(100));
    assert_eq!(t.next_after(100), Some(160));
    assert_eq!(t.next_after(161), Some(220));
    assert_eq!(Trigger::Once(50).next_after(50), None);
}

#[test]
fn poll_missed_overlap_events() {
    let mut s = Scheduler::new();
    let a = s.add("a", "a.exe", vec![Trigger::Interval { anchor: 0, secs: 100 }], MissedPolicy::Skip, 0).unwrap();
    let b = s.add("b", "b.exe", vec![Trigger::Interval { anchor: 0, secs: 100 }], MissedPolicy::RunOnce, 0).unwrap();
    assert_eq!(s.add("a", "x", vec![], MissedPolicy::Skip, 0), Err(Error::Exists));
    // machine was off from 50..1000: a skips, b runs once
    assert_eq!(s.poll(1000), vec![b]);
    s.finished(b);
    // normal on-time fire
    let nr = s.get(a).unwrap().next_run.unwrap();
    assert_eq!(nr, 1100);
    assert_eq!(s.poll(1100), vec![a, b]);
    // still running: next fire suppressed
    assert!(s.poll(1200).iter().all(|&i| i != a));
    s.finished(a);
    assert!(s.poll(1300).contains(&a));
    // events
    let c = s.add("c", "c.exe", vec![Trigger::Boot, Trigger::Logon], MissedPolicy::Skip, 0).unwrap();
    s.notify_logon();
    assert_eq!(s.poll(1301), vec![c]);
    s.finished(c);
    s.set_enabled(c, false, 0).unwrap();
    s.notify_boot();
    assert!(s.poll(1302).is_empty());
    s.remove(c).unwrap();
    assert_eq!(s.remove(c), Err(Error::NotFound));
}

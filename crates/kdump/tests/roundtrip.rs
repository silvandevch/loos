use loos_kdump::bugcheck::{self, BugCheck};
use loos_kdump::format::*;
use loos_kdump::{report, stack};

fn sample() -> Vec<u8> {
    let h = Header { bugcheck: BugCheck::new(bugcheck::PAGE_FAULT_IN_NONPAGED_AREA, [0xdead, 1, 0xfff0_0010, 0]), time_ns: 42, cpu: 1, ncpus: 4, build_id: 0xabc };
    let mut w = DumpWriter::begin(Vec::new(), &h).unwrap();
    let mut c = Context::default();
    c.rip = 0xfff0_0010;
    c.gpr[GPR_RSP] = 0x7000;
    c.cr[2] = 0xdead;
    w.context(&c).unwrap();
    w.stack(&[0xfff0_0010, 0xfff0_0200]).unwrap();
    w.memory(0x7000, &[1, 2, 3, 4]).unwrap();
    w.module(0xfff0_0000, 0x1000, "loos.sys").unwrap();
    w.thread(7, 2, 0xfff0_0010, 0x7000).unwrap();
    w.log(b"oops\n").unwrap();
    let (v, total) = w.finish().unwrap();
    assert_eq!(v.len() as u32, total);
    v
}

#[test]
fn roundtrip() {
    let r = DumpReader::parse(&sample()).unwrap();
    assert_eq!(r.header.bugcheck.code, bugcheck::PAGE_FAULT_IN_NONPAGED_AREA);
    assert_eq!(r.header.ncpus, 4);
    assert_eq!(r.context.unwrap().cr[2], 0xdead);
    assert_eq!(r.stack.len(), 2);
    assert_eq!(r.memory[0], (0x7000, vec![1, 2, 3, 4]));
    assert_eq!(r.threads[0].tid, 7);
    assert_eq!(r.resolve(0xfff0_0200), Some(("loos.sys", 0x200)));
    let t = report::render(&r);
    assert!(t.contains("PAGE_FAULT_IN_NONPAGED_AREA") && t.contains("loos.sys+0x200") && t.contains("oops"));
}

#[test]
fn corruption_detected() {
    let good = sample();
    for i in [3usize, 20, HEADER_LEN + 14, good.len() - 2] {
        let mut b = good.clone();
        b[i] ^= 0x40;
        assert!(DumpReader::parse(&b).is_err(), "flip at {i}");
    }
    assert_eq!(DumpReader::parse(&good[..good.len() - 1]).unwrap_err(), DumpError::Truncated);
}

#[test]
fn slice_sink_no_alloc_and_full() {
    let h = Header { bugcheck: BugCheck::new(bugcheck::KERNEL_PANIC, [0; 4]), time_ns: 0, cpu: 0, ncpus: 1, build_id: 0 };
    let mut buf = [0u8; 256];
    let mut w = DumpWriter::begin(SliceSink::new(&mut buf), &h).unwrap();
    w.log(b"x").unwrap();
    let (s, n) = w.finish().unwrap();
    assert_eq!(s.written().len() as u32, n);
    assert!(DumpReader::parse(s.written()).is_ok());
    let mut tiny = [0u8; 50];
    assert_eq!(DumpWriter::begin(SliceSink::new(&mut tiny), &h).err(), Some(DumpError::NoSpace));
}

#[test]
fn walk_stops_on_bad_frames() {
    // frames: rbp=0x1000 -> [0x1100, ret A]; 0x1100 -> [0x1100 (loop), ret B]
    let mem = |a: u64| match a {
        0x1000 => Some(0x1100),
        0x1008 => Some(0xA),
        0x1100 => Some(0x1100),
        0x1108 => Some(0xB),
        _ => None,
    };
    let f = stack::walk(0x99, 0x1000, 0x1000, 0x2000, 16, mem);
    assert_eq!(f, vec![0x99, 0xA, 0xB]);
    assert_eq!(stack::walk(0x99, 0x3000, 0x1000, 0x2000, 16, mem), vec![0x99]);
}

#[test]
fn prune_policy() {
    let d = [(1, 100), (2, 100), (3, 100), (4, 100)];
    assert_eq!(report::prune(&d, 10, 1000), Vec::<u64>::new());
    assert_eq!(report::prune(&d, 2, 1000), vec![1, 2]);
    assert_eq!(report::prune(&d, 10, 250), vec![1, 2]);
    assert_eq!(report::prune(&d, 1, 0), vec![1, 2, 3]);
}

#[test]
fn names() {
    assert_eq!(bugcheck::name(0x50), "PAGE_FAULT_IN_NONPAGED_AREA");
    assert_eq!(bugcheck::name(0x9999), "UNKNOWN_BUGCHECK");
}

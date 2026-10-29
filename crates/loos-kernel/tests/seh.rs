#![allow(missing_docs)]
use std::collections::BTreeMap;

use loos_kernel::seh::*;

#[derive(Default)]
struct Mem(BTreeMap<u64, u8>);
impl Mem {
    fn put(&mut self, a: u64, b: &[u8]) {
        for (i, x) in b.iter().enumerate() {
            self.0.insert(a + i as u64, *x);
        }
    }
    fn q(&mut self, a: u64, v: u64) {
        self.put(a, &v.to_le_bytes());
    }
}
impl Memory for Mem {
    fn read(&self, a: u64, buf: &mut [u8]) -> bool {
        buf.iter_mut().enumerate().all(|(i, b)| self.0.get(&(a + i as u64)).map(|v| *b = *v).is_some())
    }
}

const BASE: u64 = 0x1_4000_0000;
const STACK: (u64, u64) = (0x7000, 0x9000);

/// f: `push rbx` (ends at 1), `sub rsp,0x20` (ends at 5); body 5..0x40; handler at 0x900.
/// g (0x100..0x140): leaf-ish frame with `sub rsp,0x28` (ends at 4), no handler.
fn image(with_handler: bool) -> (Image, Mem) {
    let mut m = Mem::default();
    let mut ui = vec![1 | if with_handler { 1 << 3 } else { 0 }, 5, 2, 0];
    ui.extend_from_slice(&[5, 0x32]); // offset 5: ALLOC_SMALL info=3 -> 0x20
    ui.extend_from_slice(&[1, 0x30]); // offset 1: PUSH_NONVOL rbx(3)
    ui.extend_from_slice(&0x900u32.to_le_bytes());
    m.put(BASE + 0x500, &ui);
    m.put(BASE + 0x600, &[1, 4, 1, 0, 4, 0x42]); // ALLOC_SMALL info=4 -> 0x28
    let funcs = [(0u32, 0x40u32, 0x500u32), (0x100, 0x140, 0x600)];
    for (i, f) in funcs.iter().enumerate() {
        let a = BASE + 0x400 + i as u64 * 12;
        m.put(a, &f.0.to_le_bytes());
        m.put(a + 4, &f.1.to_le_bytes());
        m.put(a + 8, &f.2.to_le_bytes());
    }
    let img = Image::load(BASE, 0x1000, &m, 0x400, 24).unwrap();
    (img, m)
}

#[test]
fn lookup_and_unwind_body() {
    let (img, mut m) = image(true);
    assert!(img.lookup(BASE + 0x40).is_none());
    let f = img.lookup(BASE + 0x10).unwrap();
    let mut c = Context { rip: BASE + 0x10, ..Default::default() };
    c.gpr[RSP] = 0x8000;
    m.q(0x8020, 0xbb); // saved rbx
    m.q(0x8028, 0xdead); // return address
    let h = virtual_unwind(&img, f, &mut c, &m).unwrap().unwrap();
    assert_eq!((c.rip, c.rsp(), c.gpr[3]), (0xdead, 0x8030, 0xbb));
    assert_eq!((h.routine, h.establisher), (BASE + 0x900, 0x8000));
}

#[test]
fn unwind_inside_prolog_skips_later_codes_and_handler() {
    let (img, mut m) = image(true);
    let f = img.lookup(BASE + 2).unwrap();
    let mut c = Context { rip: BASE + 2, ..Default::default() };
    c.gpr[RSP] = 0x8020; // after push rbx, before sub
    m.q(0x8020, 0xbb);
    m.q(0x8028, 0xbeef);
    assert_eq!(virtual_unwind(&img, f, &mut c, &m).unwrap(), None);
    assert_eq!((c.rip, c.rsp(), c.gpr[3]), (0xbeef, 0x8030, 0xbb));
}

#[test]
fn dispatch_order_veh_then_frames_then_filter() {
    let (img, mut m) = image(true);
    // g at 0x110 called f? stack: fault in g, g's caller is f body, f's caller returns 0.
    m.q(0x8000 + 0x28, BASE + 0x10); // g return into f
    m.q(0x8030 + 0x20, 0xbb); // f saved rbx
    m.q(0x8030 + 0x28, 0); // f return: end of stack
    let mut c = Context { rip: BASE + 0x110, ..Default::default() };
    c.gpr[RSP] = 0x8000;
    let mut veh = Vectored::new();
    let a = veh.add_exception(false, 0xa);
    veh.add_exception(true, 0xb);
    veh.add_continue(false, 0xc);
    veh.unhandled_filter = Some(0xf);

    let mut log = vec![];
    let out = dispatch(&mut c, &[img.clone()], &m, &veh, STACK, &mut |h, _| {
        log.push(h);
        Disposition::ContinueSearch
    });
    assert_eq!(out, Outcome::Unhandled);
    assert_eq!(log[0], HandlerCall::Vectored(0xb));
    assert_eq!(log[1], HandlerCall::Vectored(0xa));
    assert!(matches!(log[2], HandlerCall::Frame(h) if h.routine == BASE + 0x900 && h.establisher == 0x8030));
    assert_eq!(log[3], HandlerCall::Unhandled(0xf));

    log.clear();
    let out = dispatch(&mut c, &[img], &m, &veh, STACK, &mut |h, ctx| {
        log.push(h);
        if matches!(h, HandlerCall::Frame(_)) {
            ctx.rip = 0x1234;
            Disposition::ContinueExecution
        } else {
            Disposition::ContinueSearch
        }
    });
    assert_eq!(out, Outcome::Continue);
    assert_eq!(c.rip, 0x1234);
    assert_eq!(log.last(), Some(&HandlerCall::Continue(0xc)));
    assert!(veh.remove(a));
    assert!(!veh.remove(a));
}

#[test]
fn bad_stack_detected() {
    let (img, m) = image(false);
    let mut c = Context { rip: BASE + 0x10, ..Default::default() };
    c.gpr[RSP] = 0x100;
    let out = dispatch(&mut c, &[img], &m, &Vectored::new(), STACK, &mut |_, _| Disposition::ContinueSearch);
    assert_eq!(out, Outcome::BadStack);
}

mod common;
use common::*;
use loos_drivers::hw::Mmio;
use loos_drivers::input::{keys, mods, InputEvent};
use loos_drivers::storage::BlockDev;
use loos_drivers::usb::*;
use loos_drivers::xhci::{Kind, Xhci, ARENA_BYTES};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const CAPLEN: usize = 0x20;
const DB: usize = 0x2000;
const RT: usize = 0x3000;

#[derive(Clone, Copy, PartialEq)]
enum Fn_ {
    Kbd,
    Msc,
}

#[derive(Default)]
struct RingS {
    base: u64,
    idx: usize,
    cycle: u32,
}

#[derive(PartialEq, Debug)]
enum MscS {
    Cmd,
    DataIn,
    DataOut(usize),
    Status,
}

struct DevS {
    f: Fn_,
    rings: HashMap<u8, RingS>,
    pending_in: Vec<u64>, // queued interrupt-IN buffers
    msc: MscS,
    tag: u32,
    cb: Vec<u8>,
    in_data: Vec<u8>,
    write_lba: usize,
}

struct S {
    mem: Mem,
    disk: Rc<RefCell<Vec<u8>>>,
    regs: HashMap<usize, u32>,
    run: bool,
    ports: [u32; 2],
    slots: Vec<Option<DevS>>,
    cmd: RingS,
    ev_idx: usize,
    ev_cycle: u32,
    next_slot: u8,
    log: Vec<String>,
}

type Shared = Rc<RefCell<S>>;

impl S {
    fn trb(&self, ring: &RingS) -> (u64, u32, u32) {
        let t = self.mem.rd(ring.base + ring.idx as u64 * 16, 16);
        (u64::from_le_bytes(t[0..8].try_into().unwrap()), u32::from_le_bytes(t[8..12].try_into().unwrap()), u32::from_le_bytes(t[12..16].try_into().unwrap()))
    }

    fn post(&mut self, param: u64, status: u32, control: u32) {
        let erst = u64::from(self.regs[&(RT + 0x30)]) | u64::from(*self.regs.get(&(RT + 0x34)).unwrap_or(&0)) << 32;
        let seg = self.mem.r64(erst);
        let size = self.mem.r32(erst + 8) as usize;
        let at = seg + self.ev_idx as u64 * 16;
        let mut t = [0u8; 16];
        t[0..8].copy_from_slice(&param.to_le_bytes());
        t[8..12].copy_from_slice(&status.to_le_bytes());
        t[12..16].copy_from_slice(&(control | self.ev_cycle).to_le_bytes());
        self.mem.wr(at, &t);
        self.ev_idx += 1;
        if self.ev_idx == size {
            self.ev_idx = 0;
            self.ev_cycle ^= 1;
        }
    }

    fn xfer_event(&mut self, trb: u64, slot: u8, dci: u8, code: u32, residual: u32) {
        self.post(trb, (code << 24) | residual, (32 << 10) | (u32::from(dci) << 16) | (u32::from(slot) << 24));
    }

    fn commands(&mut self) {
        loop {
            let (param, _st, ctl) = self.trb(&self.cmd);
            if ctl & 1 != self.cmd.cycle {
                return;
            }
            let at = self.cmd.base + self.cmd.idx as u64 * 16;
            let ty = (ctl >> 10) & 63;
            if ty == 6 {
                self.cmd.base = param;
                self.cmd.idx = 0;
                if ctl & 2 != 0 {
                    self.cmd.cycle ^= 1;
                }
                continue;
            }
            self.cmd.idx += 1;
            let slot = (ctl >> 24) as u8;
            let mut out_slot = 0u8;
            match ty {
                9 => {
                    self.next_slot += 1;
                    out_slot = self.next_slot;
                    self.slots.resize_with(out_slot as usize + 1, || None);
                }
                11 => {
                    let dcbaa = u64::from(self.regs[&(CAPLEN + 0x30)]) | u64::from(self.regs[&(CAPLEN + 0x34)]) << 32;
                    assert_ne!(self.mem.r64(dcbaa + u64::from(slot) * 8), 0, "output context installed before Address Device");
                    let port = (self.mem.r32(param + 32 + 4) >> 16) as usize;
                    assert_eq!(self.mem.r32(param + 4), 0b11, "A0|A1");
                    let ring = self.mem.r64(param + 64 + 8);
                    assert_eq!(ring & 1, 1, "DCS");
                    let mut rings = HashMap::new();
                    rings.insert(1u8, RingS { base: ring & !0xf, idx: 0, cycle: 1 });
                    let f = if port == 1 { Fn_::Kbd } else { Fn_::Msc };
                    self.slots[slot as usize] = Some(DevS { f, rings, pending_in: vec![], msc: MscS::Cmd, tag: 0, cb: vec![], in_data: vec![], write_lba: 0 });
                    self.log.push(format!("address slot {slot} port {port}"));
                }
                12 => {
                    let add = self.mem.r32(param + 4);
                    for dci in 2..32u8 {
                        if add & (1 << dci) != 0 {
                            let c = param + 32 * (1 + u64::from(dci));
                            let ty = (self.mem.r32(c + 4) >> 3) & 7;
                            let f = self.slots[slot as usize].as_ref().unwrap().f;
                            assert_eq!(ty, match (f, dci) { (Fn_::Kbd, 3) => 7, (Fn_::Msc, 3) => 6, (Fn_::Msc, 4) => 2, _ => 99 }, "endpoint type for dci {dci}");
                            let ring = self.mem.r64(c + 8);
                            self.slots[slot as usize].as_mut().unwrap().rings.insert(dci, RingS { base: ring & !0xf, idx: 0, cycle: 1 });
                        }
                    }
                    assert_eq!((self.mem.r32(param + 32) >> 27) as u8 >= 3, true, "context entries cover the endpoints");
                }
                t => panic!("command type {t}"),
            }
            self.post(at, 1 << 24, (33 << 10) | (u32::from(if ty == 9 { out_slot } else { slot }) << 24));
        }
    }

    fn descriptor(&self, f: Fn_, typ: u8) -> Vec<u8> {
        match (typ, f) {
            (1, f) => {
                let mut d = vec![18, 1, 0, 2, 0, 0, 0, 64, 0x34, 0x12, if f == Fn_::Kbd { 1 } else { 2 }, 0, 0, 1, 0, 0, 0, 1];
                d.truncate(18);
                d
            }
            (2, Fn_::Kbd) => vec![9, 2, 34, 0, 1, 1, 0, 0xa0, 50, 9, 4, 0, 0, 1, 3, 1, 1, 0, 9, 0x21, 0x11, 1, 0, 1, 0x22, 63, 0, 7, 5, 0x81, 3, 8, 0, 10],
            (2, Fn_::Msc) => vec![9, 2, 32, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 2, 8, 6, 0x50, 0, 7, 5, 0x81, 2, 0, 2, 0, 7, 5, 0x02, 2, 0, 2, 0],
            _ => vec![],
        }
    }

    fn transfers(&mut self, slot: u8, dci: u8) {
        loop {
            let (ring, f) = {
                let d = self.slots[slot as usize].as_ref().unwrap();
                (RingS { base: d.rings[&dci].base, idx: d.rings[&dci].idx, cycle: d.rings[&dci].cycle }, d.f)
            };
            let (param, status, ctl) = self.trb(&ring);
            if ctl & 1 != ring.cycle {
                return;
            }
            let at = ring.base + ring.idx as u64 * 16;
            let ty = (ctl >> 10) & 63;
            let advance = |s: &mut S, wrap: bool| {
                let r = s.slots[slot as usize].as_mut().unwrap().rings.get_mut(&dci).unwrap();
                if wrap {
                    r.idx = 0;
                    r.cycle ^= 1;
                } else {
                    r.idx += 1;
                }
            };
            if ty == 6 {
                let r = self.slots[slot as usize].as_mut().unwrap().rings.get_mut(&dci).unwrap();
                r.base = param;
                r.idx = 0;
                if ctl & 2 != 0 {
                    r.cycle ^= 1;
                }
                continue;
            }
            advance(self, false);
            match (dci, ty) {
                (1, 2) => {
                    let setup = param.to_le_bytes();
                    // Data stage (if any) is the next TRB; read it now.
                    let mut data_trb = None;
                    if setup[6] | setup[7] != 0 {
                        let r = self.slots[slot as usize].as_ref().unwrap().rings[&1].idx;
                        let rr = RingS { base: ring.base, idx: r, cycle: ring.cycle };
                        let (p, _, c) = self.trb(&rr);
                        assert_eq!((c >> 10) & 63, 3, "data stage follows setup");
                        data_trb = Some(p);
                        advance(self, false);
                    }
                    let want = u16::from_le_bytes([setup[6], setup[7]]) as usize;
                    match (setup[0], setup[1]) {
                        (0x80, 6) => {
                            let d = self.descriptor(f, setup[3]);
                            self.mem.wr(data_trb.unwrap(), &d[..d.len().min(want)]);
                            self.log.push(format!("get_descriptor {}", setup[3]));
                        }
                        (0x00, 9) => self.log.push("set_config".into()),
                        (0x21, 0x0b) => self.log.push(format!("set_protocol {}", u16::from_le_bytes([setup[2], setup[3]]))),
                        (0x21, 0x0a) => {}
                        s => panic!("setup {s:?}"),
                    }
                    // Status stage.
                    let (_, _, c) = self.trb(&RingS { base: ring.base, idx: self.slots[slot as usize].as_ref().unwrap().rings[&1].idx, cycle: ring.cycle });
                    assert_eq!((c >> 10) & 63, 4, "status stage");
                    advance(self, false);
                    let st = ring.base + (self.slots[slot as usize].as_ref().unwrap().rings[&1].idx as u64 - 1) * 16;
                    self.xfer_event(st, slot, 1, 1, 0);
                }
                (3, 1) if f == Fn_::Kbd => {
                    assert_eq!(status & 0x1ffff, 8);
                    self.slots[slot as usize].as_mut().unwrap().pending_in.push(param);
                    let _ = at;
                }
                (3, 1) | (4, 1) => self.msc_bulk(slot, dci, param, status & 0x1ffff, at),
                t => panic!("unexpected TRB {t:?}"),
            }
        }
    }

    fn msc_bulk(&mut self, slot: u8, dci: u8, buf: u64, len: u32, at: u64) {
        let disk = self.disk.clone();
        let mut d = self.slots[slot as usize].take().unwrap();
        match (dci, &d.msc) {
            (4, MscS::Cmd) => {
                let w = self.mem.rd(buf, 31);
                assert_eq!(w[0..4], 0x4342_5355u32.to_le_bytes());
                d.tag = u32::from_le_bytes(w[4..8].try_into().unwrap());
                let dl = u32::from_le_bytes(w[8..12].try_into().unwrap()) as usize;
                let dir_in = w[12] & 0x80 != 0;
                d.cb = w[15..15 + w[14] as usize].to_vec();
                d.msc = if dl == 0 { MscS::Status } else if dir_in {
                    d.in_data = match d.cb[0] {
                        0x25 => {
                            let last = (disk.borrow().len() / 512 - 1) as u32;
                            [last.to_be_bytes(), 512u32.to_be_bytes()].concat()
                        }
                        0x28 => {
                            let lba = u32::from_be_bytes(d.cb[2..6].try_into().unwrap()) as usize;
                            let n = u16::from_be_bytes(d.cb[7..9].try_into().unwrap()) as usize;
                            assert_eq!(n * 512, dl);
                            disk.borrow()[lba * 512..(lba + n) * 512].to_vec()
                        }
                        c => panic!("scsi {c:#x}"),
                    };
                    MscS::DataIn
                } else {
                    assert_eq!(d.cb[0], 0x2a);
                    d.write_lba = u32::from_be_bytes(d.cb[2..6].try_into().unwrap()) as usize;
                    MscS::DataOut(dl)
                };
            }
            (4, MscS::DataOut(n)) => {
                assert_eq!(len as usize, *n);
                let data = self.mem.rd(buf, *n);
                disk.borrow_mut()[d.write_lba * 512..d.write_lba * 512 + n].copy_from_slice(&data);
                d.msc = MscS::Status;
            }
            (3, MscS::DataIn) => {
                assert_eq!(len as usize, d.in_data.len());
                self.mem.wr(buf, &d.in_data);
                d.msc = MscS::Status;
            }
            (3, MscS::Status) => {
                let mut c = vec![0u8; 13];
                c[0..4].copy_from_slice(&0x5342_5355u32.to_le_bytes());
                c[4..8].copy_from_slice(&d.tag.to_le_bytes());
                self.mem.wr(buf, &c);
                d.msc = MscS::Cmd;
            }
            (dci, st) => panic!("bulk dci {dci} in state {st:?}"),
        }
        self.slots[slot as usize] = Some(d);
        self.xfer_event(at, slot, dci, 1, 0);
    }

    /// Completes the oldest queued interrupt-IN TRB with `report`.
    fn inject(&mut self, report: [u8; 8]) {
        let slot = self.slots.iter().position(|d| d.as_ref().is_some_and(|d| d.f == Fn_::Kbd)).unwrap() as u8;
        let buf = self.slots[slot as usize].as_mut().unwrap().pending_in.remove(0);
        self.mem.wr(buf, &report);
        self.xfer_event(buf, slot, 3, 1, 0);
    }
}

struct Model(Shared);

impl Mmio for Model {
    fn read32(&mut self, off: usize) -> u32 {
        let s = self.0.borrow();
        match off {
            0x00 => 0x0100_0000 | CAPLEN as u32,
            0x04 => 8 | (2 << 24),
            0x08 | 0x10 => 0,
            0x14 => DB as u32,
            0x18 => RT as u32,
            o if o == CAPLEN + 4 => u32::from(!s.run), // USBSTS: HCH
            o if o >= CAPLEN + 0x400 && o < CAPLEN + 0x420 => s.ports[(o - CAPLEN - 0x400) / 0x10],
            o => *s.regs.get(&o).unwrap_or(&0),
        }
    }
    fn write32(&mut self, off: usize, v: u32) {
        let mut s = self.0.borrow_mut();
        if off == CAPLEN {
            s.run = v & 1 != 0;
            if v & 2 != 0 {
                s.regs.clear();
            } else {
                s.regs.insert(off, v & !2);
            }
            return;
        }
        if off >= CAPLEN + 0x400 && off < CAPLEN + 0x420 {
            let p = (off - CAPLEN - 0x400) / 0x10;
            if v & (1 << 4) != 0 {
                s.ports[p] |= (1 << 1) | (1 << 21); // PED, PRC
            }
            if v & (1 << 21) != 0 {
                s.ports[p] &= !(1 << 21);
            }
            return;
        }
        if off == CAPLEN + 0x38 + 4 || off == CAPLEN + 0x18 + 4 {
            s.regs.insert(off, v);
        } else {
            s.regs.insert(off, v);
        }
        if off == CAPLEN + 0x18 {
            let hi = *s.regs.get(&(CAPLEN + 0x1c)).unwrap_or(&0);
            s.cmd = RingS { base: (u64::from(v) | u64::from(hi) << 32) & !0x3f, idx: 0, cycle: v & 1 };
        }
        if off >= DB && off < DB + 4 * 16 {
            let n = (off - DB) / 4;
            if n == 0 {
                s.commands();
            } else {
                s.transfers(n as u8, v as u8);
            }
        }
    }
    fn read16(&mut self, _: usize) -> u16 { 0 }
    fn write16(&mut self, _: usize, _: u16) {}
    fn read8(&mut self, _: usize) -> u8 { 0 }
    fn write8(&mut self, _: usize, _: u8) {}
}

fn build(disk_sectors: usize) -> (Xhci<Model, Mem>, Shared) {
    let mem = Mem::new(0x3000_0000, ARENA_BYTES);
    let disk = Rc::new(RefCell::new(pattern(disk_sectors * 512, 7)));
    let s = Rc::new(RefCell::new(S {
        mem: mem.clone(),
        disk,
        regs: HashMap::new(),
        run: false,
        ports: [1 | (3 << 10), 1 | (3 << 10)],
        slots: vec![],
        cmd: RingS::default(),
        ev_idx: 0,
        ev_cycle: 1,
        next_slot: 0,
        log: vec![],
    }));
    let x = Xhci::new(Model(s.clone()), mem).unwrap();
    (x, s)
}

#[test]
fn descriptor_parsing() {
    let cfg = parse_config(&[9, 2, 32, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 2, 8, 6, 0x50, 0, 7, 5, 0x81, 2, 0, 2, 0, 7, 5, 0x02, 2, 0, 2, 0]).unwrap();
    assert_eq!(cfg.value, 1);
    let i = &cfg.interfaces[0];
    assert_eq!((i.class, i.subclass, i.protocol), (8, 6, 0x50));
    assert_eq!(i.endpoints.len(), 2);
    assert_eq!((i.endpoints[0].dci(), i.endpoints[1].dci()), (3, 4));
    assert_eq!(i.endpoints[0].max_packet, 512);
    assert!(parse_config(&[9, 2, 9, 0, 1, 1, 0, 0, 0, 3, 4]).is_none(), "truncated descriptor");
    assert!(parse_config(&[1, 2]).is_none());
}

#[test]
fn hid_and_cbw_codecs() {
    let mut k = KeyboardDecoder::default();
    let ev = k.decode(&[0, 0, 4, 0, 0, 0, 0, 0]);
    assert!(matches!(ev[0], InputEvent::Key(e) if e.key == 'a' as u32 && e.ch == 'a' as u32 && e.down));
    let ev = k.decode(&[2, 0, 4, 5, 0, 0, 0, 0]); // shift + a held, b pressed
    assert_eq!(ev.len(), 1);
    assert!(matches!(ev[0], InputEvent::Key(e) if e.key == 'b' as u32 && e.ch == 'B' as u32 && e.mods == mods::SHIFT));
    let ev = k.decode(&[0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(ev.len(), 2, "both keys released");
    assert!(ev.iter().all(|e| matches!(e, InputEvent::Key(k) if !k.down)));
    let ev = k.decode(&[0x08, 0, 82, 0, 0, 0, 0, 0]); // gui + up
    assert!(matches!(ev[0], InputEvent::Key(e) if e.key == keys::SUPER));
    assert!(matches!(ev[1], InputEvent::Key(e) if e.key == keys::UP));
    let ev = k.decode(&[1, 0, 6, 0, 0, 0, 0, 0]);
    assert!(matches!(ev.last(), Some(InputEvent::Key(e)) if e.ch == 3), "ctrl+c");
    assert!(matches!(decode_mouse(&[1, 0xfe, 5, 0xff]), Some(InputEvent::Mouse(m)) if m.dx == -2 && m.dy == 5 && m.wheel == -1 && m.buttons == 1));
    assert!(decode_mouse(&[0, 0]).is_none());
    let w = cbw(7, 512, true, &rw10(false, 5, 1));
    assert_eq!((&w[0..4], w[12], w[14], w[15]), (&b"USBC"[..], 0x80, 10, 0x28));
    let mut c = [0u8; 13];
    c[0..4].copy_from_slice(b"USBS");
    c[4] = 7;
    assert_eq!(csw(&c, 7), Some(0));
    assert_eq!(csw(&c, 8), None);
}

#[test]
fn enumerate_keyboard_and_storage() {
    let (mut x, s) = build(2048);
    let found = x.enumerate().unwrap();
    assert_eq!(found.iter().map(|d| d.kind).collect::<Vec<_>>(), [Kind::Keyboard, Kind::MassStorage]);
    assert!(s.borrow().log.iter().any(|l| l == "set_protocol 0"), "boot protocol selected");
    let d = s.borrow().disk.clone();
    let mut disk = x.storage(0).unwrap();
    assert_eq!((disk.sector_size(), disk.sectors()), (512, 2048));
    let mut b = vec![0u8; 512 * 4];
    disk.read(20, &mut b).unwrap();
    assert_eq!(b, d.borrow()[20 * 512..24 * 512]);
    let mut big = vec![0u8; 512 * 200]; // > 32 KiB chunk: split over several READ(10)
    disk.read(300, &mut big).unwrap();
    assert_eq!(big, d.borrow()[300 * 512..500 * 512]);
    let w = pattern(512 * 100, 77);
    disk.write(900, &w).unwrap();
    assert_eq!(w, d.borrow()[900 * 512..1000 * 512]);
    assert!(disk.read(2048, &mut vec![0u8; 512]).is_err());
    assert!(disk.read(0, &mut vec![0u8; 5]).is_err());
}

#[test]
fn keyboard_reports_flow_through_event_ring() {
    let (mut x, s) = build(64);
    x.enumerate().unwrap();
    assert!(x.poll_input().is_empty());
    s.borrow_mut().inject([0, 0, 0x28, 0, 0, 0, 0, 0]); // Enter
    let ev = x.poll_input();
    assert!(matches!(ev[..], [InputEvent::Key(e)] if e.key == '\n' as u32 && e.down));
    s.borrow_mut().inject([0; 8]);
    let ev = x.poll_input();
    assert!(matches!(ev[..], [InputEvent::Key(e)] if !e.down), "endpoint re-armed after each report");
    for i in 0..100u8 {
        // Many more reports than the 64-TRB rings hold: ring + event-ring wraparound.
        s.borrow_mut().inject([0, 0, 4 + i % 20, 0, 0, 0, 0, 0]);
        assert_eq!(x.poll_input().len(), 1, "report {i}");
        s.borrow_mut().inject([0; 8]);
        x.poll_input();
    }
}

#[test]
fn bad_arena_rejected() {
    let mem = Mem::new(0x3000_0100, ARENA_BYTES);
    let s = Rc::new(RefCell::new(S { mem: mem.clone(), disk: Rc::new(RefCell::new(vec![])), regs: HashMap::new(), run: false, ports: [0; 2], slots: vec![], cmd: RingS::default(), ev_idx: 0, ev_cycle: 1, next_slot: 0, log: vec![] }));
    assert!(Xhci::new(Model(s), mem).is_err());
}

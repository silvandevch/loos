//! xHCI 1.1 host controller driver: command/event/transfer rings, port reset, device addressing,
//! control/bulk/interrupt transfers, HID boot devices and bulk-only mass storage. Polled.

use crate::hw::{DmaArena, Mmio};
use crate::input::InputEvent;
use crate::storage::*;
use crate::usb::*;
use crate::DriverError;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

// Capability registers.
const HCSPARAMS1: usize = 0x04;
const HCSPARAMS2: usize = 0x08;
const HCCPARAMS1: usize = 0x10;
const DBOFF: usize = 0x14;
const RTSOFF: usize = 0x18;
// Operational registers (relative to CAPLENGTH).
const USBCMD: usize = 0x00;
const USBSTS: usize = 0x04;
const CRCR: usize = 0x18;
const DCBAAP: usize = 0x30;
const CONFIG: usize = 0x38;
const PORTSC: usize = 0x400;
const CMD_RUN: u32 = 1;
const CMD_RESET: u32 = 2;
const STS_HALTED: u32 = 1;
const STS_CNR: u32 = 1 << 11;
// TRB types.
const T_NORMAL: u32 = 1;
const T_SETUP: u32 = 2;
const T_DATA: u32 = 3;
const T_STATUS: u32 = 4;
const T_LINK: u32 = 6;
const T_ENABLE_SLOT: u32 = 9;
const T_ADDRESS: u32 = 11;
const T_CONFIGURE: u32 = 12;
const E_TRANSFER: u8 = 32;
const E_COMMAND: u8 = 33;

const RING_TRBS: usize = 64;
const RING_BYTES: usize = RING_TRBS * 16;
/// Minimum arena size.
pub const ARENA_BYTES: usize = 0x4_0000;

/// An event TRB.
#[derive(Debug, Clone, Copy)]
pub struct Event {
    /// TRB type.
    pub kind: u8,
    /// Parameter qword.
    pub param: u64,
    /// Status dword.
    pub status: u32,
    /// Control dword.
    pub control: u32,
}

impl Event {
    fn code(&self) -> u8 {
        (self.status >> 24) as u8
    }
    fn slot(&self) -> u8 {
        (self.control >> 24) as u8
    }
    fn dci(&self) -> u8 {
        ((self.control >> 16) & 0x1f) as u8
    }
}

struct Ring {
    base: usize,
    enq: usize,
    cycle: u32,
}

/// Class of an enumerated device interface the driver can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// HID boot keyboard.
    Keyboard,
    /// HID boot mouse.
    Mouse,
    /// Bulk-only mass storage.
    MassStorage,
    /// Enumerated but unsupported.
    Other,
}

struct EpRing {
    dci: u8,
    ring: Ring,
}

struct Dev {
    slot: u8,
    kind: Kind,
    ep0: Ring,
    buf: usize,
    eps: Vec<EpRing>,
    in_dci: u8,
    out_dci: u8,
    kbd: KeyboardDecoder,
    sectors: u64,
    block: u32,
    tag: u32,
}

/// Summary of an enumerated device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Slot id.
    pub slot: u8,
    /// Detected function.
    pub kind: Kind,
}

/// The controller.
pub struct Xhci<M: Mmio, D: DmaArena> {
    m: M,
    dma: D,
    op: usize,
    rt: usize,
    db: usize,
    csz: usize,
    max_ports: usize,
    cmd: Ring,
    ev_off: usize,
    ev_deq: usize,
    ev_cycle: u32,
    bump: usize,
    pending: VecDeque<Event>,
    devs: Vec<Dev>,
    /// Last enumeration step entered (diagnostics: 1 port reset .. 9 mass-storage init).
    pub stage: u8,
}

fn trb_type(c: u32) -> u32 {
    (c >> 10) & 0x3f
}

impl<M: Mmio, D: DmaArena> Xhci<M, D> {
    /// Resets and starts the controller. `dma` must be 4 KiB aligned and at least [`ARENA_BYTES`].
    pub fn new(mut m: M, mut dma: D) -> Result<Self, DriverError> {
        if dma.phys(0) & 0xfff != 0 || dma.len() < ARENA_BYTES {
            return Err(DriverError::Invalid);
        }
        let caplen = (m.read32(0) & 0xff) as usize;
        let hcs1 = m.read32(HCSPARAMS1);
        let hcs2 = m.read32(HCSPARAMS2);
        let hcc = m.read32(HCCPARAMS1);
        let op = caplen;
        let db = m.read32(DBOFF) as usize & !3;
        let rt = m.read32(RTSOFF) as usize & !0x1f;
        let max_slots = hcs1 & 0xff;
        let max_ports = (hcs1 >> 24) as usize;
        let c = m.read32(op + USBCMD);
        m.write32(op + USBCMD, c & !CMD_RUN);
        wait(|| m.read32(op + USBSTS) & STS_HALTED != 0)?;
        m.write32(op + USBCMD, CMD_RESET);
        wait(|| m.read32(op + USBCMD) & CMD_RESET == 0 && m.read32(op + USBSTS) & STS_CNR == 0)?;
        m.write32(op + CONFIG, max_slots);

        // Fixed layout: DCBAA, scratchpad array, command ring, event ring + ERST; everything else bump-allocated.
        const DCBAA: usize = 0x0000;
        const SCRATCH_ARR: usize = 0x0800;
        const CMD_RING: usize = 0x1000;
        const EV_RING: usize = 0x1800;
        const ERST: usize = 0x1c00;
        let nscratch = (((hcs2 >> 27) & 0x1f) | (((hcs2 >> 21) & 0x1f) << 5)) as usize;
        let mut bump = 0x4000;
        if nscratch > 0 {
            if nscratch > 64 {
                return Err(DriverError::Invalid);
            }
            for i in 0..nscratch {
                let page = bump;
                bump += 0x1000;
                dma.w64(SCRATCH_ARR + i * 8, dma.phys(page));
            }
            dma.w64(DCBAA, dma.phys(SCRATCH_ARR));
        }
        w64(&mut m, op + DCBAAP, dma.phys(DCBAA));
        let cmd = Ring { base: CMD_RING, enq: 0, cycle: 1 };
        w64(&mut m, op + CRCR, dma.phys(CMD_RING) | 1);
        // Event ring segment table (one segment).
        dma.w64(ERST, dma.phys(EV_RING));
        dma.w32(ERST + 8, RING_TRBS as u32);
        dma.w32(ERST + 12, 0);
        let ir = rt + 0x20;
        m.write32(ir + 0x08, 1);
        w64(&mut m, ir + 0x18, dma.phys(EV_RING));
        w64(&mut m, ir + 0x10, dma.phys(ERST));
        m.write32(op + USBCMD, CMD_RUN);
        wait(|| m.read32(op + USBSTS) & STS_HALTED == 0)?;
        Ok(Xhci { m, dma, op, rt, db, csz: if hcc & 4 != 0 { 64 } else { 32 }, max_ports, cmd, ev_off: EV_RING, ev_deq: 0, ev_cycle: 1, bump, pending: VecDeque::new(), devs: Vec::new(), stage: 0 })
    }

    fn alloc(&mut self, size: usize, align: usize) -> Result<usize, DriverError> {
        let off = self.bump.div_ceil(align) * align;
        if off + size > self.dma.len() {
            return Err(DriverError::Busy);
        }
        self.bump = off + size;
        self.dma.write(off, &alloc::vec![0u8; size]);
        Ok(off)
    }

    fn new_ring(&mut self) -> Result<Ring, DriverError> {
        Ok(Ring { base: self.alloc(RING_BYTES, 64)?, enq: 0, cycle: 1 })
    }

    fn doorbell(&mut self, target: usize, value: u32) {
        self.dma.barrier();
        self.m.write32(self.db + target * 4, value);
    }

    /// Queues a TRB, handling the link TRB at the ring end. Returns the arena offset of the TRB.
    fn push(dma: &mut D, r: &mut Ring, param: u64, status: u32, control: u32) -> usize {
        if r.enq == RING_TRBS - 1 {
            let mut l = [0u8; 16];
            l[0..8].copy_from_slice(&dma.phys(r.base).to_le_bytes());
            l[12..16].copy_from_slice(&((T_LINK << 10) | 2 | r.cycle).to_le_bytes());
            dma.write(r.base + r.enq * 16, &l);
            r.enq = 0;
            r.cycle ^= 1;
        }
        let off = r.base + r.enq * 16;
        let mut t = [0u8; 16];
        t[0..8].copy_from_slice(&param.to_le_bytes());
        t[8..12].copy_from_slice(&status.to_le_bytes());
        t[12..16].copy_from_slice(&(control | r.cycle).to_le_bytes());
        dma.write(off, &t);
        r.enq += 1;
        off
    }

    fn next_event(&mut self) -> Option<Event> {
        let off = self.ev_off + self.ev_deq * 16;
        let mut t = [0u8; 16];
        self.dma.read(off, &mut t);
        let control = u32::from_le_bytes(t[12..16].try_into().unwrap());
        if control & 1 != self.ev_cycle {
            return None;
        }
        let ev = Event { kind: trb_type(control) as u8, param: u64::from_le_bytes(t[0..8].try_into().unwrap()), status: u32::from_le_bytes(t[8..12].try_into().unwrap()), control };
        self.ev_deq += 1;
        if self.ev_deq == RING_TRBS {
            self.ev_deq = 0;
            self.ev_cycle ^= 1;
        }
        let erdp = self.dma.phys(self.ev_off + self.ev_deq * 16) | 8;
        w64(&mut self.m, self.rt + 0x20 + 0x18, erdp);
        Some(ev)
    }

    /// Waits for the first event matching `f`; others are kept for later.
    fn wait_event(&mut self, f: impl Fn(&Event) -> bool) -> Result<Event, DriverError> {
        if let Some(i) = self.pending.iter().position(|e| f(e)) {
            return Ok(self.pending.remove(i).unwrap());
        }
        for _ in 0..SPIN {
            if let Some(e) = self.next_event() {
                if f(&e) {
                    return Ok(e);
                }
                if self.pending.len() < 256 {
                    self.pending.push_back(e);
                }
            }
        }
        Err(DriverError::Timeout)
    }

    fn command(&mut self, param: u64, status: u32, control: u32) -> Result<Event, DriverError> {
        let off = Self::push(&mut self.dma, &mut self.cmd, param, status, control);
        let want = self.dma.phys(off);
        self.doorbell(0, 0);
        let e = self.wait_event(|e| e.kind == E_COMMAND && e.param == want)?;
        if e.code() != 1 {
            return Err(DriverError::Protocol);
        }
        Ok(e)
    }

    // ------------------------------------------------------------------ enumeration
    /// Resets every connected port and enumerates the devices found. Returns what was bound.
    pub fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, DriverError> {
        let mut found = Vec::new();
        for p in 0..self.max_ports {
            let pr = self.op + PORTSC + 0x10 * p;
            let sc = self.m.read32(pr);
            if sc & 1 == 0 {
                continue;
            }
            self.stage = 1;
            self.m.write32(pr, (sc & (1 << 9)) | (1 << 4));
            let m = &mut self.m;
            wait(|| m.read32(pr) & (1 << 21) != 0)?;
            let sc = self.m.read32(pr);
            self.m.write32(pr, (sc & (1 << 9)) | (1 << 21));
            if sc & 2 == 0 {
                continue;
            }
            let speed = (sc >> 10) & 0xf;
            match self.setup_device(p + 1, speed) {
                Ok(info) => found.push(info),
                Err(DriverError::Protocol) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(found)
    }

    fn ctx(&self, base: usize, idx: usize) -> usize {
        base + idx * self.csz
    }

    fn setup_device(&mut self, port: usize, speed: u32) -> Result<DeviceInfo, DriverError> {
        self.stage = 2;
        let slot = (self.command(0, 0, T_ENABLE_SLOT << 10)?.control >> 24) as u8;
        let out_ctx = self.alloc(self.csz * 32, 64)?;
        let in_ctx = self.alloc(self.csz * 33, 64)?;
        let ep0 = self.new_ring()?;
        let buf = self.alloc(0x1_0000, 64)?;
        self.dma.w64(usize::from(slot) * 8, self.dma.phys(out_ctx));
        let mps: u32 = match speed {
            2 => 8,
            3 => 64,
            4 => 512,
            _ => 64,
        };
        self.dma.w32(in_ctx + 4, 0b11);
        self.dma.w32(self.ctx(in_ctx, 1), (speed << 20) | (1 << 27));
        self.dma.w32(self.ctx(in_ctx, 1) + 4, (port as u32) << 16);
        let e0 = self.ctx(in_ctx, 2);
        self.dma.w32(e0 + 4, (3 << 1) | (4 << 3) | (mps << 16));
        self.dma.w64(e0 + 8, self.dma.phys(ep0.base) | 1);
        self.dma.w32(e0 + 16, 8);
        self.stage = 3;
        self.command(self.dma.phys(in_ctx), 0, (T_ADDRESS << 10) | (u32::from(slot) << 24))?;
        self.devs.push(Dev { slot, kind: Kind::Other, ep0, buf, eps: Vec::new(), in_dci: 0, out_dci: 0, kbd: KeyboardDecoder::default(), sectors: 0, block: 512, tag: 1 });
        let di = self.devs.len() - 1;

        self.stage = 4;
        let dd = self.control_in(di, setup(0x80, REQ_GET_DESCRIPTOR, 0x0100, 0, 18), 18)?;
        if dd.len() < 18 || dd[1] != 1 {
            return Err(DriverError::Protocol);
        }
        self.stage = 5;
        let head = self.control_in(di, setup(0x80, REQ_GET_DESCRIPTOR, 0x0200, 0, 9), 9)?;
        let total = u16::from_le_bytes([head[2], head[3]]);
        self.stage = 6;
        let full = self.control_in(di, setup(0x80, REQ_GET_DESCRIPTOR, 0x0200, 0, total), usize::from(total))?;
        let cfg = parse_config(&full).ok_or(DriverError::Protocol)?;
        self.stage = 7;
        self.control_out(di, setup(0x00, REQ_SET_CONFIGURATION, u16::from(cfg.value), 0, 0))?;

        let mut kind = Kind::Other;
        let mut want: Vec<Endpoint> = Vec::new();
        for itf in &cfg.interfaces {
            let kep = itf.endpoints.iter().find(|e| e.kind == EpKind::Interrupt && e.dir_in);
            match (itf.class, itf.subclass, itf.protocol) {
                (3, 1, p @ (1 | 2)) if kind == Kind::Other => {
                    if let Some(&e) = kep {
                        self.control_out(di, setup(0x21, 0x0b, 0, u16::from(itf.number), 0))?; // SET_PROTOCOL(boot)
                        let _ = self.control_out(di, setup(0x21, 0x0a, 0, u16::from(itf.number), 0)); // SET_IDLE (optional)
                        kind = if p == 1 { Kind::Keyboard } else { Kind::Mouse };
                        want.push(e);
                    }
                }
                (8, 6, 0x50) if kind == Kind::Other => {
                    let bi = itf.endpoints.iter().find(|e| e.kind == EpKind::Bulk && e.dir_in);
                    let bo = itf.endpoints.iter().find(|e| e.kind == EpKind::Bulk && !e.dir_in);
                    if let (Some(&i), Some(&o)) = (bi, bo) {
                        kind = Kind::MassStorage;
                        self.devs[di].in_dci = i.dci();
                        self.devs[di].out_dci = o.dci();
                        want.push(i);
                        want.push(o);
                    }
                }
                _ => {}
            }
        }
        self.stage = 8;
        if !want.is_empty() {
            self.configure_endpoints(di, port, speed, &want, in_ctx)?;
        }
        self.devs[di].kind = kind;
        match kind {
            Kind::Keyboard | Kind::Mouse => self.arm_hid(di),
            Kind::MassStorage => {
                self.stage = 9;
                self.msc_init(di)?
            }
            Kind::Other => {}
        }
        Ok(DeviceInfo { slot, kind })
    }

    fn configure_endpoints(&mut self, di: usize, port: usize, speed: u32, eps: &[Endpoint], in_ctx: usize) -> Result<(), DriverError> {
        let slot = self.devs[di].slot;
        let max_dci = eps.iter().map(Endpoint::dci).max().unwrap_or(1);
        self.dma.w32(in_ctx, 0);
        let mut add = 1u32;
        for e in eps {
            add |= 1 << e.dci();
            let ring = self.new_ring()?;
            let c = self.ctx(in_ctx, 1 + usize::from(e.dci()));
            let ty: u32 = match (e.kind, e.dir_in) {
                (EpKind::Bulk, false) => 2,
                (EpKind::Bulk, true) => 6,
                (EpKind::Interrupt, true) => 7,
                (EpKind::Interrupt, false) => 3,
                _ => 0,
            };
            // Interval field is an exponent of 125 us; HS/SS bInterval is already exponent+1, FS/LS is in ms.
            let interval = if speed >= 3 { u32::from(e.interval.max(1)) - 1 } else { 3 };
            self.dma.w32(c, interval << 16);
            self.dma.w32(c + 4, (3 << 1) | (ty << 3) | (u32::from(e.max_packet) << 16));
            self.dma.w64(c + 8, self.dma.phys(ring.base) | 1);
            self.dma.w32(c + 16, u32::from(e.max_packet));
            self.devs[di].eps.push(EpRing { dci: e.dci(), ring });
        }
        self.dma.w32(in_ctx + 4, add);
        let s = self.ctx(in_ctx, 1);
        self.dma.w32(s, (speed << 20) | (u32::from(max_dci) << 27));
        self.dma.w32(s + 4, (port as u32) << 16);
        self.command(self.dma.phys(in_ctx), 0, (T_CONFIGURE << 10) | (u32::from(slot) << 24))?;
        Ok(())
    }

    // ------------------------------------------------------------------ transfers
    fn control(&mut self, di: usize, s: [u8; 8], len: usize, dir_in: bool) -> Result<Vec<u8>, DriverError> {
        if len > 0x1_0000 {
            return Err(DriverError::Invalid);
        }
        let (slot, buf) = (self.devs[di].slot, self.devs[di].buf);
        let bufp = self.dma.phys(buf);
        let trt = if len == 0 { 0 } else if dir_in { 3 } else { 2 };
        let dev = &mut self.devs[di];
        Self::push(&mut self.dma, &mut dev.ep0, u64::from_le_bytes(s), 8, (T_SETUP << 10) | (1 << 6) | (trt << 16));
        if len > 0 {
            Self::push(&mut self.dma, &mut dev.ep0, bufp, len as u32, (T_DATA << 10) | if dir_in { 1 << 16 } else { 0 });
        }
        let status_in = len == 0 || !dir_in;
        Self::push(&mut self.dma, &mut dev.ep0, 0, 0, (T_STATUS << 10) | (1 << 5) | if status_in { 1 << 16 } else { 0 });
        self.doorbell(usize::from(slot), 1);
        let e = self.wait_event(|e| e.kind == E_TRANSFER && e.slot() == slot && e.dci() == 1)?;
        if !matches!(e.code(), 1 | 13) {
            return Err(DriverError::Protocol);
        }
        let mut out = alloc::vec![0u8; if dir_in { len } else { 0 }];
        self.dma.read(buf, &mut out);
        Ok(out)
    }

    fn control_in(&mut self, di: usize, s: [u8; 8], len: usize) -> Result<Vec<u8>, DriverError> {
        self.control(di, s, len, true)
    }

    fn control_out(&mut self, di: usize, s: [u8; 8]) -> Result<(), DriverError> {
        self.control(di, s, 0, false).map(|_| ())
    }

    /// Queues one Normal TRB on endpoint `dci`; returns when the doorbell is rung.
    fn queue_normal(&mut self, di: usize, dci: u8, buf: usize, len: usize) -> Result<(), DriverError> {
        let slot = self.devs[di].slot;
        let p = self.dma.phys(buf);
        let dev = &mut self.devs[di];
        let ring = &mut dev.eps.iter_mut().find(|e| e.dci == dci).ok_or(DriverError::Invalid)?.ring;
        Self::push(&mut self.dma, ring, p, len as u32, (T_NORMAL << 10) | (1 << 5) | (1 << 2));
        self.doorbell(usize::from(slot), u32::from(dci));
        Ok(())
    }

    fn bulk(&mut self, di: usize, dci: u8, buf: usize, len: usize) -> Result<usize, DriverError> {
        let slot = self.devs[di].slot;
        self.queue_normal(di, dci, buf, len)?;
        let e = self.wait_event(|e| e.kind == E_TRANSFER && e.slot() == slot && e.dci() == dci)?;
        if !matches!(e.code(), 1 | 13) {
            return Err(DriverError::Protocol);
        }
        Ok(len - (e.status & 0xff_ffff) as usize)
    }

    // ------------------------------------------------------------------ HID
    fn hid_buf(&self, di: usize) -> usize {
        self.devs[di].buf + 0x800
    }

    fn arm_hid(&mut self, di: usize) {
        let dci = self.devs[di].eps.first().map_or(0, |e| e.dci);
        let b = self.hid_buf(di);
        let _ = self.queue_normal(di, dci, b, 8);
    }

    /// Collects input events from every HID device and re-arms their endpoints.
    pub fn poll_input(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        while let Some(e) = self.next_event() {
            if self.pending.len() < 256 {
                self.pending.push_back(e);
            }
        }
        let mut keep = VecDeque::new();
        while let Some(e) = self.pending.pop_front() {
            let hid = if e.kind == E_TRANSFER { self.devs.iter().position(|d| d.slot == e.slot() && matches!(d.kind, Kind::Keyboard | Kind::Mouse)) } else { None };
            let Some(di) = hid else {
                keep.push_back(e);
                continue;
            };
            if matches!(e.code(), 1 | 13) {
                let mut r = [0u8; 8];
                self.dma.read(self.hid_buf(di), &mut r);
                match self.devs[di].kind {
                    Kind::Keyboard => out.extend(self.devs[di].kbd.decode(&r)),
                    _ => out.extend(decode_mouse(&r)),
                }
            }
            self.arm_hid(di);
        }
        self.pending = keep;
        out
    }

    // ------------------------------------------------------------------ mass storage
    fn msc_cmd(&mut self, di: usize, cb: &[u8], dir_in: bool, data_len: usize, data_out: Option<&[u8]>) -> Result<Vec<u8>, DriverError> {
        let (indci, outdci, buf) = (self.devs[di].in_dci, self.devs[di].out_dci, self.devs[di].buf);
        let tag = self.devs[di].tag;
        self.devs[di].tag = tag.wrapping_add(1);
        let (cbw_off, data_off, csw_off) = (buf + 0x1000, buf + 0x2000, buf + 0x1100);
        self.dma.write(cbw_off, &cbw(tag, data_len as u32, dir_in, cb));
        self.bulk(di, outdci, cbw_off, 31)?;
        let mut got = Vec::new();
        if data_len > 0 {
            if dir_in {
                self.bulk(di, indci, data_off, data_len)?;
                got = alloc::vec![0u8; data_len];
                self.dma.read(data_off, &mut got);
            } else if let Some(d) = data_out {
                self.dma.write(data_off, d);
                self.bulk(di, outdci, data_off, data_len)?;
            }
        }
        self.bulk(di, indci, csw_off, 13)?;
        let mut c = [0u8; 13];
        self.dma.read(csw_off, &mut c);
        match csw(&c, tag) {
            Some(0) => Ok(got),
            _ => Err(DriverError::Protocol),
        }
    }

    fn msc_init(&mut self, di: usize) -> Result<(), DriverError> {
        let cap = self.msc_cmd(di, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], true, 8, None)?;
        let last = u32::from_be_bytes(cap[0..4].try_into().unwrap());
        let bs = u32::from_be_bytes(cap[4..8].try_into().unwrap());
        if bs == 0 || bs > 4096 {
            return Err(DriverError::Protocol);
        }
        self.devs[di].sectors = u64::from(last) + 1;
        self.devs[di].block = bs;
        Ok(())
    }

    /// Borrows the `n`th mass-storage device as a [`BlockDev`].
    pub fn storage(&mut self, n: usize) -> Option<UsbDisk<'_, M, D>> {
        let di = self.devs.iter().enumerate().filter(|(_, d)| d.kind == Kind::MassStorage).nth(n)?.0;
        Some(UsbDisk { ctl: self, di })
    }
}

/// A USB mass-storage LUN 0.
pub struct UsbDisk<'a, M: Mmio, D: DmaArena> {
    ctl: &'a mut Xhci<M, D>,
    di: usize,
}

impl<M: Mmio, D: DmaArena> BlockDev for UsbDisk<'_, M, D> {
    fn sector_size(&self) -> u32 {
        self.ctl.devs[self.di].block
    }
    fn sectors(&self) -> u64 {
        self.ctl.devs[self.di].sectors
    }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), DriverError> {
        let bs = self.sector_size();
        let total = check_chunk(buf.len(), bs)?;
        if lba + total as u64 > self.sectors() {
            return Err(DriverError::Invalid);
        }
        let per = 0x8000 / bs as usize;
        let mut done = 0;
        while done < total {
            let n = (total - done).min(per);
            let cb = rw10(false, (lba + done as u64) as u32, n as u16);
            let d = self.ctl.msc_cmd(self.di, &cb, true, n * bs as usize, None)?;
            buf[done * bs as usize..(done + n) * bs as usize].copy_from_slice(&d);
            done += n;
        }
        Ok(())
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), DriverError> {
        let bs = self.sector_size();
        let total = check_chunk(data.len(), bs)?;
        if lba + total as u64 > self.sectors() {
            return Err(DriverError::Invalid);
        }
        let per = 0x8000 / bs as usize;
        let mut done = 0;
        while done < total {
            let n = (total - done).min(per);
            let cb = rw10(true, (lba + done as u64) as u32, n as u16);
            self.ctl.msc_cmd(self.di, &cb, false, n * bs as usize, Some(&data[done * bs as usize..(done + n) * bs as usize]))?;
            done += n;
        }
        Ok(())
    }
}

//! Boot-time bring-up of the Rust subsystems from the C kernel: session manager, compositor + D3D11
//! bridge, TCP/IP stack, and the xHCI/NVMe/AHCI drivers over real MMIO and DMA memory.
//! Each `*_selftest` returns a bitmask (one bit per check, all ones = pass) which `kmain` prints.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};
use loos_compositor::d3d11::{Context, Format, SwapChain, SwapEffect};
use loos_compositor::Compositor;
use loos_drivers::hw::{DmaArena, Mmio};
use loos_drivers::storage::BlockDev;
use loos_drivers::xhci::{Kind, Xhci};
use loos_kernel::smss::{Launcher, Outcome, SmConfig, Smss};
use loos_kernel::ProcessId;
use loos_net::stack::Stack;
use loos_net::tcp::State;

struct KLauncher {
    next: ProcessId,
}

impl Launcher for KLauncher {
    fn launch(&mut self, _image: &str, _cmdline: &str, _session: u32, _env: &[(String, String)]) -> Result<ProcessId, String> {
        // The PE loader lands with the Win32 subsystem; until then pids are allocated, not run.
        self.next += 4;
        Ok(self.next)
    }
    fn run_to_completion(&mut self, _image: &str, _cmdline: &str) -> Result<u32, String> {
        Ok(0)
    }
    fn terminate(&mut self, _pid: ProcessId) {}
}

/// Boots smss against a recording launcher and exercises the session policy. Bits 0..4 = checks.
#[no_mangle]
pub extern "C" fn loos_smss_selftest() -> u32 {
    let mut l = KLauncher { next: 0x100 };
    let mut s = Smss::new(SmConfig::default());
    let mut r = 0u32;
    if s.boot(&mut l).is_ok() && s.booted() {
        r |= 1;
    }
    let sid = s.create_session(&mut l);
    if sid == Ok(1) {
        r |= 2;
    }
    if let Some(wl) = s.session(1).and_then(|x| x.logon) {
        if s.on_exit(wl, 0, &mut l) == Outcome::SessionEnded(1) {
            r |= 4;
        }
    }
    if let Some(ls) = s.session(0).and_then(|x| x.procs.last().map(|p| p.0)) {
        if matches!(s.on_exit(ls, 1, &mut l), Outcome::Bugcheck { .. }) {
            r |= 8;
        }
    }
    r
}

/// Compositor + swap-chain round trip into a small target. Bits 0..3.
#[no_mangle]
pub extern "C" fn loos_comp_selftest() -> u32 {
    let (w, h) = (96, 64);
    let mut c = Compositor::new(w, h);
    c.background = 0x102030;
    let mut out = alloc::vec![0u32; (w * h) as usize];
    let mut r = 0u32;
    if c.compose(&mut out, w as usize).len() == 1 && out[0] == 0x102030 {
        r |= 1;
    }
    let ctx = Context;
    if let Ok(mut sc) = SwapChain::new(&mut c, 8, 8, 16, 16, Format::B8G8R8X8, 2, SwapEffect::FlipSequential) {
        ctx.clear_render_target(sc.back_buffer(), [1.0, 0.0, 0.0, 1.0]);
        if sc.present(&mut c, 1) == loos_compositor::d3d11::hr::S_OK {
            r |= 2;
        }
        let dmg = c.compose(&mut out, w as usize);
        if dmg.len() == 1 && out[10 * w as usize + 10] == 0xff0000 && out[0] == 0x102030 {
            r |= 4;
        }
        if c.surface_at(10, 10) == Some(sc.surface()) {
            r |= 8;
        }
    }
    r
}

/// Loopback networking: ICMP echo, UDP, TCP echo. Bits 0..3.
#[no_mangle]
pub extern "C" fn loos_net_selftest() -> u32 {
    let mut s = Stack::new();
    let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut r = 0u32;
    if s.ping(lo, 1, 1, b"loos").is_ok() && s.ping_reply() == Some((lo, 1, 1)) {
        r |= 1;
    }
    if let (Ok(a), Ok(b)) = (s.udp_bind(7000), s.udp_bind(0)) {
        if s.udp_send(b, lo, 7000, b"dgram").is_ok() && s.udp_recv(a).is_some_and(|d| d.2 == b"dgram") {
            r |= 2;
        }
    }
    if let Ok(l) = s.tcp_listen(7) {
        if let Ok(c) = s.tcp_connect(lo, 7) {
            if let Some(a) = s.tcp_accept(l) {
                if s.tcp_state(c) == Some(State::Established) {
                    r |= 4;
                }
                let _ = s.tcp_send(c, b"hello tcp");
                let mut buf = [0u8; 16];
                if s.tcp_recv(a, &mut buf).is_ok_and(|n| &buf[..n] == b"hello tcp") {
                    r |= 8;
                }
            }
        }
    }
    r
}

// ---------------------------------------------------------------------------------- real hardware

/// MMIO window over a mapped BAR.
struct RawMmio(*mut u8);

impl Mmio for RawMmio {
    fn read32(&mut self, off: usize) -> u32 {
        // SAFETY: `off` stays inside the BAR the caller mapped; device registers need volatile access.
        unsafe { core::ptr::read_volatile(self.0.add(off).cast::<u32>()) }
    }
    fn write32(&mut self, off: usize, v: u32) {
        // SAFETY: as above.
        unsafe { core::ptr::write_volatile(self.0.add(off).cast::<u32>(), v) }
    }
    fn read16(&mut self, off: usize) -> u16 {
        // SAFETY: as above.
        unsafe { core::ptr::read_volatile(self.0.add(off).cast::<u16>()) }
    }
    fn write16(&mut self, off: usize, v: u16) {
        // SAFETY: as above.
        unsafe { core::ptr::write_volatile(self.0.add(off).cast::<u16>(), v) }
    }
    fn read8(&mut self, off: usize) -> u8 {
        // SAFETY: as above.
        unsafe { core::ptr::read_volatile(self.0.add(off)) }
    }
    fn write8(&mut self, off: usize, v: u8) {
        // SAFETY: as above.
        unsafe { core::ptr::write_volatile(self.0.add(off), v) }
    }
}

/// Physically contiguous, identity-visible DMA memory owned by the driver.
struct RawDma {
    virt: *mut u8,
    phys: u64,
    len: usize,
}

impl DmaArena for RawDma {
    fn len(&self) -> usize {
        self.len
    }
    fn phys(&self, off: usize) -> u64 {
        self.phys + off as u64
    }
    fn read(&self, off: usize, buf: &mut [u8]) {
        assert!(off + buf.len() <= self.len);
        // SAFETY: bounds checked; the arena is live kernel memory the device also accesses.
        unsafe { core::ptr::copy_nonoverlapping(self.virt.add(off), buf.as_mut_ptr(), buf.len()) }
    }
    fn write(&mut self, off: usize, data: &[u8]) {
        assert!(off + data.len() <= self.len);
        // SAFETY: as above.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), self.virt.add(off), data.len()) }
    }
    fn barrier(&self) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
}

/// Brings up an xHCI controller and enumerates its ports. `out[0..4]` = keyboards, mice, mass-storage
/// devices, sectors of the first mass-storage device. Returns the device count or -1 on failure.
///
/// # Safety
/// `mmio` must be the mapped xHCI BAR; `dma_virt`/`dma_phys` a 4 KiB-aligned zeroed arena of `dma_len` bytes
/// (at least `loos_drivers::xhci::ARENA_BYTES`); `out` four writable `u32`s.
#[no_mangle]
pub unsafe extern "C" fn loos_xhci_probe(mmio: u64, dma_virt: *mut u8, dma_phys: u64, dma_len: u64, out: *mut u32) -> i32 {
    let dma = RawDma { virt: dma_virt, phys: dma_phys, len: dma_len as usize };
    let mut x = match Xhci::new(RawMmio(mmio as *mut u8), dma) {
        Ok(x) => x,
        Err(e) => return -10 - e as i32,
    };
    let devs = match x.enumerate() {
        Ok(d) => d,
        Err(e) => return -(20 + e as i32 + 10 * i32::from(x.stage)),
    };
    let mut c = [0u32; 4];
    for d in &devs {
        match d.kind {
            Kind::Keyboard => c[0] += 1,
            Kind::Mouse => c[1] += 1,
            Kind::MassStorage => c[2] += 1,
            Kind::Other => {}
        }
    }
    if let Some(disk) = x.storage(0) {
        c[3] = disk.sectors().min(u64::from(u32::MAX)) as u32;
    }
    // SAFETY: caller guarantees `out` points at four u32s.
    unsafe { core::ptr::copy_nonoverlapping(c.as_ptr(), out, 4) };
    devs.len() as i32
}

/// Brings up an NVMe controller; returns the namespace size in sectors (0 on failure).
///
/// # Safety
/// Same contract as [`loos_xhci_probe`] with `dma_len >= loos_drivers::nvme::ARENA_BYTES`. The C NVMe driver
/// must not own this controller.
#[no_mangle]
pub unsafe extern "C" fn loos_nvme_probe(mmio: u64, dma_virt: *mut u8, dma_phys: u64, dma_len: u64) -> u64 {
    let dma = RawDma { virt: dma_virt, phys: dma_phys, len: dma_len as usize };
    loos_drivers::nvme::Nvme::new(RawMmio(mmio as *mut u8), dma).map_or(0, |n| n.sectors())
}

/// Brings up an AHCI HBA; returns the number of SATA disks found.
///
/// # Safety
/// Same contract with `dma_len >= loos_drivers::ahci::PORT_REGION` per disk. The C AHCI driver must not own it.
#[no_mangle]
pub unsafe extern "C" fn loos_ahci_probe(mmio: u64, dma_virt: *mut u8, dma_phys: u64, dma_len: u64) -> i32 {
    let dma = RawDma { virt: dma_virt, phys: dma_phys, len: dma_len as usize };
    loos_drivers::ahci::Ahci::new(RawMmio(mmio as *mut u8), dma).map_or(-1, |a| a.disks().len() as i32)
}

#[allow(dead_code)]
fn _keep(_: Vec<u8>) {}

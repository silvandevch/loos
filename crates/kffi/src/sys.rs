//! Boot-time bring-up of the Rust subsystems from the C kernel: session manager, compositor + D3D11
//! bridge, TCP/IP stack, and the xHCI/NVMe/AHCI drivers over real MMIO and DMA memory.
//! Each `*_selftest` returns a bitmask (one bit per check, all ones = pass) which `kmain` prints.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};
use loos_compositor::d3d11::{Context, Format, SwapChain, SwapEffect};
use loos_compositor::Compositor;
use core::cell::UnsafeCell;
use loos_compositor::SurfaceId;
use loos_drivers::e1000::E1000;
use loos_drivers::gpu::{BochsVga, Gpu};
use loos_drivers::hw::{DmaArena, Mmio, Nic};
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

// ---------------------------------------------------------------------------------- boot-time services

/// Single-CPU boot-time global (same discipline as `EXEC` in lib.rs).
struct Global<T>(UnsafeCell<Option<T>>);
// SAFETY: only touched from the BSP main loop.
unsafe impl<T> Sync for Global<T> {}
impl<T> Global<T> {
    const fn new() -> Self {
        Global(UnsafeCell::new(None))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut Option<T> {
        // SAFETY: see the Sync impl.
        unsafe { &mut *self.0.get() }
    }
}

struct Desktop {
    comp: Compositor,
    surface: SurfaceId,
}
static DESKTOP: Global<Desktop> = Global::new();

/// Presents the rectangle `(x, y, w, h)` of the shell's `back` buffer (`bw` x `bh`, 0xAARRGGBB) to the
/// scan-out buffer through the compositor. Returns 0 on success, -1 when the caller must blit itself
/// (unsupported pixel layout or no memory).
///
/// # Safety
/// `back` points at `bw * bh` pixels; `screen` at `bh` rows of `pitch` bytes of 32-bit BGRX scan-out memory.
#[no_mangle]
pub unsafe extern "C" fn loos_comp_present(back: *const u32, bw: i32, bh: i32, x: i32, y: i32, w: i32, h: i32, screen: *mut u8, pitch: u32) -> i32 {
    let d = DESKTOP.get();
    if d.is_none() {
        let mut comp = Compositor::new(bw, bh);
        let Ok(surface) = comp.create_surface(0, 0, bw, bh) else { return -1 };
        let _ = comp.set_opaque(surface, true);
        *d = Some(Desktop { comp, surface });
    }
    let d = d.as_mut().unwrap();
    if d.comp.size() != (bw, bh) {
        return -1;
    }
    // SAFETY: per the function contract.
    let src = unsafe { core::slice::from_raw_parts(back, (bw * bh) as usize) };
    let r = loos_compositor::Rect::new(x, y, w, h);
    if d.comp.commit(d.surface, src, Some(r)).is_err() {
        return -1;
    }
    // SAFETY: scan-out memory is `bh` rows of `pitch` bytes; compose writes only damaged rows inside it.
    let out = unsafe { core::slice::from_raw_parts_mut(screen.cast::<u32>(), (pitch as usize / 4) * bh as usize) };
    d.comp.compose(out, pitch as usize / 4);
    0
}

#[path = "webnet.rs"]
mod webnet;

struct NetState {
    stack: Stack,
    nic: E1000<RawMmio, RawDma>,
    iface: usize,
    dhcp: Option<loos_net::dhcp::Dhcp>,
    /// Address, prefix, gateway and DNS server in use; `ip == 0` until configured.
    cfg: NetConfig,
    /// When DHCP started (ms), for the fall-back to the QEMU user-network defaults.
    since: Option<u64>,
}
#[derive(Clone, Copy, Default)]
struct NetConfig {
    ip: u32,
    prefix: u32,
    gw: u32,
    dns: u32,
}
static NET: Global<NetState> = Global::new();
const DHCP_FALLBACK_MS: u64 = 12_000;

impl NetState {
    fn apply(&mut self, ip: [u8; 4], prefix: u8, gw: [u8; 4], dns: [u8; 4]) {
        let gwo = (gw != [0; 4]).then(|| Ipv4Addr::from(gw));
        self.stack.set_ipv4(self.iface, Ipv4Addr::from(ip), prefix, gwo);
        self.cfg = NetConfig { ip: u32::from_be_bytes(ip), prefix: u32::from(prefix), gw: u32::from_be_bytes(gw), dns: u32::from_be_bytes(dns) };
    }
}

/// Brings up an e1000 NIC and wires it to the TCP/IP stack. With `ip == 0` the address, gateway and DNS server come from DHCP
/// (see [`loos_net_config`]); otherwise `ip/24` with default gateway `gw` is configured statically.
/// Returns 0, or a negative error.
///
/// # Safety
/// Same contract as [`loos_xhci_probe`] with `dma_len >= loos_drivers::e1000::ARENA_BYTES`; the arena must stay valid forever.
#[no_mangle]
pub unsafe extern "C" fn loos_net_attach_e1000(mmio: u64, dma_virt: *mut u8, dma_phys: u64, dma_len: u64, ip: u32, gw: u32) -> i32 {
    let dma = RawDma { virt: dma_virt, phys: dma_phys, len: dma_len as usize };
    let Ok(nic) = E1000::new(RawMmio(mmio as *mut u8), dma) else { return -1 };
    let mut stack = Stack::new();
    let mac = nic.mac();
    let iface = stack.add_iface(mac, &[]);
    let xid = u32::from_le_bytes([mac[2], mac[3], mac[4], mac[5]]) ^ 0x4c6f_4f53;
    let mut st = NetState { stack, nic, iface, dhcp: None, cfg: NetConfig::default(), since: None };
    if ip != 0 {
        st.apply(ip.to_be_bytes(), 24, gw.to_be_bytes(), [10, 0, 2, 3]);
    } else {
        st.dhcp = Some(loos_net::dhcp::Dhcp::new(mac, xid));
    }
    *NET.get() = Some(st);
    0
}

/// Current configuration as `[ip, prefix, gateway, dns]` (host-order addresses); `ip == 0` while DHCP is still working.
/// Returns 1 once configured, 0 otherwise, -1 without a NIC.
///
/// # Safety
/// `out` must point at four writable `u32`s.
#[no_mangle]
pub unsafe extern "C" fn loos_net_config(out: *mut u32) -> i32 {
    let Some(n) = NET.get().as_ref() else { return -1 };
    // SAFETY: per contract.
    unsafe { out.copy_from_nonoverlapping([n.cfg.ip, n.cfg.prefix, n.cfg.gw, n.cfg.dns].as_ptr(), 4) };
    i32::from(n.cfg.ip != 0)
}

/// Moves frames between the NIC and the stack and runs stack timers. Cheap when no NIC is attached.
#[no_mangle]
pub extern "C" fn loos_net_poll(now_ms: u64) {
    let Some(n) = NET.get() else { return };
    n.stack.poll(now_ms);
    if n.dhcp.is_some() && n.since.is_none() {
        n.since = Some(now_ms);
    }
    let mut buf = [0u8; 2048];
    for _ in 0..32 {
        let Some(len) = n.nic.recv(&mut buf) else { break };
        if n.dhcp.as_mut().is_some_and(|d| d.rx(&buf[..len], now_ms)) {
            continue;
        }
        n.stack.rx(1, &buf[..len]);
    }
    if let Some(d) = n.dhcp.as_mut() {
        if let Some(f) = d.poll(now_ms) {
            let _ = n.nic.send(&f);
        }
        match d.lease() {
            Some(l) if n.cfg.ip != u32::from_be_bytes(l.ip) => {
                n.apply(l.ip, l.prefix, l.gw, if l.dns == [0; 4] { l.gw } else { l.dns });
            }
            None if n.cfg.ip == 0 && n.since.is_some_and(|s| now_ms.saturating_sub(s) > DHCP_FALLBACK_MS) => {
                n.apply([10, 0, 2, 15], 24, [10, 0, 2, 2], [10, 0, 2, 3]); // no DHCP server: QEMU user-network defaults
            }
            _ => {}
        }
    }
while let Some((_, f)) = n.stack.pop_tx() {
        let _ = n.nic.send(&f);
    }
}

/// Sends an ICMP echo request to `ip` (host order) and returns true once the reply for `seq` has arrived.
#[no_mangle]
pub extern "C" fn loos_net_ping(ip: u32, seq: u16, send: i32, now_ms: u64) -> i32 {
    let Some(n) = NET.get() else { return -1 };
    n.stack.poll(now_ms); // the stack's clock must be current before queuing anything time-stamped
    let dst = IpAddr::V4(Ipv4Addr::from(ip));
    if send != 0 {
        let _ = n.stack.ping(dst, 0x4c4f, seq, b"LoOS");
        return 0;
    }
    while let Some((from, id, s)) = n.stack.ping_reply() {
        if from == dst && id == 0x4c4f && s == seq {
            return 1;
        }
    }
    0
}

static GPU: Global<BochsVga<RawMmio>> = Global::new();

/// Probes a Bochs/QEMU VGA adapter (`mmio` = BAR2, dispi registers at +0x500). Returns the number of modes
/// it can set (0 if it is not a dispi adapter).
///
/// # Safety
/// `mmio` must be the mapped BAR.
#[no_mangle]
pub unsafe extern "C" fn loos_gpu_probe(mmio: u64) -> u32 {
    match BochsVga::new(RawMmio(mmio as *mut u8), 0x500) {
        Ok(g) => {
            let n = g.modes().len() as u32;
            *GPU.get() = Some(g);
            n
        }
        Err(_) => 0,
    }
}

/// Sets `w`x`h` on the probed adapter (two stacked pages). Returns 0 or -1.
#[no_mangle]
pub extern "C" fn loos_gpu_set_mode(w: u32, h: u32) -> i32 {
    match GPU.get() {
        Some(g) => i32::from(g.set_mode(loos_drivers::gpu::Mode { w, h }).is_err()) * -1,
        None => -1,
    }
}


// ---------------------------------------------------------------------------------- block controllers

type AhciCtl = loos_drivers::ahci::Ahci<RawMmio, RawDma>;
type NvmeCtl = loos_drivers::nvme::Nvme<RawMmio, RawDma>;
static AHCI: Global<Vec<AhciCtl>> = Global::new();
static NVME: Global<Vec<NvmeCtl>> = Global::new();

fn copy_model(model: &str, out: *mut u8, cap: u32) {
    let n = model.len().min(cap.saturating_sub(1) as usize);
    // SAFETY: caller guarantees `out` has `cap` writable bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(model.as_ptr(), out, n);
        *out.add(n) = 0;
    }
}

/// Brings up an AHCI HBA. Returns its controller index, or -1. The arena must be `PORT_REGION` bytes per possible
/// disk (4 KiB aligned) and stay valid forever.
///
/// # Safety
/// `mmio` is the mapped ABAR; `dma_*` describe the arena.
#[no_mangle]
pub unsafe extern "C" fn loos_ahci_attach(mmio: u64, dma_virt: *mut u8, dma_phys: u64, dma_len: u64) -> i32 {
    let dma = RawDma { virt: dma_virt, phys: dma_phys, len: dma_len as usize };
    let Ok(c) = loos_drivers::ahci::Ahci::new(RawMmio(mmio as *mut u8), dma) else { return -1 };
    let v = AHCI.get().get_or_insert_with(Vec::new);
    v.push(c);
    v.len() as i32 - 1
}

/// Disk count of an attached AHCI controller.
#[no_mangle]
pub extern "C" fn loos_ahci_disks(ctl: i32) -> i32 {
    AHCI.get().as_ref().and_then(|v| v.get(ctl as usize)).map_or(0, |c| c.disks().len() as i32)
}

/// Capacity (512-byte sectors) and model of one AHCI disk. Returns 0 on success.
///
/// # Safety
/// `model` must have `cap` writable bytes; `sectors` must be writable.
#[no_mangle]
pub unsafe extern "C" fn loos_ahci_info(ctl: i32, disk: i32, sectors: *mut u64, model: *mut u8, cap: u32) -> i32 {
    let Some(d) = AHCI.get().as_ref().and_then(|v| v.get(ctl as usize)).and_then(|c| c.disks().get(disk as usize)) else { return -1 };
    // SAFETY: per contract.
    unsafe { *sectors = d.sectors };
    copy_model(&d.model, model, cap);
    0
}

/// Reads or writes `n` 512-byte sectors. Returns 0 or -1.
///
/// # Safety
/// `buf` must be `n * 512` bytes.
#[no_mangle]
pub unsafe extern "C" fn loos_ahci_rw(ctl: i32, disk: i32, lba: u64, n: u32, buf: *mut u8, write: i32) -> i32 {
    let Some(c) = AHCI.get().as_mut().and_then(|v| v.get_mut(ctl as usize)) else { return -1 };
    let Some(mut d) = c.disk(disk as usize) else { return -1 };
    // SAFETY: per contract.
    let b = unsafe { core::slice::from_raw_parts_mut(buf, n as usize * 512) };
    let r = if write != 0 { d.write(lba, b) } else { d.read(lba, b) };
    i32::from(r.is_err()) * -1
}

/// Brings up an NVMe controller (namespace 1). Returns its controller index, or -1.
///
/// # Safety
/// `dma_len >= loos_drivers::nvme::ARENA_BYTES`; the arena stays valid forever.
#[no_mangle]
pub unsafe extern "C" fn loos_nvme_attach(mmio: u64, dma_virt: *mut u8, dma_phys: u64, dma_len: u64) -> i32 {
    let dma = RawDma { virt: dma_virt, phys: dma_phys, len: dma_len as usize };
    let Ok(c) = loos_drivers::nvme::Nvme::new(RawMmio(mmio as *mut u8), dma) else { return -1 };
    let v = NVME.get().get_or_insert_with(Vec::new);
    v.push(c);
    v.len() as i32 - 1
}

/// Namespace capacity in units of the namespace's own sector size, that size, and the model string.
///
/// # Safety
/// `model` must have `cap` writable bytes; `sectors` and `sector_size` must be writable.
#[no_mangle]
pub unsafe extern "C" fn loos_nvme_info(ctl: i32, sectors: *mut u64, sector_size: *mut u32, model: *mut u8, cap: u32) -> i32 {
    let Some(c) = NVME.get().as_ref().and_then(|v| v.get(ctl as usize)) else { return -1 };
    // SAFETY: per contract.
    unsafe {
        *sectors = c.sectors();
        *sector_size = c.sector_size();
    }
    copy_model(&c.model, model, cap);
    0
}

/// Reads or writes `n` namespace sectors. Returns 0 or -1.
///
/// # Safety
/// `buf` must be `n * sector_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn loos_nvme_rw(ctl: i32, lba: u64, n: u32, buf: *mut u8, write: i32) -> i32 {
    let Some(c) = NVME.get().as_mut().and_then(|v| v.get_mut(ctl as usize)) else { return -1 };
    // SAFETY: per contract.
    let b = unsafe { core::slice::from_raw_parts_mut(buf, n as usize * c.sector_size() as usize) };
    let r = if write != 0 { c.write(lba, b) } else { c.read(lba, b) };
    i32::from(r.is_err()) * -1
}

/// The kernel's network stack as the Winsock layer sees it.
pub(crate) struct KNet;

impl KNet {
    /// A handle on the stack if a NIC has been attached.
    pub(crate) fn present() -> bool {
        NET.get().is_some()
    }
}

impl loos_winrt::ws2::NetOps for KNet {
    fn poll(&mut self, now_ms: u64) {
        loos_net_poll(now_ms);
    }
    fn tcp_connect(&mut self, ip: [u8; 4], port: u16) -> Option<usize> {
        NET.get().as_mut()?.stack.tcp_connect(IpAddr::V4(Ipv4Addr::from(ip)), port).ok()
    }
    fn tcp_state(&mut self, h: usize) -> loos_winrt::ws2::TcpState {
        use loos_winrt::ws2::TcpState as T;
        let Some(n) = NET.get().as_mut() else { return T::Failed };
        match n.stack.tcp_state(h) {
            Some(State::SynSent | State::SynRcvd) => T::Connecting,
            Some(State::Closed) | None => if n.stack.tcp_failure(h).is_some() { T::Failed } else { T::Closed },
            Some(State::Listen) => T::Connecting,
            Some(_) => T::Established,
        }
    }
    fn tcp_send(&mut self, h: usize, data: &[u8]) -> Option<usize> {
        NET.get().as_mut()?.stack.tcp_send(h, data).ok()
    }
    fn tcp_recv(&mut self, h: usize, buf: &mut [u8]) -> usize {
        NET.get().as_mut().and_then(|n| n.stack.tcp_recv(h, buf).ok()).unwrap_or(0)
    }
    fn tcp_eof(&mut self, h: usize) -> bool {
        NET.get().as_mut().is_some_and(|n| n.stack.tcp_eof(h))
    }
    fn tcp_readable(&mut self, h: usize) -> bool {
        // A listener is readable when a connection waits; a stream when data is queued.
        NET.get().as_mut().is_some_and(|n| n.stack.tcp_pending(h))
    }
    fn tcp_close(&mut self, h: usize) {
        if let Some(n) = NET.get().as_mut() {
            let _ = n.stack.tcp_close(h);
        }
    }
    fn tcp_listen(&mut self, port: u16) -> Option<usize> {
        NET.get().as_mut()?.stack.tcp_listen(port).ok()
    }
    fn tcp_accept(&mut self, l: usize) -> Option<usize> {
        NET.get().as_mut()?.stack.tcp_accept(l)
    }
    fn udp_bind(&mut self, port: u16) -> Option<usize> {
        NET.get().as_mut()?.stack.udp_bind(port).ok()
    }
    fn udp_send(&mut self, h: usize, ip: [u8; 4], port: u16, data: &[u8]) -> bool {
        NET.get().as_mut().is_some_and(|n| n.stack.udp_send(h, IpAddr::V4(Ipv4Addr::from(ip)), port, data).is_ok())
    }
    fn udp_recv(&mut self, h: usize) -> Option<([u8; 4], u16, Vec<u8>)> {
        let (ip, port, d) = NET.get().as_mut()?.stack.udp_recv(h)?;
        match ip {
            IpAddr::V4(v) => Some((v.octets(), port, d)),
            IpAddr::V6(_) => None,
        }
    }
    fn dns_server(&self) -> [u8; 4] {
        NET.get().as_ref().map_or([10, 0, 2, 3], |n| n.cfg.dns.to_be_bytes())
    }
}

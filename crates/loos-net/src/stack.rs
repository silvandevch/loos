//! The stack proper: interfaces, neighbour resolution (ARP/NDP), IP in/out, ICMP, UDP and TCP sockets.

use crate::route::{prefix_match, Route, RouteTable};
use crate::tcp::{self, Seg, State, Tcb};
use crate::wire::*;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

const LOOPBACK: usize = 0;
const ND_MAX_AGE: u64 = 3000;
const BACKLOG: usize = 16;

/// Socket / stack errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    /// No route to the destination.
    NoRoute,
    /// Port already bound.
    InUse,
    /// Bad handle or wrong socket kind.
    BadSocket,
    /// Payload does not fit.
    TooBig,
}

/// A configured network interface.
#[derive(Debug, Clone)]
pub struct Iface {
    /// Hardware address.
    pub mac: Mac,
    /// Assigned addresses with prefix lengths.
    pub addrs: Vec<(IpAddr, u8)>,
    /// Loopback interfaces never touch the wire.
    pub loopback: bool,
}

struct Pending {
    iface: usize,
    next: IpAddr,
    pkt: Vec<u8>,
    since: u64,
}

struct UdpSock {
    port: u16,
    rx: VecDeque<(IpAddr, u16, Vec<u8>)>,
}

struct TcpSock {
    tcb: Tcb,
    listen_port: Option<u16>,
    parent: Option<usize>,
    accepted: bool,
}

/// The network stack. Drive it with [`Stack::rx`], [`Stack::poll`] and [`Stack::pop_tx`].
pub struct Stack {
    ifaces: Vec<Iface>,
    routes: RouteTable,
    neigh: BTreeMap<IpAddr, Mac>,
    pending: Vec<Pending>,
    txq: VecDeque<(usize, Vec<u8>)>,
    loopq: VecDeque<Vec<u8>>,
    udp: Vec<Option<UdpSock>>,
    tcp: Vec<TcpSock>,
    pings: VecDeque<(IpAddr, u16, u16)>,
    now: u64,
    ip_id: u16,
    next_port: u16,
    isn: u32,
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}

fn is_mcast(a: IpAddr) -> bool {
    match a {
        IpAddr::V4(v) => v.is_multicast() || v.is_broadcast(),
        IpAddr::V6(v) => v.octets()[0] == 0xff,
    }
}

fn solicited_node(t: Ipv6Addr) -> Ipv6Addr {
    let o = t.octets();
    Ipv6Addr::from([0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xff, o[13], o[14], o[15]])
}

fn mcast_mac(a: Ipv6Addr) -> Mac {
    let o = a.octets();
    [0x33, 0x33, o[12], o[13], o[14], o[15]]
}

impl Stack {
    /// New stack with the loopback interface (127.0.0.1/8, ::1/128) already routed.
    pub fn new() -> Self {
        let mut s = Stack {
            ifaces: Vec::new(),
            routes: RouteTable::default(),
            neigh: BTreeMap::new(),
            pending: Vec::new(),
            txq: VecDeque::new(),
            loopq: VecDeque::new(),
            udp: Vec::new(),
            tcp: Vec::new(),
            pings: VecDeque::new(),
            now: 0,
            ip_id: 1,
            next_port: 49152,
            isn: 0x1000,
        };
        s.ifaces.push(Iface { mac: [0; 6], addrs: alloc::vec![(IpAddr::V4(Ipv4Addr::LOCALHOST), 8), (IpAddr::V6(Ipv6Addr::LOCALHOST), 128)], loopback: true });
        s.routes.add(Route { dst: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 0)), prefix: 8, gw: None, iface: LOOPBACK, metric: 0 });
        s.routes.add(Route { dst: IpAddr::V6(Ipv6Addr::LOCALHOST), prefix: 128, gw: None, iface: LOOPBACK, metric: 0 });
        s
    }

    /// Adds an Ethernet interface with its addresses; installs on-link routes for each. Returns its index.
    pub fn add_iface(&mut self, mac: Mac, addrs: &[(IpAddr, u8)]) -> usize {
        let idx = self.ifaces.len();
        self.ifaces.push(Iface { mac, addrs: addrs.to_vec(), loopback: false });
        for &(a, p) in addrs {
            self.routes.add(Route { dst: a, prefix: p, gw: None, iface: idx, metric: 10 });
        }
        idx
    }

    /// Adds a route (e.g. a default route: `0.0.0.0/0` via a gateway).
    pub fn add_route(&mut self, r: Route) {
        self.routes.add(r);
    }

    /// The routing table.
    pub fn routes(&self) -> &RouteTable {
        &self.routes
    }

    /// Next frame to transmit, as `(iface, frame)`.
    pub fn pop_tx(&mut self) -> Option<(usize, Vec<u8>)> {
        self.txq.pop_front()
    }

    fn is_local(&self, a: IpAddr) -> bool {
        self.ifaces.iter().any(|i| i.addrs.iter().any(|&(x, _)| x == a))
    }

    fn local_or_group(&self, a: IpAddr) -> bool {
        if self.is_local(a) || a == IpAddr::V4(Ipv4Addr::BROADCAST) {
            return true;
        }
        match a {
            IpAddr::V6(v) if v.octets()[0] == 0xff => {
                v == Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1)
                    || self.ifaces.iter().any(|i| i.addrs.iter().any(|&(x, _)| matches!(x, IpAddr::V6(t) if solicited_node(t) == v)))
            }
            IpAddr::V4(v) if v.is_loopback() => true,
            _ => false,
        }
    }

    fn pick_src(&self, dst: IpAddr) -> Option<IpAddr> {
        if self.is_local(dst) || matches!(dst, IpAddr::V4(v) if v.is_loopback()) {
            return Some(dst);
        }
        let r = self.routes.lookup(dst)?;
        self.ifaces[r.iface].addrs.iter().map(|a| a.0).find(|a| a.is_ipv4() == dst.is_ipv4())
    }

    fn eph_port(&mut self) -> u16 {
        loop {
            let p = self.next_port;
            self.next_port = if p == 65535 { 49152 } else { p + 1 };
            let used = self.tcp.iter().any(|s| s.tcb.local.1 == p && s.tcb.state != State::Closed) || self.udp.iter().flatten().any(|u| u.port == p);
            if !used {
                return p;
            }
        }
    }

    // ------------------------------------------------------------------ output
    fn frame(&mut self, iface: usize, dst: Mac, et: u16, payload: &[u8]) {
        let f = build_eth(dst, self.ifaces[iface].mac, et, payload);
        self.txq.push_back((iface, f));
    }

    /// Sends an upper-layer message. `src` of `None` picks the source from the route.
    pub fn ip_output(&mut self, src: Option<IpAddr>, dst: IpAddr, proto: u8, payload: &[u8]) -> Result<(), NetError> {
        let src = match src {
            Some(s) => s,
            None => self.pick_src(dst).ok_or(NetError::NoRoute)?,
        };
        self.ip_id = self.ip_id.wrapping_add(1);
        let ttl = if proto == P_ICMP6 { 255 } else { 64 };
        let pkt = build_ip(src, dst, proto, ttl, self.ip_id, payload).ok_or(NetError::TooBig)?;
        if self.is_local(dst) || matches!(dst, IpAddr::V4(v) if v.is_loopback()) {
            self.loopq.push_back(pkt);
            return Ok(());
        }
        let r = *self.routes.lookup(dst).ok_or(NetError::NoRoute)?;
        if self.ifaces[r.iface].loopback {
            self.loopq.push_back(pkt);
            return Ok(());
        }
        self.output_on(r.iface, r.gw.unwrap_or(dst), pkt);
        Ok(())
    }

    fn output_on(&mut self, iface: usize, next: IpAddr, pkt: Vec<u8>) {
        let et = if next.is_ipv4() { ETH_IPV4 } else { ETH_IPV6 };
        if is_mcast(next) {
            let mac = match next {
                IpAddr::V6(v) => mcast_mac(v),
                _ => BCAST,
            };
            return self.frame(iface, mac, et, &pkt);
        }
        if let Some(&m) = self.neigh.get(&next) {
            return self.frame(iface, m, et, &pkt);
        }
        let first = !self.pending.iter().any(|p| p.next == next);
        self.pending.push(Pending { iface, next, pkt, since: self.now });
        if first {
            self.solicit(iface, next);
        }
    }

    fn solicit(&mut self, iface: usize, target: IpAddr) {
        let mac = self.ifaces[iface].mac;
        match target {
            IpAddr::V4(t) => {
                let Some(IpAddr::V4(spa)) = self.ifaces[iface].addrs.iter().map(|a| a.0).find(|a| a.is_ipv4()) else { return };
                let mut a = Vec::with_capacity(28);
                a.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
                a.extend_from_slice(&mac);
                a.extend_from_slice(&spa.octets());
                a.extend_from_slice(&[0; 6]);
                a.extend_from_slice(&t.octets());
                self.frame(iface, BCAST, ETH_ARP, &a);
            }
            IpAddr::V6(t) => {
                let Some(IpAddr::V6(src)) = self.ifaces[iface].addrs.iter().map(|a| a.0).find(|a| a.is_ipv6()) else { return };
                let mut m = alloc::vec![135, 0, 0, 0, 0, 0, 0, 0];
                m.extend_from_slice(&t.octets());
                m.extend_from_slice(&[1, 1]);
                m.extend_from_slice(&mac);
                self.send_icmp6(iface, src, solicited_node(t), m);
            }
        }
    }

    fn send_icmp6(&mut self, iface: usize, src: Ipv6Addr, dst: Ipv6Addr, mut m: Vec<u8>) {
        let c = l4_checksum(IpAddr::V6(src), IpAddr::V6(dst), P_ICMP6, &m);
        m[2..4].copy_from_slice(&c.to_be_bytes());
        self.ip_id = self.ip_id.wrapping_add(1);
        let Some(pkt) = build_ip(IpAddr::V6(src), IpAddr::V6(dst), P_ICMP6, 255, self.ip_id, &m) else { return };
        let dmac = if dst.octets()[0] == 0xff { mcast_mac(dst) } else { self.neigh.get(&IpAddr::V6(dst)).copied().unwrap_or(BCAST) };
        self.frame(iface, dmac, ETH_IPV6, &pkt);
    }

    fn learn(&mut self, ip: IpAddr, mac: Mac) {
        self.neigh.insert(ip, mac);
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].next == ip {
                let p = self.pending.remove(i);
                let et = if ip.is_ipv4() { ETH_IPV4 } else { ETH_IPV6 };
                self.frame(p.iface, mac, et, &p.pkt);
            } else {
                i += 1;
            }
        }
    }

    // ------------------------------------------------------------------ input
    /// Feeds one received Ethernet frame into the stack.
    pub fn rx(&mut self, iface: usize, frame: &[u8]) {
        let Some((dst, src, et, payload)) = parse_eth(frame) else { return };
        let mine = self.ifaces[iface].mac;
        if dst != mine && dst != BCAST && dst[0] & 1 == 0 {
            return;
        }
        match et {
            ETH_ARP => self.arp_in(iface, payload),
            ETH_IPV4 | ETH_IPV6 => self.ip_input(iface, payload, Some(src)),
            _ => {}
        }
        self.pump();
    }

    fn arp_in(&mut self, iface: usize, a: &[u8]) {
        if a.len() < 28 || a[..6] != [0, 1, 8, 0, 6, 4] {
            return;
        }
        let op = u16::from_be_bytes([a[6], a[7]]);
        let mut sha = [0u8; 6];
        sha.copy_from_slice(&a[8..14]);
        let spa = Ipv4Addr::new(a[14], a[15], a[16], a[17]);
        let tpa = Ipv4Addr::new(a[24], a[25], a[26], a[27]);
        if !spa.is_unspecified() {
            self.learn(IpAddr::V4(spa), sha);
        }
        if op == 1 && self.ifaces[iface].addrs.iter().any(|x| x.0 == IpAddr::V4(tpa)) {
            let mac = self.ifaces[iface].mac;
            let mut r = Vec::with_capacity(28);
            r.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
            r.extend_from_slice(&mac);
            r.extend_from_slice(&tpa.octets());
            r.extend_from_slice(&sha);
            r.extend_from_slice(&spa.octets());
            self.frame(iface, sha, ETH_ARP, &r);
        }
    }

    fn ip_input(&mut self, iface: usize, pkt: &[u8], eth_src: Option<Mac>) {
        let Some(ip) = parse_ip(pkt) else { return };
        if !self.local_or_group(ip.dst) {
            return;
        }
        match ip.proto {
            P_ICMP => self.icmp4_in(&ip, pkt),
            P_ICMP6 => self.icmp6_in(iface, &ip, eth_src),
            P_UDP => self.udp_in(&ip, pkt),
            P_TCP => self.tcp_in(&ip),
            _ => {}
        }
    }

    fn icmp4_in(&mut self, ip: &Ip, _raw: &[u8]) {
        let m = ip.payload;
        if m.len() < 8 || checksum(m) != 0 {
            return;
        }
        match m[0] {
            8 => {
                let mut r = m.to_vec();
                r[0] = 0;
                r[2..4].copy_from_slice(&[0, 0]);
                let c = checksum(&r);
                r[2..4].copy_from_slice(&c.to_be_bytes());
                let _ = self.ip_output(Some(ip.dst), ip.src, P_ICMP, &r);
            }
            0 => self.pings.push_back((ip.src, u16::from_be_bytes([m[4], m[5]]), u16::from_be_bytes([m[6], m[7]]))),
            _ => {}
        }
    }

    fn icmp6_in(&mut self, iface: usize, ip: &Ip, eth_src: Option<Mac>) {
        let m = ip.payload;
        if m.len() < 8 || l4_checksum(ip.src, ip.dst, P_ICMP6, m) != 0 {
            return;
        }
        let (IpAddr::V6(src), IpAddr::V6(dst)) = (ip.src, ip.dst) else { return };
        match m[0] {
            128 => {
                let mut r = m.to_vec();
                r[0] = 129;
                let me = if self.is_local(ip.dst) { dst } else { match self.pick_src(ip.src) { Some(IpAddr::V6(s)) => s, _ => return } };
                r[2..4].copy_from_slice(&[0, 0]);
                let _ = self.ip_output(Some(IpAddr::V6(me)), IpAddr::V6(src), P_ICMP6, &{
                    let c = l4_checksum(IpAddr::V6(me), IpAddr::V6(src), P_ICMP6, &r);
                    r[2..4].copy_from_slice(&c.to_be_bytes());
                    r
                });
            }
            129 => self.pings.push_back((ip.src, u16::from_be_bytes([m[4], m[5]]), u16::from_be_bytes([m[6], m[7]]))),
            135 if m.len() >= 24 && ip.ttl == 255 => {
                let mut t = [0u8; 16];
                t.copy_from_slice(&m[8..24]);
                let target = Ipv6Addr::from(t);
                let mac = if m.len() >= 32 && m[24] == 1 && m[25] == 1 { m[26..32].try_into().ok() } else { eth_src };
                if let Some(mac) = mac {
                    if !src.is_unspecified() {
                        self.learn(ip.src, mac);
                    }
                    if self.ifaces[iface].addrs.iter().any(|a| a.0 == IpAddr::V6(target)) {
                        let mut na = alloc::vec![136, 0, 0, 0, 0x60, 0, 0, 0];
                        na.extend_from_slice(&target.octets());
                        na.extend_from_slice(&[2, 1]);
                        na.extend_from_slice(&self.ifaces[iface].mac);
                        self.send_icmp6(iface, target, src, na);
                    }
                }
            }
            136 if m.len() >= 24 && ip.ttl == 255 => {
                let mut t = [0u8; 16];
                t.copy_from_slice(&m[8..24]);
                let mac = if m.len() >= 32 && m[24] == 2 && m[25] == 1 { m[26..32].try_into().ok() } else { eth_src };
                if let Some(mac) = mac {
                    self.learn(IpAddr::V6(Ipv6Addr::from(t)), mac);
                }
            }
            _ => {}
        }
    }

    fn udp_in(&mut self, ip: &Ip, raw: &[u8]) {
        let m = ip.payload;
        if m.len() < 8 {
            return;
        }
        let len = usize::from(u16::from_be_bytes([m[4], m[5]]));
        if len < 8 || len > m.len() {
            return;
        }
        let m = &m[..len];
        let csum = u16::from_be_bytes([m[6], m[7]]);
        if (csum != 0 || ip.src.is_ipv6()) && l4_checksum(ip.src, ip.dst, P_UDP, m) != 0 {
            return;
        }
        let (sport, dport) = (u16::from_be_bytes([m[0], m[1]]), u16::from_be_bytes([m[2], m[3]]));
        if let Some(Some(s)) = self.udp.iter_mut().find(|s| s.as_ref().is_some_and(|s| s.port == dport)) {
            if s.rx.len() < 64 {
                s.rx.push_back((ip.src, sport, m[8..].to_vec()));
            }
        } else if let (IpAddr::V4(_), false) = (ip.src, is_mcast(ip.dst)) {
            // ICMP destination/port unreachable carrying the offending header + 8 bytes.
            let ihl = usize::from(raw[0] & 15) * 4;
            let quote = &raw[..raw.len().min(ihl + 8)];
            let mut r = alloc::vec![3, 3, 0, 0, 0, 0, 0, 0];
            r.extend_from_slice(quote);
            let c = checksum(&r);
            r[2..4].copy_from_slice(&c.to_be_bytes());
            let _ = self.ip_output(Some(ip.dst), ip.src, P_ICMP, &r);
        }
    }

    fn tcp_in(&mut self, ip: &Ip) {
        let Some(seg) = tcp::decode(ip.src, ip.dst, ip.payload) else { return };
        let now = self.now;
        let local = (ip.dst, seg.dport);
        let remote = (ip.src, seg.sport);
        if let Some(s) = self.tcp.iter_mut().find(|s| s.listen_port.is_none() && s.tcb.state != State::Closed && s.tcb.local == local && s.tcb.remote == remote) {
            s.tcb.on_segment(&seg, now);
            return;
        }
        if seg.flags & tcp::SYN != 0 && seg.flags & (tcp::ACK | tcp::RST) == 0 {
            if let Some(l) = self.tcp.iter().position(|s| s.listen_port == Some(seg.dport) && s.tcb.state == State::Listen) {
                let pending = self.tcp.iter().filter(|s| s.parent == Some(l) && !s.accepted).count();
                if pending < BACKLOG {
                    self.isn = self.isn.wrapping_add(64000).wrapping_add(now as u32 * 250);
                    let tcb = Tcb::accept(local, remote, &seg, self.isn, now);
                    self.tcp.push(TcpSock { tcb, listen_port: None, parent: Some(l), accepted: false });
                    return;
                }
            }
        }
        if seg.flags & tcp::RST == 0 {
            let rst = if seg.flags & tcp::ACK != 0 {
                Seg { sport: seg.dport, dport: seg.sport, seq: seg.ack, ack: 0, flags: tcp::RST, win: 0, data: Vec::new() }
            } else {
                Seg { sport: seg.dport, dport: seg.sport, seq: 0, ack: seg.seq.wrapping_add(seg.seq_len()), flags: tcp::RST | tcp::ACK, win: 0, data: Vec::new() }
            };
            let b = tcp::encode(ip.dst, ip.src, &rst);
            let _ = self.ip_output(Some(ip.dst), ip.src, P_TCP, &b);
        }
    }

    // ------------------------------------------------------------------ time / pump
    /// Advances time (milliseconds, monotonic) and runs timers.
    pub fn poll(&mut self, now_ms: u64) {
        self.now = now_ms;
        let n = self.now;
        self.pending.retain(|p| n.saturating_sub(p.since) < ND_MAX_AGE);
        self.pump();
    }

    fn pump(&mut self) {
        for _ in 0..256 {
            let mut busy = false;
            while let Some(p) = self.loopq.pop_front() {
                busy = true;
                self.ip_input(LOOPBACK, &p, None);
            }
            let now = self.now;
            let mut out: Vec<(IpAddr, IpAddr, Vec<u8>)> = Vec::new();
            for s in self.tcp.iter_mut().filter(|s| s.listen_port.is_none()) {
                s.tcb.poll(now);
                let (l, r) = (s.tcb.local.0, s.tcb.remote.0);
                for seg in s.tcb.take_out() {
                    out.push((l, r, tcp::encode(l, r, &seg)));
                }
            }
            for (l, r, b) in out {
                busy = true;
                let _ = self.ip_output(Some(l), r, P_TCP, &b);
            }
            if !busy {
                break;
            }
        }
    }

    // ------------------------------------------------------------------ ICMP echo
    /// Sends an echo request.
    pub fn ping(&mut self, dst: IpAddr, id: u16, seq: u16, payload: &[u8]) -> Result<(), NetError> {
        let src = self.pick_src(dst).ok_or(NetError::NoRoute)?;
        let mut m = alloc::vec![if dst.is_ipv4() { 8 } else { 128 }, 0, 0, 0];
        m.extend_from_slice(&id.to_be_bytes());
        m.extend_from_slice(&seq.to_be_bytes());
        m.extend_from_slice(payload);
        let c = if dst.is_ipv4() { checksum(&m) } else { l4_checksum(src, dst, P_ICMP6, &m) };
        m[2..4].copy_from_slice(&c.to_be_bytes());
        self.ip_output(Some(src), dst, if dst.is_ipv4() { P_ICMP } else { P_ICMP6 }, &m)?;
        self.pump();
        Ok(())
    }

    /// Next received echo reply as `(from, id, seq)`.
    pub fn ping_reply(&mut self) -> Option<(IpAddr, u16, u16)> {
        self.pings.pop_front()
    }

    // ------------------------------------------------------------------ UDP
    /// Binds a UDP socket (`port` 0 picks an ephemeral port).
    pub fn udp_bind(&mut self, port: u16) -> Result<usize, NetError> {
        let port = if port == 0 { self.eph_port() } else { port };
        if self.udp.iter().flatten().any(|u| u.port == port) {
            return Err(NetError::InUse);
        }
        self.udp.push(Some(UdpSock { port, rx: VecDeque::new() }));
        Ok(self.udp.len() - 1)
    }

    /// Sends a datagram from socket `h`.
    pub fn udp_send(&mut self, h: usize, dst: IpAddr, dport: u16, data: &[u8]) -> Result<(), NetError> {
        let sport = self.udp.get(h).and_then(|s| s.as_ref()).ok_or(NetError::BadSocket)?.port;
        let src = self.pick_src(dst).ok_or(NetError::NoRoute)?;
        let len = u16::try_from(8 + data.len()).map_err(|_| NetError::TooBig)?;
        let mut m = Vec::with_capacity(usize::from(len));
        m.extend_from_slice(&sport.to_be_bytes());
        m.extend_from_slice(&dport.to_be_bytes());
        m.extend_from_slice(&len.to_be_bytes());
        m.extend_from_slice(&[0, 0]);
        m.extend_from_slice(data);
        let c = match l4_checksum(src, dst, P_UDP, &m) {
            0 => 0xffff,
            c => c,
        };
        m[6..8].copy_from_slice(&c.to_be_bytes());
        self.ip_output(Some(src), dst, P_UDP, &m)?;
        self.pump();
        Ok(())
    }

    /// Next datagram as `(from, from_port, data)`.
    pub fn udp_recv(&mut self, h: usize) -> Option<(IpAddr, u16, Vec<u8>)> {
        self.udp.get_mut(h)?.as_mut()?.rx.pop_front()
    }

    /// Closes a UDP socket.
    pub fn udp_close(&mut self, h: usize) {
        if let Some(s) = self.udp.get_mut(h) {
            *s = None;
        }
    }

    // ------------------------------------------------------------------ TCP
    /// Listens on `port` (any local address).
    pub fn tcp_listen(&mut self, port: u16) -> Result<usize, NetError> {
        if self.tcp.iter().any(|s| s.listen_port == Some(port)) {
            return Err(NetError::InUse);
        }
        let tcb = Tcb::listener((IpAddr::V4(Ipv4Addr::UNSPECIFIED), port));
        self.tcp.push(TcpSock { tcb, listen_port: Some(port), parent: None, accepted: false });
        Ok(self.tcp.len() - 1)
    }

    /// Takes one established connection from listener `l`.
    pub fn tcp_accept(&mut self, l: usize) -> Option<usize> {
        let i = self.tcp.iter().position(|s| s.parent == Some(l) && !s.accepted && !matches!(s.tcb.state, State::SynRcvd | State::Listen))?;
        self.tcp[i].accepted = true;
        Some(i)
    }

    /// Active open.
    pub fn tcp_connect(&mut self, dst: IpAddr, port: u16) -> Result<usize, NetError> {
        let src = self.pick_src(dst).ok_or(NetError::NoRoute)?;
        let lport = self.eph_port();
        self.isn = self.isn.wrapping_add(64000).wrapping_add(self.now as u32 * 250);
        let tcb = Tcb::connect((src, lport), (dst, port), self.isn, self.now);
        self.tcp.push(TcpSock { tcb, listen_port: None, parent: None, accepted: true });
        let h = self.tcp.len() - 1;
        self.pump();
        Ok(h)
    }

    fn sock(&mut self, h: usize) -> Result<&mut Tcb, NetError> {
        match self.tcp.get_mut(h) {
            Some(s) if s.listen_port.is_none() => Ok(&mut s.tcb),
            _ => Err(NetError::BadSocket),
        }
    }

    /// Queues data; returns bytes accepted.
    pub fn tcp_send(&mut self, h: usize, data: &[u8]) -> Result<usize, NetError> {
        let n = self.sock(h)?.send(data);
        self.pump();
        Ok(n)
    }

    /// Reads received data.
    pub fn tcp_recv(&mut self, h: usize, buf: &mut [u8]) -> Result<usize, NetError> {
        let n = self.sock(h)?.recv(buf);
        self.pump();
        Ok(n)
    }

    /// Half-closes our side (FIN after queued data).
    pub fn tcp_close(&mut self, h: usize) -> Result<(), NetError> {
        self.sock(h)?.close();
        self.pump();
        Ok(())
    }

    /// Connection state.
    pub fn tcp_state(&self, h: usize) -> Option<State> {
        self.tcp.get(h).map(|s| s.tcb.state)
    }

    /// True when `h` has bytes to read, or (for a listener) an established connection to accept.
    pub fn tcp_pending(&self, h: usize) -> bool {
        match self.tcp.get(h) {
            Some(s) if s.listen_port.is_some() => self.tcp.iter().any(|c| c.parent == Some(h) && !c.accepted && !matches!(c.tcb.state, State::SynRcvd | State::Listen)),
            Some(s) => s.tcb.readable() > 0,
            None => false,
        }
    }

    /// True when the peer closed and everything was read.
    pub fn tcp_eof(&self, h: usize) -> bool {
        self.tcp.get(h).is_some_and(|s| s.tcb.eof())
    }

    /// Why the connection failed, if it did.
    pub fn tcp_failure(&self, h: usize) -> Option<tcp::Fail> {
        self.tcp.get(h).and_then(|s| s.tcb.failed)
    }

    /// True when `dst` is covered by a configured on-link prefix (used by callers for diagnostics).
    pub fn on_link(&self, dst: IpAddr) -> bool {
        self.ifaces.iter().any(|i| i.addrs.iter().any(|&(a, p)| prefix_match(a, dst, p)))
    }
}

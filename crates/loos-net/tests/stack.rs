use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use loos_net::route::{Route, RouteTable};
use loos_net::stack::Stack;
use loos_net::tcp::State;
use loos_net::wire::*;

fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}
fn v6(n: u16) -> IpAddr {
    IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, n))
}

/// Two hosts on one wire; `drop` decides whether a frame (by index) is lost.
struct Net {
    a: Stack,
    b: Stack,
    now: u64,
    n: usize,
    drop: fn(usize) -> bool,
}

impl Net {
    fn new(drop: fn(usize) -> bool) -> Net {
        let mut a = Stack::new();
        let mut b = Stack::new();
        a.add_iface([2, 0, 0, 0, 0, 1], &[(v4(10, 0, 0, 1), 24), (v6(1), 64)]);
        b.add_iface([2, 0, 0, 0, 0, 2], &[(v4(10, 0, 0, 2), 24), (v6(2), 64)]);
        Net { a, b, now: 0, n: 0, drop }
    }
    fn step(&mut self, ms: u64) {
        self.now += ms;
        self.a.poll(self.now);
        self.b.poll(self.now);
        for _ in 0..64 {
            let mut moved = false;
            while let Some((_, f)) = self.a.pop_tx() {
                moved = true;
                self.n += 1;
                if !(self.drop)(self.n) {
                    self.b.rx(1, &f);
                }
            }
            while let Some((_, f)) = self.b.pop_tx() {
                moved = true;
                self.n += 1;
                if !(self.drop)(self.n) {
                    self.a.rx(1, &f);
                }
            }
            if !moved {
                break;
            }
        }
    }
    fn run(&mut self, ms: u64) {
        for _ in 0..ms / 10 {
            self.step(10);
        }
    }
}

#[test]
fn checksum_rfc1071_vector() {
    let d = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
    assert_eq!(checksum(&d), !0xddf2u16);
}

#[test]
fn routing_longest_prefix_and_metric() {
    let mut t = RouteTable::default();
    t.add(Route { dst: v4(0, 0, 0, 0), prefix: 0, gw: Some(v4(10, 0, 0, 254)), iface: 1, metric: 5 });
    t.add(Route { dst: v4(10, 0, 0, 0), prefix: 24, gw: None, iface: 1, metric: 10 });
    t.add(Route { dst: v4(10, 0, 0, 0), prefix: 24, gw: None, iface: 2, metric: 1 });
    t.add(Route { dst: v6(0), prefix: 64, gw: None, iface: 3, metric: 0 });
    assert_eq!(t.lookup(v4(10, 0, 0, 9)).unwrap().iface, 2);
    assert_eq!(t.lookup(v4(8, 8, 8, 8)).unwrap().gw, Some(v4(10, 0, 0, 254)));
    assert_eq!(t.lookup(v6(77)).unwrap().iface, 3);
    assert!(t.lookup(IpAddr::V6(Ipv6Addr::LOCALHOST)).is_none());
}

#[test]
fn loopback_ping_both_families() {
    let mut s = Stack::new();
    s.ping(v4(127, 0, 0, 1), 7, 1, b"hi").unwrap();
    assert_eq!(s.ping_reply(), Some((v4(127, 0, 0, 1), 7, 1)));
    s.ping(IpAddr::V6(Ipv6Addr::LOCALHOST), 8, 2, b"hi6").unwrap();
    assert_eq!(s.ping_reply(), Some((IpAddr::V6(Ipv6Addr::LOCALHOST), 8, 2)));
    assert!(s.pop_tx().is_none(), "loopback never reaches the wire");
}

#[test]
fn loopback_udp_and_port_unreachable() {
    let mut s = Stack::new();
    let srv = s.udp_bind(5000).unwrap();
    assert!(s.udp_bind(5000).is_err());
    let cli = s.udp_bind(0).unwrap();
    s.udp_send(cli, v4(127, 0, 0, 1), 5000, b"ping").unwrap();
    let (from, _, d) = s.udp_recv(srv).unwrap();
    assert_eq!((from, d.as_slice()), (v4(127, 0, 0, 1), &b"ping"[..]));
    s.udp_send(cli, IpAddr::V6(Ipv6Addr::LOCALHOST), 5000, b"v6").unwrap();
    assert_eq!(s.udp_recv(srv).unwrap().2, b"v6");
    s.udp_send(cli, v4(127, 0, 0, 1), 9, b"nobody").unwrap(); // generates ICMP unreachable, must not loop
}

fn tcp_echo_roundtrip(s: &mut Stack, dst: IpAddr, size: usize) {
    let l = s.tcp_listen(8080).unwrap();
    let c = s.tcp_connect(dst, 8080).unwrap();
    let a = s.tcp_accept(l).expect("accepted");
    assert_eq!(s.tcp_state(c), Some(State::Established));
    let msg: Vec<u8> = (0..size).map(|i| (i * 7 % 251) as u8).collect();
    let mut sent = 0;
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    let mut back = Vec::new();
    for _ in 0..2000 {
        if sent < msg.len() {
            sent += s.tcp_send(c, &msg[sent..]).unwrap();
        }
        let n = s.tcp_recv(a, &mut buf).unwrap();
        got.extend_from_slice(&buf[..n]);
        s.tcp_send(a, &buf[..n]).unwrap();
        let n = s.tcp_recv(c, &mut buf).unwrap();
        back.extend_from_slice(&buf[..n]);
        if back.len() == msg.len() {
            break;
        }
    }
    assert_eq!((got.len(), back.len()), (msg.len(), msg.len()));
    assert!(got == msg);
    assert!(back == msg);
    s.tcp_close(c).unwrap();
    assert!(s.tcp_eof(a));
    s.tcp_close(a).unwrap();
    assert_eq!(s.tcp_state(a), Some(State::Closed));
    assert!(matches!(s.tcp_state(c), Some(State::FinWait2 | State::TimeWait | State::Closed)));
}

#[test]
fn loopback_tcp_echo_v4_and_v6() {
    tcp_echo_roundtrip(&mut Stack::new(), v4(127, 0, 0, 1), 300_000);
    tcp_echo_roundtrip(&mut Stack::new(), IpAddr::V6(Ipv6Addr::LOCALHOST), 100_000);
}

#[test]
fn tcp_refused_gets_reset() {
    let mut s = Stack::new();
    let c = s.tcp_connect(v4(127, 0, 0, 1), 1).unwrap();
    assert_eq!(s.tcp_state(c), Some(State::Closed));
    assert_eq!(s.tcp_failure(c), Some(loos_net::tcp::Fail::Reset));
}

#[test]
fn wire_arp_ndp_ping_udp() {
    let mut n = Net::new(|_| false);
    n.a.ping(v4(10, 0, 0, 2), 1, 1, b"x").unwrap();
    n.run(100);
    assert_eq!(n.a.ping_reply(), Some((v4(10, 0, 0, 2), 1, 1)));
    n.a.ping(v6(2), 2, 5, b"y").unwrap();
    n.run(100);
    assert_eq!(n.a.ping_reply(), Some((v6(2), 2, 5)));
    let srv = n.b.udp_bind(53).unwrap();
    let cli = n.a.udp_bind(0).unwrap();
    n.a.udp_send(cli, v4(10, 0, 0, 2), 53, b"q4").unwrap();
    n.a.udp_send(cli, v6(2), 53, b"q6").unwrap();
    n.run(50);
    assert_eq!(n.b.udp_recv(srv).unwrap().2, b"q4");
    assert_eq!(n.b.udp_recv(srv).unwrap().2, b"q6");
}

#[test]
fn wire_tcp_transfer_with_loss() {
    // Drop every 7th frame in both directions: forces SYN/data/FIN retransmits.
    let mut n = Net::new(|i| i % 7 == 0);
    let l = n.b.tcp_listen(80).unwrap();
    let c = n.a.tcp_connect(v4(10, 0, 0, 2), 80).unwrap();
    n.run(3000);
    let s = n.b.tcp_accept(l).expect("handshake survives loss");
    let msg: Vec<u8> = (0..50_000u32).map(|i| (i % 253) as u8).collect();
    let (mut sent, mut got) = (0, Vec::new());
    let mut buf = [0u8; 2048];
    for _ in 0..4000 {
        if sent < msg.len() {
            sent += n.a.tcp_send(c, &msg[sent..]).unwrap();
        }
        n.step(10);
        let k = n.b.tcp_recv(s, &mut buf).unwrap();
        got.extend_from_slice(&buf[..k]);
        if got.len() == msg.len() {
            break;
        }
    }
    assert_eq!(got.len(), msg.len());
    assert!(got == msg, "stream corrupted");
    n.a.tcp_close(c).unwrap();
    n.run(3000);
    assert!(n.b.tcp_eof(s));
    n.b.tcp_close(s).unwrap();
    n.run(6000);
    assert_eq!(n.a.tcp_state(c), Some(State::Closed));
    assert_eq!(n.b.tcp_state(s), Some(State::Closed));
}

#[test]
fn default_route_uses_gateway_mac() {
    let mut a = Stack::new();
    a.add_iface([2, 0, 0, 0, 0, 1], &[(v4(10, 0, 0, 1), 24)]);
    a.add_route(Route { dst: v4(0, 0, 0, 0), prefix: 0, gw: Some(v4(10, 0, 0, 254)), iface: 1, metric: 100 });
    let h = a.udp_bind(0).unwrap();
    a.udp_send(h, v4(8, 8, 8, 8), 53, b"x").unwrap();
    let (_, f) = a.pop_tx().unwrap();
    let (dst, _, et, p) = parse_eth(&f).unwrap();
    assert_eq!((dst, et), (BCAST, ETH_ARP));
    assert_eq!(&p[24..28], &[10, 0, 0, 254], "ARP asks for the gateway, not the destination");
}

//! DHCPv4 client (RFC 2131/2132): DISCOVER → OFFER → REQUEST → ACK, then renewal at half the lease.
//!
//! Sans-I/O like the rest of the stack: [`Dhcp::poll`] hands back the next frame to transmit, [`Dhcp::rx`] consumes received
//! frames. It builds whole Ethernet frames itself because it must run before the interface has an address.

use crate::wire::*;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr};

const CLIENT_PORT: u16 = 68;
const SERVER_PORT: u16 = 67;
const MAGIC: [u8; 4] = [99, 130, 83, 99];
const RETRY_MS: u64 = 2000;
const GIVE_UP_TRIES: u8 = 4;

/// What the server handed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lease {
    /// Our address.
    pub ip: [u8; 4],
    /// Subnet prefix length.
    pub prefix: u8,
    /// Default gateway (all zero if none was offered).
    pub gw: [u8; 4],
    /// First DNS server (all zero if none was offered).
    pub dns: [u8; 4],
    /// Lease length in seconds.
    pub secs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Discover,
    Request { ip: [u8; 4], server: [u8; 4] },
    Bound,
    Renew,
    Failed,
}

/// The client.
pub struct Dhcp {
    mac: Mac,
    xid: u32,
    state: State,
    lease: Option<Lease>,
    bound_at: u64,
    last_tx: Option<u64>,
    tries: u8,
}

impl Dhcp {
    /// A client for the interface with hardware address `mac`; `xid` should differ per boot (any entropy will do).
    pub fn new(mac: Mac, xid: u32) -> Self {
        Dhcp { mac, xid, state: State::Discover, lease: None, bound_at: 0, last_tx: None, tries: 0 }
    }

    /// The current lease, once bound (it stays valid while a renewal is in flight).
    pub fn lease(&self) -> Option<Lease> {
        self.lease
    }

    /// True when no server answered after several attempts.
    pub fn failed(&self) -> bool {
        self.state == State::Failed
    }

    /// The next frame to transmit at time `now` (ms), if any.
    pub fn poll(&mut self, now: u64) -> Option<Vec<u8>> {
        match self.state {
            State::Bound => {
                let l = self.lease?;
                if now.saturating_sub(self.bound_at) >= u64::from(l.secs) * 500 {
                    self.state = State::Renew;
                    self.last_tx = None;
                    self.tries = 0;
                }
                return None;
            }
            State::Failed => return None,
            _ => {}
        }
        if self.last_tx.is_some_and(|t| now.saturating_sub(t) < RETRY_MS) {
            return None;
        }
        if self.tries >= GIVE_UP_TRIES {
            if self.state == State::Renew {
                self.state = State::Bound; // keep the old lease; try again at the next half-lease check
                self.bound_at = now.saturating_sub(u64::from(self.lease?.secs) * 500).saturating_add(60_000);
            } else {
                self.tries = 0; // keep asking: the link may only come up seconds after the NIC reset
                self.last_tx = None;
                self.state = State::Discover;
            }
            return None;
        }
        self.last_tx = Some(now);
        self.tries += 1;
        Some(match self.state {
            State::Discover => self.build(1, None, None),
            State::Request { ip, server } => self.build(3, Some(ip), Some(server)),
            State::Renew => self.build(3, self.lease.map(|l| l.ip), None),
            _ => return None,
        })
    }

    /// Offers a received frame; returns true if it was a DHCP reply for us (the caller then does not pass it on).
    pub fn rx(&mut self, frame: &[u8], now: u64) -> bool {
        let Some((_, _, ETH_IPV4, ip)) = parse_eth(frame) else { return false };
        let Some(p) = parse_ip(ip) else { return false };
        if p.proto != P_UDP || p.payload.len() < 8 + 240 {
            return false;
        }
        let u = p.payload;
        if u16::from_be_bytes([u[0], u[1]]) != SERVER_PORT || u16::from_be_bytes([u[2], u[3]]) != CLIENT_PORT {
            return false;
        }
        let d = &u[8..];
        if d[0] != 2 || d[4..8] != self.xid.to_be_bytes() || d[28..34] != self.mac || d[236..240] != MAGIC {
            return false;
        }
        let (mut kind, mut mask, mut gw, mut dns, mut server, mut secs) = (0u8, None, [0u8; 4], [0u8; 4], [0u8; 4], 3600u32);
        let mut o = &d[240..];
        while let [code, rest @ ..] = o {
            if *code == 255 {
                break;
            }
            if *code == 0 {
                o = rest;
                continue;
            }
            let [len, rest @ ..] = rest else { break };
            let len = usize::from(*len);
            if rest.len() < len {
                break;
            }
            let v = &rest[..len];
            match (*code, len) {
                (53, 1) => kind = v[0],
                (1, 4) => mask = Some(u32::from_be_bytes([v[0], v[1], v[2], v[3]])),
                (3, 4..) => gw.copy_from_slice(&v[..4]),
                (6, 4..) => dns.copy_from_slice(&v[..4]),
                (54, 4) => server.copy_from_slice(v),
                (51, 4) => secs = u32::from_be_bytes([v[0], v[1], v[2], v[3]]).max(120),
                _ => {}
            }
            o = &rest[len..];
        }
        let yi = [d[16], d[17], d[18], d[19]];
        match (self.state, kind) {
            (State::Discover, 2) if yi != [0; 4] => {
                self.state = State::Request { ip: yi, server };
                self.last_tx = None;
                self.tries = 0;
            }
            (State::Request { .. } | State::Renew, 5) if yi != [0; 4] => {
                let prefix = mask.map_or(24, |m| m.leading_ones() as u8);
                self.lease = Some(Lease { ip: yi, prefix, gw, dns, secs });
                self.state = State::Bound;
                self.bound_at = now;
            }
            (State::Request { .. }, 6) => {
                self.state = State::Discover; // NAK: start over
                self.last_tx = None;
                self.tries = 0;
            }
            _ => {}
        }
        true
    }

    fn build(&self, kind: u8, want: Option<[u8; 4]>, server: Option<[u8; 4]>) -> Vec<u8> {
        let mut d = alloc::vec![0u8; 240];
        d[0] = 1; // BOOTREQUEST
        d[1] = 1; // Ethernet
        d[2] = 6;
        d[4..8].copy_from_slice(&self.xid.to_be_bytes());
        d[10] = 0x80; // ask for a broadcast reply: we have no address to unicast to yet
        d[28..34].copy_from_slice(&self.mac);
        d[236..240].copy_from_slice(&MAGIC);
        d.extend_from_slice(&[53, 1, kind]);
        d.extend_from_slice(&[61, 7, 1]);
        d.extend_from_slice(&self.mac);
        d.extend_from_slice(&[12, 4, b'l', b'o', b'o', b's']);
        if let Some(ip) = want {
            d.extend_from_slice(&[50, 4]);
            d.extend_from_slice(&ip);
        }
        if let Some(s) = server {
            d.extend_from_slice(&[54, 4]);
            d.extend_from_slice(&s);
        }
        d.extend_from_slice(&[55, 3, 1, 3, 6, 255]); // parameter request: mask, router, DNS
        while d.len() < 300 {
            d.push(0); // some servers drop BOOTP packets shorter than 300 bytes
        }
        let mut u = Vec::with_capacity(8 + d.len());
        u.extend_from_slice(&CLIENT_PORT.to_be_bytes());
        u.extend_from_slice(&SERVER_PORT.to_be_bytes());
        u.extend_from_slice(&((8 + d.len()) as u16).to_be_bytes());
        u.extend_from_slice(&[0, 0]); // UDP checksum optional over IPv4
        u.extend_from_slice(&d);
        let ip = build_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED), IpAddr::V4(Ipv4Addr::BROADCAST), P_UDP, 64, 1, &u).unwrap_or_default();
        build_eth(BCAST, self.mac, ETH_IPV4, &ip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(c: &Dhcp, kind: u8, yi: [u8; 4]) -> Vec<u8> {
        let mut d = alloc::vec![0u8; 240];
        d[0] = 2;
        d[4..8].copy_from_slice(&c.xid.to_be_bytes());
        d[16..20].copy_from_slice(&yi);
        d[28..34].copy_from_slice(&c.mac);
        d[236..240].copy_from_slice(&MAGIC);
        d.extend_from_slice(&[53, 1, kind, 1, 4, 255, 255, 255, 0, 3, 4, 192, 168, 80, 2, 6, 8, 192, 168, 80, 2, 8, 8, 8, 8, 54, 4, 192, 168, 80, 254, 51, 4, 0, 0, 7, 8, 255]);
        let mut u = alloc::vec![0, 67, 0, 68, 0, 0, 0, 0];
        u.extend_from_slice(&d);
        let l = (u.len() as u16).to_be_bytes();
        u[4..6].copy_from_slice(&l);
        let ip = build_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 80, 254)), IpAddr::V4(Ipv4Addr::BROADCAST), P_UDP, 64, 1, &u).unwrap();
        build_eth(BCAST, [2; 6], ETH_IPV4, &ip)
    }

    #[test]
    fn full_exchange_and_renewal() {
        let mut c = Dhcp::new([2, 0, 0, 0, 0, 1], 0xdead_beef);
        let discover = c.poll(0).unwrap();
        assert_eq!(discover[14 + 20 + 8 + 240 + 2], 1);
        assert!(c.poll(100).is_none(), "no retransmit before the retry interval");
        assert!(c.rx(&reply(&c, 2, [192, 168, 80, 130]), 200));
        let request = c.poll(200).unwrap();
        assert_eq!(request[14 + 20 + 8 + 240 + 2], 3);
        assert!(c.rx(&reply(&c, 5, [192, 168, 80, 130]), 300));
        let l = c.lease().unwrap();
        assert_eq!((l.ip, l.prefix, l.gw, l.dns), ([192, 168, 80, 130], 24, [192, 168, 80, 2], [192, 168, 80, 2]));
        assert_eq!(l.secs, 1800);
        assert!(c.poll(1000).is_none());
        assert!(c.poll(300 + 1800 * 500 + 1).is_none()); // moves to Renew
        assert!(c.poll(300 + 1800 * 500 + 2).is_some(), "renewal request goes out at half the lease");
        assert!(c.rx(&reply(&c, 5, [192, 168, 80, 130]), 1_000_000));
        assert!(c.poll(1_000_001).is_none());
    }

    #[test]
    fn foreign_frames_and_silence() {
        let mut c = Dhcp::new([2, 0, 0, 0, 0, 1], 7);
        let mut other = reply(&c, 2, [1, 2, 3, 4]);
        let n = other.len();
        other[14 + 20 + 8 + 4] ^= 1; // wrong xid
        assert!(!c.rx(&other, 0));
        assert!(!c.rx(&other[..n - 100], 0));
        let mut sent = 0;
        for t in (0..100_000).step_by(500) {
            sent += usize::from(c.poll(t).is_some());
        }
        assert!(!c.failed() && c.lease().is_none());
        assert!(sent > 10, "keeps retrying");
    }
}

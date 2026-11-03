//! Wire formats: Ethernet, IPv4/IPv6 headers and the Internet checksum.

use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Ethernet II ethertypes.
pub const ETH_IPV4: u16 = 0x0800;
/// ARP ethertype.
pub const ETH_ARP: u16 = 0x0806;
/// IPv6 ethertype.
pub const ETH_IPV6: u16 = 0x86dd;
/// IP protocol numbers.
pub const P_ICMP: u8 = 1;
/// TCP.
pub const P_TCP: u8 = 6;
/// UDP.
pub const P_UDP: u8 = 17;
/// ICMPv6.
pub const P_ICMP6: u8 = 58;

/// 48-bit MAC address.
pub type Mac = [u8; 6];
/// Ethernet broadcast.
pub const BCAST: Mac = [0xff; 6];

/// One's-complement running sum over `data`, continuing from `acc`.
pub fn sum(data: &[u8], mut acc: u32) -> u32 {
    let mut it = data.chunks_exact(2);
    for c in &mut it {
        acc += u32::from(u16::from_be_bytes([c[0], c[1]]));
    }
    if let [b] = it.remainder() {
        acc += u32::from(*b) << 8;
    }
    acc
}

/// Folds a running sum into the final complemented checksum.
pub fn fold(mut acc: u32) -> u16 {
    while acc >> 16 != 0 {
        acc = (acc & 0xffff) + (acc >> 16);
    }
    !(acc as u16)
}

/// Internet checksum of `data`.
pub fn checksum(data: &[u8]) -> u16 {
    fold(sum(data, 0))
}

/// Checksum of an upper-layer message including the pseudo-header. A message whose checksum field is
/// already filled in yields 0 when valid.
pub fn l4_checksum(src: IpAddr, dst: IpAddr, proto: u8, data: &[u8]) -> u16 {
    let mut acc = 0u32;
    match (src, dst) {
        (IpAddr::V4(s), IpAddr::V4(d)) => {
            acc = sum(&s.octets(), acc);
            acc = sum(&d.octets(), acc);
            acc += u32::from(proto) + data.len() as u32;
        }
        (IpAddr::V6(s), IpAddr::V6(d)) => {
            acc = sum(&s.octets(), acc);
            acc = sum(&d.octets(), acc);
            acc += (data.len() as u32 >> 16) + (data.len() as u32 & 0xffff) + u32::from(proto);
        }
        _ => return 0xffff,
    }
    fold(sum(data, acc))
}

/// Builds an Ethernet II frame.
pub fn build_eth(dst: Mac, src: Mac, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(14 + payload.len());
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    f.extend_from_slice(&ethertype.to_be_bytes());
    f.extend_from_slice(payload);
    f
}

/// Splits an Ethernet frame into `(dst, src, ethertype, payload)`.
pub fn parse_eth(f: &[u8]) -> Option<(Mac, Mac, u16, &[u8])> {
    if f.len() < 14 {
        return None;
    }
    let mut d = [0u8; 6];
    let mut s = [0u8; 6];
    d.copy_from_slice(&f[0..6]);
    s.copy_from_slice(&f[6..12]);
    Some((d, s, u16::from_be_bytes([f[12], f[13]]), &f[14..]))
}

/// A parsed IP packet (either family).
#[derive(Debug, Clone, Copy)]
pub struct Ip<'a> {
    /// Source address.
    pub src: IpAddr,
    /// Destination address.
    pub dst: IpAddr,
    /// Upper-layer protocol / next header.
    pub proto: u8,
    /// TTL / hop limit.
    pub ttl: u8,
    /// Payload after the IP header.
    pub payload: &'a [u8],
}

/// Parses and validates an IPv4 or IPv6 packet. Fragments and IPv6 extension headers are rejected.
pub fn parse_ip(b: &[u8]) -> Option<Ip<'_>> {
    match b.first()? >> 4 {
        4 => {
            let ihl = usize::from(b[0] & 15) * 4;
            if b.len() < 20 || ihl < 20 || b.len() < ihl {
                return None;
            }
            let total = usize::from(u16::from_be_bytes([b[2], b[3]]));
            if total < ihl || total > b.len() || checksum(&b[..ihl]) != 0 {
                return None;
            }
            if u16::from_be_bytes([b[6], b[7]]) & 0x3fff != 0 {
                return None; // MF set or fragment offset != 0
            }
            Some(Ip {
                src: IpAddr::V4(Ipv4Addr::new(b[12], b[13], b[14], b[15])),
                dst: IpAddr::V4(Ipv4Addr::new(b[16], b[17], b[18], b[19])),
                proto: b[9],
                ttl: b[8],
                payload: &b[ihl..total],
            })
        }
        6 => {
            if b.len() < 40 {
                return None;
            }
            let len = usize::from(u16::from_be_bytes([b[4], b[5]]));
            if 40 + len > b.len() {
                return None;
            }
            let mut s = [0u8; 16];
            let mut d = [0u8; 16];
            s.copy_from_slice(&b[8..24]);
            d.copy_from_slice(&b[24..40]);
            Some(Ip { src: IpAddr::V6(Ipv6Addr::from(s)), dst: IpAddr::V6(Ipv6Addr::from(d)), proto: b[6], ttl: b[7], payload: &b[40..40 + len] })
        }
        _ => None,
    }
}

/// Builds an IP packet. Returns `None` when the address families differ or the payload is too large.
pub fn build_ip(src: IpAddr, dst: IpAddr, proto: u8, ttl: u8, id: u16, payload: &[u8]) -> Option<Vec<u8>> {
    let mut p = Vec::with_capacity(40 + payload.len());
    match (src, dst) {
        (IpAddr::V4(s), IpAddr::V4(d)) => {
            let total = u16::try_from(20 + payload.len()).ok()?;
            p.extend_from_slice(&[0x45, 0]);
            p.extend_from_slice(&total.to_be_bytes());
            p.extend_from_slice(&id.to_be_bytes());
            p.extend_from_slice(&[0x40, 0, ttl, proto, 0, 0]);
            p.extend_from_slice(&s.octets());
            p.extend_from_slice(&d.octets());
            let c = checksum(&p);
            p[10..12].copy_from_slice(&c.to_be_bytes());
        }
        (IpAddr::V6(s), IpAddr::V6(d)) => {
            let len = u16::try_from(payload.len()).ok()?;
            p.extend_from_slice(&[0x60, 0, 0, 0]);
            p.extend_from_slice(&len.to_be_bytes());
            p.extend_from_slice(&[proto, ttl]);
            p.extend_from_slice(&s.octets());
            p.extend_from_slice(&d.octets());
        }
        _ => return None,
    }
    p.extend_from_slice(payload);
    Some(p)
}

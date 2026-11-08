//! Longest-prefix-match routing table for both address families.

use alloc::vec::Vec;
use core::net::IpAddr;

/// One routing table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// Destination network.
    pub dst: IpAddr,
    /// Prefix length in bits.
    pub prefix: u8,
    /// Next-hop gateway; `None` means the destination is on-link.
    pub gw: Option<IpAddr>,
    /// Outgoing interface index.
    pub iface: usize,
    /// Lower wins among equal-length prefixes.
    pub metric: u32,
}

fn octets(a: IpAddr) -> ([u8; 16], usize) {
    match a {
        IpAddr::V4(v) => {
            let mut o = [0u8; 16];
            o[..4].copy_from_slice(&v.octets());
            (o, 32)
        }
        IpAddr::V6(v) => (v.octets(), 128),
    }
}

/// True when the first `prefix` bits of `a` and `b` agree (and the families match).
pub fn prefix_match(a: IpAddr, b: IpAddr, prefix: u8) -> bool {
    if a.is_ipv4() != b.is_ipv4() {
        return false;
    }
    let (x, width) = octets(a);
    let (y, _) = octets(b);
    let n = usize::from(prefix).min(width);
    let full = n / 8;
    if x[..full] != y[..full] {
        return false;
    }
    let rem = n % 8;
    rem == 0 || (x[full] ^ y[full]) >> (8 - rem) == 0
}

/// The routing table.
#[derive(Debug, Default)]
pub struct RouteTable {
    routes: Vec<Route>,
}

impl RouteTable {
    /// Adds a route.
    pub fn add(&mut self, r: Route) {
        self.routes.push(r);
    }

    /// Removes every route that goes out `iface`.
    pub fn remove_iface(&mut self, iface: usize) {
        self.routes.retain(|r| r.iface != iface);
    }

    /// Removes the IPv4 routes that go out `iface` (a new lease replaces them).
    pub fn remove_iface_v4(&mut self, iface: usize) {
        self.routes.retain(|r| r.iface != iface || !r.dst.is_ipv4());
    }

    /// Best route for `dst`: longest prefix, then lowest metric.
    pub fn lookup(&self, dst: IpAddr) -> Option<&Route> {
        self.routes
            .iter()
            .filter(|r| prefix_match(r.dst, dst, r.prefix))
            .max_by(|a, b| a.prefix.cmp(&b.prefix).then(b.metric.cmp(&a.metric)))
    }
}

//! Winsock (`ws2_32`) over a kernel-provided TCP/UDP stack ([`NetOps`]): sockets with blocking and non-blocking I/O,
//! `select`, name resolution through DNS over UDP, `getaddrinfo`/`gethostbyname`. Blocking calls yield to other threads.

use crate::{Process, Ret};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// First syscall number of this table.
pub const WS_BASE: u32 = 0x800;
const SOCK_BASE: u64 = 0x30000;
const SOCKET_ERROR: u64 = u64::MAX;
const WOULDBLOCK: i32 = 10035;
const TIMEOUT_MS: u64 = 30_000;

/// TCP connection state as the sockets layer needs it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TcpState {
    /// Handshake in progress.
    Connecting,
    /// Data can flow.
    Established,
    /// Closed or finished.
    Closed,
    /// Reset or timed out.
    Failed,
}

/// The network stack as the sockets layer sees it (implemented by the kernel over `loos-net`).
pub trait NetOps {
    /// Runs the stack: NIC receive/transmit and timers.
    fn poll(&mut self, now_ms: u64);
    /// Active open to `ip:port`.
    fn tcp_connect(&mut self, ip: [u8; 4], port: u16) -> Option<usize>;
    /// State of connection `h`.
    fn tcp_state(&mut self, h: usize) -> TcpState;
    /// Queues data; returns the bytes accepted.
    fn tcp_send(&mut self, h: usize, data: &[u8]) -> Option<usize>;
    /// Reads received data.
    fn tcp_recv(&mut self, h: usize, buf: &mut [u8]) -> usize;
    /// True when the peer closed and everything was read.
    fn tcp_eof(&mut self, h: usize) -> bool;
    /// Bytes ready to read.
    fn tcp_readable(&mut self, h: usize) -> bool;
    /// Closes our side.
    fn tcp_close(&mut self, h: usize);
    /// Listener on `port`.
    fn tcp_listen(&mut self, port: u16) -> Option<usize>;
    /// Takes an established connection from a listener.
    fn tcp_accept(&mut self, l: usize) -> Option<usize>;
    /// UDP socket bound to `port`.
    fn udp_bind(&mut self, port: u16) -> Option<usize>;
    /// Sends a datagram.
    fn udp_send(&mut self, h: usize, ip: [u8; 4], port: u16, data: &[u8]) -> bool;
    /// Receives a datagram `(source, port, data)`.
    fn udp_recv(&mut self, h: usize) -> Option<([u8; 4], u16, Vec<u8>)>;
    /// Bytes ready to read on a TCP connection; stacks that cannot tell report 1 when anything is readable.
    fn tcp_available(&mut self, h: usize) -> usize {
        usize::from(self.tcp_readable(h))
    }
    /// Resolver address.
    fn dns_server(&self) -> [u8; 4];
}

/// A Winsock socket.
#[derive(Default)]
pub struct Sock {
    dgram: bool,
    tcp: Option<usize>,
    udp: Option<usize>,
    listener: bool,
    nonblock: bool,
    connecting: bool,
    port: u16,
    peer: Option<([u8; 4], u16)>,
    /// `setsockopt` values by `(level, name)`.
    opts: BTreeMap<(i32, i32), u32>,
    /// Our side was shut down (`shutdown(SD_SEND)`): the stack connection is already closed.
    shut: bool,
}

/// An in-flight DNS lookup.
pub struct Dns {
    name: String,
    udp: usize,
    id: u16,
    sent: u64,
    tries: u32,
}

const SERVICES: &[(&str, u16)] = &[
    ("ftp", 21), ("ssh", 22), ("telnet", 23), ("smtp", 25), ("domain", 53), ("http", 80), ("pop3", 110), ("ntp", 123),
    ("imap", 143), ("https", 443), ("imaps", 993), ("pop3s", 995),
];
const PROTOCOLS: &[(&str, u8)] = &[("ip", 0), ("icmp", 1), ("tcp", 6), ("udp", 17)];

const NAMES: &[&str] = &[
    "WSAStartup", "WSACleanup", "WSAGetLastError", "WSASetLastError", "socket", "closesocket", "ioctlsocket", "setsockopt",
    "getsockopt", "connect", "bind", "listen", "accept", "send", "recv", "sendto", "recvfrom", "shutdown", "select", "getsockname",
    "getpeername", "gethostname", "gethostbyname", "getaddrinfo", "GetAddrInfoW", "freeaddrinfo", "FreeAddrInfoW", "htons", "htonl",
    "ntohs", "ntohl", "inet_addr", "inet_ntoa", "__WSAFDIsSet", "WSAIoctl", "WSASocketW", "WSAEventSelect", "gethostbyaddr",
    "WSASend", "WSARecv", "WSASendTo", "WSARecvFrom", "WSAPoll", "inet_pton", "inet_ntop", "InetPtonW", "getservbyname",
    "getservbyport", "getprotobyname", "getprotobynumber", "getnameinfo", "WSAGetOverlappedResult", "WSAIsBlocking",
    "WSAHtons", "WSAHtonl", "WSANtohs", "WSANtohl",
];

/// Syscall number of a ws2_32 import.
pub fn lookup(name: &str) -> Option<u32> {
    NAMES.iter().position(|n| *n == name).map(|i| WS_BASE + i as u32)
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

pub fn dns_query(name: &str, id: u16) -> Vec<u8> {
    let mut q = Vec::new();
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.split('.') {
        q.push(label.len().min(63) as u8);
        q.extend_from_slice(&label.as_bytes()[..label.len().min(63)]);
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]);
    q
}

/// First A record of a DNS response, or `Err(())` for a failure code.
pub fn dns_answer(r: &[u8], id: u16) -> Result<Option<[u8; 4]>, ()> {
    if r.len() < 12 || be16(&r[0..2]) != id {
        return Ok(None);
    }
    if r[3] & 15 != 0 {
        return Err(());
    }
    let (qd, an) = (be16(&r[4..6]), be16(&r[6..8]));
    let skip_name = |mut i: usize| -> Option<usize> {
        loop {
            let l = *r.get(i)? as usize;
            if l == 0 {
                return Some(i + 1);
            }
            if l & 0xc0 == 0xc0 {
                return Some(i + 2);
            }
            i += 1 + l;
        }
    };
    let mut i = 12;
    for _ in 0..qd {
        i = skip_name(i).ok_or(())? + 4;
    }
    for _ in 0..an {
        i = skip_name(i).ok_or(())?;
        let (ty, rdlen) = (be16(r.get(i..i + 2).ok_or(())?), be16(r.get(i + 8..i + 10).ok_or(())?) as usize);
        let data = r.get(i + 10..i + 10 + rdlen).ok_or(())?;
        if ty == 1 && rdlen == 4 {
            return Ok(Some([data[0], data[1], data[2], data[3]]));
        }
        i += 10 + rdlen;
    }
    Err(())
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut o = [0u8; 4];
    let mut it = s.split('.');
    for b in &mut o {
        *b = it.next()?.parse().ok()?;
    }
    it.next().is_none().then_some(o)
}

enum Lookup {
    Done([u8; 4]),
    Pending,
    Failed,
}

impl Process {
    /// Installs the network stack (the kernel does this when a NIC is present).
    pub fn set_net(&mut self, n: Box<dyn NetOps>) {
        self.net = Some(n);
    }

    /// Runs the stack's timers and NIC queues.
    pub fn net_poll(&mut self, now_ms: u64) {
        if let Some(n) = self.net.as_mut() {
            n.poll(now_ms);
        }
    }

    fn ws_fail(&mut self, e: i32) -> Option<Ret> {
        self.ws_err = e;
        Some(Ret::Value(SOCKET_ERROR))
    }

    fn sock_idx(&self, h: u64) -> Option<usize> {
        let i = h.checked_sub(SOCK_BASE)? as usize / 4;
        self.socks.get(i)?.as_ref().map(|_| i)
    }

    /// Blocks the calling thread (retry later) unless the socket is non-blocking.
    fn would_block(&mut self, nonblock: bool, now: u64) -> Option<Ret> {
        if nonblock {
            return self.ws_fail(WOULDBLOCK);
        }
        let t = self.cur_thread();
        let d = *t.deadline.get_or_insert(now + TIMEOUT_MS);
        if now >= d {
            t.deadline = None;
            return self.ws_fail(10060);
        }
        Some(Ret::Yield)
    }

    fn done(&mut self, v: u64) -> Option<Ret> {
        self.cur_thread().deadline = None;
        Some(Ret::Value(v))
    }

    fn read_addr(&mut self, a: u64) -> Option<([u8; 4], u16)> {
        let b = self.mem.read_vec(a, 8).ok()?;
        Some(([b[4], b[5], b[6], b[7]], be16(&b[2..4])))
    }

    fn write_addr(&mut self, a: u64, ip: [u8; 4], port: u16) {
        let mut b = [0u8; 16];
        b[0] = 2;
        b[2..4].copy_from_slice(&port.to_be_bytes());
        b[4..8].copy_from_slice(&ip);
        let _ = self.mem.write(a, &b);
    }

    fn resolve(&mut self, name: &str, now: u64) -> Lookup {
        if let Some(ip) = parse_ip(name) {
            return Lookup::Done(ip);
        }
        if name.eq_ignore_ascii_case("localhost") {
            return Lookup::Done([127, 0, 0, 1]);
        }
        let Some(net) = self.net.as_mut() else { return Lookup::Failed };
        let server = net.dns_server();
        match &mut self.dns {
            Some(q) if q.name == name => {
                if let Some((_, _, data)) = net.udp_recv(q.udp) {
                    match dns_answer(&data, q.id) {
                        Ok(Some(ip)) => {
                            self.dns = None;
                            return Lookup::Done(ip);
                        }
                        Ok(None) => {}
                        Err(()) => {
                            self.dns = None;
                            return Lookup::Failed;
                        }
                    }
                }
                if now.saturating_sub(q.sent) > 1500 {
                    if q.tries >= 3 {
                        self.dns = None;
                        return Lookup::Failed;
                    }
                    q.tries += 1;
                    q.sent = now;
                    net.udp_send(q.udp, server, 53, &dns_query(name, q.id));
                }
                Lookup::Pending
            }
            _ => {
                self.dns_seq = self.dns_seq.wrapping_add(1);
                let (id, port) = (0x4c00 | (self.dns_seq & 0xff), 40000 + (self.dns_seq % 2000));
                let Some(udp) = net.udp_bind(port) else { return Lookup::Failed };
                net.udp_send(udp, server, 53, &dns_query(name, id));
                self.dns = Some(Dns { name: String::from(name), udp, id, sent: now, tries: 1 });
                Lookup::Pending
            }
        }
    }

    /// Readiness of socket `i` for `select`/`WSAPoll`.
    fn sock_ready(&mut self, i: usize, writable: bool) -> bool {
        let Some(s) = self.socks.get(i).and_then(|s| s.as_ref()) else { return false };
        let (tcp, udp, listener) = (s.tcp, s.udp, s.listener);
        let Some(net) = self.net.as_mut() else { return false };
        match (tcp, udp) {
            (Some(t), _) if listener => !writable && net.tcp_readable(t),
            (Some(t), _) if writable => net.tcp_state(t) == TcpState::Established,
            (Some(t), _) => net.tcp_readable(t) || net.tcp_eof(t) || net.tcp_state(t) == TcpState::Failed,
            (None, Some(_)) => writable,
            _ => false,
        }
    }

    /// Bytes `FIONREAD` reports.
    fn sock_available(&mut self, i: usize) -> usize {
        match (self.socks.get(i).and_then(|s| s.as_ref()).and_then(|s| s.tcp), self.net.as_mut()) {
            (Some(t), Some(n)) => n.tcp_available(t),
            _ => 0,
        }
    }

    /// Concatenates the `WSABUF` array `{ u32 len; u8 *buf }` at `bufs`.
    fn gather(&mut self, bufs: u64, count: u32) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        for k in 0..u64::from(count.min(64)) {
            let (len, p) = (self.mem.read_u32(bufs + 16 * k).ok()? as usize, self.mem.read_u64(bufs + 16 * k + 8).ok()?);
            out.extend_from_slice(&self.mem.read_vec(p, len.min(1 << 20)).ok()?);
        }
        Some(out)
    }

    fn wsabuf_cap(&mut self, bufs: u64, count: u32) -> usize {
        (0..u64::from(count.min(64))).map(|k| self.mem.read_u32(bufs + 16 * k).unwrap_or(0) as usize).sum()
    }

    fn scatter(&mut self, bufs: u64, count: u32, mut data: &[u8]) {
        for k in 0..u64::from(count.min(64)) {
            let (len, p) = (self.mem.read_u32(bufs + 16 * k).unwrap_or(0) as usize, self.mem.read_u64(bufs + 16 * k + 8).unwrap_or(0));
            let n = len.min(data.len());
            let _ = self.mem.write(p, &data[..n]);
            data = &data[n..];
        }
    }

    /// Sends `data` on socket `i`; `to` selects the datagram destination (connected peer otherwise).
    fn ws_send(&mut self, i: usize, data: &[u8], to: bool, dest: ([u8; 4], u16), now: u64) -> Option<Ret> {
        let (dgram, nb, shut) = {
            let s = self.socks[i].as_ref()?;
            (s.dgram, s.nonblock, s.shut)
        };
        if dgram {
            let dest = if to { dest } else { self.socks[i].as_ref()?.peer? };
            let h = self.socks[i].as_ref()?.udp?;
            let ok = self.net.as_mut()?.udp_send(h, dest.0, dest.1, data);
            return if ok { Some(Ret::Value(data.len() as u64)) } else { self.ws_fail(10051) };
        }
        if shut {
            return self.ws_fail(10058);
        }
        let h = self.socks[i].as_ref()?.tcp?;
        if self.net.as_mut()?.tcp_state(h) != TcpState::Established {
            return self.ws_fail(10057);
        }
        match self.net.as_mut()?.tcp_send(h, data) {
            Some(n) if n > 0 || data.is_empty() => self.done(n as u64),
            _ => self.would_block(nb, now),
        }
    }

    /// Receives up to `cap` bytes; `Err` carries the call result to return (error, would-block, EOF).
    #[allow(clippy::type_complexity)]
    fn ws_recv(&mut self, i: usize, cap: usize, now: u64) -> Option<Result<(Vec<u8>, Option<([u8; 4], u16)>), Ret>> {
        let (dgram, nb) = (self.socks[i].as_ref()?.dgram, self.socks[i].as_ref()?.nonblock);
        if dgram {
            let h = self.socks[i].as_ref()?.udp?;
            return Some(match self.net.as_mut()?.udp_recv(h) {
                Some((ip, port, mut d)) => {
                    d.truncate(cap);
                    Ok((d, Some((ip, port))))
                }
                None => Err(self.would_block(nb, now)?),
            });
        }
        let h = self.socks[i].as_ref()?.tcp?;
        let mut buf = alloc::vec![0u8; cap];
        let n = self.net.as_mut()?.tcp_recv(h, &mut buf);
        if n > 0 {
            buf.truncate(n);
            return Some(Ok((buf, None)));
        }
        let (eof, st) = (self.net.as_mut()?.tcp_eof(h), self.net.as_mut()?.tcp_state(h));
        if eof || st == TcpState::Closed {
            self.cur_thread().deadline = None;
            return Some(Ok((Vec::new(), None)));
        }
        if st == TcpState::Failed {
            return Some(Err(self.ws_fail(10054)?));
        }
        Some(Err(self.would_block(nb, now)?))
    }

    fn put_str(&mut self, s: &str) -> u64 {
        let mut b = Vec::from(s.as_bytes());
        b.push(0);
        let a = self.heap_alloc(b.len() as u64);
        let _ = self.mem.write(a, &b);
        a
    }

    /// Winsock calls. `None` for numbers outside this table.
    pub(crate) fn ws_syscall(&mut self, nr: u32, a: [u64; 4], rsp: u64, now: u64) -> Option<Ret> {
        let name = *NAMES.get(nr.checked_sub(WS_BASE)? as usize)?;
        let v = |x: u64| Some(Ret::Value(x));
        match name {
            "WSAStartup" => {
                let req = a[0] as u16;
                let (major, minor) = ((req & 0xff) as u8, (req >> 8) as u8);
                if major == 0 {
                    return v(10092);
                }
                let ver = if major > 2 || (major == 2 && minor > 2) { 0x0202 } else { req };
                // 64-bit WSADATA: versions, iMaxSockets, iMaxUdpDg, lpVendorInfo, szDescription[257], szSystemStatus[129].
                let mut b = alloc::vec![0u8; 408];
                b[0..2].copy_from_slice(&ver.to_le_bytes());
                b[2..4].copy_from_slice(&0x0202u16.to_le_bytes());
                b[4..6].copy_from_slice(&32767u16.to_le_bytes());
                b[6..8].copy_from_slice(&65467u16.to_le_bytes());
                b[16..16 + 24].copy_from_slice(b"LoOS Winsock 2.2\0\0\0\0\0\0\0\0");
                b[273..273 + 8].copy_from_slice(b"Running\0");
                let _ = self.mem.write(a[1], &b);
                self.ws_init += 1;
                v(0)
            }
            "WSACleanup" => {
                if self.ws_init == 0 {
                    return self.ws_fail(10093);
                }
                self.ws_init -= 1;
                v(0)
            }
            "WSAEventSelect" => v(0),
            "WSAIsBlocking" => v(0),
            "WSAHtons" | "WSANtohs" => {
                let r = (a[1] as u16).swap_bytes();
                let _ = self.mem.write(a[2], &r.to_le_bytes());
                v(0)
            }
            "WSAHtonl" | "WSANtohl" => {
                let r = (a[1] as u32).swap_bytes();
                let _ = self.mem.write(a[2], &r.to_le_bytes());
                v(0)
            }
            "WSAGetOverlappedResult" => v(1),
            "shutdown" => {
                let i = self.sock_idx(a[0])?;
                if a[1] as u32 > 2 {
                    return self.ws_fail(10022);
                }
                let s = self.socks[i].as_ref()?;
                if s.dgram || s.listener || s.tcp.is_none() {
                    return self.ws_fail(10057);
                }
                if a[1] != 0 && !s.shut {
                    let t = s.tcp?;
                    self.net.as_mut()?.tcp_close(t);
                    self.socks[i].as_mut()?.shut = true;
                }
                v(0)
            }
            "setsockopt" => {
                let i = self.sock_idx(a[0])?;
                let len = self.stack_arg(rsp, 5) as usize;
                let val = match len {
                    0 => 0,
                    1..=3 => u32::from(self.mem.read_vec(a[3], 1).ok()?[0]),
                    _ => self.mem.read_u32(a[3]).ok()?,
                };
                self.socks[i].as_mut()?.opts.insert((a[1] as i32, a[2] as i32), val);
                v(0)
            }
            "getsockopt" => {
                let i = self.sock_idx(a[0])?;
                let lenp = self.stack_arg(rsp, 5);
                let s = self.socks[i].as_ref()?;
                let key = (a[1] as i32, a[2] as i32);
                let val = match key {
                    (0xffff, 0x1007) => 0, // SO_ERROR
                    (0xffff, 0x1008) => if s.dgram { 2 } else { 1 }, // SO_TYPE
                    (0xffff, 0x1001 | 0x1002) => s.opts.get(&key).copied().unwrap_or(65536), // SO_SNDBUF / SO_RCVBUF
                    (0xffff, 0x0002) => u32::from(s.listener), // SO_ACCEPTCONN
                    _ => s.opts.get(&key).copied().unwrap_or(0),
                };
                if a[3] == 0 || lenp == 0 {
                    return self.ws_fail(10014);
                }
                let _ = self.mem.write(a[3], &val.to_le_bytes());
                let _ = self.mem.write(lenp, &4u32.to_le_bytes());
                v(0)
            }
            "WSAGetLastError" => v(self.ws_err as u32 as u64),
            "WSASetLastError" => {
                self.ws_err = a[0] as i32;
                v(0)
            }
            "socket" | "WSASocketW" => {
                let dgram = a[1] == 2;
                self.socks.push(Some(Sock { dgram, ..Default::default() }));
                v(SOCK_BASE + (self.socks.len() as u64 - 1) * 4)
            }
            "closesocket" => {
                let i = self.sock_idx(a[0])?;
                if let (Some(s), Some(n)) = (self.socks[i].take(), self.net.as_mut()) {
                    if let (Some(t), false) = (s.tcp, s.shut) {
                        n.tcp_close(t);
                    }
                }
                v(0)
            }
            "ioctlsocket" => {
                let i = self.sock_idx(a[0])?;
                match a[1] as u32 {
                    0x8004_667e => {
                        let on = self.mem.read_u32(a[2]).unwrap_or(0) != 0;
                        self.socks[i].as_mut()?.nonblock = on;
                    }
                    0x4004_667f => {
                        let n = self.sock_available(i);
                        let _ = self.mem.write(a[2], &(n as u32).to_le_bytes());
                    }
                    _ => return self.ws_fail(10022),
                }
                v(0)
            }
            "WSAIoctl" => {
                // (s, code, in, insize, out[5], outsize[6], &returned[7], overlapped[8], routine[9])
                let i = self.sock_idx(a[0])?;
                let (out, ret) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 7));
                let wrote = match a[1] as u32 {
                    0x8004_667e => {
                        let on = self.mem.read_u32(a[2]).unwrap_or(0) != 0;
                        self.socks[i].as_mut()?.nonblock = on;
                        0
                    }
                    0x4004_667f => {
                        let n = self.sock_available(i);
                        if out == 0 {
                            return self.ws_fail(10014);
                        }
                        let _ = self.mem.write(out, &(n as u32).to_le_bytes());
                        4
                    }
                    0x9800_0004 | 0x9800_000c | 0x9800_0010 => 0, // KEEPALIVE_VALS, UDP_CONNRESET, UDP_NETRESET: accepted, no effect
                    0xc800_0006 => return self.ws_fail(10022), // SIO_GET_EXTENSION_FUNCTION_POINTER (AcceptEx, ConnectEx, ...) not provided
                    _ => return self.ws_fail(10045),
                };
                if ret != 0 {
                    let _ = self.mem.write(ret, &(wrote as u32).to_le_bytes());
                }
                v(0)
            }
            "htons" | "ntohs" => v(u64::from((a[0] as u16).swap_bytes())),
            "htonl" | "ntohl" => v(u64::from((a[0] as u32).swap_bytes())),
            "inet_addr" => {
                let s = self.mem.read_cstr(a[0], 32).unwrap_or_default();
                v(parse_ip(&s).map_or(0xffff_ffff, |o| u64::from(u32::from_le_bytes(o))))
            }
            "inet_ntoa" => {
                let o = (a[0] as u32).to_le_bytes();
                let s = alloc::format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3]);
                v(self.put_str(&s))
            }
            "gethostname" => {
                let _ = self.mem.write(a[0], b"loos\0");
                v(0)
            }
            "bind" => {
                let i = self.sock_idx(a[0])?;
                let (_, port) = self.read_addr(a[1])?;
                let dgram = self.socks[i].as_ref()?.dgram;
                if dgram {
                    let h = self.net.as_mut()?.udp_bind(port);
                    let Some(h) = h else { return self.ws_fail(10048) };
                    self.socks[i].as_mut()?.udp = Some(h);
                }
                self.socks[i].as_mut()?.port = port;
                v(0)
            }
            "listen" => {
                let i = self.sock_idx(a[0])?;
                let port = self.socks[i].as_ref()?.port;
                let Some(h) = self.net.as_mut()?.tcp_listen(port) else { return self.ws_fail(10048) };
                let s = self.socks[i].as_mut()?;
                (s.tcp, s.listener) = (Some(h), true);
                v(0)
            }
            "accept" => {
                let i = self.sock_idx(a[0])?;
                let (l, nb) = (self.socks[i].as_ref()?.tcp?, self.socks[i].as_ref()?.nonblock);
                match self.net.as_mut()?.tcp_accept(l) {
                    Some(h) => {
                        self.socks.push(Some(Sock { tcp: Some(h), ..Default::default() }));
                        self.done(SOCK_BASE + (self.socks.len() as u64 - 1) * 4)
                    }
                    None => self.would_block(nb, now),
                }
            }
            "connect" => {
                let i = self.sock_idx(a[0])?;
                let (ip, port) = self.read_addr(a[1])?;
                let s = self.socks[i].as_mut()?;
                if s.dgram {
                    s.peer = Some((ip, port));
                    if s.udp.is_none() {
                        let h = self.net.as_mut()?.udp_bind(50000 + i as u16);
                        self.socks[i].as_mut()?.udp = h;
                    }
                    return v(0);
                }
                let (nb, connecting) = (s.nonblock, s.connecting);
                if !connecting {
                    let Some(h) = self.net.as_mut()?.tcp_connect(ip, port) else { return self.ws_fail(10051) };
                    let s = self.socks[i].as_mut()?;
                    (s.tcp, s.connecting, s.peer) = (Some(h), true, Some((ip, port)));
                }
                let h = self.socks[i].as_ref()?.tcp?;
                match self.net.as_mut()?.tcp_state(h) {
                    TcpState::Established => {
                        self.socks[i].as_mut()?.connecting = false;
                        self.done(0)
                    }
                    TcpState::Connecting => {
                        if nb {
                            self.ws_fail(10036)
                        } else {
                            self.would_block(false, now)
                        }
                    }
                    _ => {
                        self.socks[i].as_mut()?.connecting = false;
                        self.cur_thread().deadline = None;
                        self.ws_fail(10061)
                    }
                }
            }
            "send" | "sendto" => {
                let i = self.sock_idx(a[0])?;
                let len = (a[2] as usize).min(1 << 20);
                let data = self.mem.read_vec(a[1], len).ok()?;
                let sa = self.stack_arg(rsp, 5);
                let dest = if name == "sendto" { self.read_addr(sa)? } else { ([0; 4], 0) };
                self.ws_send(i, &data, name == "sendto", dest, now)
            }
            "WSASend" | "WSASendTo" => {
                let i = self.sock_idx(a[0])?;
                let data = self.gather(a[1], a[2] as u32)?;
                let to = name == "WSASendTo";
                let sa = self.stack_arg(rsp, 6);
                let dest = if to { self.read_addr(sa)? } else { ([0; 4], 0) };
                match self.ws_send(i, &data, to, dest, now)? {
                    Ret::Value(n) if n != SOCKET_ERROR => {
                        if a[3] != 0 {
                            let _ = self.mem.write(a[3], &(n as u32).to_le_bytes());
                        }
                        v(0)
                    }
                    Ret::Value(_) => v(SOCKET_ERROR),
                    r => Some(r),
                }
            }
            "recv" | "recvfrom" => {
                let i = self.sock_idx(a[0])?;
                let cap = (a[2] as usize).min(1 << 20);
                match self.ws_recv(i, cap, now)? {
                    Ok((d, from)) => {
                        let _ = self.mem.write(a[1], &d);
                        if name == "recvfrom" {
                            let fp = self.stack_arg(rsp, 5);
                            if let (Some((ip, port)), true) = (from, fp != 0) {
                                self.write_addr(fp, ip, port);
                            }
                        }
                        self.done(d.len() as u64)
                    }
                    Err(r) => Some(r),
                }
            }
            "WSARecv" | "WSARecvFrom" => {
                let i = self.sock_idx(a[0])?;
                let cap = self.wsabuf_cap(a[1], a[2] as u32).min(1 << 20);
                match self.ws_recv(i, cap, now)? {
                    Ok((d, from)) => {
                        self.scatter(a[1], a[2] as u32, &d);
                        if a[3] != 0 {
                            let _ = self.mem.write(a[3], &(d.len() as u32).to_le_bytes());
                        }
                        if name == "WSARecvFrom" {
                            let fp = self.stack_arg(rsp, 6);
                            if let (Some((ip, port)), true) = (from, fp != 0) {
                                self.write_addr(fp, ip, port);
                            }
                        }
                        self.done(0)
                    }
                    Err(r) => Some(r),
                }
            }
            "WSAPoll" => {
                // WSAPOLLFD { SOCKET fd; i16 events; i16 revents } x nfds; timeout in ms (-1 = infinite).
                let (fds, n) = (a[0], a[1].min(64));
                let mut ready = 0u64;
                for k in 0..n {
                    let at = fds + 16 * k;
                    let h = self.mem.read_u64(at).unwrap_or(0);
                    let ev = self.mem.read_vec(at + 8, 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
                    let rev: u16 = match self.sock_idx(h) {
                        None => 4, // POLLNVAL
                        Some(i) => {
                            let mut r = 0;
                            if ev & 0x0300 != 0 && self.sock_ready(i, false) {
                                r |= ev & 0x0300;
                            }
                            if ev & 0x0010 != 0 && self.sock_ready(i, true) {
                                r |= 0x0010;
                            }
                            r
                        }
                    };
                    let _ = self.mem.write(at + 10, &rev.to_le_bytes());
                    ready += u64::from(rev != 0);
                }
                if ready > 0 {
                    return self.done(ready);
                }
                let ms = a[2] as u32 as i32;
                let t = self.cur_thread();
                let d = *t.deadline.get_or_insert(now + if ms < 0 { TIMEOUT_MS } else { ms as u64 });
                if now >= d {
                    t.deadline = None;
                    return v(0);
                }
                Some(Ret::Yield)
            }
            "inet_pton" | "InetPtonW" => {
                if a[0] != 2 {
                    self.ws_err = 10047;
                    return v(u64::MAX);
                }
                let s = if name == "InetPtonW" { crate::k32_wide(&mut self.mem, a[1]) } else { self.mem.read_cstr(a[1], 64).ok() };
                match s.as_deref().and_then(parse_ip) {
                    Some(ip) => {
                        let _ = self.mem.write(a[2], &ip);
                        v(1)
                    }
                    None => v(0),
                }
            }
            "inet_ntop" => {
                if a[0] != 2 {
                    self.ws_err = 10047;
                    return v(0);
                }
                let ip = self.mem.read_vec(a[1], 4).ok()?;
                let s = alloc::format!("{}.{}.{}.{}\0", ip[0], ip[1], ip[2], ip[3]);
                if (a[3] as usize) < s.len() {
                    self.ws_err = 10065;
                    return v(0);
                }
                let _ = self.mem.write(a[2], s.as_bytes());
                v(a[2])
            }
            "getservbyname" | "getservbyport" => {
                let proto = self.mem.read_cstr(a[1], 8).unwrap_or_default();
                let proto = if proto.is_empty() { "tcp".into() } else { proto };
                let found = if name == "getservbyname" {
                    let n = self.mem.read_cstr(a[0], 32).unwrap_or_default().to_ascii_lowercase();
                    SERVICES.iter().find(|(s, _)| *s == n)
                } else {
                    let port = (a[0] as u16).swap_bytes();
                    SERVICES.iter().find(|(_, p)| *p == port)
                };
                let Some(&(sname, port)) = found else {
                    self.ws_err = 11004;
                    return v(0);
                };
                let blk = self.heap_alloc(64);
                let (nm, pr) = (self.put_str(sname), self.put_str(&proto));
                let mut b = [0u8; 32];
                b[0..8].copy_from_slice(&nm.to_le_bytes());
                b[8..16].copy_from_slice(&(blk + 32).to_le_bytes()); // empty alias list (NULL at blk+32)
                b[16..18].copy_from_slice(&port.swap_bytes().to_le_bytes());
                b[24..32].copy_from_slice(&pr.to_le_bytes());
                let _ = self.mem.write(blk, &b);
                v(blk)
            }
            "getprotobyname" | "getprotobynumber" => {
                let found = if name == "getprotobyname" {
                    let n = self.mem.read_cstr(a[0], 16).unwrap_or_default().to_ascii_lowercase();
                    PROTOCOLS.iter().find(|(s, _)| *s == n)
                } else {
                    PROTOCOLS.iter().find(|(_, p)| i32::from(*p) == a[0] as i32)
                };
                let Some(&(pname, num)) = found else {
                    self.ws_err = 11004;
                    return v(0);
                };
                let blk = self.heap_alloc(48);
                let nm = self.put_str(pname);
                let mut b = [0u8; 24];
                b[0..8].copy_from_slice(&nm.to_le_bytes());
                b[8..16].copy_from_slice(&(blk + 24).to_le_bytes());
                b[16..18].copy_from_slice(&i16::from(num).to_le_bytes());
                let _ = self.mem.write(blk, &b);
                v(blk)
            }
            "getnameinfo" => {
                // (sa, salen, host, hostlen, serv[5], servlen[6], flags[7]): numeric results only (no reverse DNS).
                let (ip, port) = self.read_addr(a[0])?;
                let host = alloc::format!("{}.{}.{}.{}\0", ip[0], ip[1], ip[2], ip[3]);
                let (serv, servlen) = (self.stack_arg(rsp, 5), self.stack_arg(rsp, 6) as usize);
                let svc = alloc::format!("{port}\0");
                if (a[3] as usize) < host.len() || (serv != 0 && servlen < svc.len()) {
                    return v(10014); // WSAEFAULT: buffer too small
                }
                if a[2] != 0 {
                    let _ = self.mem.write(a[2], host.as_bytes());
                }
                if serv != 0 {
                    let _ = self.mem.write(serv, svc.as_bytes());
                }
                v(0)
            }
            "select" => {
                // (nfds, readfds, writefds, exceptfds, timeout*): fd_set = u32 count + SOCKET[64] at +8.
                let (rd, wr, tv) = (a[1], a[2], self.stack_arg(rsp, 5));
                let mut ready = 0u64;
                for (set, writable) in [(rd, false), (wr, true)] {
                    if set == 0 {
                        continue;
                    }
                    let n = self.mem.read_u32(set).unwrap_or(0).min(64) as usize;
                    let mut keep = Vec::new();
                    for k in 0..n {
                        let h = self.mem.read_u64(set + 8 + 8 * k as u64).unwrap_or(0);
                        let Some(i) = self.sock_idx(h) else { continue };
                        let ok = self.sock_ready(i, writable);
                        if ok {
                            keep.push(h);
                        }
                    }
                    let _ = self.mem.write(set, &(keep.len() as u32).to_le_bytes());
                    for (k, h) in keep.iter().enumerate() {
                        let _ = self.mem.write(set + 8 + 8 * k as u64, &h.to_le_bytes());
                    }
                    ready += keep.len() as u64;
                }
                if ready > 0 {
                    return self.done(ready);
                }
                let timeout = if tv == 0 { None } else { Some(self.mem.read_u32(tv).unwrap_or(0) as u64 * 1000 + self.mem.read_u32(tv + 4).unwrap_or(0) as u64 / 1000) };
                let t = self.cur_thread();
                let d = *t.deadline.get_or_insert(now + timeout.unwrap_or(TIMEOUT_MS));
                if now >= d {
                    t.deadline = None;
                    return v(0);
                }
                Some(Ret::Yield)
            }
            "__WSAFDIsSet" => {
                let n = self.mem.read_u32(a[1]).unwrap_or(0).min(64);
                v((0..n).any(|k| self.mem.read_u64(a[1] + 8 + 8 * u64::from(k)).unwrap_or(0) == a[0]) as u64)
            }
            "getsockname" | "getpeername" => {
                let i = self.sock_idx(a[0])?;
                let (ip, port) = if name == "getpeername" { self.socks[i].as_ref()?.peer.unwrap_or(([0; 4], 0)) } else { ([10, 0, 2, 15], self.socks[i].as_ref()?.port) };
                self.write_addr(a[1], ip, port);
                if a[2] != 0 {
                    let _ = self.mem.write(a[2], &16u32.to_le_bytes());
                }
                v(0)
            }
            "gethostbyname" | "gethostbyaddr" => {
                let n = self.mem.read_cstr(a[0], 256).unwrap_or_default();
                match self.resolve(&n, now) {
                    Lookup::Done(ip) => {
                        let host = self.heap_alloc(32 + 16 + 16);
                        let (nm, addr) = (self.put_str(&n), host + 32);
                        let list = host + 48;
                        let _ = self.mem.write(addr, &ip);
                        let _ = self.mem.write(list, &addr.to_le_bytes());
                        let mut h = [0u8; 32];
                        h[0..8].copy_from_slice(&nm.to_le_bytes());
                        h[8..16].copy_from_slice(&(list + 8).to_le_bytes()); // empty alias list (NULL at list+8)
                        h[16..18].copy_from_slice(&2u16.to_le_bytes());
                        h[18..20].copy_from_slice(&4u16.to_le_bytes());
                        h[24..32].copy_from_slice(&list.to_le_bytes());
                        let _ = self.mem.write(host, &h);
                        self.done(host)
                    }
                    Lookup::Pending => self.would_block(false, now),
                    Lookup::Failed => {
                        self.cur_thread().deadline = None;
                        self.ws_err = 11001;
                        v(0)
                    }
                }
            }
            "getaddrinfo" | "GetAddrInfoW" => {
                let wide = name == "GetAddrInfoW";
                let rd = |p: &mut Process, ptr: u64| -> Option<String> {
                    if ptr == 0 {
                        return None;
                    }
                    if wide { crate::k32_wide(&mut p.mem, ptr) } else { p.mem.read_cstr(ptr, 256).ok() }
                };
                let node = rd(self, a[0]).unwrap_or_default();
                let svc = rd(self, a[1]).unwrap_or_default();
                let port: u16 = svc.parse().unwrap_or(match svc.as_str() {
                    "http" => 80,
                    "https" => 443,
                    "domain" => 53,
                    _ => 0,
                });
                let hints = a[2];
                let socktype = if hints != 0 { self.mem.read_u32(hints + 8).unwrap_or(0) } else { 0 };
                match self.resolve(&node, now) {
                    Lookup::Done(ip) => {
                        let ai = self.heap_alloc(48 + 16);
                        let sa = ai + 48;
                        self.write_addr(sa, ip, port);
                        let mut b = [0u8; 48];
                        b[4..8].copy_from_slice(&2i32.to_le_bytes());
                        b[8..12].copy_from_slice(&(if socktype == 0 { 1 } else { socktype }).to_le_bytes());
                        b[12..16].copy_from_slice(&6i32.to_le_bytes());
                        b[16..24].copy_from_slice(&16u64.to_le_bytes());
                        b[32..40].copy_from_slice(&sa.to_le_bytes());
                        let _ = self.mem.write(ai, &b);
                        let _ = self.mem.write(a[3], &ai.to_le_bytes());
                        self.done(0)
                    }
                    Lookup::Pending => self.would_block(false, now),
                    Lookup::Failed => {
                        self.cur_thread().deadline = None;
                        self.ws_err = 11001;
                        v(11001)
                    }
                }
            }
            "freeaddrinfo" | "FreeAddrInfoW" => v(0),
            _ => None,
        }
    }
}
